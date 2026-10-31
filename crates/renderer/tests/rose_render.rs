//! End-to-end pixel verification of category bars on a polar chart, a rose
//! (`DataRenderType::CategoryBars` with `Config::polar`, #133), through the
//! public renderer API and the headless `export_panel_rgba`: grouped and
//! stacked wedges at 1x and 2x, the gap between grouped wedges, the inner
//! ring and an inverted radius axis, the pick and `step_selection`, the
//! picked highlight, the value labels, the refused horizontal group and a
//! rose of four thousand categories.
//!
//! The first series is pure red, the second pure blue, both flat with no
//! outline and square corners, so a colour predicate attributes every pixel
//! to one series. Expected positions come from the chart's own
//! `polar_geometry` and `polar_angle_map`, the circle and the angle mapping
//! the renderer draws with, and the slot rule of the rectangular bar: the
//! group covers the centred `group_width` of a category's angular slot, its
//! wedges side by side in the categories' order, `bar_gap` pixels apart at
//! the ring.

#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::{Arc, OnceLock};

use renderer::config::{Category, TickVisibility};
use renderer::data::Column;
use renderer::data_config::{
    BarGroups, CategoryBarDirection, CategoryBarLabels, CategoryBarMode, CategoryBarStyle,
};
use renderer::layout::{ChartArea, Rect};
use renderer::polar::PolarOptions;
use renderer::{
    Chart, Color, ColumnSource, DataRenderType, DataSelectionsConfig, Material, PickedDataRef,
    RasterImage, Renderer, RendererDevice, SeriesConfig, Subplot,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 480;
const RED: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};
const BLUE: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};
const MAGENTA: Color = Color {
    r: 1.0,
    g: 0.0,
    b: 1.0,
    a: 1.0,
};
/// Values of the two series over the four categories.
const ONE: [f64; 4] = [10.0, 20.0, 5.0, 15.0];
const TWO: [f64; 4] = [20.0, 10.0, 15.0, 5.0];
const GROUP_WIDTH: f32 = 0.8;
const BAR_GAP: f32 = 4.0;

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

/// A renderer holding the position column `0..n` and the value columns,
/// uploaded as one batch.
fn renderer(columns: &[(&str, &[f64])]) -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    let n = columns[0].1.len();
    r.add_column("pos", &col((0..n).map(|i| i as f64).collect()))
        .unwrap();
    let values: Vec<Column<f64>> = columns.iter().map(|(_, v)| col(v.to_vec())).collect();
    let batch: Vec<(&str, &dyn ColumnSource)> = columns
        .iter()
        .zip(&values)
        .map(|((id, _), c)| (*id, c as &dyn ColumnSource))
        .collect();
    r.add_columns(&batch).unwrap();
    r
}

fn flat_style() -> CategoryBarStyle {
    CategoryBarStyle {
        material: Material::Flat,
        corner_radius: 0.0,
        outline: false,
        ..CategoryBarStyle::default()
    }
}

fn bars(id: &str, color: Color) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        z_column: None,
        label: None,
        x_column: "pos".into(),
        y_column: id.into(),
        render_type: DataRenderType::CategoryBars {
            color,
            style: flat_style(),
            labels: Default::default(),
            label_format: Default::default(),
            overrides: None,
        },
    }
}

fn categories(n: usize) -> Vec<Category> {
    (0..n)
        .map(|i| Category::new(format!("c{i}"), format!("Cat {i}")))
        .collect()
}

/// A polar chart with `n` categories on the angle axis, the first one up
/// and the rest clockwise, the radius over `value`; no grid, chrome or
/// legend: the only ink is the series (and the value labels when a test
/// asks for them).
fn chart(mode: CategoryBarMode, n: usize, value: (f64, f64)) -> Chart {
    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.legend.visible = false;
    config.chart_title.visible = false;
    config.grid.show_major_x = false;
    config.grid.show_major_y = false;
    config.grid.show_minor_x = false;
    config.grid.show_minor_y = false;
    for axis in [&mut config.bottom_x, &mut config.left_y] {
        axis.label_style.visible = false;
        axis.line_visible = false;
        axis.tick = TickVisibility::None;
        axis.title_option.visible = false;
    }
    config.bottom_x.categories = Some(categories(n));
    config.polar = Some(PolarOptions {
        zero_angle_degrees: 90.0,
        clockwise: true,
        ..PolarOptions::default()
    });
    config.bar_groups = Some(BarGroups {
        mode,
        direction: CategoryBarDirection::Vertical,
        group_width: GROUP_WIDTH,
        bar_gap: BAR_GAP,
    });
    let mut chart = Chart::new(config);
    chart.set_y_range(value.0, value.1);
    chart
}

