//! The decoration oracle (#47 section 4): every scene of the playground's
//! catalogue (`support/playground_scenes.rs`), every layer it has, every
//! decoration element of that layer, at 1x and 2x, checked against what the
//! renderer draws for it.
//!
//! One sweep builds every scene once and collects a [`Report`] of plain
//! data; one thin test per invariant asserts that its rows are empty after
//! the [`EXCLUSIONS`] and prints them, so a run yields the whole divergence
//! table instead of the first failed assertion. An exclusion that no longer
//! matches a row fails its invariant's test too.
//!
//! The invariants:
//!
//! - **I1** every drawn part lies inside its panel's chart area (a subplot's
//!   parts also outside every other chart area); bounded charts: every
//!   element's bounds inside the canvas.
//! - **I2** every element's hit region is its drawn outline: every strong
//!   ink pixel inside the region the selection outlines, the region near
//!   the ink, `contains` agreeing with it, and the topmost region at every
//!   ink pixel belonging to an element with ink there.
//! - **I3** the drawn highlight is the model's selection box: the pixels it
//!   changes lie on the box's outline and handles, and every edge is drawn.
//! - **I4** no two decorations that must stay apart overlap: the pair
//!   table of [`overlap_pairs`] on drawn part extents, with 1 px of
//!   tolerance.
//! - **I5** 2x is 1x scaled: every element's bounds at 2x halved within
//!   1 px of its 1x bounds, the same tick labels drawn, and each element's
//!   2x ink box-filtered to 1x matching its 1x ink up to hinting.
//! - **I6** a drag keeps every element grabbable: every draggable element
//!   of every layer moves by (+7, +5) (an axis across its direction only),
//!   I2 holds on its new ink, the hit test on the moved ink names it, and a
//!   drag out of the chart is rejected without a change.
//! - **I7** the hit map of every layer holds exactly the elements the
//!   layer's documented registration set names, in that order; no ink
//!   without an element, no element without ink; and the CPU decoration
//!   raster is what the product exports.
//!
//! The annotation layer runs I1, I2, I3, I6 and I7 on its
//! renderer-measured `AnnotationMap`, with each item's ink measured on the
//! product's exports, and its highlight painted through the chart view as
//! on screen.
//!
//! The three bounded families (radial, box plot, radar) run the
//! same invariants on their hit maps (the renderer-measured `DecorationMap`
//! of the box plot and radar frames; the radial frame's
//! `HitMap<RadialChart>`, whose elements place their parts from the chart,
//! an outside label's leader line among them), with each element's ink
//! measured by drag differential and the highlight through
//! `bounded_selection::assert_selection_only`.
//!
//! The Cartesian decoration ink needs no GPU: the CPU raster of each
//! expanded panel (`try_raster_chart_layer_to_rgba` with
//! `AxisLayerKind::Decoration`) runs the placement pass the renderer
//! uploads, and a 2x display or export is `Config::scaled(2.0)`. The GPU
//! builds the scenes (column uploads, `auto_fit_*`) and renders the bounded
//! families and the annotation layer.
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/bounded_decorations.rs"]
mod bounded_decorations;
#[allow(dead_code)]
#[path = "support/bounded_selection.rs"]
mod bounded_selection;
use bounded_decorations::Decorations;
#[path = "support/edge_band.rs"]
mod edge_band;
#[path = "support/playground_scenes.rs"]
mod scenes;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use renderer::axis_render::{
    AxisLayerKind, try_raster_chart_layer_to_rgba, try_raster_chart_layer_to_rgba_with_selection,
};
use renderer::drag::DragConfig;
use renderer::layout::{Element, NudgeResult, RectF, Side};
use renderer::legend::Legend;
use renderer::select::{
    PartOutline, SELECTION_COLOR, SELECTION_PADDING, part_outline, segment_grab_half_width,
};
use renderer::sink::Parts;
use renderer::text::{MeasureText, RichText};
use renderer::{
    BoxPlotChart, BoxPlotFrame, BoxPlotRenderer, Chart, Color, ColorMap, Config, CpuTextMeasure,
    DecorationId, DecorationLayout, DrawnPart, HitId, HitMap, LayerRef, OverlayLayer, RadarChart,
    RadarFrame, RadarRenderer, RadialChart, RadialFrame, RadialRenderer, RasterImage, Renderer,
    RendererDevice, ResizeHandle, SelectionBox, SelectionGeometry, SelectionOutline, SeriesConfig,
};

use scenes::SceneKind;

/// Points of the large-data scene: enough for its layout (the axis ranges
/// come from the generated statistics), small enough for an unoptimized
/// build.
const BIG_DATA_POINTS: usize = 100_000;
/// Pool capacity for every Cartesian scene's columns.
const POOL_BYTES: u64 = 64 * 1024 * 1024;
/// The two output scales: a 1x display and a 2x display or export.
const SCALES: [f32; 2] = [1.0, 2.0];
/// Slack of the containment checks, in chart pixels: float noise only.
const EPS: f32 = 1e-3;

// ---------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------

/// The invariants, in the order of the module documentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Inv {
    I1,
    I2,
    I3,
    I4,
    I5,
    I6,
    I7,
}

impl Inv {
    const ALL: [Inv; 7] = [
        Inv::I1,
        Inv::I2,
        Inv::I3,
        Inv::I4,
        Inv::I5,
        Inv::I6,
        Inv::I7,
    ];

    fn title(self) -> &'static str {
        match self {
            Inv::I1 => "I1 every drawn part inside its panel",
            Inv::I2 => "I2 hit region = drawn outline",
            Inv::I3 => "I3 drawn highlight = model box",
            Inv::I4 => "I4 no overlap",
            Inv::I5 => "I5 2x = 1x scaled",
            Inv::I6 => "I6 a drag keeps every element grabbable",
            Inv::I7 => "I7 hit-map entries = emitted elements",
        }
    }
}

/// One divergence: a check of `inv` that failed for `element` of `layer`
/// in `scene` at `scale`. Repeated failures of one check (many pixels) are
/// one row with a count in `detail`.
#[derive(Clone, Debug)]
struct Row {
    scene: &'static str,
    layer: String,
    element: String,
    inv: Inv,
    scale: f32,
    detail: String,
}

/// A known divergence that is not fixed here: rows of `inv` for `element`
/// of `layer` in `scene` whose detail matches `detail` pass, for the
/// `reason` and the tracking `issue` given. A `*` in a pattern stands for
/// any run of characters.
struct Exclusion {
    scene: &'static str,
    layer: &'static str,
    element: &'static str,
    inv: Inv,
    detail: &'static str,
    reason: &'static str,
    issue: &'static str,
}

impl Exclusion {
    fn matches(&self, row: &Row) -> bool {
        self.inv == row.inv
            && glob(self.scene, row.scene)
            && glob(self.layer, &row.layer)
            && glob(self.element, &row.element)
            && glob(self.detail, &row.detail)
    }
}

/// Whether `text` matches `pattern`, where `*` stands for any run of
/// characters.
fn glob(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((head, rest)) => {
            let Some(mut tail) = text.strip_prefix(head) else {
                return false;
            };
            if rest.is_empty() {
                return true;
            }
            loop {
                if glob(rest, tail) {
                    return true;
                }
                let mut chars = tail.chars();
                if chars.next().is_none() {
                    return false;
                }
                tail = chars.as_str();
            }
        }
    }
}

/// The known divergences: what the first run of the oracle found that is
/// not a small fix (#47). Each needs a policy or type change the
/// maintainer decides on; the hit-region ones are #38's.
const EXCLUSIONS: &[Exclusion] = &[
    Exclusion {
        scene: "*",
        layer: "*",
        element: "*axis_top",
        inv: Inv::I2,
        detail: "*hit another element that has no ink there*axis_*",
        reason: "axis corners: the perpendicular Y axis's box (its line plus the inward tick \
                 band) covers the first tick length of the X axis line, and the later \
                 registration takes those pixels",
        issue: "#38",
    },
    Exclusion {
        scene: "*",
        layer: "*",
        element: "*axis_bottom",
        inv: Inv::I2,
        detail: "*hit another element that has no ink there*axis_*",
        reason: "axis corners, as for the top axis",
        issue: "#38",
    },
    Exclusion {
        scene: "*",
        layer: "*",
        element: "*axis_*",
        inv: Inv::I6,
        detail: "after the drag, I2 of *axis_*: *hit another element that has no ink there*axis_*",
        reason: "axis corners after the axis moved, as in I2",
        issue: "#38",
    },
    Exclusion {
        scene: "*",
        layer: "*",
        element: "colorbar",
        inv: Inv::I2,
        detail: "*hit another element that has no ink there ({\"colorbar_axis\": *",
        reason: "the colourbar axis's box (line plus inward ticks) covers a band of the strip \
                 between its ticks and wins over the strip there",
        issue: "#38",
    },
    Exclusion {
        scene: "*",
        layer: "*",
        element: "colorbar",
        inv: Inv::I6,
        detail: "after the drag, I2 of colorbar: *hit another element*colorbar_axis*",
        reason: "the colourbar axis band over the strip, as in I2",
        issue: "#38",
    },
    Exclusion {
        scene: "TwinAxes",
        layer: "overlay_0",
        element: "legend",
        inv: Inv::I2,
        detail: "*hit another element that has no ink there ({\"overlay_0:axis_right\": *",
        reason: "on an overlay layer the overlay axis registers after the merged legend, so \
                 its tick band takes the legend's pixels near the right edge",
        issue: "#38",
    },
    Exclusion {
        scene: "TwinAxes",
        layer: "overlay_0",
        element: "legend",
        inv: Inv::I6,
        detail: "after the drag, I2 of legend: *overlay_0:axis_right*",
        reason: "the overlay axis band over the legend, as in I2",
        issue: "#38",
    },
    Exclusion {
        scene: "TwinAxes",
        layer: "overlay_0",
        element: "overlay_0:tick_labels_right",
        inv: Inv::I6,
        detail: "after the drag, I2 of overlay_0:tick_labels_right: *overlay_0:axis_title_right*",
        reason: "the (+7, +5) drag moves the labels under the title's padded box; they stay \
                 grabbable elsewhere",
        issue: "#38",
    },
    Exclusion {
        scene: "Subplots",
        layer: "subplot_2/overlay_0",
        element: "subplot_2:overlay_0:tick_labels_right",
        inv: Inv::I2,
        detail: "*subplot_2:overlay_0:axis_title_right*",
        reason: "at 2x the overlay title's padded box reaches over the label column (10 px)",
        issue: "#38",
    },
    Exclusion {
        scene: "Annotations",
        layer: "annotations",
        element: "annotation_1",
        inv: Inv::I2,
        detail: "*stroke outlines are farther than the grab reach from ink*",
        reason: "a dashed reference line is grabbed along its whole length, its gaps included",
        issue: "#47",
    },
    Exclusion {
        scene: "Annotations",
        layer: "annotations",
        element: "annotation_1",
        inv: Inv::I6,
        detail: "after the drag, I2 of annotation_1: *stroke outlines are farther*",
        reason: "the dashed reference line's gaps, as in I2",
        issue: "#47",
    },
    Exclusion {
        scene: "Annotations",
        layer: "annotations",
        element: "annotation_3",
        inv: Inv::I2,
        detail: "4 points of stroke outlines are farther than the grab reach from ink*",
        reason: "the arrow shaft's grab band runs the selection padding past the head's tip",
        issue: "#47",
    },
    Exclusion {
        scene: "Annotations",
        layer: "annotations",
        element: "annotation_3",
        inv: Inv::I6,
        detail: "after the drag, I2 of annotation_3: 4 points of stroke outlines*",
        reason: "the arrow shaft past the tip, as in I2",
        issue: "#47",
    },
    Exclusion {
        scene: "Annotations",
        layer: "annotations",
        element: "annotation_0",
        inv: Inv::I2,
        detail: "4 of * strong ink pixels hit another element that has no ink there ({\"annotation_3\": 4})*",
        reason: "the span's pixels under the arrow shaft's band past the tip",
        issue: "#47",
    },
    Exclusion {
        scene: "Annotations",
        layer: "annotations",
        element: "annotation_0",
        inv: Inv::I6,
        detail: "after the drag, I2 of annotation_0: 4 of * ({\"annotation_3\": 4})*",
        reason: "the span under the arrow shaft past the tip, as in I2",
        issue: "#47",
    },
    Exclusion {
        scene: "Polar",
        layer: "base",
        element: "axis_left",
        inv: Inv::I6,
        detail: "a drag by (10000.0, 10000.0) gave Moved*",
        reason: "the polar ray turns around the centre: every drag is a valid turn",
        issue: "#47",
    },
    Exclusion {
        scene: "Polar",
        layer: "document",
        element: "axis_bottom",
        inv: Inv::I7,
        detail: "*px of orphan ink: the element has no bounds*",
        reason: "the polar outer ring is the data boundary and belongs to no element",
        issue: "#47",
    },
];

/// What the sweep found: the divergences, and the counts and timings the
/// report prints.
#[derive(Default)]
struct Report {
    rows: Vec<Row>,
    /// Checks run and failed per invariant and element kind.
    matrix: BTreeMap<(Inv, String), (usize, usize)>,
    /// Summary lines: scenes, layer cases, element counts and so on.
    summary: Vec<String>,
    /// Seconds per phase, in the order the phases ran.
    phases: Vec<(&'static str, f64)>,
    /// Totals the summary prints: ink pixels, samples, rasters.
    counts: BTreeMap<String, usize>,
    /// While set, I2 checks count as checks of this invariant for this
    /// element: I6 runs I2 on the ink of a dragged element.
    redirect: Option<(Inv, String)>,
}

impl Report {
    /// Record one check: count it, and keep a row when it failed.
    fn check(
        &mut self,
        at: &At<'_>,
        element: &str,
        inv: Inv,
        ok: bool,
        detail: impl FnOnce() -> String,
    ) {
        let (inv, element, detail): (Inv, String, Box<dyn FnOnce() -> String + '_>) =
            match &self.redirect {
                Some((to, dragged)) if inv == Inv::I2 => {
                    let checked = element.to_string();
                    (
                        *to,
                        dragged.clone(),
                        Box::new(move || format!("after the drag, I2 of {checked}: {}", detail())),
                    )
                }
                _ => (inv, element.to_string(), Box::new(detail)),
            };
        let cell = self.matrix.entry((inv, kind_of(&element))).or_default();
        cell.0 += 1;
        if !ok {
            cell.1 += 1;
            self.rows.push(Row {
                scene: at.scene,
                layer: at.layer.to_string(),
                element,
                inv,
                scale: at.scale,
                detail: detail(),
            });
        }
    }

    fn line(&mut self, line: String) {
        self.summary.push(line);
    }

    /// Add `n` to the total `what` of the scene family `at` belongs to.
    fn count(&mut self, at: &At<'_>, what: &str, n: usize) {
        let family = if ["Radial", "BoxPlot", "Radar"].contains(&at.scene) {
            "bounded"
        } else {
            "Cartesian"
        };
        *self
            .counts
            .entry(format!("{family} {what} at {}x", at.scale))
            .or_default() += n;
    }

    /// Add the time since `start` to `phase`.
    fn time(&mut self, phase: &'static str, start: Instant) {
        let seconds = start.elapsed().as_secs_f64();
        match self.phases.iter_mut().find(|(p, _)| *p == phase) {
            Some((_, total)) => *total += seconds,
            None => self.phases.push((phase, seconds)),
        }
    }

    /// Print the rows of `inv` and fail on any that no exclusion covers, or
    /// on an exclusion of `inv` that covers no row.
    fn assert_clean(&self, inv: Inv) {
        let mut out = String::new();
        let mut open = 0;
        let rows: Vec<&Row> = self.rows.iter().filter(|r| r.inv == inv).collect();
        let _ = writeln!(out, "{}: {} rows", inv.title(), rows.len());
        for row in &rows {
            let excluded = EXCLUSIONS.iter().find(|e| e.matches(row));
            if excluded.is_none() {
                open += 1;
            }
            let _ = writeln!(
                out,
                "  [{}] {} / {} / {} @{}x: {}",
                if excluded.is_some() {
                    "excluded"
                } else {
                    "OPEN"
                },
                row.scene,
                row.layer,
                row.element,
                row.scale,
                row.detail
            );
        }
        let stale: Vec<&Exclusion> = EXCLUSIONS
            .iter()
            .filter(|e| e.inv == inv && !self.rows.iter().any(|r| e.matches(r)))
            .collect();
        for e in &stale {
            let _ = writeln!(
                out,
                "  [STALE exclusion] {} / {} / {}: {} ({})",
                e.scene, e.layer, e.element, e.reason, e.issue
            );
        }
        println!("{out}");
        assert!(
            open == 0 && stale.is_empty(),
            "{}: {open} open rows, {} stale exclusions",
            inv.title(),
            stale.len()
        );
    }

    fn print(&self) {
        let mut out = String::new();
        let _ = writeln!(out, "decoration oracle");
        for line in &self.summary {
            let _ = writeln!(out, "  {line}");
        }
        for (what, n) in &self.counts {
            let _ = writeln!(out, "  {what}: {n}");
        }
        let _ = writeln!(out, "invariant x element kind (checks / failed):");
        let kinds: std::collections::BTreeSet<&String> =
            self.matrix.keys().map(|(_, k)| k).collect();
        for kind in kinds {
            let mut cells = String::new();
            for inv in Inv::ALL {
                match self.matrix.get(&(inv, kind.clone())) {
                    Some((n, f)) => {
                        let _ = write!(cells, " {inv:?} {n}/{f}");
                    }
                    None => {
                        let _ = write!(cells, " {inv:?} -");
                    }
                }
            }
            let _ = writeln!(out, "  {kind:<34}{cells}");
        }
        let _ = writeln!(out, "rows per invariant (all / excluded):");
        for inv in Inv::ALL {
            let rows: Vec<&Row> = self.rows.iter().filter(|r| r.inv == inv).collect();
            let excluded = rows
                .iter()
                .filter(|r| EXCLUSIONS.iter().any(|e| e.matches(r)))
                .count();
            let _ = writeln!(out, "  {}: {} / {excluded}", inv.title(), rows.len());
        }
        let _ = writeln!(out, "exclusions: {}", EXCLUSIONS.len());
        for e in EXCLUSIONS {
            let rows = self.rows.iter().filter(|r| e.matches(r)).count();
            let _ = writeln!(
                out,
                "  {:?} {} / {} / {} ({rows} rows): {} ({})",
                e.inv, e.scene, e.layer, e.element, e.reason, e.issue
            );
        }
        let total: f64 = self.phases.iter().map(|(_, s)| s).sum();
        let _ = writeln!(out, "seconds per phase (total {total:.1}):");
        for (phase, seconds) in &self.phases {
            let _ = writeln!(out, "  {phase:<28}{seconds:>8.2}");
        }
        println!("{out}");
    }
}

/// The element kind a matrix cell counts: the element id with the layer
/// prefixes and per-item indices folded, so each kind is one row.
fn kind_of(element: &str) -> String {
    let fold = |part: &str| -> String {
        for prefix in [
            "subplot_",
            "overlay_",
            "annotation_",
            "radial_label_",
            "data_label_",
            "spoke_label_",
        ] {
            if part.starts_with(prefix) {
                return prefix.trim_end_matches('_').to_string();
            }
        }
        part.to_string()
    };
    if let Some((a, b)) = element.split_once(" x ") {
        return format!("{} x {}", kind_of(a), kind_of(b));
    }
    element.split(':').map(fold).collect::<Vec<_>>().join(":")
}

/// Where a check runs: the scene, its layer case and the output scale.
struct At<'a> {
    scene: &'static str,
    layer: &'a str,
    scale: f32,
}

fn report() -> &'static Report {
    static REPORT: OnceLock<Report> = OnceLock::new();
    REPORT.get_or_init(|| {
        let report = sweep();
        report.print();
        report
    })
}

