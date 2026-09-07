//! The thin domux client: raw mode, capability negotiation, keys out, frames in.

pub mod caps;
pub mod clipboard;
pub mod control;
pub mod frame;
pub mod input;
pub mod terminal;

use crate::frame::Screen;
use crate::terminal::TerminalGuard;
use anyhow::Context;
use crossterm::event::{Event, EventStream};
// `SERVER_STOPPED` is the reason the server sends when the whole server is going away, rather
// than one view. It is defined beside the message that carries it, so the server writing it and
// this crate reading it are one string rather than two literals a reword could part.
use domux_core::proto::{
    encode, Capabilities, ClientMsg, Decoder, Hello, ServerMsg, PROTOCOL_VERSION, SERVER_STOPPED,
};
use futures::{Stream, StreamExt};
use ratatui::backend::{Backend, CrosstermBackend};
use std::future::Future;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::signal::unix::{signal, SignalKind};

/// How the client's own session ended. The caller prints it, after the terminal is back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachOutcome {
    /// The server ended the session on request; the string is its reason.
    Detached(String),
    ServerStopped,
    ConnectionLost,
    Refused(String),
}

/// The client's own reason when the terminal it draws on has gone.
const TERMINAL_ENDED: &str = "the terminal ended";

fn detach_outcome(reason: String) -> AttachOutcome {
    if reason == SERVER_STOPPED {
        AttachOutcome::ServerStopped
    } else {
        AttachOutcome::Detached(reason)
    }
}

/// V2 runs in its own terminal, not inside tmux (the coexistence table).
fn refuse_inside_tmux(tmux: Option<std::ffi::OsString>) -> anyhow::Result<()> {
    if tmux.is_some() {
        anyhow::bail!(
            "domux2 runs in its own terminal, not inside tmux; open a new Ghostty tab and run it there"
        );
    }
    Ok(())
}

/// Puts text on the outer terminal's clipboard. A field on the session rather than a direct
/// call, so a test can drive the failure path without touching the real clipboard.
type CopyFn = fn(&str, &Capabilities) -> Result<(), String>;

/// What ends the loop from outside the socket and the terminal.
enum Stop {
    Terminate,
    Hangup,
}

/// The session's terminal-facing half: the decoder, the screen and the clipboard route.
/// Split from the socket so the loop can be driven in a test with no terminal at all.
struct Session<B: Backend> {
    caps: Capabilities,
    screen: Screen,
    backend: B,
    dec: Decoder,
    copy: CopyFn,
}

