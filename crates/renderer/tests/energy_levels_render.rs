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
fn svg_and_pdf_exports_carry_the_series_as_vector_parts() {
    // The series is DataLess: the vector export writes it from the same
    // emit as its raster, as vector parts in the data slot (#234). A panel
    // with no other series writes no data image; the parts are clipped to
    // the data area, at 1x and 2x.
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
        assert!(!svg.contains("<image"), "{scale}x: no data image");
        for (part, colour) in [
            ("level", "#ff0000"),
            ("limit", "#0000ff"),
            ("transition", "#00c800"),
        ] {
            assert!(
                svg.contains(&format!("stroke=\"{colour}\"")),
                "{scale}x: the {part} is a vector stroke"
            );
        }
        // The arrowheads are filled triangles of the transition colour.
        assert!(svg.contains("fill=\"#00c800\""), "{scale}x: arrowheads");
        // The export places the document as `place_document` does.
        let size = |n: u32| (n as f32 * scale).round() as u32;
        let target = Rect {
            x: 0,
            y: 0,
            width: size(WIDTH),
            height: size(HEIGHT),
        };
        let placed = renderer::place_document(chart.config(), scale, target);
        let da = placed.data_area().expect("data area").0;
        let clip = format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
            da.x, da.y, da.width, da.height
        );
        assert!(svg.contains(&clip), "{scale}x: clipped to {clip}");
        let pdf = r
            .export_pdf(
                renderer::ExportSource::chart(&chart, &list),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"), "{scale}x pdf");
        let text = String::from_utf8_lossy(&pdf);
        assert!(!text.contains("/Subtype /Image"), "{scale}x: no PDF image");
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
        marker: None,
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

// #205: insets draw as units in their tree order. An inset listed before
// the inset it is nested in is still drawn over it, a later sibling covers
// both, and the pointer goes to the inset drawn on top.
#[test]
fn nested_and_overlapping_insets_draw_and_pick_in_tree_order() {
    let mut r = renderer();
    let mut chart = chart_with_inset();
    // A, made large enough to hold an inset of its own.
    chart.config_mut().chart_area.0.width = 640;
    chart.config_mut().chart_area.0.height = 480;
    let parent = chart.config().data_area().unwrap().0;
    chart.config_mut().subplots[0].config.chart_area = ChartArea(Rect {
        x: parent.x + parent.width - 270,
        y: parent.y + 10,
        width: 260,
        height: 200,
    });
    let a = chart.config().subplots[0].config.clone();
    let a_data = a.data_area().unwrap().0;
    let filled = |area: Rect, fill: Color| {
        let mut inset = a.clone();
        inset.chart_area = ChartArea(area);
        inset.plot_background = Some(fill);
        renderer::Subplot {
            config: inset,
            series: Vec::new(),
            link_x: None,
            link_y: None,
            marker: None,
        }
    };
    const GREEN: Color = Color::from_rgb8(190, 235, 190);
    const CREAM: Color = Color::from_rgb8(255, 236, 190);
    // N, nested in A's data area, listed first.
    let nested = filled(
        Rect {
            x: a_data.x + 1,
            y: a_data.y + 1,
            width: a_data.width - 2,
            height: a_data.height - 2,
        },
        GREEN,
    );
    let n_data = nested.config.data_area().unwrap().0;
    // B, a sibling of A, over N's lower-left corner.
    let b_area = Rect {
        x: n_data.x + n_data.width / 2 - 160,
        y: n_data.y - 20,
        width: 160,
        height: 120,
    };
    let sibling = filled(b_area, CREAM);
    let b_data = sibling.config.data_area().unwrap().0;
    chart.config_mut().subplots.insert(0, nested);
    chart.config_mut().subplots.push(sibling);
    let list = [series("he", records()), inset_series()];
    // Points in N alone, in N under B, and in B's plot.
    let n_only = (n_data.x + n_data.width - 6, n_data.y + 6);
    let under_b = (b_data.x + b_data.width - 6, n_data.y + n_data.height - 6);
    assert!(
        under_b.0 > n_data.x && under_b.1 > b_data.y,
        "{n_data:?} {b_data:?}"
    );
    let config = chart.config();
    let at = |p: (u32, u32)| config.nested_subplot_at(p.0 as f32 + 0.5, p.1 as f32 + 0.5);
    let a_area = chart.config().subplots[1].config.chart_area.0;
    assert!(a_area.within(&parent), "{a_area:?} in {parent:?}");
    assert_eq!(at(n_only), Some(0), "N over A");
    assert_eq!(at(under_b), Some(2), "B over N");
    let a_only = (a_data.x + a_data.width - 1, a.chart_area.0.y + 2);
    assert_eq!(at(a_only), Some(1), "A outside N");
    for scale in [1.0f32, 2.0] {
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &list),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        let s = |p: (u32, u32)| ((p.0 as f32 * scale) as u32, (p.1 as f32 * scale) as u32);
        let colour = |p: (u32, u32)| {
            let (x, y) = s(p);
            pixel(&img, x, y).to_vec()
        };
        assert_eq!(colour(n_only)[..3], [190, 235, 190], "{scale}x N over A");
        assert_eq!(colour(under_b)[..3], [255, 236, 190], "{scale}x B over N");
    }
}