#[test]
fn i1_every_drawn_part_is_inside_its_panel() {
    report().assert_clean(Inv::I1);
}

#[test]
fn i2_every_hit_region_is_the_drawn_outline() {
    report().assert_clean(Inv::I2);
}

#[test]
fn i3_every_drawn_highlight_is_the_model_box() {
    report().assert_clean(Inv::I3);
}

#[test]
fn i4_no_two_decorations_overlap() {
    report().assert_clean(Inv::I4);
}

#[test]
fn i5_2x_is_1x_scaled() {
    report().assert_clean(Inv::I5);
}

#[test]
fn i6_a_drag_keeps_every_element_grabbable() {
    report().assert_clean(Inv::I6);
}

#[test]
fn i7_hit_map_entries_are_the_emitted_elements() {
    report().assert_clean(Inv::I7);
}

// ---------------------------------------------------------------------------
// The sweep
// ---------------------------------------------------------------------------

fn gpu() -> RendererDevice {
    let (device, queue) = test_gpu::device();
    RendererDevice::new(Arc::new(device), Arc::new(queue))
}

fn scene_name(kind: SceneKind) -> &'static str {
    match kind {
        SceneKind::Lines => "Lines",
        SceneKind::Scatter => "Scatter",
        SceneKind::ValueMaps => "ValueMaps",
        SceneKind::Histogram => "Histogram",
        SceneKind::Field => "Field",
        SceneKind::VectorField => "VectorField",
        SceneKind::LogAxes => "LogAxes",
        SceneKind::TwinAxes => "TwinAxes",
        SceneKind::Subplots => "Subplots",
        SceneKind::Annotations => "Annotations",
        SceneKind::Polar => "Polar",
        SceneKind::CategoryAxis => "CategoryAxis",
        SceneKind::BigData => "BigData",
        SceneKind::Radial => "Radial",
        SceneKind::Categorical => "Categorical",
        SceneKind::BoxPlot => "BoxPlot",
        SceneKind::Radar => "Radar",
    }
}

fn sweep() -> Report {
    let mut report = Report::default();
    let start = Instant::now();
    let gpu = gpu();
    let mut r = Renderer::try_new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm, POOL_BYTES)
        .expect("renderer");
    report.time("GPU setup", start);
    let mut cartesian = 0;
    let mut bounded = 0;
    let mut cases = 0;
    for kind in SceneKind::ALL {
        if kind.is_bounded() {
            bounded += 1;
            bounded_scene(&mut report, &gpu, kind);
            continue;
        }
        cartesian += 1;
        let phase = Instant::now();
        let scene = match scenes::build(&mut r, kind, BIG_DATA_POINTS) {
            Ok(scene) => scene,
            Err(e) => {
                let at = At {
                    scene: scene_name(kind),
                    layer: "base",
                    scale: 1.0,
                };
                report.check(&at, "<scene>", Inv::I7, false, || {
                    format!("build failed: {e}")
                });
                continue;
            }
        };
        report.time("Cartesian scene builds", phase);
        cases += cartesian_scene(
            &mut report,
            &mut r,
            scene_name(kind),
            &scene.config,
            &scene.series,
        );
    }
    report.line(format!(
        "{} scenes: {cartesian} Cartesian, {bounded} bounded",
        SceneKind::ALL.len()
    ));
    report.line(format!(
        "Cartesian layer cases: {cases} (x {} scales)",
        SCALES.len()
    ));
    report
}

// ---------------------------------------------------------------------------
// Cartesian scenes
// ---------------------------------------------------------------------------

/// Every layer a document has an interaction case for: the base layer,
/// each overlay, each subplot and its overlays, and the annotation layer
/// when it has items.
fn layer_cases(cfg: &Config) -> Vec<LayerRef> {
    let mut layers = vec![LayerRef::Base];
    layers.extend((0..cfg.overlays.len()).map(LayerRef::Overlay));
    for (index, subplot) in cfg.subplots.iter().enumerate() {
        layers.push(LayerRef::Subplot(index));
        layers
            .extend((0..subplot.config.overlays.len()).map(|j| LayerRef::SubplotOverlay(index, j)));
    }
    if !cfg.annotations.items.is_empty() {
        layers.push(LayerRef::Annotations);
    }
    layers
}

fn layer_name(layer: LayerRef) -> String {
    match layer {
        LayerRef::Base => "base".into(),
        LayerRef::Overlay(i) => format!("overlay_{i}"),
        LayerRef::Subplot(i) => format!("subplot_{i}"),
        LayerRef::SubplotOverlay(i, j) => format!("subplot_{i}/overlay_{j}"),
        LayerRef::Annotations => "annotations".into(),
    }
}

const SIDES: [&str; 4] = ["top", "bottom", "left", "right"];

/// The registration set of a single chart's base layer, back to front, as
/// `HitMap::standard_chart` documents it.
fn standard_names(prefix: &str) -> Vec<String> {
    let mut names = vec![format!("{prefix}data_area")];
    for kind in ["axis_", "tick_labels_", "axis_title_"] {
        names.extend(SIDES.iter().map(|s| format!("{prefix}{kind}{s}")));
    }
    for name in [
        "legend",
        "chart_title",
        "colorbar",
        "colorbar_axis",
        "colorbar_tick_labels",
        "colorbar_title",
    ] {
        names.push(format!("{prefix}{name}"));
    }
    names
}

/// The shared elements a non-base layer registers first, as
/// `HitMap::for_active_layer` documents them: the data area, the X axes
/// with their labels and titles, the legend, the chart title and the
/// colourbar chrome.
fn shared_names(prefix: &str) -> Vec<String> {
    let mut names = vec![format!("{prefix}data_area")];
    for kind in ["axis_", "tick_labels_", "axis_title_"] {
        names.extend(
            ["top", "bottom"]
                .iter()
                .map(|s| format!("{prefix}{kind}{s}")),
        );
    }
    for name in [
        "legend",
        "chart_title",
        "colorbar",
        "colorbar_axis",
        "colorbar_tick_labels",
        "colorbar_title",
    ] {
        names.push(format!("{prefix}{name}"));
    }
    names
}

fn overlay_names(prefix: &str, overlay: usize) -> Vec<String> {
    [
        "axis_right",
        "tick_labels_right",
        "axis_title_right",
        "legend",
    ]
    .iter()
    .map(|n| format!("{prefix}overlay_{overlay}:{n}"))
    .collect()
}

/// The registration set of `layer`, derived from the documented policy of
/// `HitMap::standard_chart` and `HitMap::for_active_layer`, not from the
/// map: the oracle's side of I7.
fn expected_names(layer: LayerRef) -> Vec<String> {
    match layer {
        LayerRef::Base => standard_names(""),
        LayerRef::Annotations => shared_names(""),
        LayerRef::Overlay(i) => {
            let mut names = shared_names("");
            names.extend(overlay_names("", i));
            names
        }
        LayerRef::Subplot(i) => standard_names(&format!("subplot_{i}:")),
        LayerRef::SubplotOverlay(i, j) => {
            let prefix = format!("subplot_{i}:");
            let mut names = shared_names(&prefix);
            names.extend(overlay_names(&prefix, j));
            names
        }
    }
}

/// The registration index of a hit id. `HitId` only exposes it through its
/// `Debug` form.
fn hit_index(id: HitId) -> usize {
    let text = format!("{id:?}");
    text.trim_start_matches("HitId(")
        .trim_end_matches(')')
        .parse()
        .expect("HitId debug form")
}

/// One registered element of a layer case, measured once.
struct Measured {
    name: String,
    id: HitId,
    bounds: Option<RectF>,
    parts: Vec<DrawnPart>,
    selection: Option<SelectionBox>,
}

/// The chart area a registered element is drawn in: its subplot's for a
/// subplot element, the parent's otherwise.
fn panel_area(cfg: &Config, name: &str) -> RectF {
    let area = name
        .strip_prefix("subplot_")
        .and_then(|rest| rest.split(':').next())
        .and_then(|i| i.parse::<usize>().ok())
        .and_then(|i| cfg.subplots.get(i))
        .map_or(cfg.chart_area.0, |s| s.config.chart_area.0);
    RectF::from_rect(&area)
}

/// Every chart area of the document but `own`.
fn other_areas(cfg: &Config, own: RectF) -> Vec<RectF> {
    std::iter::once(RectF::from_rect(&cfg.chart_area.0))
        .chain(
            cfg.subplots
                .iter()
                .map(|s| RectF::from_rect(&s.config.chart_area.0)),
        )
        .filter(|a| *a != own)
        .collect()
}

fn inside(outer: RectF, r: RectF) -> bool {
    r.x >= outer.x - EPS
        && r.y >= outer.y - EPS
        && r.x + r.width <= outer.x + outer.width + EPS
        && r.y + r.height <= outer.y + outer.height + EPS
}

fn intersects(a: RectF, b: RectF, tolerance: f32) -> bool {
    let w = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let h = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    w > tolerance && h > tolerance
}

/// Sweep one Cartesian scene; returns its number of layer cases.
fn cartesian_scene(
    report: &mut Report,
    r: &mut Renderer,
    scene: &'static str,
    config: &Config,
    series: &[SeriesConfig],
) -> usize {
    let layers = layer_cases(config);
    let (palette_config, codes) = palette(config);
    // What I5 compares across the scales.
    let mut runs: Vec<ScaleRun> = Vec::with_capacity(SCALES.len());
    for scale in SCALES {
        let doc = if scale == 1.0 {
            palette_config.clone()
        } else {
            palette_config.scaled(scale)
        };
        let document = At {
            scene,
            layer: "document",
            scale,
        };
        let phase = Instant::now();
        let ink = match SceneInk::new(&doc, &codes) {
            Ok(ink) => ink,
            Err(e) => {
                report.check(&document, "<raster>", Inv::I7, false, || e);
                continue;
            }
        };
        report.time("Cartesian palette rasters", phase);
        let phase = Instant::now();
        check_export(report, &document, r, &palette_config, series, &ink);
        report.time("Cartesian exports", phase);
        // Each element's bounds in the layer cases that register it.
        let mut registered: BTreeMap<String, Option<RectF>> = BTreeMap::new();
        let mut geometry: BTreeMap<(String, String), (Option<RectF>, usize)> = BTreeMap::new();
        // Each element's drawn parts, for the overlap pairs of I4.
        let mut drawn: BTreeMap<String, Vec<DrawnPart>> = BTreeMap::new();
        for &layer in &layers {
            let phase = Instant::now();
            let mut cfg = doc.clone();
            let stored = cfg.set_active_layer(layer);
            let name = layer_name(layer);
            let at = At {
                scene,
                layer: &name,
                scale,
            };
            report.check(&at, "<layer>", Inv::I7, stored == layer, || {
                format!("set_active_layer stored {stored:?}")
            });
            let (map, measured) = measure_layer(report, &at, &cfg, layer);
            check_i1(report, &at, &cfg, &measured);
            let drawn_count = measured.iter().filter(|m| m.bounds.is_some()).count();
            let parts: usize = measured.iter().map(|m| m.parts.len()).sum();
            report.count(&at, "registered elements", measured.len());
            report.count(&at, "drawn elements", drawn_count);
            report.count(&at, "drawn parts", parts);
            if scale == 1.0 {
                report.line(format!(
                    "{scene} / {name}: {} registered, {drawn_count} drawn, {parts} parts",
                    measured.len()
                ));
            }
            for m in &measured {
                registered.insert(m.name.clone(), m.bounds);
                drawn.insert(m.name.clone(), m.parts.clone());
                let texts = m
                    .parts
                    .iter()
                    .filter(|p| matches!(p, DrawnPart::Text(_)))
                    .count();
                geometry.insert((name.clone(), m.name.clone()), (m.bounds, texts));
            }
            report.time("Cartesian geometry", phase);
            let phase = Instant::now();
            let masks: Vec<Mask> = measured.iter().map(|m| Mask::of(m, &ink)).collect();
            check_i2(
                report,
                &at,
                &cfg,
                &CpuTextMeasure::for_style(&cfg.draw_style),
                &map,
                &measured,
                &masks,
                &ink,
                None,
            );
            report.time("Cartesian hit regions (I2)", phase);
            if layer == LayerRef::Annotations {
                let geometry_items =
                    check_annotations(report, &at, r, config, series, &cfg, &measured, &ink);
                for (item, bounds) in geometry_items {
                    geometry.insert((name.clone(), item), (bounds, 0));
                }
            }
            if scale == 1.0 {
                let phase = Instant::now();
                check_i3(report, &at, &measured, &ink);
                report.time("Cartesian highlights (I3)", phase);
                let phase = Instant::now();
                check_i6(report, &at, &cfg, layer, &map, &measured, &ink, &codes);
                report.time("Cartesian drags (I6)", phase);
            }
        }
        check_owners(report, &document, &ink, &registered);
        check_i4(report, &document, &doc, &drawn);
        runs.push(ScaleRun { ink, geometry });
    }
    if let [one, two] = &runs[..] {
        let phase = Instant::now();
        check_i5(report, scene, one, two);
        report.time("Cartesian scale (I5)", phase);
    }
    layers.len()
}

/// Build the layer's hit map, check it against the documented registration
/// set (I7) and measure every element.
fn measure_layer(
    report: &mut Report,
    at: &At<'_>,
    cfg: &Config,
    layer: LayerRef,
) -> (HitMap, Vec<Measured>) {
    let map = HitMap::for_active_layer(cfg);
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let names = expected_names(layer);
    report.check(at, "<map>", Inv::I7, map.len() == names.len(), || {
        format!(
            "map.len() {} but the documented set has {}",
            map.len(),
            names.len()
        )
    });
    let mut measured = Vec::with_capacity(names.len());
    for (order, name) in names.iter().enumerate() {
        let Some(id) = map.find(name) else {
            report.check(at, name, Inv::I7, false, || "not registered".into());
            continue;
        };
        let element = map.get(id).expect("found id");
        let index = hit_index(id);
        report.check(
            at,
            name,
            Inv::I7,
            index == order && element.element_id() == *name,
            || {
                format!(
                    "registered at {index} as {:?}, documented at {order}",
                    element.element_id()
                )
            },
        );
        let mut parts = Vec::new();
        let laid_out = element.emit(cfg, &measure, &mut Parts(&mut |p| parts.push(p)));
        let bounds = element.bounds(cfg, &measure);
        report.check(at, name, Inv::I7, laid_out, || {
            "the renderer's measure cannot lay it out".into()
        });
        let selection = map.selection_box(id, cfg, &measure);
        measured.push(Measured {
            name: name.clone(),
            id,
            bounds,
            parts,
            selection,
        });
    }
    (map, measured)
}

/// I1: every part an element draws lies inside its panel's chart area and,
/// for a document with subplots, outside every other chart area.
fn check_i1(report: &mut Report, at: &At<'_>, cfg: &Config, measured: &[Measured]) {
    for m in measured {
        if m.parts.is_empty() {
            continue;
        }
        let own = panel_area(cfg, &m.name);
        let others = other_areas(cfg, own);
        let outside: Vec<RectF> = m
            .parts
            .iter()
            .map(DrawnPart::extent)
            .filter(|e| !inside(own, *e))
            .collect();
        report.check(at, &m.name, Inv::I1, outside.is_empty(), || {
            format!(
                "{} of {} parts leave the chart area {own:?}, first {:?}",
                outside.len(),
                m.parts.len(),
                outside[0]
            )
        });
        if !others.is_empty() {
            let foreign: Vec<RectF> = m
                .parts
                .iter()
                .map(DrawnPart::extent)
                .filter(|e| others.iter().any(|o| intersects(*o, *e, EPS)))
                .collect();
            report.check(at, &m.name, Inv::I1, foreign.is_empty(), || {
                format!(
                    "{} parts reach into another chart area, first {:?}",
                    foreign.len(),
                    foreign[0]
                )
            });
        }
    }
}

