//! `Harness::fact` and `Harness::wait_for_fact`: reading a published fact through a running
//! server, and waiting for one to arrive instead of sleeping a guessed-at duration.

mod support;

use domux_core::config::Config;
use domux_core::facts::{FactKey, FACT_BRANCH};
use domux_core::ids::WorkspaceId;
use domux_core::model::Model;
use domux_core::state_file;
use domux_server::facts::branch::BranchProvider;
use domux_server::testing::{Harness, HarnessOptions};
use std::sync::Arc;
use std::time::Duration;

/// A harness holding a git project with `workspace-1` in it, made the way the server makes
/// one. Returns the harness and the new workspace's id.
///
/// This used to hand-build a worktree and write a `state.json` for the harness to load,
/// because `workspace.create` did not exist. It does now, and a fixture the server built is
/// the fixture these tests want: a provider reads the model and the disk, and the two agree
/// here because one call made both.
async fn harness_with_a_slot(
    providers: Vec<Arc<dyn domux_server::facts::FactProvider>>,
) -> (Harness, WorkspaceId) {
    let mut h = Harness::start_with(HarnessOptions {
        providers,
        ..HarnessOptions::new(Config::default(), 80, 24)
    })
    .await;
    h.git_project("main").await;
    let made = h
        .api(
            "workspace.create",
            serde_json::json!({"project": "audrey-app"}),
        )
        .await
        .expect("workspace.create");
    let workspace = WorkspaceId(made["id"].as_str().expect("an id").to_string());
    (h, workspace)
}

#[tokio::test]
async fn a_harness_test_can_ask_for_the_branch_provider_and_wait_for_its_fact() {
    let (h, workspace) = harness_with_a_slot(vec![Arc::new(BranchProvider)]).await;

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

/// The harness runs on `FixedClock`, which never advances. `BranchProvider` stamps
/// `fact.fetched_at` with the target's `now`, which is the same clock reading the registry
/// measures freshness against (`facts::FactTarget::now`, `is_fresh_at`), so the two can never
/// disagree about how much time has passed: under a clock that has not moved, the answer is
/// always zero, and the fact must still be there. Before that field existed the provider read
/// its own wall clock instead, which was always chronologically ahead of the frozen one, so
/// the very first tick judged the fact "stamped in the future" and dropped it, and it never
/// came back because the interval math used the same frozen clock too. This test would have
/// failed against that code well inside the sleep below.
#[tokio::test]
async fn a_branch_fact_stays_fresh_across_several_ticks_under_the_harness_s_fixed_clock() {
    let (h, workspace) = harness_with_a_slot(vec![Arc::new(BranchProvider)]).await;

    let key = FactKey::workspace(&workspace, FACT_BRANCH);
    h.wait_for_fact(&key, |f| f.is_some(), Duration::from_secs(5))
        .await
        .expect("the branch provider answered");
    // The server ticks once a second; three and a half real seconds spans several of them,
    // which is exactly when the old behaviour had already dropped the fact for good.
    tokio::time::sleep(Duration::from_millis(3500)).await;
    let fact = h.fact(&key);
    assert_eq!(
        fact.as_ref().map(|f| f.text.as_str()),
        Some("workspace-1"),
        "the fact must not expire under a clock that has not moved: {fact:?}"
    );
}

#[tokio::test]
async fn a_harness_test_with_no_providers_never_sees_a_branch_fact() {
    // The default (`HarnessOptions::new`'s empty `providers`) is what `ServerOptions.providers`
    // documents: a test that does not ask for a provider never shells out to git, even with a
    // real git project sitting right there on disk.
    let (h, workspace) = harness_with_a_slot(Vec::new()).await;

    let key = FactKey::workspace(&workspace, FACT_BRANCH);
    // Long enough to see the provider miss several ticks were one registered; short enough
    // this test does not sit around proving a negative.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(h.fact(&key), None);
}

/// A bare `#[should_panic]` around this call is not enough: if the deadline check itself is
/// ever lost, `wait_for_fact` does not return the wrong thing, it never returns, and that
/// failure mode does not fail this test, it hangs the binary (the review measured this: the
/// sibling test alone printed `FAILED` at 2s under the mutation and the run then never
/// finished). The outer `tokio::time::timeout` bounds it from the outside: the correct
/// implementation panics with "not met within" well inside the two seconds, which satisfies
/// `should_panic` before the outer timeout ever matters; a lost deadline check instead trips
/// the outer timeout, whose own panic message does not contain "not met within", so
/// `should_panic` reports a normal, fast failure instead of a hang.
#[tokio::test]
#[should_panic(expected = "not met within")]
async fn wait_for_fact_panics_when_the_condition_never_holds_within_the_timeout() {
    let h = Harness::start(Config::default(), 80, 24).await;
    let key = FactKey::workspace(&WorkspaceId("w_9999".into()), FACT_BRANCH);
    // No provider is registered (the default), so this key can never become present: the
    // deadline, not the predicate, is what ends the wait.
    tokio::time::timeout(
        Duration::from_secs(2),
        h.wait_for_fact(&key, |f| f.is_some(), Duration::from_millis(50)),
    )
    .await
    .expect("wait_for_fact hung past its own timeout");
}

/// A fact a provider found reaches a drawn frame, so the renderer is handed the core's own
/// registry rather than any registry of the right type.
///
/// The claim needs its own test because "some `FactRegistry` was present" and "the one the
/// core fills" are different claims, and the whole suite was satisfied by the first: with
/// `RenderInput.facts` replaced by a fresh empty registry, every render test still passed,
/// because none of them had a provider running behind the frame they asserted on. This one
/// registers the branch provider, waits for its answer to be published, and then waits for
/// that same text to appear on the screen.
///
/// `develop` is the value to look for because nothing else this fixture draws can produce
/// it: the project's name is `audrey-app` and the workspace is `main`, the project's
/// default branch is not drawn anywhere, and no path is. It is not that a branch line could
/// never say something a frame says elsewhere.
#[tokio::test]
async fn a_branch_fact_a_provider_found_reaches_the_drawn_frame() {
    let (_repo_tmp, repo) = support::repo_with_origin("develop");
    let mut model = Model::new(1);
    let (_pid, main, _events) = model.add_git_project(repo, "develop".into()).unwrap();

    let state_tmp = tempfile::tempdir().unwrap();
    let state_dir = state_tmp.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(
        state_dir.join("state.json"),
        state_file::to_json(&state_file::snapshot(&model, "2026-09-04T14:32:00+00:00")),
    )
    .unwrap();

    // 120 columns: the width at which the sidebar draws itself (interface spec 12.1).
    let mut h = Harness::start_with(HarnessOptions {
        state_dir: Some(state_dir),
        providers: vec![Arc::new(BranchProvider)],
        ..HarnessOptions::new(Config::default(), 120, 24)
    })
    .await;
    let key = FactKey::workspace(&main, FACT_BRANCH);
    let fact = h
        .wait_for_fact(&key, |f| f.is_some(), Duration::from_secs(5))
        .await
        .expect("the branch provider answered");
    assert_eq!(fact.text, "develop", "the fact the frame should carry");

    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("develop"),
            Duration::from_secs(5),
        )
        .await;
    assert!(
        f.contains("AUDREY-APP"),
        "and it is the Projects box it reached:\n{f}"
    );
}
