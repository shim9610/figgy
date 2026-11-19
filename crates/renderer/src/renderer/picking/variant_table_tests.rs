//! The exact pick variant table (#363): `tests/support/pick_variants.csv`.
//!
//! One row per exact pick pipeline a chart can reach: the module, the
//! `PICK_VARIANT` bits, the marker shape and the shader set, with the series
//! families and options that produce it and the pipeline's measured
//! SwiftShader compile time and budget. The rows are never written by hand.
//! [`pick_variant_table_matches_the_picker`] builds every series family over
//! every combination of the options that feed the exact variant, registers
//! each chart, prepares its pick as `pick_chart` does
//! ([`Renderer::exact_pick_query`]) and reads the keys from the picker
//! (`exact_variants`, and `stream_exact_variants` where every series of the
//! chart may stream). The table also records, for each exact module and
//! shader set, the hash of the assembled source its rows were measured on
//! and the fragment files it is assembled from; `ci/test_pick_variants.py`
//! rehashes those fragments in `python3 ci/verify.py static`, so a change to
//! an exact module forces that module's rows, and only those, to be
//! re-measured. A WGSL change outside the exact modules changes nothing here.
//!
//! Options that do not feed the variant are held at one value: colours,
//! sizes, widths above zero, step interpolation, size and colour maps, axis
//! scales and breaks (they choose the shader set, and every key is listed
//! for the full set), subplots, overlays and line offsets.

use super::*;
use crate::config::{ConstellationOptions, DrawStyle, MilkywayOptions, SketchOptions};
use crate::data::Column;
use crate::data_config::{
    BarGroups, BoxOutlierShape, BoxPlotSeriesLabels, BoxPlotStyle, BoxPoints, BoxPointsStyle,
    BoxSeries, CategoryBarDirection, DataErrorBarStyleConfig, DataScatterPointStyleConfig,
    DataScatterPointStyleOverride, ErrorBars, ErrorRef, LineFill, RadarPolygonSeries, Stem,
    StreamlineSeries, TernarySeries, VectorComponents, VectorPivot, VectorScale, VectorSeries,
    VectorStyle, XySeries,
};
use crate::data_config::{
    DataLineStyleConfig, DataScatterStyleConfig, LineInterpolation, ScatterShape,
};
use crate::gpu_pick::{PickExactKey, PickShader, exact_variant};
use crate::line::LineStylePreset;
use crate::shader_assembly::ShaderSet;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

/// The table, relative to the renderer crate.
const TABLE: &str = "tests/support/pick_variants.csv";
/// Set to 1 to rewrite the rows and hashes (`FIGGY_UPDATE_*`, as the shader
/// pipeline contract).
const UPDATE: &str = "FIGGY_UPDATE_PICK_VARIANTS";
const UPDATE_COMMAND: &str = "FIGGY_UPDATE_PICK_VARIANTS=1 cargo test -p figgy-renderer --lib \
pick_variant_table_matches_the_picker";
const MEASURE_COMMAND: &str = "node ci/shader_compile.cjs --pick-variants \
target/ci-results/shader-compilation-native.json crates/renderer/tests/support/pick_variants.csv \
--update";
/// The columns of a row, in order.
const HEADER: &str =
    "module,set,bits,bits_names,shape,constants,families,example,cases,measured_ms,budget_ms";
/// The workgroup-zeroing override is a DX12 workaround the browser never
/// takes (`gpu_compile::workgroup_options`); the table lists what a browser
/// compiles.
const NATIVE_ONLY_CONSTANT: &str = "FIGGY_EXPLICIT_WORKGROUP_ZERO";

/// 64-bit FNV-1a, the table's hash (`ci/test_pick_variants.py` and
/// `ci/shader_compile.cjs` implement the same).
fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

fn module_name(shader: PickShader) -> &'static str {
    match shader {
        PickShader::Gate => "gate",
        PickShader::Marker => "marker",
        PickShader::MarkerLine => "marker-line",
        PickShader::Line => "line",
        PickShader::Vector => "vector",
    }
}

fn set_name(set: ShaderSet) -> &'static str {
    match set {
        ShaderSet::Basic => "basic",
        ShaderSet::Full => "full",
    }
}

/// One exact module of one shader set as the table records it: its name
/// (`marker-line/full`), the hash of its assembled source (what the browser
/// compiles for its rows; `ci/shader_compile.cjs` finds the capture's
/// source by it) and its fragment files relative to the crate.
/// `ci/test_pick_variants.py` rehashes the fragments, joined with one
/// newline as the assembly joins them, so a change to any of them fails the
/// static check for that module's rows only.
struct ModuleRecord {
    name: String,
    hash: String,
    fragments: Vec<String>,
}