// #205: a categorical axis takes a view's range as its category window.
// Only the slots inside it are drawn and picked, the annotation space
// follows it, and the full range clears it again.
#[test]
fn a_category_window_draws_and_picks_only_its_slots() {
    let mut r = renderer();
    let mut chart = chart();
    chart.set_x_range(0.5, 2.5);
    assert_eq!(chart.config().bottom_x.category_window, Some([0.5, 2.5]));
    assert_eq!(chart.config().validate_chart(), Ok(()));
    let list = [series("he", records())];
    let da = chart.config().data_area().unwrap().0;
    // Category `i` of the window sits at `(i - 0.5) / 2` of the plot.
    let at = |x: f64, y: f64, scale: f32| {
        let px = da.x as f64 + (x - 0.5) / 2.0 * da.width as f64;
        let py = da.y as f64 + (1.0 - y / 100.0) * da.height as f64;
        (
            (px * scale as f64).round() as u32,
            (py * scale as f64).round() as u32,
        )
    };
    {
        use renderer::AnnotationSpace as _;
        let space = renderer::annotation::pixel_space_for_config(chart.config());
        let px = space.value_to_px(renderer::AnnotationAxis::X, 2.0).unwrap();
        assert!(
            (px - at(2.0, 0.0, 1.0).0 as f32).abs() <= 1.0,
            "annotation x {px}"
        );
    }
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &list, scale);
        // Column c at 60 is drawn where the window puts it, and column a's
        // level (centre -0.25 of the plot) leaves no stroke in the plot.
        assert!(near(&img, at(2.0, 60.0, scale), 2, is_red), "{scale}x c");
        let left = (((da.x as f32 + 2.0) * scale) as u32, at(0.0, 20.0, scale).1);
        assert!(!near(&img, left, 1, is_red), "{scale}x a is outside");
        // Off the transition's end at the level's centre.
        match pick(&mut r, &chart, &list, scale, at(2.3, 60.0, scale)) {
            Some(renderer::PickedData::HistogramBin { bin_index, .. }) => {
                assert_eq!(bin_index, 3, "{scale}x")
            }
            other => panic!("{scale}x window pick: {other:?}"),
        }
    }
    chart.set_x_range(-0.5, 2.5);
    assert_eq!(chart.config().bottom_x.category_window, None);
}

fn is_magenta(p: &[u8]) -> bool {
    p[3] > 200 && p[0] > 200 && p[2] > 200 && p[1] < 80
}