/// The ring's centre, radius and inner radius in export pixels.
fn geometry(chart: &Chart, scale: f32) -> ((f64, f64), f64, f64) {
    let g = chart
        .config()
        .scaled(scale)
        .polar_geometry()
        .expect("geometry");
    (
        (f64::from(g.center[0]), f64::from(g.center[1])),
        f64::from(g.radius),
        f64::from(g.inner_radius),
    )
}

/// The radius in export pixels of value `v` on the radius axis.
fn radius_px(chart: &Chart, scale: f32, v: f64) -> f64 {
    let (_, outer, inner) = geometry(chart, scale);
    let axis = &chart.config().left_y;
    let t = (v - axis.min) / (axis.max - axis.min);
    inner + (outer - inner) * t
}

/// The screen angle (radians, counter-clockwise with y up) of position
/// `x` on the angle axis.
fn angle(chart: &Chart, x: f64) -> f64 {
    chart
        .config()
        .polar_angle_map()
        .expect("polar")
        .screen_angle(x)
}

/// The angles `(low, high)` of member `m` of `members` in category `k`'s
/// slot, by the bar rule: the centred `group_width` of the slot, grouped
/// wedges side by side from the slot edge the axis arrives from, `bar_gap`
/// pixels apart at the ring.
fn wedge_angles(chart: &Chart, scale: f32, k: usize, m: usize, members: usize) -> (f64, f64) {
    let (_, ring, _) = geometry(chart, scale);
    let a_low = angle(chart, k as f64 - 0.5);
    let a_high = angle(chart, k as f64 + 0.5);
    let dir = (a_high - a_low).signum();
    let slot = (a_high - a_low).abs();
    let span = slot * f64::from(GROUP_WIDTH);
    let gap = f64::from(BAR_GAP * scale) / ring;
    let thickness = (span - gap * (members as f64 - 1.0)) / members as f64;
    let from = a_low + dir * ((slot - span) * 0.5 + m as f64 * (thickness + gap));
    let to = from + dir * thickness;
    (from.min(to), from.max(to))
}

/// The middle angle of a wedge.
fn mid_angle(chart: &Chart, scale: f32, k: usize, m: usize, members: usize) -> f64 {
    let (a, b) = wedge_angles(chart, scale, k, m, members);
    (a + b) * 0.5
}

/// The export pixel at screen angle `a` and radius `r` from the centre.
fn at(centre: (f64, f64), a: f64, r: f64) -> (f64, f64) {
    (centre.0 + a.cos() * r, centre.1 - a.sin() * r)
}

fn pixel(img: &RasterImage, (x, y): (f64, f64)) -> [u8; 4] {
    let x = (x.floor() as i64).clamp(0, img.width as i64 - 1) as u32;
    let y = (y.floor() as i64).clamp(0, img.height as i64 - 1) as u32;
    let i = ((y * img.width + x) * 4) as usize;
    [
        img.rgba[i],
        img.rgba[i + 1],
        img.rgba[i + 2],
        img.rgba[i + 3],
    ]
}

fn is_red(p: [u8; 4]) -> bool {
    p[3] > 200 && p[0] > 200 && p[1] < 60 && p[2] < 60
}
fn is_blue(p: [u8; 4]) -> bool {
    p[3] > 200 && p[2] > 200 && p[0] < 60 && p[1] < 60
}
fn is_magenta(p: [u8; 4]) -> bool {
    p[3] > 200 && p[0] > 200 && p[2] > 200 && p[1] < 60
}
fn is_empty(p: [u8; 4]) -> bool {
    p[3] < 16
}
/// A value label's glyph pixel outside a bar: dark ink.
fn is_dark(p: [u8; 4]) -> bool {
    p[3] > 120 && p[0] < 110 && p[1] < 110 && p[2] < 110
}
/// A value label's glyph pixel inside a bar: light ink over the fill (the
/// background is transparent, so an opaque white pixel is ink).
fn is_light(p: [u8; 4]) -> bool {
    p[3] > 120 && p[0] > 200 && p[1] > 200 && p[2] > 200
}

