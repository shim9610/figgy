//! Energy-level (Grotrian) diagrams (#203): a DataLess series drawn on the
//! CPU and composited in the data slot. Every test exports at 1x and 2x.

use test_gpu::BlockOn as _;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::OnceLock;

use renderer::config::{AxisBreak, AxisBreakStyle, AxisBreaks, BreakMark, Category};
use renderer::energy_levels::{
    EnergyLevel, EnergyLevelSeries, IonizationLimit, LevelRole, LevelView, Transition,
};
use renderer::layout::{ChartArea, Rect};
use renderer::{
    Chart, Color, DataRenderType, DataSelectionsConfig, PickedDataRef, RasterImage, Renderer,
    RendererDevice, SeriesConfig,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;
const RED: Color = Color::from_rgb8(255, 0, 0);
const BLUE: Color = Color::from_rgb8(0, 0, 255);
const GREEN: Color = Color::from_rgb8(0, 200, 0);

fn shared_device() -> (wgpu::Device, wgpu::Queue) {
    static DEVICE: OnceLock<(wgpu::Device, wgpu::Queue)> = OnceLock::new();
    let (device, queue) = DEVICE.get_or_init(test_gpu::device);
    (device.clone(), queue.clone())
}

fn renderer() -> Renderer {
    let (device, queue) = shared_device();
    Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        4 * 1024 * 1024,
    )
    .expect("renderer init")
}

fn level(id: &str, column: &str, energy: f64) -> EnergyLevel {
    EnergyLevel {
        id: id.into(),
        column: column.into(),
        configuration: String::new(),
        term: String::new(),
        j: String::new(),
        g: None,
        prefix: String::new(),
        energy,
        suffix: String::new(),
        uncertainty: None,
        reference: String::new(),
        role: LevelRole::Level,
        label: None,
        label_offset: [0.0; 2],
    }
}

fn transition(id: &str, lower: &str, upper: &str) -> Transition {
    Transition {
        id: id.into(),
        lower: lower.into(),
        upper: upper.into(),
        label: None,
        label_offset: [0.0; 2],
        value: None,
        x_offset: 0.0,
        observed_wavelength: None,
        observed_uncertainty: None,
        ritz_wavelength: None,
        ritz_uncertainty: None,
        intensity: String::new(),
        aki: None,
        accuracy: String::new(),
        line_type: String::new(),
        tp_reference: String::new(),
        line_reference: String::new(),
    }
}

/// Levels at 20, 40 and 60 in columns a, b and c (b's has a hidden
/// component), a limit at 90 and a transition from 60 down to 20.
fn records() -> EnergyLevelSeries {
    let mut levels = EnergyLevelSeries {
        levels: vec![
            level("low", "a", 20.0),
            EnergyLevel {
                role: LevelRole::Centroid,
                ..level("mid", "b", 40.0)
            },
            EnergyLevel {
                role: LevelRole::Component {
                    multiplet: "mid".into(),
                },
                ..level("mid1", "b", 45.0)
            },
            level("high", "c", 60.0),
        ],
        limits: vec![IonizationLimit {
            id: "ion".into(),
            energy: 90.0,
            label: None,
            label_offset: [0.0; 2],
            uncertainty: None,
            reference: String::new(),
        }],
        transitions: vec![transition("t", "low", "high")],
        ..EnergyLevelSeries::default()
    };
    levels.style.level.color = RED;
    levels.style.level.width = 4.0;
    levels.style.limit.color = BLUE;
    levels.style.limit.width = 3.0;
    levels.style.limit.style = renderer::line::LineStylePreset::Solid;
    levels.style.transition.color = GREEN;
    levels.style.transition.width = 3.0;
    levels
}

fn series(id: &str, levels: EnergyLevelSeries) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: String::new(),
        y_column: String::new(),
        z_column: None,
        render_type: DataRenderType::EnergyLevels(levels),
    }
}

fn chart() -> Chart {
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
    config.bottom_x.categories = Some(vec![
        Category::new("a", "A"),
        Category::new("b", "B"),
        Category::new("c", "C"),
    ]);
    let mut chart = Chart::new(config);
    chart.set_y_range(0.0, 100.0);
    chart
}

