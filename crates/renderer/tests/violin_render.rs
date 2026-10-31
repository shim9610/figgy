//! End-to-end pixel verification of box plot violins and raw points
//! (`BoxPlot.violin`, `BoxPlot.points`) through the public renderer API and
//! the headless `export_panel_rgba`: symmetric and split bodies in both
//! directions at 1x and 2x, the density scale, the narrowed and hidden
//! box, a box override restyling its category's violin, NaN breaks,
//! the jittered raw points, every pick and highlight, value labels past the
//! curve, the fit, replaced and mismatched columns, overlays and subplots,
//! and curves and point sets past every size the bounded charts once
//! capped.
//!
//! Every part has its own pure colour, flat with no outline unless a test
//! asks for one, so a colour predicate attributes every pixel to one part.
//! Expected extents come from the same data area and axis ranges the
//! renderer draws with.

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::config::Category;
use renderer::data::Column;
use renderer::data_config::{
    BarGroups, BoxOutlierShape, BoxPlotOverride, BoxPlotSeriesLabels, BoxPlotStyle, BoxPlotTarget,
    BoxPoints, BoxPointsStyle, CategoryBarDirection, CategoryBarMode, Violin, ViolinBody,
    ViolinHalf, ViolinSide, ViolinStyle,
};
use renderer::layout::{ChartArea, Rect};
use renderer::{
    BOX_PLOT_RECORD, Chart, Color, ColumnSource, DataRenderType, DataSelectionsConfig, LayerRef,
    Material, PickedDataRef, RaggedSource, RasterImage, RecordSource, Renderer, RendererDevice,
    SeriesConfig,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
const RED: Color = Color::from_rgb8(255, 0, 0);
const BLUE: Color = Color::from_rgb8(0, 0, 255);
const GREEN: Color = Color::from_rgb8(0, 255, 0);
const YELLOW: Color = Color::from_rgb8(255, 255, 0);
const CYAN: Color = Color::from_rgb8(0, 255, 255);
const ORANGE: Color = Color::from_rgb8(255, 128, 0);

/// The curve grid of every category: values 10..=90, one per unit.
const GRID: std::ops::RangeInclusive<i32> = 10..=90;

fn shared_device() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    static DEVICE: OnceLock<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(|| {
        let (device, queue) = test_gpu::device();
        (Arc::new(device), Arc::new(queue))
    });
    (Arc::clone(device), Arc::clone(queue))
}

fn col(data: Vec<f64>) -> Column<f64> {
    let finite = data.iter().copied().filter(|v| v.is_finite());
    let min = finite.clone().fold(f64::INFINITY, f64::min);
    let max = finite.fold(f64::NEG_INFINITY, f64::max);
    Column { data, min, max }
}

/// A triangular density on the grid peaking at `peak` with height 1 and
/// reaching 0 `spread` away.
fn triangle(peak: f64, spread: f64) -> impl Fn(f64) -> f64 {
    move |v| (1.0 - (v - peak).abs() / spread).max(0.0)
}

/// Upload the curve `id` (one group per density in `densities`, on the
/// grid) and its density column `{id}.density`.
fn upload_curve(r: &mut Renderer, id: &str, densities: &[&dyn Fn(f64) -> f64]) {
    let mut values = Vec::new();
    let mut density = Vec::new();
    let mut offsets = vec![0.0];
    for d in densities {
        for v in GRID {
            values.push(f64::from(v));
            density.push(d(f64::from(v)));
        }
        offsets.push(values.len() as f64);
    }
    r.upsert_ragged(
        id,
        &RaggedSource {
            values: &col(values),
            offsets: &col(offsets),
        },
    )
    .unwrap();
    r.add_column(&format!("{id}.density"), &col(density))
        .unwrap();
}

/// A box record of three boxes, the quartiles 40..60 around 50.
fn upload_record(r: &mut Renderer, id: &str) {
    let field = |v: f64| col(vec![v; 3]);
    let columns = [
        field(50.0),
        field(40.0),
        field(60.0),
        field(20.0),
        field(80.0),
    ];
    let names = ["median", "q1", "q3", "whisker_low", "whisker_high"];
    let fields: Vec<(&str, &dyn ColumnSource)> = names
        .iter()
        .zip(&columns)
        .map(|(name, column)| (*name, column as &dyn ColumnSource))
        .collect();
    r.upsert_record(
        id,
        &RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &fields,
        },
    )
    .unwrap();
}

/// A renderer holding the positions, a record, a symmetric curve (`sym`,
/// peaking at 50 in every category), two split curves (`lo` peaking at
/// 35, `hi` at 65) and 300 raw points (100 per category).
fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        16 * 1024 * 1024,
    )
    .expect("renderer init");
    r.add_column("pos", &col(vec![0.0, 1.0, 2.0])).unwrap();
    upload_record(&mut r, "box.record");
    let sym = triangle(50.0, 40.0);
    upload_curve(&mut r, "sym", &[&sym, &sym, &sym]);
    let lo = triangle(35.0, 25.0);
    upload_curve(&mut r, "lo", &[&lo, &lo, &lo]);
    let hi = triangle(65.0, 25.0);
    upload_curve(&mut r, "hi", &[&hi, &hi, &hi]);
    r.add_column(
        "pt.cat",
        &col((0..300).map(|i| f64::from(i / 100)).collect()),
    )
    .unwrap();
    r.add_column(
        "pt.value",
        &col((0..300).map(|i| 30.0 + f64::from(i % 41)).collect()),
    )
    .unwrap();
    r
}

