//! The categorical axis (`AxisOptions::categories`) through the public API.
//!
//! A categorical axis is an ordinary axis whose majors are the category
//! positions and whose tick labels are the category labels, so it is held
//! to what an ordinary axis draws: with labels "0", "1", … it rasterizes and
//! exports pixel for pixel like the numeric axis over the same range with a
//! tick at every integer, at 1x and 2x. Its labels are the `AxisLabel`
//! element of the side they sit on (bounds, hit, selection box and both drag
//! offsets), on overlay axes and in subplots too, and the data drawn against
//! it picks, highlights and anchors annotations at the category positions.
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::Arc;

use renderer::axis_render::{AxisLayerKind, try_raster_chart_layer_to_rgba};
use renderer::config::{
    AxisOptions, Category, DataSelectionsConfig, PickedDataRef, TickVisibility,
};
use renderer::data::Column;
use renderer::format::{LabelFormat, PlainNumberFormat};
use renderer::layout::{ChartArea, NudgeResult, Rect, RectF, Side, axis_fraction};
use renderer::sink::{Border, PlacedText, Sink, Slot, StrokeStyle, TextRef};
use renderer::{
    AxisLabelElement, Chart, Color, ColorBarOptions, Config, CpuTextMeasure, DataRenderType,
    DataScatterStyleConfig, Draggable, GpuPickRequest, HitMap, LayerRef, OverlayLayer, Renderer,
    RendererDevice, ScatterShape, Selectable, SelectionGeometry, SeriesConfig, Subplot,
};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

fn categories(labels: &[&str]) -> Option<Vec<Category>> {
    Some(
        labels
            .iter()
            .enumerate()
            .map(|(i, label)| Category::new(format!("id{i}"), *label))
            .collect(),
    )
}

/// The labels "0", "1", … of `n` categories.
fn digit_categories(n: usize) -> Option<Vec<Category>> {
    let labels: Vec<String> = (0..n).map(|i| i.to_string()).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    categories(&refs)
}

/// What a categorical axis of `n` categories draws, written as a numeric
/// axis: the range `[-0.5, n - 0.5]`, a major tick at every integer, no
/// minors, integer labels.
fn numeric_twin(axis: &mut AxisOptions, n: usize) {
    axis.categories = None;
    axis.min = -0.5;
    axis.max = n as f64 - 0.5;
    axis.major_spacing = 1.0;
    axis.minor_count = 0;
    axis.label_style.format = LabelFormat::Plain(PlainNumberFormat {
        decimals: Some(0),
        trim_zeros: true,
    });
}

fn base_config() -> Config {
    let mut cfg = renderer::default::default_config();
    cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    for axis in [
        &mut cfg.top_x,
        &mut cfg.bottom_x,
        &mut cfg.left_y,
        &mut cfg.right_y,
    ] {
        axis.label_style.visible = true;
        axis.label_style.label_visible = true;
        axis.tick = TickVisibility::Both;
    }
    cfg.grid.show_major_x = true;
    cfg.grid.show_major_y = true;
    cfg.grid.show_minor_x = true;
    cfg.grid.show_minor_y = true;
    cfg
}

/// Categorical axes on all four sides (`nx` categories on X, `ny` on Y),
/// with digit labels, and the same document written with numeric axes.
fn categorical_and_numeric(nx: usize, ny: usize) -> (Config, Config) {
    let mut categorical = base_config();
    categorical.top_x.categories = digit_categories(nx);
    categorical.bottom_x.categories = digit_categories(nx);
    categorical.left_y.categories = digit_categories(ny);
    categorical.right_y.categories = digit_categories(ny);
    // Ignored on a categorical axis.
    categorical.bottom_x.min = 3.0;
    categorical.bottom_x.max = 4.0;
    categorical.bottom_x.major_spacing = 0.01;
    categorical.bottom_x.minor_count = 9;
    categorical.left_y.label_style.format = LabelFormat::Scientific;
    let mut numeric = base_config();
    numeric_twin(&mut numeric.top_x, nx);
    numeric_twin(&mut numeric.bottom_x, nx);
    numeric_twin(&mut numeric.left_y, ny);
    numeric_twin(&mut numeric.right_y, ny);
    (categorical, numeric)
}

