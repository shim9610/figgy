//! `layout::auto_layout` (#44) through the renderer's text measure: pure and
//! idempotent on every catalogue scene and released config document, data
//! untouched, documents that already clear kept as they are, and an
//! auto-laid-out document as an ordinary SSOT (serialization, the registry's
//! invalidation, hit testing). The no-overlap property on drawn pixels at 1x
//! and 2x, and drag, selection and export on every scene, are the decoration
//! oracle's `auto_layout_*` tests.
#![cfg(not(target_arch = "wasm32"))]

#[path = "support/playground_scenes.rs"]
mod scenes;
#[path = "support/gpu.rs"]
mod test_gpu;

use std::sync::Arc;

use renderer::layout::{Element, RectF, Side, auto_layout};
use renderer::sink::Parts;
use renderer::{Config, CpuTextMeasure, HitMap, Renderer, RendererDevice};

use scenes::SceneKind;

const POOL_BYTES: u64 = 64 * 1024 * 1024;
const BIG_DATA_POINTS: usize = 100_000;
#[cfg(feature = "serde")]
const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/released");

fn renderer() -> Renderer {
    let (device, queue) = test_gpu::device();
    let gpu = RendererDevice::new(Arc::new(device), Arc::new(queue));
    Renderer::try_new(gpu, wgpu::TextureFormat::Rgba8Unorm, POOL_BYTES).expect("renderer")
}

fn measure(cfg: &Config) -> CpuTextMeasure {
    CpuTextMeasure::for_style(&cfg.draw_style)
}

fn laid_out(cfg: &Config) -> Config {
    auto_layout(cfg, &measure(cfg))
}

/// Every Cartesian scene of the catalogue, its columns in `r`.
fn catalogue(r: &mut Renderer) -> Vec<(SceneKind, scenes::CartesianScene)> {
    SceneKind::ALL
        .into_iter()
        .filter(|kind| !kind.is_radial())
        .map(|kind| {
            let scene =
                scenes::build(r, kind, BIG_DATA_POINTS).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
            (kind, scene)
        })
        .collect()
}

/// The released `Config` documents: every model version's default, edited
/// and series configs.
#[cfg(feature = "serde")]
fn released_configs() -> Vec<(String, Config)> {
    let mut out = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(FIXTURE_DIR)
        .expect("fixture dir")
        .map(|e| e.expect("entry").file_name().into_string().expect("utf-8"))
        .filter(|n| n.contains("-config-"))
        .collect();
    names.sort();
    for name in names {
        let text = std::fs::read_to_string(format!("{FIXTURE_DIR}/{name}")).expect("fixture");
        let cfg: Config = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        out.push((name, cfg));
    }
    out
}

/// What `auto_layout` may write, set back to the input's, so the rest can be
/// compared whole: the bands, the title offsets across them, the label
/// formats, the chart title band and the legend's placement.
fn without_layout(mut out: Config, input: &Config) -> Config {
    fn axis(to: &mut renderer::config::AxisOptions, from: &renderer::config::AxisOptions) {
        to.out_margin = from.out_margin;
        to.title_option.offset_y = from.title_option.offset_y;
        to.label_style.format = from.label_style.format.clone();
    }
    fn chart(out: &mut Config, input: &Config) {
        axis(&mut out.top_x, &input.top_x);
        axis(&mut out.bottom_x, &input.bottom_x);
        axis(&mut out.left_y, &input.left_y);
        axis(&mut out.right_y, &input.right_y);
        for (to, from) in out.overlays.iter_mut().zip(&input.overlays) {
            axis(&mut to.y_axis, &from.y_axis);
        }
        if let (Some(to), Some(from)) = (out.colorbar.as_mut(), input.colorbar.as_ref()) {
            axis(&mut to.axis, &from.axis);
        }
        out.chart_title.top_margin = input.chart_title.top_margin;
        out.legend.corner = input.legend.corner;
        out.legend.offset_x = input.legend.offset_x;
        out.legend.offset_y = input.legend.offset_y;
    }
    chart(&mut out, input);
    for (to, from) in out.subplots.iter_mut().zip(&input.subplots) {
        chart(&mut to.config, &from.config);
    }
    out
}

// The same input gives the same output, the input is not modified, and a
// second call changes nothing: on every scene of the catalogue.
#[test]
fn auto_layout_is_pure_and_idempotent_on_every_catalogue_scene() {
    let mut r = renderer();
    for (kind, scene) in catalogue(&mut r) {
        let input = scene.config.clone();
        let once = laid_out(&scene.config);
        assert_eq!(scene.config, input, "{kind:?}: the input changed");
        assert_eq!(laid_out(&scene.config), once, "{kind:?}: not deterministic");
        assert_eq!(laid_out(&once), once, "{kind:?}: not idempotent");
    }
}