fn pixel(img: &RasterImage, x: u32, y: u32) -> &[u8] {
    let i = ((y * img.width + x) * 4) as usize;
    &img.rgba[i..i + 4]
}

fn is_red(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && p[1] < 80 && p[2] < 80
}
fn is_blue(p: &[u8]) -> bool {
    p[3] > 200 && p[2] > 200 && p[0] < 80 && p[1] < 80
}
fn is_green(p: &[u8]) -> bool {
    p[3] > 200 && p[1] > 150 && p[0] < 80 && p[2] < 80
}

/// The export pixel of data coordinate `(x, y)`: `x` a category index, `y`
/// on `left_y`'s 0..100.
fn at(chart: &Chart, x: f64, y: f64, scale: f32) -> (u32, u32) {
    let da = chart.config().data_area().expect("data area");
    let px = da.x as f64 + (x + 0.5) / 3.0 * da.width as f64;
    let py = da.y as f64 + (1.0 - y / 100.0) * da.height as f64;
    (
        (px * scale as f64).round() as u32,
        (py * scale as f64).round() as u32,
    )
}

/// Whether `pred` holds within `r` export pixels of `(x, y)`.
fn near(img: &RasterImage, (x, y): (u32, u32), r: u32, pred: fn(&[u8]) -> bool) -> bool {
    (y.saturating_sub(r)..=(y + r).min(img.height - 1)).any(|py| {
        (x.saturating_sub(r)..=(x + r).min(img.width - 1)).any(|px| pred(pixel(img, px, py)))
    })
}

fn export(r: &mut Renderer, chart: &Chart, series: &[SeriesConfig], scale: f32) -> RasterImage {
    r.export_rgba(
        renderer::ExportSource::chart(chart, series),
        renderer::ExportOptions::panel(scale),
    )
    .block_on()
    .unwrap()
}

#[test]
fn levels_limits_and_transitions_draw_where_their_values_place_them() {
    let mut r = renderer();
    let chart = chart();
    let list = [series("he", records())];
    assert_eq!(chart.config().validate_series(&list), Ok(()));
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &list, scale);
        let s = (2.0 * scale).ceil() as u32;
        // Each level's stroke spans its slot at its energy, off its centre.
        for (x, y) in [(0.0, 20.0), (1.0, 40.0)] {
            let left = at(&chart, x - 0.25, y, scale);
            assert!(near(&img, left, s, is_red), "{scale}x level at ({x}, {y})");
        }
        // The component is not drawn in the main view.
        assert!(
            !near(&img, at(&chart, 1.0 - 0.25, 45.0, scale), s, is_red),
            "{scale}x a component stays out of the main view"
        );
        // The limit crosses the plot.
        for x in [-0.4, 1.0, 2.4] {
            assert!(
                near(&img, at(&chart, x, 90.0, scale), s, is_blue),
                "{scale}x limit at {x}"
            );
        }
        // The transition joins the two levels' centres, a straight line.
        let mid = at(&chart, 1.0, 40.0, scale);
        assert!(
            near(&img, mid, s + 1, is_green),
            "{scale}x the arrow's middle"
        );
        // Nothing far from every part.
        assert!(!near(&img, at(&chart, 0.0, 70.0, scale), s, is_red));
    }
}

#[test]
fn the_picked_level_is_drawn_wider() {
    let mut r = renderer();
    let mut chart = chart();
    let list = [series("he", records())];
    let probe = crate::chart();
    let thickness = |img: &RasterImage, scale: f32| {
        let (x, y) = at(&probe, 0.0, 20.0, scale);
        let x = x - (0.2 * scale * 160.0 / 3.0) as u32;
        (y.saturating_sub(20)..y + 20)
            .filter(|&py| is_red(pixel(img, x, py)))
            .count()
    };
    let plain = export(&mut r, &chart, &list, 1.0);
    let before = thickness(&plain, 1.0);
    chart.config_mut().picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::HistogramBin {
            source_id: None,
            series_id: "he".into(),
            bin_index: 0,
            half: None,
        }],
        ..DataSelectionsConfig::default()
    });
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &list, scale);
        let after = thickness(&img, scale);
        assert!(
            after as f32 >= before as f32 * 1.5 * scale,
            "{scale}x picked {after} vs plain {before}"
        );
    }
}

