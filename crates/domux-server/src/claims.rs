//! The programs that claimed a pane's passthrough keys (decision 0054).
//!
//! A claim is a process id. It holds while that process is in the pane's foreground process
//! group, so a program that is suspended or has ended gives the keys back without saying so.
//! The group questions are the inspector's; this type only keeps the set.

use std::collections::BTreeSet;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Claims(BTreeSet<u32>);

impl Claims {
    /// The claims an upgrade carried. The processes are the same ones, still in the pane
    /// (decision 0046).
    pub fn from_pids(pids: &[u32]) -> Claims {
        Claims(pids.iter().copied().collect())
    }

    /// The process ids, lowest first, for the handover.
    pub fn pids(&self) -> Vec<u32> {
        self.0.iter().copied().collect()
    }

    /// Adds `pid`. A second claim does not replace the first: Neovim's `:terminal` can run a
    /// second Neovim with the pane's variables. The claims whose process `alive` says has gone
    /// are dropped first, so the set stays short without a tick of its own.
    pub fn claim(&mut self, pid: u32, alive: impl Fn(u32) -> bool) {
        self.0.retain(|&held| alive(held));
        self.0.insert(pid);
    }

    /// Removes `pid`'s claim. Releasing a claim that is not held changes nothing.
    pub fn release(&mut self, pid: u32) {
        self.0.remove(&pid);
    }

    /// Whether a claimant is in front: its process group is `front`, the pane's foreground
    /// group.
    pub fn hold(&self, front: u32, group_of: impl Fn(u32) -> Option<u32>) -> bool {
        self.0.iter().any(|&pid| group_of(pid) == Some(front))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always(_: u32) -> bool {
        true
    }

    #[test]
    fn a_second_claim_keeps_the_first() {
        let mut claims = Claims::default();
        claims.claim(10, always);
        claims.claim(20, always);
        assert_eq!(claims.pids(), vec![10, 20]);
    }

    #[test]
    fn a_claim_drops_the_claims_whose_process_has_gone() {
        let mut claims = Claims::from_pids(&[10, 20]);
        claims.claim(30, |pid| pid != 10);
        assert_eq!(claims.pids(), vec![20, 30]);
    }

    #[test]
    fn a_release_removes_only_that_process_and_a_second_changes_nothing() {
        let mut claims = Claims::from_pids(&[10, 20]);
        claims.release(10);
        claims.release(10);
        assert_eq!(claims.pids(), vec![20]);
    }

    #[test]
    fn claims_hold_only_while_a_claimant_is_in_the_front_group() {
        let claims = Claims::from_pids(&[10, 20]);
        let group_of = |pid| match pid {
            10 => Some(100),
            20 => Some(200),
            _ => None,
        };
        assert!(claims.hold(200, group_of));
        assert!(!claims.hold(300, group_of), "the shell is in front");
        assert!(!claims.hold(100, |_| None), "the claimants have gone");
        assert!(!Claims::default().hold(100, group_of));
        assert!(Claims::default().is_empty());
    }
}
