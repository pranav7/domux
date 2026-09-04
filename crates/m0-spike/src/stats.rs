//! Frame timing and resource numbers for the spike criteria, written as JSON at exit.

use serde::Serialize;
use std::time::{Duration, Instant};

#[derive(Serialize, Default, Clone, Copy)]
pub struct Percentiles {
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub max: u64,
}

fn percentiles(samples: &mut [u64]) -> Percentiles {
    if samples.is_empty() {
        return Percentiles::default();
    }
    samples.sort_unstable();
    let at = |q: f64| samples[((samples.len() as f64 - 1.0) * q).round() as usize];
    Percentiles {
        p50: at(0.50),
        p95: at(0.95),
        p99: at(0.99),
        max: *samples.last().unwrap(),
    }
}

#[derive(Serialize)]
pub struct Report {
    pub emulator: String,
    pub panes: usize,
    pub command: Vec<String>,
    pub term: String,
    pub outer_cols: u16,
    pub outer_rows: u16,
    pub duration_ms: u64,
    pub frames: u64,
    pub bytes_fed: u64,
    /// Whole render pass: snapshot every dirty grid, draw, flush to the outer terminal.
    pub frame_us: Percentiles,
    /// Emulator feed time per drained batch.
    pub feed_us: Percentiles,
    /// Snapshot plus ratatui draw, without the flush.
    pub draw_us: Percentiles,
    /// Writing the diff to the outer terminal.
    pub flush_us: Percentiles,
    pub cpu_ms_total: u64,
    /// CPU time consumed after `settle_s` seconds; the idle criterion divides this by the
    /// remaining wall time.
    pub cpu_ms_after_settle: u64,
    pub settle_s: u64,
    pub max_rss_kb: u64,
}

pub struct Stats {
    start: Instant,
    settle: Duration,
    cpu_at_settle: Option<Duration>,
    frame: Vec<u64>,
    feed: Vec<u64>,
    draw: Vec<u64>,
    flush: Vec<u64>,
    pub bytes_fed: u64,
}

impl Stats {
    pub fn new(settle: Duration) -> Self {
        Stats {
            start: Instant::now(),
            settle,
            cpu_at_settle: None,
            frame: Vec::new(),
            feed: Vec::new(),
            draw: Vec::new(),
            flush: Vec::new(),
            bytes_fed: 0,
        }
    }

    pub fn record_frame(&mut self, frame: Duration, draw: Duration, flush: Duration) {
        self.frame.push(frame.as_micros() as u64);
        self.draw.push(draw.as_micros() as u64);
        self.flush.push(flush.as_micros() as u64);
    }

    pub fn record_feed(&mut self, d: Duration) {
        self.feed.push(d.as_micros() as u64);
    }

    /// Call on every loop iteration; captures CPU time once the settle period passes.
    pub fn tick(&mut self) {
        if self.cpu_at_settle.is_none() && self.start.elapsed() >= self.settle {
            self.cpu_at_settle = Some(cpu_time());
        }
    }

    pub fn finish(
        mut self,
        emulator: &str,
        panes: usize,
        command: &[String],
        term: &str,
        outer: (u16, u16),
    ) -> Report {
        let total = cpu_time();
        let after = self
            .cpu_at_settle
            .map(|s| total.saturating_sub(s))
            .unwrap_or_default();
        Report {
            emulator: emulator.to_string(),
            panes,
            command: command.to_vec(),
            term: term.to_string(),
            outer_cols: outer.0,
            outer_rows: outer.1,
            duration_ms: self.start.elapsed().as_millis() as u64,
            frames: self.frame.len() as u64,
            bytes_fed: self.bytes_fed,
            frame_us: percentiles(&mut self.frame),
            feed_us: percentiles(&mut self.feed),
            draw_us: percentiles(&mut self.draw),
            flush_us: percentiles(&mut self.flush),
            cpu_ms_total: total.as_millis() as u64,
            cpu_ms_after_settle: after.as_millis() as u64,
            settle_s: self.settle.as_secs(),
            max_rss_kb: max_rss_kb(),
        }
    }
}

fn rusage() -> libc::rusage {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    ru
}

fn cpu_time() -> Duration {
    let ru = rusage();
    let tv = |t: libc::timeval| Duration::new(t.tv_sec as u64, (t.tv_usec as u32) * 1000);
    tv(ru.ru_utime) + tv(ru.ru_stime)
}

fn max_rss_kb() -> u64 {
    let ru = rusage();
    // macOS reports bytes, Linux kilobytes.
    if cfg!(target_os = "macos") {
        ru.ru_maxrss as u64 / 1024
    } else {
        ru.ru_maxrss as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_pick_the_expected_samples() {
        let mut s: Vec<u64> = (1..=100).collect();
        let p = percentiles(&mut s);
        assert_eq!((p.p50, p.p95, p.p99, p.max), (51, 95, 99, 100));
    }
}
