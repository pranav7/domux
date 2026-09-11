//! `agent ...`, `peek`, `whoami` and the three messaging verbs M4 fills in.
//!
//! Each one is a single API call to the handler a keybinding reaches, so a key, a subcommand
//! and an API call are one implementation (architecture spec section 8).

use super::{call, call_as, location, print_line};
use clap::{Args, Subcommand};
use domux_core::api::{AgentInfo, AgentListResult, AgentReportResult};
use domux_core::model::agent::AgentKind;
use domux_server::render::agents_box::{empty_text, RowForm, RECAP_GLYPH};
use serde_json::{json, Value};
use std::io::{Read, Write};

#[derive(Args)]
pub struct AgentCmd {
    #[command(subcommand)]
    pub action: AgentAction,
}

#[derive(Subcommand)]
pub enum AgentAction {
    /// List the agents: kind, place, state, recap
    List {
        /// Print the API result rather than the rows
        #[arg(long)]
        json: bool,
    },
    /// Post one hook payload, read from standard input. Hooks run this
    Report {
        /// Which adapter reads the payload: claude, codex or opencode
        #[arg(long = "agent", value_parser = parse_kind)]
        kind: AgentKind,
    },
    /// Switch to an agent's workspace, tab and pane
    Focus {
        /// An agent id, a workspace with one agent, or workspace/tab; the default is the
        /// agent in this pane
        agent: Option<String>,
    },
}

/// An agent kind through its own parser, so a fourth word is refused in the words the rest of
/// domux refuses it in: `unknown agent kind "gemini"; expected claude, codex or opencode`.
///
/// `AgentKind` carries no `clap::ValueEnum` derive, because `domux-core` is the pure model and
/// does not depend on clap. The reader sees the same three kinds either way.
pub fn parse_kind(text: &str) -> Result<AgentKind, String> {
    text.parse()
}

pub async fn run(cmd: AgentCmd) -> anyhow::Result<()> {
    match cmd.action {
        // `peek` is the same call under the name the SessionStart block gives an agent, so the
        // two spellings are one rendering.
        AgentAction::List { json } => peek(json).await,
        AgentAction::Report { kind } => report(kind).await,
        // Quiet on success: what a focus did is on the screen.
        AgentAction::Focus { agent } => {
            call("agent.focus", json!({ "agent": agent })).await?;
            Ok(())
        }
    }
}

/// `agent report --agent <kind>`: the payload on standard input, the pane from the environment.
///
/// Once the arguments have parsed, it never fails and never writes to standard error. Outside a
/// domux pane there is nothing to report, and a server that is not listening has nothing to
/// report to; the same hook lines run under V1's tmux panes until the cut-over, and a hook that
/// fails is an interruption in the agent's session (M3 plan assumption 17). Every other failure
/// is silent for the same reason: a hook is not a place a person reads an error.
pub async fn report(kind: AgentKind) -> anyhow::Result<()> {
    let Some(pane) = location::pane_from_env() else {
        return Ok(());
    };
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return Ok(());
    }
    // The object the agent wrote, when it wrote one. A payload that is not JSON travels as the
    // string it is, and the server's adapter refuses it there rather than here.
    let payload = serde_json::from_str::<Value>(&payload).unwrap_or(Value::String(payload));
    let Ok(answer) = call(
        "agent.report",
        json!({ "pane": pane, "kind": kind, "payload": payload }),
    )
    .await
    else {
        return Ok(());
    };
    let Ok(result) = serde_json::from_value::<AgentReportResult>(answer) else {
        return Ok(());
    };
    if let Some(block) = result.context {
        let output = session_start_output(kind, block);
        // Written whole rather than line by line. A reader that went away ends the output
        // rather than failing, which is the rule every line of data here follows.
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(output.as_bytes());
        let _ = out.flush();
    }
    Ok(())
}

/// The hook client's SessionStart wire format. Claude reads plain text. Codex reads a JSON
/// object; the context itself starts with `[domux]`, which Codex otherwise mistakes for JSON.
fn session_start_output(kind: AgentKind, block: String) -> String {
    if kind == AgentKind::Codex {
        return format!(
            "{}\n",
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "SessionStart",
                    "additionalContext": block,
                }
            })
        );
    }
    block
}

/// `peek`: every agent as rows, or the API result under `--json` (M3 plan assumption 38).
pub async fn peek(as_json: bool) -> anyhow::Result<()> {
    if as_json {
        // The answer as the server wrote it, so a caller reads the same fields the API
        // documents rather than a rendering of them (principle 12).
        let value = call("agent.list", json!({})).await?;
        return print_line(&serde_json::to_string_pretty(&value)?);
    }
    let result: AgentListResult = call_as("agent.list", json!({})).await?;
    if result.agents.is_empty() {
        // The sentence both Agents boxes draw, from the one function that writes it: the state
        // and the next action (principle 9), read one way wherever the reader meets it. The
        // overlay's wording, because `peek` lists the exited records too.
        eprintln!("{}", empty_text("", RowForm::Overlay));
        return Ok(());
    }
    for a in &result.agents {
        print_line(&lines_for(a))?;
    }
    Ok(())
}

/// What leads every block of the listing. It is a bullet, not the box's waiting dot: the box
/// draws its dot only while an agent is waiting (decision record 0030), and this marks a row
/// whatever its state, so the two are spelled apart and a change to one does not reach the other.
const BULLET: &str = "•";

