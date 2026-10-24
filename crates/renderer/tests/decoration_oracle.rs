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
//! - **I7** the hit map of every layer holds exactly the elements the
//!   layer's documented registration set names, in that order; no ink
//!   without an element, no element without ink; and the CPU decoration
//!   raster is what the product exports.
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

use renderer::axis_render::{
    AxisLayerKind, try_raster_chart_layer_to_rgba, try_raster_chart_layer_to_rgba_with_selection,
};
use renderer::layout::RectF;
use renderer::legend::Legend;
use renderer::select::{
    PartOutline, SELECTION_COLOR, SELECTION_PADDING, part_outline, segment_grab_half_width,
};
use renderer::sink::Parts;
use renderer::text::RichText;
use renderer::{
    Chart, Color, ColorMap, Config, CpuTextMeasure, DrawnPart, HitId, HitMap, LayerRef,
    OverlayLayer, Renderer, RendererDevice, SelectionBox, SelectionGeometry, SelectionOutline,
    SeriesConfig,
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
/// given. A `*` in a pattern stands for any run of characters.
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
            && glob(self.scene, row.scene)
            && glob(self.layer, &row.layer)
            && glob(self.element, &row.element)
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
    /// Totals the summary prints: ink pixels, samples, rasters.
    counts: BTreeMap<String, usize>,
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

    /// Add `n` to the total `what` of the scene family `at` belongs to.
    fn count(&mut self, at: &At<'_>, what: &str, n: usize) {
        let family = if at.layer == "bounded" {
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
            for m in &measured {
                registered.insert(m.name.clone(), m.bounds);
                drawn.insert(m.name.clone(), m.parts.clone());
            }
            report.time("Cartesian geometry", phase);
            let phase = Instant::now();
            let masks: Vec<Mask> = measured.iter().map(|m| Mask::of(m, &ink)).collect();
            check_i2(report, &at, &cfg, &map, &measured, &masks, &ink);
            report.time("Cartesian hit regions (I2)", phase);
            if scale == 1.0 {
                let phase = Instant::now();
                check_i3(report, &at, &measured, &ink);
                report.time("Cartesian highlights (I3)", phase);
            }
        }
        check_owners(report, &document, &ink, &registered);
        check_i4(report, &document, &doc, &drawn);
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
fn check_i2(
    report: &mut Report,
    at: &At<'_>,
    cfg: &Config,
    map: &HitMap,
    measured: &[Measured],
    masks: &[Mask],
    ink: &SceneInk,
) {
    let measure = CpuTextMeasure::for_style(&cfg.draw_style);
    let mut strong_total = 0usize;
    let mut samples_total = 0usize;
    for (index, (m, mask)) in measured.iter().zip(masks).enumerate() {
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
            if element.contains(cfg, &measure, cx, cy) != mask.get(i64::from(x), i64::from(y)) {
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
    report.count(at, "strong ink pixels", strong_total);
    report.count(at, "contains samples", samples_total);
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
/// layer hidden, so nothing is drawn over the decoration).
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
    // Composite the panels as the renderer does: in order, premultiplied over.
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
