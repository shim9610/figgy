//! Host-side allocation measurement: the data lives in the pool, so uploading
//! more of it must not cost more host memory.
//!
//! This is the direct evidence for "there is no copy of the data". The
//! instrument is a counting global allocator — exact, portable, no dependency —
//! and it lives in its own test binary. Every measured test takes one process
//! lock, while process-wide atomics pair allocations with frees performed by
//! wgpu worker threads.
//!
//! Expected: slope ≈ 0 host bytes per input byte, and a per-upload allocation
//! count that does not grow with the data (the pool records one slot and one id
//! string per column, whatever its length).
//!
//! Run:
//!     cargo test -p figgy-renderer --test cpu_allocation_scaling -- --nocapture

#[path = "support/gpu.rs"]
mod test_gpu;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use renderer::{
    Chart, ChartDrawItem, Column, ColumnSource, DefragPolicy, GrowthPolicy, Renderer,
    RendererDevice,
};

static LIVE_BYTES: AtomicI64 = AtomicI64::new(0);
static PEAK_LIVE_BYTES: AtomicI64 = AtomicI64::new(0);
static CALLS: AtomicU64 = AtomicU64::new(0);
/// Bytes ever requested, frees not subtracted: a temporary copy shows here
/// even when it is freed before the measured window ends.
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static MEASUREMENT_LOCK: Mutex<()> = Mutex::new(());

struct CountingAllocator;