fn module_records() -> Vec<ModuleRecord> {
    let mut out = Vec::new();
    for shader in PickShader::EXACT {
        for set in [ShaderSet::Basic, ShaderSet::Full] {
            let assembled = shader.assembled_for(set);
            let name = format!("{}/{}", module_name(shader), set_name(set));
            // What the static check rehashes is the source itself.
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
            let joined = assembled
                .fragments
                .iter()
                .map(|fragment| std::fs::read_to_string(root.join(fragment)).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(joined, assembled.source, "{name} is its fragments joined");
            out.push(ModuleRecord {
                hash: format!("{:016x}", fnv1a(FNV_OFFSET, assembled.source.as_bytes())),
                fragments: assembled
                    .fragments
                    .iter()
                    .map(|fragment| format!("src/{fragment}"))
                    .collect(),
                name,
            });
        }
    }
    out
}

/// Every marker shape. The match below has no wildcard, so a new shape
/// fails to compile until it is listed.
const SHAPES: [ScatterShape; 26] = [
    ScatterShape::Circle,
    ScatterShape::Square,
    ScatterShape::Triangle,
    ScatterShape::Diamond,
    ScatterShape::Cross,
    ScatterShape::CircleFilled,
    ScatterShape::SquareFilled,
    ScatterShape::TriangleFilled,
    ScatterShape::DiamondFilled,
    ScatterShape::TriangleDown,
    ScatterShape::TriangleLeft,
    ScatterShape::TriangleRight,
    ScatterShape::Plus,
    ScatterShape::Pentagon,
    ScatterShape::Hexagon,
    ScatterShape::Octagon,
    ScatterShape::Star,
    ScatterShape::TriangleDownFilled,
    ScatterShape::TriangleLeftFilled,
    ScatterShape::TriangleRightFilled,
    ScatterShape::PlusFilled,
    ScatterShape::CrossFilled,
    ScatterShape::PentagonFilled,
    ScatterShape::HexagonFilled,
    ScatterShape::OctagonFilled,
    ScatterShape::StarFilled,
];

#[allow(dead_code)]
fn every_shape_is_listed(shape: ScatterShape) {
    match shape {
        ScatterShape::Circle
        | ScatterShape::Square
        | ScatterShape::Triangle
        | ScatterShape::Diamond
        | ScatterShape::Cross
        | ScatterShape::CircleFilled
        | ScatterShape::SquareFilled
        | ScatterShape::TriangleFilled
        | ScatterShape::DiamondFilled
        | ScatterShape::TriangleDown
        | ScatterShape::TriangleLeft
        | ScatterShape::TriangleRight
        | ScatterShape::Plus
        | ScatterShape::Pentagon
        | ScatterShape::Hexagon
        | ScatterShape::Octagon
        | ScatterShape::Star
        | ScatterShape::TriangleDownFilled
        | ScatterShape::TriangleLeftFilled
        | ScatterShape::TriangleRightFilled
        | ScatterShape::PlusFilled
        | ScatterShape::CrossFilled
        | ScatterShape::PentagonFilled
        | ScatterShape::HexagonFilled
        | ScatterShape::OctagonFilled
        | ScatterShape::StarFilled => {}
    }
}

/// Every line dash preset, checked the same way.
const PRESETS: [LineStylePreset; 11] = [
    LineStylePreset::Solid,
    LineStylePreset::Dash,
    LineStylePreset::Dot,
    LineStylePreset::DashDot,
    LineStylePreset::DashDotDot,
    LineStylePreset::ShortDash,
    LineStylePreset::ShortDot,
    LineStylePreset::ShortDashDot,
    LineStylePreset::LongDash,
    LineStylePreset::LongDashDot,
    LineStylePreset::LongDashDotDot,
];

#[allow(dead_code)]
fn every_preset_is_listed(preset: LineStylePreset) {
    match preset {
        LineStylePreset::Solid
        | LineStylePreset::Dash
        | LineStylePreset::Dot
        | LineStylePreset::DashDot
        | LineStylePreset::DashDotDot
        | LineStylePreset::ShortDash
        | LineStylePreset::ShortDot
        | LineStylePreset::ShortDashDot
        | LineStylePreset::LongDash
        | LineStylePreset::LongDashDot
        | LineStylePreset::LongDashDotDot => {}
    }
}

/// Every draw style, checked the same way.
fn draw_styles() -> [(&'static str, DrawStyle); 4] {
    [
        ("precise", DrawStyle::Precise),
        ("sketch", DrawStyle::Sketch(SketchOptions::default())),
        ("milkyway", DrawStyle::Milkyway(MilkywayOptions::default())),
        (
            "constellation",
            DrawStyle::Constellation(ConstellationOptions::default()),
        ),
    ]
}

#[allow(dead_code)]
fn every_draw_style_is_listed(style: DrawStyle) {
    match style {
        DrawStyle::Precise
        | DrawStyle::Sketch(_)
        | DrawStyle::Milkyway(_)
        | DrawStyle::Constellation(_) => {}
    }
}

/// The marker option of a scatter: a fixed shape, or a style map (a table
/// with an index column, or sparse overrides), which lets a point change
/// its shape.
#[derive(Clone)]
enum Marker {
    None,
    Shape(ScatterShape),
    StyleTable,
    StyleOverrides,
}

impl Marker {
    fn all() -> Vec<Marker> {
        let mut out = vec![Marker::None];
        out.extend(SHAPES.iter().cloned().map(Marker::Shape));
        out.push(Marker::StyleTable);
        out.push(Marker::StyleOverrides);
        out
    }

    fn label(&self) -> String {
        match self {
            Marker::None => "markers=none".into(),
            Marker::Shape(shape) => format!("markers={shape:?}"),
            Marker::StyleTable => "markers=style-table".into(),
            Marker::StyleOverrides => "markers=style-overrides".into(),
        }
    }

    fn scatter(&self) -> Option<DataScatterStyleConfig> {
        let base = |shape| DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: shape,
            point_size: 4.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        };
        let slot = DataScatterPointStyleConfig {
            point_color: None,
            point_shape: Some(ScatterShape::Square),
            point_size: Some(6.0),
        };
        match self {
            Marker::None => None,
            Marker::Shape(shape) => Some(base(shape.clone())),
            Marker::StyleTable => {
                let mut scatter = base(ScatterShape::CircleFilled);
                scatter.point_style_table = Some(vec![slot.clone(), slot]);
                scatter.point_style_index_column = Some("idx".into());
                Some(scatter)
            }
            Marker::StyleOverrides => {
                let mut scatter = base(ScatterShape::CircleFilled);
                scatter.point_style_overrides = Some(vec![DataScatterPointStyleOverride {
                    index: 1,
                    style: slot,
                }]);
                Some(scatter)
            }
        }
    }
}

fn line_style(preset: LineStylePreset, width: f32) -> DataLineStyleConfig {
    DataLineStyleConfig {
        line_style: preset,
        line_color: Color::BLACK,
        line_width: width,
        fill: None,
        fill_color: None,
        interpolation: LineInterpolation::Linear,
    }
}

fn series(id: &str, x: &str, y: &str, render_type: DataRenderType) -> SeriesConfig {
    SeriesConfig {
        series_id: id.into(),
        source_id: None,
        label: None,
        x_column: x.into(),
        y_column: y.into(),
        z_column: None,
        render_type,
    }
}

/// A chart over the columns below: x and y over 0..10, or a polar chart
/// over 0..360 degrees and 0..10.
fn chart_config(polar: bool, draw: &DrawStyle) -> Config {
    let mut config = crate::default::default_config();
    config.chart_area = crate::layout::ChartArea(Rect {
        x: 0,
        y: 0,
        width: 480,
        height: 360,
    });
    config.legend.visible = false;
    let x = if polar { (0.0, 360.0) } else { (0.0, 10.0) };
    for axis in [&mut config.bottom_x, &mut config.top_x] {
        axis.min = x.0;
        axis.max = x.1;
    }
    for axis in [&mut config.left_y, &mut config.right_y] {
        axis.min = 0.0;
        axis.max = 10.0;
    }
    if polar {
        config.set_polar(Some(crate::polar::PolarOptions::default()));
    }
    config.draw_style = draw.clone();
    config
}

/// One chart of the enumeration.
struct Case {
    family: &'static str,
    options: String,
    config: Config,
    series: Vec<SeriesConfig>,
}

fn coordinates() -> [(&'static str, bool); 2] {
    [("cartesian", false), ("polar", true)]
}

/// Markers, a line, error bars, stems and a fill on an xy series, over
/// cartesian and polar axes and every draw style.
fn xy_cases(out: &mut Vec<Case>) {
    let errors = [("errors=none", 0), ("errors=y", 1), ("errors=x", 2)];
    let fills = [("fill=none", 0), ("fill=baseline", 1), ("fill=series", 2)];
    for (coords, polar) in coordinates() {
        for (draw_name, draw) in draw_styles() {
            for marker in Marker::all() {
                let mut lines: Vec<Option<LineStylePreset>> = vec![None];
                lines.extend(PRESETS.iter().copied().map(Some));
                for line in lines {
                    if matches!(marker, Marker::None) && line.is_none() {
                        continue;
                    }
                    for (errors_name, errors_kind) in errors {
                        for stem in [false, true] {
                            for (fill_name, fill_kind) in fills {
                                let scatter = marker.scatter();
                                let mut line_config = line.map(|preset| line_style(preset, 2.0));
                                if let Some(line) = line_config.as_mut() {
                                    line.fill = match fill_kind {
                                        1 => Some(LineFill::ToBaseline(0.0)),
                                        2 => Some(LineFill::ToSeries("base".into())),
                                        _ => None,
                                    };
                                    line.fill_color = Some(Color::BLACK);
                                } else if fill_kind != 0 {
                                    continue;
                                }
                                let symmetric = |column: &str| ErrorRef::Symmetric {
                                    column: column.into(),
                                };
                                let errors = (errors_kind != 0).then(|| ErrorBars {
                                    x: (errors_kind == 2).then(|| symmetric("ex")),
                                    y: (errors_kind == 1).then(|| symmetric("ey")),
                                    style: DataErrorBarStyleConfig {
                                        error_bar_color: Color::BLACK,
                                        error_bar_width: 1.0,
                                        error_bar_cap_size: 4.0,
                                        cap_width: 1.0,
                                        error_bar_style_table: None,
                                        error_bar_style_index_column: None,
                                        error_bar_style_overrides: None,
                                    },
                                });
                                let stem = stem.then(|| Stem {
                                    baseline: 0.0,
                                    direction: CategoryBarDirection::Vertical,
                                    color: Color::BLACK,
                                    width: 1.0,
                                    line_style: LineStylePreset::Solid,
                                });
                                let mut list = vec![series(
                                    "subject",
                                    "x",
                                    "y",
                                    DataRenderType::Xy(XySeries {
                                        scatter,
                                        line: line_config,
                                        errors,
                                        stem: stem.clone(),
                                    }),
                                )];
                                if fill_kind == 2 {
                                    list.push(series(
                                        "base",
                                        "x",
                                        "y2",
                                        DataRenderType::Xy(XySeries {
                                            scatter: None,
                                            line: Some(line_style(LineStylePreset::Solid, 1.0)),
                                            errors: None,
                                            stem: None,
                                        }),
                                    ));
                                }
                                let line_name = line.map_or("line=none".to_string(), |preset| {
                                    format!("line={preset:?}")
                                });
                                out.push(Case {
                                    family: "xy",
                                    options: format!(
                                        "{coords} {draw_name} {} {line_name} {errors_name} stem={} {fill_name}",
                                        marker.label(),
                                        if stem.is_some() { "yes" } else { "no" },
                                    ),
                                    config: chart_config(polar, &draw),
                                    series: list,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A radar polygon's marker, fill and outline under every draw style.
fn radar_cases(out: &mut Vec<Case>) {
    use crate::RadarMarker;
    for (draw_name, draw) in draw_styles() {
        for (marker_name, marker) in [
            ("none", RadarMarker::None),
            ("circle", RadarMarker::Circle),
            ("square", RadarMarker::Square),
        ] {
            for (fill_name, fill) in [
                ("default", None),
                ("transparent", Some(Color::new(0.0, 0.0, 0.0, 0.0))),
            ] {
                for width in [2.0f32, 0.0] {
                    let mut config = chart_config(true, &draw);
                    config.bottom_x.min = 0.0;
                    config.bottom_x.max = 4.0;
                    config.bottom_x.categories = Some(
                        (0..5)
                            .map(|i| crate::config::Category::new(format!("c{i}"), format!("C{i}")))
                            .collect(),
                    );
                    let style = crate::RadarSeriesStyle {
                        line_width_px: Some(width),
                        fill,
                        marker: Some(marker),
                        marker_size_px: Some(6.0),
                        fill_opacity: None,
                    };
                    out.push(Case {
                        family: "radar",
                        options: format!(
                            "{draw_name} marker={marker_name} fill={fill_name} width={width}"
                        ),
                        config,
                        series: vec![series(
                            "subject",
                            "cat",
                            "val",
                            DataRenderType::Radar(RadarPolygonSeries {
                                color: Color::BLACK,
                                style,
                                labels: Default::default(),
                            }),
                        )],
                    });
                }
            }
        }
    }
}

/// Ternary markers, line and region under every draw style.
fn ternary_cases(out: &mut Vec<Case>) {
    for (draw_name, draw) in draw_styles() {
        for marker in Marker::all() {
            let mut lines: Vec<Option<LineStylePreset>> = vec![None];
            lines.extend(PRESETS.iter().copied().map(Some));
            for line in lines {
                for region in [false, true] {
                    if matches!(marker, Marker::None) && line.is_none() && !region {
                        continue;
                    }
                    let mut config = chart_config(false, &draw);
                    for axis in [
                        &mut config.bottom_x,
                        &mut config.left_y,
                        &mut config.right_y,
                    ] {
                        axis.min = 0.0;
                        axis.max = 100.0;
                    }
                    config.set_ternary(Some(crate::TernaryOptions {}));
                    let line_name =
                        line.map_or("line=none".to_string(), |preset| format!("line={preset:?}"));
                    out.push(Case {
                        family: "ternary",
                        options: format!(
                            "{draw_name} {} {line_name} region={}",
                            marker.label(),
                            if region { "yes" } else { "no" }
                        ),
                        config,
                        series: vec![series(
                            "subject",
                            "a",
                            "b",
                            DataRenderType::Ternary(TernarySeries {
                                c_column: "c".into(),
                                scatter: marker.scatter(),
                                line: line.map(|preset| line_style(preset, 2.0)),
                                region: region.then_some(crate::TernaryRegionStyle {
                                    color: Color::BLACK,
                                    opacity: 0.5,
                                }),
                            }),
                        )],
                    });
                }
            }
        }
    }
}

/// Streamlines with and without arrowheads, every dash, cartesian and
/// polar, under every draw style.
fn streamline_cases(out: &mut Vec<Case>) {
    for (coords, polar) in coordinates() {
        for (draw_name, draw) in draw_styles() {
            for preset in PRESETS {
                for arrows in [false, true] {
                    out.push(Case {
                        family: "streamlines",
                        options: format!(
                            "{coords} {draw_name} line={preset:?} arrows={}",
                            if arrows { "yes" } else { "no" }
                        ),
                        config: chart_config(polar, &draw),
                        series: vec![series(
                            "subject",
                            "x",
                            "y",
                            DataRenderType::Streamlines(StreamlineSeries {
                                line: line_style(preset, 2.0),
                                arrows: arrows.then_some(crate::StreamlineArrows {
                                    spacing_px: 24.0,
                                    head_length_px: 9.0,
                                    head_width_px: 10.0,
                                }),
                                color_map: None,
                            }),
                        )],
                    });
                }
            }
        }
    }
}

/// Point vectors, cartesian and polar, under every draw style.
fn vector_cases(out: &mut Vec<Case>) {
    for (coords, polar) in coordinates() {
        for (draw_name, draw) in draw_styles() {
            out.push(Case {
                family: "vector",
                options: format!("{coords} {draw_name}"),
                config: chart_config(polar, &draw),
                series: vec![series(
                    "subject",
                    "x",
                    "y",
                    DataRenderType::Vector(VectorSeries {
                        components: VectorComponents::Points {
                            u: "u".into(),
                            v: "v".into(),
                            w: None,
                        },
                        style: VectorStyle {
                            color: Color::BLACK,
                            width_px: 2.0,
                            scale: VectorScale::Pixels { px_per_unit: 4.0 },
                            pivot: VectorPivot::Tail,
                            head_length_px: 6.0,
                            head_width_px: 6.0,
                            min_length_px: 0.0,
                            color_map: None,
                            auto_scale: false,
                        },
                    }),
                )],
            });
        }
    }
}

/// A box plot's raw points: every point shape, both directions, under
/// every draw style.
fn box_point_cases(out: &mut Vec<Case>) {
    for (draw_name, draw) in draw_styles() {
        for direction in [
            CategoryBarDirection::Vertical,
            CategoryBarDirection::Horizontal,
        ] {
            for shape in [BoxOutlierShape::Circle, BoxOutlierShape::Square] {
                for filled in [false, true] {
                    let mut config = chart_config(false, &draw);
                    let categories = Some(vec![
                        crate::config::Category::new("a", "A"),
                        crate::config::Category::new("b", "B"),
                        crate::config::Category::new("c", "C"),
                    ]);
                    match direction {
                        CategoryBarDirection::Vertical => {
                            config.bottom_x.categories = categories;
                            config.left_y.min = 0.0;
                            config.left_y.max = 100.0;
                        }
                        CategoryBarDirection::Horizontal => {
                            config.left_y.categories = categories;
                            config.bottom_x.min = 0.0;
                            config.bottom_x.max = 100.0;
                        }
                    }
                    config.bar_groups = Some(BarGroups {
                        direction,
                        ..BarGroups::default()
                    });
                    out.push(Case {
                        family: "box points",
                        options: format!(
                            "{draw_name} {direction:?} shape={shape:?} filled={filled}"
                        ),
                        config,
                        series: vec![series(
                            "subject",
                            "pos",
                            "box.record",
                            DataRenderType::BoxPlot(BoxSeries {
                                color: Color::BLACK,
                                style: BoxPlotStyle::default(),
                                labels: BoxPlotSeriesLabels::default(),
                                outliers: None,
                                overrides: None,
                                violin: None,
                                points: Some(BoxPoints {
                                    positions: "pt.cat".into(),
                                    values: "pt.value".into(),
                                    style: BoxPointsStyle {
                                        color: Color::BLACK,
                                        size: 6.0,
                                        shape,
                                        filled,
                                        jitter: 0.3,
                                        offset: 0.0,
                                    },
                                }),
                            }),
                        )],
                    });
                }
            }
        }
    }
}

/// Every column the cases read.
fn add_columns(renderer: &mut Renderer) {
    let column = |data: Vec<f64>| Column { data };
    for (id, data) in [
        ("x", vec![1.0, 3.0, 5.0, 7.0, 9.0]),
        ("y", vec![2.0, 6.0, 4.0, 8.0, 5.0]),
        ("y2", vec![1.0, 2.0, 1.0, 3.0, 2.0]),
        ("ex", vec![0.5; 5]),
        ("ey", vec![0.5; 5]),
        ("idx", vec![0.0, 1.0, 0.0, 1.0, 0.0]),
        ("u", vec![1.0, 0.5, -0.5, 0.2, 1.0]),
        ("v", vec![0.5, 1.0, 0.5, -0.2, 0.3]),
        ("a", vec![20.0, 30.0, 50.0, 10.0, 40.0]),
        ("b", vec![30.0, 50.0, 20.0, 60.0, 30.0]),
        ("c", vec![50.0, 20.0, 30.0, 30.0, 30.0]),
        ("cat", vec![0.0, 1.0, 2.0, 3.0, 4.0]),
        ("val", vec![5.0, 8.0, 3.0, 9.0, 6.0]),
        ("pos", vec![0.0, 1.0, 2.0]),
        ("pt.cat", vec![0.0, 0.0, 1.0, 1.0, 2.0, 2.0]),
        ("pt.value", vec![30.0, 60.0, 40.0, 70.0, 50.0, 55.0]),
    ] {
        renderer.add_column(id, &column(data)).unwrap();
    }
    let field = |value: f64| column(vec![value; 3]);
    let fields = [
        field(50.0),
        field(40.0),
        field(60.0),
        field(20.0),
        field(80.0),
    ];
    let names = ["median", "q1", "q3", "whisker_low", "whisker_high"];
    let sources: Vec<(&str, &dyn ColumnSource)> = names
        .iter()
        .zip(&fields)
        .map(|(name, column)| (*name, column as &dyn ColumnSource))
        .collect();
    renderer
        .upsert_record(
            "box.record",
            &crate::RecordSource {
                schema: data_render::BOX_PLOT_RECORD,
                fields: &sources,
            },
        )
        .unwrap();
}

/// A table row's identity: the module, the shader set, the bits and the
/// shape, in the table's order.
type RowKey = (u8, u8, u32, u32);

#[derive(Default)]
struct RowFacts {
    families: BTreeSet<String>,
    example: Option<String>,
    cases: usize,
    constants: String,
}

fn bit_names(bits: u32) -> String {
    let names = [
        (exact_variant::STYLED_MARKER, "STYLED_MARKER"),
        (exact_variant::ERRORBAR, "ERRORBAR"),
        (exact_variant::AREA, "AREA"),
        (exact_variant::DASH, "DASH"),
        (exact_variant::STYLED_LINE, "STYLED_LINE"),
        (exact_variant::POLAR, "POLAR"),
        (exact_variant::PATH_ARROWS, "PATH_ARROWS"),
        (exact_variant::CATEGORY_JITTER, "CATEGORY_JITTER"),
        (exact_variant::TERNARY, "TERNARY"),
    ];
    let known = names.iter().fold(0, |all, (bit, _)| all | bit);
    assert_eq!(bits & !known, 0, "unnamed PICK_VARIANT bits in {bits}");
    let listed: Vec<&str> = names
        .iter()
        .filter(|(bit, _)| bits & bit != 0)
        .map(|(_, name)| *name)
        .collect();
    if listed.is_empty() {
        "none".into()
    } else {
        listed.join("|")
    }
}

fn shape_name(shape: u32) -> String {
    if shape == PickExactKey::ANY_SHAPE {
        return "any".into();
    }
    SHAPES
        .iter()
        .find(|s| data_render::shape_id(s) == shape)
        .map(|s| format!("{s:?}"))
        .unwrap_or_else(|| panic!("no marker shape has id {shape}"))
}

/// The table's rows: every key the cases reach, with what produced it.
fn reachable_rows(renderer: &mut Renderer) -> (BTreeMap<RowKey, RowFacts>, usize, usize) {
    let mut cases = Vec::new();
    xy_cases(&mut cases);
    radar_cases(&mut cases);
    ternary_cases(&mut cases);
    streamline_cases(&mut cases);
    vector_cases(&mut cases);
    box_point_cases(&mut cases);
    let total = cases.len();
    let mut rows: BTreeMap<RowKey, RowFacts> = BTreeMap::new();
    let mut registered = 0;
    let mut failures = Vec::new();
    for case in cases {
        let Ok(id) = renderer.register_chart(case.config.clone(), case.series.clone()) else {
            // A combination the model refuses never reaches the picker.
            continue;
        };
        registered += 1;
        let area = case.config.data_area().unwrap().0;
        let request = GpuPickRequest {
            canvas_position_px: [
                area.x as f32 + area.width as f32 * 0.5,
                area.y as f32 + area.height as f32 * 0.5,
            ],
            display_panel_px: case.config.chart_area.0,
            display_scale: 1.0,
            max_distance_px: 4.0,
        };
        match renderer.exact_pick_query(id, request) {
            Ok(Some(query)) => {
                let streams = case
                    .series
                    .iter()
                    .all(|s| PrepareContext::validate_stream_series(&case.config, s).is_ok());
                let active = renderer.picker.active.as_ref().unwrap();
                let mut keys: Vec<(PickExactKey, bool)> = active
                    .engine
                    .exact_variants(&query)
                    .map(|key| (key, false))
                    .collect();
                if streams {
                    keys.extend(
                        active
                            .engine
                            .stream_exact_variants(&query)
                            .map(|k| (k, true)),
                    );
                }
                // What a browser renderer compiles ahead of the first pick
                // (`for_each_exact_pick_key`, #363) is exactly what the pick
                // needs.
                let needed: BTreeSet<(u8, u32, u32)> = keys
                    .iter()
                    .filter(|(_, streamed)| !streamed)
                    .map(|(k, _)| (k.shader as u8, k.bits, k.shape))
                    .collect();
                let mut warmed = BTreeSet::new();
                renderer
                    .for_each_exact_pick_key(id, |k| {
                        warmed.insert((k.shader as u8, k.bits, k.shape));
                    })
                    .unwrap();
                if warmed != needed {
                    failures.push(format!(
                        "{} {}: warm-up keys {warmed:?} differ from the pick's {needed:?}",
                        case.family, case.options
                    ));
                }
                let needs_full = config_needs_full_shaders(&case.config);
                for (key, streamed) in keys {
                    let (constants, count) = key.constants(renderer.device());
                    let constants = constants[..count]
                        .iter()
                        .filter(|(name, _)| *name != NATIVE_ONLY_CONSTANT)
                        .map(|(name, value)| format!("{name}={value}"))
                        .collect::<Vec<_>>()
                        .join(";");
                    let family = if streamed {
                        format!("{} (streamed)", case.family)
                    } else {
                        case.family.to_string()
                    };
                    let sets: &[ShaderSet] = if needs_full {
                        &[ShaderSet::Full]
                    } else {
                        &[ShaderSet::Basic, ShaderSet::Full]
                    };
                    for &set in sets {
                        let row = rows
                            .entry((key.shader as u8, set as u8, key.bits, key.shape))
                            .or_default();
                        row.families.insert(family.clone());
                        let example = format!("{family}: {}", case.options);
                        if row.example.as_ref().is_none_or(|e| example < *e) {
                            row.example = Some(example);
                        }
                        row.cases += 1;
                        row.constants = constants.clone();
                    }
                }
            }
            Ok(None) => failures.push(format!("{}: no query", case.options)),
            Err(error) => failures.push(format!("{} {}: {error:?}", case.family, case.options)),
        }
        renderer.remove_chart(id).unwrap();
    }
    assert!(
        failures.is_empty(),
        "{} registered charts did not prepare a pick or warm up its keys, first {:?}",
        failures.len(),
        &failures[..failures.len().min(8)]
    );
    (rows, total, registered)
}

/// A parsed table: its header lines' values and each row's measured time
/// and budget by key.
struct Table {
    /// Each module's hash, by name (`marker-line/full`).
    module_hashes: BTreeMap<String, String>,
    rows: BTreeMap<String, (String, String)>,
    body: Vec<String>,
}

fn row_identity(fields: &[&str]) -> String {
    // module, set, bits, shape
    format!("{},{},{},{}", fields[0], fields[1], fields[2], fields[4])
}

fn parse(text: &str) -> Table {
    let mut table = Table {
        module_hashes: BTreeMap::new(),
        rows: BTreeMap::new(),
        body: Vec::new(),
    };
    for line in text.lines() {
        if let Some((name, hash)) = line
            .strip_prefix("# module_hash=")
            .and_then(|value| value.split_once('='))
        {
            table.module_hashes.insert(name.into(), hash.into());
        } else if line.starts_with('#') || line == HEADER || line.is_empty() {
        } else {
            let fields: Vec<&str> = line.split(',').collect();
            if fields.len() == 11 {
                table.rows.insert(
                    row_identity(&fields),
                    (fields[9].to_string(), fields[10].to_string()),
                );
                table.body.push(fields[..9].join(","));
            }
        }
    }
    table
}

fn render(
    rows: &BTreeMap<RowKey, RowFacts>,
    modules: &[ModuleRecord],
    times: impl Fn(&str) -> (String, String),
) -> String {
    let mut out = String::new();
    out.push_str(
        "# figgy exact pick variant table (#363). Generated: never edit the rows by hand.\n\
# One row per exact pick pipeline a chart can reach (module, shader set, PICK_VARIANT bits,\n\
# marker shape), from `renderer/picking/variant_table_tests.rs`, with the series families\n\
# that produce it, the first producing case in order, the number of producing cases, and\n\
# the pipeline's compile time in headless Chromium with SwiftShader (createComputePipelineAsync,\n\
# a fresh device per row, the capture's descriptor and layout).\n",
    );
    let _ = writeln!(out, "# Regenerate the rows and hashes: {UPDATE_COMMAND}");
    let _ = writeln!(
        out,
        "# Then measure the rows without a time: {MEASURE_COMMAND} --missing"
    );
    out.push_str(
        "# Budget rule: budget_ms = max(3 x measured_ms, 2000), rounded up to the next 100 ms.\n\
# The runner fails a row whose compile exceeds its budget.\n",
    );
    out.push_str(
        "# Each module's hash covers its assembled source; a change re-measures that module's\n\
# rows only (the update keeps the other rows' times; then measure with --missing).\n",
    );
    for module in modules {
        let _ = writeln!(out, "# module_hash={}={}", module.name, module.hash);
        let _ = writeln!(
            out,
            "# module_fragments={}={}",
            module.name,
            module.fragments.join(";")
        );
    }
    out.push_str(HEADER);
    out.push('\n');
    for ((shader, set, bits, shape), facts) in rows {
        let shader = PickShader::EXACT
            .into_iter()
            .find(|s| *s as u8 == *shader)
            .unwrap();
        let set = if *set == ShaderSet::Basic as u8 {
            ShaderSet::Basic
        } else {
            ShaderSet::Full
        };
        let families = facts
            .families
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        let example = facts.example.clone().unwrap_or_default();
        for field in [&families, &example, &facts.constants] {
            assert!(!field.contains(',') && !field.contains('"'), "{field}");
        }
        let identity = format!(
            "{},{},{},{}",
            module_name(shader),
            set_name(set),
            bits,
            shape_name(*shape)
        );
        let (measured, budget) = times(&identity);
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{},{},{},{}",
            module_name(shader),
            set_name(set),
            bits,
            bit_names(*bits),
            shape_name(*shape),
            facts.constants,
            families,
            example,
            facts.cases,
            measured,
            budget
        );
    }
    out
}

/// The table holds exactly the keys the picker reaches, records the hash of
/// each exact module its rows were measured on, and every row has a
/// measured time and budget.
#[test]
fn pick_variant_table_matches_the_picker() {
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut renderer = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        1024 * 1024,
    )
    .expect("required GPU test: renderer initialization failed");
    // Every chart prepares on the full set; a chart that does not need it
    // is listed for the basic set too.
    renderer.admit_full_shaders(true);
    renderer.enable_gpu_picking().unwrap();
    add_columns(&mut renderer);
    let started = std::time::Instant::now();
    let (rows, total, registered) = reachable_rows(&mut renderer);
    eprintln!(
        "pick variant table: {total} cases, {registered} registered, {} rows, {:?}",
        rows.len(),
        started.elapsed()
    );
    let modules = module_records();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(TABLE);
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    let old = parse(&current);
    // A row's module, `marker-line/full`, from its identity.
    let module_of = |identity: &str| {
        let mut fields = identity.split(',');
        format!(
            "{}/{}",
            fields.next().unwrap_or_default(),
            fields.next().unwrap_or_default()
        )
    };
    let unchanged = |name: &str| {
        modules
            .iter()
            .find(|module| module.name == name)
            .is_some_and(|module| old.module_hashes.get(name) == Some(&module.hash))
    };
    // A row keeps its measurement while its module is unchanged.
    let kept = |identity: &str| {
        old.rows
            .get(identity)
            .filter(|_| unchanged(&module_of(identity)))
            .cloned()
            .unwrap_or_default()
    };
    let expected = render(&rows, &modules, kept);
    if std::env::var(UPDATE).as_deref() == Ok("1") {
        std::fs::write(&path, &expected).unwrap();
        return;
    }
    let changed: Vec<String> = modules
        .iter()
        .filter(|module| !unchanged(&module.name))
        .map(|module| {
            let rows = old
                .rows
                .keys()
                .filter(|identity| module_of(identity) == module.name)
                .count();
            format!("{} ({rows} rows)", module.name)
        })
        .collect();
    assert!(
        changed.is_empty(),
        "the exact pick modules {} changed since {TABLE} was measured: regenerate it with \
`{UPDATE_COMMAND}`, which keeps every other row's time, and measure those rows with \
`{MEASURE_COMMAND} --missing`",
        changed.join(", ")
    );
    let new = parse(&expected);
    let added: Vec<_> = new
        .rows
        .keys()
        .filter(|k| !old.rows.contains_key(*k))
        .collect();
    let removed: Vec<_> = old
        .rows
        .keys()
        .filter(|k| !new.rows.contains_key(*k))
        .collect();
    assert!(
        new.body == old.body,
        "the reachable exact pick variants differ from {TABLE} ({} added, {} removed; first added \
{:?}, first removed {:?}): regenerate it with `{UPDATE_COMMAND}` and measure the new rows with \
`{MEASURE_COMMAND}`",
        added.len(),
        removed.len(),
        added.first(),
        removed.first()
    );
    let unmeasured: Vec<_> = old
        .rows
        .iter()
        .filter(|(_, (measured, budget))| measured.is_empty() || budget.is_empty())
        .map(|(k, _)| k)
        .collect();
    assert!(
        unmeasured.is_empty(),
        "{} rows of {TABLE} have no measured time or budget, first {:?}: measure them with \
`{MEASURE_COMMAND}`",
        unmeasured.len(),
        unmeasured.first()
    );
}

/// #363: the keys a browser renderer compiles ahead of the first pick
/// ([`Renderer::for_each_exact_pick_key`]) are all the pick needs. Once
/// they are created, as a finished warm-up leaves them, the first pick
/// creates no exact pipeline; an edit that changes a series' pick key names
/// a key not created yet, and after that warm-up the next pick creates none
/// either.
#[test]
fn a_warmed_chart_picks_without_compiling_and_an_edit_warms_again() {
    use crate::gpu_compile::capture;
    let session = capture::Session::begin();
    let Some((device, queue)) = crate::data_render::shared_device() else {
        return;
    };
    let mut renderer = Renderer::try_new(
        RendererDevice::new(device, queue).expect("figgy poll thread"),
        wgpu::TextureFormat::Rgba8Unorm,
        1024 * 1024,
    )
    .expect("required GPU test: renderer initialization failed");
    renderer.enable_gpu_picking().unwrap();
    add_columns(&mut renderer);
    let exact_created = || {
        capture::snapshot_pipelines()
            .iter()
            .filter(|p| p["descriptor"]["label"] == "figgy GPU pick exact candidate pipeline")
            .count()
    };
    let warm_up = |renderer: &mut Renderer, id: ChartId| {
        let mut keys = Vec::new();
        renderer
            .for_each_exact_pick_key(id, |key| keys.push(key))
            .unwrap();
        let bundle = renderer.picker.ready_bundle().unwrap();
        let missing = keys
            .iter()
            .filter(|key| renderer.picker.exact.get(**key).is_none())
            .count();
        for key in keys {
            renderer.picker.exact.ensure(&bundle, key).unwrap();
        }
        missing
    };
    let pick = |renderer: &mut Renderer, config: &Config, id: ChartId| {
        let area = config.data_area().unwrap().0;
        let request = GpuPickRequest {
            canvas_position_px: [
                area.x as f32 + area.width as f32 * 0.5,
                area.y as f32 + area.height as f32 * 0.5,
            ],
            display_panel_px: config.chart_area.0,
            display_scale: 1.0,
            max_distance_px: 4.0,
        };
        crate::test_gpu::block_on(renderer.pick_chart(id, request).unwrap().resolve()).unwrap();
    };
    // A radar chart, and an xy chart of markers and a line.
    let mut radar = Vec::new();
    radar_cases(&mut radar);
    let radar = radar
        .into_iter()
        .find(|case| case.options == "precise marker=circle fill=default width=2")
        .unwrap();
    let xy_config = chart_config(false, &DrawStyle::Precise);
    let xy_series = |preset, fill| {
        let mut line = line_style(preset, 2.0);
        line.fill = fill;
        vec![series(
            "subject",
            "x",
            "y",
            DataRenderType::Xy(XySeries {
                scatter: Marker::Shape(ScatterShape::CircleFilled).scatter(),
                line: Some(line),
                errors: None,
                stem: None,
            }),
        )]
    };
    for (config, series) in [
        (radar.config, radar.series),
        (xy_config.clone(), xy_series(LineStylePreset::Solid, None)),
    ] {
        let id = renderer.register_chart(config.clone(), series).unwrap();
        assert!(warm_up(&mut renderer, id) > 0, "a new chart's keys are new");
        let before = exact_created();
        pick(&mut renderer, &config, id);
        assert_eq!(
            exact_created(),
            before,
            "the warmed first pick compiles nothing"
        );
        assert_eq!(
            warm_up(&mut renderer, id),
            0,
            "an unchanged chart warms nothing new"
        );
        if config == xy_config {
            // A dashed, filled line needs another variant: the edit warms it.
            renderer
                .set_chart_series(
                    id,
                    xy_series(LineStylePreset::Dash, Some(LineFill::ToBaseline(0.0))),
                )
                .unwrap();
            assert!(warm_up(&mut renderer, id) > 0, "the edit names a new key");
            let before = exact_created();
            pick(&mut renderer, &config, id);
            assert_eq!(
                exact_created(),
                before,
                "the re-warmed pick compiles nothing"
            );
            // A draw style edit moves the query's bits: warmed again.
            let mut sketch = config.clone();
            sketch.draw_style = DrawStyle::Sketch(SketchOptions::default());
            renderer.set_chart_config(id, sketch.clone()).unwrap();
            assert!(
                warm_up(&mut renderer, id) > 0,
                "the style edit names a new key"
            );
            let before = exact_created();
            pick(&mut renderer, &sketch, id);
            assert_eq!(
                exact_created(),
                before,
                "the re-warmed pick compiles nothing"
            );
        }
    }
    drop(session.finish());
}
