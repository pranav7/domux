//! Who is in the foreground of a pane, and where. `tcgetpgrp` on the PTY master gives the
//! foreground process group; its leader's name and working directory come from the OS.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!(
    "the process inspector has no `process_name` or `process_cwd` for this platform: domux \
     supports macOS and Linux, so add an arm for this target or build on one of those"
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundProcess {
    pub pid: u32,
    /// The executable's base name: `zsh`, `nvim`, `claude`.
    pub name: String,
}

pub trait ProcessInspector: Send + Sync {
    /// Who is in the foreground of the pane behind `pty_fd`. `None` for the descriptor
    /// means the pane has no PTY to ask - a fake one in a test - rather than a descriptor
    /// the caller invented, so the real inspector answers `None` and never signals a
    /// number that belongs to something else.
    fn foreground(&self, pty_fd: Option<RawFd>) -> Option<ForegroundProcess>;
    fn cwd_of(&self, pid: u32) -> Option<PathBuf>;

    /// Is this process still there? The default asks the OS for its working directory, which
    /// fails for a dead process on macOS and on Linux, so one question answers both platforms
    /// and no signal probe needs a platform arm of its own. The observer uses this to tell
    /// "the agent is running a tool" from "the agent is gone".
    fn is_alive(&self, pid: u32) -> bool {
        self.cwd_of(pid).is_some()
    }
}

pub struct RealInspector;

impl ProcessInspector for RealInspector {
    fn foreground(&self, pty_fd: Option<RawFd>) -> Option<ForegroundProcess> {
        let pty_fd = pty_fd?;
        // Safe: tcgetpgrp only reads; a bad fd returns -1.
        let pgid = unsafe { libc::tcgetpgrp(pty_fd) };
        if pgid <= 0 {
            return None;
        }
        let name = process_name(pgid as u32)?;
        Some(ForegroundProcess {
            pid: pgid as u32,
            name,
        })
    }

    fn cwd_of(&self, pid: u32) -> Option<PathBuf> {
        process_cwd(pid)
    }
}

/// The base name of an executable path held as raw bytes. The bytes may carry more than the
/// path, since a Linux `/proc/<pid>/cmdline` holds the whole argument vector separated by NUL,
/// so only the first entry is read. Absent when there is no base name, and absent when the
/// bytes are not UTF-8: a lossy conversion would invent a name no process has.
fn base_name(path: &[u8]) -> Option<String> {
    let first = path.split(|&b| b == 0).next().unwrap_or_default();
    // A login shell's argv[0] carries a leading `-`: that is the marker telling the shell to
    // read its profile, not part of its name. Every pane runs one, so this is the common case
    // and without it every border would read `-zsh`.
    let first = first.strip_prefix(b"-").unwrap_or(first);
    let name = Path::new(OsStr::from_bytes(first)).file_name()?;
    Some(name.to_str()?.to_string())
}

#[cfg(target_os = "macos")]
fn process_name(pid: u32) -> Option<String> {
    // argv[0] is the name the process was invoked as, which is the name its user knows it by
    // and what Linux already reads from `cmdline`. The executable path is a worse answer than
    // it looks: a versioned install resolves to a file named after its version, so asking the
    // path what Claude Code is called answers `2.1.263`. The path stays as the fallback,
    // because `KERN_PROCARGS2` refuses processes belonging to another user.
    if let Some(name) = process_argv0(pid).as_deref().and_then(base_name) {
        return Some(name);
    }
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // Safe: the buffer length is passed and proc_pidpath writes at most that many bytes.
    let n = unsafe {
        libc::proc_pidpath(
            pid as libc::c_int,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len() as u32,
        )
    };
    if n <= 0 {
        return None;
    }
    base_name(&buf[..n as usize])
}

/// The raw argv[0] of a process, from the kernel's copy of its argument area.
#[cfg(target_os = "macos")]
fn process_argv0(pid: u32) -> Option<Vec<u8>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    let mut len: libc::size_t = 0;
    // Safe: a null buffer asks sysctl for the size it would write and nothing more.
    let sized = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if sized != 0 || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    // Safe: the buffer holds `len` bytes and sysctl is told that length, so it writes no more.
    let filled = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if filled != 0 {
        return None;
    }
    buf.truncate(len);
    argv0_from_procargs2(&buf)
}

