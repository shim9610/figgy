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
//! - **I7** the hit map of every layer holds exactly the elements the
//!   layer's documented registration set names, in that order.
//!
//! The Cartesian decoration ink needs no GPU: the CPU raster of each
//! expanded panel (`try_raster_chart_layer_to_rgba` with
//! `AxisLayerKind::Decoration`) runs the placement pass the renderer
//! uploads, and a 2x display or export is `Config::scaled(2.0)`. The GPU
//! builds the scenes (column uploads, `auto_fit_*`) and renders the bounded
//! families and the annotation layer.
#![cfg(not(target_arch = "wasm32"))]

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

use renderer::layout::RectF;
use renderer::sink::Parts;
use renderer::{
    Config, CpuTextMeasure, DrawnPart, HitId, HitMap, LayerRef, Renderer, RendererDevice,
    SelectionGeometry,
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
/// of `layer` in `scene` pass, for the `reason` and the tracking `issue`
/// given. `"*"` matches any layer or element.
struct Exclusion {
    scene: &'static str,
    layer: &'static str,
    element: &'static str,
    inv: Inv,
    reason: &'static str,
    issue: &'static str,
}

impl Exclusion {
    fn matches(&self, row: &Row) -> bool {
        self.inv == row.inv
            && self.scene == row.scene
            && (self.layer == "*" || self.layer == row.layer)
            && (self.element == "*" || self.element == row.element)
    }
}

/// The known divergences.
const EXCLUSIONS: &[Exclusion] = &[];

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
        let cell = self.matrix.entry((inv, kind_of(element))).or_default();
        cell.0 += 1;
        if !ok {
            cell.1 += 1;
            self.rows.push(Row {
                scene: at.scene,
                layer: at.layer.to_string(),
                element: element.to_string(),
                inv,
                scale: at.scale,
                detail: detail(),
            });
        }
    }

    fn line(&mut self, line: String) {
        self.summary.push(line);
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
            let _ = writeln!(
                out,
                "  {:?} {} / {} / {}: {} ({})",
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
        cases += cartesian_scene(&mut report, scene_name(kind), &scene.config);
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
#[allow(dead_code)] // read by the invariants that follow I1 and I7
struct Measured {
    name: String,
    id: HitId,
    bounds: Option<RectF>,
    parts: Vec<DrawnPart>,
    laid_out: bool,
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
fn cartesian_scene(report: &mut Report, scene: &'static str, config: &Config) -> usize {
    let layers = layer_cases(config);
    let phase = Instant::now();
    for scale in SCALES {
        let doc = if scale == 1.0 {
            config.clone()
        } else {
            config.scaled(scale)
        };
        for &layer in &layers {
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
            let measured = measure_layer(report, &at, &cfg, layer);
            check_i1(report, &at, &cfg, &measured);
        }
    }
    report.time("Cartesian geometry", phase);
    layers.len()
}

/// Build the layer's hit map, check it against the documented registration
/// set (I7) and measure every element.
fn measure_layer(report: &mut Report, at: &At<'_>, cfg: &Config, layer: LayerRef) -> Vec<Measured> {
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
        measured.push(Measured {
            name: name.clone(),
            id,
            bounds,
            parts,
            laid_out,
        });
    }
    measured
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