fn record_live_delta(delta: i64) {
    let live = LIVE_BYTES.fetch_add(delta, Ordering::Relaxed) + delta;
    if delta > 0 {
        PEAK_LIVE_BYTES.fetch_max(live, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record_live_delta(layout.size() as i64);
            CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        record_live_delta(-(layout.size() as i64));
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(ptr, layout, new_size) };
        if !result.is_null() {
            record_live_delta(new_size as i64 - layout.size() as i64);
            CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOCATED_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Process-wide allocated bytes minus freed bytes.
///
/// Every measured test holds `MEASUREMENT_LOCK`, so test bodies cannot overlap.
/// Global counters are required because wgpu may free on a worker thread; a
/// thread-local counter cannot pair that free with the originating allocation.
fn live_bytes() -> i64 {
    LIVE_BYTES.load(Ordering::Relaxed)
}

fn reset_peak_live_bytes() -> i64 {
    let live = live_bytes();
    PEAK_LIVE_BYTES.store(live, Ordering::Relaxed);
    live
}

fn peak_live_bytes() -> i64 {
    PEAK_LIVE_BYTES.load(Ordering::Relaxed)
}

fn calls() -> u64 {
    CALLS.load(Ordering::Relaxed)
}

fn allocated_bytes() -> u64 {
    ALLOCATED_BYTES.load(Ordering::Relaxed)
}

/// Set in the child process `measured` starts, so it measures instead of
/// starting another.
const CHILD_ENV: &str = "FIGGY_ALLOCATION_MEASUREMENT_CHILD";

/// Whether `RUST_BACKTRACE` or `RUST_LIB_BACKTRACE` turns backtrace capture
/// on. In debug builds wgpu-core's snatch-lock trace then captures a real
/// backtrace on every `queue.write_buffer`, which adds host allocations
/// the renderer does not make. The counts would depend on the shell that
/// runs the test.
fn backtraces_requested() -> bool {
    ["RUST_LIB_BACKTRACE", "RUST_BACKTRACE"]
        .iter()
        .find_map(std::env::var_os)
        .is_some_and(|value| value != "0")
}

/// The measurement lock for test `name`, or `None` when this process
/// should not measure. With backtrace capture requested, the test runs
/// again in a child process with both variables removed, so every shell
/// measures the same counts; this process then only checks that the child
/// passed. Setting the variables here instead would race the other test
/// threads that read the environment.
fn measured(name: &str) -> Option<MutexGuard<'static, ()>> {
    if !backtraces_requested() || std::env::var_os(CHILD_ENV).is_some() {
        return Some(measurement_guard());
    }
    let status = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args([name, "--exact", "--nocapture", "--test-threads=1"])
        .env_remove("RUST_BACKTRACE")
        .env_remove("RUST_LIB_BACKTRACE")
        .env(CHILD_ENV, "1")
        .status()
        .expect("run the measurement without backtrace capture");
    assert!(
        status.success(),
        "{name} failed without backtrace capture: {status}"
    );
    None
}

fn measurement_guard() -> MutexGuard<'static, ()> {
    MEASUREMENT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn no_gpu(context: &str) {
    panic!("required GPU test: {context}: GPU initialization failed");
}

fn settle_gpu(renderer: &Renderer) {
    test_gpu::block_on(renderer.wait_submitted_work()).unwrap();
}

const COLUMN_VALUE_BYTES: u64 = 8;
const STEP_INPUT_BYTES: u64 = 1 << 20;
const STEPS: &[u64] = &[1, 2, 4, 8];

fn least_squares(points: &[(f64, f64)]) -> (f64, f64) {
    let n = points.len() as f64;
    let mean_x = points.iter().map(|(x, _)| x).sum::<f64>() / n;
    let mean_y = points.iter().map(|(_, y)| y).sum::<f64>() / n;
    let sxx: f64 = points.iter().map(|(x, _)| (x - mean_x).powi(2)).sum();
    let sxy: f64 = points
        .iter()
        .map(|(x, y)| (x - mean_x) * (y - mean_y))
        .sum();
    let slope = if sxx == 0.0 { 0.0 } else { sxy / sxx };
    (slope, mean_y - slope * mean_x)
}

fn renderer_with_pool(pool_bytes: u64) -> Option<Renderer> {
    let (device, queue) = test_gpu::device();
    Some(
        Renderer::try_new(
            RendererDevice::new(device, queue).expect("figgy poll thread"),
            wgpu::TextureFormat::Rgba8Unorm,
            pool_bytes,
        )
        .expect("required GPU test: renderer initialization failed"),
    )
}

#[test]
fn host_bytes_do_not_scale_with_uploaded_data() {
    let Some(_measurement) = measured("host_bytes_do_not_scale_with_uploaded_data") else {
        return;
    };
    // One renderer, pre-sized for the largest column, warmed up first.
    // Measuring across construction would fold in shader compilation and
    // one-time driver caches: large, one-off, and nothing to do with the data.
    let largest_values = (STEPS.last().expect("steps") * STEP_INPUT_BYTES / 4) as usize;
    let pool = (largest_values as u64) * COLUMN_VALUE_BYTES + (1 << 20);
    let Some(mut renderer) = renderer_with_pool(pool) else {
        no_gpu("host allocation measurement");
        return;
    };
    renderer.set_defrag_policy(DefragPolicy::OnAllocFailure);
    renderer
        .add_column(
            "warm",
            &Column {
                data: (0..1024).map(|i| i as f32).collect(),
            },
        )
        .unwrap();
    renderer.end_gpu_frame();
    settle_gpu(&renderer);

    println!("\nhost bytes across one upload (same renderer, warmed)");
    println!(
        "{:>12} {:>12} {:>12} {:>10}",
        "input B", "host ΔB", "peak ΔB", "allocs"
    );
    let mut byte_points = Vec::new();
    let mut peak_points = Vec::new();
    let mut call_points = Vec::new();
    for step in STEPS {
        let values = ((step * STEP_INPUT_BYTES) / 4) as usize;
        // The caller's buffer is built before the baseline: it is the host's
        // own data, and counting it would hide whether the renderer copies it.
        let column = Column {
            data: (0..values).map(|i| i as f32).collect(),
        };
        settle_gpu(&renderer);
        let bytes_before = reset_peak_live_bytes();
        let calls_before = calls();
        renderer.upsert_column("scale", &column).unwrap();
        renderer.end_gpu_frame();
        settle_gpu(&renderer);
        let bytes = live_bytes() - bytes_before;
        let peak_bytes = peak_live_bytes() - bytes_before;
        let allocations = calls() - calls_before;
        println!(
            "{:>12} {:>12} {:>12} {:>10}",
            values * 4,
            bytes,
            peak_bytes,
            allocations
        );
        byte_points.push(((values * 4) as f64, bytes as f64));
        peak_points.push(((values * 4) as f64, peak_bytes as f64));
        call_points.push(((values * 4) as f64, allocations as f64));
    }

    let (byte_slope, byte_dc) = least_squares(&byte_points);
    let (peak_slope, peak_dc) = least_squares(&peak_points);
    let (call_slope, call_dc) = least_squares(&call_points);
    println!("  bytes: slope {byte_slope:.8} B/input B, DC {byte_dc:.0} B");
    println!("  peak:  slope {peak_slope:.8} B/input B, DC {peak_dc:.0} B");
    println!("  allocs: slope {call_slope:.8} per input B, DC {call_dc:.0}");

    assert!(
        byte_slope.abs() <= 0.02,
        "host bytes grew {byte_slope:.6} B per input byte (limit 0.02). The pool is \
         where the data lives, so a host-side copy of the values would show up \
         here. (The mapped staging buffer is device memory and is released before \
         this is read.)"
    );
    assert!(
        peak_slope.abs() <= 0.125,
        "peak host bytes grew {peak_slope:.6} B per input byte (limit 0.125). \
         This catches a temporary data-sized host copy even when it is freed \
         before upload returns."
    );
    assert!(
        call_slope.abs() * (STEP_INPUT_BYTES as f64) <= 8.0,
        "allocation count grew {:.2} per MiB of input (limit 8). A per-value or \
         per-chunk host allocation would show up here.",
        call_slope * (STEP_INPUT_BYTES as f64)
    );
}

// Scenario baselines — the CPU half of the allocation gate (design A.7 ①).
//
// The design's target for a warm frame draw is **zero** host allocations. It is
// not zero today, and pretending otherwise by loosening the number would hide
// the gap. So each scenario records what it actually costs and fails if the
// count grows: the numbers below are a measured baseline to drive down, not an
// endorsement.

/// Allocations a warm prepare may make **per panel**.
///
/// Measured 7 for one panel and 4 more per further panel on lavapipe.
/// Figgy's own share is three `Vec`s per frame, whatever the panel count: the
/// panel inputs `prepare` borrows, the per-panel products its phases fill in
/// place, and the token's prepared items. The rest is wgpu's own bookkeeping
/// inside `queue.write_buffer` for each panel's transform uniform: the
/// staging buffer's label copied twice and an `Arc<Backtrace>` in
/// gpu-allocator, and the barrier `Vec` in wgpu-core. A debug build run with
/// `RUST_BACKTRACE` set would cost 4 more per write, because wgpu-core's
/// snatch-lock trace then captures a real backtrace, so the measured tests
/// run with backtrace capture off (`measured`).
const MAX_PREPARE_ALLOCATIONS_PER_PANEL: u64 = 12;

/// Measured 135 on llvmpipe: allocator bookkeeping for the relayout, not data.
const MAX_GROWTH_ALLOCATIONS: u64 = 200;

/// Host allocations a batch upload may cost **per column**.
///
/// Measured 4.09 on llvmpipe, and *identical* at 64 and 4096 values per column:
/// the registry's three owned id strings plus amortized map growth. The
/// per-column path costs 169 (64 values) to 553 (4096 values) for the same work,
/// and unlike the batch it follows the data.
const MAX_BATCH_ALLOCATIONS_PER_COLUMN: f64 = 6.0;

fn panel_config(width: u32, height: u32) -> renderer::Config {
    let mut config = renderer::default::default_config();
    config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
        x: 0,
        y: 0,
        width,
        height,
    });
    config
}

