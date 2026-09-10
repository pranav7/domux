//! Stay awake: while it is on, a child process holds the machine awake and the machine does
//! not fall asleep on its own. `caffeinate` does it on macOS and `systemd-inhibit` on Linux,
//! and the hold ends when that child does, so it ends with domux (design principle 11).
//!
//! Full mode also stops the lid putting the machine to sleep. On Linux that is one more thing
//! for the same child to hold. macOS has no such flag: the setting is global, it needs a
//! password, and the system clears it across some sleeps, so full mode there leans on a launch
//! daemon that keeps setting it. `stay-awake install --full` writes that daemon and the
//! sudoers line that lets it run unattended, and nothing else writes either file.
//!
//! Modelled on V1's `caffeinate.go`, at this domux's own paths, so V1's pair is left alone.

use crate::command::CommandRunner;
use crate::persist::write_atomic;
use crate::process::ProcessInspector;
use domux_core::config::StayAwakeMode;
use domux_core::names::{BIN_NAME, PRODUCT_NAME};
use std::path::{Path, PathBuf};

/// The holder's process id lives here, beside the state file, so a server that comes back
/// after a crash finds the hold it left rather than starting a second one.
pub const PID_FILE_NAME: &str = "stay-awake.pid";

/// V1's pair, named so that nothing here can be pointed at them by accident.
pub const V1_PLIST_PATH: &str = "/Library/LaunchDaemons/com.domux.noclamshell.plist";
pub const V1_SUDOERS_PATH: &str = "/etc/sudoers.d/domux-caffeinate";

pub fn plist_label() -> String {
    format!("com.{BIN_NAME}.stay-awake")
}

pub fn plist_path() -> String {
    format!("/Library/LaunchDaemons/{}.plist", plist_label())
}

pub fn sudoers_path() -> String {
    format!("/etc/sudoers.d/{BIN_NAME}-stay-awake")
}

/// The daemon full mode needs on macOS. It sets the flag again every 30 seconds because the
/// system clears it across some sleep and wake cycles, and `KeepAlive` starts the loop again
/// if it ever stops. The job only exists while it is loaded, and turning stay awake off
/// unloads it, so it never outlives a hold.
pub fn plist_text() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/bin/sh</string>
        <string>-c</string>
        <string>while :; do /usr/bin/pmset -a disablesleep 1; sleep 30; done</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
</dict>
</plist>
"#,
        plist_label()
    )
}

/// The two programs full mode drives, and nothing else. Without this line each toggle would
/// stop for a password, which is not something a keypress can do.
pub fn sudoers_text(user: &str) -> String {
    format!("{user} ALL=(ALL) NOPASSWD: /usr/bin/pmset, /bin/launchctl\n")
}

/// What holds this machine awake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// macOS.
    Caffeinate,
    /// Linux.
    SystemdInhibit,
}

impl Backend {
    pub fn program(self) -> &'static str {
        match self {
            Backend::Caffeinate => "caffeinate",
            Backend::SystemdInhibit => "systemd-inhibit",
        }
    }

    /// What to hold, as the program says it. `-dimsu` is V1's set: the display, the idle
    /// timer, the disk, the system, and all of it on battery too.
    fn args(self, mode: StayAwakeMode) -> Vec<String> {
        match self {
            Backend::Caffeinate => vec!["-dimsu".into()],
            Backend::SystemdInhibit => {
                let what = match mode {
                    StayAwakeMode::Partial => "--what=idle:sleep",
                    StayAwakeMode::Full => "--what=idle:sleep:handle-lid-switch",
                };
                vec![
                    what.into(),
                    format!("--who={PRODUCT_NAME}"),
                    format!("--why={PRODUCT_NAME} stay awake"),
                    "--mode=block".into(),
                    // The child has to outlive the call and do nothing, and this is the one
                    // program that is on every machine the inhibitor is.
                    "sleep".into(),
                    "infinity".into(),
                ]
            }
        }
    }

    /// Whether the lid is the child's to hold. On Linux it is one more word in the arguments
    /// above; on macOS it is a global setting behind a password.
    fn holds_the_lid_itself(self) -> bool {
        matches!(self, Backend::SystemdInhibit)
    }
}

pub fn backend_for(os: &str) -> Option<Backend> {
    match os {
        "macos" => Some(Backend::Caffeinate),
        "linux" => Some(Backend::SystemdInhibit),
        _ => None,
    }
}