/// Bounding box `(x0, y0, x1, y1)` (inclusive) of the pixels `pred`
/// accepts.
fn bbox(img: &RasterImage, pred: fn([u8; 4]) -> bool) -> Option<(u32, u32, u32, u32)> {
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for y in 0..img.height {
        for x in 0..img.width {
            if pred(pixel(img, (f64::from(x), f64::from(y)))) {
                let b = found.get_or_insert((x, y, x, y));
                *b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        }
    }
    found
}

/// Whether any pixel within `radius` of `centre` satisfies `pred`.
fn any_within(
    img: &RasterImage,
    centre: (f64, f64),
    radius: f64,
    pred: fn([u8; 4]) -> bool,
) -> bool {
    let n = radius.ceil() as i64;
    (-n..=n)
        .any(|dy| (-n..=n).any(|dx| pred(pixel(img, (centre.0 + dx as f64, centre.1 + dy as f64)))))
}

fn export(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], scale: f32) -> RasterImage {
    r.export_panel_rgba(chart, series, scale).unwrap()
}

/// Writes the image when `FIGGY_INTERACTION_ARTIFACTS` names a directory.
fn save(name: &str, image: &RasterImage) {
    if let Ok(path) = std::env::var("FIGGY_INTERACTION_ARTIFACTS") {
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            std::path::Path::new(&path).join(format!("{name}.png")),
            renderer::encode_png(image).unwrap(),
        )
        .unwrap();
    }
}

fn pick_request(x: f64, y: f64, w: u32, h: u32, scale: f32) -> renderer::GpuPickRequest {
    renderer::GpuPickRequest {
        canvas_position_px: [x as f32, y as f32],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        },
        display_scale: scale,
        max_distance_px: 0.0,
    }
}

/// Grouped wedges span zero to their value side by side in their
/// category's slot; stacked wedges sit on the earlier series. Along each
/// wedge's middle direction the fill holds between its start and end radii
/// and stops past them, and across its middle radius it stops past its
/// edge angles. At 1x and 2x.
#[test]
fn wedges_span_their_values_in_both_modes_and_scales() {
    let mut r = renderer(&[("one", &ONE), ("two", &TWO)]);
    let series = [bars("one", RED), bars("two", BLUE)];
    for mode in [CategoryBarMode::Grouped, CategoryBarMode::Stacked] {
        let range = match mode {
            CategoryBarMode::Grouped => (0.0, 25.0),
            _ => (0.0, 40.0),
        };
        let chart = chart(mode, 4, range);
        for scale in [1.0f32, 2.0] {
            let img = export(&mut r, &chart, &series, scale);
            save(&format!("rose-{mode:?}-{scale}x"), &img);
            let (centre, _, _) = geometry(&chart, scale);
            let members = match mode {
                CategoryBarMode::Grouped => 2,
                _ => 1,
            };
            for k in 0..4 {
                let (one, two) = (ONE[k], TWO[k]);
                let spans: [(usize, (f64, f64), fn([u8; 4]) -> bool); 2] = match mode {
                    CategoryBarMode::Grouped => [(0, (0.0, one), is_red), (1, (0.0, two), is_blue)],
                    _ => [(0, (0.0, one), is_red), (0, (one, one + two), is_blue)],
                };
                for (m, (from, to), pred) in spans {
                    let case = format!("{mode:?} {scale}x category {k} member {m}");
                    let (r0, r1) = (radius_px(&chart, scale, from), radius_px(&chart, scale, to));
                    let mid = mid_angle(&chart, scale, k, m, members);
                    let slack = 2.5 * f64::from(scale);
                    // Along the middle direction: filled between the radii.
                    for t in [0.2, 0.5, 0.8] {
                        let p = at(centre, mid, r0 + (r1 - r0) * t);
                        assert!(
                            pred(pixel(&img, p)),
                            "{case}: fill at {t} of the depth, {p:?}"
                        );
                    }
                    assert!(
                        pred(pixel(&img, at(centre, mid, r1 - slack))),
                        "{case}: fill just inside the end radius"
                    );
                    assert!(
                        !pred(pixel(&img, at(centre, mid, r1 + slack))),
                        "{case}: no fill past the end radius"
                    );
                    if r0 > slack * 2.0 {
                        assert!(
                            pred(pixel(&img, at(centre, mid, r0 + slack))),
                            "{case}: fill just past the start radius"
                        );
                        assert!(
                            !pred(pixel(&img, at(centre, mid, r0 - slack))),
                            "{case}: no fill before the start radius"
                        );
                    }
                    // Across the middle radius: filled inside the edge
                    // angles, not outside them.
                    let rm = (r0 + r1) * 0.5;
                    let (a, b) = wedge_angles(&chart, scale, k, m, members);
                    let da = slack / rm;
                    for (inside, outside) in [(a + da, a - da), (b - da, b + da)] {
                        assert!(
                            pred(pixel(&img, at(centre, inside, rm))),
                            "{case}: fill inside the edge angle"
                        );
                        assert!(
                            !pred(pixel(&img, at(centre, outside, rm))),
                            "{case}: no fill outside the edge angle"
                        );
                    }
                }
            }
        }
    }
}