/// The design's target for a warm frame is zero host allocations. It *is* zero
/// with nothing to prepare, and per panel it is a small constant. The literal
/// zero is not reachable while a frame writes its transform uniform through
/// `queue.write_buffer`: wgpu allocates inside that call, and no amount of
/// figgy-side discipline removes it.
///
/// This asserts on the **count** only. Retained bytes are printed beside it as a
/// diagnostic because warm driver caches are not a per-frame ownership bound.
#[test]
fn a_warm_prepare_costs_a_constant_per_panel_and_nothing_for_no_panels() {
    let Some(_measurement) =
        measured("a_warm_prepare_costs_a_constant_per_panel_and_nothing_for_no_panels")
    else {
        return;
    };
    let Some(mut renderer) = renderer_with_pool(4 << 20) else {
        no_gpu("prepare allocation measurement");
        return;
    };
    let chart = Chart::new(panel_config(320, 240));
    let mut views: Vec<_> = (0..4)
        .map(|_| {
            renderer
                .create_chart_view(&chart, chart.config().chart_area.0)
                .unwrap()
        })
        .collect();

    println!("\nwarm prepare");
    // `retained B` is a diagnostic, not an assertion — see `live_bytes`.
    println!(
        "{:>8} {:>14} {:>14} {:>14}",
        "panels", "allocations", "retained B", "warm-up B"
    );
    let mut measured = Vec::new();
    for panels in [0usize, 1, 2, 4] {
        let mut items: Vec<ChartDrawItem<'_>> = views
            .iter_mut()
            .take(panels)
            .map(|view| ChartDrawItem {
                view,
                chart: &chart,
                series: &[],
            })
            .collect();
        // Warm this exact shape: a shape's first prepare compiles pipeline
        // variants and fills caches, which is not per-frame cost.
        renderer.prepare(&mut items).unwrap();

        let calls_before = calls();
        let prepared = renderer.prepare(&mut items).unwrap();
        let allocations = calls() - calls_before;
        drop(prepared);

        // Retained is measured as a **steady-state** diagnostic, not a one-shot
        // ownership bound. A one-shot delta cannot distinguish a leak from a
        // driver cache filling for the first time.
        let mut cycle = |renderer: &mut Renderer, rounds: usize| -> i64 {
            let before = live_bytes();
            for _ in 0..rounds {
                let prepared = renderer.prepare(&mut items).unwrap();
                drop(prepared);
            }
            live_bytes() - before
        };
        let warm = cycle(&mut renderer, 4);
        let retained = cycle(&mut renderer, 4);
        println!("{panels:>8} {allocations:>14} {retained:>14} {warm:>14}");
        measured.push((panels as u64, allocations));
    }

    let (_, empty_allocations) = measured[0];
    assert_eq!(
        empty_allocations, 0,
        "a prepare with no panels must allocate nothing at all"
    );
    for (panels, allocations) in &measured[1..] {
        assert!(
            *allocations <= panels * MAX_PREPARE_ALLOCATIONS_PER_PANEL,
            "{allocations} allocations for {panels} panel(s) exceeds \
             {MAX_PREPARE_ALLOCATIONS_PER_PANEL} per panel. A new per-frame \
             allocation was introduced."
        );
    }
}

/// The frame's cost must not depend on how much data the panel draws. This is
/// the invariant behind "no per-point work in the frame": a series 256× longer
/// must prepare in the same number of host allocations.
#[test]
fn prepare_cost_is_independent_of_series_length() {
    let Some(_measurement) = measured("prepare_cost_is_independent_of_series_length") else {
        return;
    };
    let Some(mut renderer) = renderer_with_pool(16 << 20) else {
        no_gpu("prepare-vs-length measurement");
        return;
    };
    let short = 1_024usize;
    let long = 262_144usize;
    for (id, len) in [("x", long), ("y", long), ("sx", short), ("sy", short)] {
        renderer
            .add_column(
                id,
                &Column {
                    data: (0..len).map(|i| i as f32).collect(),
                },
            )
            .unwrap();
    }
    renderer.end_gpu_frame();

    let chart = Chart::new(panel_config(320, 240));
    let mut view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();

    let mut counts = Vec::new();
    for (x, y, len) in [("sx", "sy", short), ("x", "y", long)] {
        let series_config = renderer::SeriesConfig {
            series_id: format!("series-{len}"),
            source_id: None,
            label: None,
            x_column: x.to_string(),
            y_column: y.to_string(),
            z_column: None,
            render_type: renderer::DataRenderType::Xy(renderer::XySeries {
                scatter: None,
                line: Some(renderer::DataLineStyleConfig {
                    line_style: renderer::line::LineStylePreset::Solid,
                    line_color: renderer::Color::new(0.1, 0.2, 0.3, 1.0),
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: renderer::data_config::LineInterpolation::Linear,
                }),
                errors: None,
                stem: None,
            }),
        };
        let style = renderer.create_style_for_series(&series_config).unwrap();
        let series = [renderer::Series {
            config: &series_config,
            style: &style,
        }];
        let mut items = [ChartDrawItem {
            view: &mut view,
            chart: &chart,
            series: &series,
        }];
        // Two warm-ups: the first prepare of a series compiles what it needs
        // and builds its arc scratch; the second settles the cache.
        renderer.prepare(&mut items).unwrap();
        renderer.prepare(&mut items).unwrap();

        let before = calls();
        let prepared = renderer.prepare(&mut items).unwrap();
        let allocations = calls() - before;
        drop(prepared);
        println!("prepare with {len} points: {allocations} allocations");
        counts.push(allocations);
    }

    let (short_count, long_count) = (counts[0], counts[1]);
    // Not exact equality: the scan is chunked, so the number of chunks can
    // shift a couple of allocations either way (measured 18 vs 17 — the longer
    // series took one *fewer*). A per-point cost could not hide inside that
    // window: at 256× the length it would be hundreds of allocations more.
    assert!(
        long_count.abs_diff(short_count) <= 4 && long_count <= short_count * 2,
        "a series {}× longer changed the frame's host allocation count \
         ({short_count} → {long_count}). Frame work must be per series and per \
         chunk, never per point.",
        long / short
    );
}

