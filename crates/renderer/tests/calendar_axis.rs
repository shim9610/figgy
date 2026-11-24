//! The calendar header of a timeline (#357, item 14 B): a time axis whose
//! ticks walk a forced calendar step, with labels centred on their periods,
//! counted from an origin, or the host's own strings. The labels are the
//! axis' tick-label element: they hit, select and drag as any tick label,
//! draw at 1x and 2x, and serialize.

use test_gpu::BlockOn as _;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/gpu.rs"]
mod test_gpu;
use test_gpu::shared_device;

use renderer::config::Category;
use renderer::format::{
    CalendarStep, LabelFormat, TimestampLabelFormat, TimestampLabelMode, Weekday,
};
use renderer::layout::{ChartArea, NudgeResult, Rect, RectF, Side, axis_fraction};
use renderer::sink::{Border, PlacedText, Sink, Slot, StrokeStyle, TextRef};
use renderer::{
    AxisLabelElement, Chart, Color, ColorBarOptions, Config, CpuTextMeasure, Draggable, HitMap,
    Renderer, RendererDevice, Selectable, SelectionGeometry, SeriesConfig,
};

const WIDTH: u32 = 960;
const HEIGHT: u32 = 480;
const DAY: f64 = 86_400.0;
/// Monday 2026-03-02T00:00:00Z.
const T0: f64 = 1_772_409_600.0;

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
    fn path(&mut self, _: Slot, _: &[renderer::sink::PathSegment], _: &StrokeStyle<'_>) {}
    fn polygon(&mut self, _: Slot, _: &[[f32; 2]], _: Option<Color>, _: Option<&StrokeStyle<'_>>) {}
    fn ramp(&mut self, _: Slot, _: RectF, _: &ColorBarOptions) {}
}

/// The texts and boxes `side`'s tick-label element draws.
fn labels(cfg: &Config, side: Side) -> Vec<(String, RectF)> {
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let mut log = TextLog::default();
    AxisLabelElement { side }.emit(cfg, &measure, &mut log);
    log.0
}

fn texts(cfg: &Config, side: Side) -> Vec<String> {
    labels(cfg, side).into_iter().map(|(t, _)| t).collect()
}

/// The calendar timeline over 2026-03-02 .. 2026-05-17 with three rows.
fn timeline() -> Chart {
    let mut config = renderer::default::default_calendar_timeline_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    });
    config.chart_title.visible = false;
    config.legend.visible = false;
    config.left_y.categories = Some(
        vec![
            Category::new("a", "Design"),
            Category::new("b", "Build"),
            Category::new("c", "Launch"),
        ]
        .into(),
    );
    let mut chart = Chart::new(config);
    chart.set_x_range(T0, T0 + 76.0 * DAY);
    chart.fit_layout();
    chart
}

/// The x of `value` on `side`'s axis of `cfg`, in logical pixels.
fn x_of(cfg: &Config, side: Side, value: f64) -> f32 {
    let da = cfg.data_area().expect("data area");
    let axis = match side {
        Side::Top => &cfg.top_x,
        _ => &cfg.bottom_x,
    };
    da.x as f32 + axis_fraction(value, axis, da.width as f32) * da.width as f32
}

fn centre_x(r: &RectF) -> f32 {
    r.x + r.width / 2.0
}

/// The month header names each month once, centred on its part of the
/// range; the week axis ticks every Monday with its day of the month.
#[test]
fn the_month_header_centres_each_month_and_the_weeks_start_on_monday() {
    let chart = timeline();
    let cfg = chart.config();
    let top = labels(cfg, Side::Top);
    let names: Vec<&str> = top.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(names, ["Mar 2026", "Apr 2026", "May 2026"]);
    let (min, max) = (T0, T0 + 76.0 * DAY);
    let (apr, may) = (T0 + 30.0 * DAY, T0 + 60.0 * DAY);
    for ((_, rect), (lo, hi)) in top.iter().zip([(min, apr), (apr, may), (may, max)]) {
        let want = x_of(cfg, Side::Top, 0.5 * (lo + hi));
        assert!((centre_x(rect) - want).abs() <= 1.0, "{rect:?} vs {want}");
    }
    let weeks = texts(cfg, Side::Bottom);
    assert_eq!(&weeks[..4], ["02", "09", "16", "23"]);
    assert!(weeks.len() >= 10, "{weeks:?}");
}

