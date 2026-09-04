use domux_term::{new_emulator, EmulatorConfig, EmulatorKind, Grid, Rgb, Size};
use m0_spike::pty::{spawn, PaneMsg};
use std::time::Duration;

fn emulator(size: Size) -> Box<dyn domux_term::Emulator> {
    new_emulator(
        EmulatorKind::Alacritty,
        EmulatorConfig {
            size,
            scrollback_lines: 100,
            default_fg: Rgb {
                r: 255,
                g: 255,
                b: 255,
            },
            default_bg: Rgb { r: 0, g: 0, b: 0 },
        },
    )
    .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn child_output_reaches_the_grid_and_exit_is_reported() {
    let size = Size { cols: 40, rows: 5 };
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let command = vec![
        "sh".to_string(),
        "-c".to_string(),
        "printf 'hello from pty'; exit 3".to_string(),
    ];
    let mut pane = spawn(0, &command, size, "xterm-256color", emulator(size), tx).unwrap();
    let mut exited = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !exited {
        let msg = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("pane message before deadline")
            .expect("channel open");
        match msg {
            PaneMsg::Output { bytes, .. } => pane.feed(&bytes),
            PaneMsg::Exited { .. } => exited = true,
        }
    }
    let mut grid = Grid::new(size);
    pane.emulator.snapshot_grid(&mut grid);
    let row: String = grid.row(0).iter().map(|c| c.text.as_str()).collect();
    assert!(row.starts_with("hello from pty"), "{row:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn term_is_injected_into_the_child_environment() {
    let size = Size { cols: 40, rows: 5 };
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let command = vec![
        "sh".to_string(),
        "-c".to_string(),
        "printf \"$TERM\"".to_string(),
    ];
    let mut pane = spawn(0, &command, size, "xterm-256color", emulator(size), tx).unwrap();
    while let PaneMsg::Output { bytes, .. } =
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap()
    {
        pane.feed(&bytes);
    }
    let mut grid = Grid::new(size);
    pane.emulator.snapshot_grid(&mut grid);
    let row: String = grid.row(0).iter().map(|c| c.text.as_str()).collect();
    assert!(row.starts_with("xterm-256color"), "{row:?}");
}
