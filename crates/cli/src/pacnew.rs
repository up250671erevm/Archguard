//! `archguard pacnew {list,diff,merge}`.
//!
//! Privilege model: everything (reading, merging, launching meld/kdiff3)
//! runs as the invoking user. Only the final backup / install / rename /
//! remove steps are elevated with sudo or run0, and only when the target
//! is not writable by the user.

use anyhow::{bail, Context, Result};
use archguard_core::config::{Elevation, MergeTool};
use archguard_core::pacnew::{classify, find_base, scan, CachedPkg, Extractor, Kind, Owner};
use archguard_merge::{merge3, unified_diff};
use archguard_runner::{CommandRunner, CommandSpec};
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Resolved external merge tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Meld,
    Kdiff3,
}

impl Tool {
    pub fn bin(self) -> &'static str {
        match self {
            Tool::Meld => "meld",
            Tool::Kdiff3 => "kdiff3",
        }
    }

    /// Command for the tool. `out` already contains the user's file; for a
    /// 2-way session (no base) the user edits `out` against `theirs`.
    pub fn command(
        self,
        base: Option<&Path>,
        ours: &Path,
        theirs: &Path,
        out: &Path,
    ) -> CommandSpec {
        let p = |x: &Path| x.display().to_string();
        let args = match (self, base) {
            (Tool::Meld, Some(b)) => vec![format!("--output={}", p(out)), p(ours), p(b), p(theirs)],
            (Tool::Meld, None) => vec![p(out), p(theirs)],
            (Tool::Kdiff3, Some(b)) => vec![p(b), p(ours), p(theirs), "-o".into(), p(out)],
            (Tool::Kdiff3, None) => vec![p(ours), p(theirs), "-o".into(), p(out)],
        };
        CommandSpec {
            program: self.bin().into(),
            args,
        }
    }
}

/// Find `bin` in `$PATH`.
pub fn in_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// Resolve the configured tool against what is installed.
pub fn resolve_tool(cfg: MergeTool, available: &dyn Fn(&str) -> bool) -> Option<Tool> {
    let pick = |t: Tool| available(t.bin()).then_some(t);
    match cfg {
        MergeTool::None => None,
        MergeTool::Meld => pick(Tool::Meld),
        MergeTool::Kdiff3 => pick(Tool::Kdiff3),
        MergeTool::Auto => pick(Tool::Meld).or_else(|| pick(Tool::Kdiff3)),
    }
}

/// Dependencies of the pacnew commands (injectable for tests).
pub struct PacnewCtx<'a> {
    pub scan_dirs: Vec<PathBuf>,
    pub owner: &'a dyn Owner,
    pub extractor: &'a dyn Extractor,
    pub cache: Vec<CachedPkg>,
    pub runner: &'a mut dyn CommandRunner,
    pub confirm: &'a mut dyn FnMut(&str) -> bool,
    pub tool: Option<Tool>,
    pub elevation: Elevation,
    pub is_root: bool,
    /// Private (0700) scratch directory.
    pub tmpdir: PathBuf,
    /// Whether the user can write `target` (and its directory) directly.
    pub writable: &'a dyn Fn(&Path) -> bool,
    /// Unix time used for backup names.
    pub now: i64,
}

/// `(live target, .pacnew path)` from either path.
pub fn resolve_pair(file: &Path) -> Result<(PathBuf, PathBuf)> {
    match classify(file) {
        Some(l) if l.kind == Kind::Pacnew => Ok((l.target, l.path)),
        Some(_) => bail!(".pacsave files are backups of removed configs; inspect them manually"),
        None => {
            let mut p = file.as_os_str().to_owned();
            p.push(".pacnew");
            Ok((file.to_path_buf(), PathBuf::from(p)))
        }
    }
}

fn read(p: &Path) -> Result<String> {
    let b = std::fs::read(p).with_context(|| format!("reading {}", p.display()))?;
    String::from_utf8(b).with_context(|| format!("{} is not UTF-8 text", p.display()))
}

