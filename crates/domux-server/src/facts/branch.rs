//! The branch a workspace is on. Cheap, so it runs often; not cached to disk, because a
//! branch read from a file after a restart could be a week old and the real one is one
//! `git rev-parse` away.

use super::{FactProvider, FactTarget, ProviderScope};
use chrono::Local;
use domux_core::facts::{Fact, FACT_BRANCH};
use std::time::Duration;

/// V1 refreshes its switcher rows every two seconds while it is open. V2's server has no
/// "open", so five seconds keeps the row honest without a git process per workspace per
/// second.
pub const BRANCH_INTERVAL: Duration = Duration::from_secs(5);
/// Long enough that a fact survives between two ticks, short enough that nothing stale is
/// ever drawn. The branch is never written to the cache file.
pub const BRANCH_TTL: Duration = Duration::from_secs(30);

pub struct BranchProvider;

impl FactProvider for BranchProvider {
    fn name(&self) -> &str {
        FACT_BRANCH
    }

    fn interval(&self) -> Duration {
        BRANCH_INTERVAL
    }

    fn scope(&self) -> ProviderScope {
        ProviderScope::Workspace
    }

    fn fetch(&self, target: &FactTarget) -> Result<Option<Fact>, String> {
        // A missing path and a directory that is not a repository are both absent, never an
        // error the engineer sees: a workspace whose slot was removed outside domux, or a
        // plain folder project mistakenly asked about, both answer "no branch" rather than
        // shell error text.
        if !target.path.is_dir() || !crate::git::is_repo(&target.path) {
            return Ok(None);
        }
        match crate::git::branch_of(&target.path) {
            // A detached head prints `HEAD`, which is not a branch, so the fact is absent
            // (never fabricate a name git did not give).
            Ok(branch) if branch != "HEAD" && !branch.is_empty() => Ok(Some(Fact::new(
                branch,
                None,
                Local::now().to_rfc3339(),
                BRANCH_TTL,
            ))),
            Ok(_) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}
