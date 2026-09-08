//! `Harness::fact` and `Harness::wait_for_fact`: reading a published fact through a running
//! server, and waiting for one to arrive instead of sleeping a guessed-at duration.

mod support;

use domux_core::config::Config;
use domux_core::facts::{FactKey, FACT_BRANCH};
use domux_core::ids::WorkspaceId;
use domux_core::model::Model;
use domux_core::state_file;
use domux_server::facts::branch::BranchProvider;
use domux_server::git;
use domux_server::testing::{Harness, HarnessOptions};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// A real git worktree at slot 1 (branch `workspace-1`), registered in a `state.json` a
/// harness can load on start. `workspace.create` is not built yet, so this is how a test
/// gets a project and a workspace onto the model without it. The two temp directories must
/// outlive the harness; the caller keeps them bound in scope.
fn seeded_workspace() -> (tempfile::TempDir, tempfile::TempDir, PathBuf, WorkspaceId) {
    let (repo_tmp, repo) = support::repo_with_origin("main");
    let path = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &path, "workspace-1", "origin/main").unwrap();

    let mut model = Model::new(1);
    let (pid, _main, _events) = model.add_git_project(repo, "main".into()).unwrap();
    let (workspace, _events) = model.add_slot(&pid, 1, path).unwrap();

    let state_tmp = tempfile::tempdir().unwrap();
    let state_dir = state_tmp.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(
        state_dir.join("state.json"),
        state_file::to_json(&state_file::snapshot(&model, "2026-09-04T14:32:00+00:00")),
    )
    .unwrap();
    (repo_tmp, state_tmp, state_dir, workspace)
}

#[tokio::test]
async fn a_harness_test_can_ask_for_the_branch_provider_and_wait_for_its_fact() {
    let (_repo_tmp, _state_tmp, state_dir, workspace) = seeded_workspace();
    let h = Harness::start_with(HarnessOptions {
        state_dir: Some(state_dir),
        providers: vec![Arc::new(BranchProvider)],
        ..HarnessOptions::new(Config::default(), 80, 24)
    })
    .await;

    let key = FactKey::workspace(&workspace, FACT_BRANCH);
    let fact = h
        .wait_for_fact(&key, |f| f.is_some(), Duration::from_secs(5))
        .await
        .expect("the branch provider answered");
    assert_eq!(fact.text, "workspace-1");

    // A one-shot read, once the fact has arrived, sees the same value `wait_for_fact` already
    // returned: `fact` does not have to wait again for something already published.
    assert_eq!(h.fact(&key), Some(fact));
}

#[tokio::test]
async fn a_harness_test_with_no_providers_never_sees_a_branch_fact() {
    // The default (`HarnessOptions::new`'s empty `providers`) is what `ServerOptions.providers`
    // documents: a test that does not ask for a provider never shells out to git, even with a
    // real git project sitting right there on disk.
    let (_repo_tmp, _state_tmp, state_dir, workspace) = seeded_workspace();
    let h = Harness::start_with(HarnessOptions {
        state_dir: Some(state_dir),
        ..HarnessOptions::new(Config::default(), 80, 24)
    })
    .await;

    let key = FactKey::workspace(&workspace, FACT_BRANCH);
    // Long enough to see the provider miss several ticks were one registered; short enough
    // this test does not sit around proving a negative.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(h.fact(&key), None);
}

#[tokio::test]
#[should_panic(expected = "not met within")]
async fn wait_for_fact_panics_when_the_condition_never_holds_within_the_timeout() {
    let h = Harness::start(Config::default(), 80, 24).await;
    let key = FactKey::workspace(&WorkspaceId("w_9999".into()), FACT_BRANCH);
    // No provider is registered (the default), so this key can never become present: the
    // deadline, not the predicate, is what ends the wait.
    h.wait_for_fact(&key, |f| f.is_some(), Duration::from_millis(50))
        .await;
}

/// The panic above only proves `wait_for_fact` gives up correctly; it does not prove what
/// happens if the deadline check itself is ever lost, because that failure mode is not a
/// wrong answer but a wait that never returns. A bug there would not fail this suite, it
/// would hang whatever CI job ran it. `tokio::spawn` catches the awaited call's panic as an
/// `Err` instead of unwinding this test, and the outer `tokio::time::timeout` turns "the
/// spawned call never finished" into a normal, fast test failure rather than a stuck job.
#[tokio::test]
async fn wait_for_fact_gives_up_on_its_own_rather_than_hanging_the_test_that_calls_it() {
    let h = Harness::start(Config::default(), 80, 24).await;
    let key = FactKey::workspace(&WorkspaceId("w_9999".into()), FACT_BRANCH);
    let called = tokio::spawn(async move {
        h.wait_for_fact(&key, |f| f.is_some(), Duration::from_millis(50))
            .await;
    });
    match tokio::time::timeout(Duration::from_secs(2), called).await {
        Ok(join) => assert!(
            join.is_err(),
            "wait_for_fact returned instead of panicking on a condition that never held"
        ),
        Err(_) => panic!(
            "wait_for_fact did not give up within its own timeout; it hung well past it instead"
        ),
    }
}