// #205: the SSOT marker is drawn on every frame from the inset: the window
// around the parent slot the inset shows, and connectors that follow the
// inset when it moves. At 1x and 2x.
#[test]
fn the_inset_marker_is_drawn_and_follows_the_inset() {
    let mut r = renderer();
    let mut chart = chart_with_inset();
    chart.config_mut().chart_area.0.width = 640;
    chart.config_mut().chart_area.0.height = 480;
    let plot = chart.config().data_area().unwrap().0;
    // Right of the window around column b, then moved left of it.
    let place = |x: u32| Rect {
        x,
        y: plot.y + 8,
        width: 160,
        height: 120,
    };
    chart.config_mut().subplots[0].config.chart_area =
        ChartArea(place(plot.x + plot.width * 2 / 3 + 8));
    chart.config_mut().subplots[0].marker = Some(renderer::InsetMarker {
        line: renderer::annotation::AnnotationLineStyle {
            color: Color::from_rgb8(255, 0, 255),
            width: 2.0,
            style: renderer::line::LineStylePreset::Solid,
        },
        ..renderer::InsetMarker::default()
    });
    let list = [series("he", records()), inset_series()];
    for moved in [false, true] {
        if moved {
            chart.config_mut().subplots[0].config.chart_area = ChartArea(place(plot.x + 4));
        }
        let parts = chart.config().inset_marker_parts(0).expect("a marker");
        let [link, _] = parts.links.expect("connectors");
        for scale in [1.0f32, 2.0] {
            let img = export(&mut r, &chart, &list, scale);
            let s = |p: [f32; 2]| ((p[0] * scale).round() as u32, (p[1] * scale).round() as u32);
            let w = parts.window;
            // The window's bottom edge, off the level drawn at its centre.
            let edge = s([w.x + w.width * 0.25, w.y + w.height]);
            assert!(
                near(&img, edge, 2, is_magenta),
                "{scale}x window, moved {moved}"
            );
            // A point a third of the way along the first connector.
            let along = s([
                link[0][0] + (link[1][0] - link[0][0]) / 3.0,
                link[0][1] + (link[1][1] - link[0][1]) / 3.0,
            ]);
            assert!(
                near(&img, along, 2, is_magenta),
                "{scale}x connector, moved {moved}"
            );
        }
    }
}

/// The anchor space and text measure of `chart`'s panel at 1x.
fn layout_of(
    chart: &Chart,
    _levels: &EnergyLevelSeries,
) -> (renderer::annotation::PixelSpace, renderer::CpuTextMeasure) {
    let config = chart.config();
    (
        renderer::annotation::pixel_space_for_config(config),
        renderer::CpuTextMeasure::for_style(&config.draw_style),
    )
}

fn labelled(id: &str, column: &str, energy: f64) -> EnergyLevel {
    EnergyLevel {
        label: Some(renderer::text::RichText::plain(id, Color::BLACK, 12.0, "")),
        ..level(id, column, energy)
    }
}

// #205: a crowd that a stack would push far from its strokes collapses
// its dense run to a bracket, so every drawn label stays within one and a
// half label heights of its own level; an isolated label is not moved.
#[test]
fn a_crowd_collapses_instead_of_fanning_out() {
    let chart = chart();
    let mut levels = EnergyLevelSeries::default();
    // Twelve levels about two thirds of a label apart: too far apart for
    // the dense-run rule, too close to stack near their strokes.
    for k in 0..12 {
        levels
            .levels
            .push(labelled(&format!("c{k}"), "b", 20.0 + 4.0 * k as f64));
    }
    levels.levels.push(labelled("alone", "b", 92.0));
    let (space, measure) = layout_of(&chart, &levels);
    let placement = renderer::energy_levels::LevelPlacement {
        space: &space,
        categories: chart.config().bottom_x.categories.as_deref().unwrap(),
        view: LevelView::Main,
        colorbar: None,
        picked: None,
    };
    let layout = placement.layout(&levels, &levels.style, &measure);
    let label = |i: usize| {
        placement.label(
            &levels,
            &levels.style,
            &layout,
            renderer::energy_levels::LabelPart::Level,
            i,
            &measure,
        )
    };
    let drawn: Vec<usize> = (0..12).filter(|&i| label(i).is_some()).collect();
    assert!(drawn.len() < 12, "the crowd collapses: {drawn:?}");
    assert!(
        drawn.contains(&0) && drawn.contains(&11),
        "its ends stay: {drawn:?}"
    );
    use renderer::AnnotationSpace as _;
    for i in (0..levels.levels.len()).filter(|&i| label(i).is_some()) {
        let rect = label(i).unwrap().rect();
        let stroke = space
            .value_to_px(renderer::AnnotationAxis::Y, levels.levels[i].energy)
            .unwrap();
        let shift = (rect.y + rect.height / 2.0 - stroke).abs();
        assert!(
            shift <= 1.5 * rect.height + 0.5,
            "label {i} moved {shift} px"
        );
    }
    let alone = label(12).expect("the isolated label").rect();
    let stroke = space
        .value_to_px(renderer::AnnotationAxis::Y, 92.0)
        .unwrap();
    assert!(
        (alone.y + alone.height / 2.0 - stroke).abs() < 1.0,
        "not moved"
    );
}