/// argv[0] out of a `KERN_PROCARGS2` buffer, which holds argc as a native-endian 32-bit int,
/// then the executable path, then NUL padding to an alignment boundary, then the arguments
/// separated by NULs. Absent when the buffer is truncated or the process has no argument -
/// a short buffer must not be read as an empty name.
#[cfg(any(target_os = "macos", test))]
fn argv0_from_procargs2(buf: &[u8]) -> Option<Vec<u8>> {
    const ARGC: usize = std::mem::size_of::<u32>();
    let argc = u32::from_ne_bytes(buf.get(..ARGC)?.try_into().ok()?);
    if argc == 0 {
        return None;
    }
    let rest = buf.get(ARGC..)?;
    // Step over the executable path, then over the NULs padding it out.
    let path_end = rest.iter().position(|&b| b == 0)?;
    let args = &rest[path_end..];
    let start = args.iter().position(|&b| b != 0)?;
    let arg = &args[start..];
    let end = arg.iter().position(|&b| b == 0)?;
    Some(arg[..end].to_vec())
}

#[cfg(target_os = "macos")]
fn process_cwd(pid: u32) -> Option<PathBuf> {
    // Safe: proc_pidinfo fills a zeroed struct of the size we pass.
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    let n = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        )
    };
    // proc_pidinfo returns the number of bytes it wrote. Anything short of the whole struct is
    // a partial fill, which is not the fact we asked for.
    if n != size {
        return None;
    }
    // libc declares `vip_path` in 32-byte chunks, so flatten before reading to the first NUL.
    let bytes: Vec<u8> = info
        .pvi_cdir
        .vip_path
        .iter()
        .flatten()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    if bytes.is_empty() {
        return None;
    }
    Some(PathBuf::from(OsStr::from_bytes(&bytes)))
}

#[cfg(target_os = "linux")]
fn process_name(pid: u32) -> Option<String> {
    // The kernel truncates `comm` to 15 bytes and a process can rewrite it through `prctl`, so
    // it is a self-declared short name where macOS observes the executable. Read the command
    // line instead and take the base name of its first entry.
    linux_process_name_from_reads(std::fs::read(format!("/proc/{pid}/cmdline")), || {
        std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()
    })
}

