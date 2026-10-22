//! Tier 2 of #46: leaf mutations against [`SeriesExtentKey`]. The web host
//! keeps one GPU fit-extent job per key and hands a later series with an
//! equal key the job computed for the first one, so two series
//! declarations with equal keys must have equal extents.
//!
//! For every leaf of each `SeriesConfig` in the fixture: take the key and
//! the renderer's extent (`Renderer::begin_series_fit_extent`, what the job
//! resolves to) before and after the edit. Equal keys with different
//! extents are a key gap.
//!
//! The walker is the renderer crate's test-support module, compiled here
//! from its source file.
#[path = "../../renderer/src/renderer/config_mutation.rs"]
mod config_mutation;

use std::collections::HashMap;
use std::sync::Arc;

use renderer::data_config::{
    FieldFillConfig, FillMode, GridLayout, MatrixOrientation, MatrixRef, Shading, VectorComponents,
    VectorPivot, VectorScale, VectorStyle,
};
use renderer::{
    Color, DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType, DataScatterStyleConfig,
    ErrorRef, GpuSeriesExtent, Renderer, RendererDevice, ScatterShape, SeriesConfig,
};

use self::config_mutation::{Outcome, Tally, run};
use super::series_extent_request_from;

const POINTS: usize = 16;
const GRID_X: usize = 4;
const GRID_Y: usize = 3;

/// Column ids of each shape; valid replacements for the string leaves.
const STRINGS: &[&str] = &["x", "y", "e", "u", "v", "gx", "gy", "z1", "vg1"];

/// Drive `future` to completion on this thread, polling the device so the
/// readback it waits for is mapped. The web crate has no blocking executor
/// of its own; this is the native stand-in for the browser's event loop.
fn block_on<F: std::future::Future>(device: &wgpu::Device, future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }
}

fn column(values: impl Iterator<Item = f64>) -> renderer::Column<f64> {
    let data: Vec<f64> = values.collect();
    let min = data.iter().copied().fold(f64::INFINITY, f64::min);
    let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    renderer::Column { data, min, max }
}

fn fixture() -> (Renderer, Arc<wgpu::Device>) {
    let instance = renderer::data_render::create_instance();
    let adapter = renderer::data_render::request_adapter(&instance).expect("GPU adapter");
    let (device, queue) = renderer::data_render::request_device(&adapter).expect("GPU device");
    let device = Arc::new(device);
    let mut r = Renderer::try_new(
        RendererDevice::new(Arc::clone(&device), Arc::new(queue)),
        wgpu::TextureFormat::Rgba8Unorm,
        1 << 20,
    )
    .unwrap();
    let ramp = |scale: f64, offset: f64| {
        column((0..POINTS).map(move |i| offset + scale * i as f64 / (POINTS - 1) as f64))
    };
    let wave = |phase: f64, scale: f64| {
        column((0..POINTS).map(move |i| scale * (i as f64 * 0.7 + phase).sin()))
    };
    r.add_column("x", &ramp(1.0, 0.0)).unwrap();
    r.add_column("y", &wave(0.0, 1.0)).unwrap();
    r.add_column("e", &ramp(0.2, 0.05)).unwrap();
    r.add_column("u", &wave(0.5, 0.3)).unwrap();
    r.add_column("v", &wave(1.7, 0.3)).unwrap();
    r.add_column("gx", &column((0..GRID_X).map(|i| i as f64)))
        .unwrap();
    r.add_column("gy", &column((0..GRID_Y).map(|i| i as f64 * 2.0)))
        .unwrap();
    for i in 0..GRID_X {
        let values = (0..GRID_Y).map(move |j| (i * GRID_Y + j) as f64 * 0.1);
        r.add_column(&format!("z{i}"), &column(values)).unwrap();
        let values = (0..GRID_Y).map(move |j| 0.2 + (i + j) as f64 * 0.05);
        r.add_column(&format!("ug{i}"), &column(values)).unwrap();
        let values = (0..GRID_Y).map(move |j| 0.1 * (j as f64 - i as f64));
        r.add_column(&format!("vg{i}"), &column(values)).unwrap();
    }
    block_on(&device, r.ensure_errorbar_extent_engine()).unwrap();
    (r, device)
}

fn scatter() -> DataScatterStyleConfig {
    DataScatterStyleConfig {
        point_color: Color::BLACK,
        point_shape: ScatterShape::Circle,
        point_size: 4.0,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
        point_color_map: None,
        point_size_map: None,
    }
}

fn grid(prefix: &str) -> MatrixRef {
    MatrixRef {
        columns: (0..GRID_X).map(|i| format!("{prefix}{i}")).collect(),
        orientation: MatrixOrientation::ColumnsAreX,
        grid_layout: GridLayout::Centers,
    }
}

fn arrows(scale: VectorScale) -> VectorStyle {
    VectorStyle {
        color: Color::BLACK,
        width_px: 1.0,
        scale,
        pivot: VectorPivot::Tail,
        head_length_px: 4.0,
        head_width_px: 4.0,
        min_length_px: 0.0,
        color_map: None,
    }
}

fn declared(id: &str, x: &str, y: &str, render_type: DataRenderType) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        render_type,
    }
}

