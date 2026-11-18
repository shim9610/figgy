//! Monotonic-x line series (#271): the envelope draw against the full line
//! draw of the same data (M2), the four calls that take the source and move
//! the x-to-pixel mapping (`set_monotonic`, `append_monotonic`,
//! `resize_monotonic`, `fit_layout`), the general calls that would move it
//! and are refused, the settings that need no source, and every interaction
//! (M6), through the public registered-chart calls.
//!
//! The pixel oracle compares a chart drawing the monotonic series with a
//! chart drawing the same samples as an ordinary line through host
//! columns, on one renderer:
//!
//! - **Sparse views** (at most 4 visible samples per pixel column) keep
//!   every sample, so the two images are byte-identical.
//! - **Dense views** draw first, lowest, highest and last per column. Their
//!   painted pixels are the full draw's within one pixel, both ways: the
//!   full draw stacks one antialiased stroke per sample, whose fringes
//!   reach one pixel into the neighbouring columns from wherever the sample
//!   lies in its column, while the envelope draws four.
//! - An envelope built by appends, or slid by whole columns, is
//!   byte-identical to a full call on the same window; slid by part of a
//!   column, it is the full draw's within one pixel.

use test_gpu::BlockOn as _;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::OnceLock;

use renderer::layout::{ChartArea, Rect};
use renderer::renderer::{MonotonicGroupRef, MonotonicRange};
use renderer::{
    ChartId, Color, ExportOptions, ExportSource, FiggyError, MappingChange, MonotonicAppend,
    MonotonicChartId, MonotonicInterleaved, MonotonicSlices, MonotonicSource, MonotonicUpdate,
    RasterImage, Renderer, RendererDevice, SeriesConfig,
};

fn shared_device() -> (wgpu::Device, wgpu::Queue) {
    static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(test_gpu::device);
    (device.clone(), queue.clone())
}

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Bgra8Unorm,
        64 * 1024 * 1024,
    )
    .expect("renderer init")
}

/// A chart with grid and legend off, so the images hold the series and the
/// axes only.
fn bare_config(width: u32, height: u32, x: (f64, f64), y: (f64, f64)) -> renderer::Config {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width,
        height,
    });
    config.legend.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    config.bottom_x.min = x.0;
    config.bottom_x.max = x.1;
    config.left_y.min = y.0;
    config.left_y.max = y.1;
    config
}

const BLUE: Color = Color {
    r: 0.1,
    g: 0.2,
    b: 0.9,
    a: 1.0,
};

/// A deterministic signal: two sines and a little noise.
fn signal(i: usize) -> f64 {
    let t = i as f64 * 1e-3;
    (t * 3.0).sin() + 0.3 * (t * 37.0).sin() + 0.05 * ((i * 2654435761) % 1000) as f64 / 1000.0
}

fn signal_values(range: std::ops::Range<usize>) -> Vec<f64> {
    range.map(signal).collect()
}

/// A chart the tests export at the screen's mapping: a general chart, or a
/// monotonic one with no sources lent.
trait Exported: Copy {
    fn source(self) -> ExportSource<'static>;
}

impl Exported for ChartId {
    fn source(self) -> ExportSource<'static> {
        ExportSource::Registered(self)
    }
}

impl Exported for MonotonicChartId {
    fn source(self) -> ExportSource<'static> {
        ExportSource::Monotonic {
            chart: self,
            monotonic: &[],
        }
    }
}

fn export(r: &mut Renderer, id: impl Exported, scale: f32) -> Result<RasterImage, FiggyError> {
    r.export_rgba(id.source(), ExportOptions::panel(scale))
        .block_on()
}

/// Monotonic chart `id`'s document replaced by `config` through its editor
/// (`MonotonicConfig::assign`): refused when it would move the mapping.
fn edit(
    r: &mut Renderer,
    id: MonotonicChartId,
    config: renderer::Config,
) -> Result<(), FiggyError> {
    r.monotonic_chart(id)?
        .edit_config(|editor| editor.assign(config))?
        .map_err(|kind| FiggyError::InvalidConfig {
            field: "config",
            kind,
        })
}

