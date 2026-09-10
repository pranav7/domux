//! Running a program that is not a pane's own: the one seam every such call goes through.
//!
//! Stay awake starts a child that holds the machine awake, kills it again, and on macOS asks
//! `launchctl` and `pmset` for the lid. None of that may happen on a test machine, so the
//! calls go through a trait and the fake records them instead. The Notifier's `osascript`,
//! `notify-send` and sound players will use the same seam.
//!
//! One-shot commands with a deadline belong to `subprocess::output_within`, which is what a
//! fact provider uses. This trait is for the calls whose result is an effect rather than
//! output: the caller wants the program run, not what it printed.

use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

/// One call, as the fake recorded it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Call {
    pub program: String,
    pub args: Vec<String>,
}

pub trait CommandRunner: Send + Sync {
    /// Runs to completion and throws the output away. A non-zero status is an error whose
    /// message names the program and the first line of its stderr (design principle 9).
    fn run(&self, program: &str, args: &[String]) -> Result<(), String>;

    /// Runs to completion and answers what it printed, trimmed.
    fn output(&self, program: &str, args: &[String]) -> Result<String, String>;

    /// Starts a child that outlives the call, and answers its process id. The hold that keeps
    /// the machine awake is one of these: it lasts as long as the child does.
    fn spawn_detached(&self, program: &str, args: &[String]) -> Result<u32, String>;

    /// Ends a process this runner started, or one an earlier server left behind.
    fn kill(&self, pid: u32) -> Result<(), String>;

    /// Where the program is, when it is on PATH at all.
    fn which(&self, program: &str) -> Option<PathBuf>;
}

/// The real one. It keeps each child it started so that killing one can also reap it: a child
/// nobody waits for stays in the process table as a zombie until the server exits.
#[derive(Default)]
pub struct RealRunner {
    children: Mutex<HashMap<u32, std::process::Child>>,
}

impl RealRunner {
    /// What a finished command comes back as: its output when it succeeded, and the program,
    /// its status and the first line of its complaint when it did not.
    fn finish(program: &str, out: std::io::Result<std::process::Output>) -> Result<String, String> {
        let out = out.map_err(|e| format!("could not run {program}: {e}"))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let first = stderr.lines().next().unwrap_or("").trim();
            return Err(format!(
                "{program} exited {}: {first}",
                out.status.code().unwrap_or(-1)
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn collect(program: &str, args: &[String]) -> Result<String, String> {
        let out = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .output();
        RealRunner::finish(program, out)
    }
}

impl CommandRunner for RealRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<(), String> {
        RealRunner::collect(program, args).map(|_| ())
    }

    fn output(&self, program: &str, args: &[String]) -> Result<String, String> {
        RealRunner::collect(program, args)
    }

    fn spawn_detached(&self, program: &str, args: &[String]) -> Result<u32, String> {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Its own session, so a signal sent to domux's process group stops at domux. The hold
        // is released deliberately, on the way out, and not by whatever killed the server.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command
            .spawn()
            .map_err(|e| format!("could not start {program}: {e}"))?;
        let pid = child.id();
        self.children.lock().unwrap().insert(pid, child);
        Ok(pid)
    }

    fn kill(&self, pid: u32) -> Result<(), String> {
        // Safe: `kill` only signals, and a pid that is gone answers with an error rather than
        // reaching something else. Term rather than kill, so the program can put back
        // whatever it changed.
        let sent = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
        if sent == -1 {
            let e = std::io::Error::last_os_error();
            // Already gone is the outcome the caller wanted, so it is not a failure.
            if e.raw_os_error() != Some(libc::ESRCH) {
                return Err(format!("could not stop process {pid}: {e}"));
            }
        }
        if let Some(mut child) = self.children.lock().unwrap().remove(&pid) {
            let _ = child.wait();
        }
        Ok(())
    }

    fn which(&self, program: &str) -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|dir| dir.join(program))
            .find(|candidate| is_executable(candidate))
    }
}

