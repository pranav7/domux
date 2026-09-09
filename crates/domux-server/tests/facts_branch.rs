mod support;

use domux_core::facts::{FactKey, FACT_BRANCH};
use domux_core::ids::WorkspaceId;
use domux_server::facts::branch::{BranchProvider, BRANCH_INTERVAL, BRANCH_TTL};
use domux_server::facts::{FactProvider, FactTarget, ProviderScope};
use domux_server::git;
use domux_server::FixedClock;
use std::path::PathBuf;
use std::time::Duration;
use support::repo_with_origin;

/// The one clock every target in this file is stamped with, so `fetch`'s answer can be
/// checked against it exactly rather than merely "parses as some time".
fn now() -> chrono::DateTime<chrono::Local> {
    FixedClock::at("2026-09-04T14:32:00").0
}

fn target(path: PathBuf, root: PathBuf) -> FactTarget {
    FactTarget {
        key: FactKey::workspace(&WorkspaceId("w_0001".into()), FACT_BRANCH),
        path,
        root,
        default_branch: Some("main".into()),
        branch: None,
        now: now(),
    }
}

#[test]
fn the_branch_provider_reports_the_workspace_s_branch() {
    let (_tmp, repo) = repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();
    let p = BranchProvider;
    assert_eq!(p.name(), FACT_BRANCH);
    assert_eq!(p.scope(), ProviderScope::Workspace);
    assert_eq!(p.interval(), BRANCH_INTERVAL);
    let fact = p.fetch(&target(path, repo.clone())).unwrap().unwrap();
    assert_eq!(fact.text, "workspace-1");
    assert_eq!(
        fact.state, None,
        "a branch has no state; only the pull request has one"
    );
    assert_eq!(
        fact.ttl, BRANCH_TTL,
        "the registry expires a branch fact on this time to live, so the constant must be what it reads"
    );
    assert_eq!(
        fact.fetched_at,
        now().to_rfc3339(),
        "the provider stamps the target's own clock, not one it read itself, so it agrees \
         with whatever the registry's freshness check is measured against"
    );
}

/// `p.interval() == BRANCH_INTERVAL` and `fact.ttl == BRANCH_TTL` above pin the wiring (the
/// getter and the fact both read the constant), but not the value: changing what the
/// constant *is* leaves both of those green, because they compare it to itself. This test
/// names the literals on purpose - do not fold them back into `BRANCH_INTERVAL`/`BRANCH_TTL`,
/// that would make it the same tautology again - and checks the relationship the doc comments
/// promise: a fact must outlive the interval it is refetched on, or a row blinks absent
/// between one fetch and the next.
#[test]
fn the_branch_time_to_live_outlasts_its_interval_so_a_row_never_blinks() {
    assert_eq!(BRANCH_INTERVAL, Duration::from_secs(5));
    assert_eq!(BRANCH_TTL, Duration::from_secs(30));
    assert!(
        BRANCH_TTL > BRANCH_INTERVAL,
        "a fact must survive from one fetch to the next, or the row appears and disappears"
    );
}

#[test]
fn a_path_that_is_gone_or_is_not_a_repository_leaves_the_branch_absent() {
    let p = BranchProvider;
    let gone = target(
        PathBuf::from("/nonexistent/workspace-9"),
        PathBuf::from("/nonexistent"),
    );
    assert_eq!(
        p.fetch(&gone).unwrap(),
        None,
        "a missing path is absent, not an error the engineer sees"
    );
    let plain = tempfile::tempdir().unwrap();
    let not_a_repo = target(plain.path().to_path_buf(), plain.path().to_path_buf());
    assert_eq!(p.fetch(&not_a_repo).unwrap(), None);
}

#[test]
fn a_detached_head_reports_no_branch_rather_than_the_word_head() {
    let (_tmp, repo) = repo_with_origin("main");
    let sha = support::git(&repo, &["rev-parse", "HEAD"]);
    support::git(&repo, &["checkout", "-q", &sha]);
    let fact = BranchProvider
        .fetch(&target(repo.clone(), repo.clone()))
        .unwrap();
    assert_eq!(
        fact, None,
        "HEAD is not a branch name and must not render as one"
    );
}

#[test]
fn a_repository_before_its_first_commit_is_an_error_not_a_silent_absence() {
    // `is_repo` is true here (there is a `.git` directory to work in), but HEAD resolves to
    // nothing yet, so `branch_of` fails. That is different from "there is no branch": it is
    // a real problem worth a log line, not a workspace domux should quietly say nothing
    // about.
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().to_path_buf();
    support::git(&repo, &["init", "-q", "-b", "main"]);
    let fact = BranchProvider.fetch(&target(repo.clone(), repo.clone()));
    assert!(
        fact.is_err(),
        "a repository with no commits yet is an error, not the same absence as a plain folder"
    );
}
