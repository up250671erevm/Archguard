//! `.pacnew` / `.pacsave` discovery and base-file lookup in the package cache.
//!
//! The 3-way merge base is the file as shipped by the *previous* version of
//! the owning package, extracted from a cached `*.pkg.tar.*` in
//! `/var/cache/pacman/pkg`.

use anyhow::{bail, Context, Result};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// Kind of leftover file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pacnew,
    Pacsave,
}

/// A discovered `.pacnew` / `.pacsave`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leftover {
    pub path: PathBuf,
    /// The live file it belongs to (path without suffix).
    pub target: PathBuf,
    pub kind: Kind,
}

/// Classify a path as `.pacnew` / `.pacsave` (also `.pacsave.N`).
pub fn classify(path: &Path) -> Option<Leftover> {
    let s = path.to_str()?;
    let (target, kind) = if let Some(t) = s.strip_suffix(".pacnew") {
        (t, Kind::Pacnew)
    } else {
        let idx = s.rfind(".pacsave")?;
        let rest = &s[idx + ".pacsave".len()..];
        let ok = rest.is_empty()
            || (rest.starts_with('.') && rest[1..].chars().all(|c| c.is_ascii_digit()));
        if !ok {
            return None;
        }
        (&s[..idx], Kind::Pacsave)
    };
    if target.is_empty() || target.ends_with('/') {
        return None;
    }
    Some(Leftover {
        path: path.to_path_buf(),
        target: PathBuf::from(target),
        kind,
    })
}