/// Grouped wedges that reach the ring leave `bar_gap` pixels of background
/// between them along the ring, and the group leaves the slot's margins
/// outside `group_width` empty.
#[test]
fn grouped_wedges_leave_bar_gap_pixels_between_them_at_the_ring() {
    let full = [20.0; 4];
    let mut r = renderer(&[("one", &full), ("two", &full)]);
    let series = [bars("one", RED), bars("two", BLUE)];
    let chart = chart(CategoryBarMode::Grouped, 4, (0.0, 20.0));
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &series, scale);
        save(&format!("rose-gap-{scale}x"), &img);
        let (centre, ring, _) = geometry(&chart, scale);
        // Three pixels inside the ring: clear of the ring's own edge.
        let rr = ring - 3.0 * f64::from(scale);
        for k in 0..4 {
            let case = format!("{scale}x category {k}");
            let red = mid_angle(&chart, scale, k, 0, 2);
            let blue = mid_angle(&chart, scale, k, 1, 2);
            assert!(is_red(pixel(&img, at(centre, red, rr))), "{case}: red");
            assert!(is_blue(pixel(&img, at(centre, blue, rr))), "{case}: blue");
            // Walk the arc from the red middle to the blue middle in
            // quarter-pixel steps: the background run is the gap.
            let steps = 4000;
            let mut empty = 0usize;
            for i in 0..=steps {
                let a = red + (blue - red) * i as f64 / steps as f64;
                if is_empty(pixel(&img, at(centre, a, rr))) {
                    empty += 1;
                }
            }
            let arc = (blue - red).abs() * rr;
            let measured = empty as f64 / steps as f64 * arc;
            let want = f64::from(BAR_GAP * scale) * rr / ring;
            assert!(
                (measured - want).abs() <= 1.5 * f64::from(scale),
                "{case}: a gap of {measured:.2} px along the arc, expected {want:.2}"
            );
            // The slot's margins outside the group are empty, the group's
            // outer edges filled.
            let (a0, b0) = wedge_angles(&chart, scale, k, 0, 2);
            let (a1, b1) = wedge_angles(&chart, scale, k, 1, 2);
            let (first, last) = (a0.min(a1), b0.max(b1));
            let margin = (angle(&chart, k as f64 + 0.5) - angle(&chart, k as f64 - 0.5)).abs()
                * f64::from(1.0 - GROUP_WIDTH)
                * 0.5;
            for edge in [first, last] {
                let out = if edge == first {
                    edge - margin * 0.5
                } else {
                    edge + margin * 0.5
                };
                assert!(
                    is_empty(pixel(&img, at(centre, out, rr))),
                    "{case}: the slot margin is empty"
                );
            }
        }
    }
}

/// A stack grows from the inner ring outward; on an inverted radius axis
/// it grows from the ring inward, the exposed end at the inner arc.
#[test]
fn a_stack_grows_from_the_inner_ring_and_inward_on_an_inverted_axis() {
    let mut r = renderer(&[("one", &ONE), ("two", &TWO)]);
    let series = [bars("one", RED), bars("two", BLUE)];
    let mut chart = chart(CategoryBarMode::Stacked, 4, (0.0, 40.0));
    chart
        .config_mut()
        .polar
        .as_mut()
        .unwrap()
        .inner_radius_fraction = 0.3;
    let img = export(&mut r, &chart, &series, 1.0);
    save("rose-inner-ring", &img);
    let (centre, ring, inner) = geometry(&chart, 1.0);
    assert!(
        inner > ring * 0.25,
        "an inner ring of {inner} px in a ring of {ring}"
    );
    for k in 0..4 {
        let mid = mid_angle(&chart, 1.0, k, 0, 1);
        assert!(
            is_empty(pixel(&img, at(centre, mid, inner * 0.5))),
            "category {k}: hollow centre"
        );
        let r_one = radius_px(&chart, 1.0, ONE[k]);
        assert!(
            is_red(pixel(&img, at(centre, mid, (inner + r_one) * 0.5))),
            "category {k}: red from the inner ring"
        );
        assert!(
            is_blue(pixel(
                &img,
                at(
                    centre,
                    mid,
                    (r_one + radius_px(&chart, 1.0, ONE[k] + TWO[k])) * 0.5
                )
            )),
            "category {k}: blue on the red"
        );
    }
    chart
        .config_mut()
        .polar
        .as_mut()
        .unwrap()
        .inner_radius_fraction = 0.0;
    chart.set_y_range(40.0, 0.0);
    let img = export(&mut r, &chart, &series, 1.0);
    save("rose-inverted", &img);
    let (centre, ring, _) = geometry(&chart, 1.0);
    for k in 0..4 {
        let mid = mid_angle(&chart, 1.0, k, 0, 1);
        // Value 0 at the ring, the values inward.
        let r_one = radius_px(&chart, 1.0, ONE[k]);
        let r_two = radius_px(&chart, 1.0, ONE[k] + TWO[k]);
        assert!(
            r_one < ring && r_two < r_one,
            "category {k}: inverted radii"
        );
        assert!(
            is_red(pixel(&img, at(centre, mid, (ring + r_one) * 0.5))),
            "category {k}: red from the ring"
        );
        assert!(
            is_blue(pixel(&img, at(centre, mid, (r_one + r_two) * 0.5))),
            "category {k}: blue inside the red"
        );
        assert!(
            is_empty(pixel(&img, at(centre, mid, r_two - 3.0))),
            "category {k}: nothing inside the stack"
        );
    }
}