// Only display parameters move: ranges, ticks, scales, categories, series
// membership, links, annotations, picks and every style but the formats are
// the input's.
#[test]
fn auto_layout_leaves_every_data_field_as_it_was() {
    let mut r = renderer();
    for (kind, scene) in catalogue(&mut r) {
        let out = laid_out(&scene.config);
        assert_eq!(
            without_layout(out, &scene.config),
            scene.config,
            "{kind:?}: a field outside the layout changed"
        );
    }
}

/// What changed between two configs' layout fields, one line per field.
fn layout_changes(before: &Config, after: &Config) -> Vec<String> {
    let mut out = Vec::new();
    let mut axes = vec![
        ("top_x", &before.top_x, &after.top_x),
        ("bottom_x", &before.bottom_x, &after.bottom_x),
        ("left_y", &before.left_y, &after.left_y),
        ("right_y", &before.right_y, &after.right_y),
    ];
    if let (Some(b), Some(a)) = (&before.colorbar, &after.colorbar) {
        axes.push(("colorbar.axis", &b.axis, &a.axis));
    }
    for (name, b, a) in axes {
        if b.out_margin != a.out_margin {
            out.push(format!(
                "{name}.out_margin {} -> {}",
                b.out_margin, a.out_margin
            ));
        }
        if b.title_option.offset_y != a.title_option.offset_y {
            out.push(format!(
                "{name}.title_option.offset_y {} -> {}",
                b.title_option.offset_y, a.title_option.offset_y
            ));
        }
        if b.label_style.format != a.label_style.format {
            out.push(format!(
                "{name}.label_style.format {:?} -> {:?}",
                b.label_style.format, a.label_style.format
            ));
        }
    }
    if before.chart_title.top_margin != after.chart_title.top_margin {
        out.push(format!(
            "chart_title.top_margin {} -> {}",
            before.chart_title.top_margin, after.chart_title.top_margin
        ));
    }
    let (b, a) = (&before.legend, &after.legend);
    if (b.corner, b.offset_x, b.offset_y) != (a.corner, a.offset_x, a.offset_y) {
        out.push(format!(
            "legend {:?} ({}, {}) -> {:?} ({}, {})",
            b.corner, b.offset_x, b.offset_y, a.corner, a.offset_x, a.offset_y
        ));
    }
    out
}

// The default document already clears, so it is returned as it is: 0 px.
// So are the released config documents of every model version (their
// chrome: margins, offsets, formats), except the changes listed.
#[cfg(feature = "serde")]
#[test]
fn auto_layout_keeps_the_default_and_the_released_config_documents() {
    let default = renderer::default::default_config();
    assert_eq!(laid_out(&default), default);
    let mut seen = Vec::new();
    for (name, cfg) in released_configs() {
        let out = laid_out(&cfg);
        assert_eq!(laid_out(&out), out, "{name}: not idempotent");
        assert_eq!(without_layout(out.clone(), &cfg), cfg, "{name}");
        for change in layout_changes(&cfg, &out) {
            seen.push(format!("{name}: {change}"));
        }
    }
    // The edited documents of 0.10.0 and 0.11.0 set a 48 px title band for a
    // taller title (fitted to 60 px), show the top labels in an 8 px band
    // and centre a left title less than the gap from its labels, which moves
    // 10 px outward. Every other released config is kept to the pixel.
    let expected: &[&str] = &[
        "model-0.10.0-config-edited.json: top_x.out_margin 8 -> 12",
        "model-0.10.0-config-edited.json: left_y.title_option.offset_y 0 -> -10",
        "model-0.10.0-config-edited.json: chart_title.top_margin 48 -> 60",
        "model-0.11.0-config-edited.json: top_x.out_margin 8 -> 12",
        "model-0.11.0-config-edited.json: left_y.title_option.offset_y 0 -> -10",
        "model-0.11.0-config-edited.json: chart_title.top_margin 48 -> 60",
    ];
    assert_eq!(seen, expected);
}

/// The drawn rects of one element of `cfg`.
fn rects(cfg: &Config, element: Element) -> Vec<RectF> {
    let mut out = Vec::new();
    renderer::select::on_drawn_panel(cfg, |panel| {
        renderer::select::emit_element(
            panel,
            &element,
            &measure(panel),
            &mut Parts(&mut |part| out.push(part.extent())),
        );
    });
    out
}

fn overlap(a: RectF, b: RectF) -> bool {
    let w = (a.x + a.width).min(b.x + b.width) - a.x.max(b.x);
    let h = (a.y + a.height).min(b.y + b.height) - a.y.max(b.y);
    w > 0.0 && h > 0.0
}