/// The hold, and the file that outlives this process's memory of it.
pub struct StayAwake {
    pid_file: PathBuf,
    holder: Option<u32>,
}

impl StayAwake {
    pub fn new(state_dir: &Path) -> StayAwake {
        StayAwake {
            pid_file: state_dir.join(PID_FILE_NAME),
            holder: None,
        }
    }

    /// Whether this machine is being held awake by this domux.
    pub fn on(&self) -> bool {
        self.holder.is_some()
    }

    /// Takes over a hold an earlier server left behind, and clears the file when there is
    /// nothing there to take over.
    ///
    /// Two questions, because one is not enough: the process has to still be running, and it
    /// has to still be the program that was started. Process ids come around again, and a
    /// hold that thinks it owns somebody else's process would kill it on the way out.
    pub fn adopt(
        &mut self,
        os: &str,
        runner: &dyn CommandRunner,
        inspector: &dyn ProcessInspector,
    ) {
        let Some(pid) = self.recorded_pid() else {
            return;
        };
        let holder_program = backend_for(os).map(Backend::program);
        let still_ours = inspector.is_alive(pid)
            && match holder_program {
                Some(program) => is_running(runner, pid, program),
                None => false,
            };
        if still_ours {
            self.holder = Some(pid);
        } else {
            self.forget();
        }
    }

    /// Looks at the holder again, and answers whether the hold has gone since the last look.
    /// A holder killed from outside domux leaves a green dot saying something that is no
    /// longer true, so the core asks this on its tick.
    pub fn recheck(&mut self, inspector: &dyn ProcessInspector) -> bool {
        match self.holder {
            Some(pid) if !inspector.is_alive(pid) => {
                self.forget();
                true
            }
            _ => false,
        }
    }

    /// Takes the hold. `Ok(None)` means it is on and there is nothing more to say; `Ok(Some)`
    /// means it is on and something the reader asked for did not happen; `Err` means the
    /// machine is not being held awake and this is why.
    pub fn enable(
        &mut self,
        mode: StayAwakeMode,
        os: &str,
        runner: &dyn CommandRunner,
    ) -> Result<Option<String>, String> {
        let backend = backend(os)?;
        if self.on() {
            return Ok(None);
        }
        if runner.which(backend.program()).is_none() {
            return Err(not_installed(backend));
        }
        let pid = runner.spawn_detached(backend.program(), &backend.args(mode))?;
        if let Err(e) = write_atomic(&self.pid_file, &format!("{pid}\n")) {
            // Nothing would know how to end a hold whose process id was never written down,
            // so the hold is given up rather than left running with no way back to it.
            let _ = runner.kill(pid);
            return Err(format!("could not write {}: {e}", self.pid_file.display()));
        }
        self.holder = Some(pid);
        Ok(self.hold_lid(true, mode, backend, runner))
    }

    /// Gives the hold back. The answers mean what they do in `enable`.
    pub fn disable(
        &mut self,
        mode: StayAwakeMode,
        os: &str,
        runner: &dyn CommandRunner,
    ) -> Result<Option<String>, String> {
        let backend = backend(os)?;
        // The lid goes back first: after `forget` there is no hold to say it belongs to.
        let note = self.hold_lid(false, mode, backend, runner);
        if let Some(pid) = self.holder {
            runner.kill(pid)?;
        }
        self.forget();
        Ok(note)
    }

    /// Full mode's half of the hold on macOS, where the lid is a global setting rather than
    /// something the holder carries. Answers a note when the reader asked for the lid and did
    /// not get it: the hold itself is unaffected either way, so this never fails the call.
    ///
    /// Both steps run even when the first one fails. They are two independent settings, and
    /// stopping after the first would leave the machine half way between the two states with
    /// nothing said about which half.
    fn hold_lid(
        &self,
        on: bool,
        mode: StayAwakeMode,
        backend: Backend,
        runner: &dyn CommandRunner,
    ) -> Option<String> {
        if mode == StayAwakeMode::Partial || backend.holds_the_lid_itself() {
            return None;
        }
        let load = if on { "load" } else { "unload" };
        let flag = if on { "1" } else { "0" };
        let steps = [
            vec![
                "-n".to_string(),
                "launchctl".into(),
                load.into(),
                plist_path(),
            ],
            vec![
                "-n".to_string(),
                "pmset".into(),
                "-a".into(),
                "disablesleep".into(),
                flag.into(),
            ],
        ];
        let mut first_failure = None;
        for step in steps {
            if let Err(e) = runner.run("sudo", &step) {
                first_failure.get_or_insert(e);
            }
        }
        let e = first_failure?;
        Some(if on {
            format!(
                "the machine is held awake, but not against the lid closing: {e}. Run {BIN_NAME} stay-awake install --full to set full mode up"
            )
        } else {
            format!(
                "the machine is no longer held awake, but the lid setting is still on: {e}. Run sudo pmset -a disablesleep 0 to put it back"
            )
        })
    }