// ---------------------------------------------------------------------------
// The palette raster
// ---------------------------------------------------------------------------
//
// One CPU raster per panel shows every element at once: public SSOT colour
// edits give each element an opaque code colour (its strokes, text, fills
// and the colourbar ramp), which changes no layout, and each pixel's
// un-premultiplied colour names the element that painted it. Overlapping
// ink inside a panel decodes as a blend of two codes; the panels are
// decoded separately, so the layers never blend.

/// Pixels with less alpha are anti-aliasing fringe, too faint to name their
/// owner.
const INK_ALPHA: u8 = 64;
/// Strong ink: the pixels the hit region must contain (#47: "every ink pixel
/// hits", at the half-covered threshold `axis_hit_geometry` uses).
const STRONG_ALPHA: u8 = 128;
/// The largest channel distance between an un-premultiplied pixel and its
/// code colour. Codes are 64 apart; un-premultiplying an 8-bit pixel at
/// alpha 64 is off by at most 2.
const CODE_TOLERANCE: i32 = 24;
/// A box edge may lie this far outside the ink it bounds beyond the padding
/// at 1x (`axis_hit_geometry`'s box rule: anti-aliasing plus glyph side
/// bearings). The bearings grow with the text, so the slack scales with the
/// output.
const BOX_FIT: f32 = 3.5;
/// The topmost region at an ink pixel may belong to another element whose
/// own ink is this close: the selection padding by which every hit region
/// extends past its ink, plus a pixel of anti-aliasing (the "policy plus
/// 1 px" of #47's plan).
const TOPMOST_REACH: i64 = 3;
/// The hit region of a stroke reaches this far past its grab half width
/// (anti-aliasing), as `axis_hit_geometry`'s stroke rule.
const STROKE_FIT: f32 = 1.0;
/// `contains` cross-check samples per element and layer case.
const CONTAINS_SAMPLES: usize = 48;
/// Owner value of a pixel no code explains.
const UNOWNED: u16 = u16::MAX;

/// One element's code colour.
struct Code {
    name: String,
    /// The panel that draws the element.
    panel: LayerRef,
    rgb: [u8; 3],
}

/// Hands out code colours: a lattice of five levels per channel, in a
/// stride order so consecutive codes differ in every channel, without the
/// colours near the selection blue and the white of the resize handles.
struct Painter {
    colors: Vec<[u8; 3]>,
    codes: Vec<Code>,
}

impl Painter {
    fn new() -> Self {
        const LEVELS: [u8; 5] = [0, 64, 128, 192, 255];
        let blue = SELECTION_COLOR;
        let selection = [blue.r, blue.g, blue.b].map(|c| (c * 255.0).round() as i32);
        let far = |rgb: [u8; 3], other: [i32; 3]| {
            (0..3)
                .map(|k| (i32::from(rgb[k]) - other[k]).abs())
                .max()
                .unwrap_or(0)
                >= 48
        };
        let colors = (0..125)
            .map(|i| (i * 47) % 125)
            .map(|k| [LEVELS[k / 25], LEVELS[(k / 5) % 5], LEVELS[k % 5]])
            .filter(|rgb| far(*rgb, selection) && far(*rgb, [255, 255, 255]))
            .collect();
        Self {
            colors,
            codes: Vec::new(),
        }
    }

    fn code(&mut self, name: String, panel: LayerRef) -> Color {
        let rgb = self.colors[self.codes.len()];
        self.codes.push(Code { name, panel, rgb });
        Color::from_rgb8(rgb[0], rgb[1], rgb[2])
    }

    fn text(&mut self, text: &mut RichText, name: String, panel: LayerRef) {
        let c = self.code(name, panel);
        text.color = c;
        for segment in &mut text.segments {
            if segment.color.is_some() {
                segment.color = Some(c);
            }
            if segment.rule_band.is_some() {
                segment.rule_band = Some(c);
            }
        }
    }

    fn legend(&mut self, legend: &mut Legend, name: String, panel: LayerRef) {
        self.text(&mut legend.content, name, panel);
        let c = Color::from_rgb8(
            self.codes.last().map_or(0, |c| c.rgb[0]),
            self.codes.last().map_or(0, |c| c.rgb[1]),
            self.codes.last().map_or(0, |c| c.rgb[2]),
        );
        legend.bg_color = c;
        legend.border_color = c;
    }

    /// The elements of one chart: its four axes with their labels and
    /// titles, the chart title, the legend and the colourbar chrome.
    fn chart(&mut self, cfg: &mut Config, prefix: &str, panel: LayerRef) {
        for (side, axis) in [
            ("top", &mut cfg.top_x),
            ("bottom", &mut cfg.bottom_x),
            ("left", &mut cfg.left_y),
            ("right", &mut cfg.right_y),
        ] {
            axis.line_color = self.code(format!("{prefix}axis_{side}"), panel);
            axis.label_style.color = self.code(format!("{prefix}tick_labels_{side}"), panel);
            self.text(
                &mut axis.title_option.text,
                format!("{prefix}axis_title_{side}"),
                panel,
            );
        }
        self.text(
            &mut cfg.chart_title.text,
            format!("{prefix}chart_title"),
            panel,
        );
        self.legend(&mut cfg.legend, format!("{prefix}legend"), panel);
        if let Some(bar) = cfg.colorbar.as_mut() {
            let c = self.code(format!("{prefix}colorbar"), panel);
            bar.colormap = ColorMap::Custom { stops: vec![c, c] };
            bar.border_color = c;
            bar.axis.line_color = self.code(format!("{prefix}colorbar_axis"), panel);
            bar.axis.label_style.color = self.code(format!("{prefix}colorbar_tick_labels"), panel);
            self.text(
                &mut bar.axis.title_option.text,
                format!("{prefix}colorbar_title"),
                panel,
            );
        }
    }

    fn overlay(&mut self, overlay: &mut OverlayLayer, prefix: &str, panel: LayerRef) {
        let axis = &mut overlay.y_axis;
        axis.line_color = self.code(format!("{prefix}axis_right"), panel);
        axis.label_style.color = self.code(format!("{prefix}tick_labels_right"), panel);
        self.text(
            &mut axis.title_option.text,
            format!("{prefix}axis_title_right"),
            panel,
        );
        if let Some(legend) = overlay.legend.as_mut() {
            self.legend(legend, format!("{prefix}legend"), panel);
        }
    }
}

/// `config` with every decoration element in its own code colour, and the
/// codes. Colours feed no layout, so every hit map, bound and selection box
/// of the palette document is the scene's own.
fn palette(config: &Config) -> (Config, Vec<Code>) {
    let mut painter = Painter::new();
    let mut cfg = config.clone();
    painter.chart(&mut cfg, "", LayerRef::Base);
    for (i, overlay) in cfg.overlays.iter_mut().enumerate() {
        painter.overlay(overlay, &format!("overlay_{i}:"), LayerRef::Overlay(i));
    }
    for (i, subplot) in cfg.subplots.iter_mut().enumerate() {
        let prefix = format!("subplot_{i}:");
        painter.chart(&mut subplot.config, &prefix, LayerRef::Subplot(i));
        for (j, overlay) in subplot.config.overlays.iter_mut().enumerate() {
            painter.overlay(
                overlay,
                &format!("{prefix}overlay_{j}:"),
                LayerRef::SubplotOverlay(i, j),
            );
        }
    }
    (cfg, painter.codes)
}

/// The panels the renderer draws a document as, base first: the document
/// itself without layers, else its expansion, else (no data area at this
/// size) its base panel alone; the renderer's own `expand_for_render`.
fn render_panels(doc: &Config) -> Vec<(LayerRef, Config)> {
    let alone = || {
        let mut panel = doc.clone();
        panel.overlays.clear();
        panel.subplots.clear();
        panel.active_layer = LayerRef::Base;
        vec![(LayerRef::Base, panel)]
    };
    if doc.overlays.is_empty() && doc.subplots.is_empty() {
        return alone();
    }
    match doc.expand() {
        Ok(expansion) => expansion
            .panels
            .into_iter()
            .map(|panel| (panel.layer, panel.config))
            .collect(),
        Err(_) => alone(),
    }
}

/// One panel's decoration raster and the owner of each of its pixels.
#[derive(Clone)]
struct PanelInk {
    layer: LayerRef,
    config: Config,
    /// The panel's chart area in the document image.
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    rgba: Vec<u8>,
    /// Per pixel: 0 no ink, `code + 1`, or [`UNOWNED`].
    owner: Vec<u16>,
    /// The second code of a blend, 0 otherwise.
    owner2: Vec<u16>,
}

/// The decoration ink of one document at one scale.
struct SceneInk {
    /// The document area's origin in the config frame.
    origin: [f32; 2],
    w: usize,
    h: usize,
    codes: Vec<([u8; 3], String, usize)>,
    panels: Vec<PanelInk>,
    /// Every pixel each code has ink in, alone or blended, as document pixel
    /// indices.
    by_code: Vec<Vec<u32>>,
    /// Of those, the pixels the code owns outright.
    exact: Vec<Vec<u32>>,
}

impl SceneInk {
    fn new(doc: &Config, codes: &[Code]) -> Result<Self, String> {
        Self::reusing(doc, codes, None)
    }

    /// The ink of `doc`, reusing every panel of `previous` whose config is
    /// unchanged: a drag re-rasters only the panels it moved.
    fn reusing(doc: &Config, codes: &[Code], previous: Option<&SceneInk>) -> Result<Self, String> {
        let area = doc.document_area();
        let (w, h) = (area.width as usize, area.height as usize);
        let mut panels = Vec::new();
        let mut table: Vec<([u8; 3], String, usize)> = Vec::with_capacity(codes.len());
        let layers: Vec<(LayerRef, Config)> = render_panels(doc);
        for code in codes {
            let panel = layers
                .iter()
                .position(|(l, _)| *l == code.panel)
                .unwrap_or(usize::MAX);
            table.push((code.rgb, code.name.clone(), panel));
        }
        for (index, (layer, config)) in layers.into_iter().enumerate() {
            if let Some(p) = previous
                .and_then(|prev| prev.panels.get(index))
                .filter(|p| p.layer == layer && p.config == config)
            {
                panels.push(p.clone());
                continue;
            }
            let rgba = try_raster_chart_layer_to_rgba(&config, AxisLayerKind::Decoration)
                .map_err(|e| format!("{layer:?} raster: {e}"))?;
            let a = config.chart_area.0;
            let (pw, ph) = (a.width as usize, a.height as usize);
            let local: Vec<usize> = table
                .iter()
                .enumerate()
                .filter(|(_, (_, _, p))| *p == index)
                .map(|(i, _)| i)
                .collect();
            let (owner, owner2) = decode(&rgba, pw, ph, &local, &table);
            panels.push(PanelInk {
                layer,
                x0: (a.x - area.x) as usize,
                y0: (a.y - area.y) as usize,
                w: pw,
                h: ph,
                config,
                rgba,
                owner,
                owner2,
            });
        }
        let mut by_code = vec![Vec::new(); table.len()];
        let mut exact = vec![Vec::new(); table.len()];
        for p in &panels {
            for (i, &o) in p.owner.iter().enumerate() {
                if o == 0 || o == UNOWNED {
                    continue;
                }
                let (x, y) = (p.x0 + i % p.w, p.y0 + i / p.w);
                if x >= w || y >= h {
                    continue;
                }
                let at = (y * w + x) as u32;
                by_code[usize::from(o) - 1].push(at);
                match p.owner2[i] {
                    0 => exact[usize::from(o) - 1].push(at),
                    o2 => by_code[usize::from(o2) - 1].push(at),
                }
            }
        }
        Ok(Self {
            origin: [area.x as f32, area.y as f32],
            w,
            h,
            codes: table,
            panels,
            by_code,
            exact,
        })
    }

    fn code_of(&self, name: &str) -> Option<usize> {
        self.codes.iter().position(|(_, n, _)| n == name)
    }

    /// The config-frame centre of document pixel `(x, y)`.
    fn point(&self, x: usize, y: usize) -> (f32, f32) {
        (
            x as f32 + 0.5 + self.origin[0],
            y as f32 + 0.5 + self.origin[1],
        )
    }

    /// The panel of `code` and the pixel `(x, y)` in it.
    fn local(&self, code: usize, x: i64, y: i64) -> Option<(&PanelInk, usize)> {
        let p = self.panels.get(self.codes[code].2)?;
        let (lx, ly) = (x - p.x0 as i64, y - p.y0 as i64);
        (lx >= 0 && ly >= 0 && (lx as usize) < p.w && (ly as usize) < p.h)
            .then(|| (p, ly as usize * p.w + lx as usize))
    }

    /// Whether `code` has ink at document pixel `(x, y)`, alone or blended.
    fn inked(&self, code: usize, x: i64, y: i64) -> bool {
        let c = code as u16 + 1;
        self.local(code, x, y)
            .is_some_and(|(p, i)| p.owner[i] == c || p.owner2[i] == c)
    }

    /// The alpha at document pixel `(x, y)` of `code`'s panel.
    fn alpha_at(&self, code: usize, x: i64, y: i64) -> u8 {
        self.local(code, x, y).map_or(0, |(p, i)| p.rgba[i * 4 + 3])
    }

    /// Whether `code` has ink within `reach` pixels (a square) of `(x, y)`.
    fn inked_near(&self, code: usize, x: i64, y: i64, reach: i64) -> bool {
        (y - reach..=y + reach).any(|ny| (x - reach..=x + reach).any(|nx| self.inked(code, nx, ny)))
    }

    /// The alpha of document pixel index `i` in `code`'s panel.
    fn alpha(&self, code: usize, i: u32) -> u8 {
        let (x, y) = (i as usize % self.w, i as usize / self.w);
        self.local(code, x as i64, y as i64)
            .map_or(0, |(p, i)| p.rgba[i * 4 + 3])
    }
}

fn unpremultiply(px: &[u8]) -> [i32; 3] {
    let a = i32::from(px[3]).max(1);
    [0, 1, 2].map(|k| (i32::from(px[k]) * 255 + a / 2) / a)
}

fn distance(a: [i32; 3], b: [u8; 3]) -> i32 {
    (0..3)
        .map(|k| (a[k] - i32::from(b[k])).abs())
        .max()
        .unwrap_or(0)
}

/// The owner of every pixel of one panel raster: the code within
/// [`CODE_TOLERANCE`], else a blend of two codes of the panel (one of them
/// inked within two pixels), else [`UNOWNED`].
fn decode(
    rgba: &[u8],
    w: usize,
    h: usize,
    local: &[usize],
    table: &[([u8; 3], String, usize)],
) -> (Vec<u16>, Vec<u16>) {
    let mut owner = vec![0u16; w * h];
    let mut owner2 = vec![0u16; w * h];
    let mut pending = Vec::new();
    for i in 0..w * h {
        let px = &rgba[i * 4..i * 4 + 4];
        if px[3] < INK_ALPHA {
            continue;
        }
        let u = unpremultiply(px);
        let best = local.iter().map(|&c| (distance(u, table[c].0), c)).min();
        match best {
            Some((d, c)) if d <= CODE_TOLERANCE => owner[i] = c as u16 + 1,
            _ => pending.push(i),
        }
    }
    // A pixel where two other inks cross (a corner of two anti-aliased
    // lines) can match a third code by chance. A code pixel with fewer than
    // three pixels of the same code around it (5 x 5) stays exact only when
    // no blend of its neighbours explains it.
    let lone: Vec<usize> = (0..w * h)
        .filter(|&i| {
            let o = owner[i];
            o != 0 && {
                let (x, y) = ((i % w) as i64, (i / w) as i64);
                let support = (y - 2..=y + 2)
                    .flat_map(|ny| (x - 2..=x + 2).map(move |nx| (nx, ny)))
                    .filter(|&(nx, ny)| {
                        (nx, ny) != (x, y)
                            && nx >= 0
                            && ny >= 0
                            && (nx as usize) < w
                            && (ny as usize) < h
                            && owner[ny as usize * w + nx as usize] == o
                    })
                    .count();
                support < 3
            }
        })
        .collect();
    for &i in &lone {
        let keep = owner[i];
        owner[i] = 0;
        let code = usize::from(keep) - 1;
        if !resolve_blend(
            rgba,
            w,
            h,
            local,
            table,
            &mut owner,
            &mut owner2,
            i,
            Some(code),
        ) {
            owner[i] = keep;
        }
    }
    for i in pending {
        if !resolve_blend(rgba, w, h, local, table, &mut owner, &mut owner2, i, None) {
            owner[i] = UNOWNED;
        }
    }
    (owner, owner2)
}

/// Explain pixel `i` as a blend of two codes of the panel, one of them
/// inked within two pixels; returns whether one fits. For a pixel that
/// matched code `lone` by chance, both codes must be inked nearby and
/// neither may be `lone`.
#[allow(clippy::too_many_arguments)]
fn resolve_blend(
    rgba: &[u8],
    w: usize,
    h: usize,
    local: &[usize],
    table: &[([u8; 3], String, usize)],
    owner: &mut [u16],
    owner2: &mut [u16],
    i: usize,
    lone: Option<usize>,
) -> bool {
    let u = unpremultiply(&rgba[i * 4..i * 4 + 4]);
    let (x, y) = ((i % w) as i64, (i / w) as i64);
    let mut near: Vec<usize> = Vec::new();
    for ny in (y - 2).max(0)..=(y + 2).min(h as i64 - 1) {
        for nx in (x - 2).max(0)..=(x + 2).min(w as i64 - 1) {
            let j = ny as usize * w + nx as usize;
            for o in [owner[j], owner2[j]] {
                let code = usize::from(o).wrapping_sub(1);
                if o != 0 && o != UNOWNED && Some(code) != lone && !near.contains(&code) {
                    near.push(code);
                }
            }
        }
    }
    // Two inks of the neighbourhood first; a pixel with no exact code may
    // also blend a neighbour's code with any code of the panel.
    let fit = |partners: &[usize]| {
        let mut best: Option<(i32, usize, usize)> = None;
        for &a in &near {
            for &b in partners {
                if a == b {
                    continue;
                }
                let d = blend_distance(u, table[a].0, table[b].0);
                if best.is_none_or(|(e, _, _)| d < e) {
                    best = Some((d, a, b));
                }
            }
        }
        best.filter(|(d, _, _)| *d <= CODE_TOLERANCE)
    };
    let best = match fit(&near) {
        Some(best) => Some(best),
        None if lone.is_none() => fit(local),
        None => None,
    };
    match best {
        Some((_, a, b)) => {
            owner[i] = a as u16 + 1;
            owner2[i] = b as u16 + 1;
            true
        }
        None => false,
    }
}