/// One agent as the row grammar in text: the bullet line, the place line carrying the id, and
/// the recap line when there is one (M3 plan assumption 38).
///
/// The recap glyph is the box's own constant, so a change to it reaches both surfaces. The
/// place line always names the kind, which the box drops when a session name has taken the first
/// line: the command line has no colour to carry the kind, and a reader about to name this record
/// needs the id from the same block.
pub fn lines_for(a: &AgentInfo) -> String {
    let name = a.name.clone().unwrap_or_else(|| a.kind.to_string());
    let unseen = if a.unseen { "  unseen" } else { "" };
    let mut out = format!("{BULLET} {name}  {}{unseen}", a.state);
    out.push_str(&format!("\n  {} · {}  ({})", a.kind, a.place, a.id));
    if let Some(recap) = &a.recap {
        out.push_str(&format!("\n  {RECAP_GLYPH} {recap}"));
    }
    out
}

/// `whoami`: the agent in this pane.
pub async fn whoami() -> anyhow::Result<()> {
    let pane = location::pane_from_env();
    let a: AgentInfo = call_as("agent.self", json!({ "pane": pane })).await?;
    print_line(&lines_for(&a))
}

// `send`, `read` and `wait`: the three messaging verbs the `SessionStart` block names. Their
// handlers arrive in M4 and every call answers "is not built yet" until then. The subcommands
// are here now because that block promises them, and a command a message promises has to reach
// an answer that says when it works rather than clap's "unrecognized subcommand" (principle 9).
//
// Every argument is optional, so a bare verb reaches that answer too. An empty message travels
// as one: the rule that refuses it belongs in the handler, where the key and the API call meet
// it as well, and not in a second copy here.

#[derive(Args)]
pub struct SendCmd {
    /// An agent id, a workspace with one agent, or workspace/tab
    pub agent: Option<String>,
    /// The message
    pub text: Option<String>,
}

#[derive(Args)]
pub struct ReadCmd {
    /// An agent id, a workspace with one agent, or workspace/tab
    pub agent: Option<String>,
}

#[derive(Args)]
pub struct WaitCmd {
    /// An agent id, a workspace with one agent, or workspace/tab
    pub agent: Option<String>,
    /// Give up after this many milliseconds
    #[arg(long)]
    pub timeout_ms: Option<u64>,
}

pub async fn send(cmd: SendCmd) -> anyhow::Result<()> {
    call(
        "agent.send",
        json!({ "agent": cmd.agent, "text": cmd.text.unwrap_or_default() }),
    )
    .await?;
    Ok(())
}

pub async fn read(cmd: ReadCmd) -> anyhow::Result<()> {
    call("agent.read", json!({ "agent": cmd.agent })).await?;
    Ok(())
}

pub async fn wait(cmd: WaitCmd) -> anyhow::Result<()> {
    call(
        "agent.wait",
        json!({ "agent": cmd.agent, "timeout_ms": cmd.timeout_ms }),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_core::ids::{AgentId, PaneId, WorkspaceId};
    use domux_core::model::agent::AgentState;
    use std::path::PathBuf;

    fn info() -> AgentInfo {
        AgentInfo {
            id: AgentId("a_5e21".into()),
            kind: AgentKind::Claude,
            name: None,
            session_id: Some("s1".into()),
            state: AgentState::Waiting,
            unseen: false,
            recap: None,
            reason: None,
            cwd: PathBuf::from("/repo/audrey-app"),
            pane: Some(PaneId("p_8f2a".into())),
            workspace: WorkspaceId("w_c3a1".into()),
            project: None,
            tab: None,
            place: "audrey-app › main › 1".into(),
            started_at: "2026-09-04T14:32:00+00:00".into(),
            last_activity_at: "2026-09-04T14:32:00+00:00".into(),
        }
    }

    /// Two lines with no name, no recap and nothing unseen: the kind stands in for the name and
    /// the id is on the place line.
    #[test]
    fn a_record_with_nothing_to_add_prints_two_lines() {
        assert_eq!(
            lines_for(&info()),
            "• claude  waiting\n  claude · audrey-app › main › 1  (a_5e21)"
        );
    }

    /// The session name leads, the kind stays on the place line, the dot line says what is
    /// unseen, and the recap is the third line.
    #[test]
    fn a_named_record_with_a_recap_prints_three_lines() {
        let mut a = info();
        a.name = Some("auth refactor".into());
        a.unseen = true;
        a.recap = Some("Replaced three session checks with one guard.".into());
        assert_eq!(
            lines_for(&a),
            "• auth refactor  waiting  unseen\n\
             \x20 claude · audrey-app › main › 1  (a_5e21)\n\
             \x20 ※ Replaced three session checks with one guard."
        );
    }

    /// The state is printed as the model spells it and is never swapped for a working word: the
    /// word belongs to `working` alone, and the command line has no animation to carry it.
    #[test]
    fn every_state_prints_its_own_word() {
        for (state, word) in [
            (AgentState::Working, "working"),
            (AgentState::Waiting, "waiting"),
            (AgentState::Compacting, "compacting"),
            (AgentState::Idle, "idle"),
            (AgentState::Unknown, "unknown"),
        ] {
            let mut a = info();
            a.state = state;
            assert_eq!(
                lines_for(&a).lines().next().unwrap(),
                format!("• claude  {word}")
            );
        }
    }

    #[test]
    fn a_kind_nobody_adapts_is_refused_in_the_words_the_rest_of_domux_refuses_it_in() {
        assert_eq!(parse_kind("codex"), Ok(AgentKind::Codex));
        assert_eq!(
            parse_kind("gemini"),
            Err(r#"unknown agent kind "gemini"; expected claude, codex or opencode"#.to_string())
        );
    }
}