/// One series of every GPU extent mode the key distinguishes: paired
/// points, a line with error bars in both directions, a field lattice, and
/// the two vector forms (data-scaled tips, a pixel-scaled grid).
fn series() -> Vec<SeriesConfig> {
    vec![
        declared(
            "points",
            "x",
            "y",
            DataRenderType::Scatter { scatter: scatter() },
        ),
        declared(
            "errors",
            "x",
            "y",
            DataRenderType::LineScatterErrorbarXY {
                scatter: scatter(),
                line: DataLineStyleConfig {
                    line_style: renderer::line::LineStylePreset::Solid,
                    line_color: Color::BLACK,
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: renderer::data_config::LineInterpolation::Linear,
                },
                err_x: ErrorRef::Symmetric { column: "e".into() },
                err_y: ErrorRef::Asymmetric {
                    lower: "e".into(),
                    upper: "u".into(),
                },
                err_style: DataErrorBarStyleConfig {
                    error_bar_color: Color::BLACK,
                    error_bar_width: 1.0,
                    error_bar_cap_size: 2.0,
                    cap_width: 1.0,
                    error_bar_style_table: None,
                    error_bar_style_index_column: None,
                    error_bar_style_overrides: None,
                },
            },
        ),
        declared(
            "field",
            "gx",
            "gy",
            DataRenderType::Heatmap {
                matrix: grid("z"),
                fill: FieldFillConfig {
                    mode: FillMode::Continuous,
                    shading: Shading::Flat,
                    opacity: 1.0,
                },
            },
        ),
        declared(
            "tips",
            "x",
            "y",
            DataRenderType::Vector {
                components: VectorComponents::Points {
                    u: "u".into(),
                    v: "v".into(),
                },
                style: arrows(VectorScale::Data { factor: 1.0 }),
            },
        ),
        declared(
            "quiver",
            "gx",
            "gy",
            DataRenderType::Vector {
                components: VectorComponents::Grid {
                    u: grid("ug"),
                    v: grid("vg"),
                },
                style: arrows(VectorScale::Pixels { px_per_unit: 10.0 }),
            },
        ),
    ]
}

/// What the extent job of `series` resolves to: x min, x max, x smallest
/// positive, then the same for y (NaN for no positive value).
fn extent(r: &Renderer, device: &wgpu::Device, series: &SeriesConfig) -> Option<[f64; 6]> {
    let ticket = r.begin_series_fit_extent(series).ok()??;
    let GpuSeriesExtent { x, y } = block_on(device, ticket.resolve()).ok()??;
    let positive = |value: Option<f64>| value.unwrap_or(f64::NAN);
    Some([
        x.min,
        x.max,
        positive(x.min_positive),
        y.min,
        y.max,
        positive(y.min_positive),
    ])
}

fn same(a: Option<[f64; 6]>, b: Option<[f64; 6]>) -> bool {
    a.map(|a| a.map(f64::to_bits)) == b.map(|b| b.map(f64::to_bits))
}

#[test]
fn every_series_leaf_that_changes_the_extent_changes_the_extent_key() {
    let (mut r, device) = fixture();
    let series = series();
    let mut config = renderer::default::default_config();
    config.colorbar = Some(renderer::default::default_colorbar_options());
    let id = r.register_chart(config, series.clone()).unwrap();
    // Every registered column at one live revision.
    let revisions: HashMap<String, u64> = ["x", "y", "e", "u", "v", "gx", "gy"]
        .into_iter()
        .map(str::to_string)
        .chain((0..GRID_X).flat_map(|i| [format!("z{i}"), format!("ug{i}"), format!("vg{i}")]))
        .map(|id| (id, 1))
        .collect();
    let mut tally = Tally::default();
    for (index, base) in series.iter().enumerate() {
        let key = series_extent_request_from(&revisions, base)
            .expect("every fixture series has a GPU extent key")
            .key;
        let before = extent(&r, &device, base).expect("every fixture series has an extent");
        let label = format!("series[{index}] ({}): ", base.series_id);
        let part = run(&label, base, STRINGS, |_, next| {
            let mut list = series.clone();
            list[index] = next.clone();
            if r.set_chart_series(id, list).is_err() {
                return Outcome::Rejected;
            }
            let outcome = match series_extent_request_from(&revisions, &next) {
                // No GPU job (a histogram's metadata fit).
                None => Outcome::Held,
                Some(request) if request.key != key => Outcome::Held,
                Some(_) => {
                    let after = extent(&r, &device, &next);
                    if same(after, Some(before)) {
                        Outcome::Held
                    } else {
                        Outcome::Gap(format!(
                            "the key stayed equal and the extent [x min, x max, x min positive, \
                             y min, y max, y min positive] changed from {before:?} to {after:?}"
                        ))
                    }
                }
            };
            r.set_chart_series(id, series.clone()).unwrap();
            outcome
        });
        eprintln!(
            "{}",
            part.summary(&format!("extent key/{}", base.series_id))
        );
        tally.absorb(part);
    }
    eprintln!("{}", tally.summary("extent key (all)"));
    eprintln!("skipped:\n{}", tally.skipped.join("\n"));
    assert!(
        tally.gaps.is_empty(),
        "{} series leaves change the fit extent without changing SeriesExtentKey:\n{}",
        tally.gaps.len(),
        tally.gaps.join("\n")
    );
}
