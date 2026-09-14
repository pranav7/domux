//! A nested agent: an agent that another agent started, such as a worker a session hands a
//! task to from its shell. It inherits the pane's environment, so its hooks report from the
//! pane, and it is not the pane's agent (decision record 0045).
//!
//! What tells the two apart is the process tree. A hook runs under the agent that ran it, and
//! that agent runs under the pane's shell. A nested agent's hook has a second agent above the
//! first one.

use crate::agents::manifests::Registry;
use crate::process::ProcessInspector;

/// Whether `caller` runs under two agents the manifests name: the one that ran the hook, and
/// another one above it.
///
/// The walk stops below `server`, because nothing above the server is in a pane: a server that
/// was started from inside an agent's shell would otherwise mark every agent in it nested.
/// A walk that ends early, at a process the OS no longer answers for, finds at most one agent
/// and says no, so a report is only ever dropped on evidence of a second agent. It also ends at
/// a process it has already read: the table is read one process at a time while processes come
/// and go, and a loop would count one agent twice.
pub fn is_nested(
    inspector: &dyn ProcessInspector,
    manifests: &Registry,
    caller: u32,
    server: u32,
) -> bool {
    let mut agents = 0;
    let mut seen = Vec::new();
    let mut next = Some(caller);
    while let Some(pid) = next {
        if seen.contains(&pid) {
            break;
        }
        seen.push(pid);
        let named = inspector.name_of(pid);
        if named.is_some_and(|n| manifests.for_process(&n).is_some()) {
            agents += 1;
            if agents == 2 {
                return true;
            }
        }
        next = inspector.parent_of(pid).filter(|&parent| parent != server);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::FakeInspector;

    const SERVER: u32 = 100;

    /// A process table from the caller up: each name is the parent of the one before it, and
    /// the last one's parent is `top`.
    fn chain(names: &[&str], top: Option<u32>) -> FakeInspector {
        let fake = FakeInspector::default();
        for (i, name) in names.iter().enumerate() {
            let pid = 1000 + i as u32;
            let parent = match i + 1 < names.len() {
                true => Some(pid + 1),
                false => top,
            };
            fake.set_process(pid, name, parent);
        }
        fake
    }

    fn nested(fake: &FakeInspector) -> bool {
        is_nested(fake, &Registry::builtin(), 1000, SERVER)
    }

    #[test]
    fn a_hook_its_panes_agent_ran_is_not_nested() {
        let fake = chain(&["domux", "sh", "claude", "zsh"], Some(SERVER));
        assert!(!nested(&fake));
    }

    #[test]
    fn a_hook_run_by_an_agent_another_agent_started_is_nested() {
        let fake = chain(
            &["domux", "sh", "claude", "zsh", "claude", "zsh"],
            Some(SERVER),
        );
        assert!(nested(&fake));
    }

    #[test]
    fn an_agent_of_another_kind_above_counts_as_the_second_agent() {
        let fake = chain(
            &["domux", "sh", "codex", "zsh", "claude", "zsh"],
            Some(SERVER),
        );
        assert!(nested(&fake));
    }

    /// The plugin that reports for OpenCode runs inside OpenCode, so the agent can be the
    /// caller's own parent with no shell between.
    #[test]
    fn an_agent_directly_above_the_caller_counts() {
        let fake = chain(&["domux", "opencode", "claude"], Some(SERVER));
        assert!(nested(&fake));
    }

    #[test]
    fn an_agent_above_the_server_is_not_in_the_pane() {
        let fake = chain(&["domux", "sh", "claude", "zsh"], Some(SERVER));
        fake.set_process(SERVER, "domux", Some(2000));
        fake.set_process(2000, "claude", None);
        assert!(!nested(&fake));
    }

    #[test]
    fn a_walk_that_ends_before_a_second_agent_is_not_nested() {
        assert!(!nested(&chain(&["domux", "sh", "claude"], None)));
        assert!(!nested(&FakeInspector::default()), "nothing known at all");
    }

    #[test]
    fn a_table_that_loops_ends_the_walk() {
        let fake = chain(&["domux", "sh", "claude"], Some(1000));
        assert!(!nested(&fake));
    }
}
