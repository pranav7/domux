//! The pull request for a workspace's branch, through the `gh` CLI. V1's command and V1's
//! rules (`pr_cache.go`), with V2's registry around them: the pull request is the fact the
//! switcher was built for, and an extension's provider registers through the same trait
//! (architecture spec section 8).

use super::{FactProvider, FactTarget, ProviderScope};
use crate::subprocess;
use domux_core::facts::{Fact, FactState, FACT_PR};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// V1's `pickerPRRefreshInterval`.
pub const PR_INTERVAL: Duration = Duration::from_secs(60);
/// How long a cached number is worth drawing after a restart. Ten minutes, so the switcher
/// is never blank on start and never shows a number the morning has already changed.
pub const PR_TTL: Duration = Duration::from_secs(600);
/// How long `gh` may take before domux stops waiting for it.
///
/// `gh` talks to a forge over the network, and a network call does not always fail: it hangs.
/// An unbounded fetch never answers, so the registry holds its key in flight for the life of
/// the server and this workspace's pull request never changes again, and the blocking task it
/// runs on is parked for good.
///
/// Twenty seconds: a `gh pr list` that has not answered by then is stuck rather than slow,
/// because a healthy one answers in a second or two, so this is ten times the slowest good
/// case and no real look-up is ever cut off. It is a third of `PR_INTERVAL`, so a stuck
/// look-up is killed and its key released with forty seconds to spare before the next one is
/// due, and two fetches for one workspace can never overlap.
pub const PR_TIMEOUT: Duration = Duration::from_secs(20);

pub struct PrProvider {
    command: OsString,
    timeout: Duration,
}

impl PrProvider {
    pub fn new(command: impl Into<OsString>) -> PrProvider {
        PrProvider {
            command: command.into(),
            timeout: PR_TIMEOUT,
        }
    }

    /// `gh` from PATH, or the path in `DOMUX_GH` when the author points at another one.
    pub fn from_env() -> PrProvider {
        PrProvider::new(std::env::var_os("DOMUX_GH").unwrap_or_else(|| "gh".into()))
    }

    /// The same provider with a different bound on `gh`. A test uses it to prove the bound
    /// holds without waiting `PR_TIMEOUT` to see it.
    pub fn with_timeout(mut self, timeout: Duration) -> PrProvider {
        self.timeout = timeout;
        self
    }

    /// How long this provider gives `gh`, so a test can see that the bound was applied at
    /// all rather than only that a short one works.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// The command as it reads in an error, so the engineer sees what domux ran and which
    /// binary it ran (principle 9).
    fn named(&self, branch: &str) -> String {
        format!(
            "{} pr list --head {branch}",
            Path::new(&self.command).display()
        )
    }
}

impl FactProvider for PrProvider {
    fn name(&self) -> &str {
        FACT_PR
    }

    fn interval(&self) -> Duration {
        PR_INTERVAL
    }

    fn scope(&self) -> ProviderScope {
        ProviderScope::Workspace
    }

    fn fetch(&self, target: &FactTarget) -> Result<Option<Fact>, String> {
        // The branch fact has not arrived, or the workspace has no branch: there is nothing
        // to look a pull request up by, and asking `gh` without a head would answer about
        // some other branch entirely.
        let Some(branch) = target.branch.as_deref() else {
            return Ok(None);
        };
        if Some(branch) == target.default_branch.as_deref() {
            // `gh pr list --head main` matches any pull request ever opened from the
            // default branch, and the row would carry a number that never clears (V1
            // learned this; the comment is in `currentPRRefreshSessions`).
            return Ok(None);
        }
        if Some(branch) == target.handle.as_deref() {
            // A slot handle is recycled the same way: `workspace-4` is the name slot 4 rests
            // on between jobs, so every piece of work that ever passed through the slot
            // branched off it and `--head workspace-4` answers with whichever of them `gh`
            // saw last. `Workspace::is_untouched` already reads a slot on its own branch as
            // having done nothing, so a number here contradicts the model and costs the row
            // the `◌` that says the slot is free.
            return Ok(None);
        }
        // A workspace whose slot was removed outside domux is absent, not an error: `gh`
        // cannot run in a directory that is gone.
        if !target.path.is_dir() {
            return Ok(None);
        }
        let mut command = Command::new(&self.command);
        command
            .args([
                "pr",
                "list",
                "--head",
                branch,
                "--state",
                "all",
                "--limit",
                "1",
                "--json",
                "number,state,title,isDraft",
            ])
            .current_dir(&target.path);
        let out = subprocess::output_within(command, self.timeout)
            .map_err(|e| format!("{}: {e}", self.named(branch)))?;
        if !out.status.success() {
            let mut message = String::from_utf8_lossy(&out.stderr).trim().to_string();
            if message.is_empty() {
                message = String::from_utf8_lossy(&out.stdout).trim().to_string();
            }
            return Err(format!("{}: {message}", self.named(branch)));
        }
        let Some(pr) = parse_gh_pr_list(&String::from_utf8_lossy(&out.stdout))? else {
            return Ok(None);
        };
        let fact = Fact::new(
            format!("PR#{}", pr.number),
            Some(pr.state),
            // The core's clock, not this thread's: it is what the registry's freshness check
            // is measured against, and a provider that reads its own clock can disagree with
            // it (a fixed clock in a test is the case that bites).
            target.now.to_rfc3339(),
            PR_TTL,
        );
        Ok(Some(match pr.title {
            Some(title) => fact.with_url(title),
            None => fact,
        }))
    }
}

/// One row of `gh pr list`, read but not yet stamped. The clock a fact carries is the core's
/// and it reaches the provider on the target, so the parser never sees one and never reads
/// one of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub state: FactState,
    /// The title, when `gh` gave one worth drawing. The switcher draws it after the number
    /// when the width allows (interface spec 5.5), and it travels in `Fact.url`, the free
    /// text slot the Layer B contract gives every provider.
    pub title: Option<String>,
}

/// V1's `parseGHPRList`. `Ok(None)` is "this branch has no pull request", which renders as
/// nothing at all, never as a zero or a closed one (principle 4).
pub fn parse_gh_pr_list(json: &str) -> Result<Option<PullRequest>, String> {
    #[derive(serde::Deserialize)]
    struct Row {
        number: u64,
        state: String,
        #[serde(default)]
        title: String,
        #[serde(rename = "isDraft", default)]
        is_draft: bool,
    }
    let rows: Vec<Row> = serde_json::from_str(json.trim())
        .map_err(|e| format!("gh printed something this domux cannot read: {e}"))?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let mut state: FactState = row.state.parse().expect("FactState::from_str never fails");
    // `gh` reports a draft as an open pull request with a flag beside it. The switcher
    // colours a draft differently from an open one, so the flag becomes the state here.
    if row.is_draft && state == FactState::Open {
        state = FactState::Draft;
    }
    // A title is one row on the screen, so a line break in it would break the row it sits in.
    let title = row.title.replace(['\n', '\r'], " ").trim().to_string();
    Ok(Some(PullRequest {
        number: row.number,
        state,
        title: (!title.is_empty()).then_some(title),
    }))
}