/// A polar heatmap (#208) prepares in the same number of host allocations
/// whatever its radius rows: the frame's tables are per constituent column,
/// never per cell. Sixteen angle columns of 64 rows and of 65,536 rows.
#[test]
fn polar_field_prepare_cost_is_independent_of_its_rows() {
    let Some(_measurement) = measured("polar_field_prepare_cost_is_independent_of_its_rows") else {
        return;
    };
    let Some(mut renderer) = renderer_with_pool(64 << 20) else {
        no_gpu("polar field prepare-vs-rows measurement");
        return;
    };
    let columns = 16usize;
    let (short, long) = (64usize, 65_536usize);
    for (prefix, rows) in [("s", short), ("l", long)] {
        renderer
            .add_column(
                &format!("{prefix}a"),
                &Column {
                    data: (0..=columns).map(|i| i as f32 * 22.5).collect(),
                },
            )
            .unwrap();
        renderer
            .add_column(
                &format!("{prefix}r"),
                &Column {
                    data: (0..=rows).map(|j| j as f32).collect(),
                },
            )
            .unwrap();
        for i in 0..columns {
            renderer
                .add_column(
                    &format!("{prefix}z{i}"),
                    &Column {
                        data: (0..rows).map(|j| ((i + j) % 7) as f32).collect(),
                    },
                )
                .unwrap();
        }
    }
    renderer.end_gpu_frame();

    let mut config = panel_config(320, 240);
    config.set_polar(Some(renderer::polar::PolarOptions::default()));
    let mut bar = renderer::default::default_colorbar_options();
    bar.axis.min = 0.0;
    bar.axis.max = 6.0;
    bar.axis.major_spacing = 1.0;
    config.colorbar = Some(bar);
    let mut counts = Vec::new();
    for (prefix, rows) in [("s", short), ("l", long)] {
        let mut chart = Chart::new(config.clone());
        chart.set_x_range(0.0, 360.0);
        chart.set_y_range(0.0, rows as f64);
        let mut view = renderer
            .create_chart_view(&chart, chart.config().chart_area.0)
            .unwrap();
        let series_config = renderer::SeriesConfig {
            series_id: format!("field-{rows}"),
            source_id: None,
            label: None,
            x_column: format!("{prefix}a"),
            y_column: format!("{prefix}r"),
            z_column: None,
            render_type: renderer::DataRenderType::Field(renderer::data_config::FieldSeries {
                matrix: renderer::data_config::MatrixRef {
                    columns: (0..columns).map(|i| format!("{prefix}z{i}")).collect(),
                    orientation: renderer::data_config::MatrixOrientation::ColumnsAreX,
                    grid_layout: renderer::data_config::GridLayout::Edges,
                },
                fill: Some(renderer::data_config::FieldFillConfig {
                    mode: renderer::data_config::FillMode::Continuous,
                    shading: renderer::data_config::Shading::Flat,
                    opacity: 1.0,
                }),
                contour: None,
                surface: None,
            }),
        };
        let style = renderer.create_style_for_series(&series_config).unwrap();
        let series = [renderer::Series {
            config: &series_config,
            style: &style,
        }];
        let mut items = [ChartDrawItem {
            view: &mut view,
            chart: &chart,
            series: &series,
        }];
        renderer.prepare(&mut items).unwrap();
        renderer.prepare(&mut items).unwrap();
        let before = calls();
        let prepared = renderer.prepare(&mut items).unwrap();
        let allocations = calls() - before;
        drop(prepared);
        println!("polar field prepare with {rows} rows: {allocations} allocations");
        counts.push(allocations);
    }
    let (short_count, long_count) = (counts[0], counts[1]);
    assert!(
        long_count.abs_diff(short_count) <= 4,
        "a polar field with {}× the rows changed the frame's host allocation \
         count ({short_count} → {long_count}). Frame work must be per column, \
         never per cell.",
        long / short
    );
}