/// The distance of `u` from the segment between codes `a` and `b`: the
/// colour of `a` composited over `b` at some coverage.
fn blend_distance(u: [i32; 3], a: [u8; 3], b: [u8; 3]) -> i32 {
    let d = [0, 1, 2].map(|k| f32::from(a[k]) - f32::from(b[k]));
    let v = [0, 1, 2].map(|k| u[k] as f32 - f32::from(b[k]));
    let len = d.iter().map(|x| x * x).sum::<f32>();
    if len == 0.0 {
        return i32::MAX;
    }
    let t = ((0..3).map(|k| d[k] * v[k]).sum::<f32>() / len).clamp(0.0, 1.0);
    (0..3)
        .map(|k| (v[k] - t * d[k]).abs().round() as i32)
        .max()
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Hit regions as pixel masks
// ---------------------------------------------------------------------------

/// The hit region of one element in document pixels: the region its
/// selection outlines (`selection_box().outline()`: the padded box, or the
/// union of its parts' `part_outline`s), sampled at pixel centres.
struct Mask {
    x0: i64,
    y0: i64,
    w: usize,
    h: usize,
    bits: Vec<bool>,
}

impl Mask {
    fn empty() -> Self {
        Self {
            x0: 0,
            y0: 0,
            w: 0,
            h: 0,
            bits: Vec::new(),
        }
    }

    fn of(m: &Measured, ink: &SceneInk) -> Self {
        let Some(sb) = &m.selection else {
            return Self::empty();
        };
        let (ox, oy) = (ink.origin[0], ink.origin[1]);
        let outlines: Vec<PartOutline> = match sb.outline() {
            SelectionOutline::Rect => Vec::new(),
            SelectionOutline::Parts(parts) => parts.iter().map(part_outline).collect(),
        };
        let r = sb.rect();
        let (mut x0, mut y0, mut x1, mut y1) = (r.x, r.y, r.x + r.width, r.y + r.height);
        for o in &outlines {
            for c in o.corners() {
                x0 = x0.min(c[0]);
                y0 = y0.min(c[1]);
                x1 = x1.max(c[0]);
                y1 = y1.max(c[1]);
            }
        }
        let px0 = ((x0 - ox).floor() as i64 - 1).max(0);
        let py0 = ((y0 - oy).floor() as i64 - 1).max(0);
        let px1 = ((x1 - ox).ceil() as i64 + 1).min(ink.w as i64);
        let py1 = ((y1 - oy).ceil() as i64 + 1).min(ink.h as i64);
        if px1 <= px0 || py1 <= py0 {
            return Self::empty();
        }
        let (w, h) = ((px1 - px0) as usize, (py1 - py0) as usize);
        let mut bits = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let (cx, cy) = ink.point(px0 as usize + x, py0 as usize + y);
                bits[y * w + x] = if outlines.is_empty() {
                    r.contains(cx, cy)
                } else {
                    outlines.iter().any(|o| o.contains(cx, cy))
                };
            }
        }
        Self {
            x0: px0,
            y0: py0,
            w,
            h,
            bits,
        }
    }

    fn get(&self, x: i64, y: i64) -> bool {
        let (lx, ly) = (x - self.x0, y - self.y0);
        lx >= 0
            && ly >= 0
            && (lx as usize) < self.w
            && (ly as usize) < self.h
            && self.bits[ly as usize * self.w + lx as usize]
    }
}

/// The pixels of the document inside `r` (config frame), as a window
/// `(x0, y0, x1, y1)` clamped to the image.
fn window(ink: &SceneInk, r: RectF) -> (i64, i64, i64, i64) {
    (
        ((r.x - ink.origin[0]).floor() as i64).max(0),
        ((r.y - ink.origin[1]).floor() as i64).max(0),
        ((r.x + r.width - ink.origin[0]).ceil() as i64).min(ink.w as i64),
        ((r.y + r.height - ink.origin[1]).ceil() as i64).min(ink.h as i64),
    )
}

/// The ink extent of `pixels` (document indices) in the config frame:
/// pixel edges, not centres.
fn extent(ink: &SceneInk, pixels: impl Iterator<Item = u32>) -> Option<RectF> {
    let mut b: Option<(usize, usize, usize, usize)> = None;
    for i in pixels {
        let (x, y) = (i as usize % ink.w, i as usize / ink.w);
        b = Some(match b {
            None => (x, y, x, y),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        });
    }
    b.map(|(x0, y0, x1, y1)| RectF {
        x: x0 as f32 + ink.origin[0],
        y: y0 as f32 + ink.origin[1],
        width: (x1 - x0 + 1) as f32,
        height: (y1 - y0 + 1) as f32,
    })
}

/// How far each edge of `outline` lies from the same edge of `ink`, the
/// worst of the four, beyond the selection padding.
fn box_misfit(outline: RectF, ink: RectF) -> f32 {
    [
        outline.x - ink.x,
        outline.y - ink.y,
        outline.x + outline.width - (ink.x + ink.width),
        outline.y + outline.height - (ink.y + ink.height),
    ]
    .into_iter()
    .map(|d| (d.abs() - SELECTION_PADDING).max(0.0))
    .fold(0.0, f32::max)
}

// ---------------------------------------------------------------------------
// I2: the hit region is the drawn outline
// ---------------------------------------------------------------------------

/// I2 for every element of one layer case: every strong ink pixel lies in
/// the element's hit region; the region stays near the ink (a box within
/// the padding plus [`BOX_FIT`] of the ink extent, a stroke's band within
/// its grab half width plus [`STROKE_FIT`] of ink); `contains` agrees with
/// the region on capped edge-band samples; and at every strong ink pixel
/// the topmost region in registration order belongs to an element that has
/// ink there.
#[allow(clippy::too_many_arguments)]
fn check_i2<C: DragConfig>(
    report: &mut Report,
    at: &At<'_>,
    cfg: &C,
    measure: &dyn MeasureText,
    map: &HitMap<C>,
    measured: &[Measured],
    masks: &[Mask],
    ink: &SceneInk,
    only: Option<usize>,
) {
    let mut strong_total = 0usize;
    let mut samples_total = 0usize;
    for (index, (m, mask)) in measured.iter().zip(masks).enumerate() {
        if only.is_some_and(|o| o != index) {
            continue;
        }
        let Some(sb) = &m.selection else {
            continue;
        };
        let element = map.get(m.id).expect("measured id");
        // `contains` against the mask, on the edge band of the mask.
        let pad = 3;
        let win = (
            (mask.x0 - pad).max(0) as u32,
            (mask.y0 - pad).max(0) as u32,
            ((mask.x0 + mask.w as i64 + pad).min(ink.w as i64)) as u32,
            ((mask.y0 + mask.h as i64 + pad).min(ink.h as i64)) as u32,
        );
        let band = edge_band::edge_band(win, |x, y| mask.get(i64::from(x), i64::from(y)));
        let stride = band.len().div_ceil(CONTAINS_SAMPLES).max(1);
        let mut disagree = Vec::new();
        for &(x, y) in band.iter().step_by(stride) {
            samples_total += 1;
            let (cx, cy) = ink.point(x as usize, y as usize);
            if element.contains(cfg, measure, cx, cy) != mask.get(i64::from(x), i64::from(y)) {
                disagree.push((cx, cy));
            }
        }
        report.check(at, &m.name, Inv::I2, disagree.is_empty(), || {
            format!(
                "contains disagrees with the outline at {} of {} samples, first {:?}",
                disagree.len(),
                band.len().div_ceil(stride),
                disagree[0]
            )
        });
        let Some(code) = ink.code_of(&m.name) else {
            continue;
        };
        let pixels = &ink.by_code[code];
        if pixels.is_empty() {
            continue;
        }
        // Every strong ink pixel inside the region. A blend pixel is the
        // overlap of two elements: its owner is ambiguous, so it is not
        // strong ink of either.
        let strong: Vec<u32> = ink.exact[code]
            .iter()
            .copied()
            .filter(|&i| ink.alpha(code, i) >= STRONG_ALPHA)
            .collect();
        strong_total += strong.len();
        let outside: Vec<u32> = strong
            .iter()
            .copied()
            .filter(|&i| !mask.get(i64::from(i) % ink.w as i64, i64::from(i) / ink.w as i64))
            .collect();
        report.check(at, &m.name, Inv::I2, outside.is_empty(), || {
            let i = outside[0] as usize;
            format!(
                "{} of {} strong ink pixels outside the hit region, first {:?}",
                outside.len(),
                strong.len(),
                ink.point(i % ink.w, i / ink.w)
            )
        });
        // The region near the ink.
        match sb.outline() {
            SelectionOutline::Rect => {
                if let Some(e) = extent(ink, pixels.iter().copied()) {
                    let misfit = box_misfit(sb.rect(), e);
                    report.check(at, &m.name, Inv::I2, misfit <= BOX_FIT * at.scale, || {
                        format!(
                            "box {:?} is {misfit:.1} px past the padding from the ink extent {e:?}",
                            sb.rect()
                        )
                    });
                }
            }
            SelectionOutline::Parts(parts) => {
                check_parts_near_ink(report, at, &m.name, parts, code, ink);
            }
        }
        // The topmost region at each strong pixel has ink there.
        let mut covered: Vec<(String, (f32, f32))> = Vec::new();
        for &i in &strong {
            let (x, y) = (i64::from(i) % ink.w as i64, i64::from(i) / ink.w as i64);
            if !mask.get(x, y) {
                continue;
            }
            for (t, (tm, tmask)) in measured.iter().zip(masks).enumerate().rev() {
                if !tmask.get(x, y) {
                    continue;
                }
                let has_ink = t == index
                    || ink
                        .code_of(&tm.name)
                        .is_some_and(|c| ink.inked_near(c, x, y, TOPMOST_REACH));
                if !has_ink {
                    covered.push((tm.name.clone(), ink.point(x as usize, y as usize)));
                }
                break;
            }
        }
        report.check(at, &m.name, Inv::I2, covered.is_empty(), || {
            let mut by: BTreeMap<&str, usize> = BTreeMap::new();
            for (n, _) in &covered {
                *by.entry(n.as_str()).or_default() += 1;
            }
            format!(
                "{} of {} strong ink pixels hit another element that has no ink there ({by:?}), first {:?}",
                covered.len(),
                strong.len(),
                covered[0].1
            )
        });
    }
    if only.is_none() {
        report.count(at, "strong ink pixels", strong_total);
        report.count(at, "contains samples", samples_total);
    }
}

/// The parts rule of I2: a text block or rectangle part's outline fits the
/// ink inside it as a box does, and every point of a stroke or triangle
/// part's outline lies within its grab reach of ink.
fn check_parts_near_ink(
    report: &mut Report,
    at: &At<'_>,
    name: &str,
    parts: &[DrawnPart],
    code: usize,
    ink: &SceneInk,
) {
    let mut empty = 0;
    let mut misfits = Vec::new();
    let mut far = Vec::new();
    for part in parts {
        let outline = part_outline(part);
        match *part {
            DrawnPart::Text(r) | DrawnPart::Rect(r) => {
                let o = r.expanded(SELECTION_PADDING);
                let (x0, y0, x1, y1) = window(ink, o);
                let inside =
                    (y0..y1)
                        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                        .filter(|&(x, y)| {
                            let (cx, cy) = ink.point(x as usize, y as usize);
                            o.contains(cx, cy) && ink.inked(code, x, y)
                        });
                match extent(
                    ink,
                    inside.map(|(x, y)| (y as usize * ink.w + x as usize) as u32),
                ) {
                    None => empty += 1,
                    Some(e) => {
                        let misfit = box_misfit(o, e);
                        if misfit > BOX_FIT * at.scale {
                            misfits.push((o, e, misfit));
                        }
                    }
                }
            }
            DrawnPart::Segment { .. } | DrawnPart::Triangle(_) => {
                let reach = match *part {
                    DrawnPart::Segment { width, .. } => segment_grab_half_width(width) + STROKE_FIT,
                    _ => SELECTION_PADDING + STROKE_FIT,
                };
                let c = outline.corners();
                let bounds = RectF {
                    x: c.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min),
                    y: c.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min),
                    width: 0.0,
                    height: 0.0,
                };
                let right = c.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
                let bottom = c.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max);
                let bounds = RectF {
                    width: right - bounds.x,
                    height: bottom - bounds.y,
                    ..bounds
                };
                let (x0, y0, x1, y1) = window(ink, bounds);
                let r = reach.ceil() as i64;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let (cx, cy) = ink.point(x as usize, y as usize);
                        if !outline.contains(cx, cy) {
                            continue;
                        }
                        let near = (y - r..=y + r)
                            .any(|ny| (x - r..=x + r).any(|nx| ink.inked(code, nx, ny)));
                        if !near {
                            far.push((cx, cy));
                        }
                    }
                }
            }
        }
    }
    report.check(at, name, Inv::I2, empty == 0, || {
        format!(
            "{empty} of {} text or box parts have no ink inside their outline",
            parts.len()
        )
    });
    report.check(at, name, Inv::I2, misfits.is_empty(), || {
        format!(
            "{} parts' outlines do not fit their ink, first {:?} against {:?} ({:.1} px)",
            misfits.len(),
            misfits[0].0,
            misfits[0].1,
            misfits[0].2
        )
    });
    report.check(at, name, Inv::I2, far.is_empty(), || {
        format!(
            "{} points of stroke outlines are farther than the grab reach from ink, first {:?}",
            far.len(),
            far[0]
        )
    });
}

// ---------------------------------------------------------------------------
// I7: no orphan ink, no ghost element; the product draws the CPU raster
// ---------------------------------------------------------------------------

/// Orphan ink: pixels no code explains, and ink of an element that no layer
/// registers or that has no bounds. A ghost: an element with bounds and no
/// ink. The data area paints nothing by design and has no code.
fn check_owners(
    report: &mut Report,
    at: &At<'_>,
    ink: &SceneInk,
    registered: &BTreeMap<String, Option<RectF>>,
) {
    for p in &ink.panels {
        let unowned: Vec<usize> = (0..p.owner.len())
            .filter(|&i| p.owner[i] == UNOWNED)
            .collect();
        let panel = layer_name(p.layer);
        report.check(at, &format!("<unowned ink in the {panel} panel>"), Inv::I7, unowned.is_empty(), || {
            let i = unowned[0];
            let u = unpremultiply(&p.rgba[i * 4..i * 4 + 4]);
            let nearest = ink
                .codes
                .iter()
                .min_by_key(|(rgb, _, _)| distance(u, *rgb))
                .map(|(_, n, _)| n.as_str())
                .unwrap_or("-");
            format!(
                "{} pixels no element's code explains, first {:?} colour {u:?} alpha {} (nearest code: {nearest})",
                unowned.len(),
                ink.point(p.x0 + i % p.w, p.y0 + i / p.w),
                p.rgba[i * 4 + 3]
            )
        });
    }
    for (code, (_, name, _)) in ink.codes.iter().enumerate() {
        let pixels = &ink.by_code[code];
        match registered.get(name) {
            None => report.check(at, name, Inv::I7, pixels.is_empty(), || {
                format!(
                    "{} px of ink, but no layer registers the element",
                    pixels.len()
                )
            }),
            Some(None) => report.check(at, name, Inv::I7, pixels.is_empty(), || {
                let i = pixels[0] as usize;
                format!(
                    "{} px of orphan ink: the element has no bounds, first {:?}",
                    pixels.len(),
                    ink.point(i % ink.w, i / ink.w)
                )
            }),
            Some(Some(b)) => report.check(at, name, Inv::I7, !pixels.is_empty(), || {
                format!("ghost: bounds {b:?} but no ink")
            }),
        }
        report.count(at, "ink pixels", pixels.len());
    }
}

/// The CPU composite of the panel rasters is what the product exports:
/// every opaque decoration pixel of `ink` has the same colour in the
/// renderer's `export_document_rgba` of the palette document (annotation
/// layer hidden, so nothing is drawn over the decoration). The renderer
/// draws every panel's data before any panel's decoration
/// (`composite::FRAME_SLOTS`), so the decoration rasters composited alone,
/// in panel order, are the export's top layer.
fn check_export(
    report: &mut Report,
    at: &At<'_>,
    r: &mut Renderer,
    palette_1x: &Config,
    series: &[SeriesConfig],
    ink: &SceneInk,
) {
    // The export takes the unscaled document and the scale.
    let mut doc = palette_1x.clone();
    doc.annotations.visible = false;
    let image = match r.export_document_rgba(&Chart::new(doc), series, at.scale) {
        Ok(image) => image,
        Err(e) => {
            report.check(at, "<export>", Inv::I7, false, || {
                format!("export failed: {e}")
            });
            return;
        }
    };
    if (image.width as usize, image.height as usize) != (ink.w, ink.h) {
        report.check(at, "<export>", Inv::I7, false, || {
            format!(
                "export is {}x{}, the document {}x{}",
                image.width, image.height, ink.w, ink.h
            )
        });
        return;
    }
    // Composite the decoration rasters as the renderer does: in panel order,
    // premultiplied over, above all data.
    let mut doc_rgba = vec![0u8; ink.w * ink.h * 4];
    for p in &ink.panels {
        for y in 0..p.h {
            for x in 0..p.w {
                let (dx, dy) = (p.x0 + x, p.y0 + y);
                if dx >= ink.w || dy >= ink.h {
                    continue;
                }
                let s = &p.rgba[(y * p.w + x) * 4..(y * p.w + x) * 4 + 4];
                let d = &mut doc_rgba[(dy * ink.w + dx) * 4..(dy * ink.w + dx) * 4 + 4];
                let inv = 255 - u32::from(s[3]);
                for k in 0..4 {
                    d[k] = (u32::from(s[k]) + (u32::from(d[k]) * inv + 127) / 255).min(255) as u8;
                }
            }
        }
    }
    let mut differ = Vec::new();
    for i in 0..ink.w * ink.h {
        let d = &doc_rgba[i * 4..i * 4 + 4];
        if d[3] < 255 {
            continue;
        }
        let e = &image.rgba[i * 4..i * 4 + 4];
        if (0..3).any(|k| d[k].abs_diff(e[k]) > 2) {
            differ.push(i);
        }
    }
    report.check(at, "<export>", Inv::I7, differ.is_empty(), || {
        let i = differ[0];
        format!(
            "{} opaque decoration pixels differ from the export, first {:?}: raster {:?}, export {:?}",
            differ.len(),
            ink.point(i % ink.w, i / ink.w),
            &doc_rgba[i * 4..i * 4 + 4],
            &image.rgba[i * 4..i * 4 + 4]
        )
    });
}

