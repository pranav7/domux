//! Records a program's output under a PTY into a golden fixture, answering its terminal
//! queries through an emulator so the program behaves as it would in a pane.
//!
//! cargo run -p domux-term --example record -- --size 120x40 --seconds 3 \
//!   --out crates/domux-term/fixtures/golden/htop.120x40.bytes -- htop

use clap::Parser;
use domux_term::{new_emulator, EmulatorConfig, EmulatorKind, Rgb, Size};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "80x24")]
    size: String,
    #[arg(long, default_value_t = 3)]
    seconds: u64,
    #[arg(long, default_value = "alacritty")]
    emulator: EmulatorKind,
    #[arg(long, default_value = "xterm-256color")]
    term: String,
    #[arg(long)]
    out: std::path::PathBuf,
    /// Keys to type after one second, as a raw string (for example "q" or ":q\r").
    #[arg(long)]
    keys: Option<String>,
    #[arg(last = true)]
    command: Vec<String>,
}

fn main() {
    let args = Args::parse();
    let (cols, rows) = args.size.split_once('x').expect("size as COLSxROWS");
    let size = Size {
        cols: cols.parse().unwrap(),
        rows: rows.parse().unwrap(),
    };
    let mut emulator = new_emulator(
        args.emulator,
        EmulatorConfig {
            size,
            scrollback_lines: 1000,
            default_fg: Rgb {
                r: 0xcd,
                g: 0xd6,
                b: 0xf4,
            },
            default_bg: Rgb {
                r: 0x1e,
                g: 0x1e,
                b: 0x2e,
            },
        },
    )
    .expect("emulator enabled");

    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut cmd = CommandBuilder::new(&args.command[0]);
    cmd.args(&args.command[1..]);
    cmd.env("TERM", &args.term);
    cmd.env("COLORTERM", "truecolor");
    cmd.env("LANG", "en_US.UTF-8");
    cmd.env_remove("TMUX");
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut writer = pair.master.take_writer().unwrap();

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 65536];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let mut recording = Vec::new();
    let mut replies = Vec::new();
    let start = Instant::now();
    let mut typed = false;
    while start.elapsed() < Duration::from_secs(args.seconds) {
        if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(50)) {
            emulator.feed(&chunk);
            emulator.take_responses(&mut replies);
            if !replies.is_empty() {
                writer.write_all(&replies).unwrap();
                writer.flush().unwrap();
                replies.clear();
            }
            recording.extend_from_slice(&chunk);
        }
        if !typed && start.elapsed() >= Duration::from_secs(1) {
            if let Some(keys) = &args.keys {
                writer.write_all(keys.as_bytes()).unwrap();
                writer.flush().unwrap();
            }
            typed = true;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let tmp = args.out.with_extension("bytes.tmp");
    std::fs::write(&tmp, &recording).unwrap();
    std::fs::rename(&tmp, &args.out).unwrap();
    eprintln!("wrote {} bytes to {}", recording.len(), args.out.display());
}
