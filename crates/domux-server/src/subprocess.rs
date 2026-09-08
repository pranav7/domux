//! Running one subprocess with a bound on how long it may take.
//!
//! `Command::output` waits for as long as the child cares to take, and a fact provider that
//! waits forever is worse than one that fails: it never sends its answer, so the registry
//! keeps that key in flight for the life of the server (`FactRegistry::due` skips a key that
//! is in flight) and that workspace's fact is frozen until a restart. The blocking task it
//! runs on is parked for good as well. Every provider that shells out goes through here, so
//! there is one bound and one place to correct it.

use std::io::{self, Read};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// How often to look at a child that has not finished. Short enough that the answer is not
/// held after it arrives, long enough that waiting costs nothing.
const POLL: Duration = Duration::from_millis(10);

/// How long to wait for the last bytes of a pipe once the child has exited. The child is
/// gone, so its reader is at end of file and only has to be scheduled; this is a floor under
/// the deadline for a command that answered just as the limit ran out, not more time for the
/// work.
const DRAIN: Duration = Duration::from_millis(500);

/// Runs `command` and returns what it printed, or an error of kind `TimedOut` when it is
/// still running after `limit`, in which case the child is killed and reaped.
///
/// The caller builds the whole command (arguments, working directory) and this sets the
/// three standard streams: stdin is closed, so a command that asks a question gets an answer
/// rather than blocking on a terminal nobody is at, and both output streams are pipes.
pub fn output_within(mut command: Command, limit: Duration) -> io::Result<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Both pipes are read on their own threads from the moment the child starts. A pipe
    // holds about 64KB and then blocks the writer until something reads it, so waiting for
    // the child to exit before reading, or reading one pipe to its end and then the other,
    // deadlocks on exactly the case this bound exists for: a command printing a long error.
    // The child never exits, the deadline never comes (nothing is polling it by then), and
    // the hang is back. Draining concurrently is what makes the bound real.
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait()? {
            // Output that could not be collected is a failure, not an empty answer. A child
            // that exits at once but leaves a grandchild holding the pipe cannot be read to
            // the end, and an `Output` saying "it worked and printed nothing" would hand the
            // caller a `gh` answer that never arrived (principle 4). The bound is what stops
            // the wait; saying so is what keeps the result honest.
            let (Some(stdout), Some(stderr)) =
                (collected(&stdout, deadline), collected(&stderr, deadline))
            else {
                return Err(timed_out(limit));
            };
            return Ok(Output {
                status,
                stdout,
                stderr,
            });
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            // The child has been signalled, so this returns at once. Without it the process
            // stays a zombie for the life of the server.
            let _ = child.wait();
            return Err(timed_out(limit));
        }
        thread::sleep(POLL);
    }
}

/// The one reason a bounded command gives when it ran out of time, whether it was the command
/// that would not finish or its output that could not be read.
fn timed_out(limit: Duration) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("no answer within {} seconds", limit.as_secs()),
    )
}

/// Reads one pipe to its end on its own thread and sends what it read.
///
/// A read error gives what arrived before it rather than a failure: the exit status is what
/// says whether the command worked, and half an error message is still worth printing.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf);
        }
        let _ = tx.send(buf);
    });
    rx
}

/// What one pipe held, or `None` when it could not be read by the deadline.
///
/// The child has exited by the time this is called, so the reader is normally already done.
/// It is not done when the child left a grandchild holding the pipe open, and waiting for that
/// grandchild is the hang all over again: the wait ends, and the caller is told it ended
/// rather than handed an empty answer that reads like a command with nothing to say.
fn collected(rx: &mpsc::Receiver<Vec<u8>>, deadline: Instant) -> Option<Vec<u8>> {
    let wait = (deadline + DRAIN).saturating_duration_since(Instant::now());
    rx.recv_timeout(wait).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::finishes_within;

    fn sh(script: &str) -> Command {
        let mut c = Command::new("/bin/sh");
        c.arg("-c").arg(script);
        c
    }

    #[test]
    fn a_command_that_finishes_gives_back_both_its_streams_and_its_status() {
        let out = output_within(
            sh("printf hello; printf trouble >&2; exit 3"),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hello");
        assert_eq!(String::from_utf8_lossy(&out.stderr), "trouble");
        assert_eq!(out.status.code(), Some(3));
    }

    /// The naive bound - spawn with pipes, poll `try_wait`, read afterwards - passes every
    /// small test and hangs the first time a command prints more than a pipe holds, which is
    /// the day `gh` prints a long authentication error. 200KB is comfortably past the 64KB
    /// buffer on both platforms domux runs on.
    ///
    /// The failure this guards against is a hang, and a test binary cannot report "this did
    /// not finish", so the call is bounded from the outside: without `finishes_within` a lost
    /// drain would stall the whole suite silently instead of failing this test.
    #[test]
    fn a_command_that_writes_more_than_a_pipe_holds_still_finishes_and_keeps_every_byte() {
        let out = finishes_within(
            Duration::from_secs(15),
            "output_within on a command that writes 200KB",
            || {
                output_within(
                    sh(
                        r#"dd if=/dev/zero bs=1000 count=200 2>/dev/null | tr '\0' 'o'
                          dd if=/dev/zero bs=1000 count=200 2>/dev/null | tr '\0' 'e' >&2"#,
                    ),
                    Duration::from_secs(5),
                )
            },
        )
        .unwrap();
        assert_eq!(out.stdout.len(), 200_000, "every byte of stdout");
        assert_eq!(out.stderr.len(), 200_000, "and every byte of stderr");
        assert!(out.status.success());
    }

    /// Bounded from the outside for the same reason: if the deadline check is lost this call
    /// never returns, which is not a failing test, it is a stuck one.
    #[test]
    fn a_command_that_never_answers_is_stopped_at_the_limit() {
        let started = Instant::now();
        let err = finishes_within(
            Duration::from_secs(5),
            "output_within on a command that never answers",
            || output_within(sh("sleep 30"), Duration::from_millis(300)).unwrap_err(),
        );
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");
        assert!(
            err.to_string().contains("no answer within"),
            "the reason says what happened: {err}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the limit is what ended it, not the command: {:?}",
            started.elapsed()
        );
    }

    /// A child that prints, exits at once, and leaves a grandchild holding the pipe. The
    /// reader never reaches end of file, so the bytes cannot be collected inside the bound.
    ///
    /// What must not come back is a successful `Output` with an empty stdout: `gh` printing a
    /// pull request number and domux reading none of it would be recorded as "the command
    /// worked and said nothing", which is a fact nobody observed (principle 4). It is also the
    /// wrong foundation for the git retrofit, which will read this module's answers the same
    /// way.
    #[test]
    fn output_that_could_not_be_read_is_a_timeout_and_never_an_empty_success() {
        let err = finishes_within(
            Duration::from_secs(15),
            "output_within on a child whose grandchild holds the pipe",
            || output_within(sh("(sleep 30 &) ; printf ok"), Duration::from_secs(1)).unwrap_err(),
        );
        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");
        assert!(
            err.to_string().contains("no answer within"),
            "the reason is the bound, not a parse failure further up: {err}"
        );
    }

    #[test]
    fn a_command_that_is_not_there_says_so_rather_than_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let err = output_within(
            Command::new(dir.path().join("no-such-command")),
            Duration::from_secs(10),
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound, "{err}");
    }
}