/// The axis raster (grid, axes, ticks, labels, titles) of a categorical
/// axis is the numeric twin's, pixel for pixel, at 1x and 2x, also when the
/// labels crowd and are thinned.
#[test]
fn a_categorical_axis_rasterizes_like_its_numeric_twin() {
    let mut compared = 0;
    for (nx, ny) in [(7, 4), (1, 2), (64, 40)] {
        let (categorical, numeric) = categorical_and_numeric(nx, ny);
        for scale in [1.0f32, 2.0] {
            let (a, b) = if scale == 1.0 {
                (categorical.clone(), numeric.clone())
            } else {
                (categorical.scaled(scale), numeric.scaled(scale))
            };
            for layer in [
                AxisLayerKind::Grid,
                AxisLayerKind::Decoration,
                AxisLayerKind::All,
            ] {
                let left = try_raster_chart_layer_to_rgba(&a, layer).unwrap();
                let right = try_raster_chart_layer_to_rgba(&b, layer).unwrap();
                let changed = left
                    .chunks_exact(4)
                    .zip(right.chunks_exact(4))
                    .filter(|(x, y)| x != y)
                    .count();
                assert_eq!(changed, 0, "{nx}x{ny} at {scale}x, {layer:?}");
                compared += 1;
            }
        }
    }
    assert_eq!(compared, 18);
}

fn gpu() -> RendererDevice {
    let (device, queue) = test_gpu::device();
    RendererDevice::new(Arc::new(device), Arc::new(queue))
}

fn upload(r: &mut Renderer, id: &str, data: Vec<f64>) {
    let (min, max) = data
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    r.add_column(id, &Column { data, min, max }).unwrap();
}

fn markers(id: &str, x: &str, y: &str) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type: DataRenderType::Scatter {
            scatter: DataScatterStyleConfig {
                point_color: Color::from_rgb8(31, 119, 180),
                point_shape: ScatterShape::CircleFilled,
                point_size: 12.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            },
        },
    }
}

/// Seven categories on X, values on a numeric Y, markers at the category
/// positions.
fn data_chart(r: &mut Renderer) -> (Config, Vec<SeriesConfig>) {
    upload(r, "cx", (0..7).map(f64::from).collect());
    upload(r, "cy", (0..7).map(|i| 1.0 + f64::from(i % 3)).collect());
    let mut cfg = base_config();
    cfg.bottom_x.categories = categories(&["a", "b", "c", "d", "e", "f", "g"]);
    cfg.left_y.min = 0.0;
    cfg.left_y.max = 4.0;
    cfg.left_y.major_spacing = 1.0;
    (cfg, vec![markers("m", "cx", "cy")])
}

/// The export (data, grid and chrome) of a chart with categorical axes is
/// the numeric twin's at 1x and 2x, and its PNG encodes at both scales.
#[test]
fn a_categorical_chart_exports_like_its_numeric_twin() {
    let mut r = Renderer::try_new(gpu(), wgpu::TextureFormat::Rgba8Unorm, 16 << 20).unwrap();
    let (mut categorical, series) = data_chart(&mut r);
    categorical.bottom_x.categories = digit_categories(7);
    categorical.top_x.categories = digit_categories(7);
    let mut numeric = categorical.clone();
    numeric_twin(&mut numeric.bottom_x, 7);
    numeric_twin(&mut numeric.top_x, 7);
    for scale in [1.0f32, 2.0] {
        let a = r
            .export_document_rgba(&Chart::new(categorical.clone()), &series, scale)
            .unwrap();
        let b = r
            .export_document_rgba(&Chart::new(numeric.clone()), &series, scale)
            .unwrap();
        assert_eq!((a.width, a.height), (b.width, b.height));
        let changed = a
            .rgba
            .chunks_exact(4)
            .zip(b.rgba.chunks_exact(4))
            .filter(|(x, y)| x != y)
            .count();
        assert_eq!(changed, 0, "{scale}x");
        let png = r
            .export_document_png_bytes(&Chart::new(categorical.clone()), &series, scale)
            .unwrap();
        assert!(png.starts_with(b"\x89PNG"), "{scale}x");
    }
}