impl<B: Backend> Session<B>
where
    B::Error: std::error::Error + Send + Sync + 'static,
{
    /// Server messages in, terminal events out, until something ends the session. Every exit
    /// leaves through this one return, so the caller's guard restores the terminal once.
    async fn run<R, W, E>(
        &mut self,
        reader: &mut R,
        writer: &mut W,
        events: &mut E,
        stop: impl Future<Output = Stop>,
    ) -> anyhow::Result<AttachOutcome>
    where
        R: AsyncRead + Unpin,
        W: AsyncWrite + Unpin,
        E: Stream<Item = std::io::Result<Event>> + Unpin,
    {
        let mut buf = vec![0u8; 256 * 1024];
        tokio::pin!(stop);
        let outcome = loop {
            tokio::select! {
                read = reader.read(&mut buf) => {
                    let n = match read {
                        Ok(0) | Err(_) => break AttachOutcome::ConnectionLost,
                        Ok(n) => n,
                    };
                    self.dec.push(&buf[..n]);
                    let mut done = None;
                    // A decode error is terminal for the stream, so `?` ends the session and
                    // drops the connection rather than meeting the same bytes again.
                    while let Some(msg) = self.dec.next::<ServerMsg>()? {
                        if let Some(outcome) = self.on_server_msg(msg, writer).await? {
                            done = Some(outcome);
                        }
                    }
                    if let Some(outcome) = done {
                        break outcome;
                    }
                }
                event = events.next() => {
                    match event {
                        Some(Ok(event)) => {
                            if let Some(msg) = self.on_event(event) {
                                // The socket is healthy, so `ConnectionLost` would be a lie and
                                // ending the session over one oversized input would be
                                // disproportionate. Drop the input that could not be framed
                                // and keep the session.
                                match encode(&msg) {
                                    Err(err) => tracing::warn!(%err, "an input was too large to send and was dropped"),
                                    Ok(bytes) => {
                                        if writer.write_all(&bytes).await.is_err() {
                                            break AttachOutcome::ConnectionLost;
                                        }
                                    }
                                }
                            }
                        }
                        // The terminal is gone: nothing left to draw on and no one left to
                        // type. Tell the server rather than leave a view attached to nothing.
                        ended => {
                            if let Some(Err(err)) = ended {
                                tracing::warn!(%err, "the terminal's events ended with an error");
                            }
                            if let Ok(bytes) = encode(&ClientMsg::Detach) {
                                let _ = writer.write_all(&bytes).await;
                            }
                            break AttachOutcome::Detached(TERMINAL_ENDED.into());
                        }
                    }
                }
                stop = &mut stop => match stop {
                    Stop::Terminate => {
                        if let Ok(bytes) = encode(&ClientMsg::Detach) {
                            let _ = writer.write_all(&bytes).await;
                        }
                        break AttachOutcome::Detached("detached".into());
                    }
                    // The terminal hung up, so there is no one to tell and nothing to draw.
                    Stop::Hangup => break AttachOutcome::ConnectionLost,
                },
            }
        };
        Ok(outcome)
    }

    /// One server message. `Some` means the session ended.
    async fn on_server_msg<W: AsyncWrite + Unpin>(
        &mut self,
        msg: ServerMsg,
        writer: &mut W,
    ) -> anyhow::Result<Option<AttachOutcome>> {
        match msg {
            ServerMsg::Welcome { .. } => {}
            ServerMsg::Refused { reason } => return Ok(Some(AttachOutcome::Refused(reason))),
            ServerMsg::Frame(diff) => self.screen.apply(&diff, &mut self.backend)?,
            ServerMsg::Clipboard(text) => {
                if let Err(reason) = (self.copy)(&text, &self.caps) {
                    // The server turns this into a hint. A clipboard that did not happen is
                    // worth saying; it is not worth ending the session over.
                    if let Ok(bytes) = encode(&ClientMsg::ClipboardFailed(reason)) {
                        let _ = writer.write_all(&bytes).await;
                    }
                }
            }
            ServerMsg::Bell => {
                use std::io::Write;
                let mut out = std::io::stdout();
                let _ = out.write_all(b"\x07").and_then(|()| out.flush());
            }
            ServerMsg::Detached { reason } => return Ok(Some(detach_outcome(reason))),
        }
        Ok(None)
    }

    /// The message one terminal event becomes, or `None` for an event with nothing to say.
    fn on_event(&mut self, event: Event) -> Option<ClientMsg> {
        match event {
            Event::Key(key) => input::to_key_event(&key).map(ClientMsg::Key),
            Event::Paste(text) => Some(ClientMsg::Paste(text)),
            // The server answers a resize with a full frame, so the screen starts empty at
            // the new size rather than diffing against a size that is gone.
            Event::Resize(cols, rows) => {
                self.screen = Screen::new(cols, rows);
                Some(ClientMsg::Resize { cols, rows })
            }
            Event::FocusGained => Some(ClientMsg::Focus(true)),
            Event::FocusLost => Some(ClientMsg::Focus(false)),
            _ => None,
        }
    }
}

/// The test-only panic timer. `DOMUX_CLIENT_PANIC_AFTER_MS` makes the client panic after
/// that many milliseconds so a test can prove the terminal comes back. It panics on the
/// client's own task rather than a spawned one: a client left running after its panic hook
/// had already restored the terminal would be a stranger state than the crash it stands in
/// for. An unreadable value asks for no panic at all.
async fn test_panic_timer() -> Stop {
    let ms = std::env::var("DOMUX_CLIENT_PANIC_AFTER_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    match ms {
        Some(ms) => {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            panic!("test panic requested by DOMUX_CLIENT_PANIC_AFTER_MS");
        }
        None => std::future::pending().await,
    }
}

