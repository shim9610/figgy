//! Memory-scaling measurement: does occupancy grow with the data, and only
//! with the data?
//!
//! The axis is **host input bytes** — `N` f32 values means `x = 4N`. Measuring
//! against pool bytes instead would make the slope 1.0 by construction and
//! throw away the only interesting fact: how many bytes one input byte costs.
//!
//! | source | one input value | stored | expected slope |
//! |---|---|---|---|
//! | f32 scalar | 4 B | 8 B `(v, 0)` | **2.0** |
//! | f64 scalar | 8 B | 8 B `(v as f32, 0)` | 1.0 |
//! | f64 hi/lo  | 8 B | 8 B `(hi, lo)`     | 1.0 |
//!
//! f32 costing double is the direct consequence of storing every value as a
//! `(hi, lo)` pair (`COLUMN_VALUE_BYTES = 8`), not a copy. The test's job is to
//! prove the ratio *stays* at 2.0: a 3.0 or 4.0 means a copy appeared
//! somewhere, and that is what this gate exists to catch.
//!
//! The host side is measured in `cpu_allocation_scaling.rs` instead of here: a
//! counting global allocator is process-global, and the tests in one binary run
//! in parallel, so it would count their allocations as well. RSS is printed
//! here as a cross-check but never asserted — on a software adapter GPU buffers
//! *are* host pages, so RSS tracks the pool rather than the host side, and the
//! number means something different on every platform.
//!
//! Run:
//!     cargo test -p renderer --test memory_scaling -- --nocapture
//!     cargo test -p renderer --test memory_scaling -- --ignored   (large scale)

use renderer::{Column, GpuResourceKind, GrowthPolicy, Renderer, RendererDevice};

/// Resident set size in bytes, for information only.
fn rss_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}

// Least squares over the sampled points.

#[derive(Debug, Clone, Copy)]
struct Fit {
    slope: f64,
    intercept: f64,
    r_squared: f64,
    worst_relative_residual: f64,
}

fn fit(points: &[(f64, f64)]) -> Fit {
    let n = points.len() as f64;
    let mean_x = points.iter().map(|(x, _)| x).sum::<f64>() / n;
    let mean_y = points.iter().map(|(_, y)| y).sum::<f64>() / n;
    let sxx: f64 = points.iter().map(|(x, _)| (x - mean_x).powi(2)).sum();
    let sxy: f64 = points
        .iter()
        .map(|(x, y)| (x - mean_x) * (y - mean_y))
        .sum();
    let slope = if sxx == 0.0 { 0.0 } else { sxy / sxx };
    let intercept = mean_y - slope * mean_x;
    let ss_tot: f64 = points.iter().map(|(_, y)| (y - mean_y).powi(2)).sum();
    let ss_res: f64 = points
        .iter()
        .map(|(x, y)| (y - (slope * x + intercept)).powi(2))
        .sum();
    let r_squared = if ss_tot == 0.0 {
        1.0
    } else {
        1.0 - ss_res / ss_tot
    };
    let worst_relative_residual = points
        .iter()
        .map(|(x, y)| {
            let predicted = slope * x + intercept;
            if predicted == 0.0 {
                0.0
            } else {
                ((y - predicted) / predicted).abs()
            }
        })
        .fold(0.0f64, f64::max);
    Fit {
        slope,
        intercept,
        r_squared,
        worst_relative_residual,
    }
}

// Scenario harness.

fn shared_device() -> Option<(std::sync::Arc<wgpu::Device>, std::sync::Arc<wgpu::Queue>)> {
    use renderer::data_render::{create_instance, request_adapter, request_device};
    use std::sync::{Arc, OnceLock};
    static DEVICE: OnceLock<Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)>> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            let instance = create_instance();
            let adapter = request_adapter(&instance).ok()?;
            let (device, queue) = request_device(&adapter).ok()?;
            Some((Arc::new(device), Arc::new(queue)))
        })
        .as_ref()
        .map(|(device, queue)| (std::sync::Arc::clone(device), std::sync::Arc::clone(queue)))
}