#[test]
fn a_continuous_break_moves_the_levels_with_the_axis_and_draws_none_in_the_gap() {
    let mut r = renderer();
    let mut chart = chart();
    chart.config_mut().left_y.breaks = Some(AxisBreaks {
        ranges: vec![AxisBreak::new(25.0, 35.0)],
        style: AxisBreakStyle::Continuous,
        gap: 12.0,
        mark: BreakMark::default(),
        frame_each_panel: false,
        sizing: Default::default(),
    });
    let mut levels = records();
    // A level inside the cut is not placed.
    levels.levels.push(level("cut", "a", 30.0));
    let list = [series("he", levels)];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &list, scale);
        // Every level is still drawn, on the broken axis' map.
        let space = renderer::annotation::pixel_space_for_config(&chart.config().scaled(scale));
        use renderer::AnnotationSpace as _;
        for (x, y) in [(0.0, 20.0), (1.0, 40.0), (2.0, 60.0)] {
            let px = space
                .value_to_px(renderer::AnnotationAxis::X, x - 0.25)
                .unwrap();
            let py = space.value_to_px(renderer::AnnotationAxis::Y, y).unwrap();
            assert!(
                near(&img, (px.round() as u32, py.round() as u32), 3, is_red),
                "{scale}x level ({x}, {y})"
            );
        }
    }
}

#[test]
fn a_components_view_draws_the_source_records_without_a_copy() {
    let mut r = renderer();
    let chart = chart();
    let view = EnergyLevelSeries {
        source: Some("he".into()),
        show: LevelView::Components,
        style: records().style,
        ..EnergyLevelSeries::default()
    };
    let list = [
        series("inset", view),
        series("he", EnergyLevelSeries::default()),
    ];
    // The source named by the view has no records here: nothing is drawn.
    let img = export(&mut r, &chart, &list[..1], 1.0);
    assert!(!near(&img, at(&chart, 0.75, 45.0, 1.0), 2, is_red));
    let list = [series("he", records()), list[0].clone()];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &list, scale);
        let s = (2.0 * scale).ceil() as u32;
        // The view draws the component at 45, from the source's records.
        assert!(
            near(&img, at(&chart, 0.75, 45.0, scale), s, is_red),
            "{scale}x component"
        );
    }
}

#[test]
fn thousands_of_records_draw_with_no_count_limit() {
    // Past the 1024 annotation items and the 64 radial slices (rule 10).
    let mut r = renderer();
    let chart = chart();
    let mut levels = records();
    levels.transitions.clear();
    for i in 0..5_000 {
        levels.levels.push(level(
            &format!("x{i}"),
            "c",
            61.0 + 38.0 * i as f64 / 5_000.0,
        ));
    }
    for i in 0..1_500 {
        levels
            .transitions
            .push(transition(&format!("t{i}"), "low", &format!("x{i}")));
    }
    let list = [series("big", levels)];
    assert_eq!(chart.config().validate_series(&list), Ok(()));
    for scale in [1.0f32, 2.0] {
        let started = std::time::Instant::now();
        let img = export(&mut r, &chart, &list, scale);
        eprintln!(
            "energy levels: 5,004 levels + 1,500 transitions exported at {scale}x in {:?}",
            started.elapsed()
        );
        // The last level, at the top of the run, is drawn.
        assert!(near(
            &img,
            at(&chart, 1.75, 99.0 - 38.0 / 5_000.0, scale),
            3,
            is_red
        ));
    }
}

/// The typed pick at export pixel `at` of a registered chart displayed at
/// `scale`.
fn pick(
    r: &mut Renderer,
    chart: &Chart,
    series: &[SeriesConfig],
    scale: f32,
    at: (u32, u32),
) -> Option<renderer::PickedData> {
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(chart.config().clone(), series.to_vec())
        .unwrap();
    let request = renderer::GpuPickRequest {
        canvas_position_px: [at.0 as f32, at.1 as f32],
        display_panel_px: Rect {
            x: 0,
            y: 0,
            width: (WIDTH as f32 * scale) as u32,
            height: (HEIGHT as f32 * scale) as u32,
        },
        display_scale: scale,
        max_distance_px: 4.0 * scale,
    };
    let hit = test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap();
    r.remove_chart(id).unwrap();
    hit
}