fn half(curve: &str, fill: Color) -> ViolinHalf {
    ViolinHalf {
        curve: curve.into(),
        density: format!("{curve}.density"),
        density_scale: 1.0,
        style: ViolinStyle {
            fill: Some(fill),
            outline: false,
            ..ViolinStyle::default()
        },
    }
}

fn flat_style() -> BoxPlotStyle {
    BoxPlotStyle {
        material: Material::Flat,
        outline: false,
        median_width: 2.0,
        median_color: GREEN,
        whisker_width: 2.0,
        whisker_color: YELLOW,
        caps: false,
        show_outliers: false,
        ..BoxPlotStyle::default()
    }
}

/// A box plot series over the positions and the record with `violin` (and
/// no points).
fn violin_series(id: &str, body: ViolinBody, box_width: f32) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: "pos".into(),
        y_column: "box.record".into(),
        z_column: None,
        render_type: DataRenderType::BoxPlot {
            color: CYAN,
            style: flat_style(),
            labels: BoxPlotSeriesLabels::default(),
            outliers: None,
            overrides: None,
            violin: Some(Violin { body, box_width }),
            points: None,
        },
    }
}

fn with_points(mut series: SeriesConfig, offset: f32, jitter: f32) -> SeriesConfig {
    let DataRenderType::BoxPlot { points, .. } = &mut series.render_type else {
        unreachable!()
    };
    *points = Some(BoxPoints {
        positions: "pt.cat".into(),
        values: "pt.value".into(),
        style: BoxPointsStyle {
            color: ORANGE,
            size: 4.0,
            shape: BoxOutlierShape::Circle,
            filled: true,
            jitter,
            offset,
        },
    });
    series
}

/// Grid, legend, title and axis lines off; three categories on the
/// direction's category axis, bottom-up; the value axis over 0..100; one
/// member covering 0.8 of its slot.
fn chart(direction: CategoryBarDirection) -> Chart {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    config.chart_title.visible = false;
    for axis in [
        &mut config.top_x,
        &mut config.bottom_x,
        &mut config.left_y,
        &mut config.right_y,
    ] {
        axis.line_visible = false;
    }
    let categories = Some(vec![
        Category::new("a", "A"),
        Category::new("b", "B"),
        Category::new("c", "C"),
    ]);
    match direction {
        CategoryBarDirection::Vertical => config.bottom_x.categories = categories,
        CategoryBarDirection::Horizontal => config.left_y.categories = categories,
    }
    config.bar_groups = Some(BarGroups {
        mode: CategoryBarMode::Grouped,
        direction,
        group_width: 0.8,
        bar_gap: 0.0,
    });
    let mut chart = Chart::new(config);
    match direction {
        CategoryBarDirection::Vertical => chart.set_y_range(0.0, 100.0),
        CategoryBarDirection::Horizontal => chart.set_x_range(0.0, 100.0),
    }
    chart
}

fn pixel(img: &RasterImage, x: u32, y: u32) -> &[u8] {
    let i = ((y * img.width + x) * 4) as usize;
    &img.rgba[i..i + 4]
}

fn is_red(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && p[1] < 60 && p[2] < 60
}
fn is_blue(p: &[u8]) -> bool {
    p[3] > 200 && p[2] > 200 && p[0] < 60 && p[1] < 60
}
fn is_cyan(p: &[u8]) -> bool {
    p[3] > 200 && p[1] > 200 && p[2] > 200 && p[0] < 60
}
fn is_orange(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && (90..170).contains(&p[1]) && p[2] < 60
}
fn is_magenta(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && p[2] > 200 && p[1] < 60
}
fn is_ink(p: &[u8]) -> bool {
    p[3] > 100
}

/// The slot centre of category `k` and the half thickness of its one
/// member (0.8 of the slot), across the category axis, in export pixels.
fn member(chart: &Chart, direction: CategoryBarDirection, k: usize, scale: f32) -> (f32, f32) {
    let da = chart.config().data_area().expect("data area");
    match direction {
        CategoryBarDirection::Vertical => {
            let w = da.width as f32 / 3.0;
            (
                (da.x as f32 + w * (k as f32 + 0.5)) * scale,
                w * 0.4 * scale,
            )
        }
        CategoryBarDirection::Horizontal => {
            let h = da.height as f32 / 3.0;
            let y = (da.y + da.height) as f32 - h * (k as f32 + 0.5);
            (y * scale, h * 0.4 * scale)
        }
    }
}

/// Value `v` along the value axis in export pixels.
fn along(chart: &Chart, direction: CategoryBarDirection, v: f64, scale: f32) -> f32 {
    let config = chart.config();
    let da = config.data_area().expect("data area");
    let axis = match direction {
        CategoryBarDirection::Vertical => &config.left_y,
        CategoryBarDirection::Horizontal => &config.bottom_x,
    };
    let t = ((v - axis.min) / (axis.max - axis.min)) as f32;
    match direction {
        CategoryBarDirection::Vertical => {
            ((da.y + da.height) as f32 - t * da.height as f32) * scale
        }
        CategoryBarDirection::Horizontal => (da.x as f32 + t * da.width as f32) * scale,
    }
}