/// Recursively scan `roots` (no symlink following; unreadable dirs skipped).
pub fn scan(roots: &[PathBuf]) -> Vec<Leftover> {
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() {
                if let Some(l) = classify(&p) {
                    out.push(l);
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// A parsed cached package filename: `name-pkgver-pkgrel-arch.pkg.tar[.ext]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedPkg {
    pub path: PathBuf,
    pub name: String,
    /// `[epoch:]pkgver-pkgrel`
    pub version: String,
}

/// Parse a cache filename; signatures and partial downloads are rejected.
pub fn parse_cache_name(path: &Path) -> Option<CachedPkg> {
    let file = path.file_name()?.to_str()?;
    let idx = file.find(".pkg.tar")?;
    let ext = &file[idx + ".pkg.tar".len()..];
    if !matches!(
        ext,
        "" | ".zst" | ".xz" | ".gz" | ".bz2" | ".lz4" | ".lzo" | ".Z"
    ) {
        return None;
    }
    let stem = &file[..idx];
    let mut parts = stem.rsplitn(4, '-');
    let _arch = parts.next()?;
    let rel = parts.next()?;
    let ver = parts.next()?;
    let name = parts.next()?;
    if name.is_empty() || ver.is_empty() || rel.is_empty() {
        return None;
    }
    Some(CachedPkg {
        path: path.to_path_buf(),
        name: name.to_string(),
        version: format!("{ver}-{rel}"),
    })
}

/// Port of libalpm's `rpmvercmp` on a single version segment.
fn rpmvercmp(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    loop {
        while i < a.len() && !a[i].is_ascii_alphanumeric() {
            i += 1;
        }
        while j < b.len() && !b[j].is_ascii_alphanumeric() {
            j += 1;
        }
        if i >= a.len() || j >= b.len() {
            break;
        }
        let (si, sj) = (i, j);
        let numeric = a[i].is_ascii_digit();
        let take = |s: &[u8], mut k: usize| {
            while k < s.len()
                && (if numeric {
                    s[k].is_ascii_digit()
                } else {
                    s[k].is_ascii_alphabetic()
                })
            {
                k += 1;
            }
            k
        };
        i = take(a, i);
        j = take(b, j);
        if sj == j {
            // Segment types differ: numeric is newer.
            return if numeric {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
        let (mut x, mut y) = (&a[si..i], &b[sj..j]);
        if numeric {
            while x.len() > 1 && x[0] == b'0' {
                x = &x[1..];
            }
            while y.len() > 1 && y[0] == b'0' {
                y = &y[1..];
            }
            match x.len().cmp(&y.len()) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        match x.cmp(y) {
            Ordering::Equal => {}
            o => return o,
        }
    }
    let ra = &a[i.min(a.len())..];
    let rb = &b[j.min(b.len())..];
    match (ra.is_empty(), rb.is_empty()) {
        (true, true) => Ordering::Equal,
        // Remaining alpha means older ("1.0a" < "1.0"), else newer.
        (true, false) => {
            if rb[0].is_ascii_alphabetic() {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (false, true) => {
            if ra[0].is_ascii_alphabetic() {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        (false, false) => Ordering::Equal,
    }
}

fn split_evr(v: &str) -> (&str, &str, Option<&str>) {
    let (epoch, rest) = match v.split_once(':') {
        Some((e, r)) if e.chars().all(|c| c.is_ascii_digit()) => (e, r),
        _ => ("0", v),
    };
    match rest.rsplit_once('-') {
        Some((ver, rel)) => (epoch, ver, Some(rel)),
        None => (epoch, rest, None),
    }
}

/// Compare full `[epoch:]pkgver[-pkgrel]` versions like `vercmp(8)`.
pub fn vercmp(a: &str, b: &str) -> Ordering {
    let (ea, va, ra) = split_evr(a);
    let (eb, vb, rb) = split_evr(b);
    rpmvercmp(ea, eb)
        .then_with(|| rpmvercmp(va, vb))
        .then_with(|| match (ra, rb) {
            (Some(x), Some(y)) => rpmvercmp(x, y),
            _ => Ordering::Equal,
        })
}

/// The newest cached version of `name` strictly older than `installed`.
pub fn find_previous(cache: &[CachedPkg], name: &str, installed: &str) -> Option<CachedPkg> {
    cache
        .iter()
        .filter(|p| p.name == name && vercmp(&p.version, installed) == Ordering::Less)
        .max_by(|a, b| vercmp(&a.version, &b.version))
        .cloned()
}

/// List parsed packages in a cache directory.
pub fn list_cache(dir: &Path) -> Vec<CachedPkg> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|e| parse_cache_name(&e.path()))
        .collect()
}

/// Resolves which installed package owns a file.
pub trait Owner {
    /// `(name, installed version)` or `None` if unowned.
    fn owner(&self, path: &Path) -> Result<Option<(String, String)>>;
}

/// Parse `pacman -Qo` output: `/etc/x is owned by name 1.2-3`.
pub fn parse_qo(out: &str) -> Option<(String, String)> {
    let rest = out.lines().next()?.split(" is owned by ").nth(1)?;
    let mut it = rest.split_whitespace();
    Some((it.next()?.to_string(), it.next()?.to_string()))
}

/// Uses `pacman -Qo` (local DB query only).
pub struct PacmanOwner;

impl Owner for PacmanOwner {
    fn owner(&self, path: &Path) -> Result<Option<(String, String)>> {
        let out = std::process::Command::new("pacman")
            .arg("-Qo")
            .arg(path)
            .env("LC_ALL", "C")
            .output()
            .context("running `pacman -Qo`")?;
        if !out.status.success() {
            return Ok(None);
        }
        Ok(parse_qo(&String::from_utf8_lossy(&out.stdout)))
    }
}

/// A member of a package archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// Regular file with its content.
    File(Vec<u8>),
    /// Symbolic link with its (raw) target.
    Symlink(String),
    /// Hard link to another archive member path.
    Hardlink(String),
    /// Directory, device, fifo, ...
    Other,
}

/// Reads members of a package archive.
pub trait Extractor {
    fn entry(&self, archive: &Path, member: &str) -> Result<Entry>;
}

/// Characters bsdtar would interpret as a pattern in member names.
pub fn has_glob(member: &str) -> bool {
    member.contains(['*', '?', '[', ']', '\\'])
}

/// Parse one `tar -tv` listing line for `member`.
pub fn parse_listing(line: &str, member: &str) -> Option<Entry> {
    let kind = line.chars().next()?;
    let after_name = |sep: &str| {
        let needle = format!("{member}{sep}");
        line.find(&needle)
            .map(|i| line[i + needle.len()..].trim_end().to_string())
    };
    Some(match kind {
        '-' => Entry::File(Vec::new()),
        'l' => Entry::Symlink(after_name(" -> ")?),
        'h' => Entry::Hardlink(after_name(" link to ")?),
        _ => Entry::Other,
    })
}

/// Uses `bsdtar` (libarchive, a pacman dependency), falling back to GNU `tar`.
/// Compression (zst/xz/gz/...) is auto-detected by both.
pub struct TarExtractor;

impl TarExtractor {
    fn run(tool: &str, args: &[&std::ffi::OsStr]) -> Option<std::process::Output> {
        std::process::Command::new(tool).args(args).output().ok()
    }
}

impl Extractor for TarExtractor {
    fn entry(&self, archive: &Path, member: &str) -> Result<Entry> {
        if has_glob(member) {
            bail!("refusing archive member with glob characters: {member}");
        }
        let a = archive.as_os_str();
        let m = std::ffi::OsStr::new(member);
        let mut last = String::from("no tar/bsdtar available");
        for (tool, extra) in [("bsdtar", None), ("tar", Some("--no-wildcards"))] {
            let flag = |f: &'static str| std::ffi::OsStr::new(f);
            let mut list = vec![];
            list.extend(extra.map(flag));
            list.extend([flag("-tvf"), a, flag("--"), m]);
            let Some(o) = Self::run(tool, &list) else {
                continue;
            };
            if !o.status.success() {
                last = String::from_utf8_lossy(&o.stderr).trim().to_string();
                continue;
            }
            let listing = String::from_utf8_lossy(&o.stdout);
            let Some(entry) = listing.lines().find_map(|l| parse_listing(l, member)) else {
                bail!("{member} not found in {}", archive.display());
            };
            if entry != Entry::File(Vec::new()) {
                return Ok(entry);
            }
            let mut ex = vec![];
            ex.extend(extra.map(flag));
            ex.extend([flag("-xOf"), a, flag("--"), m]);
            match Self::run(tool, &ex) {
                Some(o) if o.status.success() => return Ok(Entry::File(o.stdout)),
                Some(o) => last = String::from_utf8_lossy(&o.stderr).trim().to_string(),
                None => {}
            }
        }
        bail!("could not read {member} from {}: {last}", archive.display())
    }
}

/// Resolve a link `target` found at archive member `from` to a member path.
/// Absolute targets are relative to the package root. `None` if it escapes.
pub fn resolve_link(from: &str, target: &str, hard: bool) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    if !hard && !target.starts_with('/') {
        parts.extend(from.split('/').filter(|p| !p.is_empty()));
        parts.pop();
    }
    for c in target.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            c => parts.push(c),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Maximum symlink hops followed inside an archive.
pub const MAX_LINK_HOPS: usize = 8;

/// Read a regular-file member, following links inside the archive.
pub fn read_member(ex: &dyn Extractor, archive: &Path, member: &str) -> Result<Vec<u8>> {
    let mut cur = member.to_string();
    for _ in 0..=MAX_LINK_HOPS {
        let (t, hard) = match ex.entry(archive, &cur)? {
            Entry::File(c) => return Ok(c),
            Entry::Other => bail!("{cur} in {} is not a regular file", archive.display()),
            Entry::Symlink(t) => (t, false),
            Entry::Hardlink(t) => (t, true),
        };
        cur = resolve_link(&cur, &t, hard)
            .with_context(|| format!("link {cur} -> {t} escapes the package"))?;
    }
    bail!("too many links resolving {member} in {}", archive.display())
}

/// Where a merge base came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    pub content: Vec<u8>,
    pub package: CachedPkg,
}

/// Find the 3-way merge base for `target`, or `Ok(None)` with a reason.
pub fn find_base(
    target: &Path,
    owner: &dyn Owner,
    cache: &[CachedPkg],
    extractor: &dyn Extractor,
) -> Result<std::result::Result<Base, String>> {
    let Some((name, installed)) = owner.owner(target)? else {
        return Ok(Err(format!(
            "{} is not owned by any package",
            target.display()
        )));
    };
    let Some(prev) = find_previous(cache, &name, &installed) else {
        return Ok(Err(format!(
            "no cached version of {name} older than {installed} in the package cache"
        )));
    };
    let member = target.to_string_lossy();
    let member = member.trim_start_matches('/');
    match read_member(extractor, &prev.path, member) {
        Ok(content) => Ok(Ok(Base {
            content,
            package: prev,
        })),
        Err(e) => Ok(Err(format!("{e:#}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn classify_paths() {
        let l = classify(Path::new("/etc/pacman.conf.pacnew")).unwrap();
        assert_eq!(l.target, PathBuf::from("/etc/pacman.conf"));
        assert_eq!(l.kind, Kind::Pacnew);
        let l = classify(Path::new("/etc/x.conf.pacsave.2")).unwrap();
        assert_eq!(l.target, PathBuf::from("/etc/x.conf"));
        assert_eq!(l.kind, Kind::Pacsave);
        assert!(classify(Path::new("/etc/x.conf")).is_none());
        assert!(classify(Path::new("/etc/x.pacsave.bak")).is_none());
        assert!(classify(Path::new("/etc/.pacnew")).is_none());
    }

    #[test]
    fn scan_tree() {
        let d = std::env::temp_dir().join(format!("ag-scan-{}", std::process::id()));
        std::fs::create_dir_all(d.join("a/b")).unwrap();
        for f in ["a/x.conf", "a/x.conf.pacnew", "a/b/y.pacsave", "z.txt"] {
            std::fs::write(d.join(f), "").unwrap();
        }
        let found = scan(std::slice::from_ref(&d));
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|l| l.kind == Kind::Pacsave));
        std::fs::remove_dir_all(&d).unwrap();
        assert!(scan(&[PathBuf::from("/nonexistent-dir")]).is_empty());
    }

    #[test]
    fn cache_names() {
        let p =
            parse_cache_name(Path::new("/c/lib32-foo-bar-1:2.3.4-5-x86_64.pkg.tar.zst")).unwrap();
        assert_eq!(p.name, "lib32-foo-bar");
        assert_eq!(p.version, "1:2.3.4-5");
        assert!(parse_cache_name(Path::new("foo-1-1-any.pkg.tar.zst.sig")).is_none());
        assert!(parse_cache_name(Path::new("foo-1-1-any.pkg.tar.zst.part")).is_none());
        assert!(parse_cache_name(Path::new("random.txt")).is_none());
        assert!(parse_cache_name(Path::new("foo-1-any.pkg.tar")).is_none());
    }

    #[test]
    fn vercmp_cases() {
        use Ordering::*;
        let cases = [
            ("1.0-1", "1.0-1", Equal),
            ("1.0-1", "1.0-2", Less),
            ("1.10", "1.9", Greater),
            ("1.0a", "1.0", Less),
            ("1.0", "1.0.1", Less),
            ("1:1.0", "2.0", Greater),
            ("1.0alpha", "1.0beta", Less),
            ("2.0", "1.0rc1", Greater),
            ("1.001", "1.1", Equal),
            ("1.0.a", "1.0.1", Less),
        ];
        for (a, b, o) in cases {
            assert_eq!(vercmp(a, b), o, "{a} vs {b}");
            assert_eq!(vercmp(b, a), o.reverse(), "{b} vs {a}");
        }
    }

    fn pkg(name: &str, v: &str) -> CachedPkg {
        CachedPkg {
            path: PathBuf::from(format!("/cache/{name}-{v}-x86_64.pkg.tar.zst")),
            name: name.into(),
            version: v.into(),
        }
    }

    #[test]
    fn previous_version() {
        let cache = vec![
            pkg("foo", "1.0-1"),
            pkg("foo", "1.2-1"),
            pkg("foo", "2.0-1"),
            pkg("foobar", "1.9-1"),
        ];
        assert_eq!(
            find_previous(&cache, "foo", "2.0-1").unwrap().version,
            "1.2-1"
        );
        assert!(find_previous(&cache, "foo", "1.0-1").is_none());
        assert!(find_previous(&cache, "baz", "1").is_none());
    }

    #[test]
    fn qo_parse() {
        assert_eq!(
            parse_qo("/etc/pacman.conf is owned by pacman 7.1.0-1\n"),
            Some(("pacman".into(), "7.1.0-1".into()))
        );
        assert_eq!(parse_qo("error: No package owns /x"), None);
    }

    struct FakeOwner(Option<(String, String)>);
    impl Owner for FakeOwner {
        fn owner(&self, _: &Path) -> Result<Option<(String, String)>> {
            Ok(self.0.clone())
        }
    }
    struct FakeExtractor(HashMap<(PathBuf, String), Entry>);
    impl Extractor for FakeExtractor {
        fn entry(&self, a: &Path, m: &str) -> Result<Entry> {
            self.0
                .get(&(a.to_path_buf(), m.to_string()))
                .cloned()
                .context("missing member")
        }
    }

    #[test]
    fn base_lookup_with_fake_cache() {
        let cache = vec![pkg("foo", "1.0-1"), pkg("foo", "2.0-1")];
        let ex = FakeExtractor(
            [(
                (cache[0].path.clone(), "etc/foo.conf".into()),
                Entry::File(b"base\n".to_vec()),
            )]
            .into(),
        );
        let own = FakeOwner(Some(("foo".into(), "2.0-1".into())));
        let b = find_base(Path::new("/etc/foo.conf"), &own, &cache, &ex)
            .unwrap()
            .unwrap();
        assert_eq!(b.content, b"base\n");
        assert_eq!(b.package.version, "1.0-1");

        let r = find_base(Path::new("/etc/bar.conf"), &own, &cache, &ex).unwrap();
        assert!(r.unwrap_err().contains("missing member"));
        let r = find_base(Path::new("/etc/x"), &FakeOwner(None), &cache, &ex).unwrap();
        assert!(r.unwrap_err().contains("not owned"));
        let own1 = FakeOwner(Some(("foo".into(), "1.0-1".into())));
        let r = find_base(Path::new("/etc/foo.conf"), &own1, &cache, &ex).unwrap();
        assert!(r.unwrap_err().contains("no cached version"));
    }

    #[test]
    fn symlink_members_followed_or_no_base() {
        let a = PathBuf::from("/c/foo-1.0-1-any.pkg.tar.zst");
        let k = |m: &str| (a.clone(), m.to_string());
        let ex = FakeExtractor(
            [
                (
                    k("etc/foo.conf"),
                    Entry::Symlink("../usr/share/foo/foo.conf".into()),
                ),
                (k("usr/share/foo/foo.conf"), Entry::File(b"real\n".to_vec())),
                (
                    k("etc/abs.conf"),
                    Entry::Symlink("/usr/share/foo/foo.conf".into()),
                ),
                (
                    k("etc/hard.conf"),
                    Entry::Hardlink("usr/share/foo/foo.conf".into()),
                ),
                (
                    k("etc/escape.conf"),
                    Entry::Symlink("../../../../etc/shadow".into()),
                ),
                (k("etc/dangling.conf"), Entry::Symlink("nope".into())),
                (k("etc/loop1"), Entry::Symlink("loop2".into())),
                (k("etc/loop2"), Entry::Symlink("loop1".into())),
                (k("etc/dir"), Entry::Other),
            ]
            .into(),
        );
        for m in ["etc/foo.conf", "etc/abs.conf", "etc/hard.conf"] {
            assert_eq!(read_member(&ex, &a, m).unwrap(), b"real\n", "{m}");
        }
        for m in [
            "etc/escape.conf",
            "etc/dangling.conf",
            "etc/loop1",
            "etc/dir",
        ] {
            assert!(read_member(&ex, &a, m).is_err(), "{m}");
        }
        // find_base reports "no base" (never an empty base) for a bad link.
        let cache = vec![CachedPkg {
            path: a.clone(),
            name: "foo".into(),
            version: "1.0-1".into(),
        }];
        let own = FakeOwner(Some(("foo".into(), "2.0-1".into())));
        assert!(
            find_base(Path::new("/etc/dangling.conf"), &own, &cache, &ex)
                .unwrap()
                .is_err()
        );
        let b = find_base(Path::new("/etc/foo.conf"), &own, &cache, &ex)
            .unwrap()
            .unwrap();
        assert_eq!(b.content, b"real\n");
    }

    #[test]
    fn link_resolution() {
        assert_eq!(
            resolve_link("etc/a/b", "c", false).as_deref(),
            Some("etc/a/c")
        );
        assert_eq!(
            resolve_link("etc/a/b", "../c", false).as_deref(),
            Some("etc/c")
        );
        assert_eq!(
            resolve_link("etc/b", "/usr/x", false).as_deref(),
            Some("usr/x")
        );
        assert_eq!(
            resolve_link("etc/b", "usr/x", true).as_deref(),
            Some("usr/x")
        );
        assert_eq!(resolve_link("etc/b", "../../x", false), None);
    }

    #[test]
    fn listing_parse_and_globs() {
        let m = "etc/foo.conf";
        assert_eq!(
            parse_listing(
                "lrwxrwxrwx  0 root root 0 Jan  1  2026 etc/foo.conf -> ../x y",
                m
            ),
            Some(Entry::Symlink("../x y".into()))
        );
        assert_eq!(
            parse_listing(
                "hrw-r--r--  0 root root 0 Jan  1  2026 etc/foo.conf link to usr/x",
                m
            ),
            Some(Entry::Hardlink("usr/x".into()))
        );
        assert_eq!(
            parse_listing("-rw-r--r--  0 root root 5 Jan  1  2026 etc/foo.conf", m),
            Some(Entry::File(vec![]))
        );
        assert_eq!(
            parse_listing("drwxr-xr-x 0 root root 0 Jan 1 2026 etc/", m),
            Some(Entry::Other)
        );
        assert!(has_glob("etc/*.conf") && has_glob("etc/a[1]") && !has_glob("etc/a.conf"));
        assert!(TarExtractor
            .entry(Path::new("/x.pkg.tar"), "etc/*")
            .is_err());
    }

    fn have(tool: &str) -> bool {
        std::process::Command::new(tool)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// Real archives: plain, .zst and .xz, with a symlink member.
    #[test]
    fn tar_extractor_real_archives() {
        let d = std::env::temp_dir().join(format!("ag-tarz-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("root/etc")).unwrap();
        std::fs::create_dir_all(d.join("root/usr/share/foo")).unwrap();
        std::fs::write(d.join("root/usr/share/foo/foo.conf"), "hello\n").unwrap();
        std::os::unix::fs::symlink("../usr/share/foo/foo.conf", d.join("root/etc/foo.conf"))
            .unwrap();
        std::fs::write(d.join("root/etc/plain.conf"), "plain\n").unwrap();
        let mut variants = vec![("", None)];
        if have("zstd") {
            variants.push((".zst", Some("--zstd")));
        }
        if have("xz") {
            variants.push((".xz", Some("--xz")));
        }
        for (ext, flag) in variants {
            let archive = d.join(format!("foo-1.0-1-any.pkg.tar{ext}"));
            let mut c = std::process::Command::new("tar");
            if let Some(f) = flag {
                c.arg(f);
            }
            let ok = c
                .arg("-cf")
                .arg(&archive)
                .arg("-C")
                .arg(d.join("root"))
                .args(["etc/foo.conf", "etc/plain.conf", "usr/share/foo/foo.conf"])
                .status()
                .is_ok_and(|s| s.success());
            if !ok {
                continue;
            }
            assert_eq!(
                read_member(&TarExtractor, &archive, "etc/plain.conf").unwrap(),
                b"plain\n",
                "{ext}"
            );
            assert_eq!(
                TarExtractor.entry(&archive, "etc/foo.conf").unwrap(),
                Entry::Symlink("../usr/share/foo/foo.conf".into()),
                "{ext}"
            );
            assert_eq!(
                read_member(&TarExtractor, &archive, "etc/foo.conf").unwrap(),
                b"hello\n",
                "{ext}"
            );
            assert!(TarExtractor.entry(&archive, "etc/missing").is_err());
            assert_eq!(parse_cache_name(&archive).unwrap().name, "foo");
        }
        std::fs::remove_dir_all(&d).unwrap();
    }
}
