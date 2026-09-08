//! The fact registry: what domux observed, who observes it, and when. Providers run off the
//! core task on blocking tasks; the core only reads the answers.
//!
//! The two built-in providers are Tasks 8 and 9. Their `pub mod` lines go in with the files
//! they name, in those tasks: a `pub mod branch;` here with no `branch.rs` beside it is a
//! compile error, and this task has to end with a green workspace build.

pub mod branch;

use chrono::{DateTime, Local};
use domux_core::facts::{Fact, FactKey, FactScope};
use domux_core::ids::WorkspaceId;
use domux_core::model::Model;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The pull request cache, beside `state.json` in the state directory. The one place the
/// name is written for a server that has its state directory but not the whole environment
/// `domux_core::paths` reads.
pub fn pr_cache_path(state_dir: &Path) -> PathBuf {
    state_dir.join("pr-cache.json")
}

/// The pull request cache as it sits on disk (architecture spec section 5).
///
/// The values stay as json until each one is read, so one unreadable entry drops itself
/// rather than the whole file, and one hand-edited character does not cost the switcher
/// every number it knows.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CachedFacts {
    pub schema_version: u32,
    #[serde(default)]
    pub facts: BTreeMap<String, serde_json::Value>,
}

impl CachedFacts {
    /// The shape above. A file that says anything else was written by another version of
    /// domux and is not read: the numbers in it come back in one interval anyway.
    pub const SCHEMA_VERSION: u32 = 1;
}

/// True while `fact` is inside its time to live at `now`. A stamp that will not parse, and
/// one that lies ahead of `now`, both leave the age unknown, and an unknown age is not a
/// fresh one: the fact is stale rather than trusted.
fn is_fresh_at(fact: &Fact, now: DateTime<Local>) -> bool {
    fact.fetched_at
        .parse::<DateTime<chrono::FixedOffset>>()
        .ok()
        .and_then(|t| now.signed_duration_since(t).to_std().ok())
        .is_some_and(|age| fact.is_fresh(age))
}

/// Whether the object a key is about is still in the model. One rule for the sweep and for
/// the answer of a fetch that was already running when its workspace went away.
pub fn scope_lives(key: &FactKey, model: &Model) -> bool {
    match &key.scope {
        FactScope::Workspace(id) => model.workspace(id).is_some(),
        FactScope::Project(id) => model.project(id).is_some(),
        FactScope::Server => true,
    }
}

/// The providers a real server runs. One list, in one file, so adding a provider is one
/// edit and no call site changes. The branch provider goes first because the pull request
/// provider (Task 9) reads its answer off the target rather than running git again.
pub fn default_providers() -> Vec<Arc<dyn FactProvider>> {
    vec![Arc::new(branch::BranchProvider)]
}

/// What a provider watches. The registry walks the model and builds one target per object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderScope {
    Workspace,
    Project,
    Server,
}

/// One thing to look at, with everything a provider needs so it never reads the model.
#[derive(Debug, Clone)]
pub struct FactTarget {
    pub key: FactKey,
    /// The workspace's or project's path on disk.
    pub path: PathBuf,
    /// The project's main checkout, for a provider that needs the repository root.
    pub root: PathBuf,
    /// The project's default branch, so a provider can skip a workspace sitting on it.
    pub default_branch: Option<String>,
    /// The branch fact if one has arrived, so the pull request provider does not shell out
    /// to git a second time.
    pub branch: Option<String>,
}

/// A command or a built-in that produces one fact about one target on an interval
/// (architecture spec section 8, Layer B). The pull request provider is the first one, and
/// an extension's provider registers through the same trait.
pub trait FactProvider: Send + Sync {
    /// The second half of every key this provider fills: `branch`, `pr`.
    fn name(&self) -> &str;
    /// How often to look again.
    fn interval(&self) -> Duration;
    fn scope(&self) -> ProviderScope;
    /// Runs on a blocking task. `Ok(None)` means the fact is absent, which is different
    /// from `Err`: absent renders as nothing, an error also writes to the log.
    fn fetch(&self, target: &FactTarget) -> Result<Option<Fact>, String>;
}