/// The data pick names the wedge under the cursor (its series and category
/// row), nothing in a gap or past a wedge's end, and `step_selection` walks
/// the categories.
#[test]
fn the_pick_answers_the_wedge_and_the_selection_steps_the_categories() {
    let mut r = renderer(&[("one", &ONE), ("two", &TWO)]);
    let series = [bars("one", RED), bars("two", BLUE)];
    let chart = chart(CategoryBarMode::Grouped, 4, (0.0, 25.0));
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    let (centre, _, _) = geometry(&chart, 1.0);
    let pick = |r: &mut Renderer, (x, y): (f64, f64)| {
        let request = pick_request(x, y, WIDTH, HEIGHT, 1.0);
        match pollster::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap() {
            Some(renderer::PickedData::HistogramBin {
                series_id,
                bin_index,
                ..
            }) => Some((series_id, bin_index)),
            _ => None,
        }
    };
    for k in 0..4 {
        for (m, name, value) in [(0, "one", ONE[k]), (1, "two", TWO[k])] {
            let mid = mid_angle(&chart, 1.0, k, m, 2);
            let r_end = radius_px(&chart, 1.0, value);
            assert_eq!(
                pick(&mut r, at(centre, mid, r_end * 0.5)),
                Some((name.to_string(), k)),
                "category {k} {name}"
            );
            assert_eq!(
                pick(&mut r, at(centre, mid, r_end + 4.0)),
                None,
                "category {k} {name}: past the end"
            );
        }
        // Between the two wedges, at the middle of the gap.
        let (_, b0) = wedge_angles(&chart, 1.0, k, 0, 2);
        let (a1, _) = wedge_angles(&chart, 1.0, k, 1, 2);
        let gap_mid = (b0 + a1) * 0.5;
        let r_min = radius_px(&chart, 1.0, ONE[k].min(TWO[k]));
        assert_eq!(
            pick(&mut r, at(centre, gap_mid, r_min * 0.5)),
            None,
            "category {k}: the gap"
        );
    }
    let mut stepped = chart.config().clone();
    stepped.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::HistogramBin {
            source_id: None,
            series_id: "one".into(),
            bin_index: 0,
            half: None,
        }],
        ..DataSelectionsConfig::default()
    });
    let len = |_: &str| Some(4usize);
    assert_eq!(stepped.step_selection(&series, 1, 0, len), Ok(true));
    assert!(matches!(
        stepped.picked_data.as_ref().unwrap().refs[0],
        PickedDataRef::HistogramBin { bin_index: 1, .. }
    ));
}