// ---------------------------------------------------------------------------
// I3: the drawn highlight is the model's box
// ---------------------------------------------------------------------------

/// I3 for one layer case at 1x: the selection boxes of the layer's
/// elements, batched so no two boxes of a batch come near each other, are
/// drawn by the decoration raster (`..._with_selection`, each box routed
/// into the topmost panel holding its centre, as the renderer routes it);
/// every pixel the highlight changes lies on a box outline or a resize
/// handle (half the stroke plus a pixel of anti-aliasing), and every edge
/// of every outline is drawn.
fn check_i3(report: &mut Report, at: &At<'_>, measured: &[Measured], ink: &SceneInk) {
    let boxes: Vec<(usize, &SelectionBox)> = measured
        .iter()
        .enumerate()
        .filter_map(|(i, m)| m.selection.as_ref().map(|s| (i, s)))
        .collect();
    let reach = |sb: &SelectionBox| {
        let mut r = sb.rect().expanded(sb.stroke_width() + 4.0);
        for h in sb.handles() {
            r = renderer::sink::union(r, h.expanded(sb.stroke_width() + 4.0));
        }
        r
    };
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (k, (_, sb)) in boxes.iter().enumerate() {
        let region = reach(sb);
        match groups.iter_mut().find(|g| {
            g.iter()
                .all(|&j| !intersects(reach(boxes[j].1), region, 0.0))
        }) {
            Some(g) => g.push(k),
            None => groups.push(vec![k]),
        }
    }
    report.count(at, "selection rasters", 0);
    for group in &groups {
        let selection: Vec<SelectionBox> = group.iter().map(|&k| boxes[k].1.clone()).collect();
        // Route each box into the topmost panel whose area holds its centre.
        let mut routed: Vec<Vec<SelectionBox>> = vec![Vec::new(); ink.panels.len()];
        for sb in &selection {
            let (cx, cy) = sb.rect().center();
            let index = ink
                .panels
                .iter()
                .rposition(|p| RectF::from_rect(&p.config.chart_area.0).contains(cx, cy))
                .unwrap_or(ink.panels.len() - 1);
            routed[index].push(sb.clone());
        }
        let mut changed: Vec<(f32, f32)> = Vec::new();
        for (p, boxes) in ink.panels.iter().zip(&routed) {
            if boxes.is_empty() {
                continue;
            }
            report.count(at, "selection rasters", 1);
            let selected = match try_raster_chart_layer_to_rgba_with_selection(
                &p.config,
                AxisLayerKind::Decoration,
                boxes,
            ) {
                Ok(rgba) => rgba,
                Err(e) => {
                    report.check(at, "<selection raster>", Inv::I3, false, || e.to_string());
                    continue;
                }
            };
            for i in 0..p.w * p.h {
                if selected[i * 4..i * 4 + 4] != p.rgba[i * 4..i * 4 + 4] {
                    changed.push(ink.point(p.x0 + i % p.w, p.y0 + i / p.w));
                }
            }
        }
        for &k in group {
            let (index, sb) = boxes[k];
            let name = &measured[index].name;
            let halo = sb.stroke_width() * 0.5 + 1.0;
            let own: Vec<(f32, f32)> = changed
                .iter()
                .copied()
                .filter(|&(x, y)| reach(sb).contains(x, y))
                .collect();
            let stray: Vec<(f32, f32)> = own
                .iter()
                .copied()
                .filter(|&(x, y)| !on_outline(sb, x, y, halo))
                .collect();
            report.check(at, name, Inv::I3, stray.is_empty(), || {
                format!(
                    "{} highlight pixels off the box outline and handles, first {:?}",
                    stray.len(),
                    stray[0]
                )
            });
            let missing = missing_edges(sb, &own, halo);
            report.check(at, name, Inv::I3, missing.is_empty(), || {
                format!(
                    "{} outline edges are not drawn, first {:?} ({} changed pixels near the box)",
                    missing.len(),
                    missing[0],
                    own.len()
                )
            });
        }
        // A changed pixel near no box of the batch.
        let unexplained: Vec<(f32, f32)> = changed
            .iter()
            .copied()
            .filter(|&(x, y)| !group.iter().any(|&k| reach(boxes[k].1).contains(x, y)))
            .collect();
        report.check(at, "<highlight>", Inv::I3, unexplained.is_empty(), || {
            format!(
                "{} highlight pixels near no selected box, first {:?}",
                unexplained.len(),
                unexplained[0]
            )
        });
    }
}

/// The outline edges of `sb`: the box's four sides, or each part outline's
/// edges.
fn outline_edges(sb: &SelectionBox) -> Vec<([f32; 2], [f32; 2])> {
    match sb.outline() {
        SelectionOutline::Rect => {
            let r = sb.rect();
            let (x0, y0, x1, y1) = (r.x, r.y, r.x + r.width, r.y + r.height);
            vec![
                ([x0, y0], [x1, y0]),
                ([x1, y0], [x1, y1]),
                ([x1, y1], [x0, y1]),
                ([x0, y1], [x0, y0]),
            ]
        }
        SelectionOutline::Parts(parts) => parts
            .iter()
            .flat_map(|p| {
                let o = part_outline(p);
                let c = o.corners().to_vec();
                (0..c.len())
                    .map(|i| (c[i], c[(i + 1) % c.len()]))
                    .collect::<Vec<_>>()
            })
            .collect(),
    }
}

fn segment_distance(p: (f32, f32), a: [f32; 2], b: [f32; 2]) -> f32 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let len = d[0] * d[0] + d[1] * d[1];
    let t = if len > 0.0 {
        (((p.0 - a[0]) * d[0] + (p.1 - a[1]) * d[1]) / len).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a[0] - t * d[0]).hypot(p.1 - a[1] - t * d[1])
}

fn on_outline(sb: &SelectionBox, x: f32, y: f32, halo: f32) -> bool {
    outline_edges(sb)
        .iter()
        .any(|&(a, b)| segment_distance((x, y), a, b) <= halo)
        || sb.handles().iter().any(|h| h.expanded(halo).contains(x, y))
}

/// The edges of `sb`'s outline with no changed pixel along their middle
/// 60 %. Edges shorter than 3 px are left to their neighbours.
fn missing_edges(
    sb: &SelectionBox,
    changed: &[(f32, f32)],
    halo: f32,
) -> Vec<([f32; 2], [f32; 2])> {
    outline_edges(sb)
        .into_iter()
        .filter(|&(a, b)| {
            let len = (b[0] - a[0]).hypot(b[1] - a[1]);
            if len < 3.0 {
                return false;
            }
            let lerp = |t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let (m0, m1) = (lerp(0.2), lerp(0.8));
            let need = if len >= 10.0 { 4 } else { 1 };
            changed
                .iter()
                .filter(|&&p| segment_distance(p, m0, m1) <= halo)
                .count()
                < need
        })
        .collect()
}

// ---------------------------------------------------------------------------
// I4: no overlap
// ---------------------------------------------------------------------------

/// Two drawn parts overlap when their extents share more than this, in
/// both directions (anti-aliasing of abutting parts).
const OVERLAP_TOLERANCE: f32 = 1.0;

/// The pairs of elements of one chart (`prefix`: the parent's or a
/// subplot's) that must not overlap, `(a, b)` with `a == b` for the parts
/// of one element among themselves: the labels of one axis pairwise; each
/// side's labels and title; the chart title and the top labels and title;
/// the axis titles of different sides; the colourbar strip, labels and
/// title pairwise and against the labels and title of the side they sit
/// on; the legend and the colourbar chrome; each overlay's labels and
/// title against the parent's right ones and every other overlay's; a
/// polar chart's angle labels against its radius labels.
fn overlap_pairs(cfg: &Config, prefix: &str) -> Vec<(String, String)> {
    let n = |name: &str| format!("{prefix}{name}");
    let mut pairs = Vec::new();
    for side in SIDES {
        let labels = n(&format!("tick_labels_{side}"));
        pairs.push((labels.clone(), labels.clone()));
        pairs.push((labels, n(&format!("axis_title_{side}"))));
    }
    pairs.push((n("chart_title"), n("tick_labels_top")));
    pairs.push((n("chart_title"), n("axis_title_top")));
    for (i, a) in SIDES.iter().enumerate() {
        for b in &SIDES[i + 1..] {
            pairs.push((n(&format!("axis_title_{a}")), n(&format!("axis_title_{b}"))));
        }
    }
    if let Some(bar) = &cfg.colorbar {
        let side = match bar.side {
            renderer::layout::Side::Top => "top",
            renderer::layout::Side::Bottom => "bottom",
            renderer::layout::Side::Left => "left",
            renderer::layout::Side::Right => "right",
        };
        let group = ["colorbar", "colorbar_tick_labels", "colorbar_title"];
        pairs.push((n("colorbar_tick_labels"), n("colorbar_tick_labels")));
        for (i, a) in group.iter().enumerate() {
            for b in &group[i + 1..] {
                pairs.push((n(a), n(b)));
            }
            pairs.push((n(a), n(&format!("tick_labels_{side}"))));
            pairs.push((n(a), n(&format!("axis_title_{side}"))));
        }
        for a in [
            "colorbar",
            "colorbar_axis",
            "colorbar_tick_labels",
            "colorbar_title",
        ] {
            pairs.push((n("legend"), n(a)));
        }
    }
    for i in 0..cfg.overlays.len() {
        let own = [
            n(&format!("overlay_{i}:tick_labels_right")),
            n(&format!("overlay_{i}:axis_title_right")),
        ];
        pairs.push((own[0].clone(), own[0].clone()));
        pairs.push((own[0].clone(), own[1].clone()));
        for a in &own {
            pairs.push((a.clone(), n("tick_labels_right")));
            pairs.push((a.clone(), n("axis_title_right")));
            for j in i + 1..cfg.overlays.len() {
                pairs.push((a.clone(), n(&format!("overlay_{j}:tick_labels_right"))));
                pairs.push((a.clone(), n(&format!("overlay_{j}:axis_title_right"))));
            }
        }
    }
    if cfg.polar.is_some() {
        pairs.push((n("tick_labels_bottom"), n("tick_labels_left")));
    }
    pairs
}

fn overlap(a: &DrawnPart, b: &DrawnPart) -> bool {
    intersects(a.extent(), b.extent(), OVERLAP_TOLERANCE)
}

/// I4 for one document: every pair of [`overlap_pairs`] of the parent and
/// of every subplot, on the parts the layer cases measured.
fn check_i4(
    report: &mut Report,
    at: &At<'_>,
    doc: &Config,
    drawn: &BTreeMap<String, Vec<DrawnPart>>,
) {
    let mut charts = vec![(doc.clone(), String::new())];
    for i in 0..doc.subplots.len() {
        if let Some(sub) = doc.subplot_config(i) {
            charts.push((sub, format!("subplot_{i}:")));
        }
    }
    for (cfg, prefix) in &charts {
        for (a, b) in overlap_pairs(cfg, prefix) {
            let (Some(pa), Some(pb)) = (drawn.get(&a), drawn.get(&b)) else {
                continue;
            };
            let mut hits = Vec::new();
            for (i, x) in pa.iter().enumerate() {
                let others = if a == b { &pb[i + 1..] } else { &pb[..] };
                for y in others {
                    if overlap(x, y) {
                        hits.push((x.extent(), y.extent()));
                    }
                }
            }
            let name = if a == b {
                a.clone()
            } else {
                format!("{a} x {b}")
            };
            report.check(at, &name, Inv::I4, hits.is_empty(), || {
                format!(
                    "{} overlapping part pairs, first {:?} and {:?}",
                    hits.len(),
                    hits[0].0,
                    hits[0].1
                )
            });
        }
    }
}

// ---------------------------------------------------------------------------
// I5: 2x is 1x scaled
// ---------------------------------------------------------------------------

/// One scale of one scene: its ink, and per layer case and element the
/// bounds and the number of text parts drawn.
struct ScaleRun {
    ink: SceneInk,
    geometry: BTreeMap<(String, String), (Option<RectF>, usize)>,
}

/// A pixel whose box-filtered 2x ink differs from its 1x ink by more than
/// this has moved (`scale_consistency`'s threshold).
const SCALE_DIFF: i32 = 96;
/// The ink extent at 2x, halved, may differ from the 1x extent by this
/// much per edge: a pixel of anti-aliasing at 1x plus one at 2x.
const SCALE_EXTENT: f32 = 1.5;
/// The share of an element's 1x ink pixels that may differ by more than
/// [`SCALE_DIFF`] after box-filtering its 2x ink: hinting and
/// anti-aliasing at glyph and stroke edges, 14 to 22 % for tick labels and
/// up to 48 % for a two-glyph title. A moved element differs at nearly all
/// of them, and the extent check catches a shift of more than a pixel.
const SCALE_SHARE: f64 = 0.5;

/// I5 for one scene: bounds and label sets per layer case, and the ink of
/// every element.
fn check_i5(report: &mut Report, scene: &'static str, one: &ScaleRun, two: &ScaleRun) {
    for ((layer, name), (b1, t1)) in &one.geometry {
        let at = At {
            scene,
            layer,
            scale: 2.0,
        };
        let Some((b2, t2)) = two.geometry.get(&(layer.clone(), name.clone())) else {
            report.check(&at, name, Inv::I5, false, || "registered at 1x only".into());
            continue;
        };
        match (b1, b2) {
            (Some(b1), Some(b2)) => {
                let worst = [
                    b2.x / 2.0 - b1.x,
                    b2.y / 2.0 - b1.y,
                    (b2.x + b2.width) / 2.0 - (b1.x + b1.width),
                    (b2.y + b2.height) / 2.0 - (b1.y + b1.height),
                ]
                .into_iter()
                .map(f32::abs)
                .fold(0.0, f32::max);
                report.check(&at, name, Inv::I5, worst <= 1.0, || {
                    format!("bounds {b2:?} at 2x are {worst:.2} px off {b1:?} at 1x")
                });
            }
            (None, None) => {}
            _ => report.check(&at, name, Inv::I5, false, || {
                format!("bounds {b1:?} at 1x but {b2:?} at 2x")
            }),
        }
        if name.contains("tick_labels") {
            report.check(&at, name, Inv::I5, t1 == t2, || {
                format!("{t1} labels drawn at 1x, {t2} at 2x")
            });
        }
    }
    let at = At {
        scene,
        layer: "document",
        scale: 2.0,
    };
    let (i1, i2) = (&one.ink, &two.ink);
    for (code, (_, name, _)) in i1.codes.iter().enumerate() {
        let n1 = i1.by_code[code].len();
        let n2 = i2.by_code[code].len();
        if n1 == 0 && n2 == 0 {
            continue;
        }
        // The window: both inks' extents in 1x pixels, grown by 2 px.
        let e1 = extent(i1, i1.by_code[code].iter().copied());
        let e2 = extent(i2, i2.by_code[code].iter().copied()).map(|e| e.scaled(0.5));
        let Some(win) = [e1, e2].into_iter().flatten().reduce(renderer::sink::union) else {
            continue;
        };
        let (x0, y0, x1, y1) = window(i1, win.expanded(2.0));
        let mut differ = 0usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let a1 = if i1.inked(code, x, y) {
                    i32::from(i1.alpha_at(code, x, y))
                } else {
                    0
                };
                let mut sum = 0;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = (x * 2 + dx, y * 2 + dy);
                    if i2.inked(code, sx, sy) {
                        sum += i32::from(i2.alpha_at(code, sx, sy));
                    }
                }
                if (sum / 4 - a1).abs() > SCALE_DIFF {
                    differ += 1;
                }
            }
        }
        if let (Some(e1), Some(e2)) = (e1, e2) {
            let worst = [
                e2.x - e1.x,
                e2.y - e1.y,
                e2.x + e2.width - (e1.x + e1.width),
                e2.y + e2.height - (e1.y + e1.height),
            ]
            .into_iter()
            .map(f32::abs)
            .fold(0.0, f32::max);
            report.check(&at, name, Inv::I5, worst <= SCALE_EXTENT, || {
                format!("ink extent {e2:?} at 2x (halved) is {worst:.1} px off {e1:?} at 1x")
            });
        }
        let share = differ as f64 / n1.max(1) as f64;
        report.count(&at, "I5 compared ink pixels", n1);
        report.check(&at, name, Inv::I5, share <= SCALE_SHARE, || {
            format!(
                "{differ} of its {n1} ink pixels ({:.0} %) differ by more than {SCALE_DIFF} from its 2x ink box-filtered ({n2} px)",
                share * 100.0
            )
        });
    }
}