/// The marker of category `i` picks as point `i`, its highlight is drawn
/// there, and an annotation anchored at data x `i` sits on that category.
#[test]
fn data_on_a_categorical_axis_picks_and_highlights_at_its_category() {
    let mut r = Renderer::try_new(gpu(), wgpu::TextureFormat::Rgba8Unorm, 16 << 20).unwrap();
    r.enable_gpu_picking().expect("GPU picking");
    let (cfg, series) = data_chart(&mut r);
    let da = cfg.data_area().unwrap();
    let plot = RectF::from_rect(&da);
    let at = |i: usize| {
        let x = plot.x + axis_fraction(i as f64, &cfg.bottom_x) * plot.width;
        let y =
            plot.y + plot.height - axis_fraction(1.0 + (i % 3) as f64, &cfg.left_y) * plot.height;
        (x, y)
    };
    let id = r.register_chart(cfg.clone(), series.clone()).unwrap();
    for i in [0, 3, 6] {
        let (x, y) = at(i);
        let request = GpuPickRequest {
            canvas_position_px: [x, y],
            display_panel_px: Rect {
                x: 0,
                y: 0,
                width: WIDTH,
                height: HEIGHT,
            },
            display_scale: 1.0,
            max_distance_px: 3.0,
        };
        let picked = pollster::block_on(r.pick_chart(id, request).unwrap().resolve())
            .unwrap()
            .expect("a marker at its category");
        assert_eq!((picked.series_id.as_str(), picked.point_index), ("m", i));
    }

    // The highlight of point 3 changes pixels around its marker only.
    let plain = r
        .export_document_rgba(&Chart::new(cfg.clone()), &series, 1.0)
        .unwrap();
    let mut selected = cfg.clone();
    selected.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            series_id: "m".into(),
            source_id: None,
            point_index: 3,
        }],
        ..Default::default()
    });
    let highlighted = r
        .export_document_rgba(&Chart::new(selected), &series, 1.0)
        .unwrap();
    let (cx, cy) = at(3);
    let mut changed = 0;
    for (index, (a, b)) in plain
        .rgba
        .chunks_exact(4)
        .zip(highlighted.rgba.chunks_exact(4))
        .enumerate()
    {
        if a != b {
            let x = (index as u32 % WIDTH) as f32 + 0.5;
            let y = (index as u32 / WIDTH) as f32 + 0.5;
            assert!(
                (x - cx).abs() < 20.0 && (y - cy).abs() < 20.0,
                "highlight pixel ({x}, {y}) away from category 3 at ({cx}, {cy})"
            );
            changed += 1;
        }
    }
    assert!(changed > 0, "the highlight drew nothing");

    // The annotation space maps data x `i` to category `i`.
    let space = renderer::annotation::pixel_space_for_config(&cfg);
    for i in 0..7 {
        let t = space.x.fraction(i as f64).unwrap() as f32;
        assert!(
            (t - axis_fraction(i as f64, &cfg.bottom_x)).abs() < 1e-6,
            "{i}"
        );
    }
}

/// The plain texts a sink receives, with their drawn boxes.
#[derive(Default)]
struct TextLog(Vec<(String, RectF)>);