/// The span of pixels `pred` accepts across the category axis at value
/// `v`, within category `k`'s slot: `(low, high)` export pixels, inclusive.
fn span_at(
    img: &RasterImage,
    chart: &Chart,
    direction: CategoryBarDirection,
    k: usize,
    v: f64,
    scale: f32,
    pred: fn(&[u8]) -> bool,
) -> Option<(u32, u32)> {
    let (centre, half) = member(chart, direction, k, scale);
    let a = along(chart, direction, v, scale) as u32;
    let lo = (centre - half * 1.3).max(0.0) as u32;
    let hi = (centre + half * 1.3) as u32;
    let mut found: Option<(u32, u32)> = None;
    for c in lo..=hi {
        let p = match direction {
            CategoryBarDirection::Vertical => pixel(img, c, a),
            CategoryBarDirection::Horizontal => pixel(img, a, c),
        };
        if pred(p) {
            let f = found.get_or_insert((c, c));
            *f = (f.0.min(c), f.1.max(c));
        }
    }
    found
}

fn pick(
    r: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    scale: f32,
    at: (f32, f32),
) -> Option<renderer::PickedData> {
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    let request = renderer::GpuPickRequest {
        canvas_position_px: [at.0, at.1],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: (WIDTH as f32 * scale) as u32,
            height: (HEIGHT as f32 * scale) as u32,
        },
        display_scale: scale,
        max_distance_px: 0.0,
    };
    let hit = pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    r.remove_chart(id).unwrap();
    hit
}

/// A point `(across, along)` as `(x, y)` in export pixels.
fn xy(direction: CategoryBarDirection, across: f32, along: f32) -> (f32, f32) {
    match direction {
        CategoryBarDirection::Vertical => (across, along),
        CategoryBarDirection::Horizontal => (along, across),
    }
}

/// A symmetric violin is the density mirrored about the category centre:
/// as wide as the half slot where the density reaches the scale, half as
/// wide where it is half, nothing past the curve; in both directions and
/// at both scales, behind its box narrowed to a quarter of the slot.
#[test]
fn a_symmetric_violin_mirrors_its_density_in_both_directions_and_scales() {
    let mut r = renderer();
    for direction in [
        CategoryBarDirection::Vertical,
        CategoryBarDirection::Horizontal,
    ] {
        let chart = chart(direction);
        let series = [violin_series(
            "v",
            ViolinBody::Symmetric(half("sym", RED)),
            0.25,
        )];
        for scale in [1.0f32, 2.0] {
            let img = r.export_panel_rgba(&chart, &series, scale).unwrap();
            for k in 0..3 {
                let (centre, full) = member(&chart, direction, k, scale);
                // At the peak (50) the body spans the member; the box covers
                // the middle quarter, so measure the red outside it.
                let (lo, hi) = span_at(&img, &chart, direction, k, 50.0, scale, is_red)
                    .unwrap_or_else(|| panic!("{direction:?} {scale}x: no body in {k}"));
                let tol = 2.0 * scale;
                assert!(
                    (lo as f32 - (centre - full)).abs() <= tol
                        && (hi as f32 + 1.0 - (centre + full)).abs() <= tol,
                    "{direction:?} {scale}x {k}: body {lo}..{hi}, want {}..{}",
                    centre - full,
                    centre + full
                );
                // Halfway down the triangle (30 and 70), half as wide; past
                // the box, all red.
                for v in [30.0, 70.0] {
                    let (lo, hi) = span_at(&img, &chart, direction, k, v, scale, is_red)
                        .unwrap_or_else(|| panic!("{direction:?} {scale}x: no body at {v}"));
                    let width = (hi - lo + 1) as f32;
                    assert!(
                        (width - full).abs() <= 2.0 * tol,
                        "{direction:?} {scale}x {k} at {v}: width {width}, want {full}"
                    );
                    assert!(((lo + hi) as f32 * 0.5 + 0.5 - centre).abs() <= tol);
                }
                // The box: cyan, a quarter of the slot (half of 0.25 of the
                // 0.8 member is 0.125 of the half member's 0.4).
                let (blo, bhi) = span_at(&img, &chart, direction, k, 50.0 + 4.0, scale, is_cyan)
                    .expect("the narrowed box");
                let box_half = full * 0.25;
                assert!(
                    ((bhi - blo + 1) as f32 - 2.0 * box_half).abs() <= 2.0 * tol,
                    "{direction:?} {scale}x {k}: box {blo}..{bhi}, want ±{box_half}"
                );
                // Nothing past the curve's ends.
                assert_eq!(
                    span_at(&img, &chart, direction, k, 5.0, scale, is_red),
                    None
                );
                assert_eq!(
                    span_at(&img, &chart, direction, k, 95.0, scale, is_red),
                    None
                );
            }
        }
    }
}

