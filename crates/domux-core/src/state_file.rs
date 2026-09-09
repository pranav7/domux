//! `state.json`: the persisted structure, its schema version, and the migration ladder.
//! Writing the file is the server's job (`persist.rs`); this module only shapes the bytes.

use crate::ids::WorkspaceId;
use crate::model::{Agent, Model, Project};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 1 at the end of M1, 2 at M2, 3 at M3 (agent records), 4 at M4.
pub const SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateFile {
    pub schema_version: u32,
    /// RFC 3339, from the server's clock. Informational.
    pub saved_at: String,
    pub projects: Vec<Project>,
    pub last_workspace: Option<WorkspaceId>,
    /// The state a new client's sidebar starts in (roadmap decision 4). Added in schema
    /// version 2.
    #[serde(default)]
    pub sidebar_open: bool,
    /// M3. Live records are restored as exited (architecture spec section 5).
    #[serde(default)]
    pub agents: Vec<Agent>,
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("state.json is schema version {found} but this domux reads up to {supported}; upgrade domux or move the file aside")]
    Newer { found: u32, supported: u32 },
    /// No file is named here. The `.bak` this used to point at is rotated on every write,
    /// so it holds the refused file for one structure change and then does not; the server
    /// moves the refused file somewhere nothing rotates and logs that path instead.
    #[error("state.json could not be read: {0}")]
    Corrupt(String),
}

/// One step of the ladder: the version it reads, and the edit that turns that JSON into the
/// next version's JSON.
type Migration = (u32, fn(&mut Value) -> Result<(), String>);

/// Version 1 knew nothing about the sidebar, so a file written by M1 starts with it
/// hidden: the author's screen must not change shape on the first start after an upgrade.
pub fn v1_to_v2(value: &mut Value) -> Result<(), String> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| "state.json is not an object".to_string())?;
    object.entry("sidebar_open").or_insert(Value::Bool(false));
    Ok(())
}

/// 2 to 3: the `agents` list appears, empty.
fn v2_to_v3(v: &mut Value) -> Result<(), String> {
    let obj = v
        .as_object_mut()
        .ok_or_else(|| "state.json is not an object".to_string())?;
    obj.entry("agents")
        .or_insert_with(|| Value::Array(Vec::new()));
    Ok(())
}

/// Migrations from version N to N+1, in order. M1 had none; M2 adds the sidebar; M3 adds
/// agents.
pub const MIGRATIONS: &[Migration] = &[(1, v1_to_v2), (2, v2_to_v3)];

pub fn snapshot(model: &Model, saved_at: &str) -> StateFile {
    StateFile {
        schema_version: SCHEMA_VERSION,
        saved_at: saved_at.to_string(),
        projects: model.projects.clone(),
        last_workspace: model.last_workspace.clone(),
        sidebar_open: model.sidebar_open,
        agents: model.agents.clone(),
    }
}

/// A model with the saved structure, no clients, no facts, and a default id seed (the
/// server calls `reseed`).
pub fn restore(file: StateFile) -> Result<Model, StateError> {
    let mut model = Model::new(0x5eed);
    model.projects = file.projects;
    model.last_workspace = file.last_workspace;
    model.sidebar_open = file.sidebar_open;
    for p in &model.projects {
        for w in &p.workspaces {
            for t in &w.tabs {
                if !t.layout.contains(&t.focused) {
                    return Err(StateError::Corrupt(format!(
                        "tab {} focuses pane {} which is not in its layout",
                        t.id, t.focused
                    )));
                }
            }
        }
    }
    // A record whose workspace no longer exists (the workspace was cleared or deleted)
    // is dropped rather than restored dangling. Core has no logging, so this is silent;
    // the pruned-workspace footer note belongs to the server, as M2 left it.
    model.agents = file
        .agents
        .into_iter()
        .filter(|a| model.workspace(&a.workspace).is_some())
        .collect();
    model.mark_agents_exited_on_restore();
    Ok(model)
}