#[test]
fn pool_growth_allocates_a_bounded_amount() {
    let Some(_measurement) = measured("pool_growth_allocates_a_bounded_amount") else {
        return;
    };
    let Some(mut renderer) = renderer_with_pool(64 * 1024) else {
        no_gpu("growth allocation measurement");
        return;
    };
    renderer.set_pool_growth_policy(GrowthPolicy::OnAllocFailure);
    // Warm-up upload inside the initial capacity, so the measured upload is the
    // one that grows.
    renderer
        .add_column(
            "warm",
            &Column {
                data: (0..256).map(|i| i as f32).collect(),
            },
        )
        .unwrap();
    renderer.end_gpu_frame();
    settle_gpu(&renderer);

    let values = 1 << 18;
    let column = Column {
        data: (0..values).map(|i| i as f32).collect(),
    };
    let capacity_before = renderer.pool().capacity();
    let before = calls();
    let bytes_before = reset_peak_live_bytes();
    renderer.add_column("grown", &column).unwrap();
    renderer.end_gpu_frame();
    settle_gpu(&renderer);
    let allocations = calls() - before;
    let bytes = live_bytes() - bytes_before;
    let peak_bytes = peak_live_bytes() - bytes_before;

    assert!(
        renderer.pool().capacity() > capacity_before,
        "the upload was supposed to force a growth"
    );
    println!(
        "growth upload: {allocations} allocations, {bytes} B retained, \
         {peak_bytes} B peak"
    );
    assert!(
        allocations <= MAX_GROWTH_ALLOCATIONS,
        "growing the pool made {allocations} host allocations (bound \
         {MAX_GROWTH_ALLOCATIONS}). Growth copies on the GPU; it must not walk \
         the data on the host."
    );
    let stored = (values as u64) * COLUMN_VALUE_BYTES;
    // Compared as signed: the measured window can legitimately end *below* the
    // baseline (the pool's slot map rehashes and frees its old table), and
    // casting that negative to unsigned would wrap into a false failure — it
    // did, and the pre-commit hook caught it.
    assert!(
        bytes < (stored / 8) as i64,
        "growth retained {bytes} B of host memory for {stored} B of data — that \
         looks like a copy"
    );
    assert!(
        peak_bytes < (stored / 8) as i64,
        "growth peaked at {peak_bytes} B of host memory for {stored} B of data — \
         a temporary host copy is forbidden too"
    );
}

/// Host cost of the batch upload, against the per-column path it replaces.
///
/// The batch exists because a matrix declares thousands of columns, so the
/// number that matters is per column — and it must not depend on how long the
/// columns are. Both are asserted; the printout is the record.
#[test]
fn a_batch_upload_costs_less_host_heap_per_column_than_one_at_a_time() {
    let Some(_measurement) =
        measured("a_batch_upload_costs_less_host_heap_per_column_than_one_at_a_time")
    else {
        return;
    };
    const COLUMNS: usize = 64;
    let Some(mut renderer) = renderer_with_pool(64 << 20) else {
        no_gpu("batch allocation measurement");
        return;
    };
    renderer
        .add_column(
            "warm",
            &Column {
                data: (0..1024).map(|i| i as f32).collect(),
            },
        )
        .unwrap();
    renderer.end_gpu_frame();
    settle_gpu(&renderer);

    println!("\nhost allocations for {COLUMNS} columns, singles vs one batch");
    println!(
        "{:>10} {:>12} {:>12} {:>12}",
        "values", "singles", "batch", "per column"
    );
    let mut batch_points = Vec::new();
    for values in [64usize, 4096] {
        // The caller's data is built before any baseline: it is the host's own
        // buffer, and counting it would hide whether the renderer copies it.
        let sources: Vec<Column<f32>> = (0..COLUMNS)
            .map(|c| Column {
                data: (0..values).map(|i| (i + c) as f32).collect(),
            })
            .collect();
        let single_ids: Vec<String> = (0..COLUMNS).map(|c| format!("s{values}_{c}")).collect();
        let batch_ids: Vec<String> = (0..COLUMNS).map(|c| format!("b{values}_{c}")).collect();
        let batch: Vec<(&str, &dyn ColumnSource)> = batch_ids
            .iter()
            .zip(&sources)
            .map(|(id, source)| (id.as_str(), source as &dyn ColumnSource))
            .collect();

        settle_gpu(&renderer);
        let calls_before = calls();
        for (id, source) in single_ids.iter().zip(&sources) {
            renderer.add_column(id.as_str(), source).unwrap();
        }
        renderer.end_gpu_frame();
        settle_gpu(&renderer);
        let singles = calls() - calls_before;

        settle_gpu(&renderer);
        let calls_before = calls();
        renderer.add_columns(&batch).unwrap();
        renderer.end_gpu_frame();
        settle_gpu(&renderer);
        let batched = calls() - calls_before;

        println!(
            "{:>10} {:>12} {:>12} {:>12.2}",
            values,
            singles,
            batched,
            batched as f64 / COLUMNS as f64
        );
        assert!(
            batched < singles,
            "the batch ({batched}) must cost less host heap than {COLUMNS} single \
             adds ({singles}) — that is the whole reason it exists"
        );
        assert!(
            batched as f64 / COLUMNS as f64 <= MAX_BATCH_ALLOCATIONS_PER_COLUMN,
            "batch cost {:.2} allocations per column (limit {MAX_BATCH_ALLOCATIONS_PER_COLUMN}). \
             A per-value or per-chunk host allocation would show up here.",
            batched as f64 / COLUMNS as f64
        );
        // The measured allocation count covers all columns in the batch.
        batch_points.push(((COLUMNS * values * 4) as f64, batched as f64));
    }

    // Same column count, 64× the data: the count must not follow the data.
    let (slope, dc) = least_squares(&batch_points);
    println!("  batch allocs: slope {slope:.8} per input B, DC {dc:.0}");
    let growth_per_mib = slope.max(0.0) * (STEP_INPUT_BYTES as f64);
    assert!(
        growth_per_mib <= 8.0,
        "batch allocation count grew {growth_per_mib:.2} per MiB of input (limit 8)"
    );
}