#[test]
fn levels_transitions_and_labels_pick_with_their_rows() {
    let mut r = renderer();
    let chart = chart();
    let mut levels = records();
    levels.levels[3].label = Some(renderer::text::RichText::plain(
        "3s",
        Color::BLACK,
        14.0,
        "",
    ));
    let list = [series("he", levels)];
    for scale in [1.0f32, 2.0] {
        // A level stroke, off its centre (the arrow leaves from its centre).
        match pick(&mut r, &chart, &list, scale, at(&chart, 1.2, 40.0, scale)) {
            Some(renderer::PickedData::HistogramBin {
                series_id,
                bin_index,
                ..
            }) => assert_eq!((series_id.as_str(), bin_index), ("he", 1), "{scale}x"),
            other => panic!("{scale}x a level: {other:?}"),
        }
        // The transition's middle.
        match pick(
            &mut r,
            &chart,
            &list,
            scale,
            at(&chart, 1.0, 40.0 - 0.0, scale),
        ) {
            Some(renderer::PickedData::Point { point_index, .. }) => {
                assert_eq!(point_index, 0, "{scale}x")
            }
            // The level at 40 lies under the arrow too; either is drawn
            // there, the arrow over the stroke.
            Some(renderer::PickedData::HistogramBin { bin_index: 1, .. }) => {}
            other => panic!("{scale}x the arrow: {other:?}"),
        }
        let (tx, ty) = at(&chart, 0.5, 30.0, scale);
        match pick(&mut r, &chart, &list, scale, (tx, ty)) {
            Some(renderer::PickedData::Point { point_index, .. }) => {
                assert_eq!(point_index, 0, "{scale}x")
            }
            other => panic!("{scale}x the arrow between levels: {other:?}"),
        }
        // The label of the 60 eV stroke: its column is the last, and the
        // label would leave the plot on the right, so it is drawn on the
        // left of the stroke (#205).
        let (lx, ly) = at(&chart, 2.0 - 0.35, 60.0, scale);
        match pick(
            &mut r,
            &chart,
            &list,
            scale,
            (lx - (14.0 * scale) as u32, ly),
        ) {
            Some(renderer::PickedData::ValueLabel {
                category_index,
                part,
                ..
            }) => assert_eq!(
                (category_index, part),
                (3, Some(renderer::energy_levels::LabelPart::Level)),
                "{scale}x"
            ),
            other => panic!("{scale}x a label: {other:?}"),
        }
        // Empty space answers nothing.
        assert_eq!(
            pick(&mut r, &chart, &list, scale, at(&chart, 0.0, 70.0, scale)),
            None
        );
        // A hit's reference round-trips into the selection it names.
        let hit = pick(&mut r, &chart, &list, scale, at(&chart, 1.2, 40.0, scale)).unwrap();
        assert_eq!(
            hit.to_ref(),
            PickedDataRef::HistogramBin {
                source_id: None,
                series_id: "he".into(),
                bin_index: 1,
                half: None,
            }
        );
    }
}

#[test]
fn a_pick_inside_a_break_gap_answers_nothing() {
    let mut r = renderer();
    let mut chart = chart();
    chart.config_mut().left_y.breaks = Some(AxisBreaks {
        ranges: vec![AxisBreak::new(25.0, 35.0)],
        style: AxisBreakStyle::Continuous,
        gap: 0.2,
        mark: BreakMark::default(),
        frame_each_panel: false,
        sizing: Default::default(),
    });
    let list = [series("he", records())];
    for scale in [1.0f32, 2.0] {
        let space = renderer::annotation::pixel_space_for_config(&chart.config().scaled(scale));
        use renderer::AnnotationSpace as _;
        let y25 = space
            .value_to_px(renderer::AnnotationAxis::Y, 24.99)
            .unwrap();
        let y35 = space
            .value_to_px(renderer::AnnotationAxis::Y, 35.01)
            .unwrap();
        let x = space.value_to_px(renderer::AnnotationAxis::X, 0.5).unwrap();
        // The arrow from 20 to 60 crosses the gap: picked above and below
        // it, never in it.
        let mid = ((y25 + y35) / 2.0).round() as u32;
        let hit = pick(&mut r, &chart, &list, scale, (x.round() as u32, mid));
        assert!(
            !matches!(hit, Some(renderer::PickedData::Point { .. })),
            "{scale}x gap: {hit:?}"
        );
    }
}

