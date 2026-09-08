//! The pull request provider: V1's `gh` command, V1's rules, and a bound on how long domux
//! waits for an answer.

use domux_core::facts::{FactKey, FactState, FACT_BRANCH, FACT_PR};
use domux_core::ids::WorkspaceId;
use domux_server::facts::pr::{parse_gh_pr_list, PrProvider, PR_INTERVAL, PR_TIMEOUT, PR_TTL};
use domux_server::facts::{default_providers, FactProvider, FactTarget, ProviderScope};
use domux_server::testing::finishes_within;
use domux_server::FixedClock;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Writes a `gh` at `dir/gh` that records its arguments beside itself and then runs `body`.
/// The provider is given the path, so no test changes PATH and no test races another.
fn fake_gh(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("gh");
    std::fs::write(
        &path,
        format!("#!/bin/sh\necho \"$@\" > \"$(dirname \"$0\")/args\"\n{body}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A `gh` that prints `json` and succeeds.
fn gh_printing(dir: &Path, json: &str) -> PathBuf {
    fake_gh(dir, &format!("cat <<'JSON'\n{json}\nJSON"))
}

/// A `gh` that says why it could not answer and fails, as the real one does when the forge
/// refuses it.
fn gh_failing(dir: &Path, message: &str) -> PathBuf {
    fake_gh(dir, &format!("echo '{message}' >&2\nexit 1"))
}

/// The one clock every target here is stamped with, so the fact's stamp can be checked
/// against it exactly rather than merely "parses as some time".
fn now() -> chrono::DateTime<chrono::Local> {
    FixedClock::at("2026-09-04T14:32:00").0
}

fn target(path: PathBuf, branch: &str) -> FactTarget {
    FactTarget {
        key: FactKey::workspace(&WorkspaceId("w_0001".into()), FACT_PR),
        path: path.clone(),
        root: path,
        default_branch: Some("main".into()),
        branch: Some(branch.to_string()),
        now: now(),
    }
}

#[test]
fn gh_output_becomes_a_pr_number_a_state_and_a_title() {
    let pr = parse_gh_pr_list(
        r#"[{"number":212,"state":"OPEN","title":"Consolidate auth middleware","isDraft":false}]"#,
    )
    .unwrap()
    .unwrap();
    assert_eq!(pr.number, 212);
    assert_eq!(pr.state, FactState::Open);
    assert_eq!(pr.title.as_deref(), Some("Consolidate auth middleware"));

    let draft = parse_gh_pr_list(
        r#"[{"number":9,"state":"OPEN","title":"Work in progress","isDraft":true}]"#,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        draft.state,
        FactState::Draft,
        "isDraft on an open pull request is DRAFT"
    );
    assert_eq!(draft.title.as_deref(), Some("Work in progress"));

    for (state, expected) in [
        ("MERGED", FactState::Merged),
        ("CLOSED", FactState::Closed),
        ("DRAFT", FactState::Draft),
    ] {
        let row = format!(r#"[{{"number":3,"state":"{state}","title":"x","isDraft":false}}]"#);
        assert_eq!(parse_gh_pr_list(&row).unwrap().unwrap().state, expected);
    }

    assert_eq!(
        parse_gh_pr_list("[]").unwrap(),
        None,
        "no pull request is absent, not zero"
    );
    assert_eq!(
        parse_gh_pr_list(r#"[{"number":7,"state":"OPEN","title":"  ","isDraft":false}]"#)
            .unwrap()
            .unwrap()
            .title,
        None,
        "a blank title is no title, not an empty one to draw"
    );
    assert!(
        parse_gh_pr_list("not json").is_err(),
        "output this domux cannot read is a reason, not a guess"
    );
}

#[test]
fn the_provider_runs_gh_with_v1_s_arguments_in_the_workspace_directory() {
    let dir = tempfile::tempdir().unwrap();
    let gh = gh_printing(
        dir.path(),
        r#"[{"number":212,"state":"OPEN","title":"Consolidate auth middleware","isDraft":false}]"#,
    );
    let provider = PrProvider::new(&gh);
    let fact = provider
        .fetch(&target(dir.path().to_path_buf(), "feat/auth-cleanup"))
        .unwrap()
        .unwrap();
    assert_eq!(fact.text, "PR#212");
    assert_eq!(fact.state, Some(FactState::Open));
    assert_eq!(
        fact.url.as_deref(),
        Some("Consolidate auth middleware"),
        "the title rides in url until section 5.5 draws it"
    );
    assert_eq!(
        fact.ttl, PR_TTL,
        "the registry expires the pull request on this time to live, so the constant must be \
         what it reads"
    );
    assert_eq!(
        fact.fetched_at,
        now().to_rfc3339(),
        "the provider stamps the target's own clock, not one it read itself, so it agrees \
         with whatever the registry's freshness check is measured against"
    );
    let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
    assert_eq!(
        args.trim(),
        "pr list --head feat/auth-cleanup --state all --limit 1 --json number,state,title,isDraft"
    );
    assert_eq!(provider.scope(), ProviderScope::Workspace);
    assert_eq!(provider.name(), FACT_PR);
    assert_eq!(provider.interval(), PR_INTERVAL);
}

#[test]
fn a_workspace_on_the_default_branch_and_one_with_no_branch_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let gh = gh_printing(dir.path(), "[]");
    let p = PrProvider::new(&gh);
    assert_eq!(
        p.fetch(&target(dir.path().to_path_buf(), "main")).unwrap(),
        None
    );
    assert!(
        !dir.path().join("args").exists(),
        "gh is not even run for the default branch: --head main matches every pull request \
         ever opened from it"
    );
    let mut t = target(dir.path().to_path_buf(), "feat/x");
    t.branch = None;
    assert_eq!(
        p.fetch(&t).unwrap(),
        None,
        "with no branch fact there is nothing to look up"
    );
    assert!(!dir.path().join("args").exists());
    let gone = target(dir.path().join("no-such-workspace"), "feat/x");
    assert_eq!(
        p.fetch(&gone).unwrap(),
        None,
        "a slot removed outside domux is absent, not an error the engineer sees"
    );
}

#[test]
fn gh_failing_or_missing_leaves_the_pull_request_absent_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let gh = gh_failing(dir.path(), "gh: could not find any commits");
    let err = PrProvider::new(&gh)
        .fetch(&target(dir.path().to_path_buf(), "feat/x"))
        .unwrap_err();
    assert!(err.contains("could not find any commits"), "{err}");
    assert!(
        err.contains("pr list --head feat/x"),
        "the reason says what domux ran: {err}"
    );

    let missing = dir.path().join("no-such-gh");
    let err = PrProvider::new(&missing)
        .fetch(&target(dir.path().to_path_buf(), "feat/x"))
        .unwrap_err();
    assert!(
        err.contains("no-such-gh"),
        "the reason names the tool that is missing: {err}"
    );
}

/// A pull request that never answers is worse than one that fails. `FactRegistry::due` skips
/// a key that is in flight, so an unbounded `gh` freezes that workspace's number until the
/// server is restarted and parks a blocking thread for good.
///
/// The regression is a hang, not a wrong answer, and a test binary cannot report "this did
/// not finish": without `finishes_within` around the call, losing the bound would stall the
/// whole suite silently instead of failing this test.
#[test]
fn gh_that_never_answers_is_stopped_rather_than_left_in_flight() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path(), "sleep 30");
    let t = target(dir.path().to_path_buf(), "feat/x");
    let started = Instant::now();
    let err = finishes_within(Duration::from_secs(5), "PrProvider::fetch", move || {
        PrProvider::new(&gh)
            .with_timeout(Duration::from_millis(300))
            .fetch(&t)
            .unwrap_err()
    });
    assert!(err.contains("no answer within"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the bound is what ended it, not gh: {:?}",
        started.elapsed()
    );
    assert_eq!(
        PrProvider::new("gh").timeout(),
        PR_TIMEOUT,
        "and the bound a real server runs with is the constant, not one a test passed in"
    );
}

/// The bound is only real if both pipes are drained while the child runs. A pipe holds about
/// 64KB, so a `gh` that prints a long authentication error blocks on the write and never
/// exits, and a fetch that waits for it to exit first waits forever. Small output passes
/// every test one would think to write; this is the one that fails.
#[test]
fn a_long_error_from_gh_reaches_the_reason_rather_than_blocking_on_a_full_pipe() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(
        dir.path(),
        "dd if=/dev/zero bs=1000 count=200 2>/dev/null | tr '\\0' 'x' >&2\n\
         echo 'gh: could not find any commits' >&2\nexit 1",
    );
    let t = target(dir.path().to_path_buf(), "feat/x");
    // Five seconds is far more than two small processes and 200KB of pipe need, so a failure
    // here is the drain and not a slow machine.
    let err = finishes_within(
        Duration::from_secs(15),
        "PrProvider::fetch against a gh that prints 200KB",
        move || {
            PrProvider::new(&gh)
                .with_timeout(Duration::from_secs(5))
                .fetch(&t)
                .unwrap_err()
        },
    );
    assert!(
        err.contains("could not find any commits"),
        "the last line of a long error is still the reason"
    );
    assert!(
        err.len() > 200_000,
        "and every byte before it arrived too, rather than filling the pipe and stopping"
    );
}

/// `provider.interval() == PR_INTERVAL` pins the wiring, not the value: it compares the
/// constant to itself and passes for any value, including ones that invert the doc comments.
/// These name the literals on purpose - do not fold them back into the constants - and check
/// the relationships the doc comments promise.
#[test]
fn the_pull_request_interval_time_to_live_and_bound_are_the_values_the_switcher_needs() {
    assert_eq!(PR_INTERVAL, Duration::from_secs(60));
    assert_eq!(PR_TTL, Duration::from_secs(600));
    assert_eq!(PR_TIMEOUT, Duration::from_secs(20));
    assert!(
        PR_TTL > PR_INTERVAL,
        "a number must survive from one look-up to the next, or the row appears and disappears"
    );
    assert!(
        PR_TIMEOUT >= Duration::from_secs(10),
        "a slow network is not a stuck gh: a bound this short would cut off real look-ups"
    );
    assert!(
        PR_TIMEOUT <= PR_INTERVAL / 2,
        "a stuck look-up must be killed and its key released before the next one is due, or \
         the workspace is skipped for as long as it stays in flight"
    );
}

#[test]
fn a_real_server_observes_the_branch_and_then_the_pull_request() {
    let providers = default_providers();
    let names: Vec<&str> = providers.iter().map(|p| p.name()).collect();
    assert_eq!(
        names,
        vec![FACT_BRANCH, FACT_PR],
        "the branch goes first: the pull request provider reads its answer off the target \
         rather than running git again"
    );
}