/// A split violin draws each half on its side: `low` left of a vertical
/// violin (below a horizontal one), `high` right of it (above), each with
/// its own curve and colour; a box width of 0 hides the box.
#[test]
fn a_split_violin_draws_each_half_on_its_side() {
    let mut r = renderer();
    for direction in [
        CategoryBarDirection::Vertical,
        CategoryBarDirection::Horizontal,
    ] {
        let chart = chart(direction);
        let series = [violin_series(
            "v",
            ViolinBody::Split {
                low: Some(half("lo", RED)),
                high: Some(half("hi", BLUE)),
            },
            0.0,
        )];
        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        for k in 0..3 {
            let (centre, full) = member(&chart, direction, k, 1.0);
            // Low peaks at 35, high at 65.
            let (lo, hi) = span_at(&img, &chart, direction, k, 35.0, 1.0, is_red).unwrap();
            let (blo, bhi) = span_at(&img, &chart, direction, k, 65.0, 1.0, is_blue).unwrap();
            let low_side = |a: u32, b: u32| match direction {
                // Left of the centre.
                CategoryBarDirection::Vertical => {
                    (b as f32) < centre + 1.0 && a as f32 >= centre - full - 2.0
                }
                // Below on screen: larger y.
                CategoryBarDirection::Horizontal => {
                    a as f32 > centre - 1.0 && b as f32 <= centre + full + 2.0
                }
            };
            assert!(
                low_side(lo, hi),
                "{direction:?} {k}: low half {lo}..{hi} about {centre}"
            );
            assert!(
                !low_side(blo, bhi) || bhi == blo,
                "{direction:?} {k}: high half {blo}..{bhi}"
            );
            // No box at all.
            assert_eq!(
                span_at(&img, &chart, direction, k, 50.0, 1.0, is_cyan),
                None
            );
        }
    }
}

/// A raincloud: one half on the high side, the raw points jittered on the
/// low side, never moving between draws, and on the low side only.
#[test]
fn raincloud_points_sit_jittered_beside_their_half() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(direction);
    let series = [with_points(
        violin_series(
            "v",
            ViolinBody::Split {
                low: None,
                high: Some(half("hi", BLUE)),
            },
            0.2,
        ),
        -0.5,
        0.4,
    )];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let again = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    assert_eq!(img.rgba, again.rgba, "the jitter is the same on every draw");
    let da = chart.config().data_area().unwrap();
    for k in 0..3 {
        let (centre, full) = member(&chart, direction, k, 1.0);
        let mut found = 0;
        for y in da.y..da.y + da.height {
            for x in (centre - full * 1.2) as u32..(centre + full * 1.2) as u32 {
                if is_orange(pixel(&img, x, y)) {
                    found += 1;
                    // The centre line sits half a half slot to the left,
                    // the jitter 0.4 either way: within 0.1..0.9 of the
                    // half member to the left, the marker radius beside.
                    let dx = centre - x as f32;
                    assert!(
                        dx > full * 0.1 - 4.0 && dx < full * 0.9 + 4.0,
                        "{k}: a point at {x}, centre {centre}, half {full}"
                    );
                }
            }
        }
        assert!(found > 100, "{k}: {found} point pixels");
    }
}

/// Every part picks what it paints: the violin body answers its category,
/// a raw point its index, and the highlight of each rings what was picked.
#[test]
fn violins_and_points_pick_and_highlight_what_they_paint() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let mut chart = chart(direction);
    let series = [with_points(
        violin_series(
            "v",
            ViolinBody::Split {
                low: None,
                high: Some(half("hi", BLUE)),
            },
            0.0,
        ),
        -0.5,
        0.4,
    )];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    // The violin body of category b, right of its centre at 65.
    let (centre, full) = member(&chart, direction, 1, 1.0);
    let at = xy(
        direction,
        centre + full * 0.5,
        along(&chart, direction, 65.0, 1.0),
    );
    assert!(is_blue(pixel(&img, at.0 as u32, at.1 as u32)));
    let hit = pick(&mut r, &chart, &series, 1.0, (at.0 + 0.5, at.1 + 0.5));
    assert!(
        matches!(
            &hit,
            Some(renderer::PickedData::HistogramBin {
                bin_index: 1,
                half: Some(ViolinSide::High),
                ..
            })
        ),
        "violin pick names the category and the half: {hit:?}"
    );
    // An orange raw point of category c: pick at its centre.
    let (centre, full) = member(&chart, direction, 2, 1.0);
    let da = chart.config().data_area().unwrap();
    let mut point_at = None;
    'find: for y in da.y..da.y + da.height {
        for x in (centre - full) as u32..centre as u32 {
            if is_orange(pixel(&img, x, y))
                && is_orange(pixel(&img, x + 1, y))
                && is_orange(pixel(&img, x, y + 1))
                && is_orange(pixel(&img, x + 1, y + 1))
            {
                point_at = Some((x as f32 + 1.0, y as f32 + 1.0));
                break 'find;
            }
        }
    }
    let (px, py) = point_at.expect("a raw point of c");
    let hit = pick(&mut r, &chart, &series, 1.0, (px, py));
    let index = match &hit {
        Some(renderer::PickedData::Point {
            series_id,
            point_index,
            ..
        }) if series_id == "v" => *point_index,
        other => panic!("point pick: {other:?}"),
    };
    assert!((200..300).contains(&index), "a point of c: {index}");
    // The point's highlight rings it.
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::Point {
            source_id: None,
            series_id: "v".into(),
            point_index: index,
        }],
        highlight_color: Color::from_rgb8(255, 0, 255),
        outline_width_px: 2.0,
        ..Default::default()
    });
    let ringed = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let mut magenta = 0;
    for y in (py as u32).saturating_sub(8)..py as u32 + 8 {
        for x in (px as u32).saturating_sub(8)..px as u32 + 8 {
            magenta += u32::from(is_magenta(pixel(&ringed, x, y)));
        }
    }
    assert!(magenta > 4, "the picked point's ring: {magenta} px");
    // The violin's highlight rings its body.
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::HistogramBin {
            source_id: None,
            series_id: "v".into(),
            bin_index: 1,
            half: None,
        }],
        highlight_color: Color::from_rgb8(255, 0, 255),
        outline_width_px: 2.0,
        ..Default::default()
    });
    let ringed = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let (centre, full) = member(&chart, direction, 1, 1.0);
    let edge = span_at(&ringed, &chart, direction, 1, 65.0, 1.0, is_magenta)
        .expect("the violin's outline");
    assert!(
        (edge.1 as f32 - (centre + full)).abs() <= 3.0,
        "the outline at the body's edge: {edge:?}"
    );
    // Only the selected category's violin.
    assert_eq!(
        span_at(&ringed, &chart, direction, 0, 65.0, 1.0, is_magenta),
        None
    );
}

