//! Resident large-data bench: up to 100 million generated points drawn as a
//! line, as markers or as both, with upload and frame timings.
//!
//! The data is resident: both columns live in the GPU column pool and every
//! point is drawn on every frame. The generator writes each value straight
//! into the pool's mapped staging range through `HiLoColumnSource`, so no
//! dataset-sized CPU buffer exists at any point.

use std::time::{Duration, Instant};

use renderer::config::Config;
use renderer::data::split_f64_to_f32_pair;
use renderer::data_config::{DataRenderType, ScatterShape, SeriesConfig};
use renderer::{
    Chart, Color, ColumnPairWriter, ColumnUploadStats, FiggyError, HiLoColumnSource, Renderer,
};

use crate::scenes::{self, CartesianScene};

pub const MIN_POINTS: usize = 100_000;
pub const MAX_POINTS: usize = 100_000_000;
pub const START_POINTS: usize = 5_000_000;
const X_ID: &str = "big_x";
const Y_ID: &str = "big_y";
/// The X column spans `0..=X_SPAN`; hi/lo pairs keep 100 million samples
/// distinct where a single f32 could not.
const X_SPAN: f64 = 1000.0;
/// Frames of one pan benchmark; the view sweeps one full sine period.
pub const BENCH_FRAMES: usize = 120;
/// Pan amplitude of the benchmark, as a fraction of the visible X width.
const BENCH_PAN: f64 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Line,
    Scatter,
    Both,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Line, Mode::Scatter, Mode::Both];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Line => "Line",
            Mode::Scatter => "Scatter",
            Mode::Both => "Line + scatter",
        }
    }
}

/// One generated column, evaluated per index while it is written.
struct Generated {
    len: usize,
    y: bool,
    min: f64,
    max: f64,
}

impl Generated {
    fn x(len: usize) -> Self {
        Self {
            len,
            y: false,
            min: 0.0,
            max: X_SPAN,
        }
    }

    /// The Y bounds are the exact extremes of the generated values: one pass
    /// that evaluates the signal and keeps two scalars.
    fn y(len: usize) -> Self {
        let (min, max) = (0..len).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), i| {
            let v = y_at(len, i);
            (lo.min(v), hi.max(v))
        });
        Self {
            len,
            y: true,
            min,
            max,
        }
    }

    fn value(&self, i: usize) -> f64 {
        if self.y {
            y_at(self.len, i)
        } else {
            x_at(self.len, i)
        }
    }
}

fn x_at(len: usize, i: usize) -> f64 {
    X_SPAN * i as f64 / (len.max(2) - 1) as f64
}

/// Two waves and per-sample noise: a line with structure at every zoom level
/// and a scatter band with visible density.
fn y_at(len: usize, i: usize) -> f64 {
    let x = x_at(len, i);
    30.0 * (x / 40.0).sin() + 8.0 * (0.9 * x).sin() + 4.0 * scenes::noise(7, i)
}

impl HiLoColumnSource for Generated {
    fn len(&self) -> usize {
        self.len
    }
    fn min(&self) -> f64 {
        self.min
    }
    fn max(&self) -> f64 {
        self.max
    }

    fn write_f32_pair_le_into(&self, dst: &mut [u8]) {
        for (i, pair) in dst.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            let (hi, lo) = split_f64_to_f32_pair(self.value(i));
            pair[..4].copy_from_slice(&hi.to_le_bytes());
            pair[4..].copy_from_slice(&lo.to_le_bytes());
        }
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        mut dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        let mut min_positive = f64::INFINITY;
        for i in 0..self.len {
            let (hi, lo) = split_f64_to_f32_pair(self.value(i));
            dst.write_pair(i, hi, lo);
            let v = f64::from(hi) + f64::from(lo);
            if v > 0.0 && v.is_finite() {
                min_positive = min_positive.min(v);
            }
        }
        ColumnUploadStats {
            min_positive: min_positive.is_finite().then_some(min_positive),
        }
    }
}

/// Upload `points` samples of both columns, replacing any earlier ones, and
/// wait until the GPU holds them. Removing a column also removes the series
/// that read it, so callers set the series again afterwards.
fn upload(r: &mut Renderer, points: usize) -> Result<(), FiggyError> {
    r.remove_column(X_ID)?;
    r.remove_column(Y_ID)?;
    r.add_hilo_column(X_ID, &Generated::x(points))?;
    r.add_hilo_column(Y_ID, &Generated::y(points))?;
    // A failed wait only makes the upload time read short.
    if let Err(e) = r.device().poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    }) {
        eprintln!("[big data] poll: {e}");
    }
    Ok(())
}

pub fn series(mode: Mode) -> Vec<SeriesConfig> {
    let line = scenes::line_style(Color::from_rgb8(31, 119, 180), 1.0);
    let scatter = scenes::scatter_style(Color::from_rgb8(214, 39, 40), ScatterShape::Circle, 2.0);
    let render_type = match mode {
        Mode::Line => DataRenderType::Line { line },
        Mode::Scatter => DataRenderType::Scatter { scatter },
        Mode::Both => DataRenderType::ScatterLine { scatter, line },
    };
    vec![scenes::series("samples", X_ID, Y_ID, render_type)]
}

fn title(points: usize) -> String {
    format!("Resident data, {} points", group_digits(points))
}

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

/// `5000000` as `5,000,000`.
pub fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The chart for `points` samples, fitted to the generated columns.
fn chart_config(
    r: &Renderer,
    points: usize,
    series: &[SeriesConfig],
) -> Result<Config, FiggyError> {
    let mut chart = Chart::new(scenes::base_config(&title(points), "x", "value"));
    chart.auto_fit_x(r.pool(), X_ID, 0.0)?;
    chart.auto_fit_y(r.pool(), Y_ID, 0.05)?;
    Ok(scenes::with_legend(chart.config().clone(), series))
}

/// The scene at its start size; the bench state keeps the size afterwards.
pub fn build(r: &mut Renderer) -> Result<CartesianScene, FiggyError> {
    upload(r, START_POINTS)?;
    let series = series(Mode::Line);
    Ok(CartesianScene {
        config: chart_config(r, START_POINTS, &series)?,
        series,
    })
}

/// Release the columns when the scene goes away, so other scenes get the
/// pool space back.
pub fn release(r: &mut Renderer) {
    for id in [X_ID, Y_ID] {
        if let Err(e) = r.remove_column(id) {
            eprintln!("[big data] remove {id}: {e}");
        }
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