fn export_with(
    r: &mut Renderer,
    id: MonotonicChartId,
    scale: f32,
    groups: &[MonotonicGroupRef<'_>],
) -> RasterImage {
    r.export_rgba(
        ExportSource::Monotonic {
            chart: id,
            monotonic: groups,
        },
        ExportOptions::panel(scale),
    )
    .block_on()
    .expect("export with sources")
}

/// Whether a pixel is painted at all.
fn painted(img: &RasterImage, x: u32, y: u32) -> bool {
    img.rgba[((y * img.width + x) * 4 + 3) as usize] > 0
}

/// The pixels painted in one image with nothing painted within `r` pixels
/// in the other, both ways.
fn misses(a: &RasterImage, b: &RasterImage, r: u32) -> usize {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let near = |img: &RasterImage, x: u32, y: u32| {
        (x.saturating_sub(r)..=(x + r).min(img.width - 1)).any(|xx| {
            (y.saturating_sub(r)..=(y + r).min(img.height - 1)).any(|yy| painted(img, xx, yy))
        })
    };
    let mut count = 0;
    for y in 0..a.height {
        for x in 0..a.width {
            if painted(a, x, y) && !near(b, x, y) {
                count += 1;
            }
            if painted(b, x, y) && !near(a, x, y) {
                count += 1;
            }
        }
    }
    count
}

fn differing(a: &RasterImage, b: &RasterImage) -> usize {
    assert_eq!((a.width, a.height), (b.width, b.height));
    a.rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .filter(|(pa, pb)| pa != pb)
        .count()
}

/// The two images are byte-identical.
fn assert_identical(a: &RasterImage, b: &RasterImage, what: &str) {
    assert_eq!(differing(a, b), 0, "{what}: pixels differ");
}

/// Every pixel either image paints lies within one pixel of a pixel the
/// other paints.
fn assert_within_one_pixel(a: &RasterImage, b: &RasterImage, what: &str) {
    assert_eq!(
        misses(a, b, 1),
        0,
        "{what}: painted pixels further apart than 1 px"
    );
}

/// A renderer with two charts on one config: the monotonic series `m` of
/// group `g` (channel 0), and the same samples as a line through host
/// columns `x` / `y`.
struct Pair {
    r: Renderer,
    monotonic: MonotonicChartId,
    full: ChartId,
}

fn monotonic_series(width: f32) -> SeriesConfig {
    named_monotonic("m", width)
}

/// Another monotonic series of group `g`: envelope columns belong to one
/// series of one chart, so a second chart names its own.
fn named_monotonic(id: &str, width: f32) -> SeriesConfig {
    SeriesConfig::monotonic_line(id, "g", 0)
        .color(BLUE)
        .width(width)
}

fn full_series(width: f32) -> SeriesConfig {
    SeriesConfig::line("full", "x", "y")
        .color(BLUE)
        .width(width)
}

/// Uploads `values` at `start + i × interval` as the host columns of the
/// full draw.
fn upload_full(r: &mut Renderer, values: &[f64], start: f64, interval: f64) {
    let xs: Vec<f64> = (0..values.len())
        .map(|i| start + i as f64 * interval)
        .collect();
    r.upsert_column("x", &xs).expect("x");
    r.upsert_column("y", &values.to_vec()).expect("y");
}

/// The full call of group `g` on chart `id`, `values` at `start + i ×
/// interval`, at the x range the document's x axis has.
fn set(r: &mut Renderer, id: MonotonicChartId, values: &[f64], start: f64, interval: f64) {
    let (min, max) = r
        .monotonic_config(id)
        .expect("config")
        .bottom_x
        .data_range();
    set_range(
        r,
        id,
        values,
        start,
        interval,
        MonotonicRange::X { min, max },
    )
    .expect("set_monotonic");
}

/// The full call of group `g` on chart `id` with `range`.
fn set_range(
    r: &mut Renderer,
    id: MonotonicChartId,
    values: &[f64],
    start: f64,
    interval: f64,
    range: MonotonicRange,
) -> Result<MonotonicUpdate, FiggyError> {
    let channels = [values];
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_monotonic(
            "g",
            &MonotonicSlices::new(start, interval, &channels),
            range,
        )
}

/// `call` with group `g`'s whole source lent: `values` from 0 at
/// interval 1.
fn lent<T>(values: &[f64], call: impl FnOnce(&[MonotonicGroupRef<'_>]) -> T) -> T {
    let channels = [values];
    let source = MonotonicSlices::new(0.0, 1.0, &channels);
    call(&[MonotonicGroupRef::new("g", &source)])
}

fn pair(config: renderer::Config, values: &[f64], start: f64, interval: f64, width: f32) -> Pair {
    let mut r = renderer();
    upload_full(&mut r, values, start, interval);
    let full = r
        .register_chart(config.clone(), vec![full_series(width)])
        .expect("full chart");
    let monotonic = r
        .register_monotonic_chart(config, vec![monotonic_series(width)])
        .expect("monotonic chart");
    set(&mut r, monotonic, values, start, interval);
    Pair { r, monotonic, full }
}

impl Pair {
    /// Both charts at `scale`: the full draw, and the monotonic draw (with
    /// its source when the scale is not the screen's).
    fn images(
        &mut self,
        scale: f32,
        values: &[f64],
        start: f64,
        interval: f64,
    ) -> (RasterImage, RasterImage) {
        let full = export(&mut self.r, self.full, scale).expect("full export");
        let channels = [values];
        let source = MonotonicSlices::new(start, interval, &channels);
        let monotonic = if scale == 1.0 {
            export(&mut self.r, self.monotonic, scale).expect("monotonic export")
        } else {
            export_with(
                &mut self.r,
                self.monotonic,
                scale,
                &[MonotonicGroupRef::new("g", &source)],
            )
        };
        (full, monotonic)
    }

    /// Sets the x range of both charts: the monotonic one by its full call
    /// (`values` from 0 at interval 1), the only call that sets it.
    fn set_x_range(&mut self, min: f64, max: f64, values: &[f64]) {
        let mut config = self.r.chart_config(self.full).expect("config").clone();
        config.bottom_x.min = min;
        config.bottom_x.max = max;
        self.r.set_chart_config(self.full, config).expect("x range");
        set_range(
            &mut self.r,
            self.monotonic,
            values,
            0.0,
            1.0,
            MonotonicRange::X { min, max },
        )
        .expect("x range by the full call");
    }
}

// ---------------------------------------------------------------------------
// M2: the envelope draw against the full draw
// ---------------------------------------------------------------------------

/// A view of at most 4 samples per pixel column keeps every sample: the
/// image is the full draw's, byte for byte, at 1x and 2x.
#[test]
fn a_sparse_view_draws_exactly_the_full_line() {
    let values = signal_values(0..1_500);
    let config = bare_config(600, 300, (0.0, 1_499.0), (-1.6, 1.6));
    let mut p = pair(config, &values, 0.0, 1.0, 1.0);
    for scale in [1.0, 2.0] {
        let (full, monotonic) = p.images(scale, &values, 0.0, 1.0);
        assert_identical(&full, &monotonic, &format!("sparse at {scale}x"));
    }
}

/// A dense view (hundreds of samples per column) paints the full draw's
/// pixels within one pixel, at 1x and 2x, for a hairline and a thick line.
#[test]
fn a_dense_view_paints_within_one_pixel_of_the_full_line() {
    let values = signal_values(0..200_000);
    for width in [1.0, 3.0] {
        let config = bare_config(600, 300, (0.0, 199_999.0), (-1.6, 1.6));
        let mut p = pair(config, &values, 0.0, 1.0, width);
        for scale in [1.0, 2.0] {
            let (full, monotonic) = p.images(scale, &values, 0.0, 1.0);
            assert_within_one_pixel(&full, &monotonic, &format!("dense w{width} at {scale}x"));
        }
    }
}

/// About 4.5 samples per column: a dense view whose columns are dense and
/// sparse in turn, at 1x and 2x (9 samples per column there).
#[test]
fn a_mixed_view_paints_within_one_pixel_of_the_full_line() {
    let config = bare_config(600, 300, (0.0, 1.0), (-1.6, 1.6));
    let data_width = config.data_area().expect("area").0.width as usize;
    let n = data_width * 9 / 2;
    let interval = 1.0 / (n - 1) as f64;
    let values = signal_values(0..n);
    let mut p = pair(config, &values, 0.0, interval, 1.0);
    for scale in [1.0, 2.0] {
        let (full, monotonic) = p.images(scale, &values, 0.0, interval);
        assert_within_one_pixel(&full, &monotonic, &format!("mixed at {scale}x"));
    }
}

/// Zooming in turns a dense view sparse and zooming out turns it dense
/// again; each range change is a full call, and each image is the full
/// draw's (identical when sparse).
#[test]
fn zooming_switches_between_dense_and_sparse() {
    let values = signal_values(0..100_000);
    let config = bare_config(600, 300, (0.0, 99_999.0), (-1.6, 1.6));
    let mut p = pair(config, &values, 0.0, 1.0, 1.0);
    for (min, max, sparse) in [
        (40_000.0, 40_900.0, true),
        (0.0, 99_999.0, false),
        (10_000.0, 10_500.5, true),
    ] {
        // The document editor cannot zoom: the edit is refused.
        let mut zoomed = p.r.monotonic_config(p.monotonic).expect("config").clone();
        zoomed.bottom_x.min = min;
        zoomed.bottom_x.max = max;
        assert!(mapping_edit(edit(&mut p.r, p.monotonic, zoomed)));
        p.set_x_range(min, max, &values);
        let (full, monotonic) = p.images(1.0, &values, 0.0, 1.0);
        if sparse {
            assert_identical(&full, &monotonic, &format!("zoom {min}..{max}"));
        } else {
            assert_within_one_pixel(&full, &monotonic, &format!("zoom {min}..{max}"));
        }
    }
}

/// A stroke paints into the plot from samples outside it, so the envelope
/// keeps a fixed 64 pixel columns beyond each edge at the scale it is drawn
/// at, whatever the stroke width (a width is a style edit). The documented
/// limit: strokes up to about 120 device pixels wide draw their edges
/// exactly as the full line does; a wider one may miss ink at the plot's
/// edges. Zoomed into sparse windows, 40 and 60 px strokes at 1x and at 2x
/// (80 and 120 device pixels) are the full line's byte for byte.
#[test]
fn strokes_up_to_the_margin_limit_draw_their_edges_exactly() {
    let values = signal_values(0..100_000);
    let config = bare_config(600, 300, (0.0, 99_999.0), (-1.6, 1.6));
    for width in [40.0, 60.0] {
        let mut p = pair(config.clone(), &values, 0.0, 1.0, width);
        for (min, max) in [(40_000.0, 40_900.0), (10_000.0, 10_500.5)] {
            p.set_x_range(min, max, &values);
            for scale in [1.0, 2.0] {
                let (full, monotonic) = p.images(scale, &values, 0.0, 1.0);
                assert_identical(
                    &full,
                    &monotonic,
                    &format!("{width} px stroke, zoom {min}..{max} at {scale}x"),
                );
            }
        }
    }
}

/// Appending in pieces, the last one past the x range, gives the image of a
/// full call on all the samples, byte for byte, and the full draw's within
/// one pixel.
#[test]
fn appends_draw_as_a_full_call_would() {
    let n = 120_000;
    let values = signal_values(0..n);
    let config = bare_config(600, 300, (0.0, 99_999.0), (-1.6, 1.6));
    let mut p = pair(config.clone(), &values[..10_000], 0.0, 1.0, 1.0);
    let mut at = 10_000;
    for step in [1, 7, 3_333, 50_000, n - 10_000 - 1 - 7 - 3_333 - 50_000] {
        let piece = [&values[at..at + step]];
        let update =
            p.r.monotonic_chart(p.monotonic)
                .expect("monotonic chart")
                .append_monotonic(
                    "g",
                    &MonotonicSlices::new(0.0, 1.0, &piece),
                    MonotonicAppend { sliding: false },
                )
                .expect("append");
        at += step;
        assert_eq!(update.len, at);
    }
    // A fresh chart with one full call on every sample.
    let fresh =
        p.r.register_monotonic_chart(config, vec![named_monotonic("fresh", 1.0)])
            .expect("fresh");
    set(&mut p.r, fresh, &values, 0.0, 1.0);
    upload_full(&mut p.r, &values, 0.0, 1.0);
    let appended = export(&mut p.r, p.monotonic, 1.0).expect("appended");
    let full_call = export(&mut p.r, fresh, 1.0).expect("full call");
    let full = export(&mut p.r, p.full, 1.0).expect("full");
    assert_identical(&appended, &full_call, "appended against a full call");
    assert_within_one_pixel(&full, &appended, "appended against the full draw");
}

/// A window of 100 samples per pixel column slides by whole columns: the
/// x range moves by exactly `k × interval` in the document, the image is a
/// full call's on the moved window byte for byte, and the full draw's
/// within one pixel. A full call after the slides rebuilds from scratch.
#[test]
fn whole_column_slides_draw_as_a_full_call_would() {
    let config = bare_config(600, 300, (0.0, 1.0), (-1.6, 1.6));
    let width = config.data_area().expect("area").0.width as usize;
    let per_column = 100;
    let window = width * per_column;
    let n = window + 20 * per_column;
    let values = signal_values(0..n);
    let mut config = config;
    config.bottom_x.min = 0.0;
    config.bottom_x.max = window as f64;
    let mut p = pair(config.clone(), &values[..window], 0.0, 1.0, 1.0);
    let mut at = window;
    for columns in [1, 3, 5, 11] {
        let k = columns * per_column;
        let piece = [&values[at..at + k]];
        let update =
            p.r.monotonic_chart(p.monotonic)
                .expect("monotonic chart")
                .append_monotonic(
                    "g",
                    &MonotonicSlices::new(0.0, 1.0, &piece),
                    MonotonicAppend { sliding: true },
                )
                .expect("slide");
        at += k;
        let moved = (at - window) as f64;
        assert_eq!(update.x_range, (moved, moved + window as f64));
        assert_eq!(update.pixel_offset, 0.0);
        let stored = p.r.monotonic_config(p.monotonic).expect("config");
        assert_eq!(
            (stored.bottom_x.min, stored.bottom_x.max),
            (moved, moved + window as f64),
            "the slide writes the moved range to the document"
        );
        let mut fresh_config = config.clone();
        fresh_config.bottom_x.min = moved;
        fresh_config.bottom_x.max = moved + window as f64;
        let fresh =
            p.r.register_monotonic_chart(fresh_config, vec![named_monotonic("fresh", 1.0)])
                .expect("fresh");
        set(&mut p.r, fresh, &values[..at], 0.0, 1.0);
        let slid = export(&mut p.r, p.monotonic, 1.0).expect("slid");
        let full_call = export(&mut p.r, fresh, 1.0).expect("full call");
        assert_identical(&slid, &full_call, &format!("slid by {columns} columns"));
        p.r.monotonic_chart(fresh)
            .expect("fresh")
            .remove()
            .expect("remove fresh");
        let mut config = p.r.chart_config(p.full).expect("config").clone();
        config.bottom_x.min = moved;
        config.bottom_x.max = moved + window as f64;
        p.r.set_chart_config(p.full, config)
            .expect("the slid range");
        // The monotonic chart's document rewritten with the values the
        // slide wrote: no mapping change, so the editor takes it.
        let config = p.r.monotonic_config(p.monotonic).expect("config").clone();
        edit(&mut p.r, p.monotonic, config).expect("the slid range");
        upload_full(&mut p.r, &values[..at], 0.0, 1.0);
        let full = export(&mut p.r, p.full, 1.0).expect("full");
        assert_within_one_pixel(&full, &slid, &format!("slid by {columns} columns"));
        // The range was rewritten with the same values: still the slid
        // envelope's mapping, so it draws without the source.
        assert_identical(
            &export(&mut p.r, p.monotonic, 1.0).expect("unchanged"),
            &slid,
            "an identical range edit",
        );
        set(&mut p.r, p.monotonic, &values[..at], 0.0, 1.0);
        assert_identical(
            &export(&mut p.r, p.monotonic, 1.0).expect("rebuilt"),
            &full_call,
            "a full call after the slides",
        );
    }
}

/// Slides by parts of a column keep the columns anchored and offset the
/// window by a fraction of a pixel: the image stays within one pixel of the
/// full draw of the same window.
#[test]
fn fractional_slides_stay_within_a_sub_pixel_offset() {
    let config = bare_config(600, 300, (0.0, 1.0), (-1.6, 1.6));
    let width = config.data_area().expect("area").0.width as usize;
    let window = width * 37;
    let n = window + 30_000;
    let values = signal_values(0..n);
    let mut config = config;
    config.bottom_x.min = 0.0;
    config.bottom_x.max = window as f64;
    let mut p = pair(config, &values[..window], 0.0, 1.0, 1.0);
    let mut at = window;
    for k in [1, 13, 1_000, 25_000] {
        let piece = [&values[at..at + k]];
        let update =
            p.r.monotonic_chart(p.monotonic)
                .expect("monotonic chart")
                .append_monotonic(
                    "g",
                    &MonotonicSlices::new(0.0, 1.0, &piece),
                    MonotonicAppend { sliding: true },
                )
                .expect("slide");
        at += k;
        let moved = (at - window) as f64;
        assert_eq!(update.x_range, (moved, moved + window as f64));
        assert!((0.0..1.0).contains(&update.pixel_offset));
        let mut config = p.r.chart_config(p.full).expect("config").clone();
        config.bottom_x.min = moved;
        config.bottom_x.max = moved + window as f64;
        p.r.set_chart_config(p.full, config).expect("full range");
        upload_full(&mut p.r, &values[..at], 0.0, 1.0);
        let full = export(&mut p.r, p.full, 1.0).expect("full");
        let slid = export(&mut p.r, p.monotonic, 1.0).expect("slid");
        assert_within_one_pixel(&full, &slid, &format!("slid by {k} samples"));
    }
}

/// A NaN gap narrower than one pixel column is drawn joined: the image is
/// the full draw's within one pixel and the line crosses the gap's column.
/// A gap several columns wide breaks the line in both draws.
#[test]
fn sub_pixel_gaps_draw_joined_and_wider_gaps_break() {
    let n = 100_000;
    let mut values = signal_values(0..n);
    // A one-sample gap inside a column, and a gap of 2 000 samples (about
    // 9 columns).
    values[30_017] = f64::NAN;
    for value in &mut values[60_000..62_000] {
        *value = f64::NAN;
    }
    let config = bare_config(600, 300, (0.0, (n - 1) as f64), (-1.6, 1.6));
    let mut p = pair(config.clone(), &values, 0.0, 1.0, 1.0);
    for scale in [1.0, 2.0] {
        let (full, monotonic) = p.images(scale, &values, 0.0, 1.0);
        assert_within_one_pixel(&full, &monotonic, &format!("gaps at {scale}x"));
    }
    let area = config.data_area().expect("area").0;
    let column_of = |i: usize| area.x + (i as f64 / (n - 1) as f64 * f64::from(area.width)) as u32;
    let monotonic = export(&mut p.r, p.monotonic, 1.0).expect("export");
    let full = export(&mut p.r, p.full, 1.0).expect("export");
    // Inside the frame the axes draw around the data area.
    let inked = |img: &RasterImage, x: u32| {
        (area.y + 2..area.y + area.height - 2).any(|y| painted(img, x, y))
    };
    assert!(
        inked(&monotonic, column_of(30_017)),
        "a sub-pixel gap is drawn joined"
    );
    for img in [&monotonic, &full] {
        assert!(
            !inked(img, column_of(61_000)),
            "a gap several columns wide breaks the line"
        );
    }
}

/// Every element type draws as its `f64` conversion, from per-channel
/// slices and from an interleaved buffer whose other columns are never
/// drawn and whose trailing partial frame is not counted.
#[test]
fn every_element_type_draws_as_its_f64_conversion() {
    let n = 40_000;
    let base = signal_values(0..n);
    let config = bare_config(600, 300, (0.0, (n - 1) as f64), (-150.0, 300.0));
    let mut r = renderer();
    let id = r
        .register_monotonic_chart(config, vec![monotonic_series(1.0)])
        .expect("chart");
    macro_rules! typed {
        ($t:ty, $f:expr) => {{
            let typed: Vec<$t> = base.iter().map($f).collect();
            let as_f64: Vec<f64> = typed.iter().map(|&v| v as f64).collect();
            set(&mut r, id, &as_f64, 0.0, 1.0);
            let reference = export(&mut r, id, 1.0).expect("reference");
            let channels = [typed.as_slice()];
            let source = MonotonicSlices::new(0.0, 1.0, &channels);
            let range = x_range(&r, id);
            r.monotonic_chart(id)
                .expect("monotonic chart")
                .set_monotonic("g", &source, range)
                .expect("slices");
            let sliced = export(&mut r, id, 1.0).expect("slices");
            assert_identical(&reference, &sliced, stringify!($t));
            // Frames of three values: the drawn column between two the
            // series never names.
            let mut frames: Vec<$t> = Vec::with_capacity(n * 3 + 2);
            for &v in &typed {
                frames.extend([v, v, v]);
            }
            frames.extend([typed[0], typed[0]]);
            let columns = [1usize];
            let source = MonotonicInterleaved::new(0.0, 1.0, &frames, 3, &columns).expect("source");
            assert_eq!(source.len(), n);
            let range = x_range(&r, id);
            r.monotonic_chart(id)
                .expect("monotonic chart")
                .set_monotonic("g", &source, range)
                .expect("interleaved");
            let interleaved = export(&mut r, id, 1.0).expect("interleaved");
            assert_identical(
                &reference,
                &interleaved,
                concat!(stringify!($t), " interleaved"),
            );
        }};
    }
    typed!(f64, |&v| v * 100.0);
    typed!(f32, |&v| (v * 100.0) as f32);
    typed!(i32, |&v| (v * 100.0) as i32);
    typed!(i16, |&v| (v * 100.0) as i16);
    typed!(u16, |&v| (v * 100.0 + 150.0) as u16);
    typed!(u8, |&v| (v * 60.0 + 120.0) as u8);
}

// ---------------------------------------------------------------------------
// The x-to-pixel mapping: what needs the source and what does not
// ---------------------------------------------------------------------------

/// One monotonic chart over `values` (dense: 100 000 samples) on its own
/// renderer.
/// The x range chart `id`'s document has, as the full call keeps it.
fn x_range(r: &Renderer, id: MonotonicChartId) -> MonotonicRange {
    let (min, max) = r
        .monotonic_config(id)
        .expect("config")
        .bottom_x
        .data_range();
    MonotonicRange::X { min, max }
}

fn single(values: &[f64]) -> (Renderer, MonotonicChartId) {
    let mut r = renderer();
    let config = bare_config(600, 300, (0.0, (values.len() - 1) as f64), (-1.6, 1.6));
    let id = r
        .register_monotonic_chart(config, vec![monotonic_series(1.0)])
        .expect("chart");
    set(&mut r, id, values, 0.0, 1.0);
    (r, id)
}

/// The image a fresh renderer draws for `config` and the series after one
/// full call on `values`.
fn fresh_image(config: &renderer::Config, series: &[SeriesConfig], values: &[f64]) -> RasterImage {
    let mut r = renderer();
    let id = r
        .register_monotonic_chart(config.clone(), series.to_vec())
        .expect("fresh chart");
    set(&mut r, id, values, 0.0, 1.0);
    export(&mut r, id, 1.0).expect("fresh export")
}

fn needs(r: &Renderer, id: MonotonicChartId) -> Vec<(String, MappingChange)> {
    r.monotonic_needs_source(id)
        .expect("needs_source")
        .into_iter()
        .map(|need| {
            assert_eq!(need.group, "g");
            (need.series_id, need.change)
        })
        .collect()
}

/// `result` is the editor's refusal of a mapping edit.
fn mapping_edit<T: std::fmt::Debug>(result: Result<T, FiggyError>) -> bool {
    matches!(
        result,
        Err(FiggyError::InvalidConfig {
            kind: renderer::RuleKind::MonotonicMappingEdit,
            ..
        })
    )
}

fn requires(result: Result<RasterImage, FiggyError>, change: MappingChange) -> bool {
    matches!(
        result,
        Err(FiggyError::MonotonicSourceRequired { change: c, ref series_id }) if c == change && series_id == "m"
    )
}

/// The document, display and image a refused call must leave as they were.
struct Before {
    config: renderer::Config,
    series: Vec<SeriesConfig>,
    image: RasterImage,
}

impl Before {
    fn of(r: &mut Renderer, id: MonotonicChartId) -> Self {
        Self {
            config: r.monotonic_config(id).expect("config").clone(),
            series: r.monotonic_series(id).expect("series").to_vec(),
            image: export(r, id, 1.0).expect("image"),
        }
    }

    /// The document, the series, the envelopes and the image are as they
    /// were.
    fn unchanged(&self, r: &mut Renderer, id: MonotonicChartId, what: &str) {
        assert_eq!(
            r.monotonic_config(id).expect("config"),
            &self.config,
            "{what}: document"
        );
        assert_eq!(
            r.monotonic_series(id).expect("series"),
            self.series.as_slice(),
            "{what}: series"
        );
        assert!(needs(r, id).is_empty(), "{what}: envelopes kept");
        assert_identical(&export(r, id, 1.0).expect(what), &self.image, what);
    }

    /// A call that would move the mapping without the source was refused,
    /// naming the series and the change, and changed nothing.
    fn refused(
        &self,
        r: &mut Renderer,
        id: MonotonicChartId,
        result: Result<(), FiggyError>,
        change: MappingChange,
        what: &str,
    ) {
        assert!(
            matches!(
                &result,
                Err(FiggyError::MonotonicSourceRequired { series_id, change: c })
                    if series_id == "m" && *c == change
            ),
            "{what}: {result:?}"
        );
        self.unchanged(r, id, what);
    }
}

/// A pick at the middle of chart `id`'s data area, as its display shows it.
fn pick_middle(r: &mut Renderer, id: MonotonicChartId) -> Result<(), FiggyError> {
    r.enable_gpu_picking()?;
    let config = r.monotonic_config(id)?.clone();
    let area = config.data_area().expect("area").0;
    let request = renderer::GpuPickRequest {
        canvas_position_px: [
            (area.x + area.width / 2) as f32,
            (area.y + area.height / 2) as f32,
        ],
        display_panel_px: config.chart_area.0,
        display_scale: 1.0,
        max_distance_px: 4.0,
    };
    r.monotonic_chart(id)?.pick_data(request).map(|_| ())
}

/// A monotonic chart is reached only through its handle (#271): the
/// general calls take a `ChartId`, which a monotonic chart never hands out,
/// so none can move its mapping. Its document editor (`MonotonicConfig`)
/// has no setter for the mapping, and every edit that would move it (the x
/// range, scale, breaks or direction, the chart area, a layout or plot edge)
/// is refused with `RuleKind::MonotonicMappingEdit`, leaving the document,
/// the envelope and the image as they were. A monotonic line series lives
/// only in a monotonic chart: the general registration and series calls
/// refuse it. Before its first full call a series has no data: it draws
/// nothing and `needs_source` names it.
#[test]
fn only_the_monotonic_calls_move_the_mapping() {
    use renderer::layout::{Element, NudgeResult, Side};
    let values = signal_values(0..100_000);
    let mut r = renderer();
    let config = bare_config(600, 300, (0.0, 99_999.0), (-1.6, 1.6));
    let id = r
        .register_monotonic_chart(config.clone(), vec![monotonic_series(1.0)])
        .expect("chart");
    assert_eq!(needs(&r, id), [("m".to_string(), MappingChange::NotBuilt)]);
    let empty = export(&mut r, id, 1.0).expect("nothing to draw");
    set(&mut r, id, &values, 0.0, 1.0);
    assert!(needs(&r, id).is_empty());
    assert!(differing(&empty, &export(&mut r, id, 1.0).expect("built")) > 0);

    type Edit = fn(&mut renderer::Config);
    let edits: [(&str, Edit); 7] = [
        ("x range", |c| c.bottom_x.max = 50_000.0),
        ("chart area", |c| c.chart_area.0.width = 500),
        ("layout", |c| c.left_y.out_margin += 40.0),
        ("inversion", |c| c.bottom_x.inverted = true),
        ("x scale", |c| {
            c.bottom_x.scale = renderer::config::AxisScale::Logarithmic;
            c.bottom_x.min = 1.0;
        }),
        ("breaks", |c| {
            c.bottom_x.breaks = Some(renderer::config::AxisBreaks {
                ranges: vec![renderer::config::AxisBreak::new(40_000.0, 50_000.0)],
                ..Default::default()
            })
        }),
        ("plot edge", |c| c.right_y.out_margin += 30.0),
    ];
    let before = Before::of(&mut r, id);
    for (what, change) in edits {
        let mut next = r.monotonic_config(id).expect("config").clone();
        change(&mut next);
        assert!(mapping_edit(edit(&mut r, id, next)), "{what}");
        before.unchanged(&mut r, id, what);
        pick_middle(&mut r, id).expect("a pick never needs the source");
    }
    // The typed editor has no x range; a drag of a plot edge that changes
    // the plot's width is rejected.
    let measure = renderer::CpuTextMeasure::for_style(&before.config.draw_style);
    let refused = r
        .monotonic_chart(id)
        .expect("chart")
        .edit_config(|editor| {
            (
                editor.axis(Side::Bottom).set_range(0.0, 10.0),
                editor.nudge(Element::DataAreaEdge(Side::Right), &measure, -30.0, 0.0),
                editor.nudge(Element::DataAreaEdge(Side::Left), &measure, 20.0, 0.0),
                editor.axis(Side::Left).set_range(-3.0, 3.0),
            )
        })
        .expect("edit");
    assert_eq!(refused.0, Err(renderer::RuleKind::MonotonicMappingEdit));
    assert!(matches!(refused.1, NudgeResult::Rejected(_)));
    assert!(matches!(refused.2, NudgeResult::Rejected(_)));
    assert_eq!(refused.3, Ok(()), "a y range is not mapping");
    let edited = r.monotonic_config(id).expect("config");
    assert_eq!((edited.left_y.min, edited.left_y.max), (-3.0, 3.0));
    assert_eq!(
        edited.data_area().expect("area"),
        before.config.data_area().expect("area")
    );

    // The general calls refuse a monotonic line series.
    let general = |result: Result<(), FiggyError>| {
        matches!(
            result,
            Err(FiggyError::InvalidSeriesConfig {
                kind: renderer::RuleKind::MonotonicGeneralChart,
                ..
            })
        )
    };
    assert!(general(
        r.register_chart(config.clone(), vec![monotonic_series(1.0)])
            .map(|_| ())
    ));
    upload_full(&mut r, &values, 0.0, 1.0);
    let plain = r
        .register_chart(config.clone(), vec![full_series(1.0)])
        .expect("general chart");
    assert!(general(
        r.set_chart_series(plain, vec![monotonic_series(1.0)])
    ));
    assert!(general(r.set_chart_state(
        plain,
        config.clone(),
        vec![monotonic_series(1.0)]
    )));

    // A logarithmic or broken x axis is refused at registration.
    let mut log = config.clone();
    log.bottom_x.scale = renderer::config::AxisScale::Logarithmic;
    log.bottom_x.min = 1.0;
    assert!(matches!(
        r.register_monotonic_chart(log, vec![named_monotonic("l", 1.0)]),
        Err(FiggyError::InvalidSeriesConfig {
            kind: renderer::RuleKind::MonotonicXAxis,
            ..
        })
    ));
}

/// `set_monotonic` is the only call that sets a monotonic series' x range:
/// the range it takes is written to the document and the image is what a
/// fresh renderer draws at that range. A range that is not finite and
/// increasing is refused and changes nothing.
#[test]
fn set_monotonic_sets_the_x_range() {
    let values = signal_values(0..100_000);
    let (mut r, id) = single(&values);
    for (min, max) in [(20_000.0, 50_000.0), (0.0, 99_999.0), (-500.0, 120_000.0)] {
        set_range(
            &mut r,
            id,
            &values,
            0.0,
            1.0,
            MonotonicRange::X { min, max },
        )
        .expect("range");
        let config = r.monotonic_config(id).expect("config").clone();
        assert_eq!((config.bottom_x.min, config.bottom_x.max), (min, max));
        assert!(needs(&r, id).is_empty());
        assert_identical(
            &export(&mut r, id, 1.0).expect("ranged"),
            &fresh_image(&config, &[monotonic_series(1.0)], &values),
            &format!("{min}..{max}"),
        );
    }
    let before = Before::of(&mut r, id);
    for (min, max) in [
        (5.0, 5.0),
        (9.0, 1.0),
        (f64::NAN, 1.0),
        (0.0, f64::INFINITY),
    ] {
        let refused = set_range(
            &mut r,
            id,
            &values,
            0.0,
            1.0,
            MonotonicRange::X { min, max },
        );
        assert!(
            matches!(
                refused,
                Err(FiggyError::InvalidConfig {
                    kind: renderer::RuleKind::MonotonicRange,
                    ..
                })
            ),
            "{min}..{max}: {refused:?}"
        );
    }
    assert_eq!(r.monotonic_config(id).expect("config"), &before.config);
    assert_identical(
        &export(&mut r, id, 1.0).expect("kept"),
        &before.image,
        "kept",
    );
}

/// `resize_monotonic` is the resize call of a chart with monotonic series:
/// a new chart area or display surface, each group whose plot it moves to
/// other device pixels reduced again from its source in the same call. The
/// image is what a fresh renderer draws after one full call. Without a
/// source a moved group needs it is refused and changes nothing; a resize
/// that moves nothing needs none.
#[test]
fn resize_monotonic_reduces_the_moved_groups_again() {
    let values = signal_values(0..100_000);
    let (mut r, id) = single(&values);
    let area = r.monotonic_config(id).expect("config").chart_area.0;
    let narrower = Rect { width: 500, ..area };
    let before = Before::of(&mut r, id);
    let refused = r
        .monotonic_chart(id)
        .expect("monotonic chart")
        .resize(narrower, None, None, &[]);
    before.refused(&mut r, id, refused, MappingChange::PlotWidth, "no source");
    lent(&values, |groups| {
        r.monotonic_chart(id)
            .expect("monotonic chart")
            .resize(narrower, None, None, groups)
    })
    .expect("chart area");
    let config = r.monotonic_config(id).expect("config").clone();
    assert_eq!(config.chart_area.0, narrower);
    assert!(needs(&r, id).is_empty());
    pick_middle(&mut r, id).expect("pick");
    assert_identical(
        &export(&mut r, id, 1.0).expect("resized"),
        &fresh_image(&config, &[monotonic_series(1.0)], &values),
        "chart area",
    );
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .resize(narrower, None, None, &[])
        .expect("an unchanged size moves nothing");

    // A display at another size and scale (a DPI change) maps the plot to
    // other device pixels; so does removing it.
    lent(&values, |groups| {
        r.monotonic_chart(id).expect("monotonic chart").resize(
            narrower,
            None,
            Some((1000, 600)),
            groups,
        )
    })
    .expect("display");
    assert!(needs(&r, id).is_empty(), "display");
    assert!(matches!(
        r.monotonic_chart(id)
            .expect("monotonic chart")
            .resize(narrower, None, None, &[]),
        Err(FiggyError::MonotonicSourceRequired {
            change: MappingChange::PlotWidth,
            ..
        })
    ));
    lent(&values, |groups| {
        r.monotonic_chart(id)
            .expect("monotonic chart")
            .resize(narrower, None, None, groups)
    })
    .expect("no display");
    assert_identical(
        &export(&mut r, id, 1.0).expect("no display"),
        &fresh_image(&config, &[monotonic_series(1.0)], &values),
        "no display",
    );
}

/// `fit_layout` fits a registered chart's layout to its labels and titles
/// (`layout::auto_layout`) and applies it. On a chart with monotonic series
/// it can move the plot's edges, so it takes the source: without it such a
/// fit is refused and changes nothing; with it the image is what a fresh
/// renderer draws at the fitted layout; a fit that moves nothing (the
/// layout fitted already) needs none.
#[test]
fn fit_layout_takes_the_source_when_it_moves_the_plot() {
    let values = signal_values(0..100_000);
    let mut r = renderer();
    let mut config = bare_config(600, 300, (0.0, 99_999.0), (-1.6, 1.6));
    // A band too narrow for its labels: the fit grows it.
    config.left_y.out_margin = 2.0;
    let id = r
        .register_monotonic_chart(config.clone(), vec![monotonic_series(1.0)])
        .expect("chart");
    set(&mut r, id, &values, 0.0, 1.0);
    let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
    let laid_out = renderer::layout::auto_layout(&config, &measure);
    assert_ne!(
        laid_out.data_area().expect("area").0.width,
        config.data_area().expect("area").0.width,
        "the fit moves the plot's edges"
    );
    let before = Before::of(&mut r, id);
    let refused = r
        .monotonic_chart(id)
        .expect("monotonic chart")
        .fit_layout(&[]);
    before.refused(&mut r, id, refused, MappingChange::PlotWidth, "no source");
    lent(&values, |groups| {
        r.monotonic_chart(id)
            .expect("monotonic chart")
            .fit_layout(groups)
    })
    .expect("fit");
    assert_eq!(r.monotonic_config(id).expect("config"), &laid_out);
    assert!(needs(&r, id).is_empty());
    assert_identical(
        &export(&mut r, id, 1.0).expect("fitted"),
        &fresh_image(&laid_out, &[monotonic_series(1.0)], &values),
        "fitted",
    );
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .fit_layout(&[])
        .expect("a fitted layout moves nothing");
}

/// Draws and picks never need the source (#271): through a sweep of
/// general calls refused or applied, the four calls that take the source,
/// a series before its first full call, group and channel edits, series
/// added and removed, a sliding append and a chart cleared of its series,
/// every frame draws and every pick resolves.
#[test]
fn draws_and_picks_never_need_the_source() {
    let values = signal_values(0..50_000);
    let two = [values.as_slice(), values.as_slice()];
    let source = MonotonicSlices::new(0.0, 1.0, &two);
    let groups = [MonotonicGroupRef::new("g", &source)];
    let mut r = renderer();
    let config = bare_config(600, 300, (0.0, 49_999.0), (-1.6, 1.6));
    let id = r
        .register_monotonic_chart(config.clone(), vec![monotonic_series(1.0)])
        .expect("chart");
    let check = |r: &mut Renderer, what: &str| {
        export(r, id, 1.0).unwrap_or_else(|e| panic!("{what}: draw {e}"));
        pick_middle(r, id).unwrap_or_else(|e| panic!("{what}: pick {e}"));
    };
    check(&mut r, "before any data");
    // General calls, applied or refused: the chart draws after each.
    let zoomed = {
        let mut c = config.clone();
        c.bottom_x.max = 9_999.0;
        c
    };
    let _ = edit(&mut r, id, zoomed.clone());
    check(&mut r, "a zoom before any data");
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_monotonic(
            "g",
            &source,
            MonotonicRange::X {
                min: 0.0,
                max: 49_999.0,
            },
        )
        .expect("data");
    check(&mut r, "data");
    let _ = edit(&mut r, id, zoomed);
    check(&mut r, "a refused zoom");
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_monotonic(
            "g",
            &source,
            MonotonicRange::X {
                min: 0.0,
                max: 9_999.0,
            },
        )
        .expect("zoom by the full call");
    check(&mut r, "zoomed");
    let mut y = r.monotonic_config(id).expect("config").clone();
    y.left_y.max = 3.0;
    edit(&mut r, id, y).expect("y range");
    check(&mut r, "y range");
    let series_steps: Vec<(&str, Vec<SeriesConfig>)> = vec![
        (
            "channel",
            vec![SeriesConfig::monotonic_line("m", "g", 1).color(BLUE)],
        ),
        (
            "added",
            vec![
                SeriesConfig::monotonic_line("m", "g", 1).color(BLUE),
                SeriesConfig::monotonic_line("n", "g", 0),
            ],
        ),
        (
            "other group",
            vec![
                SeriesConfig::monotonic_line("m", "g", 1).color(BLUE),
                SeriesConfig::monotonic_line("n", "h", 0),
            ],
        ),
        ("removed", Vec::new()),
        ("back", vec![monotonic_series(1.0)]),
    ];
    for (what, series) in series_steps {
        r.monotonic_chart(id)
            .expect("monotonic chart")
            .set_series(series)
            .expect(what);
        check(&mut r, what);
    }
    let area = {
        let mut area = config.chart_area.0;
        area.width = 480;
        area
    };
    let _ = r
        .monotonic_chart(id)
        .expect("monotonic chart")
        .resize(area, None, None, &[]);
    check(&mut r, "a refused resize");
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .resize(area, None, None, &groups)
        .expect("resize");
    check(&mut r, "resized");
    let _ = r
        .monotonic_chart(id)
        .expect("monotonic chart")
        .fit_layout(&[]);
    check(&mut r, "a fit without the source");
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .fit_layout(&groups)
        .expect("fit");
    check(&mut r, "fitted");
    // A sliding window ends at the data's end.
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_monotonic(
            "g",
            &source,
            MonotonicRange::X {
                min: 0.0,
                max: 49_999.0,
            },
        )
        .expect("the whole range");
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .append_monotonic(
            "g",
            &MonotonicSlices::new(0.0, 1.0, &[&values[..100], &values[..100]]),
            MonotonicAppend { sliding: true },
        )
        .expect("slide");
    check(&mut r, "slid");
}

/// Every setting other than the four calls' works as on any chart, with no
/// source (#271): the legend, axis titles and their units, ticks, tick
/// labels and their format, the title, an annotation, a data selection and
/// decoration drags (title, legend, axis title, tick labels), and the
/// line's colour, width and style. None moves the plot area, so each draws
/// from the envelope the last full call built, as a fresh renderer draws
/// after one full call, byte for byte (the SSOT test checks that no
/// envelope column is written).
#[test]
fn every_other_setting_draws_without_the_source() {
    use renderer::Draggable as _;
    use renderer::layout::Side;
    let values = signal_values(0..100_000);
    let (mut r, id) = single(&values);
    let measure =
        renderer::CpuTextMeasure::for_style(&r.monotonic_config(id).expect("c").draw_style);
    type Edit = fn(&mut renderer::Config, &renderer::CpuTextMeasure);
    let edits: [(&str, Edit); 13] = [
        ("y range", |c, _| c.left_y.max = 3.0),
        ("legend", |c, _| {
            c.legend.visible = true;
            c.legend.content = renderer::text::RichText::plain("signal", Color::BLACK, 12.0, "");
        }),
        ("x title", |c, _| {
            c.bottom_x.title_option.text =
                renderer::text::RichText::plain("time (s)", Color::BLACK, 12.0, "");
            c.bottom_x.title_option.visible = true;
        }),
        ("y title and unit", |c, _| {
            c.left_y.title_option.text =
                renderer::text::RichText::plain("voltage (mV)", Color::BLACK, 12.0, "");
            c.left_y.title_option.visible = true;
        }),
        ("ticks", |c, _| {
            c.bottom_x.major_spacing = 10_000.0;
            c.bottom_x.minor_count = 4;
            c.left_y.major_spacing = 0.25;
        }),
        ("tick labels", |c, _| {
            c.bottom_x.label_style.font_size += 2.0;
            c.left_y.label_style.significant_digits = 2;
            c.left_y.label_style.color = Color::from_rgb8(120, 0, 0);
        }),
        ("title", |c, _| {
            c.chart_title.text =
                renderer::text::RichText::plain("monotonic", Color::BLACK, 14.0, "");
            c.chart_title.visible = true;
        }),
        ("annotation", |c, _| {
            c.annotations
                .items
                .push(renderer::Annotation::new(renderer::AnnotationKind::Text {
                    anchor: renderer::Anchor::Data {
                        x: 50_000.0,
                        y: 0.0,
                    },
                    text: renderer::text::RichText::plain("note", Color::BLACK, 12.0, ""),
                    box_style: None,
                    align: Default::default(),
                }));
        }),
        ("selection", |c, _| {
            c.picked_data = Some(renderer::DataSelectionsConfig {
                visible: true,
                refs: vec![renderer::PickedDataRef::Point {
                    series_id: "m".into(),
                    source_id: None,
                    point_index: 0,
                }],
                ..renderer::DataSelectionsConfig::empty()
            });
        }),
        ("title drag", |c, m| {
            let moved = renderer::select::ChartTitleElement.drag_by(c, m, 12.0, 4.0);
            assert_eq!(moved, renderer::layout::NudgeResult::Moved);
        }),
        ("legend drag", |c, m| {
            let moved = renderer::select::LegendElement.drag_by(c, m, -20.0, 10.0);
            assert_eq!(moved, renderer::layout::NudgeResult::Moved);
        }),
        ("axis title drag", |c, m| {
            let element = renderer::select::AxisTitleElement { side: Side::Bottom };
            assert_eq!(
                element.drag_by(c, m, 0.0, 3.0),
                renderer::layout::NudgeResult::Moved
            );
        }),
        ("tick label drag", |c, m| {
            let element = renderer::select::AxisLabelElement { side: Side::Left };
            assert_eq!(
                element.drag_by(c, m, -3.0, 0.0),
                renderer::layout::NudgeResult::Moved
            );
        }),
    ];
    for (what, change) in edits {
        let mut next = r.monotonic_config(id).expect("config").clone();
        let area = next.data_area().expect("area").0;
        change(&mut next, &measure);
        assert_eq!(
            next.data_area().expect("area").0,
            area,
            "{what} keeps the plot area"
        );
        edit(&mut r, id, next.clone()).expect(what);
        assert!(needs(&r, id).is_empty(), "{what} needs no source");
        let drawn = export(&mut r, id, 1.0).expect(what);
        assert_identical(
            &drawn,
            &fresh_image(&next, &[monotonic_series(1.0)], &values),
            what,
        );
    }
    // The line's colour, width and style are the series' style.
    for (what, series) in [
        (
            "colour and width",
            SeriesConfig::monotonic_line("m", "g", 0)
                .color(Color::from_rgb8(200, 30, 30))
                .width(2.5),
        ),
        (
            "wide",
            SeriesConfig::monotonic_line("m", "g", 0)
                .color(Color::from_rgb8(200, 30, 30))
                .width(40.0),
        ),
        ("dash", {
            let mut series = SeriesConfig::monotonic_line("m", "g", 0).width(2.0);
            let renderer::DataRenderType::MonotonicLine(monotonic) = &mut series.render_type else {
                unreachable!()
            };
            monotonic.line.line_style = renderer::line::LineStylePreset::Dash;
            series
        }),
    ] {
        r.monotonic_chart(id)
            .expect("monotonic chart")
            .set_series(vec![series.clone()])
            .expect(what);
        assert!(needs(&r, id).is_empty(), "{what} needs no source");
        let config = r.monotonic_config(id).expect("config").clone();
        assert_identical(
            &export(&mut r, id, 1.0).expect(what),
            &fresh_image(&config, &[series], &values),
            what,
        );
    }
}

/// Autoscale is a flag of the full call (#271): the x range is fitted to
/// `start ..= start + (len - 1) × interval` by arithmetic, and the y range,
/// padded by 5 % on each side, to the lowest and highest value of the
/// envelope the same call reduces, which keeps every column's extremes, so
/// the data's own. The image is what a plain full call draws at those
/// ranges. On an overlay, the overlay's own y axis is fitted. The general
/// auto-fit refuses a chart with monotonic series.
#[test]
fn autoscale_fits_x_to_the_grid_and_y_to_the_reduction() {
    let values = signal_values(0..50_000);
    let (lo, hi) = values
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    let pad = (hi - lo) * 0.05;
    let mut r = renderer();
    let config = bare_config(600, 300, (10.0, 20.0), (-9.0, 9.0));
    let id = r
        .register_monotonic_chart(config, vec![monotonic_series(1.0)])
        .expect("chart");
    let channels = [values.as_slice()];
    let source = MonotonicSlices::new(2.0, 0.5, &channels);
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_monotonic("g", &source, MonotonicRange::Autoscale)
        .expect("autoscale");
    let fitted = r.monotonic_config(id).expect("config").clone();
    assert_eq!(
        (fitted.bottom_x.min, fitted.bottom_x.max),
        (2.0, 2.0 + 49_999.0 * 0.5)
    );
    assert!(
        (fitted.left_y.min - (lo - pad)).abs() < 1e-12,
        "{:?}",
        fitted.left_y.min
    );
    assert!(
        (fitted.left_y.max - (hi + pad)).abs() < 1e-12,
        "{:?}",
        fitted.left_y.max
    );
    assert!(needs(&r, id).is_empty());
    let mut fresh = renderer();
    let plain = fresh
        .register_monotonic_chart(fitted.clone(), vec![monotonic_series(1.0)])
        .expect("fresh");
    let range = x_range(&fresh, plain);
    fresh
        .monotonic_chart(plain)
        .expect("monotonic chart")
        .set_monotonic("g", &source, range)
        .expect("plain");
    assert_identical(
        &export(&mut r, id, 1.0).expect("autoscaled"),
        &export(&mut fresh, plain, 1.0).expect("plain"),
        "autoscale draws as a plain full call at the fitted ranges",
    );

    // On an overlay, its own y axis.
    let mut layered = bare_config(600, 300, (0.0, 1.0), (-9.0, 9.0));
    layered.overlays.push(renderer::OverlayLayer::default());
    layered.overlays[0].series = vec!["o".to_string()];
    layered.overlays[0].y_axis.min = -100.0;
    layered.overlays[0].y_axis.max = 100.0;
    let id = r
        .register_monotonic_chart(layered, vec![named_monotonic("o", 1.0)])
        .expect("overlay chart");
    let from_zero = MonotonicSlices::new(0.0, 1.0, &channels);
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_monotonic("g", &from_zero, MonotonicRange::Autoscale)
        .expect("overlay autoscale");
    let fitted = r.monotonic_config(id).expect("config");
    assert_eq!((fitted.bottom_x.min, fitted.bottom_x.max), (0.0, 49_999.0));
    let axis = &fitted.overlays[0].y_axis;
    assert!((axis.min - (lo - pad)).abs() < 1e-12 && (axis.max - (hi + pad)).abs() < 1e-12);
    assert_eq!(
        (fitted.left_y.min, fitted.left_y.max),
        (-9.0, 9.0),
        "the base axis stays"
    );
}

/// Autoscale with several channels fits each y axis once, to the union of
/// the channels drawn on it: two channels on the base axis share one range
/// that holds both, whichever comes last, and a channel on an overlay fits
/// only the overlay's axis.
#[test]
fn autoscale_fits_each_y_axis_to_every_channel_on_it() {
    let low: Vec<f64> = (0..20_000).map(|i| -50.0 + (i % 7) as f64).collect();
    let high: Vec<f64> = (0..20_000).map(|i| 100.0 + (i % 11) as f64).collect();
    let mid: Vec<f64> = (0..20_000).map(|i| 3.0 + (i % 5) as f64).collect();
    let channels = [low.as_slice(), high.as_slice(), mid.as_slice()];
    let source = MonotonicSlices::new(0.0, 1.0, &channels);
    let padded = |lo: f64, hi: f64| {
        let pad = (hi - lo) * 0.05;
        (lo - pad, hi + pad)
    };
    let close = |got: (f64, f64), want: (f64, f64)| {
        (got.0 - want.0).abs() < 1e-9 && (got.1 - want.1).abs() < 1e-9
    };

    // Both orders: the last channel must not decide the range.
    for (first, second) in [(0, 1), (1, 0)] {
        let mut r = renderer();
        let config = bare_config(600, 300, (0.0, 1.0), (-1.0, 1.0));
        let series = vec![
            SeriesConfig::monotonic_line("a", "g", first).color(BLUE),
            SeriesConfig::monotonic_line("b", "g", second).color(BLUE),
        ];
        let id = r.register_monotonic_chart(config, series).expect("chart");
        r.monotonic_chart(id)
            .expect("monotonic chart")
            .set_monotonic("g", &source, MonotonicRange::Autoscale)
            .expect("autoscale");
        let fitted = r.monotonic_config(id).expect("config");
        let got = (fitted.left_y.min, fitted.left_y.max);
        assert!(
            close(got, padded(-50.0, 110.0)),
            "order {first},{second}: {got:?}"
        );
    }

    // A base channel pair and an overlay channel: two axes, two fits.
    let mut r = renderer();
    let mut layered = bare_config(600, 300, (0.0, 1.0), (-1.0, 1.0));
    layered.overlays.push(renderer::OverlayLayer::default());
    layered.overlays[0].series = vec!["c".to_string()];
    let series = vec![
        SeriesConfig::monotonic_line("a", "g", 0).color(BLUE),
        SeriesConfig::monotonic_line("b", "g", 1).color(BLUE),
        SeriesConfig::monotonic_line("c", "g", 2).color(BLUE),
    ];
    let id = r.register_monotonic_chart(layered, series).expect("chart");
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_monotonic("g", &source, MonotonicRange::Autoscale)
        .expect("autoscale");
    let fitted = r.monotonic_config(id).expect("config");
    let base = (fitted.left_y.min, fitted.left_y.max);
    assert!(close(base, padded(-50.0, 110.0)), "base {base:?}");
    let overlay = (fitted.overlays[0].y_axis.min, fitted.overlays[0].y_axis.max);
    assert!(close(overlay, padded(3.0, 7.0)), "overlay {overlay:?}");
}

/// The plot area dragged keeps its width and draws without the source. A
/// plot-area resize (the data area's handles) changes its width: it is a
/// resize, the plot rect passed to `resize_monotonic` with the sources, and
/// draws what a fresh renderer draws after one full call at the resized
/// layout. Without the sources it is refused and changes nothing, through
/// the document editor or the resize; a plot rect the margins cannot place
/// is refused too.
#[test]
fn a_plot_area_drag_keeps_the_envelope_and_a_resize_takes_the_source() {
    use renderer::{Draggable as _, Resizable as _};
    let values = signal_values(0..100_000);
    let (mut r, id) = single(&values);
    let mut config = r.monotonic_config(id).expect("config").clone();
    let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
    let area = renderer::select::DataAreaElement;
    let before = config.data_area().expect("area").0;
    assert_eq!(
        area.drag_by(&mut config, &measure, 6.0, 5.0),
        renderer::layout::NudgeResult::Moved
    );
    assert_eq!(config.data_area().expect("area").0.width, before.width);
    edit(&mut r, id, config.clone()).expect("drag");
    assert!(
        needs(&r, id).is_empty(),
        "a moved plot area keeps its width"
    );
    export(&mut r, id, 1.0).expect("dragged");

    // The handle's gesture proposes a plot rect.
    assert_eq!(
        area.resize_by(&mut config, &measure, renderer::ResizeHandle::E, -40.0, 0.0),
        renderer::layout::NudgeResult::Moved
    );
    assert_eq!(
        area.resize_by(&mut config, &measure, renderer::ResizeHandle::NW, 12.0, 8.0),
        renderer::layout::NudgeResult::Moved
    );
    let proposed = config.data_area().expect("area").0;
    let chart_area = config.chart_area.0;
    let before = Before::of(&mut r, id);
    let plain = edit(&mut r, id, config.clone());
    assert!(mapping_edit(plain), "the editor refuses a plot resize");
    before.unchanged(&mut r, id, "plain");
    let unsourced = r.monotonic_chart(id).expect("monotonic chart").resize(
        chart_area,
        Some(proposed),
        None,
        &[],
    );
    before.refused(&mut r, id, unsourced, MappingChange::PlotWidth, "no source");
    let outside = renderer::layout::Rect {
        x: chart_area.width + 10,
        ..proposed
    };
    assert!(matches!(
        lent(&values, |groups| r
            .monotonic_chart(id)
            .expect("monotonic chart")
            .resize(chart_area, Some(outside), None, groups)),
        Err(FiggyError::InvalidConfig {
            kind: renderer::RuleKind::NoFeasibleLayout,
            ..
        })
    ));
    assert_eq!(r.monotonic_config(id).expect("config"), &before.config);

    lent(&values, |groups| {
        r.monotonic_chart(id).expect("monotonic chart").resize(
            chart_area,
            Some(proposed),
            None,
            groups,
        )
    })
    .expect("plot-area resize");
    let resized = r.monotonic_config(id).expect("config").clone();
    assert_eq!(resized.data_area().expect("area").0, proposed);
    assert!(needs(&r, id).is_empty());
    pick_middle(&mut r, id).expect("pick");
    assert_identical(
        &export(&mut r, id, 1.0).expect("resized"),
        &fresh_image(&resized, &[monotonic_series(1.0)], &values),
        "plot-area resize",
    );
}

/// `resize_monotonic` is atomic: the display and the document are planned
/// together. A display wider than the document at scale 1 alone keeps the
/// plot's device pixels and needs no source, but with a narrower chart area
/// the call moves the plot: without the source it is refused, and neither
/// the display nor the document changes.
#[test]
fn resize_monotonic_refuses_both_steps_or_applies_both() {
    let values = signal_values(0..100_000);
    let (mut r, id) = single(&values);
    let area = r.monotonic_config(id).expect("config").chart_area.0;
    let narrower = Rect { width: 500, ..area };
    let placed = |r: &Renderer| {
        let (_, panel, scale) = r.monotonic_display_config(id).expect("display");
        (panel, scale)
    };
    let shown = placed(&r);
    let before = Before::of(&mut r, id);
    let refused = r.monotonic_chart(id).expect("monotonic chart").resize(
        narrower,
        None,
        Some((700, 300)),
        &[],
    );
    before.refused(&mut r, id, refused, MappingChange::PlotWidth, "both steps");
    assert_eq!(placed(&r), shown, "the display is not half applied");
    // The display step alone is accepted without a source, and undone.
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .resize(area, None, Some((700, 300)), &[])
        .expect("the display alone");
    assert_ne!(placed(&r), shown);
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .resize(area, None, None, &[])
        .expect("no display again");
    assert_eq!(placed(&r), shown);
    // Both steps with the source.
    lent(&values, |groups| {
        r.monotonic_chart(id).expect("monotonic chart").resize(
            narrower,
            None,
            Some((700, 300)),
            groups,
        )
    })
    .expect("both steps with the source");
    let config = r.monotonic_config(id).expect("config").clone();
    assert_eq!(config.chart_area.0, narrower);
    assert!(needs(&r, id).is_empty());
    assert_identical(
        &export(&mut r, id, 1.0).expect("resized"),
        &fresh_image(&config, &[monotonic_series(1.0)], &values),
        "both steps",
    );
}

/// A 2x export maps to other device pixels: without the source it is
/// refused, with it it draws from an envelope that lives for the export
/// alone. PNG, SVG and PDF; the screen's envelope and the pool are left as
/// they were.
#[test]
fn an_export_at_2x_takes_the_sources() {
    let values = signal_values(0..100_000);
    let (mut r, id) = single(&values);
    let screen = export(&mut r, id, 1.0).expect("1x");
    let columns = r.logical_column_ids().count();
    assert!(requires(export(&mut r, id, 2.0), MappingChange::PlotWidth));
    let channels = [values.as_slice()];
    let source = MonotonicSlices::new(0.0, 1.0, &channels);
    let groups = [MonotonicGroupRef::new("g", &source)];
    let image = export_with(&mut r, id, 2.0, &groups);
    assert_eq!((image.width, image.height), (1200, 600));
    let with = ExportSource::Monotonic {
        chart: id,
        monotonic: &groups,
    };
    let svg = r
        .export_svg(with, ExportOptions::document(2.0))
        .block_on()
        .expect("svg");
    assert!(svg.contains("<image"));
    r.export_pdf(with, ExportOptions::document(2.0))
        .block_on()
        .expect("pdf");
    #[cfg(not(target_arch = "wasm32"))]
    r.export_png_blocking(with, ExportOptions::panel(2.0))
        .expect("blocking png");
    // The sources a 1x export does not need change nothing.
    assert_identical(
        &export_with(&mut r, id, 1.0, &groups),
        &screen,
        "1x with sources",
    );
    assert_identical(
        &export(&mut r, id, 1.0).expect("1x"),
        &screen,
        "screen kept",
    );
    assert_eq!(
        r.logical_column_ids().count(),
        columns,
        "no transient column left"
    );
    assert!(needs(&r, id).is_empty());
}

// ---------------------------------------------------------------------------
// Interactions
// ---------------------------------------------------------------------------

/// The canvas pixel of `(x, y)` in data units on `config`'s data area.
fn pixel_of(config: &renderer::Config, x: f64, y: f64) -> [f32; 2] {
    let area = config.data_area().expect("area").0;
    let (x0, x1) = (config.bottom_x.min, config.bottom_x.max);
    let (y0, y1) = (config.left_y.min, config.left_y.max);
    [
        area.x as f32 + ((x - x0) / (x1 - x0)) as f32 * area.width as f32,
        area.y as f32 + ((y1 - y) / (y1 - y0)) as f32 * area.height as f32,
    ]
}

fn pick(r: &mut Renderer, id: MonotonicChartId, at: [f32; 2]) -> Option<renderer::PickedData> {
    r.enable_gpu_picking().expect("picker");
    let config = r.monotonic_config(id).expect("config").clone();
    r.monotonic_chart(id)
        .expect("chart")
        .pick_data(pick_request(&config, at))
        .expect("pick")
        .resolve_blocking()
        .expect("resolve")
}

/// [`pick`] on the full draw's general chart.
fn pick_full(r: &mut Renderer, id: ChartId, at: [f32; 2]) -> Option<renderer::PickedData> {
    r.enable_gpu_picking().expect("picker");
    let config = r.chart_config(id).expect("config").clone();
    r.pick_chart_data(id, pick_request(&config, at))
        .expect("pick")
        .resolve_blocking()
        .expect("resolve")
}

fn pick_request(config: &renderer::Config, at: [f32; 2]) -> renderer::GpuPickRequest {
    renderer::GpuPickRequest {
        canvas_position_px: at,
        display_panel_px: config.chart_area.0,
        display_scale: 1.0,
        max_distance_px: 3.0,
    }
}

fn picked_index(picked: &Option<renderer::PickedData>) -> Option<usize> {
    match picked {
        Some(renderer::PickedData::Point {
            point_index,
            series_id,
            ..
        }) if series_id == "m" => Some(*point_index),
        _ => None,
    }
}

/// A pick reports the logical sample index of the kept point it lands on,
/// without the source: in a sparse view every sample, as the full draw's
/// pick reports it, in a dense view the column's extremes;
/// `monotonic_kept_point` reads that point back. Off the line nothing is
/// picked. The point pick agrees.
#[test]
fn a_pick_reports_the_logical_index_of_a_kept_point() {
    // Sparse: 100 samples about 4 px apart, every one kept.
    let sparse: Vec<f64> = (0..100).map(|i| signal(i * 600)).collect();
    let config = bare_config(600, 300, (0.0, 99.0), (-1.6, 1.6));
    let mut p = pair(config.clone(), &sparse, 0.0, 1.0, 1.0);
    for i in [10usize, 33, 77] {
        let at = pixel_of(&config, i as f64, sparse[i]);
        let full = match pick_full(&mut p.r, p.full, at) {
            Some(renderer::PickedData::Point { point_index, .. }) => Some(point_index),
            _ => None,
        };
        assert_eq!(
            picked_index(&pick(&mut p.r, p.monotonic, at)),
            Some(i),
            "sample {i}"
        );
        assert_eq!(full, Some(i), "the full draw picks the same sample");
    }
    let (mut r, id) = (p.r, p.monotonic);
    for i in [10usize, 33, 77] {
        let at = pixel_of(&config, i as f64, sparse[i]);
        assert_eq!(
            r.monotonic_kept_point(id, "m", i).expect("kept"),
            Some((i as f64, sparse[i]))
        );
        let point = r
            .monotonic_chart(id)
            .expect("chart")
            .pick(pick_request(&config, at))
            .expect("point pick")
            .resolve_blocking()
            .expect("resolve");
        assert_eq!(point.map(|p| p.point_index), Some(i));
    }
    let empty = pixel_of(&config, 50.0, 1.55);
    assert_eq!(pick(&mut r, id, empty), None, "off the line");

    // Dense: the highest sample of the whole window is a kept extreme.
    let dense = signal_values(0..100_000);
    let (mut r, id) = single(&dense);
    let config = r.monotonic_config(id).expect("config").clone();
    let (top, _) =
        dense.iter().enumerate().fold(
            (0, f64::MIN),
            |(bi, bv), (i, &v)| if v > bv { (i, v) } else { (bi, bv) },
        );
    let at = pixel_of(&config, top as f64, dense[top]);
    let index = picked_index(&pick(&mut r, id, at)).expect("a kept point is picked");
    let (x, y) = r
        .monotonic_kept_point(id, "m", index)
        .expect("kept")
        .expect("the picked index is kept");
    assert_eq!((x, y), (index as f64, dense[index]));
    let px = pixel_of(&config, x, y);
    assert!(
        (px[0] - at[0]).abs() <= 3.5 && (px[1] - at[1]).abs() <= 3.5,
        "the picked point lies under the cursor"
    );
    assert_eq!(
        r.monotonic_kept_point(id, "m", top + 1).expect("kept"),
        None
    );
}

fn select(r: &mut Renderer, id: MonotonicChartId, index: Option<usize>) {
    let mut config = r.monotonic_config(id).expect("config").clone();
    config.picked_data = index.map(|point_index| renderer::DataSelectionsConfig {
        visible: true,
        refs: vec![renderer::PickedDataRef::Point {
            series_id: "m".into(),
            source_id: None,
            point_index,
        }],
        ..renderer::DataSelectionsConfig::empty()
    });
    edit(r, id, config).expect("select");
}

/// A selected kept point is ringed at its place; a sample the envelope does
/// not keep has no highlight. Selecting needs no source.
#[test]
fn the_picked_highlight_rings_the_kept_point() {
    let values = signal_values(0..1_000);
    let (mut r, id) = single(&values);
    let config = r.monotonic_config(id).expect("config").clone();
    let plain = export(&mut r, id, 1.0).expect("plain");
    select(&mut r, id, Some(400));
    assert!(needs(&r, id).is_empty());
    let ringed = export(&mut r, id, 1.0).expect("ringed");
    let at = pixel_of(&config, 400.0, values[400]);
    let changed: Vec<(u32, u32)> = (0..plain.height)
        .flat_map(|y| (0..plain.width).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let i = ((y * plain.width + x) * 4) as usize;
            plain.rgba[i..i + 4] != ringed.rgba[i..i + 4]
        })
        .collect();
    assert!(!changed.is_empty(), "the ring is drawn");
    assert!(
        changed
            .iter()
            .all(|&(x, y)| (x as f32 - at[0]).abs() < 12.0 && (y as f32 - at[1]).abs() < 12.0),
        "the ring is at the kept point"
    );
    // Dense: a sample the envelope drops has nothing to ring.
    let dense = signal_values(0..100_000);
    let (mut r, id) = single(&dense);
    let plain = export(&mut r, id, 1.0).expect("plain");
    let dropped = (0..dense.len())
        .find(|&i| r.monotonic_kept_point(id, "m", i).expect("kept").is_none())
        .expect("a dense view drops samples");
    select(&mut r, id, Some(dropped));
    assert_identical(
        &export(&mut r, id, 1.0).expect("none"),
        &plain,
        "dropped sample",
    );
}

/// `step_chart_selection` walks a monotonic selection over the kept points
/// in x order and across the chart's monotonic series; other selections
/// step as `Config::step_selection` steps them.
#[test]
fn step_selection_walks_the_kept_points() {
    let values = signal_values(0..100_000);
    let mut r = renderer();
    let config = bare_config(600, 300, (0.0, 99_999.0), (-1.6, 1.6));
    let series = vec![
        SeriesConfig::monotonic_line("m", "g", 0).color(BLUE),
        SeriesConfig::monotonic_line("n", "g", 1).color(Color::BLACK),
    ];
    let id = r.register_monotonic_chart(config, series).expect("chart");
    let shifted: Vec<f64> = values.iter().map(|v| -v).collect();
    let channels = [values.as_slice(), shifted.as_slice()];
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_monotonic(
            "g",
            &MonotonicSlices::new(0.0, 1.0, &channels),
            MonotonicRange::X {
                min: 0.0,
                max: 99_999.0,
            },
        )
        .expect("set");
    let kept: Vec<usize> = (0..values.len())
        .filter(|&i| r.monotonic_kept_point(id, "m", i).expect("kept").is_some())
        .collect();
    select(&mut r, id, Some(kept[10]));
    let selected = |r: &Renderer| match r
        .monotonic_config(id)
        .expect("config")
        .picked_data
        .as_ref()
        .and_then(|s| s.refs.first())
    {
        Some(renderer::PickedDataRef::Point {
            series_id,
            point_index,
            ..
        }) => (series_id.clone(), *point_index),
        other => panic!("not a point: {other:?}"),
    };
    assert!(
        r.monotonic_chart(id)
            .expect("monotonic chart")
            .step_selection(1, 0)
            .expect("step")
    );
    assert_eq!(selected(&r), ("m".to_string(), kept[11]));
    assert!(
        r.monotonic_chart(id)
            .expect("monotonic chart")
            .step_selection(-3, 0)
            .expect("step")
    );
    assert_eq!(selected(&r), ("m".to_string(), kept[8]));
    assert!(
        r.monotonic_chart(id)
            .expect("monotonic chart")
            .step_selection(0, 1)
            .expect("step across")
    );
    let (series, index) = selected(&r);
    assert_eq!(series, "n");
    assert!(
        r.monotonic_kept_point(id, "n", index)
            .expect("kept")
            .is_some()
    );
    assert!(
        !r.monotonic_chart(id)
            .expect("monotonic chart")
            .step_selection(0, 1)
            .expect("last series")
    );
    assert!(
        r.monotonic_chart(id)
            .expect("monotonic chart")
            .step_selection(-1_000_000, 0)
            .expect("to the first")
    );
    assert!(
        !r.monotonic_chart(id)
            .expect("monotonic chart")
            .step_selection(-1, 0)
            .expect("clamped")
    );
}

/// The legend lists a monotonic series as its line, in its colour.
#[test]
fn the_legend_draws_the_series_line() {
    let values = signal_values(0..10_000);
    let mut r = renderer();
    let mut config = bare_config(600, 300, (0.0, 9_999.0), (-1.6, 1.6));
    config.left_y.max = 40.0;
    let series = vec![
        SeriesConfig::monotonic_line("m", "g", 0)
            .color(Color::from_rgb8(230, 20, 20))
            .label("channel 0"),
    ];
    config.legend.visible = true;
    renderer::config::append_legend_entry(
        &mut config.legend.content,
        renderer::config::series_symbol_segments(&series[0], None),
        "channel 0",
    );
    let id = r.register_monotonic_chart(config, series).expect("chart");
    set(&mut r, id, &values, 0.0, 1.0);
    let image = export(&mut r, id, 1.0).expect("export");
    // The data stays below y = 2 of 40: the red in the upper band is the
    // legend's line swatch.
    let area = r
        .monotonic_config(id)
        .expect("c")
        .data_area()
        .expect("area")
        .0;
    let red = (area.y..area.y + area.height / 3).any(|y| {
        (area.x..area.x + area.width).any(|x| {
            let i = ((y * image.width + x) * 4) as usize;
            image.rgba[i] > 180 && image.rgba[i + 1] < 90 && image.rgba[i + 3] > 200
        })
    });
    assert!(red, "the legend swatch is drawn in the series colour");
}

/// A monotonic series on an overlay layer is drawn against the overlay's y
/// axis, and in a subplot at the subplot's own x range and width: sparse
/// views, so each is the full draw's byte for byte. A slide moves the
/// subplot's own x axis.
#[test]
fn overlays_and_subplots_draw_their_envelopes() {
    let values = signal_values(0..1_200);
    let mut r = renderer();
    upload_full(&mut r, &values, 0.0, 1.0);
    let overlaid = |mut config: renderer::Config, series: &str| {
        let mut overlay = renderer::OverlayLayer::default();
        overlay.y_axis.min = -3.0;
        overlay.y_axis.max = 2.0;
        overlay.series = vec![series.into()];
        config.overlays.push(overlay);
        config
    };
    let config = bare_config(600, 300, (0.0, 1_199.0), (-1.6, 1.6));
    let full = r
        .register_chart(overlaid(config.clone(), "full"), vec![full_series(1.0)])
        .expect("full");
    let monotonic = r
        .register_monotonic_chart(overlaid(config.clone(), "m"), vec![monotonic_series(1.0)])
        .expect("monotonic");
    set(&mut r, monotonic, &values, 0.0, 1.0);
    assert_identical(
        &export(&mut r, full, 1.0).expect("full"),
        &export(&mut r, monotonic, 1.0).expect("monotonic"),
        "overlay",
    );

    let subplotted = |series: &str| {
        let mut parent = bare_config(600, 300, (0.0, 1.0), (0.0, 1.0));
        let mut sub = bare_config(400, 200, (100.0, 900.0), (-1.6, 1.6));
        sub.chart_area.0.y = 300;
        parent.subplots = vec![renderer::Subplot {
            config: sub,
            series: vec![series.into()],
            link_x: None,
            link_y: None,
            marker: None,
            text_scale: Default::default(),
        }];
        parent
    };
    let full = r
        .register_chart(subplotted("full"), vec![full_series(1.0)])
        .expect("full");
    let monotonic = r
        .register_monotonic_chart(subplotted("m2"), vec![named_monotonic("m2", 1.0)])
        .expect("monotonic");
    // The source ends at the window's end, as a sliding host's does.
    set_range(
        &mut r,
        monotonic,
        &values[..901],
        0.0,
        1.0,
        MonotonicRange::X {
            min: 100.0,
            max: 900.0,
        },
    )
    .expect("subplot");
    fn document(r: &mut Renderer, id: impl Exported) -> RasterImage {
        r.export_rgba(id.source(), ExportOptions::document(1.0))
            .block_on()
            .expect("document")
    }
    upload_full(&mut r, &values[..901], 0.0, 1.0);
    assert_identical(
        &document(&mut r, full),
        &document(&mut r, monotonic),
        "subplot",
    );
    let piece = [&values[901..951]];
    r.monotonic_chart(monotonic)
        .expect("monotonic chart")
        .append_monotonic(
            "g",
            &MonotonicSlices::new(0.0, 1.0, &piece),
            MonotonicAppend { sliding: true },
        )
        .expect("slide");
    let stored = r.monotonic_config(monotonic).expect("config");
    assert_eq!(
        (
            stored.subplots[0].config.bottom_x.min,
            stored.subplots[0].config.bottom_x.max
        ),
        (150.0, 950.0),
        "the slide moves the subplot's x axis"
    );
    assert_eq!((stored.bottom_x.min, stored.bottom_x.max), (0.0, 1.0));
    assert!(needs(&r, monotonic).is_empty());
}

/// The document names the group and channel; a round trip through JSON is
/// byte-identical and draws the same image once its source is supplied.
#[cfg(feature = "serde")]
#[test]
fn a_document_round_trip_draws_the_same_image() {
    let values = signal_values(0..20_000);
    let (mut r, id) = single(&values);
    let config = r.monotonic_config(id).expect("config").clone();
    let series = r.monotonic_series(id).expect("series").to_vec();
    let config_json = serde_json::to_string(&config).expect("config json");
    let series_json = serde_json::to_string(&series).expect("series json");
    assert!(series_json.contains("\"MonotonicLine\""));
    assert!(series_json.contains("\"group\":\"g\""));
    let config_back: renderer::Config = serde_json::from_str(&config_json).expect("config");
    let series_back: Vec<SeriesConfig> = serde_json::from_str(&series_json).expect("series");
    assert_eq!(
        serde_json::to_string(&config_back).expect("json"),
        config_json
    );
    assert_eq!(
        serde_json::to_string(&series_back).expect("json"),
        series_json
    );
    assert_eq!(series_back, series);
    assert_identical(
        &fresh_image(&config_back, &series_back, &values),
        &export(&mut r, id, 1.0).expect("export"),
        "round trip",
    );
}

/// An edit of a series' group or channel leaves it needing its source; a
/// series removed from the chart takes its envelope columns with the next
/// full call, and a removed chart takes them at once. Envelope columns are
/// the renderer's: a host cannot write or remove them, and a series cannot
/// take a column it does not own.
#[test]
fn series_edits_and_column_ownership() {
    let values = signal_values(0..20_000);
    let (mut r, id) = single(&values);
    // A channel the group's data does not have is refused; once a full
    // call supplies it, the channel edit is a plain call.
    let other = vec![SeriesConfig::monotonic_line("m", "g", 1).color(BLUE)];
    let no_channel = |result: Result<(), FiggyError>| {
        matches!(
            result,
            Err(FiggyError::InvalidSeriesConfig {
                kind: renderer::RuleKind::MonotonicChannel {
                    channel: 1,
                    channels: 1
                },
                ..
            })
        )
    };
    assert!(no_channel(
        r.monotonic_chart(id)
            .expect("monotonic chart")
            .set_series(other.clone())
    ));
    let two = [values.as_slice(), values.as_slice()];
    let range = x_range(&r, id);
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_monotonic("g", &MonotonicSlices::new(0.0, 1.0, &two), range)
        .expect("two channels");
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_series(other)
        .expect("channel 1");
    assert!(needs(&r, id).is_empty());
    // A host cannot write or remove an envelope column.
    assert!(matches!(
        r.upsert_column("m.monotonic.x", &vec![0.0, 1.0]),
        Err(FiggyError::ReservedColumnId { .. })
    ));
    assert!(matches!(
        r.remove_column("m.monotonic.y"),
        Err(FiggyError::ReservedColumnId { .. })
    ));
    // A series whose envelope columns are a host's columns is refused.
    r.add_column("taken.x", &vec![0.0, 1.0])
        .expect("host column");
    let mut taken = SeriesConfig::monotonic_line("t", "h", 0);
    taken.x_column = "taken.x".into();
    let config = r.monotonic_config(id).expect("config").clone();
    let other = r
        .register_monotonic_chart(config, vec![taken])
        .expect("chart");
    assert!(matches!(
        r.monotonic_chart(other)
            .expect("monotonic chart")
            .set_monotonic(
                "h",
                &MonotonicSlices::new(0.0, 1.0, &[values.as_slice()]),
                MonotonicRange::Autoscale
            ),
        Err(FiggyError::InvalidSeriesConfig {
            kind: renderer::RuleKind::MonotonicColumnTaken { .. },
            ..
        })
    ));
    // No series of the chart names the group.
    assert!(matches!(
        r.monotonic_chart(id)
            .expect("monotonic chart")
            .set_monotonic(
                "nobody",
                &MonotonicSlices::new(0.0, 1.0, &[values.as_slice()]),
                MonotonicRange::Autoscale
            ),
        Err(FiggyError::UnknownMonotonicGroup { .. })
    ));
    // An append before any full call.
    assert!(matches!(
        r.monotonic_chart(other)
            .expect("monotonic chart")
            .append_monotonic(
                "h",
                &MonotonicSlices::new(0.0, 1.0, &[values.as_slice()]),
                MonotonicAppend { sliding: false }
            ),
        Err(FiggyError::Monotonic {
            error: renderer::MonotonicError::NotBuilt,
            ..
        })
    ));
    // Removing the series and calling again frees its columns; removing
    // the chart frees the rest.
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_series(vec![SeriesConfig::monotonic_line("k", "g", 0)])
        .expect("replace");
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_monotonic(
            "g",
            &MonotonicSlices::new(0.0, 1.0, &[values.as_slice()]),
            MonotonicRange::Autoscale,
        )
        .expect("set");
    let ids: Vec<String> = r.logical_column_ids().map(str::to_owned).collect();
    assert!(!ids.iter().any(|c| c.starts_with("m.monotonic")));
    assert!(ids.iter().any(|c| c == "k.monotonic.x"));
    r.monotonic_chart(id)
        .expect("chart")
        .remove()
        .expect("remove");
    assert!(!r.logical_column_ids().any(|c| c.starts_with("k.monotonic")));
}

/// No count limit (rule 10, M7): 300 channels of one interleaved buffer,
/// one series each, past every bound a series cache has (256), each drawn
/// at its own level.
#[test]
fn three_hundred_channels_draw_as_three_hundred_series() {
    let channels = 300;
    let n = 4_000;
    let stride = channels + 1;
    let mut frames = vec![0.0f64; n * stride];
    for i in 0..n {
        for c in 0..channels {
            frames[i * stride + c + 1] = c as f64 + 0.3 * signal(i);
        }
    }
    let columns: Vec<usize> = (1..=channels).collect();
    let source = MonotonicInterleaved::new(0.0, 1.0, &frames, stride, &columns).expect("source");
    let mut r = renderer();
    let config = bare_config(600, 900, (0.0, (n - 1) as f64), (-1.0, channels as f64));
    let series: Vec<SeriesConfig> = (0..channels)
        .map(|c| SeriesConfig::monotonic_line(format!("c{c}"), "g", c).color(BLUE))
        .collect();
    let id = r.register_monotonic_chart(config, series).expect("chart");
    let range = x_range(&r, id);
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_monotonic("g", &source, range)
        .expect("set");
    assert!(r.monotonic_needs_source(id).expect("needs").is_empty());
    let image = export(&mut r, id, 1.0).expect("export");
    let config = r.monotonic_config(id).expect("config").clone();
    for c in [0usize, 128, 255, 256, 299] {
        let [x, y] = pixel_of(&config, 2_000.0, c as f64 + 0.3 * signal(2_000));
        let near = (y as u32 - 2..=y as u32 + 2)
            .any(|yy| painted(&image, x as u32, yy) || painted(&image, x as u32 + 1, yy));
        assert!(near, "channel {c} is drawn");
    }
}

/// Two groups sampled on different grids, one on the base layer and one on
/// an overlay with an X axis of its own (#283): each group's full call,
/// slide and Autoscale move only its own X axis, so neither needs the
/// other's source, and the chart draws what a fresh renderer draws after one
/// full call per group at the same ranges.
#[test]
fn groups_on_an_overlay_with_its_own_x_zoom_and_slide_independently() {
    let a = signal_values(0..10_000);
    let b: Vec<f64> = (0..4_000).map(|i| 0.5 * signal(i * 3)).collect();
    let (ca, cb) = ([a.as_slice()], [b.as_slice()]);
    let (sa, sb) = (
        MonotonicSlices::new(0.0, 1.0, &ca),
        MonotonicSlices::new(0.0, 2.5, &cb),
    );
    let mut config = bare_config(600, 300, (0.0, 10_000.0), (-2.0, 2.0));
    config.overlays.push(renderer::OverlayLayer {
        link_x: false,
        series: vec!["o".to_string()],
        ..Default::default()
    });
    config.overlays[0].x_axis.min = 0.0;
    config.overlays[0].x_axis.max = 10_000.0;
    config.overlays[0].x_axis.out_margin = 30.0;
    config.overlays[0].y_axis.min = -2.0;
    config.overlays[0].y_axis.max = 2.0;
    let series = vec![
        SeriesConfig::monotonic_line("m", "g", 0).color(BLUE),
        SeriesConfig::monotonic_line("o", "h", 0).color(BLUE),
    ];
    let mut r = renderer();
    let id = r
        .register_monotonic_chart(config.clone(), series.clone())
        .expect("chart");
    let mut chart = r.monotonic_chart(id).expect("monotonic chart");
    chart
        .set_monotonic(
            "g",
            &sa,
            MonotonicRange::X {
                min: 0.0,
                max: 10_000.0,
            },
        )
        .expect("base group");
    chart
        .set_monotonic(
            "h",
            &sb,
            MonotonicRange::X {
                min: 0.0,
                max: 10_000.0,
            },
        )
        .expect("overlay group");
    // Each zoom moves its own axis only and needs only its own source.
    chart
        .set_monotonic(
            "g",
            &sa,
            MonotonicRange::X {
                min: 0.0,
                max: 5_000.0,
            },
        )
        .expect("base zoom");
    // The overlay's window ends where its samples do, as a sliding host's
    // does.
    let end = (b.len() - 1) as f64 * 2.5;
    chart
        .set_monotonic(
            "h",
            &sb,
            MonotonicRange::X {
                min: 6_000.0,
                max: end,
            },
        )
        .expect("overlay zoom");
    let fitted = r.monotonic_config(id).expect("config").clone();
    assert_eq!((fitted.bottom_x.min, fitted.bottom_x.max), (0.0, 5_000.0));
    let own = &fitted.overlays[0].x_axis;
    assert_eq!((own.min, own.max), (6_000.0, end));
    // A slide of one group needs only its own new samples.
    let tail = [&b[..200]];
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .append_monotonic(
            "h",
            &MonotonicSlices::new(0.0, 2.5, &tail),
            MonotonicAppend { sliding: true },
        )
        .expect("overlay slide");
    let slid = r.monotonic_config(id).expect("config").clone();
    assert_eq!((slid.bottom_x.min, slid.bottom_x.max), (0.0, 5_000.0));
    let own = &slid.overlays[0].x_axis;
    assert_eq!((own.min, own.max), (6_000.0 + 500.0, end + 500.0));
    // Autoscale fits the overlay's own X to its group's grid, with its own
    // tick plan, and leaves the parent's.
    let grown: Vec<f64> = b.iter().chain(&b[..200]).copied().collect();
    let cg = [grown.as_slice()];
    let sg = MonotonicSlices::new(0.0, 2.5, &cg);
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_monotonic("h", &sg, MonotonicRange::Autoscale)
        .expect("overlay autoscale");
    let auto = r.monotonic_config(id).expect("config").clone();
    let own = &auto.overlays[0].x_axis;
    assert_eq!((own.min, own.max), (0.0, (grown.len() - 1) as f64 * 2.5));
    assert_eq!((auto.bottom_x.min, auto.bottom_x.max), (0.0, 5_000.0));
    // The image is what one full call per group draws at these ranges.
    let mut fresh = renderer();
    let plain = fresh
        .register_monotonic_chart(auto.clone(), series)
        .expect("fresh");
    let mut chart = fresh.monotonic_chart(plain).expect("monotonic chart");
    chart
        .set_monotonic(
            "g",
            &sa,
            MonotonicRange::X {
                min: 0.0,
                max: 5_000.0,
            },
        )
        .expect("fresh base");
    chart
        .set_monotonic(
            "h",
            &sg,
            MonotonicRange::X {
                min: own.min,
                max: own.max,
            },
        )
        .expect("fresh overlay");
    let document = |r: &mut Renderer, id: MonotonicChartId| {
        r.export_rgba(id.source(), ExportOptions::document(1.0))
            .block_on()
            .expect("export")
    };
    assert_identical(
        &document(&mut r, id),
        &document(&mut fresh, plain),
        "groups on an overlay with its own x",
    );
}

/// Autoscale fits each y axis to every monotonic series drawn on it, of any
/// group (#283): a group on an overlay with its own x axis that follows the
/// parent's y axis (`link_y`) shares `left_y` with the base group, so an
/// Autoscale of either keeps both in view, and the other group is not
/// reduced again (its x axis did not move).
#[test]
fn autoscale_keeps_every_group_on_a_shared_y_axis_in_view() {
    let low: Vec<f64> = (0..8_000).map(|i| -1.0 + (i % 9) as f64 * 0.25).collect();
    let high: Vec<f64> = (0..3_000).map(|i| 5.0 + (i % 5) as f64 * 0.25).collect();
    let (cl, ch) = ([low.as_slice()], [high.as_slice()]);
    let (sl, sh) = (
        MonotonicSlices::new(0.0, 1.0, &cl),
        MonotonicSlices::new(0.0, 3.0, &ch),
    );
    let mut config = bare_config(600, 300, (0.0, 8_000.0), (-9.0, 9.0));
    config.overlays.push(renderer::OverlayLayer {
        link_x: false,
        link_y: true,
        series: vec!["o".to_string()],
        ..Default::default()
    });
    config.overlays[0].x_axis.max = 9_000.0;
    let series = vec![
        SeriesConfig::monotonic_line("m", "g", 0).color(BLUE),
        SeriesConfig::monotonic_line("o", "h", 0).color(BLUE),
    ];
    let mut r = renderer();
    let id = r.register_monotonic_chart(config, series).expect("chart");
    let mut chart = r.monotonic_chart(id).expect("monotonic chart");
    chart
        .set_monotonic("g", &sl, MonotonicRange::Autoscale)
        .expect("base group");
    chart
        .set_monotonic("h", &sh, MonotonicRange::Autoscale)
        .expect("overlay group");
    let y = r.monotonic_config(id).expect("config").left_y.clone();
    assert!(y.min < -1.0 && y.max > 6.0, "both groups in view: {y:?}");
    // The other order fits the same axis.
    r.monotonic_chart(id)
        .expect("monotonic chart")
        .set_monotonic("g", &sl, MonotonicRange::Autoscale)
        .expect("base group again");
    let again = r.monotonic_config(id).expect("config").left_y.clone();
    assert_eq!((again.min, again.max), (y.min, y.max));
    // The overlay's own x is its group's grid; the parent's, the base's.
    let config = r.monotonic_config(id).expect("config");
    assert_eq!(config.overlays[0].x_axis.max, 2_999.0 * 3.0);
    assert_eq!(config.bottom_x.max, 7_999.0);
}
