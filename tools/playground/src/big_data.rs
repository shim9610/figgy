//! The resident large-data bench: up to 100 million generated points, with
//! upload and frame timings and a pan benchmark.
//!
//! The scene itself (the generator, the columns, the series and the fitted
//! chart) is the catalogue's `big_data` module, shared with the renderer's
//! decoration oracle; this file is the bench around it.

use std::time::{Duration, Instant};

use renderer::config::Config;
use renderer::data_config::SeriesConfig;
use renderer::{Chart, Renderer};

use crate::scenes;
pub use crate::scenes::big_data::*;

pub const MIN_POINTS: usize = 100_000;
pub const MAX_POINTS: usize = 100_000_000;
pub const START_POINTS: usize = 5_000_000;
/// Frames of one pan benchmark; the view sweeps one full sine period.
pub const BENCH_FRAMES: usize = 120;
/// Pan amplitude of the benchmark, as a fraction of the visible X width.
const BENCH_PAN: f64 = 0.25;

/// Frames per second for a frame time in milliseconds, with enough digits
/// to stay readable below one frame per second.
pub fn fps(ms: f64) -> String {
    let fps = 1000.0 / ms.max(1e-3);
    if fps >= 10.0 {
        format!("{fps:.0} fps")
    } else {
        format!("{fps:.2} fps")
    }
}

/// Timings of one frame: CPU prepare, then everything up to the GPU finishing
/// the frame (prepare, encode, submit and GPU execution).
#[derive(Clone, Copy, Default)]
pub struct FrameTime {
    pub prepare: Duration,
    pub total: Duration,
}

/// Summary of a finished pan benchmark, in milliseconds.
#[derive(Clone, Copy)]
pub struct BenchReport {
    pub points: usize,
    pub mode: Mode,
    pub mean: f64,
    pub median: f64,
    pub p95: f64,
    pub max: f64,
}

/// What the panel asked for; applied after the panel's other edits commit.
pub enum Action {
    Upload,
    Mode(Mode),
    Bench,
}

pub struct BigData {
    /// Points currently uploaded.
    pub points: usize,
    /// Points the slider asks for; uploaded on `Upload`.
    pub pending: usize,
    pub mode: Mode,
    pub upload_time: Option<Duration>,
    /// Why the last upload failed, shown until the next one succeeds.
    pub upload_error: Option<String>,
    /// Wait for the GPU after every frame and show the timing. Off by
    /// default: the wait blocks the event loop on every repaint, which adds
    /// input lag of its own.
    pub time_frames: bool,
    pub last: Option<FrameTime>,
    /// Running benchmark: the X range it started from and the frames so far.
    bench: Option<(f64, f64, usize)>,
    bench_ms: [f64; BENCH_FRAMES],
    pub report: Option<BenchReport>,
}

impl BigData {
    pub fn new() -> Self {
        Self {
            points: START_POINTS,
            pending: START_POINTS,
            mode: Mode::Line,
            upload_time: None,
            upload_error: None,
            time_frames: false,
            last: None,
            bench: None,
            bench_ms: [0.0; BENCH_FRAMES],
            report: None,
        }
    }

    pub fn benchmarking(&self) -> Option<usize> {
        self.bench.map(|(_, _, frame)| frame)
    }

    /// Whether the host should wait for the GPU after this frame and report
    /// the time.
    pub fn wants_timing(&self) -> bool {
        self.time_frames || self.bench.is_some()
    }

    /// Upload the pending size and redraw the chart for it.
    /// The removal of the old columns takes their series with it whether or
    /// not the upload succeeds, so a failed upload returns an empty chart
    /// (`current` retitled, no legend rows) and keeps the reason.
    pub fn upload(&mut self, r: &mut Renderer, current: Config) -> (Config, Vec<SeriesConfig>) {
        let start = Instant::now();
        let result = upload(r, self.pending).and_then(|()| {
            let series = series(self.mode);
            Ok((chart_config(r, self.pending, &series)?, series))
        });
        let elapsed = start.elapsed();
        self.upload_time = Some(elapsed);
        match result {
            Ok(chart) => {
                eprintln!(
                    "[bench] upload {} points: {:.2} s",
                    group_digits(self.pending),
                    elapsed.as_secs_f64()
                );
                self.points = self.pending;
                self.upload_error = None;
                chart
            }
            Err(e) => {
                let error = format!(
                    "Upload of {} points failed: {e}",
                    group_digits(self.pending)
                );
                eprintln!("[bench] {error}");
                self.points = 0;
                self.upload_error = Some(error);
                let config = Chart::new(current)
                    .with_title("Resident data, nothing uploaded")
                    .config()
                    .clone();
                (scenes::with_legend(config, &[]), Vec::new())
            }
        }
    }

    pub fn start_bench(&mut self, config: &Config) {
        self.bench = Some((config.bottom_x.min, config.bottom_x.max, 0));
        self.report = None;
    }

    /// The X range of the next benchmark frame, `None` when no benchmark
    /// runs. Each frame moves the view, so the axes re-raster and every point
    /// is transformed again, as during a drag.
    pub fn bench_range(&self) -> Option<(f64, f64)> {
        let (min, max, frame) = self.bench?;
        let phase = frame as f64 / BENCH_FRAMES as f64 * std::f64::consts::TAU;
        let shift = (max - min) * BENCH_PAN * phase.sin();
        Some((min + shift, max + shift))
    }

    /// Record a frame. Returns the range to restore once a benchmark ends.
    pub fn frame_done(&mut self, time: FrameTime) -> Option<(f64, f64)> {
        self.last = Some(time);
        let (min, max, frame) = self.bench.as_mut()?;
        self.bench_ms[*frame] = time.total.as_secs_f64() * 1000.0;
        *frame += 1;
        if *frame < BENCH_FRAMES {
            return None;
        }
        let restore = (*min, *max);
        self.bench = None;
        let mut sorted = self.bench_ms;
        sorted.sort_by(f64::total_cmp);
        let at = |q: f64| sorted[((BENCH_FRAMES - 1) as f64 * q).round() as usize];
        let report = BenchReport {
            points: self.points,
            mode: self.mode,
            mean: sorted.iter().sum::<f64>() / BENCH_FRAMES as f64,
            median: at(0.5),
            p95: at(0.95),
            max: sorted[BENCH_FRAMES - 1],
        };
        eprintln!(
            "[bench] {} points, {}: mean {:.2} ms, median {:.2}, p95 {:.2}, max {:.2}",
            group_digits(report.points),
            report.mode.label(),
            report.mean,
            report.median,
            report.p95,
            report.max,
        );
        self.report = Some(report);
        Some(restore)
    }
}
