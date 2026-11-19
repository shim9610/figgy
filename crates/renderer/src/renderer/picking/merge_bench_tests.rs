//! MEASUREMENT ONLY (#363 variant-merge study): never merged. Times a pick
//! with the series' specialized exact pipeline against the same pick with
//! extra `PICK_VARIANT` bits ORed in (uniform runtime branches: every bit's
//! path is also guarded by uniform data, so the result must not change).
//! `FIGGY_MERGE_N` points, `FIGGY_MERGE_RUNS` timed picks per variant.

use super::*;
use crate::config::{DrawStyle, SketchOptions};
use crate::data::Column;
use crate::data_config::{
    DataErrorBarStyleConfig, DataLineStyleConfig, DataScatterStyleConfig, ErrorBars, ErrorRef,
    LineFill, LineInterpolation, ScatterShape, XySeries,
};
use crate::gpu_pick::exact_variant as v;
use crate::gpu_pick::registry_probe::MERGE_PROBE;
use crate::line::LineStylePreset;

fn scatter(shape: ScatterShape) -> DataScatterStyleConfig {
    DataScatterStyleConfig {
        point_color: Color::BLACK,
        point_shape: shape,
        point_size: 3.0,
        point_style_table: None,
        point_style_index_column: None,
        point_style_overrides: None,
        point_color_map: None,
        point_size_map: None,
    }
}

fn line(preset: LineStylePreset, fill: Option<LineFill>) -> DataLineStyleConfig {
    DataLineStyleConfig {
        line_style: preset,
        line_color: Color::BLACK,
        line_width: 1.5,
        fill,
        fill_color: Some(Color::BLACK),
        interpolation: LineInterpolation::Linear,
    }
}

fn xy(scatter: Option<DataScatterStyleConfig>, line: Option<DataLineStyleConfig>, errors: bool) -> SeriesConfig {
    SeriesConfig {
        series_id: "s".into(),
        source_id: None,
        label: None,
        x_column: "x".into(),
        y_column: "y".into(),
        z_column: None,
        render_type: DataRenderType::Xy(XySeries {
            scatter,
            line,
            errors: errors.then(|| ErrorBars {
                x: None,
                y: Some(ErrorRef::Symmetric { column: "e".into() }),
                style: DataErrorBarStyleConfig {
                    error_bar_color: Color::BLACK,
                    error_bar_width: 1.0,
                    error_bar_cap_size: 3.0,
                    cap_width: 1.0,
                    error_bar_style_table: None,
                    error_bar_style_index_column: None,
                    error_bar_style_overrides: None,
                },
            }),
            stem: None,
        }),
    }
}

fn config(n: usize, polar: bool, draw: DrawStyle) -> Config {
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 1024, height: 768 });
    config.legend.visible = false;
    for axis in [&mut config.bottom_x, &mut config.top_x] {
        axis.min = 0.0;
        axis.max = n as f64;
    }
    for axis in [&mut config.left_y, &mut config.right_y] {
        axis.min = 0.0;
        axis.max = 10.0;
    }
    if polar {
        config.set_polar(Some(crate::polar::PolarOptions::default()));
    }
    config.draw_style = draw;
    config
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