// ---------------------------------------------------------------------------
// I6: a drag keeps every element grabbable
// ---------------------------------------------------------------------------

const DRAG: (f32, f32) = (7.0, 5.0);
/// A drag that leaves every chart area.
const FAR_DRAG: (f32, f32) = (10_000.0, 10_000.0);
/// Hit tests on the moved ink, per element.
const DRAG_HIT_SAMPLES: usize = 16;

/// The chart an element of `doc` belongs to: its subplot's, else the
/// parent.
fn chart_of(doc: &Config, name: &str) -> Config {
    name.strip_prefix("subplot_")
        .and_then(|rest| rest.split(':').next())
        .and_then(|i| i.parse::<usize>().ok())
        .and_then(|i| doc.subplot_config(i))
        .unwrap_or_else(|| doc.clone())
}

/// How far a drag by [`DRAG`] moves an element's bounds under the nudge
/// policy (`layout/nudge.rs`): an axis only across its direction, `None`
/// for the polar axis and labels, which turn and slide along the ring.
fn expected_delta(chart: &Config, element: &Element) -> Option<(f32, f32)> {
    let (dx, dy) = DRAG;
    let polar = chart.polar.is_some();
    match element {
        Element::Axis(_) | Element::AxisLabel(_) if polar => None,
        Element::Axis(Side::Top | Side::Bottom) => Some((0.0, dy)),
        Element::Axis(_) => Some((dx, 0.0)),
        Element::ColorBarAxis => match chart.colorbar.as_ref().map(|b| &b.side) {
            Some(Side::Left | Side::Right) => Some((dx, 0.0)),
            _ => Some((0.0, dy)),
        },
        _ => Some((dx, dy)),
    }
}

/// Every registered element of `layer` measured, without the I7 checks of
/// [`measure_layer`].
fn measure_quiet(cfg: &Config, layer: LayerRef) -> (HitMap, Vec<Measured>) {
    let map = HitMap::for_active_layer(cfg);
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let measured = expected_names(layer)
        .into_iter()
        .filter_map(|name| {
            let id = map.find(&name)?;
            let element = map.get(id)?;
            let mut parts = Vec::new();
            element.emit(cfg, &measure, &mut Parts(&mut |p| parts.push(p)));
            Some(Measured {
                bounds: element.bounds(cfg, &measure),
                selection: map.selection_box(id, cfg, &measure),
                name,
                id,
                parts,
            })
        })
        .collect();
    (map, measured)
}

/// I6 for one layer case at 1x.
#[allow(clippy::too_many_arguments)]
fn check_i6(
    report: &mut Report,
    at: &At<'_>,
    cfg: &Config,
    layer: LayerRef,
    map: &HitMap,
    measured: &[Measured],
    ink: &SceneInk,
    codes: &[Code],
) {
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    for m in measured {
        let Some(bounds) = m.bounds else {
            continue;
        };
        let Some(drag) = map.get(m.id).and_then(|e| e.as_draggable()) else {
            continue;
        };
        report.count(at, "drags", 1);
        // A drag out of every chart area is refused and changes nothing.
        let mut far = cfg.clone();
        let result = drag.drag_by(&mut far, &measure, FAR_DRAG.0, FAR_DRAG.1);
        report.check(
            at,
            &m.name,
            Inv::I6,
            matches!(result, NudgeResult::Rejected(_)) && far == *cfg,
            || {
                format!(
                    "a drag by {FAR_DRAG:?} gave {result:?} and the config changed: {}",
                    far != *cfg
                )
            },
        );
        let mut moved = cfg.clone();
        let result = drag.drag_by(&mut moved, &measure, DRAG.0, DRAG.1);
        report.check(at, &m.name, Inv::I6, result == NudgeResult::Moved, || {
            format!("a drag by {DRAG:?} gave {result:?}")
        });
        if result != NudgeResult::Moved {
            continue;
        }
        let (map2, measured2) = measure_quiet(&moved, layer);
        let Some(index) = measured2.iter().position(|n| n.name == m.name) else {
            report.check(at, &m.name, Inv::I6, false, || "gone after the drag".into());
            continue;
        };
        let after = measured2[index].bounds;
        let expected = expected_delta(&chart_of(cfg, &m.name), &drag.nudge_element());
        match (expected, after) {
            (_, None) => report.check(at, &m.name, Inv::I6, false, || {
                "draws nothing after the drag".into()
            }),
            (Some((dx, dy)), Some(b)) => {
                let want = bounds.translated(dx, dy);
                let worst = [
                    b.x - want.x,
                    b.y - want.y,
                    b.width - want.width,
                    b.height - want.height,
                ]
                .into_iter()
                .map(f32::abs)
                .fold(0.0, f32::max);
                report.check(at, &m.name, Inv::I6, worst <= 0.05, || {
                    format!("bounds {b:?} after the drag, {want:?} expected ({worst:.2} px off)")
                });
            }
            (None, Some(b)) => report.check(at, &m.name, Inv::I6, b != bounds, || {
                "the polar drag did not move it".into()
            }),
        }
        // I2 on the new ink, for the dragged element.
        let ink2 = match SceneInk::reusing(&moved, codes, Some(ink)) {
            Ok(ink2) => ink2,
            Err(e) => {
                report.check(at, &m.name, Inv::I6, false, || e);
                continue;
            }
        };
        let masks2: Vec<Mask> = measured2.iter().map(|n| Mask::of(n, &ink2)).collect();
        report.redirect = Some((Inv::I6, m.name.clone()));
        check_i2(
            report,
            at,
            &moved,
            &measure,
            &map2,
            &measured2,
            &masks2,
            &ink2,
            Some(index),
        );
        report.redirect = None;
        // The hit test names it on its moved ink, where its region is on top.
        let Some(code) = ink2.code_of(&m.name) else {
            continue;
        };
        let topmost: Vec<u32> = ink2.exact[code]
            .iter()
            .copied()
            .filter(|&i| ink2.alpha(code, i) >= STRONG_ALPHA)
            .filter(|&i| {
                let (x, y) = (i64::from(i) % ink2.w as i64, i64::from(i) / ink2.w as i64);
                masks2[index].get(x, y) && !masks2[index + 1..].iter().any(|t| t.get(x, y))
            })
            .collect();
        report.check(at, &m.name, Inv::I6, !topmost.is_empty(), || {
            "no pixel of its moved ink is on top of the hit map".into()
        });
        let stride = topmost.len().div_ceil(DRAG_HIT_SAMPLES).max(1);
        let id2 = measured2[index].id;
        let mut wrong = Vec::new();
        for &i in topmost.iter().step_by(stride) {
            let (cx, cy) = ink2.point(i as usize % ink2.w, i as usize / ink2.w);
            let hit = map2.hit_test(&moved, &measure, cx, cy);
            if hit != Some(id2) {
                wrong.push((
                    (cx, cy),
                    hit.and_then(|h| map2.get(h)).map(|e| e.element_id()),
                ));
            }
        }
        report.check(at, &m.name, Inv::I6, wrong.is_empty(), || {
            format!(
                "the hit test on its moved ink names another element at {} samples, first {:?}",
                wrong.len(),
                wrong[0]
            )
        });
    }
}

// ---------------------------------------------------------------------------
// Bounded families
// ---------------------------------------------------------------------------
//
// A bounded chart (radial, box plot, radar) draws its
// decorations into its own GPU frame. The box plot and radar
// frames register each as one renderer-measured box in their
// `DecorationMap`; the radial frame registers each in its
// `HitMap<RadialChart>`, placed from the chart by the one emit its raster
// draws with, so an outside label is its text and its leader line, outlined
// and hit part by part. Ink is measured by drag differential: an element
// dragged beyond its own extent leaves its old place and shows at its new
// one, and every pixel that changes must lie in what the element's and its
// followers' selections outline, old or new.

/// The logical canvas of every bounded scene.
const BOUNDED_SIZE: (u32, u32) = (800, 600);
/// A channel difference above this is a changed pixel of a bounded export;
/// at or above [`STRONG_DIFF`] it is strong ink.
const CHANGED_DIFF: u8 = 32;
const STRONG_DIFF: u8 = 128;
/// A box-filtered 2x export may differ from the 1x export by more than
/// [`SCALE_DIFF`] at this share of its pixels (`scale_consistency`).
const BOUNDED_SCALE_SHARE: f64 = 0.004;

/// One bounded family's renderer, behind one face for the sweep. Its frame
/// answers the decoration queries of `bounded_decorations::Decorations`:
/// the box plot and radar frames through their
/// `DecorationMap`, the radial frame through its `HitMap<RadialChart>`.
trait Bounded {
    type Chart: Clone + PartialEq;
    type Frame: Decorations<Chart = Self::Chart>;
    fn prepare(&mut self, c: &Self::Chart, scale: f32) -> Result<Arc<Self::Frame>, String>;
    fn export(&mut self, c: &Self::Chart, scale: f32) -> Result<RasterImage, String>;
}

macro_rules! bounded_family {
    ($family:ident, $renderer:ty, $chart:ty, $frame:ty) => {
        struct $family($renderer);
        impl Bounded for $family {
            type Chart = $chart;
            type Frame = $frame;
            fn prepare(&mut self, c: &$chart, scale: f32) -> Result<Arc<$frame>, String> {
                self.0
                    .prepare(c, BOUNDED_SIZE, scale)
                    .map_err(|e| e.to_string())
            }
            fn export(&mut self, c: &$chart, scale: f32) -> Result<RasterImage, String> {
                let image = self
                    .0
                    .export_rgba(c, BOUNDED_SIZE, scale)
                    .map_err(|e| e.to_string());
                self.0.end_frame();
                image
            }
        }
    };
}

bounded_family!(RadialFamily, RadialRenderer, RadialChart, RadialFrame);
bounded_family!(BoxFamily, BoxPlotRenderer, BoxPlotChart, BoxPlotFrame);
bounded_family!(RadarFamily, RadarRenderer, RadarChart, RadarFrame);

fn layout_of<B: Bounded>(c: &mut B::Chart) -> &mut DecorationLayout {
    <B::Frame as Decorations>::layout(c)
}

fn bounded_scene(report: &mut Report, gpu: &RendererDevice, kind: SceneKind) {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let scene = scene_name(kind);
    let failed = |report: &mut Report, e: String| {
        let at = At {
            scene,
            layer: "base",
            scale: 1.0,
        };
        report.check(&at, "<renderer>", Inv::I7, false, || e);
    };
    match kind {
        SceneKind::Radial => match RadialRenderer::new(gpu.clone(), format) {
            Ok(r) => sweep_bounded(
                report,
                scene,
                RadialFamily(r),
                scenes::bounded::radial_chart(),
            ),
            Err(e) => failed(report, e.to_string()),
        },
        SceneKind::BoxPlot => match BoxPlotRenderer::new(gpu.clone(), format) {
            Ok(r) => sweep_bounded(report, scene, BoxFamily(r), scenes::bounded::box_chart()),
            Err(e) => failed(report, e.to_string()),
        },
        SceneKind::Radar => match RadarRenderer::new(gpu.clone(), format) {
            Ok(r) => sweep_bounded(
                report,
                scene,
                RadarFamily(r),
                scenes::bounded::radar_chart(),
            ),
            Err(e) => failed(report, e.to_string()),
        },
        _ => {}
    }
}

/// One element of a bounded frame.
struct BoundedEntry {
    id: HitId,
    key: DecorationId,
    name: String,
    bounds: Option<RectF>,
    /// What a drag moves: the element's text for a radial label, whose
    /// leader line keeps its start on the slice; its bounds otherwise.
    text: Option<RectF>,
    selection: Option<SelectionBox>,
    followers: Vec<DecorationId>,
}

fn bounded_entries<F: Decorations>(f: &F, c: &F::Chart) -> Vec<BoundedEntry> {
    f.entries(c)
        .into_iter()
        .map(|(id, key)| BoundedEntry {
            id,
            name: key.element_id(),
            bounds: f.bounds(c, id),
            text: f.text_bounds(c, id),
            selection: f.selection(c, id),
            followers: f.followers(c, id),
            key,
        })
        .collect()
}

fn entry_of<'a>(entries: &'a [BoundedEntry], key: &DecorationId) -> Option<&'a BoundedEntry> {
    entries.iter().find(|e| e.key == *key)
}

/// Whether `(x, y)` is in the region `sb` outlines: its padded box, or the
/// outline of one of its parts.
fn outlined(sb: &SelectionBox, x: f32, y: f32) -> bool {
    match sb.outline() {
        SelectionOutline::Rect => sb.rect().contains(x, y),
        SelectionOutline::Parts(parts) => parts.iter().any(|p| part_outline(p).contains(x, y)),
    }
}

/// [`outlined`] grown by `pad` px: where weak antialiasing ink may lie.
fn outlined_near(sb: &SelectionBox, x: f32, y: f32, pad: f32) -> bool {
    match sb.outline() {
        SelectionOutline::Rect => sb.rect().expanded(pad).contains(x, y),
        SelectionOutline::Parts(_) => {
            outlined(sb, x, y)
                || outline_edges(sb)
                    .iter()
                    .any(|(a, b)| segment_distance((x, y), *a, *b) <= pad)
        }
    }
}

/// Equal up to float noise: a radial decoration is placed again from its
/// offset rather than translated as a box.
fn near_rect(a: Option<RectF>, b: Option<RectF>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => [a.x - b.x, a.y - b.y, a.width - b.width, a.height - b.height]
            .into_iter()
            .all(|d| d.abs() <= EPS),
        (a, b) => a.is_none() && b.is_none(),
    }
}

/// The largest channel difference of pixel `i` between two images.
fn pixel_diff(a: &RasterImage, b: &RasterImage, i: usize) -> u8 {
    (0..4)
        .map(|k| a.rgba[i * 4 + k].abs_diff(b.rgba[i * 4 + k]))
        .max()
        .unwrap_or(0)
}

fn sweep_bounded<B: Bounded>(report: &mut Report, scene: &'static str, mut r: B, chart: B::Chart) {
    let mut per_scale: Vec<(Vec<BoundedEntry>, RasterImage)> = Vec::new();
    for scale in SCALES {
        let at = At {
            scene,
            layer: "base",
            scale,
        };
        let phase = Instant::now();
        let mut c = chart.clone();
        *layout_of::<B>(&mut c) = DecorationLayout::default();
        let (frame, base) = match (r.prepare(&c, scale), r.export(&c, scale)) {
            (Ok(f), Ok(b)) => (f, b),
            (Err(e), _) | (_, Err(e)) => {
                report.check(&at, "<frame>", Inv::I7, false, || e);
                continue;
            }
        };
        let entries = bounded_entries(&*frame, &c);
        report.count(&at, "bounded elements", entries.len());
        // I7: the hit map holds exactly the frame's entries.
        report.check(&at, "<map>", Inv::I7, frame.len() == entries.len(), || {
            format!(
                "hit map holds {} entries, the frame {}",
                frame.len(),
                entries.len()
            )
        });
        for e in &entries {
            let found = frame.find(&e.name);
            report.check(&at, &e.name, Inv::I7, found == Some(e.id), || {
                format!("find({:?}) is {found:?}, the entry {:?}", e.name, e.id)
            });
            // I1: inside the canvas.
            if let Some(b) = e.bounds {
                let canvas = RectF {
                    x: 0.0,
                    y: 0.0,
                    width: BOUNDED_SIZE.0 as f32,
                    height: BOUNDED_SIZE.1 as f32,
                };
                report.check(&at, &e.name, Inv::I1, inside(canvas, b), || {
                    format!("bounds {b:?} leave the {BOUNDED_SIZE:?} canvas")
                });
            }
        }
        check_bounded_i4(report, &at, &entries);
        if scale == 1.0 {
            bounded_ink(report, &at, &mut r, &c, &*frame, &entries, &base);
            report.time("bounded ink and drags (I2, I6, I7)", phase);
            let phase = Instant::now();
            bounded_selection(report, &at, &mut r, &c, &entries, &base);
            report.time("bounded highlights (I3)", phase);
        } else {
            report.time("bounded 2x", phase);
        }
        per_scale.push((entries, base));
    }
    if let [(one, base1), (two, base2)] = &per_scale[..] {
        let at = At {
            scene,
            layer: "base",
            scale: 2.0,
        };
        for e in one {
            let other = entry_of(two, &e.key);
            report.check(&at, &e.name, Inv::I5, other.is_some(), || {
                "drawn at 1x only".into()
            });
            if let (Some(b1), Some(b2)) = (e.bounds, other.and_then(|o| o.bounds)) {
                let worst = [
                    b2.x - b1.x,
                    b2.y - b1.y,
                    b2.width - b1.width,
                    b2.height - b1.height,
                ]
                .into_iter()
                .map(f32::abs)
                .fold(0.0, f32::max);
                report.check(&at, &e.name, Inv::I5, worst <= 1.0, || {
                    format!("bounds {b2:?} at 2x are {worst:.2} px off {b1:?} at 1x")
                });
            }
        }
        for e in two {
            report.check(
                &at,
                &e.name,
                Inv::I5,
                entry_of(one, &e.key).is_some(),
                || "drawn at 2x only".into(),
            );
        }
        // The 2x export box-filtered back to 1x.
        let (w, h) = (base1.width as usize, base1.height as usize);
        let mut differ = 0usize;
        for y in 0..h {
            for x in 0..w {
                let mut worst = 0;
                for k in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let j = ((y * 2 + dy) * base2.width as usize + x * 2 + dx) * 4 + k;
                        sum += u32::from(base2.rgba[j]);
                    }
                    let one = u32::from(base1.rgba[(y * w + x) * 4 + k]);
                    worst = worst.max((sum / 4).abs_diff(one));
                }
                if worst as i32 > SCALE_DIFF {
                    differ += 1;
                }
            }
        }
        let share = differ as f64 / (w * h) as f64;
        report.check(&at, "<export>", Inv::I5, share < BOUNDED_SCALE_SHARE, || {
            format!(
                "{differ} pixels ({:.2} %) of the 2x export box-filtered differ by more than {SCALE_DIFF} from 1x",
                share * 100.0
            )
        });
    }
}