#[test]
fn an_ssot_edit_of_the_series_redraws_it() {
    let mut r = renderer();
    let chart = chart();
    let id = r
        .register_chart(chart.config().clone(), vec![series("he", records())])
        .unwrap();
    for scale in [1.0f32, 2.0] {
        let before = r
            .export_rgba(
                renderer::ExportSource::Registered(id),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        assert!(near(
            &before,
            at(&chart, 0.0 - 0.25, 20.0, scale),
            3,
            is_red
        ));
        // Move the lowest level from 20 to 70: the raster follows the SSOT.
        let mut moved = records();
        moved.levels[0].energy = 70.0;
        r.set_chart_series(id, vec![series("he", moved)]).unwrap();
        let after = r
            .export_rgba(
                renderer::ExportSource::Registered(id),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        assert!(
            !near(&after, at(&chart, 0.0 - 0.25, 20.0, scale), 2, is_red),
            "{scale}x old"
        );
        assert!(
            near(&after, at(&chart, 0.0 - 0.25, 70.0, scale), 3, is_red),
            "{scale}x new"
        );
        // A style edit too.
        let mut blue = records();
        blue.style.level.color = BLUE;
        r.set_chart_series(id, vec![series("he", blue)]).unwrap();
        let recolored = r
            .export_rgba(
                renderer::ExportSource::Registered(id),
                renderer::ExportOptions::panel(scale),
            )
            .block_on()
            .unwrap();
        assert!(near(
            &recolored,
            at(&chart, 0.0 - 0.25, 20.0, scale),
            3,
            is_blue
        ));
        r.set_chart_series(id, vec![series("he", records())])
            .unwrap();
    }
}

#[test]
fn a_document_with_the_series_round_trips_and_draws_the_same() {
    let mut r = renderer();
    let chart = chart();
    let list = vec![series("he", records())];
    let json = serde_json::to_string(&(chart.config(), &list)).unwrap();
    let (config, back): (renderer::config::Config, Vec<SeriesConfig>) =
        serde_json::from_str(&json).unwrap();
    assert_eq!(back, list);
    assert_eq!(&config, chart.config());
    let again = serde_json::to_string(&(&config, &back)).unwrap();
    assert_eq!(again, json, "re-serializes unchanged");
    for scale in [1.0f32, 2.0] {
        let a = export(&mut r, &chart, &list, scale);
        let b = export(&mut r, &Chart::new(config.clone()), &back, scale);
        assert_eq!(a.rgba, b.rgba, "{scale}x");
    }
}

#[test]
fn svg_and_pdf_exports_carry_the_series_in_the_data_image() {
    // The series is data: like every series, the vector export embeds it
    // in the panel's data image (#48), here at 1x and 2x.
    let mut r = renderer();
    let chart = chart();
    let list = [series("he", records())];
    for scale in [1.0f32, 2.0] {
        let svg = r
            .export_svg(
                renderer::ExportSource::chart(&chart, &list),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        let text = svg;
        assert!(text.contains("<image"), "{scale}x svg data image");
        let pdf = r
            .export_pdf(
                renderer::ExportSource::chart(&chart, &list),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"), "{scale}x pdf");
    }
}

/// The chart with an inset: a subplot in the parent's data area showing the
/// component of column b between 43 and 47, with its own plot fill.
fn chart_with_inset() -> Chart {
    let mut chart = chart();
    let parent = chart.config().data_area().unwrap().0;
    let mut inset = renderer::default::default_config();
    inset.chart_area = ChartArea(Rect {
        x: parent.x + parent.width - 170,
        y: parent.y + 10,
        width: 160,
        height: 120,
    });
    inset.legend.visible = false;
    inset.chart_title.visible = false;
    inset.grid.show_major_x = false;
    inset.grid.show_minor_x = false;
    inset.grid.show_minor_y = false;
    inset.grid.show_major_y = false;
    for axis in [&mut inset.top_x, &mut inset.right_y] {
        axis.out_margin = 4.0;
    }
    inset.bottom_x.out_margin = 22.0;
    inset.left_y.out_margin = 34.0;
    inset.bottom_x.categories = Some(vec![Category::new("b", "B")]);
    inset.left_y.min = 43.0;
    inset.left_y.max = 47.0;
    inset.plot_background = Some(Color::from_rgb8(250, 250, 250));
    chart.config_mut().subplots.push(renderer::Subplot {
        config: inset,
        series: vec!["inset".into()],
        link_x: None,
        link_y: None,
    });
    chart
}

fn inset_series() -> SeriesConfig {
    series(
        "inset",
        EnergyLevelSeries {
            source: Some("he".into()),
            show: LevelView::Components,
            style: records().style,
            ..EnergyLevelSeries::default()
        },
    )
}

#[test]
fn an_inset_covers_its_parent_and_takes_the_pointer() {
    let mut r = renderer();
    let chart = chart_with_inset();
    // The parent draws a level under the inset that must not show through.
    let mut levels = records();
    levels.levels.push(level("under", "c", 92.0));
    let list = [series("he", levels), inset_series()];
    assert_eq!(chart.config().validate_series(&list[..1]), Ok(()));
    let mut chart = chart;
    let inset = chart.config().subplots[0].config.clone();
    let inset_area = inset.data_area().unwrap().0;
    // A parent reference line through the inset's probe row: drawn on the
    // parent's overlay, it must be covered by the inset drawn after it.
    {
        use renderer::AnnotationSpace as _;
        let parent = renderer::annotation::pixel_space_for_config(chart.config());
        let probe_y = inset_area.y as f32 + inset_area.height as f32 * 0.75;
        let value = parent
            .px_to_value(renderer::AnnotationAxis::Y, probe_y)
            .unwrap();
        chart
            .config_mut()
            .annotations
            .items
            .push(renderer::annotation::Annotation {
                id: "through".into(),
                visible: true,
                kind: renderer::annotation::AnnotationKind::ReferenceLine {
                    axis: renderer::AnnotationAxis::Y,
                    value,
                    style: renderer::annotation::AnnotationLineStyle {
                        color: BLUE,
                        width: 6.0,
                        style: renderer::line::LineStylePreset::Solid,
                    },
                    label: None,
                },
            });
    }
    for scale in [1.0f32, 2.0] {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &list),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        if let Ok(dir) = std::env::var("FIGGY_ENERGY_DUMP") {
            std::fs::write(
                format!("{dir}/inset-{scale}x.rgba"),
                [
                    &img.width.to_le_bytes()[..],
                    &img.height.to_le_bytes()[..],
                    &img.rgba[..],
                ]
                .concat(),
            )
            .unwrap();
        }
        let s = |v: f32| (v * scale).round() as u32;
        // The inset's fill covers the parent inside it.
        // A quarter up the inset's plot, between its axes and off the
        // component's stroke.
        let centre = (
            s(inset_area.x as f32 + inset_area.width as f32 * 0.5),
            s(inset_area.y as f32 + inset_area.height as f32 * 0.75),
        );
        let fill = pixel(&img, centre.0, centre.1);
        assert!(
            fill[0] >= 245 && fill[1] >= 245 && fill[2] >= 245,
            "{scale}x inset fill {fill:?}"
        );
        // The component at 45 is drawn in the inset's own axes.
        let space = renderer::annotation::pixel_space_for_config(&inset.scaled(scale));
        use renderer::AnnotationSpace as _;
        let px = space
            .value_to_px(renderer::AnnotationAxis::X, -0.25)
            .unwrap();
        let py = space
            .value_to_px(renderer::AnnotationAxis::Y, 45.0)
            .unwrap();
        assert!(
            near(&img, (px.round() as u32, py.round() as u32), 3, is_red),
            "{scale}x the inset's component"
        );
        // A pointer on it picks the inset's series, the parent's covered.
        match pick(
            &mut r,
            &chart,
            &list,
            scale,
            (px.round() as u32, py.round() as u32),
        ) {
            Some(renderer::PickedData::HistogramBin {
                series_id,
                bin_index,
                ..
            }) => assert_eq!((series_id.as_str(), bin_index), ("inset", 2), "{scale}x"),
            other => panic!("{scale}x inset pick: {other:?}"),
        }
    }
}
