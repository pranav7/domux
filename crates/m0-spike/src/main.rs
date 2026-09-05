//! M0 pane spike: N PTY panes drawn with ratatui in one process, no frame timer.

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::event::{
    Event, EventStream, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{cursor::SetCursorStyle, execute};
use domux_term::{new_emulator, CursorShape, EmulatorConfig, EmulatorKind, Grid, Rgb, Size};
use futures::StreamExt;
use m0_spike::input::{to_key_event, Action, Chord};
use m0_spike::pty::{spawn, Pane, PaneMsg};
use m0_spike::render::{cursor_position, inner_area, PaneWidget};
use m0_spike::stats::Stats;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::Terminal;
use std::io::{stdout, Stdout, Write};
use std::time::{Duration, Instant};
use tokio::signal::unix::{signal, SignalKind};

#[derive(Parser, Debug)]
#[command(name = "m0-spike", about = "domux M0 pane spike")]
struct Args {
    #[arg(long, default_value = "ghostty")]
    emulator: EmulatorKind,
    #[arg(long, default_value_t = 1)]
    panes: usize,
    #[arg(long, default_value = "xterm-256color")]
    term: String,
    #[arg(long)]
    no_border: bool,
    #[arg(long, default_value = "#cdd6f4", value_parser = parse_rgb)]
    fg: Rgb,
    #[arg(long, default_value = "#1e1e2e", value_parser = parse_rgb)]
    bg: Rgb,
    #[arg(long, default_value_t = 10000)]
    scrollback: usize,
    #[arg(long)]
    exit_after: Option<u64>,
    #[arg(long)]
    stats_out: Option<std::path::PathBuf>,
    #[arg(long, default_value_t = 5)]
    settle: u64,
    /// Command for every pane. Default: $SHELL.
    #[arg(last = true)]
    command: Vec<String>,
}

fn parse_rgb(s: &str) -> Result<Rgb, String> {
    let hex = s.strip_prefix('#').ok_or("expected #rrggbb")?;
    if hex.len() != 6 {
        return Err("expected #rrggbb".into());
    }
    let p = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| e.to_string());
    Ok(Rgb {
        r: p(0)?,
        g: p(2)?,
        b: p(4)?,
    })
}

/// Restores the outer terminal on drop, including during a panic.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        execute!(
            stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        Ok(TerminalGuard)
    }
    fn restore() {
        let _ = execute!(
            stdout(),
            PopKeyboardEnhancementFlags,
            DisableBracketedPaste,
            SetCursorStyle::DefaultUserShape,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        TerminalGuard::restore();
    }
}

fn pane_areas(area: Rect, n: usize) -> Vec<Rect> {
    // Fixed grid: up to 2 columns, rows as needed. Enough for eight panes of `yes`.
    let cols = if n > 1 { 2 } else { 1 };
    let rows = n.div_ceil(cols);
    let row_areas = Layout::vertical(vec![Constraint::Ratio(1, rows as u32); rows]).split(area);
    let mut out = Vec::with_capacity(n);
    for row_area in row_areas.iter().take(rows) {
        let col_areas =
            Layout::horizontal(vec![Constraint::Ratio(1, cols as u32); cols]).split(*row_area);
        for col_area in col_areas.iter().take(cols) {
            if out.len() < n {
                out.push(*col_area);
            }
        }
    }
    out
}

fn grid_size(area: Rect, border: bool) -> Size {
    let inner = inner_area(area, border);
    Size {
        cols: inner.width.max(1),
        rows: inner.height.max(1),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        TerminalGuard::restore();
        default_hook(info);
    }));
    let guard = TerminalGuard::enter()?;
    let result = run(&args).await;
    drop(guard);
    result
}