/// `pacnew list`
pub fn cmd_list(ctx: &PacnewCtx, out: &mut dyn Write) -> Result<i32> {
    let found = scan(&ctx.scan_dirs);
    if found.is_empty() {
        writeln!(out, "archguard: no .pacnew/.pacsave files found.")?;
    }
    for l in &found {
        let k = match l.kind {
            Kind::Pacnew => "pacnew ",
            Kind::Pacsave => "pacsave",
        };
        writeln!(out, "{k}  {}", l.path.display())?;
    }
    Ok(0)
}

/// `pacnew diff <file>`
pub fn cmd_diff(file: &Path, out: &mut dyn Write) -> Result<i32> {
    let (target, pacnew) = resolve_pair(file)?;
    let d = unified_diff(
        &read(&target)?,
        &read(&pacnew)?,
        &target.display().to_string(),
        &pacnew.display().to_string(),
    );
    write!(out, "{d}")?;
    Ok(0)
}

fn elevated(ctx: &PacnewCtx, program: &str, args: Vec<String>) -> CommandSpec {
    if ctx.is_root {
        return CommandSpec {
            program: program.into(),
            args,
        };
    }
    let mut a = vec![program.to_string()];
    a.extend(args);
    CommandSpec {
        program: match ctx.elevation {
            Elevation::Sudo => "sudo",
            Elevation::Run0 => "run0",
        }
        .into(),
        args: a,
    }
}

fn run_ok(ctx: &mut PacnewCtx, cmd: CommandSpec) -> Result<()> {
    let code = ctx.runner.run(&cmd)?;
    if code != 0 {
        bail!("`{cmd}` failed with exit code {code}");
    }
    Ok(())
}

/// Install `content` over `target` (with backup) and remove `pacnew`.
/// Returns the backup path.
pub fn install(
    ctx: &mut PacnewCtx,
    target: &Path,
    pacnew: &Path,
    content: &str,
) -> Result<PathBuf> {
    let meta = std::fs::metadata(target).with_context(|| format!("stat {}", target.display()))?;
    let name = target
        .file_name()
        .context("target has no file name")?
        .to_string_lossy();
    let backup = target.with_file_name(format!("{name}.archguard-bak.{}", ctx.now));
    let staged = target.with_file_name(format!("{name}.archguard-new"));

    if (ctx.writable)(target) {
        std::fs::copy(target, &backup)?;
        std::fs::write(&staged, content)?;
        std::fs::set_permissions(&staged, meta.permissions())?;
        std::fs::File::open(&staged)?.sync_all()?;
        std::fs::rename(&staged, target)?;
        std::fs::remove_file(pacnew)?;
        return Ok(backup);
    }

    // Stage as the user, then elevate only the final steps.
    let tmp = ctx.tmpdir.join(format!("{name}.merged"));
    std::fs::write(&tmp, content)?;
    let s = |p: &Path| p.display().to_string();
    let steps = vec![
        elevated(
            ctx,
            "cp",
            vec!["-a".into(), "--".into(), s(target), s(&backup)],
        ),
        elevated(
            ctx,
            "install",
            vec![
                "-m".into(),
                format!("{:o}", meta.mode() & 0o7777),
                "-o".into(),
                meta.uid().to_string(),
                "-g".into(),
                meta.gid().to_string(),
                "--".into(),
                s(&tmp),
                s(&staged),
            ],
        ),
        elevated(
            ctx,
            "mv",
            vec!["-f".into(), "--".into(), s(&staged), s(target)],
        ),
        elevated(ctx, "rm", vec!["-f".into(), "--".into(), s(pacnew)]),
    ];
    for c in steps {
        run_ok(ctx, c)?;
    }
    let _ = std::fs::remove_file(&tmp);
    Ok(backup)
}

/// Outcome of `pacnew merge`.
#[derive(Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    Installed {
        backup: PathBuf,
    },
    Declined,
    /// Conflicts left in this file for manual resolution.
    LeftConflicts(PathBuf),
    /// No base and no tool: user must merge manually.
    Manual,
    NothingToDo,
}