/// The categorical charts #442 measures: category bars, or range bars, on
/// a categorical x axis of `n` categories, drawn into a 1200×700 panel.
#[derive(Clone, Copy, Debug)]
enum CategoricalKind {
    Bars,
    Ranges,
}

fn categorical_chart(
    n: usize,
    kind: CategoricalKind,
) -> (renderer::Config, Vec<renderer::SeriesConfig>) {
    use renderer::config::Category;
    use renderer::data_config::{BarGroups, CategoryBarDirection, CategoryBarMode};
    use renderer::layout::{ChartArea, Rect};
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: 1200,
        height: 700,
    });
    config.bottom_x.categories = Some(
        (0..n)
            .map(|k| Category::new(format!("category-{k}"), format!("Category {k}")))
            .collect(),
    );
    config.bar_groups = Some(BarGroups {
        mode: CategoryBarMode::Grouped,
        direction: CategoryBarDirection::Vertical,
        group_width: 0.8,
        bar_gap: 0.0,
    });
    let render_type = match kind {
        CategoricalKind::Bars => {
            renderer::DataRenderType::CategoryBars(renderer::CategoryBarSeries {
                color: renderer::Color::from_rgb8(200, 40, 40),
                style: renderer::data_config::CategoryBarStyle::default(),
                labels: Default::default(),
                label_format: Default::default(),
                overrides: None,
            })
        }
        CategoricalKind::Ranges => renderer::DataRenderType::RangeBars(renderer::RangeBarSeries {
            color: renderer::Color::from_rgb8(40, 40, 200),
            style: renderer::data_config::CategoryBarStyle::default(),
            start: "start".into(),
            whiskers: None,
            color_by: renderer::data_config::RangeColor::Single,
            progress: None,
            connector: None,
            slot_width: None,
            labels: Default::default(),
        }),
    };
    let series = vec![renderer::SeriesConfig {
        series_id: "s".into(),
        source_id: None,
        label: None,
        x_column: "pos".into(),
        y_column: "value".into(),
        z_column: None,
        render_type,
    }];
    (config, series)
}

/// A renderer holding the columns of [`categorical_chart`] over `n`
/// categories, with GPU picking on.
fn categorical_renderer(n: usize) -> Renderer {
    let pool = (n as u64) * 3 * COLUMN_VALUE_BYTES + (1 << 20);
    let mut renderer = renderer_with_pool(pool).expect("renderer");
    let column = |f: &dyn Fn(usize) -> f32| Column {
        data: (0..n).map(f).collect::<Vec<f32>>(),
    };
    renderer.add_column("pos", &column(&|k| k as f32)).unwrap();
    renderer
        .add_column("value", &column(&|k| 1.0 + (k % 10) as f32))
        .unwrap();
    renderer
        .add_column("start", &column(&|k| 0.5 * (k % 3) as f32))
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    renderer
}

/// One on-screen prepare of registered chart `id`, as a host's frame runs it.
fn draw_registered(renderer: &mut Renderer, id: renderer::ChartId) {
    let (config, panel, scale) = renderer.chart_display_config(id).unwrap();
    let chart = Chart::new(config.into_owned());
    let mut view = renderer.create_chart_view_at(&chart, panel, scale).unwrap();
    renderer
        .prepare_registered(&mut [renderer::RegisteredChartDrawItem {
            chart_id: id,
            view: &mut view,
        }])
        .unwrap();
    settle_gpu(renderer);
}

/// The request of a pick at the middle of chart `id`'s data area as
/// displayed.
fn centre_pick(renderer: &Renderer, id: renderer::ChartId) -> renderer::GpuPickRequest {
    let (config, panel, scale) = renderer.chart_display_config(id).unwrap();
    let area = config.data_area().unwrap().0;
    renderer::GpuPickRequest {
        canvas_position_px: [
            area.x as f32 + area.width as f32 * 0.5,
            area.y as f32 + area.height as f32 * 0.9,
        ],
        display_panel_px: panel,
        display_scale: scale,
        max_distance_px: 4.0,
    }
}

