//! `state.json`: the persisted structure, its schema version, and the migration ladder.
//! Writing the file is the server's job (`persist.rs`); this module only shapes the bytes.

use crate::ids::WorkspaceId;
use crate::model::{Model, Project};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 1 at the end of M1. M2 makes it 2, M3 3, M4 4, each with a migration and a fixture.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateFile {
    pub schema_version: u32,
    /// RFC 3339, from the server's clock. Informational.
    pub saved_at: String,
    pub projects: Vec<Project>,
    pub last_workspace: Option<WorkspaceId>,
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("state.json is schema version {found} but this domux reads up to {supported}; upgrade domux or move the file aside")]
    Newer { found: u32, supported: u32 },
    #[error("state.json could not be read: {0}; the previous file is state.json.bak")]
    Corrupt(String),
}

/// One step of the ladder: the version it reads, and the edit that turns that JSON into the
/// next version's JSON.
type Migration = (u32, fn(&mut Value) -> Result<(), String>);

/// Migrations from version N to N+1, in order. Each edits the JSON in place. M1 has none.
pub const MIGRATIONS: &[Migration] = &[];

pub fn snapshot(model: &Model, saved_at: &str) -> StateFile {
    StateFile {
        schema_version: SCHEMA_VERSION,
        saved_at: saved_at.to_string(),
        projects: model.projects.clone(),
        last_workspace: model.last_workspace.clone(),
    }
}

/// A model with the saved structure, no clients, no facts, and a default id seed (the
/// server calls `reseed`).
pub fn restore(file: StateFile) -> Result<Model, StateError> {
    let mut model = Model::new(0x5eed);
    model.projects = file.projects;
    model.last_workspace = file.last_workspace;
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
    use crate::model::Direction;
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
        assert_eq!(file.schema_version, 1);
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

    /// A stand-in rung: `MIGRATIONS` is empty in M1, so the walk itself would otherwise
    /// ship untested and M2 would be the first thing to run it.
    fn add_a_field(v: &mut Value) -> Result<(), String> {
        v["added"] = Value::from("yes");
        Ok(())
    }

    fn refuses(_: &mut Value) -> Result<(), String> {
        Err("the 0 to 1 migration needs a field this file does not have".into())
    }

    #[test]
    fn the_ladder_runs_the_rung_for_the_version_found_and_stamps_the_new_version() {
        let ladder: &[Migration] = &[(0, add_a_field)];
        let mut value = json!({ "schema_version": 0 });
        climb(&mut value, 0, ladder).unwrap();
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
}
