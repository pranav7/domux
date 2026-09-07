//! Who is in the foreground of a pane, and where. `tcgetpgrp` on the PTY master gives the
//! foreground process group; its leader's name and working directory come from the OS.

use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundProcess {
    pub pid: u32,
    /// The executable's base name: `zsh`, `nvim`, `claude`.
    pub name: String,
}

pub trait ProcessInspector: Send + Sync {
    fn foreground(&self, pty_fd: RawFd) -> Option<ForegroundProcess>;
    fn cwd_of(&self, pid: u32) -> Option<PathBuf>;
}

pub struct RealInspector;

impl ProcessInspector for RealInspector {
    fn foreground(&self, pty_fd: RawFd) -> Option<ForegroundProcess> {
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
    let path = String::from_utf8_lossy(&buf[..n as usize]).into_owned();
    Some(
        std::path::Path::new(&path)
            .file_name()?
            .to_string_lossy()
            .into_owned(),
    )
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
    if n <= 0 {
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
    let s = String::from_utf8(bytes).ok()?;
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

#[cfg(target_os = "linux")]
fn process_name(pid: u32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let name = comm.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

#[cfg(target_os = "linux")]
fn process_cwd(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
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
    fn foreground(&self, _pty_fd: RawFd) -> Option<ForegroundProcess> {
        self.state.lock().unwrap().0.clone()
    }
    fn cwd_of(&self, _pid: u32) -> Option<PathBuf> {
        self.state.lock().unwrap().1.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::time::{Duration, Instant};

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
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let fd = pair.master.as_raw_fd().expect("master fd");
        let inspector = RealInspector;
        let deadline = Instant::now() + Duration::from_secs(5);
        let fg = loop {
            match inspector.foreground(fd) {
                Some(p) if p.name == "sleep" => break p,
                _ if Instant::now() > deadline => panic!("foreground never became sleep"),
                _ => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        assert_eq!(
            inspector.cwd_of(fg.pid).map(|p| p.canonicalize().unwrap()),
            Some(canonical)
        );
        child.kill().unwrap();
        let _ = child.wait();
    }

    #[test]
    fn cwd_of_self_is_the_current_dir() {
        let me = std::process::id();
        let cwd = RealInspector.cwd_of(me).expect("own cwd");
        assert_eq!(
            cwd.canonicalize().unwrap(),
            std::env::current_dir().unwrap().canonicalize().unwrap()
        );
    }

    #[test]
    fn fake_inspector_returns_what_it_was_told() {
        let fake = FakeInspector::default();
        assert_eq!(fake.foreground(0), None);
        fake.set(
            Some(ForegroundProcess {
                pid: 42,
                name: "nvim".into(),
            }),
            Some(PathBuf::from("/tmp")),
        );
        assert_eq!(fake.foreground(0).unwrap().name, "nvim");
        assert_eq!(fake.cwd_of(42), Some(PathBuf::from("/tmp")));
    }
}