/// Who observes what, what has arrived, and when each target was last looked at.
///
/// A fact is present or it is absent. There is no third value and no default: a target that
/// has never been fetched, one whose provider failed and one whose provider answered "there
/// is none" are all the same absence here, and every reader renders an absence as nothing
/// rather than as a zero, a closed pull request or a clean worktree (principle 4).
#[derive(Default)]
pub struct FactRegistry {
    providers: Vec<Arc<dyn FactProvider>>,
    facts: HashMap<FactKey, Fact>,
    /// When each target was last started, which is what the interval is measured from.
    started: HashMap<FactKey, DateTime<Local>>,
    /// Targets whose provider is running right now.
    inflight: HashSet<FactKey>,
}

impl FactRegistry {
    pub fn new() -> FactRegistry {
        FactRegistry::default()
    }

    pub fn register(&mut self, provider: Arc<dyn FactProvider>) {
        self.providers.push(provider);
    }

    pub fn get(&self, key: &FactKey) -> Option<&Fact> {
        self.facts.get(key)
    }

    /// Every fact about one workspace, for the Projects box and `workspace.list`. By name,
    /// so two calls with the same facts give the same order and a frame does not depend on
    /// how a hash map happened to lay out.
    pub fn all_for(&self, workspace: &WorkspaceId) -> Vec<(&str, &Fact)> {
        let mut out: Vec<(&str, &Fact)> = self
            .facts
            .iter()
            .filter(|(k, _)| k.workspace_id() == Some(workspace))
            .map(|(k, f)| (k.name.as_str(), f))
            .collect();
        out.sort_by_key(|(name, _)| *name);
        out
    }