/// The period labels are the top axis' tick-label element: a click on one
/// hits it, a drag moves every drawn label and the selection box with the
/// bounds, as for any tick label.
#[test]
fn period_labels_hit_select_and_drag_as_tick_labels() {
    let chart = timeline();
    let cfg = chart.config().clone();
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let element = AxisLabelElement { side: Side::Top };
    let name = element.element_id();
    let map = HitMap::standard_chart();
    let drawn = labels(&cfg, Side::Top);
    let (cx, cy) = drawn[1].1.center();
    let hit = map.hit_test(&cfg, &measure, cx, cy).expect("a hit");
    assert_eq!(map.get(hit).unwrap().element_id(), name);
    let bounds = element.bounds(&cfg, &measure).unwrap();
    for (_, rect) in &drawn {
        assert!(rect.x >= bounds.x - 1e-3 && rect.x + rect.width <= bounds.x + bounds.width + 1e-3);
    }
    let padding = bounds.x - element.selection_box(&cfg, &measure).unwrap().rect().x;
    for (dx, dy) in [(24.0, 0.0), (0.0, -5.0)] {
        let mut moved = cfg.clone();
        assert_eq!(
            element.drag_by(&mut moved, &measure, dx, dy),
            NudgeResult::Moved
        );
        let after = element.bounds(&moved, &measure).unwrap();
        assert_eq!((after.x - bounds.x, after.y - bounds.y), (dx, dy));
        for ((_, a), (_, b)) in drawn.iter().zip(labels(&moved, Side::Top).iter()) {
            assert!((b.x - a.x - dx).abs() < 1e-3 && (b.y - a.y - dy).abs() < 1e-3);
        }
        assert_eq!(
            element.selection_box(&moved, &measure).unwrap().rect(),
            after.expanded(padding)
        );
    }
}

/// Relative labels count periods from the host's origin; custom labels are
/// the host's strings by the same count, and a period past the list draws
/// no label. Ten thousand strings draw deep into the list (rule 10).
#[test]
fn relative_and_custom_labels_follow_the_origin() {
    let mut chart = timeline();
    let week = Some(CalendarStep::Week {
        start: Weekday::Monday,
    });
    let set = |chart: &mut Chart, label: TimestampLabelMode| {
        chart.config_mut().bottom_x.label_style.format =
            LabelFormat::Timestamp(TimestampLabelFormat {
                label,
                step: week,
                ..TimestampLabelFormat::default()
            });
    };
    set(
        &mut chart,
        TimestampLabelMode::Relative {
            origin: (T0 + 7.0 * DAY) as i64,
            prefix: "Week ".into(),
        },
    );
    let weeks = texts(chart.config(), Side::Bottom);
    assert_eq!(&weeks[..3], ["Week -1", "Week 0", "Week 1"]);
    set(
        &mut chart,
        TimestampLabelMode::Custom {
            origin: T0 as i64,
            labels: vec!["Kickoff".into(), "Sprint 1".into(), "Sprint 2".into()],
        },
    );
    assert_eq!(
        texts(chart.config(), Side::Bottom),
        ["Kickoff", "Sprint 1", "Sprint 2"]
    );
    set(
        &mut chart,
        TimestampLabelMode::Custom {
            origin: (T0 - 9_990.0 * 7.0 * DAY) as i64,
            labels: (0..10_000).map(|i| format!("W{i}")).collect(),
        },
    );
    assert_eq!(
        &texts(chart.config(), Side::Bottom)[..2],
        ["W9990", "W9991"]
    );
}

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    let mut r = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init");
    r.add_column("pos", &harness::col(vec![0.0, 1.0, 2.0]))
        .unwrap();
    r.add_hilo_column(
        "start",
        &harness::col(vec![T0 + DAY, T0 + 20.0 * DAY, T0 + 50.0 * DAY]),
    )
    .unwrap();
    r.add_hilo_column(
        "end",
        &harness::col(vec![T0 + 18.0 * DAY, T0 + 48.0 * DAY, T0 + 70.0 * DAY]),
    )
    .unwrap();
    r
}

/// The bounding box of dark label ink in `region` of an export.
fn ink(img: &renderer::RasterImage, region: (u32, u32, u32, u32)) -> Option<(u32, u32, u32, u32)> {
    harness::bbox(img, region, |p| {
        p[3] > 200 && p[0] < 120 && p[1] < 120 && p[2] < 140
    })
}