impl Sink for TextLog {
    fn text(&mut self, _slot: Slot, text: &PlacedText<'_>) {
        if let TextRef::Plain { text: plain, .. } = text.text {
            self.0.push((plain.to_owned(), text.rect()));
        }
    }
    fn stroke(&mut self, _: Slot, _: [f32; 2], _: [f32; 2], _: &StrokeStyle<'_>) {}
    fn rect(&mut self, _: Slot, _: RectF, _: Option<Color>, _: Option<Border<'_>>) {}
    fn triangle(&mut self, _: Slot, _: [[f32; 2]; 3], _: Color) {}
    fn ramp(&mut self, _: Slot, _: RectF, _: &ColorBarOptions) {}
}

/// The texts the element registered as `name` in `map` draws.
fn element_texts(cfg: &Config, map: &HitMap, name: &str) -> Vec<String> {
    let id = map
        .find(name)
        .unwrap_or_else(|| panic!("{name} not registered"));
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let mut log = TextLog::default();
    map.get(id).unwrap().emit(cfg, &measure, &mut log);
    log.0.into_iter().map(|(text, _)| text).collect()
}

const DAYS: [&str; 5] = ["Mon", "Tue", "Wed", "Thu", "Fri"];
const LETTERS: [&str; 5] = ["A", "B", "C", "D", "E"];

/// On every side, the category labels are the side's tick-label element:
/// its bounds hold every drawn label, a click on them hits it, and a drag
/// along or across the axis moves the drawn labels, the bounds and the
/// selection box together.
#[test]
fn category_labels_are_the_tick_label_element_of_their_side() {
    for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
        let mut cfg = base_config();
        let axis = match side {
            Side::Top => &mut cfg.top_x,
            Side::Bottom => &mut cfg.bottom_x,
            Side::Left => &mut cfg.left_y,
            Side::Right => &mut cfg.right_y,
        };
        axis.categories = categories(&DAYS);
        let element = AxisLabelElement { side: side.clone() };
        let name = element.element_id();
        let measure = CpuTextMeasure::for_style(&cfg.draw_style);
        let map = HitMap::standard_chart();
        assert_eq!(element_texts(&cfg, &map, &name), DAYS, "{side:?}");

        let mut log = TextLog::default();
        element.emit(&cfg, &measure, &mut log);
        let bounds = element.bounds(&cfg, &measure).unwrap();
        for (_, rect) in &log.0 {
            assert!(
                rect.x >= bounds.x - 1e-3
                    && rect.y >= bounds.y - 1e-3
                    && rect.x + rect.width <= bounds.x + bounds.width + 1e-3
                    && rect.y + rect.height <= bounds.y + bounds.height + 1e-3,
                "{side:?}: {rect:?} outside {bounds:?}"
            );
        }
        let (cx, cy) = log.0[2].1.center();
        let hit = map.hit_test(&cfg, &measure, cx, cy).unwrap();
        assert_eq!(map.get(hit).unwrap().element_id(), name, "{side:?}");

        let (along, across) = match side {
            Side::Top | Side::Bottom => ((30.0, 0.0), (0.0, 6.0)),
            Side::Left | Side::Right => ((0.0, 30.0), (6.0, 0.0)),
        };
        for (dx, dy) in [along, across] {
            let mut moved = cfg.clone();
            assert_eq!(
                element.drag_by(&mut moved, &measure, dx, dy),
                NudgeResult::Moved,
                "{side:?} by ({dx}, {dy})"
            );
            let after = element.bounds(&moved, &measure).unwrap();
            assert_eq!(
                (after.x - bounds.x, after.y - bounds.y),
                (dx, dy),
                "{side:?} by ({dx}, {dy})"
            );
            let mut moved_log = TextLog::default();
            element.emit(&moved, &measure, &mut moved_log);
            for ((_, a), (_, b)) in log.0.iter().zip(&moved_log.0) {
                assert!((b.x - a.x - dx).abs() < 1e-3 && (b.y - a.y - dy).abs() < 1e-3);
            }
            let padding = bounds.x - element.selection_box(&cfg, &measure).unwrap().rect().x;
            assert_eq!(
                element.selection_box(&moved, &measure).unwrap().rect(),
                after.expanded(padding)
            );
        }
    }
}

/// An overlay's Y axis may be categorical: its tick-label element draws the
/// categories on the right, and the panel maps the overlay's series by the
/// category range.
#[test]
fn an_overlay_axis_may_be_categorical() {
    let mut cfg = base_config();
    cfg.right_y.label_style.label_visible = false;
    cfg.overlays.push(OverlayLayer {
        series: vec!["level".into()],
        ..Default::default()
    });
    cfg.overlays[0].y_axis.categories = categories(&DAYS);
    cfg.overlays[0].y_axis.label_style.label_visible = true;
    cfg.set_active_layer(LayerRef::Overlay(0));
    let map = HitMap::for_active_layer(&cfg);
    assert_eq!(
        element_texts(&cfg, &map, "overlay_0:tick_labels_right"),
        DAYS
    );
    let expansion = cfg.expand().unwrap();
    let panel = &expansion.panels[1].config;
    assert_eq!((panel.left_y.min, panel.left_y.max), (-0.5, 4.5));
    // The overlay's labels are in the decoration raster.
    let mut numeric = cfg.clone();
    numeric.overlays[0].y_axis.categories = None;
    let drawn = |cfg: &Config| {
        let panel = cfg.expand().unwrap().panels[1].config.clone();
        try_raster_chart_layer_to_rgba(&panel, AxisLayerKind::Decoration).unwrap()
    };
    assert_ne!(drawn(&cfg), drawn(&numeric));
}

/// A subplot's axis may be categorical, its own or linked to the parent's:
/// the subplot's tick-label element draws the categories.
#[test]
fn a_subplot_axis_may_be_categorical() {
    let mut cfg = base_config();
    cfg.chart_area.0.height = HEIGHT / 2;
    cfg.bottom_x.categories = categories(&DAYS);
    let mut linked = Subplot::default();
    linked.config.chart_area = ChartArea(Rect {
        x: 0,
        y: HEIGHT / 2,
        width: WIDTH / 2,
        height: HEIGHT / 2,
    });
    linked.link_x = Some(renderer::CellRef::Parent);
    let mut own = Subplot::default();
    own.config.chart_area = ChartArea(Rect {
        x: WIDTH / 2,
        y: HEIGHT / 2,
        width: WIDTH / 2,
        height: HEIGHT / 2,
    });
    own.config.left_y.categories = categories(&["low", "high"]);
    // Letters, so the narrow subplots keep every label.
    cfg.bottom_x.categories = categories(&LETTERS);
    cfg.subplots = vec![linked, own];
    cfg.sync_subplot_links(None);
    for (index, name, expected) in [
        (0, "subplot_0:tick_labels_bottom", &LETTERS[..]),
        (1, "subplot_1:tick_labels_left", &["low", "high"][..]),
    ] {
        let mut active = cfg.clone();
        active.set_active_layer(LayerRef::Subplot(index));
        let map = HitMap::for_active_layer(&active);
        assert_eq!(element_texts(&active, &map, name), expected, "{name}");
    }
}