async fn run(args: &Args) -> Result<()> {
    let border = !args.no_border;
    let mut terminal =
        Terminal::new(CrosstermBackend::new(stdout())).context("ratatui terminal")?;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<PaneMsg>(256);
    let size = terminal.size()?;
    let areas = pane_areas(Rect::new(0, 0, size.width, size.height), args.panes);
    let mut panes: Vec<Pane> = Vec::with_capacity(args.panes);
    for (id, area) in areas.iter().enumerate() {
        let size = grid_size(*area, border);
        let emulator = new_emulator(
            args.emulator,
            EmulatorConfig {
                size,
                scrollback_lines: args.scrollback,
                default_fg: args.fg,
                default_bg: args.bg,
            },
        )
        .map_err(anyhow::Error::msg)?;
        panes.push(spawn(
            id,
            &args.command,
            size,
            &args.term,
            emulator,
            tx.clone(),
        )?);
    }
    drop(tx);

    let mut focused = 0usize;
    let mut chord = Chord::default();
    let mut grids: Vec<Grid> = panes.iter().map(|p| Grid::new(p.emulator.size())).collect();
    let mut stats = Stats::new(Duration::from_secs(args.settle));
    let mut events = EventStream::new();
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let deadline = args
        .exit_after
        .map(|s| tokio::time::Instant::now() + Duration::from_secs(s));
    let mut last_shape: Option<CursorShape> = None;
    let mut encoded = Vec::new();

    'main: loop {
        stats.tick();
        let mut render = panes.iter().any(|p| p.dirty);
        if !render {
            let timeout = async {
                match deadline {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                Some(msg) = rx.recv() => handle_pane_msg(&mut panes, msg, &mut stats),
                Some(Ok(ev)) = events.next() => {
                    match handle_event(ev, &mut panes, &mut focused, &mut chord, &mut encoded, &mut terminal, border) {
                        Ok(true) => break 'main,
                        Ok(false) => {}
                        Err(e) => return Err(e),
                    }
                }
                _ = sigterm.recv() => break 'main,
                _ = sigint.recv() => break 'main,
                _ = timeout => break 'main,
            }
        }
        // Drain what arrived without waiting, bounded so a flood cannot starve rendering.
        for _ in 0..64 {
            match rx.try_recv() {
                Ok(msg) => handle_pane_msg(&mut panes, msg, &mut stats),
                Err(_) => break,
            }
        }
        if panes.iter().all(|p| p.exited) {
            break 'main;
        }
        render = panes.iter().any(|p| p.dirty);
        if !render {
            continue;
        }
        let frame_start = Instant::now();
        for (p, g) in panes.iter_mut().zip(grids.iter_mut()) {
            if p.dirty {
                p.emulator.snapshot_grid(g);
                p.dirty = false;
            }
        }
        let cursor = panes[focused].emulator.cursor();
        let mut draw_done = frame_start;
        terminal.draw(|f| {
            let areas = pane_areas(f.area(), panes.len());
            for (i, area) in areas.iter().enumerate() {
                f.render_widget(
                    PaneWidget {
                        grid: &grids[i],
                        focused: i == focused,
                        border,
                    },
                    *area,
                );
            }
            if let Some(pos) = cursor_position(areas[focused], border, &cursor) {
                f.set_cursor_position(pos);
            }
            draw_done = Instant::now();
        })?;
        if last_shape != Some(cursor.shape) {
            let style = match (cursor.shape, cursor.blink) {
                (CursorShape::Block, true) => SetCursorStyle::BlinkingBlock,
                (CursorShape::Block, false) => SetCursorStyle::SteadyBlock,
                (CursorShape::Underline, true) => SetCursorStyle::BlinkingUnderScore,
                (CursorShape::Underline, false) => SetCursorStyle::SteadyUnderScore,
                (CursorShape::Bar, true) => SetCursorStyle::BlinkingBar,
                (CursorShape::Bar, false) => SetCursorStyle::SteadyBar,
            };
            execute!(stdout(), style)?;
            last_shape = Some(cursor.shape);
        }
        stdout().flush()?;
        let end = Instant::now();
        stats.record_frame(end - frame_start, draw_done - frame_start, end - draw_done);
    }

    for p in &mut panes {
        p.kill();
    }
    let outer = terminal.size()?;
    let report = stats.finish(
        &format!("{:?}", args.emulator).to_lowercase(),
        args.panes,
        &args.command,
        &args.term,
        (outer.width, outer.height),
    );
    if let Some(path) = &args.stats_out {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&report)?)?;
        std::fs::rename(&tmp, path)?;
    }
    Ok(())
}

fn handle_pane_msg(panes: &mut [Pane], msg: PaneMsg, stats: &mut Stats) {
    match msg {
        PaneMsg::Output { pane, bytes } => {
            let t = Instant::now();
            panes[pane].feed(&bytes);
            stats.record_feed(t.elapsed());
            stats.bytes_fed += bytes.len() as u64;
        }
        PaneMsg::Exited { pane } => panes[pane].mark_exited(),
    }
}

/// Returns Ok(true) when the loop should end.
fn handle_event(
    ev: Event,
    panes: &mut [Pane],
    focused: &mut usize,
    chord: &mut Chord,
    encoded: &mut Vec<u8>,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    border: bool,
) -> Result<bool> {
    match ev {
        Event::Key(k) => {
            let Some(key) = to_key_event(&k) else {
                return Ok(false);
            };
            match chord.handle(key) {
                Action::Quit => return Ok(true),
                Action::NextPane => {
                    *focused = (*focused + 1) % panes.len();
                    for p in panes.iter_mut() {
                        p.dirty = true;
                    }
                }
                Action::Nothing => {}
                Action::Forward(key) => {
                    encoded.clear();
                    panes[*focused].emulator.encode_key(&key, encoded);
                    if !encoded.is_empty() {
                        panes[*focused].write(encoded)?;
                    }
                }
            }
        }
        Event::Paste(text) => {
            encoded.clear();
            panes[*focused].emulator.encode_paste(&text, encoded);
            panes[*focused].write(encoded)?;
        }
        Event::Resize(_, _) => {
            terminal.autoresize()?;
            let size = terminal.size()?;
            let areas = pane_areas(Rect::new(0, 0, size.width, size.height), panes.len());
            for (p, area) in panes.iter_mut().zip(areas) {
                p.resize(grid_size(area, border))?;
            }
        }
        _ => {}
    }
    Ok(false)
}