fn renderer_with_pool(pool_bytes: u64) -> Option<Renderer> {
    let (device, queue) = shared_device()?;
    Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        pool_bytes,
    )
    .ok()
}

const COLUMN_VALUE_BYTES: u64 = 8;
/// Smallest scale that still separates signal from the fixed cost.
const SMALL_INPUT_BYTES: u64 = 1 << 20;
const SMALL_STEPS: &[u64] = &[1, 2, 4, 8];
/// Large scale, sized to fit a 128 MiB storage-binding ceiling: f32 input costs
/// two bytes per byte, so 12·24·36·48 MiB of input stores 24·48·72·96 MiB — the
/// largest still under the cap with room for the fixed cost. The previous
/// 8 MiB × [1,2,4,8] asked for 128 MiB of stored bytes, exactly the cap, which
/// the pool refused; with the skip-on-failure bug above that looked like a pass.
const LARGE_INPUT_BYTES: u64 = 12 << 20;
const LARGE_STEPS: &[u64] = &[1, 2, 3, 4];

#[derive(Debug)]
struct Sample {
    input_bytes: u64,
    gpu_settled: u64,
    gpu_peak: u64,
    pool_stored: u64,
    rss_delta: i64,
}

/// One scenario: build a renderer, upload `values` f32s as a single column,
/// then read every instrument once the submission boundary has passed.
///
/// The host's input buffer is built *before* the CPU baseline is taken. It is
/// the caller's data, not the renderer's, and counting it would hide the very
/// thing being measured — whether the renderer copies it.
fn sample_f32(values: usize, grow_from_small_pool: bool) -> Option<Sample> {
    let input_bytes = (values as u64) * 4;
    let stored_bytes = (values as u64) * COLUMN_VALUE_BYTES;
    let data: Vec<f32> = (0..values).map(|i| i as f32).collect();
    let column = Column {
        min: 0.0f32,
        max: (values.saturating_sub(1)) as f32,
        data,
    };

    // "Before": no growth, so the host must size the pool for the data up
    // front — exactly how a 0.9 host works. "After": a small pool that grows.
    let initial_pool = if grow_from_small_pool {
        64 * 1024
    } else {
        stored_bytes + 64 * 1024
    };
    let mut renderer = renderer_with_pool(initial_pool)?;
    if grow_from_small_pool {
        renderer.set_pool_growth_policy(GrowthPolicy::OnAllocFailure);
    }
    renderer.end_gpu_frame();

    let rss_before = rss_bytes().unwrap_or(0) as i64;

    renderer.add_column("scale", &column).unwrap_or_else(|e| {
        panic!(
            "uploading {input_bytes} B of input into a {initial_pool} B pool failed: \
             {e:?}\nA failed upload is a result, not a reason to skip: returning \
             `None` here would be indistinguishable from having no adapter, which \
             is how the large-scale curve passed vacuously when its pool did not \
             fit the device ceiling."
        )
    });
    renderer.end_gpu_frame();

    pollster::block_on(renderer.wait_submitted_work());

    let usage = renderer.gpu_memory_usage();
    let sample = Sample {
        input_bytes,
        gpu_settled: usage.total_bytes(),
        gpu_peak: usage.peak_bytes(),
        pool_stored: renderer.pool().used_bytes(),
        rss_delta: rss_bytes().unwrap_or(0) as i64 - rss_before,
    };
    assert_eq!(
        sample.pool_stored, stored_bytes,
        "the pool must hold exactly one (hi, lo) pair per value"
    );
    Some(sample)
}

fn print_curve(title: &str, samples: &[Sample], fit: Fit) {
    println!("\n{title}");
    println!(
        "{:>12} {:>12} {:>12} {:>12} {:>12}",
        "input B", "gpu settled", "gpu peak", "pool stored", "rss ΔB"
    );
    for s in samples {
        println!(
            "{:>12} {:>12} {:>12} {:>12} {:>12}",
            s.input_bytes, s.gpu_settled, s.gpu_peak, s.pool_stored, s.rss_delta
        );
    }
    println!(
        "  fit: slope {:.4}  DC {:.0} B  R² {:.6}  worst residual {:.2}%",
        fit.slope,
        fit.intercept,
        fit.r_squared,
        fit.worst_relative_residual * 100.0
    );
}

