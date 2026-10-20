//! Durable file writes.

use anyhow::{Context, Result};
use std::fs::File;
use std::io::Write;
use std::path::Path;

/// What is being fsynced (for tests / tracing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncTarget {
    File,
    Dir,
}

/// Write `data` to `path` atomically and durably:
/// create `path.tmp`, write, `fsync` it, rename over `path`, `fsync` the
/// parent directory so the rename itself survives a crash.
pub fn write_durable(path: &Path, data: &[u8]) -> Result<()> {
    write_durable_with(path, data, &mut |f, _| f.sync_all())
}

/// Like [`write_durable`] with an injectable sync function.
pub fn write_durable_with(
    path: &Path,
    data: &[u8],
    sync: &mut dyn FnMut(&File, SyncTarget) -> std::io::Result<()>,
) -> Result<()> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = path.with_extension("tmp");
    let res = (|| -> Result<()> {
        let mut f = File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(data)
            .with_context(|| format!("writing {}", tmp.display()))?;
        sync(&f, SyncTarget::File).with_context(|| format!("fsync {}", tmp.display()))?;
        Ok(())
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))?;
    let d = File::open(dir).with_context(|| format!("opening {}", dir.display()))?;
    sync(&d, SyncTarget::Dir).with_context(|| format!("fsync {}", dir.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ag-fs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn fsyncs_file_then_dir() {
        let d = tmpdir("order");
        let p = d.join("state");
        let mut calls = Vec::new();
        write_durable_with(&p, b"x", &mut |f, t| {
            // The file must be synced before the rename (tmp still exists).
            if t == SyncTarget::File {
                assert!(p.with_extension("tmp").exists());
                assert!(!p.exists());
            }
            calls.push(t);
            f.sync_all()
        })
        .unwrap();
        assert_eq!(calls, [SyncTarget::File, SyncTarget::Dir]);
        assert_eq!(std::fs::read(&p).unwrap(), b"x");
        assert!(!p.with_extension("tmp").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn failed_fsync_keeps_old_file() {
        let d = tmpdir("fail");
        let p = d.join("state");
        write_durable(&p, b"old").unwrap();
        let err = write_durable_with(&p, b"new", &mut |_, _| {
            Err(std::io::Error::other("disk on fire"))
        });
        assert!(err.is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"old");
        assert!(!p.with_extension("tmp").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