fn has_markers(s: &str) -> bool {
    s.lines()
        .any(|l| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> ") || l == "=======")
}

/// `pacnew merge <file>`
pub fn cmd_merge(
    ctx: &mut PacnewCtx,
    file: &Path,
    yes: bool,
    out: &mut dyn Write,
) -> Result<MergeOutcome> {
    let (target, pacnew) = resolve_pair(file)?;
    let ours = read(&target)?;
    let theirs = read(&pacnew)?;
    let name = target
        .file_name()
        .context("no file name")?
        .to_string_lossy()
        .into_owned();
    let tdisp = target.display().to_string();

    if ours == theirs {
        writeln!(out, "{tdisp}: identical to .pacnew; removing .pacnew.")?;
        if (ctx.writable)(&target) {
            std::fs::remove_file(&pacnew)?;
        } else {
            let c = elevated(
                ctx,
                "rm",
                vec!["-f".into(), "--".into(), pacnew.display().to_string()],
            );
            run_ok(ctx, c)?;
        }
        return Ok(MergeOutcome::NothingToDo);
    }

    let base = find_base(&target, ctx.owner, &ctx.cache, ctx.extractor)?.and_then(|b| {
        String::from_utf8(b.content)
            .map(|c| (c, b.package))
            .map_err(|_| "base file is not UTF-8".to_string())
    });

    let merged = match &base {
        Ok((b, pkg)) => {
            writeln!(out, "base: {tdisp} from {} {}", pkg.name, pkg.version)?;
            let r = merge3(b, &ours, &theirs);
            if r.is_clean() {
                Some(r.text)
            } else {
                writeln!(out, "{} conflicting region(s).", r.conflicts)?;
                None
            }
        }
        Err(why) => {
            writeln!(out, "no 3-way base available: {why}")?;
            None
        }
    };

    let result = match merged {
        Some(m) => m,
        None => {
            let ours_p = ctx.tmpdir.join(format!("{name}.current"));
            let theirs_p = ctx.tmpdir.join(format!("{name}.pacnew"));
            let out_p = ctx.tmpdir.join(format!("{name}.merged"));
            std::fs::write(&ours_p, &ours)?;
            std::fs::write(&theirs_p, &theirs)?;
            let base_p = match &base {
                Ok((b, _)) => {
                    let p = ctx.tmpdir.join(format!("{name}.base"));
                    std::fs::write(&p, b)?;
                    Some(p)
                }
                Err(_) => None,
            };
            match ctx.tool {
                Some(tool) => {
                    std::fs::write(&out_p, &ours)?;
                    let cmd = tool.command(base_p.as_deref(), &ours_p, &theirs_p, &out_p);
                    writeln!(out, "launching {}: {cmd}", tool.bin())?;
                    let code = ctx.runner.run(&cmd)?;
                    let r = read(&out_p).unwrap_or_default();
                    if code != 0 || r.is_empty() || has_markers(&r) {
                        writeln!(
                            out,
                            "{} did not produce a resolved file; nothing written.",
                            tool.bin()
                        )?;
                        return Ok(MergeOutcome::Declined);
                    }
                    r
                }
                None => match &base {
                    Ok((b, _)) => {
                        std::fs::write(&out_p, merge3(b, &ours, &theirs).text)?;
                        writeln!(
                            out,
                            "No merge tool (meld/kdiff3) available. Conflicts left in {}.\n\
                             Edit it, then install it with e.g. `sudo cp {} {tdisp}` and remove {}.",
                            out_p.display(),
                            out_p.display(),
                            pacnew.display()
                        )?;
                        return Ok(MergeOutcome::LeftConflicts(out_p));
                    }
                    Err(_) => {
                        writeln!(
                            out,
                            "No merge tool (meld/kdiff3) available. Compare manually:\n  \
                             archguard pacnew diff {tdisp}"
                        )?;
                        return Ok(MergeOutcome::Manual);
                    }
                },
            }
        }
    };

    write!(
        out,
        "{}",
        unified_diff(&ours, &result, &tdisp, &format!("{tdisp} (merged)"))
    )?;
    if !yes && !(ctx.confirm)(&format!("Write merged result to {tdisp}?")) {
        writeln!(out, "Not written.")?;
        return Ok(MergeOutcome::Declined);
    }
    let backup = install(ctx, &target, &pacnew, &result)?;
    writeln!(
        out,
        "Wrote {tdisp} (backup: {}); removed .pacnew.",
        backup.display()
    )?;
    Ok(MergeOutcome::Installed { backup })
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_runner::MockRunner;
    use std::collections::HashMap;

    struct FakeOwner;
    impl Owner for FakeOwner {
        fn owner(&self, p: &Path) -> Result<Option<(String, String)>> {
            Ok(p.ends_with("foo.conf")
                .then(|| ("foo".into(), "2.0-1".into())))
        }
    }

    /// Fake package cache: archive path -> {member -> content}.
    struct FakeCache(HashMap<PathBuf, HashMap<String, Vec<u8>>>);
    impl Extractor for FakeCache {
        fn extract(&self, a: &Path, m: &str) -> Result<Vec<u8>> {
            self.0
                .get(a)
                .and_then(|x| x.get(m))
                .cloned()
                .context("member not in archive")
        }
    }

    struct Fx {
        dir: PathBuf,
        target: PathBuf,
        cache: Vec<CachedPkg>,
        ex: FakeCache,
    }

    fn fixture(tag: &str, base: &str, ours: &str, theirs: &str) -> Fx {
        let dir = std::env::temp_dir().join(format!("ag-pn-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::create_dir_all(dir.join("tmp")).unwrap();
        let target = dir.join("etc/foo.conf");
        std::fs::write(&target, ours).unwrap();
        std::fs::write(dir.join("etc/foo.conf.pacnew"), theirs).unwrap();
        let old = dir.join("foo-1.0-1-x86_64.pkg.tar.zst");
        let member = target.to_string_lossy().trim_start_matches('/').to_string();
        let cache = vec![
            CachedPkg {
                path: old.clone(),
                name: "foo".into(),
                version: "1.0-1".into(),
            },
            CachedPkg {
                path: dir.join("foo-2.0-1-x86_64.pkg.tar.zst"),
                name: "foo".into(),
                version: "2.0-1".into(),
            },
        ];
        let ex = FakeCache([(old, [(member, base.as_bytes().to_vec())].into())].into());
        Fx {
            dir,
            target,
            cache,
            ex,
        }
    }

    fn run_merge(
        fx: &Fx,
        tool: Option<Tool>,
        writable: bool,
        answer: bool,
        runner: &mut MockRunner,
    ) -> (MergeOutcome, String) {
        let mut confirm = |_: &str| answer;
        let w = move |_: &Path| writable;
        let mut ctx = PacnewCtx {
            scan_dirs: vec![fx.dir.join("etc")],
            owner: &FakeOwner,
            extractor: &fx.ex,
            cache: fx.cache.clone(),
            runner,
            confirm: &mut confirm,
            tool,
            elevation: Elevation::Sudo,
            is_root: false,
            tmpdir: fx.dir.join("tmp"),
            writable: &w,
            now: 1234,
        };
        let mut out = Vec::new();
        let r = cmd_merge(&mut ctx, &fx.target, false, &mut out).unwrap();
        (r, String::from_utf8(out).unwrap())
    }

    const B: &str = "a\nb\nc\nd\ne\n";

    #[test]
    fn clean_merge_writes_with_backup() {
        let fx = fixture("clean", B, "A\nb\nc\nd\ne\n", "a\nb\nc\nd\nE\n");
        let mut r = MockRunner::default();
        let (o, text) = run_merge(&fx, None, true, true, &mut r);
        assert!(matches!(o, MergeOutcome::Installed { .. }), "{text}");
        assert_eq!(
            std::fs::read_to_string(&fx.target).unwrap(),
            "A\nb\nc\nd\nE\n"
        );
        assert_eq!(
            std::fs::read_to_string(fx.dir.join("etc/foo.conf.archguard-bak.1234")).unwrap(),
            "A\nb\nc\nd\ne\n"
        );
        assert!(!fx.dir.join("etc/foo.conf.pacnew").exists());
        assert!(text.contains("from foo 1.0-1"));
        assert!(r.calls.is_empty());
        std::fs::remove_dir_all(&fx.dir).unwrap();
    }

    #[test]
    fn declined_writes_nothing() {
        let fx = fixture("decline", B, "A\nb\nc\nd\ne\n", "a\nb\nc\nd\nE\n");
        let mut r = MockRunner::default();
        let (o, _) = run_merge(&fx, None, true, false, &mut r);
        assert_eq!(o, MergeOutcome::Declined);
        assert_eq!(
            std::fs::read_to_string(&fx.target).unwrap(),
            "A\nb\nc\nd\ne\n"
        );
        assert!(fx.dir.join("etc/foo.conf.pacnew").exists());
        std::fs::remove_dir_all(&fx.dir).unwrap();
    }

    #[test]
    fn unwritable_target_elevates_only_final_steps() {
        let fx = fixture("elev", B, "A\nb\nc\nd\ne\n", "a\nb\nc\nd\nE\n");
        let mut r = MockRunner::default();
        let (o, _) = run_merge(&fx, None, false, true, &mut r);
        assert!(matches!(o, MergeOutcome::Installed { .. }));
        let progs: Vec<_> = r.calls.iter().map(|c| c.args[0].as_str()).collect();
        assert_eq!(progs, ["cp", "install", "mv", "rm"]);
        assert!(r.calls.iter().all(|c| c.program == "sudo"));
        // Staged merged content was prepared by the user, not as root.
        let install = &r.calls[1];
        assert!(install
            .args
            .iter()
            .any(|a| a.ends_with("tmp/foo.conf.merged")));
        std::fs::remove_dir_all(&fx.dir).unwrap();
    }

    #[test]
    fn conflict_without_tool_leaves_markers() {
        let fx = fixture("conf", B, "a\nX\nc\nd\ne\n", "a\nY\nc\nd\ne\n");
        let mut r = MockRunner::default();
        let (o, text) = run_merge(&fx, None, true, true, &mut r);
        let MergeOutcome::LeftConflicts(p) = o else {
            panic!("{text}")
        };
        let s = std::fs::read_to_string(p).unwrap();
        assert!(s.contains("<<<<<<< current\nX\n"));
        assert!(text.contains("No merge tool"));
        assert_eq!(
            std::fs::read_to_string(&fx.target).unwrap(),
            "a\nX\nc\nd\ne\n"
        );
        std::fs::remove_dir_all(&fx.dir).unwrap();
    }

    #[test]
    fn conflict_with_tool_launches_3way() {
        let fx = fixture("tool", B, "a\nX\nc\nd\ne\n", "a\nY\nc\nd\ne\n");
        let mut r = MockRunner::default();
        // Mock tool "resolves" nothing (out == ours, no markers) -> accepted.
        let (o, _) = run_merge(&fx, Some(Tool::Kdiff3), true, true, &mut r);
        assert!(matches!(o, MergeOutcome::Installed { .. }));
        assert_eq!(r.calls[0].program, "kdiff3");
        assert_eq!(r.calls[0].args.len(), 5);
        assert!(r.calls[0].args[0].ends_with("foo.conf.base"));
        std::fs::remove_dir_all(&fx.dir).unwrap();
    }

    #[test]
    fn no_base_with_meld_is_two_way() {
        let fx = fixture("nobase", B, "x\n", "y\n");
        let fx = Fx {
            cache: vec![],
            ..fx
        };
        let mut r = MockRunner {
            exit_code: 1,
            ..Default::default()
        };
        let (o, text) = run_merge(&fx, Some(Tool::Meld), true, true, &mut r);
        assert_eq!(o, MergeOutcome::Declined);
        assert!(text.contains("no 3-way base"));
        assert_eq!(r.calls[0].args.len(), 2);
        std::fs::remove_dir_all(&fx.dir).unwrap();
    }

    #[test]
    fn no_base_no_tool_is_manual() {
        let fx = fixture("manual", B, "x\n", "y\n");
        let fx = Fx {
            cache: vec![],
            ..fx
        };
        let mut r = MockRunner::default();
        let (o, _) = run_merge(&fx, None, true, true, &mut r);
        assert_eq!(o, MergeOutcome::Manual);
        std::fs::remove_dir_all(&fx.dir).unwrap();
    }

    #[test]
    fn identical_just_removes_pacnew() {
        let fx = fixture("ident", B, "same\n", "same\n");
        let mut r = MockRunner::default();
        let (o, _) = run_merge(&fx, None, false, true, &mut r);
        assert_eq!(o, MergeOutcome::NothingToDo);
        assert_eq!(r.calls.len(), 1);
        assert_eq!(r.calls[0].args[0], "rm");
        std::fs::remove_dir_all(&fx.dir).unwrap();
    }

    #[test]
    fn list_and_diff() {
        let fx = fixture("list", B, "a\n", "b\n");
        let mut r = MockRunner::default();
        let mut c = |_: &str| true;
        let w = |_: &Path| true;
        let ctx = PacnewCtx {
            scan_dirs: vec![fx.dir.join("etc")],
            owner: &FakeOwner,
            extractor: &fx.ex,
            cache: vec![],
            runner: &mut r,
            confirm: &mut c,
            tool: None,
            elevation: Elevation::Run0,
            is_root: false,
            tmpdir: fx.dir.join("tmp"),
            writable: &w,
            now: 0,
        };
        let mut out = Vec::new();
        cmd_list(&ctx, &mut out).unwrap();
        assert!(String::from_utf8_lossy(&out).contains("foo.conf.pacnew"));
        let mut out = Vec::new();
        cmd_diff(&fx.dir.join("etc/foo.conf.pacnew"), &mut out).unwrap();
        let d = String::from_utf8(out).unwrap();
        assert!(d.contains("-a") && d.contains("+b"));
        std::fs::remove_dir_all(&fx.dir).unwrap();
    }

    #[test]
    fn tool_resolution() {
        let all = |_: &str| true;
        let only_kdiff = |b: &str| b == "kdiff3";
        assert_eq!(resolve_tool(MergeTool::Auto, &all), Some(Tool::Meld));
        assert_eq!(
            resolve_tool(MergeTool::Auto, &only_kdiff),
            Some(Tool::Kdiff3)
        );
        assert_eq!(resolve_tool(MergeTool::Meld, &only_kdiff), None);
        assert_eq!(resolve_tool(MergeTool::None, &all), None);
    }

    #[test]
    fn pair_resolution() {
        let (t, p) = resolve_pair(Path::new("/etc/a.conf")).unwrap();
        assert_eq!(
            (t.as_path(), p.as_path()),
            (Path::new("/etc/a.conf"), Path::new("/etc/a.conf.pacnew"))
        );
        let (t, _) = resolve_pair(Path::new("/etc/a.conf.pacnew")).unwrap();
        assert_eq!(t, Path::new("/etc/a.conf"));
        assert!(resolve_pair(Path::new("/etc/a.conf.pacsave")).is_err());
    }

    #[test]
    fn elevation_with_run0_and_root() {
        let fx = fixture("root", B, "same\n", "same\n");
        let mut r = MockRunner::default();
        let mut c = |_: &str| true;
        let w = |_: &Path| false;
        let mut ctx = PacnewCtx {
            scan_dirs: vec![],
            owner: &FakeOwner,
            extractor: &fx.ex,
            cache: vec![],
            runner: &mut r,
            confirm: &mut c,
            tool: None,
            elevation: Elevation::Run0,
            is_root: false,
            tmpdir: fx.dir.join("tmp"),
            writable: &w,
            now: 0,
        };
        assert_eq!(elevated(&ctx, "rm", vec![]).program, "run0");
        ctx.is_root = true;
        assert_eq!(elevated(&ctx, "rm", vec![]).program, "rm");
        std::fs::remove_dir_all(&fx.dir).unwrap();
    }
}