/// Attaches to a running server. Returns after the terminal is restored; the caller prints.
pub async fn attach(socket: &Path) -> anyhow::Result<AttachOutcome> {
    refuse_inside_tmux(std::env::var_os("TMUX"))?;
    let stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connect to {}", socket.display()))?;
    let (mut reader, mut writer) = stream.into_split();
    let env = caps::CapsEnv::from_process();
    let mut capabilities = caps::detect(&env);
    // The hook goes on before raw mode does, so even a panic inside `enter` prints on a
    // terminal the user can still type into.
    TerminalGuard::install_panic_hook(env.keyboard_enhancement);
    let guard = TerminalGuard::enter(env.keyboard_enhancement)?;
    // In raw mode and before the event stream reads stdin: the answers are on stdin, and
    // whichever reader gets there first keeps them.
    let (fg, bg) = caps::query_default_colors(Duration::from_millis(100));
    capabilities.default_fg = fg;
    capabilities.default_bg = bg;
    let (cols, rows) = crossterm::terminal::size()?;
    let hello = ClientMsg::Hello(Hello {
        version: domux_core::VERSION.into(),
        protocol: PROTOCOL_VERSION,
        cols,
        rows,
        caps: capabilities.clone(),
    });
    // Every `?` from here on leaves through the guard's `Drop`, which restores the terminal
    // before the error reaches a caller that prints it (principle 11).
    writer.write_all(&encode(&hello)?).await?;
    let mut session = Session {
        caps: capabilities,
        screen: Screen::new(cols, rows),
        backend: CrosstermBackend::new(std::io::stdout()),
        dec: Decoder::default(),
        copy: clipboard::copy,
    };
    let mut events = EventStream::new();
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sighup = signal(SignalKind::hangup())?;
    let stop = async move {
        tokio::select! {
            _ = sigterm.recv() => Stop::Terminate,
            _ = sighup.recv() => Stop::Hangup,
            stop = test_panic_timer() => stop,
        }
    };
    let outcome = session
        .run(&mut reader, &mut writer, &mut events, stop)
        .await;
    // Explicit as well as on drop, so the terminal is back before the caller prints.
    drop(guard);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyEventKind, KeyEventState, KeyModifiers};
    use domux_core::proto::{CellUpdate, FrameDiff, WireColor, MAX_FRAME};
    use domux_term::{Key, KeyEvent, Mods};
    use futures::channel::mpsc::{unbounded, UnboundedSender};
    use ratatui::backend::TestBackend;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

    type Events = UnboundedSender<std::io::Result<Event>>;
    type Ran = tokio::task::JoinHandle<(anyhow::Result<AttachOutcome>, Session<TestBackend>)>;

    fn clipboard_works(_: &str, _: &Capabilities) -> Result<(), String> {
        Ok(())
    }

    fn clipboard_fails(_: &str, _: &Capabilities) -> Result<(), String> {
        Err("no clipboard tool found".into())
    }

    /// A running session with its socket and its terminal events in the test's hands.
    fn running(cols: u16, rows: u16, copy: CopyFn) -> (Events, DuplexStream, Ran) {
        let (client, server) = tokio::io::duplex(1024 * 1024);
        let (mut reader, mut writer) = tokio::io::split(client);
        let (events_tx, mut events_rx) = unbounded();
        let mut session = Session {
            caps: Capabilities::default(),
            screen: Screen::new(cols, rows),
            backend: TestBackend::new(cols, rows),
            dec: Decoder::default(),
            copy,
        };
        let task = tokio::spawn(async move {
            let outcome = session
                .run(
                    &mut reader,
                    &mut writer,
                    &mut events_rx,
                    std::future::pending::<Stop>(),
                )
                .await;
            (outcome, session)
        });
        (events_tx, server, task)
    }

    fn one_cell_frame(symbol: &str) -> ServerMsg {
        ServerMsg::Frame(FrameDiff {
            full: true,
            cols: 4,
            rows: 2,
            cells: vec![CellUpdate {
                x: 0,
                y: 0,
                symbol: symbol.into(),
                fg: WireColor::Reset,
                bg: WireColor::Reset,
                underline: None,
                modifiers: 0,
            }],
            cursor: None,
        })
    }

    fn key(c: char) -> Event {
        Event::Key(CtKey {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    /// Reads exactly the bytes `expected` would occupy and compares them, so a test fails on
    /// what was sent rather than hanging on what was not.
    async fn expect_frame(server: &mut DuplexStream, expected: &ClientMsg) {
        let want = encode(expected).unwrap();
        let mut got = vec![0u8; want.len()];
        server.read_exact(&mut got).await.unwrap();
        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn a_welcome_and_a_frame_reach_the_screen() {
        let (_events, mut server, task) = running(4, 2, clipboard_works);
        server
            .write_all(
                &encode(&ServerMsg::Welcome {
                    client: domux_core::ids::ClientId("c_1".into()),
                    version: domux_core::VERSION.into(),
                })
                .unwrap(),
            )
            .await
            .unwrap();
        server
            .write_all(&encode(&one_cell_frame("a")).unwrap())
            .await
            .unwrap();
        drop(server);
        let (outcome, session) = task.await.unwrap();
        assert_eq!(outcome.unwrap(), AttachOutcome::ConnectionLost);
        session.backend.assert_buffer_lines(["a   ", "    "]);
    }

    /// The server dies with a frame half written. The bytes that did arrive are not a
    /// message, so nothing is drawn from them and the session ends.
    #[tokio::test]
    async fn a_server_that_vanishes_mid_frame_draws_nothing_and_ends_the_session() {
        let (_events, mut server, task) = running(4, 2, clipboard_works);
        let frame = encode(&one_cell_frame("a")).unwrap();
        server.write_all(&frame[..frame.len() - 1]).await.unwrap();
        drop(server);
        let (outcome, session) = task.await.unwrap();
        assert_eq!(outcome.unwrap(), AttachOutcome::ConnectionLost);
        session.backend.assert_buffer_lines(["    ", "    "]);
    }

    /// A decode error is terminal: the bytes stay in the decoder, so a client that carried
    /// on would meet the same error forever and spin. It drops the connection instead.
    #[tokio::test]
    async fn a_corrupt_frame_ends_the_session_rather_than_repeating_forever() {
        let (_events, mut server, task) = running(4, 2, clipboard_works);
        server.write_all(&4u32.to_be_bytes()).await.unwrap();
        server.write_all(&[0xff, 0xff, 0xff, 0xff]).await.unwrap();
        let (outcome, _) = task.await.unwrap();
        assert!(
            outcome.is_err(),
            "a stream that cannot be decoded must end, not loop"
        );
    }

    /// One paste too large to frame is dropped. The socket is healthy, so ending the whole
    /// session over it would be a lie and a disproportionate loss.
    #[tokio::test]
    async fn a_paste_too_large_to_frame_is_dropped_and_the_session_keeps_going() {
        let (events, mut server, task) = running(4, 2, clipboard_works);
        events
            .unbounded_send(Ok(Event::Paste("x".repeat(MAX_FRAME as usize))))
            .unwrap();
        events.unbounded_send(Ok(key('a'))).unwrap();
        expect_frame(
            &mut server,
            &ClientMsg::Key(KeyEvent::press(Key::Char('a'), Mods::empty())),
        )
        .await;
        drop(server);
        let (outcome, _) = task.await.unwrap();
        assert_eq!(outcome.unwrap(), AttachOutcome::ConnectionLost);
    }

    #[tokio::test]
    async fn a_resize_replaces_the_screen_and_tells_the_server() {
        let (events, mut server, task) = running(4, 2, clipboard_works);
        events.unbounded_send(Ok(Event::Resize(10, 3))).unwrap();
        expect_frame(&mut server, &ClientMsg::Resize { cols: 10, rows: 3 }).await;
        drop(events);
        let (_, session) = task.await.unwrap();
        assert_eq!(
            session.screen.buffer.area,
            ratatui::layout::Rect::new(0, 0, 10, 3),
            "the next diff is measured against the new size, not the old one"
        );
    }

    /// The outer terminal went away. The client tells the server rather than leaving a view
    /// attached to a terminal that no longer exists.
    #[tokio::test]
    async fn a_terminal_that_ends_detaches_and_says_so() {
        let (events, mut server, task) = running(4, 2, clipboard_works);
        drop(events);
        expect_frame(&mut server, &ClientMsg::Detach).await;
        let (outcome, _) = task.await.unwrap();
        assert_eq!(
            outcome.unwrap(),
            AttachOutcome::Detached("the terminal ended".into())
        );
    }

    #[tokio::test]
    async fn a_clipboard_that_could_not_be_written_is_reported_to_the_server() {
        let (_events, mut server, task) = running(4, 2, clipboard_fails);
        server
            .write_all(&encode(&ServerMsg::Clipboard("hello".into())).unwrap())
            .await
            .unwrap();
        expect_frame(
            &mut server,
            &ClientMsg::ClipboardFailed("no clipboard tool found".into()),
        )
        .await;
        drop(server);
        task.await.unwrap().0.unwrap();
    }

    #[tokio::test]
    async fn a_refusal_ends_the_session_with_the_servers_own_reason() {
        let (_events, mut server, task) = running(4, 2, clipboard_works);
        server
            .write_all(
                &encode(&ServerMsg::Refused {
                    reason: "the server is domux 2.0.0 and this client is 1.9.0".into(),
                })
                .unwrap(),
            )
            .await
            .unwrap();
        let (outcome, _) = task.await.unwrap();
        assert_eq!(
            outcome.unwrap(),
            AttachOutcome::Refused("the server is domux 2.0.0 and this client is 1.9.0".into())
        );
    }

    /// The server sends one message for both, and the reason is the only thing that tells
    /// them apart, so pin the string the server actually sends.
    #[test]
    fn a_server_that_stopped_is_a_different_outcome_from_a_detach() {
        assert_eq!(
            detach_outcome(SERVER_STOPPED.into()),
            AttachOutcome::ServerStopped
        );
        assert_eq!(
            detach_outcome("detached".into()),
            AttachOutcome::Detached("detached".into())
        );
    }

    #[test]
    fn attach_refuses_to_run_inside_tmux() {
        assert!(refuse_inside_tmux(None).is_ok());
        let err = refuse_inside_tmux(Some("/tmp/tmux-501/default,1,0".into()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("tmux"), "{err}");
    }
}