fn call_of(program: &str, args: &[&str]) -> Call {
    Call {
        program: program.to_string(),
        args: args.iter().map(|a| a.to_string()).collect(),
    }
}

/// A file that exists and that this user may run. `which(1)` asks the same two questions.
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Records what it was asked to do and does none of it. Every test uses this one, so no test
/// on any machine starts a hold, signals a process or asks for a password.
#[derive(Default)]
pub struct FakeRunner {
    calls: Mutex<Vec<Call>>,
    on_path: Mutex<Vec<String>>,
    answers: Mutex<HashMap<Call, String>>,
    failures: Mutex<HashMap<Call, String>>,
    /// Refusals matched on the front of the argument list, for a call whose last argument is
    /// a name only the code under test knows, such as a temporary file's.
    prefix_failures: Mutex<Vec<(String, Vec<String>, String)>>,
    next_pid: AtomicU32,
}

impl FakeRunner {
    /// The pid the first `spawn_detached` answers. Distinctive, so a test that finds it in a
    /// message knows where it came from.
    pub const FIRST_PID: u32 = 4242;

    /// What this exact call prints. A call with no answer here fails, so a test that meant to
    /// set one up finds out rather than reading an empty string as a real answer.
    pub fn answer(&self, program: &str, args: &[&str], out: &str) {
        self.answers
            .lock()
            .unwrap()
            .insert(call_of(program, args), out.to_string());
    }

    /// This exact call fails with this reason, so the path a reader reaches when sudo says
    /// no can be tested without a machine that says no.
    pub fn fail(&self, program: &str, args: &[&str], reason: &str) {
        self.failures
            .lock()
            .unwrap()
            .insert(call_of(program, args), reason.to_string());
    }

    /// As `fail`, for a call whose arguments this test cannot spell in full: every call to
    /// `program` whose arguments start with `args` fails with `reason`.
    pub fn fail_starting_with(&self, program: &str, args: &[&str], reason: &str) {
        self.prefix_failures.lock().unwrap().push((
            program.to_string(),
            args.iter().map(|a| a.to_string()).collect(),
            reason.to_string(),
        ));
    }

    /// This program is installed, as far as `which` is concerned.
    pub fn on_path(&self, program: &str) {
        self.on_path.lock().unwrap().push(program.to_string());
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub fn calls_to(&self, program: &str) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|c| c.program == program)
            .collect()
    }

    pub fn ran(&self, program: &str, args: &[&str]) -> bool {
        self.calls()
            .iter()
            .any(|c| c.program == program && c.args == args)
    }

    /// Records the call, and answers the reason it was told to refuse it with.
    fn record(&self, program: &str, args: &[String]) -> Result<(), String> {
        let call = Call {
            program: program.to_string(),
            args: args.to_vec(),
        };
        let refused = self
            .failures
            .lock()
            .unwrap()
            .get(&call)
            .cloned()
            .or_else(|| {
                self.prefix_failures
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(p, prefix, _)| *p == call.program && call.args.starts_with(prefix))
                    .map(|(_, _, reason)| reason.clone())
            });
        self.calls.lock().unwrap().push(call);
        match refused {
            Some(reason) => Err(reason),
            None => Ok(()),
        }
    }
}

impl CommandRunner for FakeRunner {
    fn run(&self, program: &str, args: &[String]) -> Result<(), String> {
        self.record(program, args)
    }

    fn output(&self, program: &str, args: &[String]) -> Result<String, String> {
        self.record(program, args)?;
        let call = Call {
            program: program.to_string(),
            args: args.to_vec(),
        };
        self.answers
            .lock()
            .unwrap()
            .get(&call)
            .cloned()
            .ok_or_else(|| format!("the fake was not told what {program} answers here"))
    }

