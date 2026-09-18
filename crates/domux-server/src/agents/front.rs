//! What agent is running in front of a pane.
//!
//! The process in front is the leader of the pane's foreground process group, and that is the
//! agent itself when the user typed its name. It is not when a wrapper started it: a version
//! manager's shim, a package manager's launcher, or a script of the user's own stays in front
//! while the agent runs in its group. Reading only the leader leaves such a pane with no
//! record at all, so the group is asked as well (decision record 0056).

use crate::agents::manifests::Registry;
use crate::process::{ForegroundProcess, ProcessInspector};
use domux_core::model::agent::AgentKind;

/// The kind and process of the agent in front of a pane, if one is there: the process in
/// front when that is an agent the manifests name, else the first agent in its group, nearest
/// to the leader first.
///
/// The process answered is the agent's own. It is what ends the record when it goes, and what
/// a hook's walk up passes through, so naming the wrapper instead would say an agent is there
/// while the pane's record answered for a process that is not the agent.
pub fn agent_in_front(
    inspector: &dyn ProcessInspector,
    manifests: &Registry,
    foreground: &ForegroundProcess,
) -> Option<(AgentKind, u32)> {
    if let Some(m) = manifests.for_process(&foreground.name) {
        return Some((m.kind, foreground.pid));
    }
    inspector
        .group_members(foreground.pid)
        .into_iter()
        .find_map(|pid| {
            let name = inspector.name_of(pid)?;
            manifests.for_process(&name).map(|m| (m.kind, pid))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::FakeInspector;

    /// A pane's foreground group: the leader first, then the processes it started, each one
    /// the child of the one before it. Every one of them is in the leader's group.
    fn group(names: &[&str]) -> (FakeInspector, Vec<u32>) {
        let fake = FakeInspector::default();
        let mut pids = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let pid = 1000 + i as u32;
            fake.set_process(pid, name, pids.last().copied());
            fake.set_group(pid, 1000);
            pids.push(pid);
        }
        (fake, pids)
    }

    fn in_front(fake: &FakeInspector, pid: u32, name: &str) -> Option<(AgentKind, u32)> {
        let f = ForegroundProcess {
            pid,
            name: name.into(),
        };
        agent_in_front(fake, &Registry::builtin(), &f)
    }

    #[test]
    fn the_process_in_front_is_the_agent_when_the_user_typed_its_name() {
        let (fake, pids) = group(&["claude"]);
        assert_eq!(
            in_front(&fake, pids[0], "claude"),
            Some((AgentKind::Claude, pids[0]))
        );
    }

    /// MUX-54. `~/.local/bin/codex` is a script, so the shell runs `bash`, and the agent is
    /// two processes further down.
    #[test]
    fn an_agent_a_wrapper_started_is_the_agent_in_front() {
        let (fake, pids) = group(&["bash", "mise", "codex"]);
        assert_eq!(
            in_front(&fake, pids[0], "bash"),
            Some((AgentKind::Codex, pids[2]))
        );
    }

    /// The agent nearest the leader is the one in front: an agent it started in turn is a
    /// nested agent, and a nested agent is not the pane's agent (decision record 0045).
    #[test]
    fn the_agent_nearest_the_process_in_front_is_the_one() {
        let (fake, pids) = group(&["node", "codex", "sh", "claude"]);
        assert_eq!(
            in_front(&fake, pids[0], "node"),
            Some((AgentKind::Codex, pids[1]))
        );
    }

    #[test]
    fn a_pane_running_no_agent_has_none() {
        let (fake, pids) = group(&["zsh", "vim"]);
        assert_eq!(in_front(&fake, pids[0], "zsh"), None);
    }

    /// A session on a daemon of its own leaves the pane's group, and a process outside the
    /// group is not in front of the pane, whatever started it.
    #[test]
    fn a_process_that_left_the_group_is_not_in_front() {
        let (fake, pids) = group(&["bash", "mise"]);
        let daemon = 2000;
        fake.set_process(daemon, "codex", Some(pids[1]));
        fake.set_group(daemon, daemon);
        assert_eq!(in_front(&fake, pids[0], "bash"), None);
    }
}