// #205: crowded labels in one column stack clear of each other, and a
// dense run (strokes closer than half a label) keeps only its first and
// last labels, joined by a bracket.
#[test]
fn crowded_labels_stack_apart_and_dense_runs_collapse() {
    let chart = chart();
    let mut levels = EnergyLevelSeries::default();
    // Five levels a little closer than a label is tall, then a run of
    // twenty a fraction of a label apart.
    for k in 0..5 {
        levels
            .levels
            .push(labelled(&format!("a{k}"), "b", 20.0 + 3.0 * k as f64));
    }
    for k in 0..20 {
        levels
            .levels
            .push(labelled(&format!("r{k}"), "b", 60.0 + 0.2 * k as f64));
    }
    let (space, measure) = layout_of(&chart, &levels);
    let placement = renderer::energy_levels::LevelPlacement {
        space: &space,
        categories: chart.config().bottom_x.categories.as_deref().unwrap(),
        view: LevelView::Main,
        colorbar: None,
        picked: None,
    };
    let layout = placement.layout(&levels, &levels.style, &measure);
    let label = |i: usize| {
        placement.label(
            &levels,
            &levels.style,
            &layout,
            renderer::energy_levels::LabelPart::Level,
            i,
            &measure,
        )
    };
    let rects: Vec<_> = (0..levels.levels.len())
        .filter_map(|i| label(i).map(|t| (i, t.rect())))
        .collect();
    // Every drawn label is clear of every other.
    for (n, (i, a)) in rects.iter().enumerate() {
        for (j, b) in &rects[n + 1..] {
            let apart = a.x + a.width <= b.x
                || b.x + b.width <= a.x
                || a.y + a.height <= b.y
                || b.y + b.height <= a.y;
            assert!(apart, "labels {i} and {j} overlap: {a:?} {b:?}");
        }
    }
    // The spaced five are all drawn; the dense run keeps its ends only.
    for i in 0..5 {
        assert!(label(i).is_some(), "label a{i}");
    }
    assert!(label(5).is_some() && label(24).is_some(), "the run's ends");
    assert!((6..24).all(|i| label(i).is_none()), "the run's middle");
}

// #205: a band fills its run, answers a pick on its fill for its first
// level, and its label picks as the band part.
#[test]
fn a_band_fills_its_run_and_answers_picks() {
    let mut r = renderer();
    let chart = chart();
    let mut levels = records();
    for (k, e) in [50.0, 52.0, 54.0, 56.0, 58.0].into_iter().enumerate() {
        levels.levels.push(level(&format!("n{k}"), "b", e));
    }
    levels.bands.push(renderer::energy_levels::LevelBand {
        id: "run".into(),
        column: "b".into(),
        first: "n0".into(),
        last: "n4".into(),
        label: Some(renderer::text::RichText::plain(
            "n ≥ 5",
            Color::BLACK,
            14.0,
            "",
        )),
        label_offset: [0.0; 2],
    });
    levels.style.band.fill = Color::from_rgb8(0, 200, 200);
    levels.validate().unwrap();
    let list = [series("he", levels.clone())];
    for scale in [1.0f32, 2.0] {
        let img = export(&mut r, &chart, &list, scale);
        // The fill between two strokes of the run.
        let (x, y) = at(&chart, 1.0, 51.0, scale);
        let p = pixel(&img, x, y);
        assert!(
            p[0] < 40 && p[1] > 150 && p[2] > 150,
            "{scale}x band fill {p:?}"
        );
        // A pick on the fill answers for the run's first level.
        let first = levels.levels.iter().position(|l| l.id == "n0").unwrap();
        match pick(&mut r, &chart, &list, scale, (x, y)) {
            Some(renderer::PickedData::HistogramBin { bin_index, .. }) => {
                assert_eq!(bin_index, first, "{scale}x band body")
            }
            other => panic!("{scale}x band body: {other:?}"),
        }
        // The band's label, right of the band's middle.
        let (bx, by) = at(&chart, 1.0 + 0.35, 54.0, scale);
        match pick(
            &mut r,
            &chart,
            &list,
            scale,
            (bx + (12.0 * scale) as u32, by),
        ) {
            Some(renderer::PickedData::ValueLabel {
                category_index,
                part,
                ..
            }) => assert_eq!(
                (category_index, part),
                (0, Some(renderer::energy_levels::LabelPart::Band)),
                "{scale}x band label"
            ),
            other => panic!("{scale}x band label: {other:?}"),
        }
    }
}