/// The signed distance in pixels from `p` to the annular sector between
/// `r0` and `r1` over the screen angles `(a, b)`.
fn sector_distance(p: (f64, f64), centre: (f64, f64), r0: f64, r1: f64, (a, b): (f64, f64)) -> f64 {
    let (dx, dy) = (p.0 - centre.0, centre.1 - p.1);
    let r = (dx * dx + dy * dy).sqrt();
    let phi = dy.atan2(dx);
    let mid = (a + b) * 0.5;
    // The angle relative to the middle, wrapped to (-π, π].
    let rel =
        (phi - mid + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI;
    let half = (b - a) * 0.5;
    let radial = (r0 - r).max(r - r1);
    if rel.abs() <= half {
        // Inside: the nearer of the arcs and the edge lines.
        return radial.max(-(half - rel.abs()).sin() * r);
    }
    // Outside the angular range: the distance to the nearer edge segment.
    let edge = if rel > 0.0 { b } else { a };
    let (ex, ey) = (edge.cos(), edge.sin());
    let along = (dx * ex + dy * ey).clamp(r0, r1);
    let (qx, qy) = (ex * along, ey * along);
    ((dx - qx).powi(2) + (dy - qy).powi(2)).sqrt()
}

/// The picked highlight outlines the selected wedge along its two arcs and
/// two edges, within the outline width of its boundary, and no other.
#[test]
fn the_picked_highlight_outlines_the_wedge() {
    let mut r = renderer(&[("one", &ONE), ("two", &TWO)]);
    let series = [bars("one", RED), bars("two", BLUE)];
    let mut chart = chart(CategoryBarMode::Grouped, 4, (0.0, 25.0));
    for scale in [1.0f32, 2.0] {
        chart.config_mut().picked_data = Some(DataSelectionsConfig {
            visible: true,
            refs: vec![PickedDataRef::HistogramBin {
                source_id: None,
                series_id: "two".into(),
                bin_index: 1,
                half: None,
            }],
            highlight_color: MAGENTA,
            outline_width_px: 3.0,
            point_radius_extra_px: 0.0,
            contour_width_extra_px: 0.0,
        });
        let img = export(&mut r, &chart, &series, scale);
        save(&format!("rose-highlight-{scale}x"), &img);
        let (centre, _, inner) = geometry(&chart, scale);
        let r1 = radius_px(&chart, scale, TWO[1]);
        let angles = wedge_angles(&chart, scale, 1, 1, 2);
        let band = 3.0 * f64::from(scale);
        let ring = bbox(&img, is_magenta).expect("the selected wedge is outlined");
        let mut count = 0usize;
        let mut on_outer = false;
        let mut on_inner = false;
        let mut on_edges = [false, false];
        for y in ring.1..=ring.3 {
            for x in ring.0..=ring.2 {
                let p = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                if !is_magenta(pixel(&img, p)) {
                    continue;
                }
                count += 1;
                let d = sector_distance(p, centre, inner, r1, angles);
                assert!(
                    d <= 1.0 && d >= -(band + 1.0),
                    "{scale}x: a highlight pixel at {p:?} is {d:.2} px from the wedge's boundary"
                );
                let (dx, dy) = (p.0 - centre.0, centre.1 - p.1);
                let rr = (dx * dx + dy * dy).sqrt();
                on_outer |= (r1 - rr).abs() <= band + 1.0;
                on_inner |= (rr - inner).abs() <= band + 1.0;
                let phi = dy.atan2(dx);
                on_edges[0] |= ((phi - angles.0 + std::f64::consts::PI)
                    .rem_euclid(std::f64::consts::TAU)
                    - std::f64::consts::PI)
                    .abs()
                    * rr
                    <= band + 1.0;
                on_edges[1] |= ((phi - angles.1 + std::f64::consts::PI)
                    .rem_euclid(std::f64::consts::TAU)
                    - std::f64::consts::PI)
                    .abs()
                    * rr
                    <= band + 1.0;
            }
        }
        assert!(count > 20, "{scale}x: {count} highlight pixels");
        assert!(
            on_outer && on_edges[0] && on_edges[1],
            "{scale}x: the outline runs along the outer arc and both edges"
        );
        // A wedge from the centre has no inner arc to outline, only its apex.
        let _ = on_inner;
        // Nothing of the other wedges is highlighted: the box of the
        // highlight is the wedge's.
        let wedge_box = {
            let mut lo = (f64::INFINITY, f64::INFINITY);
            let mut hi = (f64::NEG_INFINITY, f64::NEG_INFINITY);
            for i in 0..=64 {
                let a = angles.0 + (angles.1 - angles.0) * i as f64 / 64.0;
                for rr in [inner, r1] {
                    let q = at(centre, a, rr);
                    lo = (lo.0.min(q.0), lo.1.min(q.1));
                    hi = (hi.0.max(q.0), hi.1.max(q.1));
                }
            }
            (lo, hi)
        };
        let slack = band + 2.0;
        assert!(
            f64::from(ring.0) >= wedge_box.0.0 - slack
                && f64::from(ring.1) >= wedge_box.0.1 - slack
                && f64::from(ring.2) <= wedge_box.1.0 + slack
                && f64::from(ring.3) <= wedge_box.1.1 + slack,
            "{scale}x: the highlight {ring:?} stays within the wedge's box {wedge_box:?}"
        );
    }
}

/// Value labels sit past a wedge's exposed arc along its middle direction
/// (`Outside`, dark ink), or at its middle radius (`Inside`, light ink over
/// the fill); `None` draws none.
#[test]
fn value_labels_sit_past_the_arc_or_at_the_middle_radius() {
    let mut r = renderer(&[("one", &ONE), ("two", &TWO)]);
    let chart = chart(CategoryBarMode::Grouped, 4, (0.0, 25.0));
    let (centre, _, _) = geometry(&chart, 1.0);
    let labelled = |placement: CategoryBarLabels| {
        let mut s = [bars("one", RED), bars("two", BLUE)];
        for series in &mut s {
            let DataRenderType::CategoryBars { labels, .. } = &mut series.render_type else {
                unreachable!()
            };
            labels.placement = placement;
            labels.text = (0..4).map(|k| format!("{}", 10 * (k + 1))).collect();
        }
        s
    };
    let none = export(&mut r, &chart, &labelled(CategoryBarLabels::None), 1.0);
    assert_eq!(bbox(&none, is_dark), None, "no labels draw no ink");
    let outside = export(&mut r, &chart, &labelled(CategoryBarLabels::Outside), 1.0);
    save("rose-labels-outside", &outside);
    let inside = export(&mut r, &chart, &labelled(CategoryBarLabels::Inside), 1.0);
    save("rose-labels-inside", &inside);
    for k in 0..4 {
        for (m, value) in [(0, ONE[k]), (1, TWO[k])] {
            let mid = mid_angle(&chart, 1.0, k, m, 2);
            let r_end = radius_px(&chart, 1.0, value);
            // Outside: ink within a label's reach past the end, none at the
            // wedge's middle.
            assert!(
                any_within(&outside, at(centre, mid, r_end + 14.0), 14.0, is_dark),
                "category {k} member {m}: an outside label past the arc"
            );
            assert!(
                !any_within(&outside, at(centre, mid, r_end * 0.5), 6.0, is_dark),
                "category {k} member {m}: no outside label inside the wedge"
            );
            // Inside: ink at the middle radius, none past the end.
            assert!(
                any_within(&inside, at(centre, mid, r_end * 0.5), 12.0, is_light),
                "category {k} member {m}: an inside label at the middle radius"
            );
            assert!(
                !any_within(&inside, at(centre, mid, r_end + 14.0), 8.0, is_dark),
                "category {k} member {m}: no inside label past the arc"
            );
        }
    }
}

/// A polar subplot draws its own rose where the subplot is, and nothing
/// of it in the parent's panel.
#[test]
fn a_polar_subplot_draws_its_own_rose() {
    let mut r = renderer(&[("one", &ONE), ("two", &TWO)]);
    let mut parent = renderer::default::default_config();
    parent.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    parent.legend.visible = false;
    let mut cell = chart(CategoryBarMode::Stacked, 4, (0.0, 40.0))
        .config()
        .clone();
    cell.chart_area = ChartArea(Rect {
        x: 0,
        y: HEIGHT,
        width: WIDTH,
        height: HEIGHT,
    });
    parent.subplots.push(Subplot {
        config: cell,
        series: vec!["one".into(), "two".into()],
        link_x: None,
        link_y: None,
    });
    let chart = Chart::new(parent);
    let series = [bars("one", RED), bars("two", BLUE)];
    for scale in [1.0, 2.0] {
        let img = r.export_document_rgba(&chart, &series, scale).unwrap();
        save(&format!("rose-subplot-{scale}x"), &img);
        let (w, h) = (img.width as usize, img.height as usize);
        for (name, pred) in [("red", is_red as fn([u8; 4]) -> bool), ("blue", is_blue)] {
            let count = |rows: &[u8]| {
                rows.chunks_exact(4)
                    .filter(|p| pred([p[0], p[1], p[2], p[3]]))
                    .count()
            };
            let lower = count(&img.rgba[w * 4 * (h / 2)..]);
            let upper = count(&img.rgba[..w * 4 * (h / 2)]);
            assert!(
                lower > 1000 && upper == 0,
                "{scale}x {name}: {upper} / {lower}"
            );
        }
    }
}

/// The 2x export is the 1x rose doubled.
#[test]
fn a_2x_export_doubles_the_rose() {
    let mut r = renderer(&[("one", &ONE), ("two", &TWO)]);
    let series = [bars("one", RED), bars("two", BLUE)];
    let chart = chart(CategoryBarMode::Stacked, 4, (0.0, 40.0));
    let one = export(&mut r, &chart, &series, 1.0);
    let two = export(&mut r, &chart, &series, 2.0);
    assert_eq!((two.width, two.height), (WIDTH * 2, HEIGHT * 2));
    for pred in [is_red, is_blue] {
        let a = bbox(&one, pred).unwrap();
        let b = bbox(&two, pred).unwrap();
        for (lo, hi) in [
            (a.0, b.0),
            (a.1, b.1),
            (a.2 + 1, b.2 + 1),
            (a.3 + 1, b.3 + 1),
        ] {
            assert!(
                (i64::from(lo) * 2 - i64::from(hi)).abs() <= 3,
                "1x box {a:?} against 2x box {b:?}"
            );
        }
    }
}

/// A horizontal group has no rose: the model names the rule, and the
/// renderer refuses the chart at registration and at export.
#[test]
fn a_horizontal_group_on_a_polar_chart_is_refused() {
    let mut r = renderer(&[("one", &ONE), ("two", &TWO)]);
    let series = [bars("one", RED), bars("two", BLUE)];
    let mut chart = chart(CategoryBarMode::Grouped, 4, (0.0, 25.0));
    assert_eq!(chart.config().validate_category_bars(&series), Ok(()));
    chart.config_mut().bar_groups.as_mut().unwrap().direction = CategoryBarDirection::Horizontal;
    let reason = chart
        .config()
        .validate_category_bars(&series)
        .expect_err("a horizontal rose is refused");
    assert!(reason.contains("rose"), "{reason}");
    assert!(r.export_panel_rgba(&chart, &series, 1.0).is_err());
    assert!(
        r.register_chart(chart.config().clone(), series.to_vec())
            .is_err()
    );
}

/// A rose of 4096 categories and four stacked series draws every wedge and
/// picks the last category's top segment as its own row (rule 10: no
/// count limit). The fourth series fills every stack to 28, so its segment
/// of every category covers the ring's outer eighth, where a category's
/// wedge is over two pixels wide at this size. The draw and pick times are
/// printed, not asserted.
#[test]
fn a_four_thousand_category_rose_of_four_stacked_series_draws_and_picks() {
    const N: usize = 4096;
    const SIDE: u32 = 4000;
    let mut values: Vec<Vec<f64>> = (0..3)
        .map(|s| (0..N).map(|k| 1.0 + ((k + 2 * s) % 7) as f64).collect())
        .collect();
    values.push(
        (0..N)
            .map(|k| 28.0 - values.iter().map(|v| v[k]).sum::<f64>())
            .collect(),
    );
    let names = ["one", "two", "three", "four"];
    let columns: Vec<(&str, &[f64])> = names
        .iter()
        .zip(&values)
        .map(|(n, v)| (*n, v.as_slice()))
        .collect();
    let mut r = renderer(&columns);
    let colors = [RED, BLUE, RED, BLUE];
    let series: Vec<SeriesConfig> = names.iter().zip(colors).map(|(n, c)| bars(n, c)).collect();
    let mut chart = chart(CategoryBarMode::Stacked, N, (0.0, 28.0));
    {
        let config = chart.config_mut();
        config.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: SIDE,
            height: SIDE,
        });
        config.bottom_x.label_style.visible = false;
        let groups = config.bar_groups.as_mut().unwrap();
        groups.group_width = 0.95;
        groups.bar_gap = 0.0;
    }
    let started = std::time::Instant::now();
    let img = export(&mut r, &chart, &series, 1.0);
    let drawn = started.elapsed();
    let (centre, _, _) = geometry(&chart, 1.0);
    // Every sampled category's top segment paints at the ring's outer
    // eighth; the inner segments, under a pixel wide there, paint red
    // somewhere.
    let outer = radius_px(&chart, 1.0, 24.5);
    for k in (0..N).step_by(37) {
        let mid = mid_angle(&chart, 1.0, k, 0, 1);
        assert!(
            is_blue(pixel(&img, at(centre, mid, outer))),
            "category {k}: its top segment"
        );
    }
    assert!(
        img.rgba
            .chunks_exact(4)
            .any(|p| is_red([p[0], p[1], p[2], p[3]])),
        "the inner segments paint"
    );
    // The last category's top segment, at the ring's outer eighth.
    let k = N - 1;
    let p = at(centre, mid_angle(&chart, 1.0, k, 0, 1), outer);
    assert!(
        is_blue(pixel(&img, p)),
        "the last category's top segment at {p:?}"
    );
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(chart.config().clone(), series.clone())
        .unwrap();
    let started = std::time::Instant::now();
    let hit = pollster::block_on(
        r.pick_chart_data(id, pick_request(p.0, p.1, SIDE, SIDE, 1.0))
            .unwrap()
            .resolve(),
    )
    .unwrap();
    let picked = started.elapsed();
    assert!(
        matches!(
            &hit,
            Some(renderer::PickedData::HistogramBin { series_id, bin_index, .. })
                if series_id == "four" && *bin_index == k
        ),
        "{hit:?}"
    );
    println!(
        "rose of {N} categories x 4 series at {SIDE}x{SIDE}: draw {:?}, pick {:?}",
        drawn, picked
    );
}