    pub fn len(&self) -> usize {
        self.facts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// Records an answer. `None` removes the fact: a fetch that failed leaves nothing
    /// behind, never a stale or guessed value (principle 4).
    pub fn set(&mut self, key: FactKey, fact: Option<Fact>) {
        self.inflight.remove(&key);
        match fact {
            Some(f) => {
                self.facts.insert(key, f);
            }
            None => {
                self.facts.remove(&key);
            }
        }
    }

    /// Drops everything about a workspace or a project the model no longer holds.
    ///
    /// Ids are drawn from a 16 bit space and `retire` only remembers the closed ones while
    /// the server runs, so a workspace deleted in one run can have its id redrawn in the
    /// next. Without this, the cache would hand the new workspace the deleted one's pull
    /// request, which is the one thing facts must never do.
    pub fn forget_deleted(&mut self, model: &Model) {
        let lives = |key: &FactKey| scope_lives(key, model);
        self.facts.retain(|k, _| lives(k));
        self.started.retain(|k, _| lives(k));
        self.inflight.retain(lives);
    }

    /// Drops every fact that is past its time to live and returns what it dropped, so the
    /// caller can say the value is gone.
    ///
    /// A fact is what domux observed, and an observation has an age. Without this sweep the
    /// time to live would only ever be read when the cache file is loaded, and a pull
    /// request seen an hour ago would be drawn exactly like one seen a second ago (principle
    /// 4). The provider whose answer stopped arriving is the case this covers: one that
    /// answers, even with an error, clears or replaces its fact by itself.
    pub fn expire(&mut self, now: DateTime<Local>) -> Vec<FactKey> {
        let stale: Vec<FactKey> = self
            .facts
            .iter()
            .filter(|(_, f)| !is_fresh_at(f, now))
            .map(|(k, _)| k.clone())
            .collect();
        for key in &stale {
            self.facts.remove(key);
        }
        stale
    }

    /// Which provider and target pairs to start now. A target already in flight is skipped,
    /// so a slow `gh` never queues behind itself.
    pub fn due(
        &mut self,
        model: &Model,
        now: DateTime<Local>,
    ) -> Vec<(Arc<dyn FactProvider>, FactTarget)> {
        let mut out = Vec::new();
        for provider in &self.providers {
            for target in targets(model, provider.as_ref(), &self.facts) {
                if self.inflight.contains(&target.key) {
                    continue;
                }
                let ready = match self.started.get(&target.key) {
                    Some(last) => match now.signed_duration_since(*last).to_std() {
                        Ok(age) => age >= provider.interval(),
                        // The clock went backwards, so the age is negative and there is no
                        // interval to compare it against. Look again now rather than
                        // holding every fact until the clock has caught up with itself.
                        Err(_) => true,
                    },
                    None => true,
                };
                if ready {
                    self.started.insert(target.key.clone(), now);
                    self.inflight.insert(target.key.clone());
                    out.push((provider.clone(), target));
                }
            }
        }
        out
    }

    /// Writes the named providers' facts to `path` (the pull request cache is its own file,
    /// architecture spec section 5). Atomic, through the same writer the state file uses.
    pub fn save_cache(&self, path: &Path, names: &[&str]) {
        // Ordered, so the file only changes when a fact does.
        let mut facts = BTreeMap::new();
        for (key, fact) in self
            .facts
            .iter()
            .filter(|(k, _)| names.contains(&k.name.as_str()))
        {
            match serde_json::to_value(fact) {
                Ok(value) => {
                    facts.insert(key.to_string(), value);
                }
                Err(e) => tracing::warn!("{key} could not be cached: {e}"),
            }
        }
        let text = serde_json::to_string_pretty(&CachedFacts {
            schema_version: CachedFacts::SCHEMA_VERSION,
            facts,
        })
        .unwrap_or_else(|_| "{}".into());
        if let Err(e) = crate::persist::write_atomic(path, &text) {
            tracing::warn!(
                "could not write the pull request cache at {}: {e}",
                path.display()
            );
        }
    }

    /// Reads the cache, dropping anything past its time to live, so the switcher shows the
    /// last known pull request on start instead of a blank line, and never shows a number
    /// that has had a week to change.
    pub fn load_cache(&mut self, path: &Path, now: DateTime<Local>) {
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(cache) = serde_json::from_str::<CachedFacts>(&text) else {
            tracing::warn!(
                "the pull request cache at {} is not readable; starting without it",
                path.display()
            );
            return;
        };
        if cache.schema_version != CachedFacts::SCHEMA_VERSION {
            tracing::warn!(
                "the pull request cache at {} is version {} and this server writes version {}; starting without it",
                path.display(),
                cache.schema_version,
                CachedFacts::SCHEMA_VERSION
            );
            return;
        }
        for (key, fact) in cache.facts {
            let (Ok(key), Ok(fact)) =
                (key.parse::<FactKey>(), serde_json::from_value::<Fact>(fact))
            else {
                continue;
            };
            if is_fresh_at(&fact, now) {
                self.facts.insert(key, fact);
            }
        }
    }
}

/// One target per object in the provider's scope. The branch fact, when it is already
/// known, rides along so the pull request provider does not run git again.
fn targets(
    model: &Model,
    provider: &dyn FactProvider,
    facts: &HashMap<FactKey, Fact>,
) -> Vec<FactTarget> {
    if provider.scope() == ProviderScope::Server {
        return vec![FactTarget {
            key: FactKey::server(provider.name()),
            path: PathBuf::new(),
            root: PathBuf::new(),
            default_branch: None,
            branch: None,
        }];
    }
    let mut out = Vec::new();
    for project in &model.projects {
        let default_branch = match &project.kind {
            domux_core::model::ProjectKind::Git { default_branch } => Some(default_branch.clone()),
            // A plain folder has no branch and no pull request.
            domux_core::model::ProjectKind::Folder => continue,
        };
        match provider.scope() {
            ProviderScope::Project => out.push(FactTarget {
                key: FactKey::project(&project.id, provider.name()),
                path: project.root.clone(),
                root: project.root.clone(),
                default_branch: default_branch.clone(),
                branch: None,
            }),
            ProviderScope::Workspace => {
                for w in &project.workspaces {
                    let branch = facts
                        .get(&FactKey::workspace(&w.id, domux_core::facts::FACT_BRANCH))
                        .map(|f| f.text.clone());
                    out.push(FactTarget {
                        key: FactKey::workspace(&w.id, provider.name()),
                        path: w.path.clone(),
                        root: project.root.clone(),
                        default_branch: default_branch.clone(),
                        branch,
                    });
                }
            }
            ProviderScope::Server => unreachable!("returned above"),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_core::facts::{FactState, FACT_BRANCH, FACT_PR};
    use domux_core::model::Model;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counter {
        name: String,
        interval: Duration,
        calls: AtomicUsize,
        answer: Option<Fact>,
    }

    impl FactProvider for Counter {
        fn name(&self) -> &str {
            &self.name
        }
        fn interval(&self) -> Duration {
            self.interval
        }
        fn scope(&self) -> ProviderScope {
            ProviderScope::Workspace
        }
        fn fetch(&self, _t: &FactTarget) -> Result<Option<Fact>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.answer.clone())
        }
    }

    fn model_with_two_workspaces() -> Model {
        let mut m = Model::new(7);
        let (pid, _, _) = m
            .add_git_project(PathBuf::from("/repo/audrey-app"), "main".into())
            .unwrap();
        m.add_slot(
            &pid,
            1,
            PathBuf::from("/repo/audrey-app/.domux/worktrees/workspace-1"),
        )
        .unwrap();
        m
    }

    /// A provider with any scope and no interval, for the tests about which targets exist.
    struct Watcher {
        name: String,
        scope: ProviderScope,
    }

    impl FactProvider for Watcher {
        fn name(&self) -> &str {
            &self.name
        }
        fn interval(&self) -> Duration {
            Duration::ZERO
        }
        fn scope(&self) -> ProviderScope {
            self.scope
        }
        fn fetch(&self, _t: &FactTarget) -> Result<Option<Fact>, String> {
            Ok(None)
        }
    }

    fn a_pull_request() -> Fact {
        Fact::new(
            "PR#212",
            Some(FactState::Open),
            "2026-09-05T10:00:00+01:00",
            Duration::from_secs(600),
        )
    }

    fn at(minute: u32) -> chrono::DateTime<chrono::Local> {
        format!("2026-09-05T10:{minute:02}:00+01:00")
            .parse::<chrono::DateTime<chrono::FixedOffset>>()
            .unwrap()
            .into()
    }

    #[test]
    fn due_returns_one_target_per_workspace_and_not_again_before_the_interval() {
        let mut r = FactRegistry::new();
        r.register(Arc::new(Counter {
            name: FACT_BRANCH.into(),
            interval: Duration::from_secs(5),
            calls: AtomicUsize::new(0),
            answer: None,
        }));
        let m = model_with_two_workspaces();
        let first = r.due(&m, at(0));
        assert_eq!(first.len(), 2, "main and workspace-1");
        assert!(first.iter().all(|(_, t)| t.key.name == FACT_BRANCH));
        assert!(
            r.due(&m, at(0)).is_empty(),
            "a target already in flight is not started twice"
        );
        for (_, t) in first {
            r.set(t.key, None);
        }
        assert!(
            r.due(&m, at(0)).is_empty(),
            "not due again inside the interval"
        );
        assert_eq!(r.due(&m, at(1)).len(), 2, "due again after it");
    }

    /// The interval and the in flight flag are two separate refusals, and the test above
    /// cannot tell them apart: at the same minute the interval refuses first. This provider
    /// has no interval, so the only thing that can refuse is the flag.
    #[test]
    fn a_target_already_in_flight_is_not_started_again_once_its_interval_has_passed() {
        let mut r = FactRegistry::new();
        r.register(Arc::new(Counter {
            name: FACT_BRANCH.into(),
            interval: Duration::ZERO,
            calls: AtomicUsize::new(0),
            answer: None,
        }));
        let m = model_with_two_workspaces();
        let started = r.due(&m, at(0));
        assert_eq!(started.len(), 2);
        assert!(
            r.due(&m, at(1)).is_empty(),
            "both are still running, and a slow gh must never queue behind itself"
        );
        r.set(started[0].1.key.clone(), None);
        assert_eq!(
            r.due(&m, at(1)).len(),
            1,
            "the one that answered is due again, and the one still running is not"
        );
    }

    #[test]
    fn a_provider_that_fails_leaves_the_fact_absent_and_never_a_guess() {
        let mut r = FactRegistry::new();
        let m = model_with_two_workspaces();
        let w = m.projects[0].workspaces[0].id.clone();
        let key = FactKey::workspace(&w, FACT_PR);
        r.set(
            key.clone(),
            Some(Fact::new(
                "PR#212",
                Some(FactState::Open),
                "2026-09-05T10:00:00+01:00",
                Duration::from_secs(600),
            )),
        );
        assert_eq!(r.get(&key).map(|f| f.text.as_str()), Some("PR#212"));
        r.set(key.clone(), None);
        assert_eq!(
            r.get(&key),
            None,
            "a failed fetch clears the fact rather than keeping a stale one"
        );
    }

    #[test]
    fn only_cached_providers_reach_the_cache_file_and_stale_entries_are_dropped_on_load() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("pr-cache.json");
        let m = model_with_two_workspaces();
        let w = m.projects[0].workspaces[0].id.clone();
        let mut r = FactRegistry::new();
        r.register(Arc::new(Counter {
            name: FACT_PR.into(),
            interval: Duration::from_secs(60),
            calls: AtomicUsize::new(0),
            answer: None,
        }));
        r.set(
            FactKey::workspace(&w, FACT_PR),
            Some(Fact::new(
                "PR#212",
                Some(FactState::Open),
                "2026-09-05T10:00:00+01:00",
                Duration::from_secs(600),
            )),
        );
        r.set(
            FactKey::workspace(&w, FACT_BRANCH),
            Some(Fact::new(
                "feat/x",
                None,
                "2026-09-05T10:00:00+01:00",
                Duration::from_secs(5),
            )),
        );
        r.save_cache(&path, &["pr"]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("PR#212") && !text.contains("feat/x"),
            "only the pull request is cached: {text}"
        );
        assert!(
            text.contains(r#""schema_version": 1"#),
            "the file says which shape it is in, so a later shape can be read or refused: {text}"
        );
        let mut fresh = FactRegistry::new();
        fresh.load_cache(&path, at(5));
        assert_eq!(
            fresh
                .get(&FactKey::workspace(&w, FACT_PR))
                .map(|f| f.text.as_str()),
            Some("PR#212"),
            "the switcher is not blank on start"
        );
        let mut later = FactRegistry::new();
        later.load_cache(&path, at(20));
        assert_eq!(
            later.get(&FactKey::workspace(&w, FACT_PR)),
            None,
            "past its time to live the cache is dropped, not shown"
        );
    }

    #[test]
    fn a_workspace_target_carries_the_paths_the_default_branch_and_the_branch_fact() {
        let mut r = FactRegistry::new();
        r.register(Arc::new(Watcher {
            name: FACT_PR.into(),
            scope: ProviderScope::Workspace,
        }));
        let m = model_with_two_workspaces();
        let slot = m.projects[0].workspaces[1].id.clone();
        let main = m.projects[0].workspaces[0].id.clone();
        r.set(
            FactKey::workspace(&slot, FACT_BRANCH),
            Some(Fact::new(
                "feat/x",
                None,
                "2026-09-05T10:00:00+01:00",
                Duration::from_secs(5),
            )),
        );
        let due = r.due(&m, at(0));
        let target = |w: &WorkspaceId| {
            due.iter()
                .find(|(_, t)| t.key == FactKey::workspace(w, FACT_PR))
                .expect("one target per workspace")
                .1
                .clone()
        };
        let t = target(&slot);
        assert_eq!(
            t.path,
            PathBuf::from("/repo/audrey-app/.domux/worktrees/workspace-1"),
            "the workspace's own path, which is where a provider runs"
        );
        assert_eq!(
            t.root,
            PathBuf::from("/repo/audrey-app"),
            "the project's main checkout"
        );
        assert_eq!(t.default_branch.as_deref(), Some("main"));
        assert_eq!(
            t.branch.as_deref(),
            Some("feat/x"),
            "the branch fact rides along, so the pull request provider does not run git again"
        );
        assert_eq!(
            target(&main).branch,
            None,
            "a branch that has not arrived is absent, not a guess at the handle"
        );
    }

    #[test]
    fn a_project_provider_gets_one_target_per_project_and_a_server_provider_gets_one() {
        let mut r = FactRegistry::new();
        r.register(Arc::new(Watcher {
            name: "ci".into(),
            scope: ProviderScope::Project,
        }));
        r.register(Arc::new(Watcher {
            name: "usage".into(),
            scope: ProviderScope::Server,
        }));
        let m = model_with_two_workspaces();
        let mut keys: Vec<String> = r
            .due(&m, at(0))
            .iter()
            .map(|(_, t)| t.key.to_string())
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                format!("{}/ci", m.projects[0].id),
                "server/usage".to_string()
            ],
            "one target for the project and one for the server, and none per workspace"
        );
    }

    #[test]
    fn a_plain_folder_project_gets_no_target_because_it_has_no_branch_and_no_pull_request() {
        let mut m = Model::new(7);
        m.add_folder_project(PathBuf::from("/notes")).unwrap();
        let mut r = FactRegistry::new();
        r.register(Arc::new(Counter {
            name: FACT_BRANCH.into(),
            interval: Duration::from_secs(5),
            calls: AtomicUsize::new(0),
            answer: None,
        }));
        r.register(Arc::new(Watcher {
            name: "ci".into(),
            scope: ProviderScope::Project,
        }));
        assert!(r.due(&m, at(0)).is_empty());
    }

    #[test]
    fn a_clock_that_went_backwards_makes_a_target_due_again() {
        let mut r = FactRegistry::new();
        r.register(Arc::new(Counter {
            name: FACT_BRANCH.into(),
            interval: Duration::from_secs(5),
            calls: AtomicUsize::new(0),
            answer: None,
        }));
        let m = model_with_two_workspaces();
        for (_, t) in r.due(&m, at(10)) {
            r.set(t.key, None);
        }
        assert!(r.due(&m, at(10)).is_empty(), "inside the interval");
        assert_eq!(
            r.due(&m, at(0)).len(),
            2,
            "a clock that went backwards leaves no age to compare, so look again now rather than holding every fact until the clock catches up"
        );
    }

    #[test]
    fn facts_about_a_deleted_workspace_or_project_are_forgotten_and_a_server_fact_stays() {
        let mut m = model_with_two_workspaces();
        let pid = m.projects[0].id.clone();
        let main = m.projects[0].workspaces[0].id.clone();
        let slot = m.projects[0].workspaces[1].id.clone();
        let mut r = FactRegistry::new();
        for key in [
            FactKey::workspace(&main, FACT_PR),
            FactKey::workspace(&slot, FACT_PR),
            FactKey::project(&pid, "ci"),
            FactKey::server("usage"),
        ] {
            r.set(key, Some(a_pull_request()));
        }
        r.forget_deleted(&m);
        assert_eq!(r.len(), 4, "nothing the model still holds is dropped");
        m.remove_workspace(&slot).unwrap();
        r.forget_deleted(&m);
        assert_eq!(
            r.get(&FactKey::workspace(&slot, FACT_PR)),
            None,
            "a deleted workspace's id can be redrawn once the server restarts, and the workspace that draws it must not inherit this pull request"
        );
        assert!(r.get(&FactKey::workspace(&main, FACT_PR)).is_some());
        m.remove_project(&pid).unwrap();
        r.forget_deleted(&m);
        assert_eq!(r.len(), 1);
        assert!(
            r.get(&FactKey::server("usage")).is_some(),
            "a server fact is about nothing the model holds, so nothing deletes it"
        );
    }

    #[test]
    fn a_forgotten_target_is_started_again_rather_than_held_to_what_it_was() {
        let mut r = FactRegistry::new();
        r.register(Arc::new(Counter {
            name: FACT_BRANCH.into(),
            interval: Duration::from_secs(5),
            calls: AtomicUsize::new(0),
            answer: None,
        }));
        let m = model_with_two_workspaces();
        assert_eq!(r.due(&m, at(0)).len(), 2, "both are now in flight");
        // The same two workspaces are gone from this model, so everything about them goes:
        // the fact, the time they were started and the in flight flag. `due` below is asked
        // about the model that still holds them, and starts both from nothing.
        r.forget_deleted(&Model::new(7));
        assert_eq!(
            r.due(&m, at(0)).len(),
            2,
            "a forgotten target keeps neither its in flight flag nor its start time"
        );
    }

    #[test]
    fn all_for_returns_one_workspaces_facts_by_name() {
        let m = model_with_two_workspaces();
        let main = m.projects[0].workspaces[0].id.clone();
        let slot = m.projects[0].workspaces[1].id.clone();
        let mut r = FactRegistry::new();
        for name in [
            "pr", "branch", "ci", "usage", "agent", "dirty", "ahead", "behind",
        ] {
            r.set(FactKey::workspace(&main, name), Some(a_pull_request()));
        }
        r.set(FactKey::workspace(&slot, FACT_PR), Some(a_pull_request()));
        let names: Vec<&str> = r.all_for(&main).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            vec!["agent", "ahead", "behind", "branch", "ci", "dirty", "pr", "usage"],
            "one workspace's facts, in one order whatever the hash map did"
        );
        assert_eq!(
            r.all_for(&slot).len(),
            1,
            "another workspace's facts are not this workspace's"
        );
    }

    /// A fact is an observation and an observation has an age. Nothing on the read path
    /// takes a clock, so the sweep is what keeps an old value off the screen.
    #[test]
    fn a_fact_past_its_time_to_live_is_dropped_and_named() {
        let m = model_with_two_workspaces();
        let w = m.projects[0].workspaces[0].id.clone();
        let key = FactKey::workspace(&w, FACT_PR);
        let mut r = FactRegistry::new();
        r.set(key.clone(), Some(a_pull_request()));
        assert!(r.expire(at(5)).is_empty(), "inside its ten minutes");
        assert!(r.get(&key).is_some());
        assert_eq!(
            r.expire(at(20)),
            vec![key.clone()],
            "the sweep says what it dropped, so the screen can be told"
        );
        assert_eq!(
            r.get(&key),
            None,
            "an hour old open pull request is not handed back as if it had just arrived"
        );
    }

    #[test]
    fn a_fact_whose_stamp_cannot_be_read_or_lies_ahead_of_now_is_not_fresh() {
        let m = model_with_two_workspaces();
        let w = m.projects[0].workspaces[0].id.clone();
        let mut r = FactRegistry::new();
        r.set(
            FactKey::workspace(&w, "unreadable"),
            Some(Fact::new("x", None, "not a time", Duration::from_secs(600))),
        );
        r.set(
            FactKey::workspace(&w, "ahead"),
            Some(Fact::new(
                "y",
                None,
                "2026-09-05T10:30:00+01:00",
                Duration::from_secs(600),
            )),
        );
        assert_eq!(
            r.expire(at(5)).len(),
            2,
            "an unknown age is not a fresh one, on the read path as on the load path"
        );
        assert!(r.is_empty());
    }

    #[test]
    fn a_cache_written_by_another_schema_version_is_not_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("pr-cache.json");
        std::fs::write(
            &path,
            r#"{"schema_version":2,"facts":{
                "w_0001/pr":{"text":"PR#1","fetched_at":"2026-09-05T10:00:00+01:00","ttl":600}
            }}"#,
        )
        .unwrap();
        let mut r = FactRegistry::new();
        r.load_cache(&path, at(5));
        assert!(
            r.is_empty(),
            "a file in a shape this server does not know is not guessed at"
        );
    }

    #[test]
    fn a_saved_fact_comes_back_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("pr-cache.json");
        let m = model_with_two_workspaces();
        let w = m.projects[0].workspaces[0].id.clone();
        let key = FactKey::workspace(&w, FACT_PR);
        let mut r = FactRegistry::new();
        r.set(
            key.clone(),
            Some(a_pull_request().with_url("https://forge.invalid/audrey-app/pull/212")),
        );
        r.save_cache(&path, &[FACT_PR]);
        let mut back = FactRegistry::new();
        back.load_cache(&path, at(5));
        assert!(!back.is_empty());
        let f = back.get(&key).expect("the pull request comes back");
        assert_eq!(f.text, "PR#212");
        assert_eq!(
            f.state,
            Some(FactState::Open),
            "the state comes back too, or the number would come back with no colour"
        );
        assert_eq!(
            f.url.as_deref(),
            Some("https://forge.invalid/audrey-app/pull/212")
        );
    }

    #[test]
    fn a_cache_entry_is_dropped_when_its_stamp_is_unreadable_or_lies_ahead_of_now() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("pr-cache.json");
        std::fs::write(
            &path,
            r#"{"schema_version":1,"facts":{
                "w_0001/pr":{"text":"PR#1","fetched_at":"2026-09-05T10:00:00+01:00","ttl":600},
                "w_0002/pr":{"text":"PR#2","fetched_at":"not a time","ttl":600},
                "w_0003/pr":{"text":"PR#3","fetched_at":"2026-09-05T10:30:00+01:00","ttl":600},
                "x_0004/pr":{"text":"PR#4","fetched_at":"2026-09-05T10:00:00+01:00","ttl":600},
                "w_0005/pr":5
            }}"#,
        )
        .unwrap();
        let mut r = FactRegistry::new();
        r.load_cache(&path, at(5));
        assert_eq!(
            r.get(&"w_0001/pr".parse().unwrap())
                .map(|f| f.text.as_str()),
            Some("PR#1")
        );
        assert_eq!(
            r.len(),
            1,
            "an unreadable stamp, a stamp from the future, an unreadable key and an entry that is not a fact are all dropped rather than shown"
        );
    }

    /// The server joins the name onto its own state directory, and every other reader gets
    /// it from `paths`. Two spellings of one file name is a cache nothing reads.
    #[test]
    fn the_cache_is_the_file_the_paths_module_names() {
        let env = domux_core::paths::Env {
            home: PathBuf::from("/home/u"),
            xdg_runtime_dir: None,
            uid: 501,
            socket_override: None,
            state_dir_override: Some(PathBuf::from("/state")),
            config_file_override: None,
        };
        assert_eq!(
            pr_cache_path(Path::new("/state")),
            domux_core::paths::pr_cache_file_in(&env)
        );
    }

    #[test]
    fn a_cache_that_is_missing_or_unreadable_leaves_the_registry_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("pr-cache.json");
        let mut r = FactRegistry::new();
        r.load_cache(&path, at(5));
        assert!(r.is_empty(), "there is no cache file yet");
        std::fs::write(&path, "half a file").unwrap();
        r.load_cache(&path, at(5));
        assert!(r.is_empty(), "a cache that is not json is not a fact");
        std::fs::write(&path, r#"{"schema_version":1}"#).unwrap();
        r.load_cache(&path, at(5));
        assert!(r.is_empty(), "a cache with no facts in it holds no facts");
    }
}