/// I4 for a bounded frame, on the element bounds: the chart title against
/// every other element, the legend against the bottom labels and title,
/// the spoke labels pairwise and the radar value labels against them.
fn check_bounded_i4(report: &mut Report, at: &At<'_>, entries: &[BoundedEntry]) {
    let is = |e: &BoundedEntry, f: fn(&DecorationId) -> bool| f(&e.key);
    let mut pairs: Vec<(&BoundedEntry, &BoundedEntry)> = Vec::new();
    for (i, a) in entries.iter().enumerate() {
        for b in &entries[i + 1..] {
            let title = |k: &DecorationId| *k == DecorationId::ChartTitle;
            let legend = |k: &DecorationId| *k == DecorationId::Legend;
            let bottom = |k: &DecorationId| {
                matches!(
                    k,
                    DecorationId::AxisLabel(Side::Bottom) | DecorationId::AxisTitle(Side::Bottom)
                )
            };
            let spoke = |k: &DecorationId| matches!(k, DecorationId::SpokeLabel { .. });
            let value = |k: &DecorationId| matches!(k, DecorationId::DataLabel { .. });
            let data_area = |k: &DecorationId| *k == DecorationId::DataArea;
            let pair = (is(a, title) || is(b, title)) && !is(a, data_area) && !is(b, data_area)
                || (is(a, legend) && is(b, bottom))
                || (is(b, legend) && is(a, bottom))
                || (is(a, spoke) && is(b, spoke))
                || (is(a, spoke) && is(b, value))
                || (is(a, value) && is(b, spoke));
            if pair {
                pairs.push((a, b));
            }
        }
    }
    for (a, b) in pairs {
        let (Some(ra), Some(rb)) = (a.bounds, b.bounds) else {
            continue;
        };
        report.check(
            at,
            &format!("{} x {}", a.name, b.name),
            Inv::I4,
            !intersects(ra, rb, OVERLAP_TOLERANCE),
            || format!("{ra:?} and {rb:?} overlap"),
        );
    }
}

/// I2, I6 and I7 (ghosts) of a bounded frame at 1x, by drag differential.
fn bounded_ink<B: Bounded>(
    report: &mut Report,
    at: &At<'_>,
    r: &mut B,
    c: &B::Chart,
    frame: &B::Frame,
    entries: &[BoundedEntry],
    base: &RasterImage,
) {
    let (w, h) = (base.width as usize, base.height as usize);
    let point = |i: usize| ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
    // Each element's ink at its old place, for the topmost check.
    let mut inked: Vec<Vec<bool>> = vec![Vec::new(); entries.len()];
    let mut strong_old: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
    for (index, e) in entries.iter().enumerate() {
        let (Some(b), Some(sb)) = (e.bounds, e.selection.as_ref()) else {
            continue;
        };
        // `contains` against the region the selection outlines, on its
        // edge band.
        let rect = sb.rect();
        let win = (
            (rect.x - 3.0).floor().max(0.0) as u32,
            (rect.y - 3.0).floor().max(0.0) as u32,
            ((rect.x + rect.width + 3.0).ceil() as u32).min(w as u32),
            ((rect.y + rect.height + 3.0).ceil() as u32).min(h as u32),
        );
        let band = edge_band::edge_band(win, |x, y| outlined(sb, x as f32 + 0.5, y as f32 + 0.5));
        let stride = band.len().div_ceil(CONTAINS_SAMPLES).max(1);
        let disagree = band
            .iter()
            .step_by(stride)
            .filter(|&&(x, y)| {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                frame.contains(c, e.id, [px, py]) != outlined(sb, px, py)
            })
            .count();
        report.count(at, "contains samples", band.len().div_ceil(stride));
        report.check(at, &e.name, Inv::I2, disagree == 0, || {
            format!("contains disagrees with the outlined region at {disagree} samples")
        });
        if e.key == DecorationId::DataArea {
            // Paints nothing of its own; moving it moves the data.
            continue;
        }
        // Drag what moves beyond its own extent, inside the canvas.
        let t = e.text.unwrap_or(b);
        let (ox, oy) = (t.width + 6.0, t.height + 6.0);
        // A part that keeps its start where it is (an outside radial label's
        // leader line, from the slice to the text) must swing across its
        // old place, not slide along it: such an element is dragged
        // vertically first.
        let parts = matches!(sb.outline(), SelectionOutline::Parts(_));
        let candidates: Vec<(f32, f32)> = match e.key {
            DecorationId::Axis(Side::Top | Side::Bottom) => vec![(0.0, oy), (0.0, -oy)],
            DecorationId::Axis(_) => vec![(ox, 0.0), (-ox, 0.0)],
            _ if parts => vec![(0.0, oy), (0.0, -oy), (ox, 0.0), (-ox, 0.0)],
            _ => vec![(ox, 0.0), (-ox, 0.0), (0.0, oy), (0.0, -oy)],
        };
        let Some((moved, (dx, dy))) = candidates.into_iter().find_map(|(dx, dy)| {
            let mut trial = c.clone();
            (frame.drag(&mut trial, e.id, dx, dy) == Some(NudgeResult::Moved))
                .then_some((trial, (dx, dy)))
        }) else {
            report.check(at, &e.name, Inv::I2, false, || {
                "cannot be dragged off its own extent to measure its ink".into()
            });
            continue;
        };
        let (frame2, image) = match (r.prepare(&moved, 1.0), r.export(&moved, 1.0)) {
            (Ok(f), Ok(i)) => (f, i),
            (Err(e2), _) | (_, Err(e2)) => {
                report.check(at, &e.name, Inv::I2, false, || e2);
                continue;
            }
        };
        report.count(at, "differential exports", 1);
        let after = bounded_entries(&*frame2, &moved);
        // Where the element and its followers may change pixels: what their
        // selections outline before and after.
        let mut allowed: Vec<&SelectionBox> = vec![sb];
        let new_box = entry_of(&after, &e.key).and_then(|n| n.selection.as_ref());
        allowed.extend(new_box);
        for f in &e.followers {
            allowed.extend(entry_of(entries, f).and_then(|o| o.selection.as_ref()));
            allowed.extend(entry_of(&after, f).and_then(|o| o.selection.as_ref()));
        }
        let mut stray = Vec::new();
        let mut old_ink = Vec::new();
        let mut new_ink = 0usize;
        let mut mask = vec![false; w * h];
        for i in 0..w * h {
            let d = pixel_diff(base, &image, i);
            if d <= CHANGED_DIFF {
                continue;
            }
            let (px, py) = point(i);
            let strong = d >= STRONG_DIFF;
            let ok = allowed.iter().any(|a| {
                if strong {
                    outlined(a, px, py)
                } else {
                    outlined_near(a, px, py, 1.0)
                }
            });
            if !ok {
                stray.push((px, py));
            }
            if outlined(sb, px, py) {
                old_ink.push(i);
                mask[i] = true;
                if strong {
                    strong_old[index].push(i);
                }
            } else if new_box.is_some_and(|n| outlined(n, px, py)) {
                new_ink += 1;
            }
        }
        report.check(at, &e.name, Inv::I2, stray.is_empty(), || {
            format!(
                "dragging it by ({dx:.0}, {dy:.0}) changes {} pixels outside its and its followers' hit regions, first {:?}",
                stray.len(),
                stray[0]
            )
        });
        report.check(at, &e.name, Inv::I7, !old_ink.is_empty(), || {
            format!("ghost: bounds {b:?} but no ink")
        });
        report.check(at, &e.name, Inv::I6, new_ink > 0, || {
            format!("dragged by ({dx:.0}, {dy:.0}) it is not drawn at its new place")
        });
        let ink_extent = |pixels: &mut dyn Iterator<Item = usize>| {
            pixels
                .map(|i| {
                    let (x, y) = (i % w, i / w);
                    RectF {
                        x: x as f32,
                        y: y as f32,
                        width: 1.0,
                        height: 1.0,
                    }
                })
                .reduce(renderer::sink::union)
        };
        match sb.outline() {
            SelectionOutline::Rect => {
                if let Some(ext) = ink_extent(&mut old_ink.iter().copied()) {
                    let misfit = box_misfit(sb.rect(), ext);
                    report.check(at, &e.name, Inv::I2, misfit <= BOX_FIT, || {
                        format!(
                            "box {:?} is {misfit:.1} px past the padding from the ink extent {ext:?}",
                            sb.rect()
                        )
                    });
                }
            }
            SelectionOutline::Parts(parts) => {
                // A text part's box fits the ink inside it; a stroke part
                // has ink inside its outline.
                for part in parts {
                    let outline = part_outline(part);
                    let mut inside = old_ink.iter().copied().filter(|&i| {
                        let (px, py) = point(i);
                        outline.contains(px, py)
                    });
                    match *part {
                        DrawnPart::Text(t) => {
                            let o = t.expanded(SELECTION_PADDING);
                            match ink_extent(&mut inside) {
                                Some(ext) => {
                                    let misfit = box_misfit(o, ext);
                                    report.check(at, &e.name, Inv::I2, misfit <= BOX_FIT, || {
                                        format!(
                                            "text part {o:?} is {misfit:.1} px past the padding from its ink extent {ext:?}"
                                        )
                                    });
                                }
                                None => report.check(at, &e.name, Inv::I2, false, || {
                                    format!("text part {t:?} has no ink")
                                }),
                            }
                        }
                        _ => {
                            let inked_part = inside.next().is_some();
                            report.check(at, &e.name, Inv::I2, inked_part, || {
                                format!("part {part:?} has no ink inside its outline")
                            });
                        }
                    }
                }
            }
        }
        report.count(at, "ink pixels", old_ink.len());
        inked[index] = mask;
    }
    // The topmost region at each strong ink pixel has ink nearby.
    for (index, e) in entries.iter().enumerate() {
        let mut covered: BTreeMap<String, usize> = BTreeMap::new();
        let mut first = None;
        for &i in &strong_old[index] {
            let (px, py) = point(i);
            let Some(hit) = frame.hit(c, [px, py]) else {
                *covered.entry("<nothing>".into()).or_default() += 1;
                first.get_or_insert((px, py));
                continue;
            };
            let Some(t) = entries.iter().position(|o| o.id == hit) else {
                continue;
            };
            if t == index {
                continue;
            }
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            let reach = TOPMOST_REACH;
            let near = !inked[t].is_empty()
                && (y - reach..=y + reach).any(|ny| {
                    (x - reach..=x + reach).any(|nx| {
                        nx >= 0
                            && ny >= 0
                            && (nx as usize) < w
                            && (ny as usize) < h
                            && inked[t][ny as usize * w + nx as usize]
                    })
                });
            if !near {
                *covered.entry(entries[t].name.clone()).or_default() += 1;
                first.get_or_insert((px, py));
            }
        }
        report.check(at, &e.name, Inv::I2, covered.is_empty(), || {
            format!(
                "{} of {} strong ink pixels hit another element that has no ink there ({covered:?}), first {:?}",
                covered.values().sum::<usize>(),
                strong_old[index].len(),
                first
            )
        });
    }
    // I6: a small drag moves it and its followers, the moved element still
    // picks, a drag off the canvas is refused without a change.
    for e in entries {
        let Some(b) = e.bounds else {
            continue;
        };
        report.count(at, "drags", 1);
        let mut far = c.clone();
        let result = frame.drag(&mut far, e.id, FAR_DRAG.0, FAR_DRAG.1);
        report.check(
            at,
            &e.name,
            Inv::I6,
            matches!(result, Some(NudgeResult::Rejected(_))) && far == *c,
            || format!("a drag by {FAR_DRAG:?} gave {result:?}"),
        );
        // The data area fills its automatic region; it moves once shrunk.
        let mut moved = c.clone();
        let mut start = e.text.unwrap_or(b);
        if e.key == DecorationId::DataArea {
            let shrunk = frame.resize(&mut moved, e.id, ResizeHandle::SE, -40.0, -30.0);
            report.check(
                at,
                &e.name,
                Inv::I6,
                shrunk == Some(NudgeResult::Moved),
                || format!("shrinking it before the move gave {shrunk:?}"),
            );
            let Ok(frame) = r.prepare(&moved, 1.0) else {
                continue;
            };
            let Some(entry) = bounded_entries(&*frame, &moved)
                .into_iter()
                .find(|n| n.key == e.key)
            else {
                continue;
            };
            start = entry.bounds.unwrap_or(b);
            let result = frame.drag(&mut moved, entry.id, DRAG.0, DRAG.1);
            report.check(
                at,
                &e.name,
                Inv::I6,
                result == Some(NudgeResult::Moved),
                || format!("a drag by {DRAG:?} gave {result:?}"),
            );
        } else {
            let result = frame.drag(&mut moved, e.id, DRAG.0, DRAG.1);
            report.check(
                at,
                &e.name,
                Inv::I6,
                result == Some(NudgeResult::Moved),
                || format!("a drag by {DRAG:?} gave {result:?}"),
            );
            if result != Some(NudgeResult::Moved) {
                continue;
            }
        }
        let (dx, dy) = match e.key {
            DecorationId::Axis(Side::Top | Side::Bottom) => (0.0, DRAG.1),
            DecorationId::Axis(_) => (DRAG.0, 0.0),
            _ => DRAG,
        };
        let frame = match r.prepare(&moved, 1.0) {
            Ok(f) => f,
            Err(e2) => {
                report.check(at, &e.name, Inv::I6, false, || e2);
                continue;
            }
        };
        let after = bounded_entries(&*frame, &moved);
        let Some(n) = entry_of(&after, &e.key) else {
            report.check(at, &e.name, Inv::I6, false, || "gone after the drag".into());
            continue;
        };
        let want = start.translated(dx, dy);
        report.check(at, &e.name, Inv::I6, near_rect(n.text, Some(want)), || {
            format!("{:?} after the drag, {want:?} expected", n.text)
        });
        for f in &e.followers {
            let (before, after) = (
                entry_of(entries, f).and_then(|o| o.bounds),
                entry_of(&after, f).and_then(|o| o.bounds),
            );
            report.check(
                at,
                &e.name,
                Inv::I6,
                near_rect(after, before.map(|r| r.translated(dx, dy))),
                || {
                    format!(
                        "follower {f:?} at {after:?}, {before:?} moved by ({dx}, {dy}) expected"
                    )
                },
            );
        }
        // The moved element picks somewhere on what moved.
        let picks = n.text.is_some_and(|nb| {
            std::iter::once((4, 4))
                .chain((0..=8).flat_map(|j| (0..=8).map(move |i| (i, j))))
                .any(|(i, j)| {
                    let p = [
                        nb.x + nb.width * i as f32 / 8.0,
                        nb.y + nb.height * j as f32 / 8.0,
                    ];
                    frame.hit(&moved, p) == Some(n.id)
                })
        });
        report.check(at, &e.name, Inv::I6, picks, || {
            "no point of its moved box picks it".into()
        });
    }
}

/// I3 of a bounded frame at 1x: selecting each element changes only its
/// outline and handles (`bounded_selection::assert_selection_only`, which
/// takes a box or the outlines of its parts).
fn bounded_selection<B: Bounded>(
    report: &mut Report,
    at: &At<'_>,
    r: &mut B,
    c: &B::Chart,
    entries: &[BoundedEntry],
    base: &RasterImage,
) {
    for e in entries {
        let Some(sb) = &e.selection else {
            continue;
        };
        let mut selected = c.clone();
        layout_of::<B>(&mut selected).selected = Some(e.key.clone());
        let image = match r.export(&selected, 1.0) {
            Ok(i) => i,
            Err(e2) => {
                report.check(at, &e.name, Inv::I3, false, || e2);
                continue;
            }
        };
        report.count(at, "selection exports", 1);
        let context = format!("{} {}", at.scene, e.name);
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            bounded_selection::assert_selection_only(base, &image, sb, 1.0, &context)
        }));
        std::panic::set_hook(hook);
        report.check(at, &e.name, Inv::I3, outcome.is_ok(), || {
            let payload = outcome.as_ref().err();
            payload
                .and_then(|p| p.downcast_ref::<String>().cloned())
                .or_else(|| payload.and_then(|p| p.downcast_ref::<&str>().map(|s| s.to_string())))
                .unwrap_or_else(|| "assert_selection_only failed".into())
        });
    }
}

// ---------------------------------------------------------------------------
// The annotation layer
// ---------------------------------------------------------------------------
//
// The annotation layer is rastered into its own textures, so its ink is
// measured on the product's exports: the document with every item against
// the document with one item hidden (`Annotation::visible`). Its hit map is
// the renderer-measured `AnnotationMap` of a `ChartView`; its highlight is
// only drawn on screen, so I3 paints the view into an offscreen target.

/// One scale's annotation geometry, for I5.
type AnnotationGeometry = Vec<(String, Option<RectF>)>;

fn annotation_name(index: usize) -> String {
    DecorationId::Annotation(index).element_id()
}

/// The export of `config` at `scale` with item `hidden` (or the whole
/// layer, for `usize::MAX`) not drawn.
fn export_annotations(
    r: &mut Renderer,
    config: &Config,
    series: &[SeriesConfig],
    scale: f32,
    hidden: Option<usize>,
) -> Result<RasterImage, String> {
    let mut doc = config.clone();
    match hidden {
        Some(usize::MAX) => doc.annotations.visible = false,
        Some(i) => doc.annotations.items[i].visible = false,
        None => {}
    }
    r.export_document_rgba(&Chart::new(doc), series, scale)
        .map_err(|e| e.to_string())
}

/// The annotation map of `cfg` (already at its scale), from a chart view.
fn annotation_view(r: &Renderer, cfg: &Config) -> Result<renderer::ChartView, String> {
    r.create_chart_view(&Chart::new(cfg.clone()), cfg.chart_area.0)
        .map_err(|e| e.to_string())
}

