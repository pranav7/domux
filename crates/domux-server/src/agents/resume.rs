//! Resume: the line that puts an exited agent back, typed into a shell in its own pane.
//!
//! Typing rather than running is the whole design (architecture spec section 5). You see the
//! command before it runs, the shell history has it, and when the agent exits the second time
//! you are at a prompt instead of in a dead pane. Nothing here writes anywhere: it builds a
//! string, and `api::agent::resume` is what types it.
//!
//! Carried over from V1's `resumeAgentLaunchLine` (commit b02a3ae). The one change is where
//! the per-kind command comes from: V1 switched on the agent's name inside this function, and
//! the manifests hold it here, so the registry an extension would use answers for its own kind
//! (architecture spec section 8).

use crate::agents::manifests::AgentManifest;
use std::path::Path;

/// What a kind with no resume command answers, after its own name: "codex does not resume
/// ...". Codex and OpenCode carry no template in V2.0, so an exited row of either says which
/// kind cannot resume and what to do instead, rather than failing in words about a manifest
/// (principle 9).
///
/// It names no version, for two reasons. The release this ships in is 1.0.0 whatever the
/// internal names say, so a message naming 2.0 would be wrong in front of a reader. And the
/// product name is one word of the binary name: `names::BIN_NAME` loses its `2` at the
/// cut-over, and a message that spelled the product would then fail
/// `names::tests::nothing_outside_this_file_spells_the_binary_name` - from a resume message,
/// which is nowhere anyone renaming the binary would think to look. "yet" carries what the
/// version was there to carry, which is that the other kinds are coming.
pub const RESUME_UNAVAILABLE: &str =
    "does not resume yet; only claude does. Start it yourself in its pane";

/// The line, or `None` when this kind has no resume command.
///
/// Two guards, both V1's and both kept for its reasons. `cd` first, because `claude --resume`
/// is scoped to the project directory and finds no session from anywhere else. `command -v`
/// second, so a machine without the CLI installed leaves an untouched prompt in the pane
/// instead of an error nobody asked for.
pub fn resume_line(manifest: &AgentManifest, session_id: &str, cwd: &Path) -> Option<String> {
    let template = manifest.resume_command.as_ref()?;
    // The process name, not the kind's own word. It is what the observer matches a foreground
    // process against, so `command -v` asks about the binary domux would recognise if this
    // line worked. For all three built-ins the two strings are the same, and for a manifest an
    // extension registers they need not be.
    let binary = manifest.process_names.first()?;
    let command = template.replace("{session_id}", &shell_quote(session_id));
    let guarded = format!("command -v {binary} >/dev/null 2>&1 && {command}");
    // A record with no directory is not something the handlers can produce today: every path
    // that makes one takes the cwd from the hook payload or from the pane, and a payload
    // carrying an empty string reads as absent. The branch is V1's and it stays, because
    // `cd ''` is a line that fails in the pane where leaving the `cd` off is a line that works.
    if cwd.as_os_str().is_empty() {
        return Some(guarded);
    }
    Some(format!(
        "cd {} && {guarded}",
        shell_quote(&cwd.to_string_lossy())
    ))
}

/// One shell word: single quotes, with an embedded quote closed, escaped and reopened.
///
/// V1's rule (`shellQuote`, commit e2fe7eb), spelled `'\''` where V1 spells the same escape
/// `'"'"'`. Both close the quoted run, pass one literal quote and open a new run, so a shell
/// reads the same word from either. The empty string quotes to `''`, which is an empty word
/// rather than no word at all.
///
/// Not the same function as `install::shell_command_path`, which quotes a path only when it has
/// to. That one writes a hook line into a file a person reads and edits, where a bare path is
/// the readable form; this one wraps a session id and a directory into a line domux composes,
/// where quoting always is the safe rule and costs nothing.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::manifests::Registry;
    use domux_core::model::agent::AgentKind;
    use std::path::PathBuf;

    fn manifest(kind: AgentKind) -> AgentManifest {
        Registry::builtin().for_kind(kind).unwrap().clone()
    }

    #[test]
    fn the_claude_line_cds_first_then_guards_the_binary_then_resumes_the_session() {
        assert_eq!(
            resume_line(
                &manifest(AgentKind::Claude),
                "sess-1",
                &PathBuf::from("/Users/pranav/projects/audrey-app"),
            )
            .unwrap(),
            "cd '/Users/pranav/projects/audrey-app' && command -v claude >/dev/null 2>&1 && claude --resume 'sess-1'"
        );
    }

    #[test]
    fn a_kind_whose_manifest_carries_no_resume_command_has_no_line() {
        let cwd = PathBuf::from("/repo");
        assert_eq!(resume_line(&manifest(AgentKind::Codex), "x1", &cwd), None);
        assert_eq!(
            resume_line(&manifest(AgentKind::Opencode), "x1", &cwd),
            None
        );
    }

    #[test]
    fn a_line_for_a_record_with_no_directory_starts_at_the_guard() {
        assert_eq!(
            resume_line(&manifest(AgentKind::Claude), "s", Path::new("")).unwrap(),
            "command -v claude >/dev/null 2>&1 && claude --resume 's'",
            "no cd, rather than a cd to nowhere"
        );
    }

    /// The `?` on the process name. A manifest an extension registers can carry a resume
    /// command and no process name, and there is then no binary to guard on, so there is no
    /// line: `api::agent::plan_resume` reports that as the internal inconsistency it is.
    #[test]
    fn a_manifest_with_a_resume_command_and_no_process_name_has_no_line() {
        let mut m = manifest(AgentKind::Claude);
        m.process_names.clear();
        assert_eq!(resume_line(&m, "s", Path::new("/repo")), None);
    }

    /// The session id and the directory both go through the quoting, so neither can end the
    /// line early. Two assertions because they are two separate calls in `resume_line`, and one
    /// of them would pass with the other's quoting dropped.
    #[test]
    fn a_quote_in_the_session_id_or_in_the_directory_is_quoted_in_the_line() {
        let m = manifest(AgentKind::Claude);
        assert_eq!(
            resume_line(&m, "a'b", Path::new("/repo")).unwrap(),
            "cd '/repo' && command -v claude >/dev/null 2>&1 && claude --resume 'a'\\''b'"
        );
        assert_eq!(
            resume_line(&m, "s", Path::new("/re'po")).unwrap(),
            "cd '/re'\\''po' && command -v claude >/dev/null 2>&1 && claude --resume 's'"
        );
    }

    #[test]
    fn shell_quote_wraps_a_plain_word_and_reopens_the_run_around_an_embedded_quote() {
        assert_eq!(shell_quote("plain"), "'plain'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(
            shell_quote("a'b'c"),
            "'a'\\''b'\\''c'",
            "every quote, not just the first"
        );
    }

    #[test]
    fn shell_quote_makes_the_empty_string_one_empty_word() {
        assert_eq!(shell_quote(""), "''");
    }

    /// A space, a separator and an expansion are what would otherwise split the line or run
    /// something else. They are inside the quotes, which is what makes them harmless.
    #[test]
    fn shell_quote_keeps_a_space_a_separator_and_an_expansion_inside_the_word() {
        assert_eq!(shell_quote("my dir"), "'my dir'");
        assert_eq!(shell_quote("a; rm -rf /"), "'a; rm -rf /'");
        assert_eq!(shell_quote("$HOME"), "'$HOME'");
    }
}