    /// The process id on disk, or nothing when the file is missing or holds no number.
    fn recorded_pid(&self) -> Option<u32> {
        let text = std::fs::read_to_string(&self.pid_file).ok()?;
        text.trim().parse::<u32>().ok().filter(|pid| *pid > 0)
    }

    /// No holder, and no file claiming one. Both together, always: a holder this process has
    /// forgotten but the file still names is a hold the next server would adopt.
    fn forget(&mut self) {
        self.holder = None;
        let _ = std::fs::remove_file(&self.pid_file);
    }
}

fn backend(os: &str) -> Result<Backend, String> {
    backend_for(os)
        .ok_or_else(|| format!("stay awake works on macOS and Linux, and this machine runs {os}"))
}

fn not_installed(backend: Backend) -> String {
    match backend {
        Backend::Caffeinate => format!(
            "{} is not on PATH; it ships with macOS, so check what has changed on this machine",
            backend.program()
        ),
        Backend::SystemdInhibit => format!(
            "{} is not on PATH; install the systemd package that provides it",
            backend.program()
        ),
    }
}

/// One file full mode needs, and how it has to land.
pub struct Write {
    pub path: String,
    /// As `install(1)` takes it.
    pub mode: &'static str,
    pub content: String,
    pub what_for: &'static str,
}

/// What `stay-awake install --full` would do. It is built and printed before anything is
/// written, because these two files are the only thing domux ever asks for a password to put
/// in place: the reader sees the paths and the commands first, and `--apply` is what goes
/// through with it.
pub struct InstallFullPlan {
    pub writes: Vec<Write>,
}

pub fn full_plan(user: &str) -> InstallFullPlan {
    InstallFullPlan {
        writes: vec![
            Write {
                path: plist_path(),
                mode: "0644",
                content: plist_text(),
                what_for: "keeps the no-sleep setting on, which the system otherwise clears when it wakes",
            },
            Write {
                path: sudoers_path(),
                mode: "0440",
                content: sudoers_text(user),
                what_for: "lets the two programs above run without a password, so a keypress can turn stay awake on",
            },
        ],
    }
}

impl InstallFullPlan {
    /// What the reader is shown before anything happens: what full mode buys, both files with
    /// their modes and what each is for, the commands that put them there, and the flag that
    /// runs them.
    pub fn preview(&self) -> String {
        let mut out = "Full mode stops this machine sleeping when closing the lid, on top of what stay awake already does.\nIt needs two files, both owned by root:\n\n".to_string();
        for w in &self.writes {
            out.push_str(&format!(
                "  {}\n    mode {}, {}\n    sudo install -m {} -o root -g wheel <a temporary file> {}\n\n",
                w.path, w.mode, w.what_for, w.mode, w.path
            ));
        }
        out.push_str(&format!(
            "Nothing has been written. Run {BIN_NAME} stay-awake install --full --apply to write them, then set mode = \"full\" under [stay_awake] in the config file.\n"
        ));
        out
    }

    /// Writes both files, each through `sudo install`, so the file lands with its owner and
    /// mode in one step rather than being written and then handed over.
    ///
    /// A file already there is copied aside first, keeping V1's suffix: whatever was in place
    /// is recoverable without domux (`sudo mv <path>.domux-backup <path>`).
    pub fn apply(&self, runner: &dyn CommandRunner) -> Result<(), String> {
        for w in &self.writes {
            let staged = std::env::temp_dir().join(format!("{BIN_NAME}-stay-awake-{}", w.mode));
            std::fs::write(&staged, &w.content)
                .map_err(|e| format!("could not write {}: {e}", staged.display()))?;
            let staged = staged.to_string_lossy().to_string();
            if std::path::Path::new(&w.path).exists() {
                runner
                    .run(
                        "sudo",
                        &[
                            "cp".into(),
                            "-p".into(),
                            w.path.clone(),
                            format!("{}.domux-backup", w.path),
                        ],
                    )
                    .map_err(|e| format!("could not back up {}: {e}", w.path))?;
            }
            let outcome = runner.run(
                "sudo",
                &[
                    "install".into(),
                    "-m".into(),
                    w.mode.to_string(),
                    "-o".into(),
                    "root".into(),
                    "-g".into(),
                    "wheel".into(),
                    staged.clone(),
                    w.path.clone(),
                ],
            );
            let _ = std::fs::remove_file(&staged);
            outcome.map_err(|e| format!("could not install {}: {e}", w.path))?;
        }
        Ok(())
    }
}