/// How many label and title rects overlap on the four sides.
fn title_overlaps(cfg: &Config) -> usize {
    [Side::Top, Side::Bottom, Side::Left, Side::Right]
        .into_iter()
        .map(|side| {
            let titles = rects(cfg, Element::AxisTitle(side.clone()));
            let labels = rects(cfg, Element::AxisLabel(side));
            titles
                .iter()
                .map(|t| labels.iter().filter(|l| overlap(*t, **l)).count())
                .sum::<usize>()
        })
        .sum()
}

// The user's case: `0.00300` labels under the Y title. After `auto_layout`
// the Y labels print as powers of ten, the title sits beside them, and no
// title touches its labels, at 1x and 2x.
#[test]
fn the_long_labels_scene_overlaps_as_laid_out_and_clears_after_auto_layout() {
    let mut r = renderer();
    let scene = scenes::build(&mut r, SceneKind::LongLabels, BIG_DATA_POINTS).expect("scene");
    assert!(
        title_overlaps(&scene.config) > 0,
        "the scene must show the overlap"
    );
    let out = laid_out(&scene.config);
    for scale in [1.0, 2.0] {
        let doc = if scale == 1.0 {
            out.clone()
        } else {
            out.scaled(scale)
        };
        assert_eq!(title_overlaps(&doc), 0, "{scale}x");
    }
    assert_eq!(
        out.left_y.label_style.format,
        renderer::format::LabelFormat::Power
    );
    // The 110 px band the host set holds the labels and the title beside
    // them: the band is kept and the title moves outward by its offset.
    assert_eq!(out.left_y.out_margin, scene.config.left_y.out_margin);
    assert!(out.left_y.title_option.offset_y < 0.0);
}

// An auto-laid-out document is an ordinary SSOT: it serializes and parses
// back with every field `auto_layout` writes as it was, and laying out the
// parsed document again changes nothing. (A fitted range such as the value
// maps' colourbar minimum can come back one ulp apart: serde_json's default
// float parsing, as for any document, not a field written here.)
#[cfg(feature = "serde")]
#[test]
fn an_auto_laid_out_document_round_trips_through_json() {
    let mut r = renderer();
    for (kind, scene) in catalogue(&mut r) {
        let out = laid_out(&scene.config);
        let json = serde_json::to_string(&out).expect("serialize");
        let back: Config = serde_json::from_str(&json).expect("parse");
        assert_eq!(
            layout_changes(&out, &back),
            Vec::<String>::new(),
            "{kind:?}"
        );
        for (a, b) in out.overlays.iter().zip(&back.overlays) {
            assert_eq!(a.y_axis.out_margin, b.y_axis.out_margin, "{kind:?}");
            assert_eq!(a.y_axis.label_style.format, b.y_axis.label_style.format);
        }
        for (a, b) in out.subplots.iter().zip(&back.subplots) {
            assert_eq!(layout_changes(&a.config, &b.config), Vec::<String>::new());
        }
        assert_eq!(
            laid_out(&back),
            back,
            "{kind:?}: the parsed document is laid out"
        );
    }
}

// Applied through `set_chart_config`, the result invalidates the chart's
// raster like any config edit, the exported pixels change, the hit map of
// the stored document names the moved title, and applying it again is an
// identical config that invalidates nothing.
#[test]
fn set_chart_config_applies_an_auto_laid_out_document_as_any_edit() {
    let mut r = renderer();
    let scene = scenes::build(&mut r, SceneKind::LongLabels, BIG_DATA_POINTS).expect("scene");
    let id = r
        .register_chart(scene.config.clone(), scene.series.clone())
        .expect("register");
    let chart = renderer::Chart::new(scene.config.clone());
    let before = r
        .export_document_png_bytes(&chart, &scene.series, 1.0)
        .expect("export");
    let stamp = r.chart_render_stamp(id).expect("stamp");

    let out = laid_out(&scene.config);
    r.set_chart_config(id, out.clone()).expect("apply");
    assert_eq!(r.chart_config(id).unwrap(), &out);
    let applied = r.chart_render_stamp(id).expect("stamp");
    assert_ne!(applied, stamp, "the edit invalidates the chart");
    let after = r
        .export_document_png_bytes(&renderer::Chart::new(out.clone()), &scene.series, 1.0)
        .expect("export");
    assert_ne!(after, before);

    r.set_chart_config(id, laid_out(&out)).expect("apply again");
    assert_eq!(
        r.chart_render_stamp(id).unwrap(),
        applied,
        "idempotent: no change"
    );

    let stored = r.chart_config(id).unwrap().clone();
    let map = HitMap::for_active_layer(&stored);
    let title = rects(&stored, Element::AxisTitle(Side::Left))[0];
    let (x, y) = title.center();
    let hit = map
        .hit_test(&stored, &measure(&stored), x, y)
        .and_then(|id| map.get(id))
        .map(|e| e.element_id());
    assert_eq!(hit.as_deref(), Some("axis_title_left"));
}
