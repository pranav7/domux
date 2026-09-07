//! Who is in the foreground of a pane, and where. `tcgetpgrp` on the PTY master gives the
//! foreground process group; its leader's name and working directory come from the OS.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!(
    "the process inspector has no `process_name` or `process_cwd` for this platform: domux2 \
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
    let name = Path::new(OsStr::from_bytes(first)).file_name()?;
    Some(name.to_str()?.to_string())
}

#[cfg(target_os = "macos")]
fn process_name(pid: u32) -> Option<String> {
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

/// The test double: answers whatever the test set, for every pane.
#[derive(Default)]
pub struct FakeInspector {
    state: Mutex<(Option<ForegroundProcess>, Option<PathBuf>)>,
}

impl FakeInspector {
    pub fn set(&self, foreground: Option<ForegroundProcess>, cwd: Option<PathBuf>) {
        *self.state.lock().unwrap() = (foreground, cwd);
    }
}

impl ProcessInspector for FakeInspector {
    fn foreground(&self, _pty_fd: Option<RawFd>) -> Option<ForegroundProcess> {
        self.state.lock().unwrap().0.clone()
    }
    fn cwd_of(&self, _pid: u32) -> Option<PathBuf> {
        self.state.lock().unwrap().1.clone()
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
    /// The real inspector must say so rather than guess, and never signal or read a number
    /// that belongs to something else.
    #[test]
    fn the_real_inspector_reports_nothing_when_there_is_no_descriptor() {
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
}