/// Resolves Linux process-name bytes while keeping failed reads distinct from empty cmdlines.
#[cfg(any(target_os = "linux", test))]
fn linux_process_name_from_reads(
    cmdline: std::io::Result<Vec<u8>>,
    read_comm: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let cmdline = cmdline.ok()?;
    if !cmdline.is_empty() {
        return base_name(&cmdline);
    }
    // An empty command line means a kernel thread, where `comm` is the only name there is.
    let comm = read_comm()?;
    let name = comm.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

#[cfg(target_os = "linux")]
fn process_cwd(pid: u32) -> Option<PathBuf> {
    let cwd = std::fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
    // The kernel renders a removed directory as `/path (deleted)`, which is a plausible looking
    // path that never existed as written. The fact did not arrive, so report absence rather
    // than a path no one can enter.
    if cwd.as_os_str().as_bytes().ends_with(b" (deleted)") {
        return None;
    }
    Some(cwd)
}

/// One entry of the fake's process table: who is in the foreground of a pane, and where that
/// process is.
type Entry = (Option<ForegroundProcess>, Option<PathBuf>);

/// The test double: a process table. `set` answers for every pane; `set_for` answers for one
/// pane's PTY file descriptor and wins over `set`, so two panes can hold two agents.
#[derive(Default)]
pub struct FakeInspector {
    all: Mutex<Entry>,
    per_fd: Mutex<HashMap<RawFd, Entry>>,
    dead: Mutex<HashSet<u32>>,
}

impl FakeInspector {
    pub fn set(&self, foreground: Option<ForegroundProcess>, cwd: Option<PathBuf>) {
        *self.all.lock().unwrap() = (foreground, cwd);
    }

    /// What the pane behind `fd` is running, and where.
    pub fn set_for(&self, fd: RawFd, foreground: Option<ForegroundProcess>, cwd: Option<PathBuf>) {
        self.per_fd.lock().unwrap().insert(fd, (foreground, cwd));
    }

    /// The process is gone; `is_alive` says so from now on, and it has no working directory.
    pub fn set_dead(&self, pid: u32) {
        self.dead.lock().unwrap().insert(pid);
    }
}

impl ProcessInspector for FakeInspector {
    fn foreground(&self, pty_fd: Option<RawFd>) -> Option<ForegroundProcess> {
        if let Some(fd) = pty_fd {
            if let Some((foreground, _)) = self.per_fd.lock().unwrap().get(&fd) {
                return foreground.clone();
            }
        }
        self.all.lock().unwrap().0.clone()
    }

    fn cwd_of(&self, pid: u32) -> Option<PathBuf> {
        if self.dead.lock().unwrap().contains(&pid) {
            return None;
        }
        // A per-pane entry answers for the process it put in the foreground, and only for
        // that one: a table keyed by descriptor still has to answer a question about a pid.
        let per_fd = self.per_fd.lock().unwrap();
        let named = per_fd
            .values()
            .find_map(|(foreground, cwd)| match foreground {
                Some(f) if f.pid == pid => cwd.clone(),
                _ => None,
            });
        drop(per_fd);
        named.or_else(|| self.all.lock().unwrap().1.clone())
    }

    /// The fake keeps its own list rather than taking the default: a test sets a foreground
    /// process without a working directory, and under the default every such process would
    /// read as dead.
    fn is_alive(&self, pid: u32) -> bool {
        !self.dead.lock().unwrap().contains(&pid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_pty::{native_pty_system, Child, CommandBuilder, PtySize};
    use std::time::{Duration, Instant};

    /// Kills and reaps the child however the test ends, panic included.
    struct ChildGuard(Box<dyn Child + Send + Sync>);

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn real_inspector_names_the_foreground_process_and_its_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: 5,
                cols: 40,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = CommandBuilder::new("sh");
        cmd.args(["-c", "exec sleep 30"]);
        cmd.cwd(&canonical);
        let _child = ChildGuard(pair.slave.spawn_command(cmd).unwrap());
        drop(pair.slave);
        let fd = pair.master.as_raw_fd().expect("master fd");
        let inspector = RealInspector;
        let deadline = Instant::now() + Duration::from_secs(5);
        let fg = loop {
            match inspector.foreground(Some(fd)) {
                Some(p) if p.name == "sleep" => break p,
                _ if Instant::now() > deadline => panic!("foreground never became sleep"),
                _ => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        assert_eq!(
            inspector.cwd_of(fg.pid).map(|p| p.canonicalize().unwrap()),
            Some(canonical)
        );
    }

    /// A pane with no PTY - a fake one under the harness - has no descriptor to ask about.
    #[test]
    fn real_inspector_foreground_is_absent_without_a_descriptor() {
        assert_eq!(RealInspector.foreground(None), None);
    }

    #[test]
    fn cwd_of_self_is_the_current_dir() {
        let me = std::process::id();
        let cwd = RealInspector.cwd_of(me).expect("own cwd");
        // The OS reports where a process is, so the answer is an absolute path. Canonicalizing
        // alone would accept a relative stub such as `.`.
        assert!(cwd.is_absolute(), "cwd must be absolute, got {cwd:?}");
        assert_eq!(
            cwd.canonicalize().unwrap(),
            std::env::current_dir().unwrap().canonicalize().unwrap()
        );
    }

    /// The name a process is known by is argv[0], not the file behind it. The two differ
    /// whenever a launcher sets one - and a versioned install makes the file name useless on
    /// its own: `~/.local/bin/claude` resolves to `.../claude/versions/2.1.263`, so asking the
    /// executable path what Claude Code is called answers `2.1.263`.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_process_is_named_by_argv0_not_by_the_file_behind_it() {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: 5,
                cols: 40,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        // `exec -a` sets argv[0] apart from the path, which is what a launcher does. The two
        // answers here are `claude` and `sleep`, so the assertion cannot pass by accident.
        // macOS `/bin/sh` is bash and has `exec -a`; this test is macOS-only, and Linux reads
        // argv[0] from `cmdline` already.
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.args(["-c", "exec -a claude /bin/sleep 30"]);
        let _child = ChildGuard(pair.slave.spawn_command(cmd).unwrap());
        drop(pair.slave);
        let fd = pair.master.as_raw_fd().expect("master fd");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match RealInspector.foreground(Some(fd)) {
                Some(p) if p.name == "claude" => break,
                other => {
                    let last = other.map(|p| p.name);
                    assert!(
                        Instant::now() <= deadline,
                        "foreground never became claude, last {last:?}"
                    );
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }

    #[test]
    fn a_login_shells_argv0_names_the_shell_without_its_marker() {
        assert_eq!(base_name(b"-zsh").as_deref(), Some("zsh"));
        assert_eq!(base_name(b"-bash").as_deref(), Some("bash"));
        // A non-login invocation is a path and keeps every part of its base name.
        assert_eq!(base_name(b"/bin/zsh").as_deref(), Some("zsh"));
        assert_eq!(base_name(b"my-prog").as_deref(), Some("my-prog"));
        // The marker alone is not a name.
        assert_eq!(base_name(b"-"), None);
    }

    /// The buffer is argc, the executable path, NUL padding, then the arguments.
    #[test]
    fn argv0_is_read_from_the_kernels_argument_area() {
        let mut buf = 2u32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/usr/local/share/app/versions/9.9.9\0\0\0");
        buf.extend_from_slice(b"claude\0--resume\0");
        assert_eq!(argv0_from_procargs2(&buf).as_deref(), Some(&b"claude"[..]));
    }

    #[test]
    fn a_truncated_argument_area_yields_no_name() {
        // Too short to hold argc at all.
        assert_eq!(argv0_from_procargs2(&[0, 0]), None);
        // argc says there are no arguments.
        let mut none = 0u32.to_ne_bytes().to_vec();
        none.extend_from_slice(b"/bin/sh\0\0sh\0");
        assert_eq!(argv0_from_procargs2(&none), None);
        // The path is there but the argument after it was cut off, which is not an empty
        // name: reading it as one would title a pane with nothing at all.
        let mut cut = 1u32.to_ne_bytes().to_vec();
        cut.extend_from_slice(b"/bin/sh\0\0sh");
        assert_eq!(argv0_from_procargs2(&cut), None);
    }

    #[test]
    fn foreground_is_absent_for_a_bad_fd() {
        assert_eq!(RealInspector.foreground(Some(-1)), None);
    }

    #[test]
    fn foreground_is_absent_for_an_fd_that_is_not_a_terminal() {
        use std::os::unix::io::AsRawFd;
        let file = tempfile::NamedTempFile::new().unwrap();
        assert_eq!(
            RealInspector.foreground(Some(file.as_file().as_raw_fd())),
            None
        );
    }

    #[test]
    fn cwd_is_absent_for_a_dead_pid() {
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert_eq!(RealInspector.cwd_of(pid), None);
    }

    #[test]
    fn cwd_is_absent_for_pid_zero() {
        assert_eq!(RealInspector.cwd_of(0), None);
    }

    #[test]
    fn base_name_is_the_last_component_of_the_path() {
        assert_eq!(base_name(b"/usr/bin/sleep").as_deref(), Some("sleep"));
    }

    #[test]
    fn base_name_reads_only_the_first_entry_of_an_argument_vector() {
        assert_eq!(
            base_name(b"/usr/bin/docker-credential-osxkeychain\0get\0").as_deref(),
            Some("docker-credential-osxkeychain")
        );
    }

    #[test]
    fn base_name_is_absent_for_bytes_that_are_not_utf8() {
        assert_eq!(base_name(b"/usr/bin/sl\xffeep"), None);
    }

    #[test]
    fn base_name_is_absent_for_a_path_with_no_last_component() {
        assert_eq!(base_name(b""), None);
        assert_eq!(base_name(b"/"), None);
    }

    #[test]
    fn linux_name_is_absent_when_cmdline_read_fails() {
        let result = linux_process_name_from_reads(
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            || panic!("must not read comm after a failed cmdline read"),
        );
        assert_eq!(result, None);
    }

    #[test]
    fn linux_name_uses_comm_when_cmdline_is_empty() {
        let result = linux_process_name_from_reads(Ok(Vec::new()), || Some("kworker/0:1\n".into()));
        assert_eq!(result.as_deref(), Some("kworker/0:1"));
    }

    #[test]
    fn linux_name_is_absent_when_cmdline_starts_with_nul() {
        let result = linux_process_name_from_reads(Ok(b"\0argument\0".to_vec()), || {
            panic!("must not read comm for a nonempty cmdline")
        });
        assert_eq!(result, None);
    }

    #[test]
    fn child_guard_reaps_the_child_during_unwinding() {
        let child = std::process::Command::new("sh")
            .args(["-c", "exec sleep 30"])
            .spawn()
            .unwrap();
        let pid = child.id() as libc::pid_t;
        let result = std::panic::catch_unwind(|| {
            let _child = ChildGuard(Box::new(child));
            panic!("unwind while child guard owns the child");
        });
        assert!(result.is_err());
        // Safe: kill with signal zero does not send a signal or mutate the process.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[test]
    fn fake_inspector_returns_what_it_was_told() {
        let fake = FakeInspector::default();
        assert_eq!(fake.foreground(None), None);
        fake.set(
            Some(ForegroundProcess {
                pid: 42,
                name: "nvim".into(),
            }),
            Some(PathBuf::from("/tmp")),
        );
        assert_eq!(fake.foreground(None).unwrap().name, "nvim");
        assert_eq!(fake.cwd_of(42), Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn fake_inspector_answers_per_descriptor_before_it_answers_for_every_pane() {
        let fake = FakeInspector::default();
        fake.set(
            Some(ForegroundProcess {
                pid: 1,
                name: "sh".into(),
            }),
            None,
        );
        fake.set_for(
            101,
            Some(ForegroundProcess {
                pid: 5000,
                name: "claude".into(),
            }),
            Some(PathBuf::from("/work")),
        );
        // The descriptor with an entry gets it; every other pane still gets `set`.
        assert_eq!(fake.foreground(Some(101)).unwrap().name, "claude");
        assert_eq!(fake.foreground(Some(102)).unwrap().name, "sh");
        assert_eq!(fake.foreground(None).unwrap().name, "sh");
        // The entry's working directory answers for the process it named, and for no other.
        assert_eq!(fake.cwd_of(5000), Some(PathBuf::from("/work")));
        assert_eq!(fake.cwd_of(1), None);
        // A descriptor can also be told nothing is in front of it.
        fake.set_for(101, None, None);
        assert_eq!(fake.foreground(Some(101)), None);
    }

    #[test]
    fn fake_inspector_calls_a_killed_process_dead_and_leaves_the_rest_alive() {
        let fake = FakeInspector::default();
        fake.set_for(
            101,
            Some(ForegroundProcess {
                pid: 5000,
                name: "claude".into(),
            }),
            Some(PathBuf::from("/work")),
        );
        assert!(fake.is_alive(5000));
        fake.set_dead(5000);
        assert!(!fake.is_alive(5000));
        assert!(fake.is_alive(5001));
        // A dead process has no working directory either.
        assert_eq!(fake.cwd_of(5000), None);
        // And the foreground table is untouched: what a pane is running is a separate
        // question from whether one pid is still there.
        assert_eq!(fake.foreground(Some(101)).unwrap().pid, 5000);
    }

    /// The trait's default, which the real inspector takes and the fake replaces.
    #[test]
    fn is_alive_by_default_is_whether_the_os_reports_a_working_directory() {
        struct OnlyCwd(Option<PathBuf>);
        impl ProcessInspector for OnlyCwd {
            fn foreground(&self, _pty_fd: Option<RawFd>) -> Option<ForegroundProcess> {
                None
            }
            fn cwd_of(&self, _pid: u32) -> Option<PathBuf> {
                self.0.clone()
            }
        }
        assert!(OnlyCwd(Some(PathBuf::from("/tmp"))).is_alive(7));
        assert!(!OnlyCwd(None).is_alive(7));
    }

    #[test]
    fn real_inspector_calls_this_process_alive_and_a_reaped_child_gone() {
        assert!(RealInspector.is_alive(std::process::id()));
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(!RealInspector.is_alive(pid));
    }
}