/// The host interactions #442 measures on a categorical chart of `n`
/// categories: bytes the host heap handed out for each (frees not
/// subtracted, so a temporary copy counts) and its wall time. Each
/// interaction is measured through the renderer's calls and the resolved
/// pick; the draw that follows it is measured on its own (`draw`), since
/// its axis chrome measures the category tick labels (not a copy).
fn categorical_interaction_costs(
    n: usize,
    kind: CategoricalKind,
) -> Vec<(&'static str, u64, std::time::Duration)> {
    let mut renderer = categorical_renderer(n);
    let (config, series) = categorical_chart(n, kind);
    let id = renderer.register_chart(config, series).unwrap();
    renderer.set_chart_display(id, Some((1200, 700))).unwrap();
    // Warm: every pipeline, kernel and cache the interactions reach.
    for _ in 0..2 {
        draw_registered(&mut renderer, id);
        let request = centre_pick(&renderer, id);
        let _ = renderer
            .pick_chart(id, request)
            .unwrap()
            .resolve_full_blocking()
            .unwrap();
        let _ = renderer
            .pick_chart_data(id, request)
            .unwrap()
            .resolve_full_blocking()
            .unwrap();
    }
    let mut costs = Vec::new();
    let mut measure = |name: &'static str, renderer: &mut Renderer, f: &dyn Fn(&mut Renderer)| {
        settle_gpu(renderer);
        let before = allocated_bytes();
        let started = std::time::Instant::now();
        f(renderer);
        settle_gpu(renderer);
        let elapsed = started.elapsed();
        costs.push((name, allocated_bytes() - before, elapsed));
    };
    let request = centre_pick(&renderer, id);
    measure("hover pick", &mut renderer, &|r| {
        let _ = r
            .pick_chart(id, request)
            .unwrap()
            .resolve_full_blocking()
            .unwrap();
    });
    measure("click: typed pick and select", &mut renderer, &|r| {
        let picked = r
            .pick_chart_data(id, request)
            .unwrap()
            .resolve_full_blocking()
            .unwrap();
        let reference = picked.picked.expect("the pick lands on a bar").to_ref();
        let mut config = r.chart_config(id).unwrap().clone();
        let selections = config
            .picked_data
            .get_or_insert_with(renderer::DataSelectionsConfig::empty);
        selections.refs.clear();
        selections.refs.push(reference);
        r.set_chart_config(id, config).unwrap();
    });
    measure("step_selection", &mut renderer, &|r| {
        let mut config = r.chart_config(id).unwrap().clone();
        let moved = config
            .step_selection(r.chart_series(id).unwrap(), 1, 0, |_| Some(n))
            .unwrap();
        assert!(moved, "the selection steps to the next category");
        r.set_chart_config(id, config).unwrap();
    });
    measure("config edit (title)", &mut renderer, &|r| {
        let mut config = r.chart_config(id).unwrap().clone();
        config.chart_title.offset_x += 3.0;
        r.set_chart_config(id, config).unwrap();
    });
    measure("resize", &mut renderer, &|r| {
        r.set_chart_display(id, Some((1100, 640))).unwrap();
        let (config, panel, scale) = r.chart_display_config(id).unwrap();
        let chart = Chart::new(config.into_owned());
        std::hint::black_box((chart, panel, scale));
    });
    measure("draw", &mut renderer, &|r| draw_registered(r, id));
    costs
}

/// #442: on a categorical axis, a pick, `step_selection`, a config edit and
/// a resize copy no category. The categories are shared storage of the
/// document (SSOT), never copied by a hot path; ten times the categories
/// may cost no more host heap per pick or resize than a fixed allowance.
/// An interaction that edits the document also validates it, whose
/// duplicate-id check indexes the borrowed ids in a transient hash set
/// (#450): it may add that index, far less than a copy of the categories.
///
/// The draw is printed, not held to it: its axis chrome measures the tick
/// label of every category to thin them (`each_category_tick_label`), work
/// and scratch that grow with the categories but copy none of them.
#[test]
fn categorical_interactions_do_not_copy_the_categories() {
    let Some(_measurement) = measured("categorical_interactions_do_not_copy_the_categories") else {
        return;
    };
    /// Host heap an interaction may hand out beyond the small chart's,
    /// whatever the category count: 50 000 categories' storage is several
    /// MB, so one copy of them is far past it.
    const ALLOWANCE: u64 = 64 * 1024;
    /// What a document edit may add per category: the validation's index
    /// of borrowed ids (`validate_categories`, #450), a transient hash set
    /// of `&str` of about 22 B per category. One copy of a category (its
    /// struct and its two strings) is about 156 B, so a copy still fails.
    const ID_INDEX_PER_CATEGORY: u64 = 32;
    /// The interactions that validate the edited document.
    const EDITS: [&str; 3] = [
        "click: typed pick and select",
        "step_selection",
        "config edit (title)",
    ];
    const SMALL: usize = 5_000;
    const LARGE: usize = 50_000;
    let mut growths = Vec::new();
    for kind in [CategoricalKind::Bars, CategoricalKind::Ranges] {
        let small = categorical_interaction_costs(SMALL, kind);
        let large = categorical_interaction_costs(LARGE, kind);
        for ((name, small_bytes, small_time), (_, large_bytes, large_time)) in
            small.iter().zip(&large)
        {
            println!(
                "{kind:?} {name}: {SMALL} categories {small_bytes} B {small_time:?}, \
                 {LARGE} categories {large_bytes} B {large_time:?}"
            );
            if *name != "draw" {
                growths.push((kind, *name, *small_bytes, *large_bytes));
            }
        }
    }
    for (kind, name, small_bytes, large_bytes) in growths {
        let allowance = if EDITS.contains(&name) {
            ALLOWANCE + ID_INDEX_PER_CATEGORY * (LARGE - SMALL) as u64
        } else {
            ALLOWANCE
        };
        assert!(
            large_bytes.saturating_sub(small_bytes) <= allowance,
            "{kind:?} {name}: {LARGE} categories allocated {large_bytes} B, \
             {SMALL} allocated {small_bytes} B; the difference grows with \
             the categories"
        );
    }
}

