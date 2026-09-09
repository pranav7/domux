//! Agent manifests: how domux recognises, colours, resumes and hooks one kind of agent. A
//! manifest is a Layer B declaration domux carries, not a runtime artifact a session writes
//! (architecture spec section 8). Claude, Codex and OpenCode are the built-ins, and they go
//! through the same registry an extension would use.

use crate::agents::hooks::{EVENTS_CLAUDE, EVENTS_CODEX};
use domux_core::model::agent::AgentKind;
use std::path::{Path, PathBuf};

/// One kind of agent, declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentManifest {
    pub kind: AgentKind,
    /// Foreground process names the observer recognises. Matched without case.
    pub process_names: Vec<String>,
    /// The kind's colour (interface spec 9.1), as a hex string so a declaration can carry it.
    pub color_hex: String,
    /// The line resume types into a shell, with `{session_id}` replaced. `None` means this
    /// kind does not resume in V2.0.
    pub resume_command: Option<String>,
    pub hooks: HookTarget,
    pub recap: RecapSource,
    pub session: SessionSource,
}

/// Where the install command writes for one kind, and what it writes there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookTarget {
    /// `~/.claude/settings.json`: a `hooks` object keyed by event name.
    ClaudeSettings,
    /// `~/.codex/hooks.json`: the same object shape, Codex's event names.
    CodexHooks,
    /// `~/.config/opencode/plugins/domux.js`: a generated plugin file.
    OpencodePlugin,
}

impl HookTarget {
    pub fn path_in(&self, home: &Path) -> PathBuf {
        match self {
            HookTarget::ClaudeSettings => home.join(".claude/settings.json"),
            HookTarget::CodexHooks => home.join(".codex/hooks.json"),
            HookTarget::OpencodePlugin => home.join(".config/opencode/plugins/domux.js"),
        }
    }

    /// The events the installer writes one command line for. The OpenCode plugin listens for
    /// its own events inside the file it writes, so it installs none here.
    pub fn events(&self) -> &'static [&'static str] {
        match self {
            HookTarget::ClaudeSettings => &EVENTS_CLAUDE,
            HookTarget::CodexHooks => &EVENTS_CODEX,
            HookTarget::OpencodePlugin => &[],
        }
    }
}

/// Where the recap comes from. Codex and OpenCode have none in V2.0, so their rows show no
/// recap rather than a guessed one (principle 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecapSource {
    /// The JSONL transcript the hook names, read by `agents::recap`.
    ClaudeTranscript,
    None,
}

/// Where the session id comes from when a record has no hook payload yet. V2.0 uses the hook
/// payload for every kind; the other two are declared because the manifest is the place that
/// answers where to find the session id (architecture spec section 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionSource {
    HookPayload,
    /// `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`, first line `session_meta.payload.id`
    /// (V1's `readCodexSessions`). Not read in V2.0.
    CodexRollout,
    /// `opencode session list --format json` (V1's `readOpencodeSessions`). Not read in V2.0.
    OpencodeCli,
}

/// The registry. `builtin()` is the three manifests; `register` is what a Layer B extension
/// calls in V2.x. A manifest replaces any manifest already registered for its kind.
#[derive(Debug, Clone)]
pub struct Registry {
    manifests: Vec<AgentManifest>,
}

impl Registry {
    pub fn builtin() -> Registry {
        Registry {
            manifests: vec![
                AgentManifest {
                    kind: AgentKind::Claude,
                    process_names: vec!["claude".into()],
                    color_hex: "#DE7356".into(),
                    // V1's `resumeAgentLaunchLine`, claude arm.
                    resume_command: Some("claude --resume {session_id}".into()),
                    hooks: HookTarget::ClaudeSettings,
                    recap: RecapSource::ClaudeTranscript,
                    session: SessionSource::HookPayload,
                },
                AgentManifest {
                    kind: AgentKind::Codex,
                    process_names: vec!["codex".into()],
                    color_hex: "#89b4fa".into(),
                    // V2.x: "codex resume {session_id}" (V1's codex arm).
                    resume_command: None,
                    hooks: HookTarget::CodexHooks,
                    recap: RecapSource::None,
                    session: SessionSource::CodexRollout,
                },
                AgentManifest {
                    kind: AgentKind::Opencode,
                    process_names: vec!["opencode".into()],
                    color_hex: "#C678B8".into(),
                    // V2.x: "opencode --session {session_id}" (V1's opencode arm).
                    resume_command: None,
                    hooks: HookTarget::OpencodePlugin,
                    recap: RecapSource::None,
                    session: SessionSource::OpencodeCli,
                },
            ],
        }
    }

    pub fn register(&mut self, manifest: AgentManifest) {
        self.manifests.retain(|m| m.kind != manifest.kind);
        self.manifests.push(manifest);
    }