/// A box override restyles its category's violin (D7): the colour fills
/// it, only in that category.
#[test]
fn a_box_override_restyles_its_categorys_violin() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(direction);
    let mut series = violin_series("v", ViolinBody::Symmetric(half("sym", RED)), 0.0);
    let DataRenderType::BoxPlot { overrides, .. } = &mut series.render_type else {
        unreachable!()
    };
    let mut o = BoxPlotOverride::new(BoxPlotTarget::new("b", "v"));
    o.color = Some(BLUE);
    *overrides = Some(vec![o]);
    let img = r.export_panel_rgba(&chart, &[series], 1.0).unwrap();
    assert!(span_at(&img, &chart, direction, 0, 50.0, 1.0, is_red).is_some());
    assert!(span_at(&img, &chart, direction, 1, 50.0, 1.0, is_blue).is_some());
    assert_eq!(span_at(&img, &chart, direction, 1, 50.0, 1.0, is_red), None);
    assert!(span_at(&img, &chart, direction, 2, 50.0, 1.0, is_red).is_some());
}

/// A violin without a record draws no box, and an outline draws round the
/// body; the series validates without its `y_column`.
#[test]
fn a_violin_alone_draws_without_a_record_and_outlines_its_body() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(direction);
    let mut h = half("sym", RED);
    h.style.outline = true;
    h.style.outline_width = 2.0;
    h.style.outline_color = BLUE;
    let mut series = violin_series("v", ViolinBody::Symmetric(h), 0.25);
    series.y_column.clear();
    let img = r.export_panel_rgba(&chart, &[series], 1.0).unwrap();
    for k in 0..3 {
        let (centre, full) = member(&chart, direction, k, 1.0);
        assert_eq!(
            span_at(&img, &chart, direction, k, 50.0, 1.0, is_cyan),
            None
        );
        let blue = span_at(&img, &chart, direction, k, 50.0, 1.0, is_blue).expect("outline");
        assert!(
            (blue.1 as f32 - (centre + full)).abs() <= 2.5,
            "{k}: {blue:?}"
        );
        assert!(
            (blue.0 as f32 - (centre - full)).abs() <= 2.5,
            "{k}: {blue:?}"
        );
        assert!(span_at(&img, &chart, direction, k, 50.0, 1.0, is_red).is_some());
    }
}

/// The value axis fits the curves and the raw points, and a replaced
/// curve redraws; a density of the wrong length and a curve with the
/// wrong group count are errors naming them.
#[test]
fn the_fit_covers_curves_and_points_and_mismatched_columns_are_errors() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let mut chart = chart(direction);
    let series = vec![with_points(
        violin_series("v", ViolinBody::Symmetric(half("sym", RED)), 0.25),
        0.0,
        0.5,
    )];
    chart
        .auto_fit_layer_union(r.pool(), LayerRef::Base, &series, 0.0)
        .unwrap();
    let y = &chart.config().left_y;
    // The curve covers 10..90, the record 20..80, the points 30..70.
    assert!(
        y.min <= 10.0 && y.min > 0.0 && y.max >= 90.0 && y.max < 100.0,
        "{} {}",
        y.min,
        y.max
    );
    // A replaced curve, peaking at 20 instead, redraws.
    let chart = self::chart(direction);
    let low = triangle(20.0, 10.0);
    r.remove_column("sym.density").unwrap();
    upload_curve(&mut r, "sym", &[&low, &low, &low]);
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    assert_eq!(span_at(&img, &chart, direction, 0, 60.0, 1.0, is_red), None);
    assert!(span_at(&img, &chart, direction, 0, 20.0, 1.0, is_red).is_some());
    // A density one value short.
    r.remove_column("sym.density").unwrap();
    r.add_column("sym.density", &col(vec![0.5; 3 * 81 - 1]))
        .unwrap();
    let err = r
        .export_panel_rgba(&chart, &series, 1.0)
        .err()
        .expect("an error");
    assert!(err.to_string().contains("sym.density"), "{err}");
    // A curve of two groups for three categories.
    let sym = triangle(50.0, 40.0);
    r.remove_column("sym.density").unwrap();
    upload_curve(&mut r, "sym", &[&sym, &sym]);
    let err = r
        .export_panel_rgba(&chart, &series, 1.0)
        .err()
        .expect("an error");
    assert!(err.to_string().contains("groups"), "{err}");
}

/// A NaN in a curve breaks the body there: the segments either side of it
/// draw nothing, the rest of the body does.
#[test]
fn a_nan_breaks_the_body() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(direction);
    let sym = triangle(50.0, 40.0);
    let broken = |v: f64| {
        if (v - 50.0).abs() < 0.5 {
            f64::NAN
        } else {
            sym(v)
        }
    };
    r.remove_column("sym.density").unwrap();
    upload_curve(&mut r, "sym", &[&broken, &sym, &sym]);
    let series = [violin_series(
        "v",
        ViolinBody::Symmetric(half("sym", RED)),
        0.0,
    )];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    assert_eq!(span_at(&img, &chart, direction, 0, 50.0, 1.0, is_red), None);
    assert!(span_at(&img, &chart, direction, 0, 40.0, 1.0, is_red).is_some());
    assert!(span_at(&img, &chart, direction, 1, 50.0, 1.0, is_red).is_some());
}