#[test]
#[ignore = "measurement only"]
fn merge_bench() {
    let n: usize = std::env::var("FIGGY_MERGE_N").ok().and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let runs: usize = std::env::var("FIGGY_MERGE_RUNS").ok().and_then(|s| s.parse().ok()).unwrap_or(5);
    let filter = std::env::var("FIGGY_MERGE_ONLY").unwrap_or_default();
    let (device, queue) = crate::data_render::shared_device().unwrap();
    let limits = device.limits();
    eprintln!(
        "MERGE adapter limits: max_storage_buffer_binding_size={} max_buffer_size={}",
        limits.max_storage_buffer_binding_size, limits.max_buffer_size
    );
    let mut renderer = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        limits.max_storage_buffer_binding_size.min(limits.max_buffer_size),
    )
    .unwrap();
    renderer.admit_full_shaders(true);
    renderer.enable_gpu_picking().unwrap();
    // A smooth wave: x = index, y in 1..9; the polar charts read x as degrees.
    let xs: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let ys: Vec<f64> = (0..n).map(|i| 5.0 + 4.0 * ((i as f64) * 0.001).sin()).collect();
    renderer.add_column("x", &Column { data: xs }).unwrap();
    renderer.add_column("y", &Column { data: ys }).unwrap();
    renderer.add_column("e", &Column { data: vec![0.2; n] }).unwrap();
    renderer.add_column("cat", &Column { data: (0..n).map(|i| (i % 3) as f64).collect::<Vec<f64>>() }).unwrap();
    renderer.add_column("c", &Column { data: vec![3.0; n] }).unwrap();
    renderer.add_column("pos", &Column { data: vec![0.0, 1.0, 2.0] }).unwrap();
    {
        let field = |value: f64| Column { data: vec![value; 3] };
        let fields = [field(5.0), field(4.0), field(6.0), field(2.0), field(8.0)];
        let names = ["median", "q1", "q3", "whisker_low", "whisker_high"];
        let sources: Vec<(&str, &dyn ColumnSource)> = names
            .iter()
            .zip(&fields)
            .map(|(name, column)| (*name, column as &dyn ColumnSource))
            .collect();
        renderer
            .upsert_record("box.record", &crate::RecordSource { schema: data_render::BOX_PLOT_RECORD, fields: &sources })
            .unwrap();
    }
    crate::gpu_pick::registry_probe::FORCE_DIRECT.with(|f| f.set(std::env::var("FIGGY_MERGE_GATED").is_err()));
    let all = v::STYLED_MARKER | v::ERRORBAR | v::AREA | v::DASH | v::STYLED_LINE | v::POLAR
        | v::PATH_ARROWS | v::CATEGORY_JITTER | v::TERNARY;
    let sketch = || DrawStyle::Sketch(SketchOptions::default());
    let marker = || Some(scatter(ScatterShape::CircleFilled));
    let solid = |fill| Some(line(LineStylePreset::Solid, fill));
    let mut kinds: Vec<(&str, Config, SeriesConfig)> = vec![
        ("scatter", config(n, false, DrawStyle::Precise), xy(marker(), None, false)),
        ("scatter+errorbar", config(n, false, DrawStyle::Precise), xy(marker(), None, true)),
        ("scatter sketch", config(n, false, sketch()), xy(marker(), None, false)),
        ("line", config(n, false, DrawStyle::Precise), xy(None, solid(None), false)),
        ("line+fill", config(n, false, DrawStyle::Precise), xy(None, solid(Some(LineFill::ToBaseline(0.0))), false)),
        ("dashed line", config(n, false, DrawStyle::Precise), xy(None, Some(line(LineStylePreset::Dash, None)), false)),
        ("line sketch", config(n, false, sketch()), xy(None, solid(None), false)),
        ("polar line", config(n, true, DrawStyle::Precise), xy(None, solid(None), false)),
        ("marker+line", config(n, false, DrawStyle::Precise), xy(marker(), solid(None), false)),
        ("polar marker+line+fill", config(n, true, DrawStyle::Precise), xy(marker(), solid(Some(LineFill::ToBaseline(0.0))), false)),
    ];
    {
        let mut ternary = config(n, false, DrawStyle::Precise);
        for axis in [&mut ternary.bottom_x, &mut ternary.left_y, &mut ternary.right_y] {
            axis.min = 0.0;
            axis.max = 100.0;
        }
        ternary.set_ternary(Some(crate::TernaryOptions {}));
        kinds.push(("ternary scatter", ternary, SeriesConfig {
            render_type: DataRenderType::Ternary(crate::data_config::TernarySeries {
                c_column: "c".into(),
                scatter: marker(),
                line: None,
                region: None,
            }),
            ..xy(None, None, false)
        }));
    }
    kinds.push(("streamline+arrows", config(n, false, DrawStyle::Precise), SeriesConfig {
        render_type: DataRenderType::Streamlines(crate::data_config::StreamlineSeries {
            line: line(LineStylePreset::Solid, None),
            arrows: Some(crate::StreamlineArrows { spacing_px: 24.0, head_length_px: 9.0, head_width_px: 10.0 }),
            color_map: None,
        }),
        ..xy(None, None, false)
    }));
    {
        use crate::data_config::{BarGroups, BoxOutlierShape, BoxPoints, BoxPointsStyle, BoxSeries, CategoryBarDirection};
        let mut boxes = config(n, false, DrawStyle::Precise);
        boxes.bottom_x.categories = Some(vec![
            crate::config::Category::new("a", "A"),
            crate::config::Category::new("b", "B"),
            crate::config::Category::new("c", "C"),
        ]);
        boxes.bottom_x.min = -0.5;
        boxes.bottom_x.max = 2.5;
        boxes.bar_groups = Some(BarGroups { direction: CategoryBarDirection::Vertical, ..BarGroups::default() });
        kinds.push(("box points", boxes, SeriesConfig {
            x_column: "pos".into(),
            y_column: "box.record".into(),
            render_type: DataRenderType::BoxPlot(BoxSeries {
                color: Color::BLACK,
                style: crate::data_config::BoxPlotStyle::default(),
                labels: Default::default(),
                outliers: None,
                overrides: None,
                violin: None,
                points: Some(BoxPoints {
                    positions: "cat".into(),
                    values: "y".into(),
                    style: BoxPointsStyle { color: Color::BLACK, size: 6.0, shape: BoxOutlierShape::Circle, filled: true, jitter: 0.3, offset: 0.0 },
                }),
            }),
            ..xy(None, None, false)
        }));
    }
    // (label, bits ORed, shape merged)
    let cheap = v::POLAR | v::DASH | v::STYLED_MARKER | v::STYLED_LINE | v::CATEGORY_JITTER | v::TERNARY;
    let merged: Vec<(String, u32, bool)> = vec![
        ("specialized".to_string(), 0, false),
        ("shape->any".to_string(), 0, true),
        ("merge cheap + shape".to_string(), cheap, true),
        ("merge all".to_string(), all, true),
    ];
    let mut per_bit = merged.clone();
    for (name, bit) in [
        ("STYLED_MARKER", v::STYLED_MARKER),
        ("ERRORBAR", v::ERRORBAR),
        ("AREA", v::AREA),
        ("DASH", v::DASH),
        ("STYLED_LINE", v::STYLED_LINE),
        ("POLAR", v::POLAR),
        ("PATH_ARROWS", v::PATH_ARROWS),
        ("CATEGORY_JITTER", v::CATEGORY_JITTER),
        ("TERNARY", v::TERNARY),
    ] {
        per_bit.push((format!("+{name}"), bit, false));
    }
    for (kind, cfg, series) in kinds {
        if !filter.is_empty() && !kind.contains(filter.as_str()) {
            continue;
        }
        let variants = if kind == "scatter" || kind == "line" || std::env::var("FIGGY_MERGE_PERBIT").is_ok() { &per_bit } else { &merged };
        let id = renderer.register_chart(cfg.clone(), vec![series]).unwrap();
        let area = cfg.data_area().unwrap().0;
        let request = GpuPickRequest {
            canvas_position_px: [area.x as f32 + area.width as f32 * 0.37, area.y as f32 + area.height as f32 * 0.5],
            display_panel_px: cfg.chart_area.0,
            display_scale: 1.0,
            max_distance_px: 6.0,
        };
        let mut reference = None;
        // Compile every variant and check its pick first.
        for (label, bits, any) in variants.iter() {
            MERGE_PROBE.with(|p| p.set((*bits, *any)));
            let warm = crate::test_gpu::block_on(renderer.pick_chart(id, request).unwrap().resolve()).unwrap();
            let picked = warm.as_ref().map(|h| (h.series_id.clone(), h.point_index, h.distance_px.to_bits()));
            match &reference {
                None => reference = Some(picked.clone()),
                Some(r) => assert_eq!(r, &picked, "{kind} {label}: the pick changed"),
            }
        }
        // Round-robin, so drift spreads over every variant alike.
        let mut times = vec![Vec::new(); variants.len()];
        for _ in 0..runs {
            for (index, (_, bits, any)) in variants.iter().enumerate() {
                MERGE_PROBE.with(|p| p.set((*bits, *any)));
                let start = std::time::Instant::now();
                crate::test_gpu::block_on(renderer.pick_chart(id, request).unwrap().resolve()).unwrap();
                times[index].push(start.elapsed().as_secs_f64() * 1e3);
            }
        }
        let base = median(times[0].clone());
        for ((label, _, _), t) in variants.iter().zip(&times) {
            let m = median(t.clone());
            let mut sorted = t.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            eprintln!(
                "MERGE n={n} kind={kind} variant={label} median_ms={m:.2} slowdown_pct={:+.1} min_ms={:.2} max_ms={:.2} hit={:?}",
                (m / base - 1.0) * 100.0,
                sorted[0],
                sorted[sorted.len() - 1],
                reference.as_ref().unwrap()
            );
        }
        MERGE_PROBE.with(|p| p.set((0, false)));
        renderer.remove_chart(id).unwrap();
    }
}