    fn spawn_detached(&self, program: &str, args: &[String]) -> Result<u32, String> {
        self.record(program, args)?;
        let n = self.next_pid.fetch_add(1, Ordering::SeqCst);
        Ok(FakeRunner::FIRST_PID + n)
    }

    fn kill(&self, pid: u32) -> Result<(), String> {
        self.record("kill", &[pid.to_string()])
    }

    fn which(&self, program: &str) -> Option<PathBuf> {
        self.on_path
            .lock()
            .unwrap()
            .iter()
            .any(|p| p == program)
            .then(|| PathBuf::from("/usr/bin").join(program))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fake_records_every_call_in_order() {
        let fake = FakeRunner::default();
        fake.run("launchctl", &["load".into()]).unwrap();
        fake.spawn_detached("caffeinate", &["-dimsu".into()])
            .unwrap();
        let calls = fake.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].program, "launchctl");
        assert_eq!(calls[1].args, vec!["-dimsu".to_string()]);
        assert!(fake.ran("caffeinate", &["-dimsu"]));
        assert!(!fake.ran("caffeinate", &["-di"]));
    }

    #[test]
    fn a_program_the_fake_was_not_told_about_is_not_on_its_path() {
        let fake = FakeRunner::default();
        assert!(fake.which("systemd-inhibit").is_none());
        fake.on_path("systemd-inhibit");
        assert!(fake.which("systemd-inhibit").is_some());
    }

    #[test]
    fn each_spawn_answers_a_new_process_id() {
        let fake = FakeRunner::default();
        assert_eq!(fake.spawn_detached("caffeinate", &[]).unwrap(), 4242);
        assert_eq!(fake.spawn_detached("caffeinate", &[]).unwrap(), 4243);
    }

    #[test]
    fn the_fake_records_a_kill_as_a_call_like_any_other() {
        let fake = FakeRunner::default();
        let pid = fake.spawn_detached("caffeinate", &[]).unwrap();
        fake.kill(pid).unwrap();
        assert!(fake.ran("kill", &["4242"]), "{:?}", fake.calls());
    }

    #[test]
    fn the_fake_answers_what_it_was_told_to_and_nothing_for_anything_else() {
        let fake = FakeRunner::default();
        fake.answer("ps", &["-o", "comm=", "-p", "4242"], "caffeinate");
        assert_eq!(
            fake.output(
                "ps",
                &["-o".into(), "comm=".into(), "-p".into(), "4242".into()]
            ),
            Ok("caffeinate".to_string())
        );
        assert!(fake.output("ps", &["-p".into(), "17".into()]).is_err());
    }

    #[test]
    fn a_call_the_fake_was_told_to_refuse_comes_back_with_that_reason() {
        let fake = FakeRunner::default();
        fake.fail("sudo", &["-n", "pmset"], "sudo: a password is required");
        assert_eq!(
            fake.run("sudo", &["-n".into(), "pmset".into()]),
            Err("sudo: a password is required".to_string())
        );
        assert!(
            fake.ran("sudo", &["-n", "pmset"]),
            "a refused call still happened"
        );
        assert!(fake.run("sudo", &["-n".into(), "launchctl".into()]).is_ok());
    }

    #[test]
    fn output_comes_back_trimmed() {
        let out = RealRunner::default()
            .output("sh", &["-c".into(), "echo '  spaced  '".into()])
            .unwrap();
        assert_eq!(out, "spaced");
    }

    #[test]
    fn a_command_that_fails_names_the_program_and_the_first_line_of_its_complaint() {
        let e = RealRunner::default()
            .run("sh", &["-c".into(), "echo nope 1>&2; exit 3".into()])
            .unwrap_err();
        assert_eq!(e, "sh exited 3: nope");
    }

    #[test]
    fn a_program_that_is_not_there_is_not_on_the_path() {
        let runner = RealRunner::default();
        assert!(runner.which("sh").is_some());
        assert!(runner.which("domux-no-such-program").is_none());
    }
}