/// Value labels sit past the farther of the box top and the curve's top,
/// and a violin without a box still labels its category.
#[test]
fn value_labels_clear_the_curve() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let mut chart = chart(direction);
    // Room above the curve's top for the label.
    chart.set_y_range(0.0, 150.0);
    let mut series = violin_series("v", ViolinBody::Symmetric(half("sym", RED)), 0.25);
    series.y_column.clear();
    let DataRenderType::BoxPlot { labels, .. } = &mut series.render_type else {
        unreachable!()
    };
    labels.placement = renderer::data_config::BoxPlotLabels::Median;
    labels.text = vec!["A".into(), "B".into(), "C".into()];
    let img = r.export_panel_rgba(&chart, &[series], 1.0).unwrap();
    let top = along(&chart, direction, 90.0, 1.0);
    for k in 0..3 {
        let (centre, full) = member(&chart, direction, k, 1.0);
        let mut ink_above = 0;
        let mut ink_on_curve = 0;
        for y in 0..img.height {
            for x in (centre - full) as u32..(centre + full) as u32 {
                let p = pixel(&img, x, y);
                if is_ink(p) && !is_red(p) {
                    if (y as f32) < top - 1.0 {
                        ink_above += 1;
                    } else {
                        ink_on_curve += u32::from(
                            (y as f32) < top + 30.0 && !is_red(p) && p[3] > 200 && p[0] < 100,
                        );
                    }
                }
            }
        }
        assert!(
            ink_above > 10,
            "{k}: the label above the curve top ({ink_above} px)"
        );
        assert_eq!(ink_on_curve, 0, "{k}: no label ink over the curve");
    }
}

/// Violins and raw points draw on overlays and subplots with box groups of
/// their own.
#[test]
fn overlays_and_subplots_draw_their_violins_and_points() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let mut parent = chart(direction);
    let mut overlay = renderer::OverlayLayer::default();
    overlay.y_axis.min = 0.0;
    overlay.y_axis.max = 200.0;
    overlay.y_axis.major_spacing = 40.0;
    overlay.series = vec!["over".into()];
    let mut sub = chart(direction).config().clone();
    sub.chart_area = ChartArea(Rect {
        x: 0,
        y: HEIGHT,
        width: WIDTH,
        height: HEIGHT,
    });
    let config = parent.config_mut();
    config.overlays.push(overlay);
    config.subplots.push(renderer::Subplot {
        config: sub,
        series: vec!["sub".into()],
        link_x: None,
        link_y: None,
    });
    let base = with_points(
        violin_series("base", ViolinBody::Symmetric(half("sym", RED)), 0.0),
        0.0,
        0.3,
    );
    let over = violin_series("over", ViolinBody::Symmetric(half("sym", BLUE)), 0.0);
    let sub = with_points(
        violin_series("sub", ViolinBody::Symmetric(half("sym", RED)), 0.0),
        0.0,
        0.3,
    );
    let img = r
        .export_document_rgba(&parent, &[base, over, sub], 1.0)
        .unwrap();
    let rows = |pred: fn(&[u8]) -> bool, y0: u32, y1: u32| {
        (y0..y1)
            .flat_map(|y| (0..img.width).map(move |x| (x, y)))
            .filter(|&(x, y)| pred(pixel(&img, x, y)))
            .count()
    };
    assert!(rows(is_red, 0, HEIGHT) > 1000, "the base violins");
    assert!(rows(is_blue, 0, HEIGHT) > 300, "the overlay's violins");
    assert!(rows(is_orange, 0, HEIGHT) > 100, "the base points");
    assert!(
        rows(is_red, HEIGHT, 2 * HEIGHT) > 1000,
        "the subplot's violins"
    );
    assert!(
        rows(is_orange, HEIGHT, 2 * HEIGHT) > 100,
        "the subplot's points"
    );
}