fn run_curve(
    title: &str,
    step_bytes: u64,
    steps: &[u64],
    grow: bool,
) -> Option<(Vec<Sample>, Fit)> {
    let mut samples = Vec::new();
    for step in steps {
        let values = ((step * step_bytes) / 4) as usize;
        samples.push(sample_f32(values, grow)?);
    }
    let points: Vec<(f64, f64)> = samples
        .iter()
        .map(|s| (s.input_bytes as f64, s.gpu_settled as f64))
        .collect();
    let f = fit(&points);
    print_curve(title, &samples, f);
    Some((samples, f))
}

fn assert_curve(label: &str, samples: &[Sample], f: Fit, expected_slope: f64) {
    assert!(
        (f.slope - expected_slope).abs() / expected_slope <= 0.15,
        "{label}: slope {:.4} is more than 15% off the expected {expected_slope:.2}. \
         A slope above expectation means a copy of the data appeared; below means \
         the measurement is not seeing the data at all.",
        f.slope
    );
    assert!(
        f.r_squared >= 0.99,
        "{label}: R² {:.6} < 0.99 — occupancy is not linear in input bytes, so \
         something scales super- or sub-linearly with the data",
        f.r_squared
    );
    assert!(
        f.worst_relative_residual <= 0.05,
        "{label}: a point sits {:.2}% off the fit (limit 5%)",
        f.worst_relative_residual * 100.0
    );
    let largest = samples.last().expect("at least one sample");
    assert!(
        f.intercept >= 0.0 && (f.intercept as u64) < largest.gpu_settled / 2,
        "{label}: DC offset {:.0} B is not small against the largest sample \
         ({} B) — the fixed cost dominates and the curve says little",
        f.intercept,
        largest.gpu_settled
    );
}

#[test]
fn gpu_occupancy_is_linear_in_input_bytes_at_two_bytes_per_f32_byte() {
    let Some((samples, f)) = run_curve(
        "f32 input, pool sized up front (0.9 behavior)",
        SMALL_INPUT_BYTES,
        SMALL_STEPS,
        false,
    ) else {
        eprintln!("no GPU adapter; skipping memory scaling measurement");
        return;
    };
    assert_curve("sized-up-front", &samples, f, 2.0);
}

#[test]
fn growth_from_a_small_pool_lands_on_the_same_slope() {
    let Some((samples, f)) = run_curve(
        "f32 input, small pool grown on demand (after)",
        SMALL_INPUT_BYTES,
        SMALL_STEPS,
        true,
    ) else {
        eprintln!("no GPU adapter; skipping memory scaling measurement");
        return;
    };
    // Same slope as sizing up front: growing to fit must not cost bytes per
    // value, only a one-off relayout.
    assert_curve("grown-on-demand", &samples, f, 2.0);
}

/// f64 costs one stored byte per input byte: the value narrows to f32 in the
/// hi lane and the lo lane carries the residual, so the pair is the same 8 B
/// the f32 path uses for half the input.
#[test]
fn f64_input_costs_one_stored_byte_per_input_byte() {
    let mut samples = Vec::new();
    for step in SMALL_STEPS {
        let values = ((step * SMALL_INPUT_BYTES) / 8) as usize;
        let stored = (values as u64) * COLUMN_VALUE_BYTES;
        let Some(mut renderer) = renderer_with_pool(stored + 64 * 1024) else {
            eprintln!("no GPU adapter; skipping f64 scaling measurement");
            return;
        };
        renderer.end_gpu_frame();
        let column = Column {
            min: 0.0f64,
            max: (values - 1) as f64,
            data: (0..values).map(|i| i as f64).collect(),
        };
        let rss_before = rss_bytes().unwrap_or(0) as i64;
        renderer.add_column("scale", &column).unwrap();
        renderer.end_gpu_frame();
        pollster::block_on(renderer.wait_submitted_work());
        let usage = renderer.gpu_memory_usage();
        assert_eq!(renderer.pool().used_bytes(), stored);
        samples.push(Sample {
            input_bytes: (values as u64) * 8,
            gpu_settled: usage.total_bytes(),
            gpu_peak: usage.peak_bytes(),
            pool_stored: renderer.pool().used_bytes(),
            rss_delta: rss_bytes().unwrap_or(0) as i64 - rss_before,
        });
    }
    let points: Vec<(f64, f64)> = samples
        .iter()
        .map(|s| (s.input_bytes as f64, s.gpu_settled as f64))
        .collect();
    let f = fit(&points);
    print_curve("f64 input, pool sized up front", &samples, f);
    assert_curve("f64", &samples, f, 1.0);
}