/// A registered candlestick of `n` rows with one value label per row
/// (#443), on a renderer of its own: its chart and a view of it.
fn labelled_candles(n: usize) -> (Renderer, renderer::ChartId, renderer::ChartView, Chart) {
    let mut renderer = renderer_with_pool(16 << 20).expect("required GPU test: renderer");
    let column = |data: Vec<f32>| Column { data };
    renderer
        .add_column("pos", &column((0..n).map(|i| i as f32).collect()))
        .unwrap();
    renderer.add_column("open", &column(vec![20.0; n])).unwrap();
    renderer
        .add_column("close", &column(vec![60.0; n]))
        .unwrap();
    renderer.end_gpu_frame();
    let mut config = panel_config(480, 360);
    config.legend.visible = false;
    config.bottom_x.min = -1.0;
    config.bottom_x.max = n as f64;
    config.left_y.min = 0.0;
    config.left_y.max = 100.0;
    let series = renderer::SeriesConfig {
        series_id: "candles".into(),
        source_id: None,
        label: None,
        x_column: "pos".into(),
        y_column: "close".into(),
        z_column: None,
        render_type: renderer::DataRenderType::RangeBars(renderer::RangeBarSeries {
            color: renderer::Color::new(0.2, 0.3, 0.8, 1.0),
            style: Default::default(),
            start: "open".into(),
            whiskers: None,
            color_by: Default::default(),
            progress: None,
            connector: None,
            slot_width: Some(0.8),
            labels: renderer::data_config::CategoryBarSeriesLabels {
                placement: renderer::data_config::CategoryBarLabels::Outside,
                text: (0..n).map(|i| (i % 1000).to_string()).collect(),
                ..Default::default()
            },
        }),
    };
    let id = renderer
        .register_chart(config.clone(), vec![series])
        .unwrap();
    renderer.enable_gpu_picking().unwrap();
    let chart = Chart::new(config);
    let view = renderer
        .create_chart_view(&chart, chart.config().chart_area.0)
        .unwrap();
    (renderer, id, view, chart)
}

/// The host bytes and allocations one step of a labelled chart costs at
/// its peak: an idle frame, a pan (a view edit and its frame) and a pick at
/// display scale 1.5.
fn labelled_step_costs(n: usize) -> [(i64, u64); 3] {
    let (mut renderer, id, mut view, chart) = labelled_candles(n);
    let draw = |renderer: &mut Renderer, view: &mut renderer::ChartView| {
        let frame = renderer
            .prepare_registered(&mut [renderer::RegisteredChartDrawItem { chart_id: id, view }])
            .unwrap();
        drop(frame);
        settle_gpu(renderer);
    };
    let rect = chart.config().chart_area.0;
    let request = renderer::GpuPickRequest {
        canvas_position_px: [rect.width as f32 * 0.75, rect.height as f32 * 0.75],
        display_panel_px: renderer::layout::Rect {
            x: 0,
            y: 0,
            width: rect.width * 3 / 2,
            height: rect.height * 3 / 2,
        },
        display_scale: 1.5,
        max_distance_px: 4.0,
    };
    let pick = |renderer: &mut Renderer| {
        let ticket = renderer.pick_chart_data(id, request).unwrap();
        let _ = test_gpu::block_on(ticket.resolve());
        settle_gpu(renderer);
    };
    let pan = |renderer: &mut Renderer, view: &mut renderer::ChartView, by: f64| {
        let mut state = renderer.chart_view_state(id).unwrap();
        state.bottom_x.min += by;
        state.bottom_x.max += by;
        renderer.set_chart_view_state(id, state).unwrap();
        draw(renderer, view);
    };
    // Warm every step: the first of each bakes, compiles and fills caches.
    for _ in 0..2 {
        draw(&mut renderer, &mut view);
        pick(&mut renderer);
        pan(&mut renderer, &mut view, 1.0);
        pan(&mut renderer, &mut view, -1.0);
    }
    let mut measure = |step: &mut dyn FnMut(&mut Renderer, &mut renderer::ChartView)| {
        let start = reset_peak_live_bytes();
        let before = calls();
        step(&mut renderer, &mut view);
        (peak_live_bytes() - start, calls() - before)
    };
    let edit = measure(&mut |renderer, _| {
        let mut state = renderer.chart_view_state(id).unwrap();
        state.bottom_x.min += 0.5;
        state.bottom_x.max += 0.5;
        renderer.set_chart_view_state(id, state).unwrap();
    });
    let after_edit = measure(&mut |renderer, view| draw(renderer, view));
    println!("DIAG {n}: edit {edit:?} frame after edit {after_edit:?}");
    let idle = measure(&mut |renderer, view| draw(renderer, view));
    let picked = measure(&mut |renderer, _| pick(renderer));
    let panned = measure(&mut |renderer, view| pan(renderer, view, 2.0));
    [idle, picked, panned]
}

/// Value labels cost a frame, a pick or a pan nothing that grows with their
/// count (#443): the atlas, the 16 B cell per label and its bind group are
/// kept across them, and no label string is copied. 5k and 50k labels peak
/// at the same host bytes, within a margin far below one byte per label.
#[test]
fn value_label_frames_picks_and_pans_do_not_allocate_per_label() {
    let Some(_measurement) =
        measured("value_label_frames_picks_and_pans_do_not_allocate_per_label")
    else {
        return;
    };
    let small = labelled_step_costs(5_000);
    let large = labelled_step_costs(50_000);
    for (step, (small, large)) in ["idle frame", "pick", "pan"]
        .iter()
        .zip(small.iter().zip(large))
    {
        println!(
            "{step}: 5k labels peak {} B in {} allocations, 50k labels peak {} B in {} allocations",
            small.0, small.1, large.0, large.1
        );
        assert!(
            large.0 <= small.0 + 16 * 1024,
            "{step}: 45k more labels raised the peak host bytes from {} to {}",
            small.0,
            large.0
        );
        assert!(
            large.1.abs_diff(small.1) <= 8,
            "{step}: 45k more labels changed the allocation count from {} to {}",
            small.1,
            large.1
        );
    }
}