/// A 2x export is the 1x layout doubled: the same body, box and points,
/// scaled.
#[test]
fn a_2x_export_is_the_1x_layout_doubled() {
    let mut r = renderer();
    let direction = CategoryBarDirection::Vertical;
    let chart = chart(direction);
    let body = violin_series(
        "v",
        ViolinBody::Split {
            low: Some(half("lo", RED)),
            high: Some(half("hi", BLUE)),
        },
        0.25,
    );
    // The bodies and the box alone, then the points over them.
    let count = |img: &RasterImage, pred: fn(&[u8]) -> bool| {
        img.rgba.chunks(4).filter(|p| pred(p)).count() as f32
    };
    let parts: [(fn(&[u8]) -> bool, &str); 3] =
        [(is_red, "low"), (is_blue, "high"), (is_cyan, "box")];
    let one = r.export_panel_rgba(&chart, &[body.clone()], 1.0).unwrap();
    let two = r.export_panel_rgba(&chart, &[body.clone()], 2.0).unwrap();
    for (pred, name) in parts {
        let (a, b) = (count(&one, pred), count(&two, pred));
        assert!(a > 0.0, "{name} at 1x");
        assert!((b / a - 4.0).abs() < 0.4, "{name}: {a} px at 1x, {b} at 2x");
    }
    // Every point stays at its place doubled: the 2x centroid of the
    // orange ink is the 1x one doubled.
    let points = [with_points(body, 0.0, 0.6)];
    let one = r.export_panel_rgba(&chart, &points, 1.0).unwrap();
    let two = r.export_panel_rgba(&chart, &points, 2.0).unwrap();
    let centroid = |img: &RasterImage| {
        let (mut x, mut y, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for (i, p) in img.rgba.chunks(4).enumerate() {
            if is_orange(p) {
                x += (i as u32 % img.width) as f64 + 0.5;
                y += (i as u32 / img.width) as f64 + 0.5;
                n += 1.0;
            }
        }
        (x / n, y / n, n)
    };
    let (x1, y1, n1) = centroid(&one);
    let (x2, y2, n2) = centroid(&two);
    assert!(n1 > 100.0 && (n2 / n1 - 4.0).abs() < 1.0, "{n1} {n2}");
    assert!(
        (x2 - 2.0 * x1).abs() < 2.0 && (y2 - 2.0 * y1).abs() < 2.0,
        "({x1}, {y1}) ({x2}, {y2})"
    );
}

/// No count caps a violin or its points (rule 10): 4 096-point curves over
/// 600 categories (past the 64 categories and the 1 024 curve points the
/// plan once proposed) draw and pick, and a million raw points draw and
/// pick through the point picker, the cost measured.
#[test]
fn long_curves_many_categories_and_a_million_points_draw_and_pick() {
    const N: usize = 600;
    const POINTS: usize = 4096;
    const WIDE: u32 = 4800;
    let mut r = renderer();
    let mut values = Vec::with_capacity(N * POINTS);
    let mut density = Vec::with_capacity(N * POINTS);
    let mut offsets = vec![0.0];
    for _ in 0..N {
        for j in 0..POINTS {
            let v = 10.0 + 80.0 * j as f64 / (POINTS - 1) as f64;
            values.push(v);
            density.push((1.0 - (v - 50.0).abs() / 40.0).max(0.0));
        }
        offsets.push(values.len() as f64);
    }
    r.upsert_ragged(
        "long",
        &RaggedSource {
            values: &col(values),
            offsets: &col(offsets),
        },
    )
    .unwrap();
    r.add_column("long.density", &col(density)).unwrap();
    r.add_column("long.pos", &col((0..N).map(|k| k as f64).collect()))
        .unwrap();
    let mut chart = chart(CategoryBarDirection::Vertical);
    let config = chart.config_mut();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDE,
        height: HEIGHT,
    });
    config.bottom_x.categories = Some(
        (0..N)
            .map(|k| Category::new(format!("c{k}"), format!("C{k}")))
            .collect(),
    );
    let mut h = half("long", RED);
    h.density = "long.density".into();
    let mut series = violin_series("v", ViolinBody::Symmetric(h), 0.0);
    series.x_column = "long.pos".into();
    series.y_column.clear();
    let series = [series];
    let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    let da = chart.config().data_area().unwrap();
    let y = along(&chart, CategoryBarDirection::Vertical, 50.0, 1.0) as u32;
    for k in [0, 63, 64, 599] {
        let x = (da.x as f32 + da.width as f32 * (k as f32 + 0.5) / N as f32) as u32;
        assert!(is_red(pixel(&img, x, y)), "violin {k} paints");
    }
    let x = da.x as f32 + da.width as f32 * (N as f32 - 0.5) / N as f32;
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    let request = renderer::GpuPickRequest {
        canvas_position_px: [x, y as f32 + 0.5],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: WIDE,
            height: HEIGHT,
        },
        display_scale: 1.0,
        max_distance_px: 0.0,
    };
    let hit = pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    assert!(
        matches!(&hit, Some(renderer::PickedData::HistogramBin { bin_index, .. }) if *bin_index == N - 1),
        "{hit:?}"
    );
    let started = std::time::Instant::now();
    let _ = pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    eprintln!(
        "violin pick over {N} categories of {POINTS} curve points: {:?}",
        started.elapsed()
    );
    r.remove_chart(id).unwrap();

    // A million and three million raw points over the three categories.
    let chart = self::chart(CategoryBarDirection::Vertical);
    for count in [1_000_000usize, 3_000_000] {
        for id in ["big.cat", "big.value"] {
            let _ = r.remove_column(id);
        }
        r.add_column(
            "big.cat",
            &col((0..count).map(|i| (i % 3) as f64).collect()),
        )
        .unwrap();
        r.add_column(
            "big.value",
            &col((0..count).map(|i| 20.0 + (i % 997) as f64 * 0.06).collect()),
        )
        .unwrap();
        let mut series = violin_series("v", ViolinBody::Symmetric(half("sym", RED)), 0.0);
        let DataRenderType::BoxPlot { points, .. } = &mut series.render_type else {
            unreachable!()
        };
        *points = Some(BoxPoints {
            positions: "big.cat".into(),
            values: "big.value".into(),
            style: BoxPointsStyle {
                color: ORANGE,
                size: 2.0,
                jitter: 0.9,
                ..BoxPointsStyle::default()
            },
        });
        let series = [series];
        let started = std::time::Instant::now();
        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let draw = started.elapsed();
        assert!(img.rgba.chunks(4).filter(|p| is_orange(p)).count() > 1000);
        r.enable_gpu_picking().unwrap();
        let id = r
            .register_chart(chart.config().clone(), series.to_vec())
            .unwrap();
        let (centre, _) = member(&chart, CategoryBarDirection::Vertical, 1, 1.0);
        let request = renderer::GpuPickRequest {
            canvas_position_px: [
                centre,
                along(&chart, CategoryBarDirection::Vertical, 50.0, 1.0),
            ],
            display_panel_px: Rect {
                x: 0,
                y: 0,
                width: WIDTH,
                height: HEIGHT,
            },
            display_scale: 1.0,
            max_distance_px: 4.0,
        };
        let _ = pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
        let started = std::time::Instant::now();
        let hit = pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
        let pick = started.elapsed();
        assert!(
            matches!(&hit, Some(renderer::PickedData::Point { .. })),
            "a point under the cursor among {count}: {hit:?}"
        );
        eprintln!("{count} raw points: export {draw:?}, pick {pick:?}");
        r.remove_chart(id).unwrap();
    }
}