/// The ink of every item as a one-panel [`SceneInk`]: item `k` owns the
/// pixels its pair of exports (every item, and every item but `k`)
/// differ in.
fn items_ink(cfg: &Config, pairs: &[(&RasterImage, &RasterImage)]) -> SceneInk {
    let (w, h) = (pairs[0].0.width as usize, pairs[0].0.height as usize);
    let mut owner = vec![0u16; w * h];
    let mut owner2 = vec![0u16; w * h];
    let mut rgba = vec![0u8; w * h * 4];
    for i in 0..w * h {
        for (k, (all, image)) in pairs.iter().enumerate() {
            let d = pixel_diff(all, image, i);
            if d <= CHANGED_DIFF {
                continue;
            }
            rgba[i * 4 + 3] = rgba[i * 4 + 3].max(d);
            if owner[i] == 0 {
                owner[i] = k as u16 + 1;
            } else if owner2[i] == 0 {
                owner2[i] = k as u16 + 1;
            }
        }
    }
    let mut by_code = vec![Vec::new(); pairs.len()];
    let mut exact = vec![Vec::new(); pairs.len()];
    for i in 0..w * h {
        if owner[i] != 0 {
            by_code[usize::from(owner[i]) - 1].push(i as u32);
            match owner2[i] {
                0 => exact[usize::from(owner[i]) - 1].push(i as u32),
                o2 => by_code[usize::from(o2) - 1].push(i as u32),
            }
        }
    }
    let area = cfg.document_area();
    SceneInk {
        origin: [area.x as f32, area.y as f32],
        w,
        h,
        codes: (0..pairs.len())
            .map(|k| ([0, 0, 0], annotation_name(k), 0))
            .collect(),
        panels: vec![PanelInk {
            layer: LayerRef::Annotations,
            config: cfg.clone(),
            x0: 0,
            y0: 0,
            w,
            h,
            rgba,
            owner,
            owner2,
        }],
        by_code,
        exact,
    }
}

/// Every item of an annotation map, measured.
fn measure_items(map: &renderer::AnnotationMap) -> Vec<Measured> {
    let measure = CpuTextMeasure::default();
    map.entries()
        .map(|(id, e)| {
            let mut parts = Vec::new();
            if let Some(element) = map.hit_map().get(id) {
                element.emit(map.layer(), &measure, &mut Parts(&mut |p| parts.push(p)));
            }
            Measured {
                name: annotation_name(e.index),
                id,
                bounds: map.bounds(id),
                parts,
                selection: map.selection_box(id),
            }
        })
        .collect()
}

/// The annotation layer of the `Annotations` layer case at `cfg`'s scale:
/// I1 (parts inside the chart area), I2 on the items' export ink, I7 (an
/// entry per visible item, no ghost, no ink of no item) and, at 1x, I3 and
/// I6. Decoration ink under an item's region without item ink is an I2 row
/// of the decoration element. Returns the items' bounds for I5.
#[allow(clippy::too_many_arguments)]
fn check_annotations(
    report: &mut Report,
    at: &At<'_>,
    r: &mut Renderer,
    config: &Config,
    series: &[SeriesConfig],
    cfg: &Config,
    decorations: &[Measured],
    palette_ink: &SceneInk,
) -> AnnotationGeometry {
    let phase = Instant::now();
    let items = config.annotations.items.len();
    let view = match annotation_view(r, cfg) {
        Ok(view) => view,
        Err(e) => {
            report.check(at, "<annotations>", Inv::I7, false, || e);
            return Vec::new();
        }
    };
    let map = view.annotation_map();
    let measured = measure_items(map);
    report.check(
        at,
        "<annotations>",
        Inv::I7,
        map.hit_map().len() == items,
        || format!("{} entries for {items} visible items", map.hit_map().len()),
    );
    for index in 0..items {
        let found = map.find(index);
        report.check(
            at,
            &annotation_name(index),
            Inv::I7,
            found.is_some(),
            || "the visible item is not registered".into(),
        );
        if let Some(id) = found {
            let name = map.hit_map().get(id).map(|e| e.element_id());
            report.check(
                at,
                &annotation_name(index),
                Inv::I7,
                name.as_deref() == Some(annotation_name(index).as_str()),
                || format!("registered as {name:?}"),
            );
        }
    }
    let area = RectF::from_rect(&cfg.chart_area.0);
    for m in &measured {
        let outside: Vec<RectF> = m
            .parts
            .iter()
            .map(DrawnPart::extent)
            .filter(|e| !inside(area, *e))
            .collect();
        report.check(at, &m.name, Inv::I1, outside.is_empty(), || {
            format!(
                "{} parts leave the chart area, first {:?}",
                outside.len(),
                outside[0]
            )
        });
    }
    // The items' ink on the product's export.
    let exports: Result<Vec<RasterImage>, String> = std::iter::once(None)
        .chain(std::iter::once(Some(usize::MAX)))
        .chain((0..items).map(Some))
        .map(|hidden| export_annotations(r, config, series, at.scale, hidden))
        .collect();
    let exports = match exports {
        Ok(e) => e,
        Err(e) => {
            report.check(at, "<annotations>", Inv::I7, false, || e);
            return Vec::new();
        }
    };
    report.count(at, "annotation exports", exports.len());
    let (all, none) = (&exports[0], &exports[1]);
    let pairs: Vec<(&RasterImage, &RasterImage)> = exports[2..].iter().map(|w| (all, w)).collect();
    let ink = items_ink(cfg, &pairs);
    let masks: Vec<Mask> = measured.iter().map(|m| Mask::of(m, &ink)).collect();
    check_i2(
        report,
        at,
        map.layer(),
        &CpuTextMeasure::default(),
        map.hit_map(),
        &measured,
        &masks,
        &ink,
        None,
    );
    // I7: no ghost item, and no layer ink that is no item's.
    for m in &measured {
        let pixels = ink.code_of(&m.name).map_or(0, |c| ink.by_code[c].len());
        report.check(
            at,
            &m.name,
            Inv::I7,
            m.bounds.is_none() || pixels > 0,
            || format!("ghost: bounds {:?} but no ink", m.bounds),
        );
        report.count(at, "annotation ink pixels", pixels);
    }
    // A pixel two items cover changes with the layer but with neither item
    // alone; only ink outside every item's region is no item's.
    let orphan: Vec<usize> = (0..ink.w * ink.h)
        .filter(|&i| {
            let (x, y) = ((i % ink.w) as i64, (i / ink.w) as i64);
            pixel_diff(all, none, i) > CHANGED_DIFF
                && ink.panels[0].owner[i] == 0
                && !masks.iter().any(|m| m.get(x, y))
        })
        .collect();
    report.check(at, "<annotation layer>", Inv::I7, orphan.is_empty(), || {
        format!(
            "{} pixels the layer draws belong to no item, first {:?}",
            orphan.len(),
            ink.point(orphan[0] % ink.w, orphan[0] / ink.w)
        )
    });
    // The items sit on top of the chart's hit map: decoration ink under an
    // item's region must have item ink nearby.
    for m in decorations {
        let Some(code) = palette_ink.code_of(&m.name) else {
            continue;
        };
        let mut covered: BTreeMap<String, usize> = BTreeMap::new();
        let mut first = None;
        for &i in &palette_ink.exact[code] {
            if palette_ink.alpha(code, i) < STRONG_ALPHA {
                continue;
            }
            let (x, y) = (
                i64::from(i) % palette_ink.w as i64,
                i64::from(i) / palette_ink.w as i64,
            );
            if let Some((t, tm)) = masks
                .iter()
                .zip(&measured)
                .enumerate()
                .rev()
                .find(|(_, (tmask, _))| tmask.get(x, y))
                .map(|(t, (_, tm))| (t, tm))
                && !ink.inked_near(t, x, y, TOPMOST_REACH)
            {
                *covered.entry(tm.name.clone()).or_default() += 1;
                first.get_or_insert(palette_ink.point(x as usize, y as usize));
            }
        }
        report.check(at, &m.name, Inv::I2, covered.is_empty(), || {
            format!(
                "{} strong ink pixels hit an annotation item that has no ink there ({covered:?}), first {first:?}",
                covered.values().sum::<usize>()
            )
        });
    }
    report.time("annotation layer (I1, I2, I7)", phase);
    if at.scale == 1.0 {
        let phase = Instant::now();
        annotation_highlights(report, at, r, cfg, &measured);
        report.time("annotation highlights (I3)", phase);
        let phase = Instant::now();
        annotation_drags(report, at, r, config, series, cfg, map, &measured, &exports);
        report.time("annotation drags (I6)", phase);
    }
    measured
        .iter()
        .map(|m| (m.name.clone(), m.bounds))
        .collect()
}

/// Paint `chart`'s view (no series) with annotation `selected` highlighted
/// into an offscreen target and read it back: the on-screen path, where the
/// annotation highlight is drawn.
fn paint_view(
    r: &mut Renderer,
    cfg: &Config,
    selected: Option<usize>,
) -> Result<RasterImage, String> {
    let e = |e: renderer::FiggyError| e.to_string();
    let chart = Chart::new(cfg.clone());
    let rect = cfg.chart_area.0;
    let mut view = r.create_chart_view(&chart, rect).map_err(e)?;
    r.refresh_annotations(&mut view, &chart, rect, selected)
        .map_err(e)?;
    let items = [renderer::ChartDrawItem {
        view: &view,
        chart_config: chart.config(),
        series: &[],
    }];
    let prepared = r.prepare(&items).map_err(e)?;
    let (w, h) = (rect.x + rect.width, rect.y + rect.height);
    let device = r.device().clone();
    let queue = r.queue().clone();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("decoration oracle target"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target = texture.create_view(&Default::default());
    let row = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("decoration oracle readback"),
        size: u64::from(row) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("decoration oracle paint"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            })
            .forget_lifetime();
        r.paint_prepared(&mut pass, (w, h), &prepared).map_err(e)?;
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    drop(prepared);
    r.end_gpu_frame();
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|e| e.to_string())?;
    let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let start = (y * row) as usize;
        rgba.extend_from_slice(&data[start..start + (w * 4) as usize]);
    }
    drop(data);
    buffer.unmap();
    Ok(RasterImage {
        width: w,
        height: h,
        rgba,
    })
}

/// I3 of the annotation layer at 1x: the highlight of each item, painted
/// through the view as on screen, changes only its outline and handles.
fn annotation_highlights(
    report: &mut Report,
    at: &At<'_>,
    r: &mut Renderer,
    cfg: &Config,
    measured: &[Measured],
) {
    let base = match paint_view(r, cfg, None) {
        Ok(image) => image,
        Err(e) => {
            report.check(at, "<annotation paint>", Inv::I3, false, || e);
            return;
        }
    };
    for (index, m) in measured.iter().enumerate() {
        let Some(sb) = &m.selection else {
            continue;
        };
        let item = m
            .name
            .trim_start_matches("annotation_")
            .parse::<usize>()
            .unwrap_or(index);
        let image = match paint_view(r, cfg, Some(item)) {
            Ok(image) => image,
            Err(e) => {
                report.check(at, &m.name, Inv::I3, false, || e);
                continue;
            }
        };
        report.count(at, "annotation paints", 1);
        let halo = sb.stroke_width() * 0.5 + 1.0;
        let (w, h) = (image.width as usize, image.height as usize);
        let changed: Vec<(f32, f32)> = (0..w * h)
            .filter(|&i| image.rgba[i * 4..i * 4 + 4] != base.rgba[i * 4..i * 4 + 4])
            .map(|i| ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5))
            .collect();
        let stray: Vec<(f32, f32)> = changed
            .iter()
            .copied()
            .filter(|&(x, y)| !on_outline(sb, x, y, halo))
            .collect();
        report.check(at, &m.name, Inv::I3, stray.is_empty(), || {
            format!(
                "{} of {} highlight pixels off the box outline and handles, first {:?}",
                stray.len(),
                changed.len(),
                stray[0]
            )
        });
        let missing = missing_edges(sb, &changed, halo);
        report.check(at, &m.name, Inv::I3, missing.is_empty(), || {
            format!(
                "{} outline edges are not drawn, first {:?}",
                missing.len(),
                missing[0]
            )
        });
    }
}

/// How a (+7, +5) drag moves an item: a reference line or span along its
/// axis, a text box or arrow by the whole delta.
fn annotation_delta(kind: &renderer::AnnotationKind) -> (f32, f32) {
    use renderer::{AnnotationAxis, AnnotationKind};
    match kind {
        AnnotationKind::ReferenceLine { axis, .. } | AnnotationKind::Span { axis, .. } => {
            match axis {
                AnnotationAxis::X => (DRAG.0, 0.0),
                AnnotationAxis::Y => (0.0, DRAG.1),
            }
        }
        _ => DRAG,
    }
}

/// I6 of the annotation layer at 1x: every item drags by (+7, +5), its
/// bounds move by its axis' share of it, its new ink lies in its new
/// region and picks it, and a drag off the chart is refused unchanged.
#[allow(clippy::too_many_arguments)]
fn annotation_drags(
    report: &mut Report,
    at: &At<'_>,
    r: &mut Renderer,
    config: &Config,
    series: &[SeriesConfig],
    cfg: &Config,
    map: &renderer::AnnotationMap,
    measured: &[Measured],
    before: &[RasterImage],
) {
    for m in measured {
        let Some(bounds) = m.bounds else {
            continue;
        };
        let Some(index) = map.element(m.id).map(|e| e.index) else {
            continue;
        };
        report.count(at, "annotation drags", 1);
        let mut far = cfg.annotations.clone();
        let result = map.drag_by(&mut far, m.id, FAR_DRAG.0, FAR_DRAG.1);
        report.check(
            at,
            &m.name,
            Inv::I6,
            matches!(result, Some(NudgeResult::Rejected(_))) && far == cfg.annotations,
            || format!("a drag by {FAR_DRAG:?} gave {result:?}"),
        );
        let mut layer = cfg.annotations.clone();
        let result = map.drag_by(&mut layer, m.id, DRAG.0, DRAG.1);
        report.check(
            at,
            &m.name,
            Inv::I6,
            result == Some(NudgeResult::Moved),
            || format!("a drag by {DRAG:?} gave {result:?}"),
        );
        if result != Some(NudgeResult::Moved) {
            continue;
        }
        let mut moved = cfg.clone();
        moved.annotations = layer;
        let mut moved_1x = config.clone();
        moved_1x.annotations = moved.annotations.clone();
        let view = match annotation_view(r, &moved) {
            Ok(view) => view,
            Err(e) => {
                report.check(at, &m.name, Inv::I6, false, || e);
                continue;
            }
        };
        let map2 = view.annotation_map();
        let measured2 = measure_items(map2);
        let Some(k) = measured2.iter().position(|n| n.name == m.name) else {
            report.check(at, &m.name, Inv::I6, false, || "gone after the drag".into());
            continue;
        };
        let (dx, dy) = annotation_delta(&cfg.annotations.items[index].kind);
        let want = bounds.translated(dx, dy);
        let worst = measured2[k].bounds.map_or(f32::INFINITY, |b| {
            [
                b.x - want.x,
                b.y - want.y,
                b.width - want.width,
                b.height - want.height,
            ]
            .into_iter()
            .map(f32::abs)
            .fold(0.0, f32::max)
        });
        report.check(at, &m.name, Inv::I6, worst <= 0.05, || {
            format!(
                "bounds {:?} after the drag, {want:?} expected",
                measured2[k].bounds
            )
        });
        // Its new ink lies in its new region, and picks it where on top.
        let (Ok(all), Ok(without)) = (
            export_annotations(r, &moved_1x, series, 1.0, None),
            export_annotations(r, &moved_1x, series, 1.0, Some(index)),
        ) else {
            report.check(at, &m.name, Inv::I6, false, || "export failed".into());
            continue;
        };
        report.count(at, "annotation exports", 2);
        // The dragged item's ink from the moved exports, every other
        // item's from the exports before the drag: they did not move.
        let pairs: Vec<(&RasterImage, &RasterImage)> = (0..moved_1x.annotations.items.len())
            .map(|j| {
                if j == index {
                    (&all, &without)
                } else {
                    (&before[0], &before[j + 2])
                }
            })
            .collect();
        let ink2 = items_ink(&moved, &pairs);
        let masks2: Vec<Mask> = measured2.iter().map(|n| Mask::of(n, &ink2)).collect();
        report.redirect = Some((Inv::I6, m.name.clone()));
        check_i2(
            report,
            at,
            map2.layer(),
            &CpuTextMeasure::default(),
            map2.hit_map(),
            &measured2,
            &masks2,
            &ink2,
            Some(k),
        );
        report.redirect = None;
        let code = ink2.code_of(&m.name).unwrap_or(index);
        let id2 = measured2[k].id;
        let topmost: Vec<u32> = ink2.exact[code]
            .iter()
            .copied()
            .filter(|&i| ink2.alpha(code, i) >= STRONG_ALPHA)
            .filter(|&i| {
                let (x, y) = (i64::from(i) % ink2.w as i64, i64::from(i) / ink2.w as i64);
                masks2[k].get(x, y) && !masks2[k + 1..].iter().any(|t| t.get(x, y))
            })
            .collect();
        let stride = topmost.len().div_ceil(DRAG_HIT_SAMPLES).max(1);
        let wrong = topmost
            .iter()
            .step_by(stride)
            .filter(|&&i| {
                let (cx, cy) = ink2.point(i as usize % ink2.w, i as usize / ink2.w);
                map2.hit_test([cx, cy]) != Some(id2)
            })
            .count();
        report.check(at, &m.name, Inv::I6, !topmost.is_empty() && wrong == 0, || {
            format!(
                "the hit test on its moved ink names another item at {wrong} samples ({} on top)",
                topmost.len()
            )
        });
    }
}