/// Parses and migrates. A file from a newer domux is refused, not guessed at.
pub fn parse(text: &str) -> Result<StateFile, StateError> {
    let mut value: Value =
        serde_json::from_str(text).map_err(|e| StateError::Corrupt(e.to_string()))?;
    let found = value
        .get("schema_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| StateError::Corrupt("missing schema_version".into()))?
        as u32;
    if found > SCHEMA_VERSION {
        return Err(StateError::Newer {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    climb(&mut value, found, MIGRATIONS)?;
    serde_json::from_value(value).map_err(|e| StateError::Corrupt(e.to_string()))
}

/// Runs the ladder from `found` up to `SCHEMA_VERSION`, stamping each new version into the
/// JSON as it goes. A rung that is missing is a refusal, not a silent pass: a file written
/// by a version whose shape this build never knew would otherwise be read with today's
/// field names and quietly lose whatever it actually held.
///
/// Takes the ladder as an argument so the walk itself is testable while `MIGRATIONS` is
/// still empty. Only `parse` calls it, and only with `MIGRATIONS`.
fn climb(value: &mut Value, found: u32, ladder: &[Migration]) -> Result<(), StateError> {
    let mut version = found;
    while version < SCHEMA_VERSION {
        let (_, migrate) = ladder
            .iter()
            .find(|(from, _)| *from == version)
            .ok_or_else(|| {
                StateError::Corrupt(format!("no migration from schema version {version}"))
            })?;
        migrate(value).map_err(StateError::Corrupt)?;
        version += 1;
        value["schema_version"] = Value::from(version);
    }
    Ok(())
}

pub fn to_json(file: &StateFile) -> String {
    serde_json::to_string_pretty(file).expect("state serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Direction, ProjectKind, WorkspaceHandle};
    use serde_json::json;
    use std::path::PathBuf;

    fn fixture() -> String {
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/state/v1.json"
        ))
        .unwrap()
    }

    #[test]
    fn v1_fixture_restores_tabs_panes_names_and_focus() {
        let file = parse(&fixture()).unwrap();
        assert_eq!(
            file.schema_version, SCHEMA_VERSION,
            "parse migrates in place"
        );
        let model = restore(file).unwrap();
        let ws = model
            .workspace(&crate::ids::WorkspaceId("w_c3a1".into()))
            .unwrap();
        assert_eq!(ws.tabs.len(), 1);
        let tab = &ws.tabs[0];
        assert_eq!(tab.name.as_deref(), Some("tests"));
        assert_eq!(
            tab.layout.pane_ids(),
            vec![
                crate::ids::PaneId("p_8f2a".into()),
                crate::ids::PaneId("p_0b11".into())
            ]
        );
        assert_eq!(tab.focused, crate::ids::PaneId("p_0b11".into()));
        assert_eq!(
            model
                .pane(&crate::ids::PaneId("p_0b11".into()))
                .unwrap()
                .cwd,
            PathBuf::from("/Users/pranav/projects/domux/crates")
        );
        assert_eq!(
            model
                .pane(&crate::ids::PaneId("p_0b11".into()))
                .unwrap()
                .pid,
            None,
            "facts are not persisted"
        );
        assert!(model.clients.is_empty());
    }

    #[test]
    fn snapshot_then_restore_is_the_identity_on_structure() {
        let mut m = Model::new(3);
        let (_, ws, _) = m.add_folder_project(PathBuf::from("/x")).unwrap();
        let (_, p, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        m.split_pane(&p, Direction::Down, PathBuf::from("/x/y"))
            .unwrap();
        m.rename_tab(
            &m.workspace(&ws).unwrap().tabs[0].id.clone(),
            Some("a".into()),
        )
        .unwrap();
        let file = snapshot(&m, "2026-09-05T10:00:00Z");
        let json = to_json(&file);
        let back = restore(parse(&json).unwrap()).unwrap();
        assert_eq!(back.projects, m.projects);
        assert_eq!(back.last_workspace, m.last_workspace);
    }

    #[test]
    fn a_newer_schema_is_refused_with_both_versions() {
        let text = fixture().replace("\"schema_version\": 1", "\"schema_version\": 99");
        match parse(&text) {
            Err(StateError::Newer { found, supported }) => {
                assert_eq!((found, supported), (99, SCHEMA_VERSION));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn corrupt_json_is_reported_not_panicked() {
        assert!(matches!(parse("{not json"), Err(StateError::Corrupt(_))));
        assert!(matches!(
            parse("{\"schema_version\": 1}"),
            Err(StateError::Corrupt(_))
        ));
    }

    /// A stand-in rung, kept independent of `MIGRATIONS` so the walk itself stays tested
    /// even on a version whose real ladder happens to refuse or short-circuit. Anchored to
    /// `SCHEMA_VERSION - 1` rather than a literal so it keeps working as later milestones
    /// bump the constant.
    fn add_a_field(v: &mut Value) -> Result<(), String> {
        v["added"] = Value::from("yes");
        Ok(())
    }

    fn refuses(_: &mut Value) -> Result<(), String> {
        Err("the 0 to 1 migration needs a field this file does not have".into())
    }

    #[test]
    fn the_ladder_runs_the_rung_for_the_version_found_and_stamps_the_new_version() {
        let found = SCHEMA_VERSION - 1;
        let ladder: &[Migration] = &[(found, add_a_field)];
        let mut value = json!({ "schema_version": found });
        climb(&mut value, found, ladder).unwrap();
        assert_eq!(value["added"], Value::from("yes"));
        assert_eq!(value["schema_version"], Value::from(SCHEMA_VERSION));
    }

    #[test]
    fn a_migration_that_fails_reports_what_it_could_not_do() {
        let ladder: &[Migration] = &[(0, refuses)];
        let mut value = json!({ "schema_version": 0 });
        match climb(&mut value, 0, ladder) {
            Err(StateError::Corrupt(message)) => {
                assert!(
                    message.contains("needs a field this file does not have"),
                    "{message}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_older_schema_with_no_migration_is_refused_not_read() {
        let text = fixture().replace("\"schema_version\": 1", "\"schema_version\": 0");
        match parse(&text) {
            Err(StateError::Corrupt(message)) => {
                assert!(
                    message.contains("no migration from schema version 0"),
                    "{message}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_tab_focusing_a_pane_outside_its_layout_is_refused() {
        let text = fixture().replace("\"focused\": \"p_0b11\"", "\"focused\": \"p_dead\"");
        match restore(parse(&text).unwrap()) {
            Err(StateError::Corrupt(message)) => {
                assert!(
                    message.contains("t_41b2") && message.contains("p_dead"),
                    "{message}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    /// The round trip on the whole model, not just its two persisted fields. `Model`'s
    /// `PartialEq` compares `clients` too, so this holds exactly while none is attached,
    /// which is the state a restore produces.
    #[test]
    fn a_restored_model_equals_the_saved_one_when_no_client_is_attached() {
        let mut m = Model::new(7);
        let (_, ws, _) = m.add_folder_project(PathBuf::from("/x")).unwrap();
        let (_, p, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        m.split_pane(&p, Direction::Right, PathBuf::from("/x/y"))
            .unwrap();
        assert!(m.clients.is_empty());
        let back =
            restore(parse(&to_json(&snapshot(&m, "2026-09-05T10:00:00Z"))).unwrap()).unwrap();
        assert_eq!(back, m);
    }

    /// `StateFile::sidebar_open` carries `#[serde(default)]`, whose fallback is also
    /// `false`. That makes a migrated file that starts hidden indistinguishable, from the
    /// outside, from a migration that quietly does nothing at all: both leave the key
    /// absent and both restore to `false`. This test pins `v1_to_v2` directly, so a
    /// no-op migration function - one that returns `Ok(())` without touching the value -
    /// fails here even though every test that goes through `parse` and `restore` would
    /// still pass.
    #[test]
    fn v1_to_v2_actually_writes_sidebar_open_false_when_absent() {
        let mut value = json!({
            "schema_version": 1,
            "saved_at": "x",
            "projects": [],
            "last_workspace": null
        });
        v1_to_v2(&mut value).unwrap();
        assert_eq!(value["sidebar_open"], Value::from(false));
    }

    #[test]
    fn a_schema_version_1_file_migrates_and_starts_with_the_sidebar_hidden() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/state/v1.json"
        ))
        .unwrap();
        // Deliberately the literal 1, not a variable: this test exists to prove a
        // version-1 file migrates, so it must first prove the fixture actually is one.
        // Without this, editing the fixture's schema_version to 2 would still leave every
        // assertion below green while `v1_to_v2` never ran.
        let raw: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            raw["schema_version"], 1,
            "the fixture must be a version-1 file"
        );
        let file = parse(&text).unwrap();
        assert_eq!(file.schema_version, SCHEMA_VERSION);
        assert!(
            !file.sidebar_open,
            "a file written before the sidebar existed starts hidden"
        );
        let model = restore(file).unwrap();
        assert!(!model.projects.is_empty(), "the M1 fixture still restores");
        assert!(!model.sidebar_open);
    }

    #[test]
    fn a_schema_version_2_file_round_trips_with_git_projects_slots_and_names() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/state/v2.json"
        ))
        .unwrap();
        let model = restore(parse(&text).unwrap()).unwrap();
        assert!(model.sidebar_open, "the fixture remembers an open sidebar");
        let p = &model.projects[0];
        assert_eq!(p.name, "audrey-app");
        assert_eq!(
            p.kind,
            ProjectKind::Git {
                default_branch: "main".into()
            }
        );
        assert_eq!(p.workspaces.len(), 2);
        assert_eq!(p.workspaces[1].handle, WorkspaceHandle::Slot(1));
        assert_eq!(p.workspaces[1].name.as_deref(), Some("auth cleanup"));
        let again = to_json(&snapshot(&model, "2026-09-05T10:00:00+01:00"));
        assert_eq!(restore(parse(&again).unwrap()).unwrap(), model);
    }

    #[test]
    fn a_file_from_a_newer_domux_is_refused_by_name_and_not_guessed_at() {
        let err =
            parse(r#"{"schema_version":9,"saved_at":"x","projects":[],"last_workspace":null}"#)
                .unwrap_err();
        assert!(
            matches!(
                err,
                StateError::Newer {
                    found: 9,
                    supported: SCHEMA_VERSION
                }
            ),
            "{err}"
        );
    }

    fn fixture_v3() -> String {
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/state/v3.json"
        ))
        .unwrap()
    }

    #[test]
    fn v3_fixture_restores_agents_with_live_ones_exited_and_places_kept() {
        let file = parse(&fixture_v3()).unwrap();
        assert_eq!(file.schema_version, SCHEMA_VERSION);
        assert_eq!(file.agents.len(), 2);
        let model = restore(file).unwrap();
        let a = model.agent(&crate::ids::AgentId("a_5e21".into())).unwrap();
        assert_eq!(
            a.state,
            crate::model::AgentState::Exited,
            "it was working when the server stopped"
        );
        assert_eq!(a.pane, None);
        assert_eq!(a.last_pane, Some(crate::ids::PaneId("p_8f2a".into())));
        assert_eq!(a.name.as_deref(), Some("auth-cleanup"));
        assert_eq!(
            a.recap.as_deref(),
            Some("Replaced three session checks with one guard in auth/middleware.go")
        );
        assert!(!a.unseen, "restore does not paint rows red");
        assert_eq!(a.source, crate::model::AgentSource::Restore);
        let b = model.agent(&crate::ids::AgentId("a_0b77".into())).unwrap();
        assert!(
            b.unseen,
            "an exit the agent made before the stop stays unseen"
        );
        assert_eq!(b.source, crate::model::AgentSource::Hook);
    }

    #[test]
    fn v2_and_v1_fixtures_migrate_to_v3_with_no_agents() {
        let v2 = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/state/v2.json"
        ))
        .unwrap();
        let file = parse(&v2).unwrap();
        assert_eq!(file.schema_version, SCHEMA_VERSION);
        assert!(file.agents.is_empty());
        let v1 = fixture();
        let file = parse(&v1).unwrap();
        assert_eq!(
            file.schema_version, SCHEMA_VERSION,
            "the ladder runs 1 to 2 to 3"
        );
        assert!(file.agents.is_empty());
        restore(file).unwrap();
    }

    #[test]
    fn snapshot_carries_agents_and_restore_gives_them_back() {
        let mut m = Model::new(3);
        let (_, ws, _) = m.add_folder_project(PathBuf::from("/x")).unwrap();
        let (_, p, _) = m.create_tab(&ws, PathBuf::from("/x")).unwrap();
        let report = crate::model::AgentReport {
            event: Some(crate::model::AgentEvent::SessionStart),
            session_id: Some("s".into()),
            transcript_path: None,
            cwd: None,
            reason: None,
        };
        let id = m
            .report_agent(
                &p,
                crate::model::AgentKind::Claude,
                report,
                "2026-09-05T10:00:00Z",
            )
            .unwrap()
            .agent;
        let file = snapshot(&m, "2026-09-05T10:00:00Z");
        assert_eq!(file.schema_version, SCHEMA_VERSION);
        assert_eq!(file.agents.len(), 1);
        let json = to_json(&file);
        assert!(json.contains("\"agents\""));
        assert!(
            !json.contains("\"pid\""),
            "pid is a fact and is not persisted"
        );
        let back = restore(parse(&json).unwrap()).unwrap();
        assert_eq!(back.agent(&id).unwrap().session_id.as_deref(), Some("s"));
        assert_eq!(
            back.agent(&id).unwrap().state,
            crate::model::AgentState::Exited
        );
    }

    #[test]
    fn a_restored_agent_whose_workspace_is_gone_is_dropped_and_the_rest_kept() {
        let mut value: Value = serde_json::from_str(&fixture_v3()).unwrap();
        value["agents"][0]["workspace"] = Value::from("w_dead");
        let file: StateFile = serde_json::from_value(value).unwrap();
        assert_eq!(file.agents.len(), 2, "parse itself does not prune");
        let model = restore(file).unwrap();
        assert_eq!(
            model.agents.len(),
            1,
            "the record with no workspace is dropped"
        );
        assert!(model.agent(&crate::ids::AgentId("a_5e21".into())).is_none());
        assert!(model.agent(&crate::ids::AgentId("a_0b77".into())).is_some());
    }
}