/// A split violin's pick names the half under the cursor, a symmetric
/// one's none; selecting a half rings that half alone, without the box's
/// ring, and selecting the category rings both halves.
#[test]
fn a_split_violin_picks_and_highlights_each_half() {
    let mut r = renderer();
    for direction in [
        CategoryBarDirection::Vertical,
        CategoryBarDirection::Horizontal,
    ] {
        let mut chart = chart(direction);
        let series = [violin_series(
            "v",
            ViolinBody::Split {
                low: Some(half("lo", RED)),
                high: Some(half("hi", BLUE)),
            },
            0.25,
        )];
        let (centre, full) = member(&chart, direction, 2, 1.0);
        // Low peaks at 35 on the low side, high at 65 on the high side.
        let low_sign = match direction {
            CategoryBarDirection::Vertical => -1.0,
            CategoryBarDirection::Horizontal => 1.0,
        };
        let low = xy(
            direction,
            centre + low_sign * full * 0.6,
            along(&chart, direction, 35.0, 1.0),
        );
        let high = xy(
            direction,
            centre - low_sign * full * 0.6,
            along(&chart, direction, 65.0, 1.0),
        );
        for (at, want) in [(low, ViolinSide::Low), (high, ViolinSide::High)] {
            let hit = pick(&mut r, &chart, &series, 1.0, (at.0 + 0.5, at.1 + 0.5));
            assert!(
                matches!(
                    &hit,
                    Some(renderer::PickedData::HistogramBin { bin_index: 2, half: Some(h), .. })
                        if *h == want
                ),
                "{direction:?}: {want:?} picked {hit:?}"
            );
        }
        // Inside the box, which is painted over the violin: the box.
        let inside = xy(direction, centre, along(&chart, direction, 50.0, 1.0));
        let hit = pick(
            &mut r,
            &chart,
            &series,
            1.0,
            (inside.0 + 0.5, inside.1 + 0.5),
        );
        assert!(
            matches!(
                &hit,
                Some(renderer::PickedData::HistogramBin {
                    bin_index: 2,
                    half: None,
                    ..
                })
            ),
            "{direction:?}: the box: {hit:?}"
        );
        let select = |half| DataSelectionsConfig {
            refs: vec![PickedDataRef::HistogramBin {
                source_id: None,
                series_id: "v".into(),
                bin_index: 2,
                half,
            }],
            highlight_color: Color::from_rgb8(255, 0, 255),
            outline_width_px: 2.0,
            ..Default::default()
        };
        // Selecting the low half rings it alone, not the high half nor the
        // box.
        chart.config_mut().picked_data = Some(select(Some(ViolinSide::Low)));
        let ringed = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        assert!(
            span_at(&ringed, &chart, direction, 2, 35.0, 1.0, is_magenta).is_some(),
            "{direction:?}: the low half's ring"
        );
        assert_eq!(
            span_at(&ringed, &chart, direction, 2, 65.0, 1.0, is_magenta),
            None,
            "{direction:?}: not the high half's"
        );
        // The box's edge on the high side, where only a box ring could be.
        let box_edge = centre - low_sign * full * 0.25;
        let ring_px = (-2..=2).any(|d| {
            let (x, y) = xy(
                direction,
                box_edge + d as f32,
                along(&chart, direction, 50.0, 1.0),
            );
            is_magenta(pixel(&ringed, x as u32, y as u32))
        });
        assert!(!ring_px, "{direction:?}: the box is not ringed");
        // The whole category: both halves.
        chart.config_mut().picked_data = Some(select(None));
        let ringed = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        assert!(span_at(&ringed, &chart, direction, 2, 35.0, 1.0, is_magenta).is_some());
        assert!(span_at(&ringed, &chart, direction, 2, 65.0, 1.0, is_magenta).is_some());
    }
    // A symmetric violin names no half.
    let chart = chart(CategoryBarDirection::Vertical);
    let series = [violin_series(
        "v",
        ViolinBody::Symmetric(half("sym", RED)),
        0.0,
    )];
    let (centre, full) = member(&chart, CategoryBarDirection::Vertical, 0, 1.0);
    let at = (
        centre - full * 0.5,
        along(&chart, CategoryBarDirection::Vertical, 50.0, 1.0),
    );
    let hit = pick(&mut r, &chart, &series, 1.0, (at.0 + 0.5, at.1 + 0.5));
    assert!(
        matches!(
            &hit,
            Some(renderer::PickedData::HistogramBin {
                bin_index: 0,
                half: None,
                ..
            })
        ),
        "{hit:?}"
    );
}
