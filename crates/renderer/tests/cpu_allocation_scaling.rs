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
//!     cargo test -p renderer --test cpu_allocation_scaling -- --nocapture

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

fn measurement_guard() -> MutexGuard<'static, ()> {
    MEASUREMENT_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn no_gpu(context: &str) {
    if std::env::var_os("FIGGY_REQUIRE_GPU_TESTS").is_some() {
        panic!("{context}: no GPU adapter while FIGGY_REQUIRE_GPU_TESTS is set");
    }
    eprintln!("no GPU adapter; skipping {context}");
}

fn settle_gpu(renderer: &Renderer) {
    pollster::block_on(renderer.wait_submitted_work());
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
    use renderer::data_render::{create_instance, request_adapter, request_device};
    use std::sync::Arc;
    let instance = create_instance();
    let adapter = request_adapter(&instance).ok()?;
    let (device, queue) = request_device(&adapter).ok()?;
    Renderer::try_new(
        RendererDevice::new(Arc::new(device), Arc::new(queue)),
        wgpu::TextureFormat::Rgba8Unorm,
        pool_bytes,
    )
    .ok()
}

#[test]
fn host_bytes_do_not_scale_with_uploaded_data() {
    let _measurement = measurement_guard();
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
                min: 0.0f32,
                max: 1023.0,
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
            min: 0.0f32,
            max: (values - 1) as f32,
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
/// Measured 8–9 on llvmpipe. Figgy's own share is the two
/// `Vec::with_capacity(items.len())` its two prepare phases build; the rest is
/// wgpu's own bookkeeping inside `queue.write_buffer` for the transform
/// uniform — label strings and pending-write vectors in wgpu-core.
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
    let _measurement = measurement_guard();
    let Some(mut renderer) = renderer_with_pool(4 << 20) else {
        no_gpu("prepare allocation measurement");
        return;
    };
    let chart = Chart::new(panel_config(320, 240));
    let views: Vec<_> = (0..4)
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
        let items: Vec<ChartDrawItem<'_>> = views
            .iter()
            .take(panels)
            .map(|view| ChartDrawItem {
                view,
                chart_config: chart.config(),
                series: &[],
            })
            .collect();
        // Warm this exact shape: a shape's first prepare compiles pipeline
        // variants and fills caches, which is not per-frame cost.
        renderer.prepare(&items).unwrap();

        let calls_before = calls();
        let prepared = renderer.prepare(&items).unwrap();
        let allocations = calls() - calls_before;
        drop(prepared);

        // Retained is measured as a **steady-state** diagnostic, not a one-shot
        // ownership bound. A one-shot delta cannot distinguish a leak from a
        // driver cache filling for the first time.
        let cycle = |renderer: &mut Renderer, rounds: usize| -> i64 {
            let before = live_bytes();
            for _ in 0..rounds {
                let prepared = renderer.prepare(&items).unwrap();
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
    let _measurement = measurement_guard();
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
                    min: 0.0f32,
                    max: (len - 1) as f32,
                    data: (0..len).map(|i| i as f32).collect(),
                },
            )
            .unwrap();
    }
    renderer.end_gpu_frame();

    let chart = Chart::new(panel_config(320, 240));
    let view = renderer
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
            render_type: renderer::DataRenderType::Line {
                line: renderer::DataLineStyleConfig {
                    line_style: renderer::line::LineStylePreset::Solid,
                    line_color: renderer::Color::new(0.1, 0.2, 0.3, 1.0),
                    line_width: 1.0,
                },
            },
        };
        let style = renderer.create_style_for_series(&series_config);
        let series = [renderer::Series {
            config: &series_config,
            style: &style,
        }];
        let items = [ChartDrawItem {
            view: &view,
            chart_config: chart.config(),
            series: &series,
        }];
        // Two warm-ups: the first prepare of a series compiles what it needs
        // and builds its arc scratch; the second settles the cache.
        renderer.prepare(&items).unwrap();
        renderer.prepare(&items).unwrap();

        let before = calls();
        let prepared = renderer.prepare(&items).unwrap();
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

#[test]
fn pool_growth_allocates_a_bounded_amount() {
    let _measurement = measurement_guard();
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
                min: 0.0f32,
                max: 255.0,
                data: (0..256).map(|i| i as f32).collect(),
            },
        )
        .unwrap();
    renderer.end_gpu_frame();
    settle_gpu(&renderer);

    let values = 1 << 18;
    let column = Column {
        min: 0.0f32,
        max: (values - 1) as f32,
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
    let _measurement = measurement_guard();
    const COLUMNS: usize = 64;
    let Some(mut renderer) = renderer_with_pool(64 << 20) else {
        no_gpu("batch allocation measurement");
        return;
    };
    renderer
        .add_column(
            "warm",
            &Column {
                min: 0.0f32,
                max: 1023.0,
                data: (0..1024).map(|i| i as f32).collect(),
            },
        )
        .unwrap();
    renderer.end_gpu_frame();

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
                min: 0.0f32,
                max: (values - 1) as f32,
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

        let calls_before = calls();
        for (id, source) in single_ids.iter().zip(&sources) {
            renderer.add_column(id.as_str(), source).unwrap();
        }
        renderer.end_gpu_frame();
        let singles = calls() - calls_before;

        let calls_before = calls();
        renderer.add_columns(&batch).unwrap();
        renderer.end_gpu_frame();
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
        batch_points.push(((values * 4) as f64, batched as f64));
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
