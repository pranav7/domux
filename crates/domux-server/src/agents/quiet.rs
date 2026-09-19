//! Quiet: a record that says it is in the middle of something, that nothing is reporting on.
//!
//! `working` and `compacting` are the two states only a hook can leave, and an agent has more
//! ways to stop than it has hooks to say so. A turn cancelled before its request goes out
//! sends nothing, and a hook that never reaches the socket is the same thing from here. Such a
//! row kept the working word, the turning star and the band until the agent's process died
//! (decision record 0058).
//!
//! The evidence is the transcript's modification time. Every hook a session sends comes with
//! entries in that file, and an agent working writes to it continuously, so a transcript
//! standing still is the session standing still. It costs one `stat` per busy record per tick,
//! beside the read `agents::recap` already does.
//!
//! Nothing here guesses what the agent is doing. `unknown` says an agent is running and
//! nothing is reporting, which is exactly what has been established, and the next hook takes
//! the row back.

use domux_core::api::Event;
use domux_core::ids::AgentId;
use domux_core::model::agent::AgentState;
use domux_core::model::Model;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// How long a busy record's transcript may stand still before nothing is reporting on it.
///
/// Measured rather than chosen. Across four of the author's own transcripts, 33,500 entries and
/// 890 silences inside a turn, the longest a transcript went untouched while its agent was
/// genuinely working was 191 seconds, and the rest of the distribution ends between two and
/// three minutes. Ten minutes is about three times the longest real one, so a row that reaches
/// it has stopped. Read decision record 0058 before shortening it: the cost of being wrong is a
/// row going quiet while its agent works, which is the thing this was built to stop.
pub const AFTER: Duration = Duration::from_secs(10 * 60);

/// Every busy record whose transcript has stood still, asked once. The core calls this from its
/// once-a-second tick.
pub fn poll(model: &mut Model, now: SystemTime) -> Vec<Event> {
    let quiet: Vec<AgentId> = model
        .agents
        .iter()
        .filter(|a| matches!(a.state, AgentState::Working | AgentState::Compacting))
        .filter(|a| {
            a.transcript_path
                .as_deref()
                .is_some_and(|p| stood_still(p, now))
        })
        .map(|a| a.id.clone())
        .collect();
    quiet.iter().flat_map(|id| model.agent_quiet(id)).collect()
}

/// Whether the file has not been written for `AFTER`.
///
/// False whenever there is no answer to read. A transcript domux cannot stat, and one whose
/// time is ahead of this machine's, are no evidence that a session stopped, and a record is
/// only ever moved on evidence. A record with no transcript at all never reaches here.
fn stood_still(path: &Path, now: SystemTime) -> bool {
    let Ok(written) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return false;
    };
    now.duration_since(written).is_ok_and(|d| d >= AFTER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_core::ids::{PaneId, WorkspaceId};
    use domux_core::model::agent::{Agent, AgentKind, AgentSource};

    /// A model holding one record in `state`, whose transcript was last written `ago` before
    /// the `now` the poll is given.
    fn model_with(state: AgentState, ago: Duration) -> (Model, SystemTime, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        let now = SystemTime::now();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(now - ago)
            .unwrap();
        let mut model = Model::new(1);
        let mut a = Agent::new(
            AgentId("a_0001".into()),
            AgentKind::Claude,
            WorkspaceId("w_0001".into()),
            PaneId("p_0001".into()),
            std::path::PathBuf::from("/x"),
            AgentSource::Hook,
            "2026-09-18T13:00:00+01:00",
        );
        a.state = state;
        a.transcript_path = Some(path);
        model.agents.push(a);
        (model, now, dir)
    }

    fn state_after(state: AgentState, ago: Duration) -> AgentState {
        let (mut model, now, _dir) = model_with(state, ago);
        poll(&mut model, now);
        model.agents[0].state
    }

    #[test]
    fn a_busy_record_whose_transcript_stood_still_goes_unknown() {
        for state in [AgentState::Working, AgentState::Compacting] {
            assert_eq!(
                state_after(state, AFTER + Duration::from_secs(1)),
                AgentState::Unknown,
                "{state:?}"
            );
        }
    }

    #[test]
    fn a_busy_record_whose_transcript_is_still_being_written_stands() {
        for state in [AgentState::Working, AgentState::Compacting] {
            assert_eq!(
                state_after(state, Duration::from_secs(30)),
                state,
                "{state:?}"
            );
            assert_eq!(
                state_after(state, AFTER - Duration::from_secs(1)),
                state,
                "just inside the window: {state:?}"
            );
        }
    }

    /// A waiting row is blocked on you and says so until it is answered, however long that is,
    /// and idle and unknown are already resting. None of them is a state a hook has to leave.
    #[test]
    fn a_record_at_rest_stands_however_long_its_transcript_stands_still() {
        for state in [AgentState::Waiting, AgentState::Idle, AgentState::Unknown] {
            assert_eq!(state_after(state, AFTER * 6), state, "{state:?}");
        }
    }

    #[test]
    fn the_poll_answers_with_the_state_change_it_made() {
        let (mut model, now, _dir) = model_with(AgentState::Working, AFTER * 2);
        let events = poll(&mut model, now);
        assert_eq!(
            events,
            vec![Event::AgentStateChanged {
                agent: AgentId("a_0001".into()),
                from: AgentState::Working,
                to: AgentState::Unknown,
            }]
        );
        assert!(
            poll(&mut model, now).is_empty(),
            "a second pass has nothing left to move"
        );
    }

    #[test]
    fn a_record_with_no_transcript_to_read_stands() {
        let (mut model, now, dir) = model_with(AgentState::Working, AFTER * 2);
        model.agents[0].transcript_path = None;
        assert!(poll(&mut model, now).is_empty(), "no transcript on record");
        model.agents[0].transcript_path = Some(dir.path().join("gone.jsonl"));
        assert!(poll(&mut model, now).is_empty(), "no file to stat");
        assert_eq!(model.agents[0].state, AgentState::Working);
    }

    /// A transcript written by a machine whose clock is ahead of this one is no evidence that
    /// the session stopped.
    #[test]
    fn a_transcript_written_ahead_of_this_machine_stands() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        let now = SystemTime::now();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(now + AFTER)
            .unwrap();
        let (mut model, _, _keep) = model_with(AgentState::Working, Duration::ZERO);
        model.agents[0].transcript_path = Some(path);
        assert!(poll(&mut model, now).is_empty());
        assert_eq!(model.agents[0].state, AgentState::Working);
    }
}