// #205: on the base layer a press on an inset's plot selects its frame. The
// frame's selection box and its eight handles lie inside the frame, so the
// box is composited into the inset's own panel wherever the inset is, at 1x
// and 2x; the parent's plot beside the inset stays the parent's.
#[test]
fn a_subplot_frame_selects_and_its_box_lies_inside_the_frame() {
    use renderer::axis_render::{AxisLayerKind, try_raster_chart_layer_to_rgba_with_selection};
    use renderer::layout::RectF;
    let r = renderer();
    let chart = chart_with_inset();
    let config = chart.config();
    let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
    let map = renderer::select::HitMap::for_active_layer(config);
    let plot = config.subplot_config(0).unwrap().data_area().unwrap().0;
    let centre = (
        plot.x as f32 + plot.width as f32 / 2.0,
        plot.y as f32 + plot.height as f32 / 2.0,
    );
    let hit = map
        .hit_test(config, &measure, centre.0, centre.1)
        .expect("the inset's plot is hit");
    assert_eq!(map.get(hit).unwrap().element_id(), "subplot_0:frame");
    let parent = config.data_area().unwrap().0;
    let beside = map
        .hit_test(
            config,
            &measure,
            parent.x as f32 + parent.width as f32 * 0.25,
            parent.y as f32 + parent.height as f32 * 0.75,
        )
        .expect("the parent's plot is hit");
    assert_eq!(map.get(beside).unwrap().element_id(), "data_area");
    for scale in [1.0f32, 2.0] {
        let scaled = Chart::new(config.scaled(scale));
        let sb = r
            .selection_box(&scaled, "subplot_0:frame")
            .expect("a selection box");
        let frame = RectF::from_rect(&scaled.config().subplots[0].config.chart_area.0);
        let rect = sb.rect();
        assert!(
            rect.x > frame.x
                && rect.y > frame.y
                && rect.x + rect.width < frame.x + frame.width
                && rect.y + rect.height < frame.y + frame.height,
            "{scale}x box {rect:?} inside frame {frame:?}"
        );
        let handles = sb.handles();
        assert_eq!(handles.len(), 8, "{scale}x");
        for handle in handles {
            assert!(
                frame.contains(handle.x, handle.y)
                    && frame.contains(handle.x + handle.width, handle.y + handle.height),
                "{scale}x handle {handle:?} inside frame {frame:?}"
            );
        }
        // Rastered into the inset's own panel, the highlight is drawn.
        let panel = scaled.config().subplot_config(0).unwrap();
        let plain =
            try_raster_chart_layer_to_rgba_with_selection(&panel, AxisLayerKind::Decoration, &[])
                .unwrap();
        let selected = try_raster_chart_layer_to_rgba_with_selection(
            &panel,
            AxisLayerKind::Decoration,
            std::slice::from_ref(&sb),
        )
        .unwrap();
        let changed = plain
            .chunks(4)
            .zip(selected.chunks(4))
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed > 0,
            "{scale}x the box is drawn in the inset's panel"
        );
    }
}

