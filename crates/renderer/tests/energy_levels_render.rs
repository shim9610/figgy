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