#[test]
fn upload_peak_stays_within_settled_plus_one_staging_buffer() {
    let values = (SMALL_INPUT_BYTES / 4) as usize;
    let Some(sample) = sample_f32(values, false) else {
        eprintln!("no GPU adapter; skipping peak measurement");
        return;
    };
    let staging = sample.pool_stored;
    assert!(
        sample.gpu_peak <= sample.gpu_settled + staging,
        "peak {} B exceeds settled {} B + one staging buffer {} B",
        sample.gpu_peak,
        sample.gpu_settled,
        staging
    );
    assert!(
        sample.gpu_peak >= sample.gpu_settled,
        "peak {} B below settled {} B — the peak is not being recorded",
        sample.gpu_peak,
        sample.gpu_settled
    );
}

#[test]
fn releasing_the_data_returns_the_pool_to_empty() {
    let values = (SMALL_INPUT_BYTES / 4) as usize;
    let data: Vec<f32> = (0..values).map(|i| i as f32).collect();
    let column = Column {
        min: 0.0f32,
        max: (values - 1) as f32,
        data,
    };
    let stored = (values as u64) * COLUMN_VALUE_BYTES;
    let Some(mut renderer) = renderer_with_pool(stored + 64 * 1024) else {
        eprintln!("no GPU adapter; skipping release measurement");
        return;
    };
    renderer.end_gpu_frame();
    pollster::block_on(renderer.wait_submitted_work());
    let baseline = renderer.gpu_memory_usage();
    let dc = baseline.total_bytes();

    renderer.add_column("released", &column).unwrap();
    renderer.end_gpu_frame();
    pollster::block_on(renderer.wait_submitted_work());
    assert_eq!(renderer.pool().used_bytes(), stored);

    assert!(renderer.remove_column("released").unwrap());
    renderer.end_gpu_frame();
    pollster::block_on(renderer.wait_submitted_work());
    let after = renderer.gpu_memory_usage();
    assert_eq!(
        renderer.pool().used_bytes(),
        0,
        "every stored byte must be free again"
    );
    assert_eq!(
        after.total_bytes(),
        dc,
        "the slab itself is not released — the pool never shrinks — so the total \
         must return to exactly the fixed cost it started at, with nothing extra \
         left charged\n{}",
        after.report()
    );
    assert_eq!(after.retired_bytes(), 0, "nothing should still be retired");
    for kind in GpuResourceKind::ALL {
        assert_eq!(
            after.live_bytes_of(kind),
            baseline.live_bytes_of(kind),
            "{} bytes did not return to the fixed cost across an upload/release \
             cycle\n{}",
            kind.label(),
            after.report()
        );
    }
}

#[test]
#[ignore = "large scale: minutes on a software adapter, run manually"]
fn large_scale_curve() {
    let Some((samples, f)) = run_curve(
        "f32 input, large scale, pool sized up front",
        LARGE_INPUT_BYTES,
        LARGE_STEPS,
        false,
    ) else {
        eprintln!("no GPU adapter; skipping large-scale measurement");
        return;
    };
    assert_curve("large-scale", &samples, f, 2.0);
}