/// Whether `pid` is still the program it was started as. `ps` is the one question that
/// answers it the same way on both platforms, and V1 asked it too.
fn is_running(runner: &dyn CommandRunner, pid: u32, program: &str) -> bool {
    runner
        .output(
            "ps",
            &["-o".into(), "comm=".into(), "-p".into(), pid.to_string()],
        )
        .map(|out| out.contains(program))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::FakeRunner;
    use crate::process::FakeInspector;
    use domux_core::config::StayAwakeMode::{Full, Partial};

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// A runner with the holder installed, answering `ps` for the process id the fake hands
    /// out first, so a hold it started reads back as the program that started it.
    fn runner_with(program: &str) -> FakeRunner {
        let runner = FakeRunner::default();
        runner.on_path(program);
        runner.answer("ps", &["-o", "comm=", "-p", "4242"], program);
        runner
    }

    #[test]
    fn a_partial_hold_on_macos_starts_the_holder_and_records_its_process_id() {
        let d = dir();
        let runner = runner_with("caffeinate");
        let mut s = StayAwake::new(d.path());
        assert!(!s.on());
        assert_eq!(s.enable(Partial, "macos", &runner), Ok(None));
        assert!(s.on());
        assert!(
            runner.ran("caffeinate", &["-dimsu"]),
            "{:?}",
            runner.calls()
        );
        let written = std::fs::read_to_string(d.path().join(PID_FILE_NAME)).unwrap();
        assert_eq!(written.trim(), "4242");
        assert!(
            runner.calls_to("sudo").is_empty(),
            "partial mode asks for no password"
        );
    }

    #[test]
    fn a_full_hold_on_macos_asks_launchd_and_pmset_for_the_lid() {
        let d = dir();
        let runner = runner_with("caffeinate");
        let mut s = StayAwake::new(d.path());
        assert_eq!(s.enable(Full, "macos", &runner), Ok(None));
        assert!(runner.ran("caffeinate", &["-dimsu"]));
        assert!(
            runner.ran("sudo", &["-n", "launchctl", "load", &plist_path()]),
            "{:?}",
            runner.calls()
        );
        assert!(runner.ran("sudo", &["-n", "pmset", "-a", "disablesleep", "1"]));
    }

    #[test]
    fn disabling_stops_the_holder_and_removes_the_process_id_file() {
        let d = dir();
        let runner = runner_with("caffeinate");
        let mut s = StayAwake::new(d.path());
        s.enable(Partial, "macos", &runner).unwrap();
        assert_eq!(s.disable(Partial, "macos", &runner), Ok(None));
        assert!(!s.on());
        assert!(!d.path().join(PID_FILE_NAME).exists());
        assert!(runner.ran("kill", &["4242"]), "{:?}", runner.calls());
    }

    #[test]
    fn disabling_a_full_hold_gives_the_lid_back() {
        let d = dir();
        let runner = runner_with("caffeinate");
        let mut s = StayAwake::new(d.path());
        s.enable(Full, "macos", &runner).unwrap();
        s.disable(Full, "macos", &runner).unwrap();
        assert!(runner.ran("sudo", &["-n", "launchctl", "unload", &plist_path()]));
        assert!(runner.ran("sudo", &["-n", "pmset", "-a", "disablesleep", "0"]));
    }

    #[test]
    fn a_hold_on_linux_stops_idle_and_sleep_and_full_mode_adds_the_lid() {
        let d = dir();
        let runner = runner_with("systemd-inhibit");
        let mut partial = StayAwake::new(d.path());
        partial.enable(Partial, "linux", &runner).unwrap();
        assert_eq!(
            runner.calls_to("systemd-inhibit")[0].args,
            vec![
                "--what=idle:sleep".to_string(),
                "--who=domux".to_string(),
                "--why=domux stay awake".to_string(),
                "--mode=block".to_string(),
                "sleep".to_string(),
                "infinity".to_string(),
            ]
        );
        let mut full = StayAwake::new(dir().path());
        full.enable(Full, "linux", &runner).unwrap();
        assert_eq!(
            runner.calls_to("systemd-inhibit")[1].args[0],
            "--what=idle:sleep:handle-lid-switch"
        );
        assert!(
            runner.calls_to("sudo").is_empty(),
            "the lid costs no password on linux"
        );
    }

    #[test]
    fn a_platform_with_no_way_to_hold_it_awake_says_so_and_runs_nothing() {
        let d = dir();
        let runner = FakeRunner::default();
        let mut s = StayAwake::new(d.path());
        assert_eq!(
            s.enable(Partial, "freebsd", &runner),
            Err("stay awake works on macOS and Linux, and this machine runs freebsd".into())
        );
        assert!(!s.on());
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn a_holder_that_is_not_installed_names_it_and_leaves_no_process_id_file() {
        let d = dir();
        let runner = FakeRunner::default();
        let mut s = StayAwake::new(d.path());
        assert_eq!(
            s.enable(Partial, "linux", &runner),
            Err(
                "systemd-inhibit is not on PATH; install the systemd package that provides it"
                    .into()
            )
        );
        assert!(!s.on());
        assert!(!d.path().join(PID_FILE_NAME).exists());
    }

    #[test]
    fn enabling_a_hold_that_is_already_on_keeps_the_first_holder() {
        let d = dir();
        let runner = runner_with("caffeinate");
        let mut s = StayAwake::new(d.path());
        s.enable(Partial, "macos", &runner).unwrap();
        s.enable(Partial, "macos", &runner).unwrap();
        assert_eq!(runner.calls_to("caffeinate").len(), 1);
        assert!(s.on());
    }

    #[test]
    fn a_hold_left_by_an_earlier_server_is_adopted_rather_than_doubled() {
        let d = dir();
        let runner = runner_with("caffeinate");
        std::fs::write(d.path().join(PID_FILE_NAME), "4242\n").unwrap();
        let mut s = StayAwake::new(d.path());
        s.adopt("macos", &runner, &FakeInspector::default());
        assert!(s.on());
        s.enable(Partial, "macos", &runner).unwrap();
        assert!(
            runner.calls_to("caffeinate").is_empty(),
            "the adopted hold is the hold"
        );
    }

    #[test]
    fn a_process_id_whose_process_is_gone_is_dropped_along_with_its_file() {
        let d = dir();
        let runner = runner_with("caffeinate");
        std::fs::write(d.path().join(PID_FILE_NAME), "4242\n").unwrap();
        let inspector = FakeInspector::default();
        inspector.set_dead(4242);
        let mut s = StayAwake::new(d.path());
        s.adopt("macos", &runner, &inspector);
        assert!(!s.on());
        assert!(!d.path().join(PID_FILE_NAME).exists());
    }

    #[test]
    fn a_process_id_that_now_belongs_to_something_else_is_not_adopted() {
        let d = dir();
        let runner = FakeRunner::default();
        runner.on_path("caffeinate");
        runner.answer("ps", &["-o", "comm=", "-p", "4242"], "postgres");
        std::fs::write(d.path().join(PID_FILE_NAME), "4242\n").unwrap();
        let mut s = StayAwake::new(d.path());
        s.adopt("macos", &runner, &FakeInspector::default());
        assert!(!s.on(), "the number came back around to another program");
        assert!(!d.path().join(PID_FILE_NAME).exists());
    }

    #[test]
    fn a_holder_that_died_on_its_own_stops_reading_as_a_hold() {
        let d = dir();
        let runner = runner_with("caffeinate");
        let mut s = StayAwake::new(d.path());
        s.enable(Partial, "macos", &runner).unwrap();
        let inspector = FakeInspector::default();
        assert!(!s.recheck(&inspector), "nothing changed while it is alive");
        inspector.set_dead(4242);
        assert!(s.recheck(&inspector), "the hold went away");
        assert!(!s.on());
        assert!(!d.path().join(PID_FILE_NAME).exists());
        assert!(!s.recheck(&inspector), "it only changes once");
    }

    #[test]
    fn a_lid_that_sudo_refuses_leaves_the_hold_on_and_says_what_to_run() {
        let d = dir();
        let runner = runner_with("caffeinate");
        runner.fail(
            "sudo",
            &["-n", "launchctl", "load", &plist_path()],
            "sudo: a password is required",
        );
        let mut s = StayAwake::new(d.path());
        let note = s.enable(Full, "macos", &runner).unwrap().unwrap();
        assert!(s.on(), "the machine is held awake even without the lid");
        assert!(note.contains("stay-awake install --full"), "{note}");
    }

    #[test]
    fn a_lid_that_will_not_go_back_says_what_to_run_to_put_it_back() {
        let d = dir();
        let runner = runner_with("caffeinate");
        let mut s = StayAwake::new(d.path());
        s.enable(Full, "macos", &runner).unwrap();
        runner.fail(
            "sudo",
            &["-n", "pmset", "-a", "disablesleep", "0"],
            "sudo: a password is required",
        );
        let note = s.disable(Full, "macos", &runner).unwrap().unwrap();
        assert!(note.contains("sudo pmset -a disablesleep 0"), "{note}");
        assert!(!s.on(), "the hold itself went either way");
        assert!(
            runner.ran("sudo", &["-n", "launchctl", "unload", &plist_path()]),
            "the other step still ran: {:?}",
            runner.calls()
        );
    }

    #[test]
    fn the_full_install_plan_names_both_files_their_modes_and_the_line_it_adds() {
        let plan = full_plan("ada");
        assert_eq!(plan.writes.len(), 2);
        assert_eq!(plan.writes[0].path, plist_path());
        assert_eq!(plan.writes[0].mode, "0644");
        assert_eq!(plan.writes[1].path, sudoers_path());
        assert_eq!(plan.writes[1].mode, "0440");
        let preview = plan.preview();
        assert!(preview.contains(&plist_path()), "{preview}");
        assert!(preview.contains(&sudoers_path()), "{preview}");
        assert!(
            preview.contains("sudo install -m 0644 -o root -g wheel"),
            "the reader sees the command before it runs: {preview}"
        );
        assert!(
            preview.contains("closing the lid"),
            "and what it buys them: {preview}"
        );
        assert!(
            preview.contains("--apply"),
            "and how to go through with it: {preview}"
        );
    }

    #[test]
    fn applying_the_plan_puts_both_files_in_place_owned_by_root() {
        let runner = FakeRunner::default();
        full_plan("ada").apply(&runner).unwrap();
        let installs: Vec<_> = runner
            .calls_to("sudo")
            .into_iter()
            .filter(|c| c.args.first().map(String::as_str) == Some("install"))
            .collect();
        assert_eq!(installs.len(), 2, "{:?}", runner.calls());
        for (call, (mode, dest)) in installs
            .iter()
            .zip([("0644", plist_path()), ("0440", sudoers_path())])
        {
            assert_eq!(
                call.args[..7],
                ["install", "-m", mode, "-o", "root", "-g", "wheel"]
            );
            assert_eq!(call.args.last(), Some(&dest), "{:?}", call.args);
        }
    }

    #[test]
    fn a_refused_install_says_which_file_it_was_and_leaves_the_second_alone() {
        let runner = FakeRunner::default();
        // The temporary file's name is not known here, so the whole call cannot be named:
        // the fake refuses by program and by the arguments that are the same every run.
        runner.fail_starting_with("sudo", &["install", "-m", "0644"], "sudo: no");
        let e = full_plan("ada").apply(&runner).unwrap_err();
        assert!(e.contains(&plist_path()), "{e}");
        assert!(e.contains("sudo: no"), "{e}");
        assert!(
            runner
                .calls_to("sudo")
                .iter()
                .all(|c| c.args.last() != Some(&sudoers_path())),
            "the second file is not written after the first failed: {:?}",
            runner.calls()
        );
    }

    #[test]
    fn the_two_files_full_mode_needs_are_not_the_ones_v1_wrote() {
        assert_ne!(plist_path(), V1_PLIST_PATH);
        assert_ne!(sudoers_path(), V1_SUDOERS_PATH);
        assert!(plist_path().starts_with("/Library/LaunchDaemons/"));
        assert!(sudoers_path().starts_with("/etc/sudoers.d/"));
        let plist = plist_text();
        assert!(plist.contains(&plist_label()), "{plist}");
        assert!(plist.contains("while :; do /usr/bin/pmset -a disablesleep 1; sleep 30; done"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert_eq!(
            sudoers_text("ada"),
            "ada ALL=(ALL) NOPASSWD: /usr/bin/pmset, /bin/launchctl\n"
        );
    }
}