    pub fn for_kind(&self, kind: AgentKind) -> Option<&AgentManifest> {
        self.manifests.iter().find(|m| m.kind == kind)
    }

    /// The manifest whose process names contain `name`, ignoring case. This is the observer's
    /// question: is the foreground process a known agent?
    pub fn for_process(&self, name: &str) -> Option<&AgentManifest> {
        self.manifests
            .iter()
            .find(|m| m.process_names.iter().any(|p| p.eq_ignore_ascii_case(name)))
    }

    /// In `AgentKind::ALL` order, so lists are stable.
    pub fn kinds(&self) -> Vec<AgentKind> {
        AgentKind::ALL
            .into_iter()
            .filter(|k| self.for_kind(*k).is_some())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_built_in_manifests_are_registered_with_their_colours_and_processes() {
        let r = Registry::builtin();
        assert_eq!(
            r.kinds(),
            vec![AgentKind::Claude, AgentKind::Codex, AgentKind::Opencode]
        );
        assert_eq!(r.for_kind(AgentKind::Claude).unwrap().color_hex, "#DE7356");
        assert_eq!(r.for_kind(AgentKind::Codex).unwrap().color_hex, "#89b4fa");
        assert_eq!(
            r.for_kind(AgentKind::Opencode).unwrap().color_hex,
            "#C678B8"
        );
        assert_eq!(
            r.for_process("claude").map(|m| m.kind),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            r.for_process("codex").map(|m| m.kind),
            Some(AgentKind::Codex)
        );
        assert_eq!(
            r.for_process("opencode").map(|m| m.kind),
            Some(AgentKind::Opencode)
        );
        assert_eq!(r.for_process("zsh"), None);
        assert_eq!(r.for_process("nvim"), None);
        assert_eq!(
            r.for_process("Claude").map(|m| m.kind),
            Some(AgentKind::Claude),
            "the process name match ignores case"
        );
    }

    #[test]
    fn only_claude_resumes_in_v2_0_and_its_template_is_v1s_line() {
        let r = Registry::builtin();
        assert_eq!(
            r.for_kind(AgentKind::Claude)
                .unwrap()
                .resume_command
                .as_deref(),
            Some("claude --resume {session_id}")
        );
        assert_eq!(r.for_kind(AgentKind::Codex).unwrap().resume_command, None);
        assert_eq!(
            r.for_kind(AgentKind::Opencode).unwrap().resume_command,
            None
        );
    }

    #[test]
    fn each_manifest_names_where_its_hooks_go_and_which_events_it_installs() {
        let r = Registry::builtin();
        let home = std::path::Path::new("/Users/pranav");
        let claude = r.for_kind(AgentKind::Claude).unwrap();
        assert_eq!(
            claude.hooks.path_in(home),
            home.join(".claude/settings.json")
        );
        assert_eq!(
            claude.hooks.events(),
            &crate::agents::hooks::EVENTS_CLAUDE[..]
        );
        let codex = r.for_kind(AgentKind::Codex).unwrap();
        assert_eq!(codex.hooks.path_in(home), home.join(".codex/hooks.json"));
        assert_eq!(
            codex.hooks.events(),
            &crate::agents::hooks::EVENTS_CODEX[..]
        );
        let opencode = r.for_kind(AgentKind::Opencode).unwrap();
        assert_eq!(
            opencode.hooks.path_in(home),
            home.join(".config/opencode/plugins/domux.js")
        );
        assert!(
            opencode.hooks.events().is_empty(),
            "the plugin listens for its own events"
        );
    }

    #[test]
    fn only_claude_has_a_transcript_recap_in_v2_0() {
        let r = Registry::builtin();
        assert_eq!(
            r.for_kind(AgentKind::Claude).unwrap().recap,
            RecapSource::ClaudeTranscript
        );
        assert_eq!(
            r.for_kind(AgentKind::Codex).unwrap().recap,
            RecapSource::None
        );
        assert_eq!(
            r.for_kind(AgentKind::Opencode).unwrap().recap,
            RecapSource::None
        );
    }

    #[test]
    fn a_registered_manifest_joins_the_registry_the_built_ins_use() {
        let mut r = Registry::builtin();
        let before = r.kinds().len();
        let mut gemini = r.for_kind(AgentKind::Claude).unwrap().clone();
        gemini.process_names = vec!["gemini".into()];
        r.register(gemini);
        assert_eq!(
            r.kinds().len(),
            before,
            "the kind was already there, so the manifest replaced it"
        );
        assert_eq!(
            r.for_process("gemini").map(|m| m.kind),
            Some(AgentKind::Claude)
        );
        assert_eq!(
            r.for_process("claude"),
            None,
            "the replacement took the kind's process names with it"
        );
    }
}