// #205: a subplot moved out of its parent is a panel of its own: the
// document grows to hold it, it draws there at 1x and 2x, a base-layer pick
// in it answers its series and steps over them, the vector exports carry
// it, and the moved document round-trips.
#[test]
fn a_subplot_moved_out_of_its_parent_draws_picks_and_exports_there() {
    let mut r = renderer();
    let mut chart = chart_with_inset();
    let list = vec![series("he", records()), inset_series()];
    let parent = chart.config().chart_area.0;
    let frame = Rect {
        x: parent.x + parent.width + 20,
        y: 30,
        width: 160,
        height: 120,
    };
    chart
        .config_mut()
        .set_subplot_area(0, ChartArea(frame))
        .unwrap();
    assert_eq!(chart.config().insets().count(), 0, "a panel of its own");
    assert_eq!(
        chart
            .config()
            .subplot_at(frame.x as f32 + 80.0, frame.y as f32 + 60.0),
        Some(0)
    );
    assert_eq!(
        chart
            .config()
            .subplot_at(parent.x as f32 + 40.0, parent.y as f32 + 40.0),
        None
    );
    let document = chart.config().document_area();
    assert_eq!((document.x, document.y), (0, 0));
    assert_eq!(document.x + document.width, frame.x + frame.width);
    let inset = chart.config().subplot_config(0).unwrap();
    r.enable_gpu_picking().unwrap();
    let id = r
        .register_chart(chart.config().clone(), list.clone())
        .unwrap();
    for scale in [1.0f32, 2.0] {
        let s = |v: u32| (v as f32 * scale).round() as u32;
        let img = r
            .export_rgba(
                renderer::ExportSource::chart(&chart, &list),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        assert_eq!(
            (img.width, img.height),
            (s(document.width), s(document.height)),
            "{scale}x document size"
        );
        // The parent's lowest level still draws in column a.
        assert!(
            near(&img, at(&chart, -0.25, 20.0, scale), 3, is_red),
            "{scale}x parent"
        );
        // The subplot's component at 45 draws in its own axes at the new place.
        let space = renderer::annotation::pixel_space_for_config(&inset.scaled(scale));
        use renderer::AnnotationSpace as _;
        let px = space
            .value_to_px(renderer::AnnotationAxis::X, -0.25)
            .unwrap();
        let py = space
            .value_to_px(renderer::AnnotationAxis::Y, 45.0)
            .unwrap();
        let at_inset = (px.round() as u32, py.round() as u32);
        assert!(
            near(&img, at_inset, 3, is_red),
            "{scale}x the moved subplot's component"
        );
        // A base-layer pick there answers the subplot's series.
        let request = renderer::GpuPickRequest {
            canvas_position_px: [at_inset.0 as f32, at_inset.1 as f32],
            display_panel_px: Rect {
                x: 0,
                y: 0,
                width: s(document.width),
                height: s(document.height),
            },
            display_scale: scale,
            max_distance_px: 4.0 * scale,
        };
        match test_gpu::block_on(r.pick_chart_data(id, request).unwrap().resolve()).unwrap() {
            Some(renderer::PickedData::HistogramBin {
                series_id,
                bin_index,
                ..
            }) => assert_eq!((series_id.as_str(), bin_index), ("inset", 2), "{scale}x"),
            other => panic!("{scale}x pick in the moved subplot: {other:?}"),
        }
        let svg = r
            .export_svg(
                renderer::ExportSource::chart(&chart, &list),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        // The subplot's levels are vector strokes of the level colour, in
        // the subplot's own clipped data slot (#234).
        assert!(!svg.contains("<image"), "{scale}x svg: no data image");
        assert!(
            svg.contains("stroke=\"#ff0000\""),
            "{scale}x svg: the levels are vector strokes"
        );
        let pdf = r
            .export_pdf(
                renderer::ExportSource::chart(&chart, &list),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        assert!(pdf.starts_with(b"%PDF"), "{scale}x pdf");
    }
    // The base-layer pick's selection steps over the subplot's drawn rows
    // as any energy-level selection does: the series list is the chart's.
    let mut stepped = chart.config().clone();
    stepped.picked_data = Some(DataSelectionsConfig {
        refs: vec![PickedDataRef::HistogramBin {
            source_id: None,
            series_id: "inset".into(),
            bin_index: 2,
            half: None,
        }],
        ..DataSelectionsConfig::default()
    });
    assert_eq!(stepped.step_selection(&list, 1, 0, |_| None), Ok(true));
    assert!(
        matches!(
            stepped.picked_data.as_ref().unwrap().refs[0],
            PickedDataRef::HistogramBin { bin_index: 3, .. }
        ),
        "{:?}",
        stepped.picked_data
    );
    assert_eq!(stepped.step_selection(&list, -1, 0, |_| None), Ok(true));
    assert!(matches!(
        stepped.picked_data.as_ref().unwrap().refs[0],
        PickedDataRef::HistogramBin { bin_index: 2, .. }
    ));
    r.remove_chart(id).unwrap();
    let json = serde_json::to_string(chart.config()).unwrap();
    let back: renderer::config::Config = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, chart.config());
    assert_eq!(serde_json::to_string(&back).unwrap(), json);
}

// #205: a frame move is an SSOT edit of the document: the registered chart
// redraws the inset at its new place at 1x and 2x.
#[test]
fn a_moved_frame_redraws_the_inset_where_it_now_is() {
    let mut r = renderer();
    let mut chart = chart_with_inset();
    chart.config_mut().subplots[0].config.plot_background = Some(Color::from_rgb8(255, 250, 200));
    let list = vec![series("he", records()), inset_series()];
    let id = r.register_chart(chart.config().clone(), list).unwrap();
    let before = chart.config().subplots[0].config.chart_area.0;
    let after = Rect {
        y: before.y + 60,
        ..before
    };
    // A quarter up the inset's plot, off the component's stroke.
    let probe = |frame: Rect, scale: f32| {
        let mut inset = chart.config().subplots[0].config.clone();
        inset.chart_area = ChartArea(frame);
        let plot = inset.data_area().unwrap().0;
        (
            ((plot.x as f32 + plot.width as f32 * 0.5) * scale).round() as u32,
            ((plot.y as f32 + plot.height as f32 * 0.75) * scale).round() as u32,
        )
    };
    let is_cream = |p: &[u8]| p[0] >= 250 && p[1] >= 245 && (150..=215).contains(&p[2]);
    for scale in [1.0f32, 2.0] {
        let first = r
            .export_rgba(
                renderer::ExportSource::Registered(id),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        let (x, y) = probe(before, scale);
        assert!(
            is_cream(pixel(&first, x, y)),
            "{scale}x the inset's fill at its first place"
        );
        let (x, y) = probe(after, scale);
        assert!(
            !is_cream(pixel(&first, x, y)),
            "{scale}x nothing of it at the next place yet"
        );
        let mut moved = chart.config().clone();
        moved.set_subplot_area(0, ChartArea(after)).unwrap();
        r.set_chart_config(id, moved).unwrap();
        let second = r
            .export_rgba(
                renderer::ExportSource::Registered(id),
                renderer::ExportOptions::document(scale),
            )
            .block_on()
            .unwrap();
        let (x, y) = probe(after, scale);
        assert!(
            is_cream(pixel(&second, x, y)),
            "{scale}x the fill follows the frame"
        );
        let (x, y) = probe(before, scale);
        assert!(
            !is_cream(pixel(&second, x, y)),
            "{scale}x and leaves its first place"
        );
        r.set_chart_config(id, chart.config().clone()).unwrap();
    }
}

// #205: while a host holds the display placement, a frame moved past the
// document's edge grows the document without rescaling the page under the
// pointer; the release fits the document to the surface again.
#[test]
fn a_held_display_keeps_its_scale_and_origin_while_a_frame_grows_the_document() {
    let mut r = renderer();
    let chart = chart_with_inset();
    let id = r
        .register_chart(
            chart.config().clone(),
            vec![series("he", records()), inset_series()],
        )
        .unwrap();
    let surface = (WIDTH * 2, HEIGHT * 2);
    r.set_chart_display(id, Some(surface)).unwrap();
    let (origin, scale) = {
        let (placed, _, scale) = r.chart_display_config(id).unwrap();
        (placed.chart_area.0, scale)
    };
    assert_eq!(scale, 2.0);
    r.hold_chart_display(id, true).unwrap();
    let far = Rect {
        x: WIDTH + 20,
        y: 30,
        width: 160,
        height: 120,
    };
    let mut moved = chart.config().clone();
    moved.set_subplot_area(0, ChartArea(far)).unwrap();
    r.set_chart_config(id, moved.clone()).unwrap();
    {
        let (held, _, held_scale) = r.chart_display_config(id).unwrap();
        assert_eq!(held_scale, 2.0, "the hold keeps the scale");
        assert_eq!(held.chart_area.0, origin, "and the parent's place");
        assert_eq!(
            held.subplots[0].config.chart_area.0,
            Rect {
                x: origin.x + far.x * 2,
                y: origin.y + far.y * 2,
                width: far.width * 2,
                height: far.height * 2,
            },
            "the subplot is placed at the held scale from the parent's origin"
        );
    }
    r.hold_chart_display(id, false).unwrap();
    let (_, _, fitted) = r.chart_display_config(id).unwrap();
    let document = moved.document_area();
    let (expected, _) = renderer::fit_display_panel((document.width, document.height), surface);
    assert_eq!(
        fitted, expected,
        "released, the document fits the surface again"
    );
    assert!(fitted < 2.0);
}