/// The header draws above the plot at 1x and 2x, the 2x header is the 1x
/// one doubled, and a saved document draws the same pixels.
#[test]
fn the_header_draws_at_1x_and_2x_and_round_trips() {
    let mut r = renderer();
    let chart = timeline();
    let series = [SeriesConfig::timeline_bars("t", "pos", "start", "end")];
    let da = chart.config().data_area().unwrap();
    let mut boxes = Vec::new();
    for scale in [1.0f32, 2.0] {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale).with_clear(Color::WHITE),
            )
            .block_on()
            .unwrap();
        let s = |v: u32| (v as f32 * scale) as u32;
        let header = (s(da.x), 0, s(da.x + da.width), s(da.y));
        boxes.push(ink(&img, header).expect("the month header draws"));
    }
    let (one, two) = (boxes[0], boxes[1]);
    for (a, b) in [
        (one.0, two.0),
        (one.1, two.1),
        (one.2, two.2),
        (one.3, two.3),
    ] {
        assert!((b as f32 - 2.0 * a as f32).abs() <= 3.0, "{one:?} {two:?}");
    }
    #[cfg(feature = "serde")]
    {
        let draw = |r: &mut Renderer, chart: &Chart| {
            r.export_rgba(
                renderer::ExportSource::chart(chart, &series),
                renderer::ExportOptions::panel(1.0),
            )
            .block_on()
            .unwrap()
            .rgba
        };
        let before = draw(&mut r, &chart);
        let json = serde_json::to_string(chart.config()).unwrap();
        assert!(json.contains("period_labels") && json.contains("Week"));
        let config: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(&config, chart.config());
        assert_eq!(draw(&mut r, &Chart::new(config)), before);
    }
}

/// Every placed label's text and baseline origin.
#[derive(Default)]
struct BaselineLog(Vec<(String, (f32, f32), RectF)>);

impl Sink for BaselineLog {
    fn text(&mut self, _slot: Slot, text: &PlacedText<'_>) {
        if let TextRef::Plain { text: plain, .. } = text.text {
            self.0.push((plain.to_owned(), text.origin, text.rect()));
        }
    }
    fn stroke(&mut self, _: Slot, _: [f32; 2], _: [f32; 2], _: &StrokeStyle<'_>) {}
    fn rect(&mut self, _: Slot, _: RectF, _: Option<Color>, _: Option<Border<'_>>) {}
    fn triangle(&mut self, _: Slot, _: [[f32; 2]; 3], _: Color) {}
    fn path(&mut self, _: Slot, _: &[renderer::sink::PathSegment], _: &StrokeStyle<'_>) {}
    fn polygon(&mut self, _: Slot, _: &[[f32; 2]], _: Option<Color>, _: Option<&StrokeStyle<'_>>) {}
    fn ramp(&mut self, _: Slot, _: RectF, _: &ColorBarOptions) {}
}

fn baselines(cfg: &Config, side: Side) -> Vec<(String, (f32, f32), RectF)> {
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let mut log = BaselineLog::default();
    AxisLabelElement { side }.emit(cfg, &measure, &mut log);
    log.0
}

/// One tick row, one baseline (#357): labels with and without descenders
/// on a top and a bottom axis share their baseline at 1x and 2x, in the
/// layout's geometry and in the drawn pixels (the bottom of "2026" in each
/// month name is the baseline).
#[test]
fn labels_with_and_without_descenders_share_one_baseline() {
    let words = ["Mar", "Sep", "Aug", "jumpy", "ok", "Hg"];
    let mut cfg = timeline().config().clone();
    for axis in [&mut cfg.top_x, &mut cfg.bottom_x] {
        axis.categories = Some(
            words
                .iter()
                .enumerate()
                .map(|(i, w)| Category::new(format!("c{i}"), *w))
                .collect(),
        );
    }
    for scale in [1.0f32, 2.0] {
        let scaled = Chart::new(cfg.clone()).scaled(scale);
        for side in [Side::Top, Side::Bottom] {
            let row = baselines(scaled.config(), side.clone());
            assert_eq!(row.len(), words.len(), "{side:?}");
            let y = row[0].1.1;
            for (text, origin, _) in &row {
                assert!(
                    (origin.1 - y).abs() < 1e-3,
                    "{scale}x {side:?}: {text} at {origin:?}, row at {y}"
                );
            }
        }
    }

    // The drawn month header: each label's "2026" ends on the same row of
    // pixels, at 1x and 2x.
    let mut r = renderer();
    let chart = timeline();
    let series = [SeriesConfig::timeline_bars("t", "pos", "start", "end")];
    for scale in [1.0f32, 2.0] {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &series),
                renderer::ExportOptions::panel(scale).with_clear(Color::WHITE),
            )
            .block_on()
            .unwrap();
        let header = baselines(chart.config(), Side::Top);
        let bottoms: Vec<u32> = header
            .iter()
            .map(|(text, _, rect)| {
                // The last four characters, "2026", in the right part of the box.
                let s = |v: f32| (v * scale).round() as u32;
                let x0 = rect.x + rect.width * 0.55;
                let region = (
                    s(x0),
                    s(rect.y) - 2,
                    s(rect.x + rect.width) + 2,
                    s(rect.y + rect.height) + 2,
                );
                ink(&img, region)
                    .unwrap_or_else(|| panic!("{scale}x: {text} drew no ink"))
                    .3
            })
            .collect();
        assert!(
            bottoms.windows(2).all(|w| w[0] == w[1]),
            "{scale}x: {bottoms:?} for {header:?}"
        );
    }
}
