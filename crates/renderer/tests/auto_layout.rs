//! `layout::auto_layout` (#44) through the renderer's text measure: pure and
//! idempotent on every catalogue scene and released config document, data
//! untouched, the bands of the default and released documents set exactly
//! to what they hold (#270), and an auto-laid-out document as an ordinary
//! SSOT (serialization, the registry's invalidation, hit testing). The
//! no-overlap property on drawn pixels at 1x and 2x, and drag, selection and
//! export on every scene, are the decoration oracle's `auto_layout_*` tests.
#![cfg(not(target_arch = "wasm32"))]

use test_gpu::BlockOn as _;
#[path = "support/playground_scenes.rs"]
mod scenes;
#[path = "support/gpu.rs"]
mod test_gpu;

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
    let gpu = RendererDevice::new(device, queue).expect("figgy poll thread");
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
/// formats, the chart title band and its offset down (a grid row's shared
/// baseline) and the legend's placement.
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
            axis(&mut to.x_axis, &from.x_axis);
        }
        if let (Some(to), Some(from)) = (out.colorbar.as_mut(), input.colorbar.as_ref()) {
            axis(&mut to.axis, &from.axis);
        }
        out.chart_title.top_margin = input.chart_title.top_margin;
        out.chart_title.offset_y = input.chart_title.offset_y;
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
    if before.chart_title.offset_y != after.chart_title.offset_y {
        out.push(format!(
            "chart_title.offset_y {} -> {}",
            before.chart_title.offset_y, after.chart_title.offset_y
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

// Every band of the default document and of the released config documents
// of every model version (their chrome: margins, offsets, formats) is set to
// exactly what it holds (#270), smaller or larger than the document had it:
// the changes listed, and nothing else.
//
// The edge padding is half the font of what sits at the band's outer edge,
// at least 8 px: 9 px past the 18 px labels and the frame, half the title
// font past an axis title (#270); a title sits half its labels' font (9 px)
// past them.
//
// - The default document: the bottom and left bands shrink to their labels
//   and the 9 px edge padding (no axis titles); the top band holds the pixel
//   of the top y label that runs past the top tick, and the right band the
//   last x label's overhang past the right tick, each with the padding; the
//   empty chart title gets no band.
// - `config-default` (0.10.0 and 0.11.0) is the same document with an 8 px
//   right band, which grows to the overhang.
// - `config-series` is the default with a right-hand colourbar: its band
//   shrinks to its labels and the 9 px padding (no title).
// - `config-edited` shows the top labels (the band grows to their reach and
//   the padding), puts each axis title beside its labels in a band set to
//   labels, the 9 px gap, title and padding (10 px past the 20 px bottom
//   title, 11 px past the 22 px left one; the bottom title moves 8 px
//   outward, the left one 26 px), holds the last x label's overhang past
//   the 8 px right tick, shrinks the bottom colourbar's band to its labels,
//   its 16 px title (half the 18 px label font past them) and 8 px of
//   padding, and pads its two-line 30 px title by 0.3 em above and below;
//   above its first baseline the band holds the font's line ascent, which
//   rises past the ink of "Released".
#[cfg(feature = "serde")]
#[test]
fn auto_layout_sets_the_default_and_the_released_config_documents_to_exact_bands() {
    let default = renderer::default::default_config();
    let out = laid_out(&default);
    assert_eq!(laid_out(&out), out, "default: not idempotent");
    assert_eq!(
        layout_changes(&default, &out),
        [
            "top_x.out_margin 8 -> 10",
            "bottom_x.out_margin 80 -> 25",
            "left_y.out_margin 110 -> 49",
            "right_y.out_margin 32 -> 22",
            "chart_title.top_margin 32 -> 0",
        ]
    );
    let mut seen = Vec::new();
    for (name, cfg) in released_configs() {
        let out = laid_out(&cfg);
        assert_eq!(laid_out(&out), out, "{name}: not idempotent");
        assert_eq!(without_layout(out.clone(), &cfg), cfg, "{name}");
        for change in layout_changes(&cfg, &out) {
            seen.push(format!("{name}: {change}"));
        }
    }
    let mut expected = Vec::new();
    for version in ["0.10.0", "0.11.0"] {
        for (doc, changes) in [
            (
                "default",
                &[
                    "top_x.out_margin 8 -> 10",
                    "bottom_x.out_margin 80 -> 25",
                    "left_y.out_margin 110 -> 49",
                    "right_y.out_margin 8 -> 22",
                    "chart_title.top_margin 32 -> 0",
                ][..],
            ),
            (
                "edited",
                &[
                    "top_x.out_margin 8 -> 17",
                    "bottom_x.out_margin 80 -> 54",
                    "bottom_x.title_option.offset_y 0 -> 8",
                    "left_y.out_margin 120 -> 97",
                    "left_y.title_option.offset_y 0 -> -26",
                    "right_y.out_margin 8 -> 15",
                    "colorbar.axis.out_margin 60 -> 48",
                    "chart_title.top_margin 48 -> 80",
                ][..],
            ),
            (
                "series",
                &[
                    "top_x.out_margin 8 -> 10",
                    "bottom_x.out_margin 80 -> 25",
                    "left_y.out_margin 110 -> 49",
                    "right_y.out_margin 8 -> 22",
                    "colorbar.axis.out_margin 60 -> 55",
                    "chart_title.top_margin 32 -> 0",
                ][..],
            ),
        ] {
            for change in changes {
                expected.push(format!("model-{version}-config-{doc}.json: {change}"));
            }
        }
    }
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

/// A title-less 1280 x 1024 chart, the size of the style previews, with
/// an x and a y title, its tick labels `label` px and its axis titles
/// `title` px.
fn titled_axes(label: f32, title: f32) -> Config {
    let mut cfg = renderer::default::default_config();
    cfg.chart_area.0.width = 1280;
    cfg.chart_area.0.height = 1024;
    for (axis, text) in [
        (&mut cfg.bottom_x, "Time [min]"),
        (&mut cfg.left_y, "Response mean"),
    ] {
        axis.title_option.text =
            renderer::text::RichText::plain(text, axis.title_option.text.color, title, "");
    }
    for axis in [
        &mut cfg.top_x,
        &mut cfg.bottom_x,
        &mut cfg.left_y,
        &mut cfg.right_y,
    ] {
        axis.label_style.font_size = label;
    }
    cfg
}

/// The smallest gap between the measured glyphs of the title on `side`
/// and those of its labels, across the band.
fn title_clearance(cfg: &Config, side: Side) -> f32 {
    let title = rects(cfg, Element::AxisTitle(side.clone()))[0];
    rects(cfg, Element::AxisLabel(side.clone()))
        .iter()
        .map(|l| match side {
            Side::Left => l.x - (title.x + title.width),
            Side::Right => title.x - (l.x + l.width),
            Side::Top => l.y - (title.y + title.height),
            Side::Bottom => title.y - (l.y + l.height),
        })
        .fold(f32::INFINITY, f32::min)
}

// An axis title's glyphs clear its labels' by at least half the label font
// (#270), at least 4 px, and by no more than a pixel past it (the offset is
// rounded outward); a 2x export of the result keeps twice the gap, within
// the pixel the glyphs' measured extents move between the two sizes.
#[test]
fn an_axis_title_clears_its_labels_by_half_their_font() {
    for (label, title) in [(6.0, 8.0), (12.0, 14.0), (16.0, 18.0), (25.0, 32.0)] {
        let out = laid_out(&titled_axes(label, title));
        let gap = f32::max(4.0, 0.5 * label);
        for side in [Side::Bottom, Side::Left] {
            let clearance = title_clearance(&out, side.clone());
            assert!(
                clearance >= gap - 1e-3 && clearance < gap + 1.0,
                "{label} px {side:?}: {clearance}"
            );
            if label >= 8.0 {
                let twice = title_clearance(&out.scaled(2.0), side.clone());
                assert!(
                    twice >= 2.0 * gap - 1.0,
                    "{label} px {side:?} at 2x: {twice}"
                );
            }
        }
    }
}

// The padding between what a band holds and the chart edge is half the
// font of the text at that edge (#270), at least 8 px: the x title's
// glyphs end that far above the bottom edge, and the top band, which holds
// the frame, pads by half the label font. Larger text pads more.
#[test]
fn the_edge_padding_grows_with_the_font() {
    let mut paddings = Vec::new();
    for (label, title) in [(12.0, 14.0), (18.0, 22.0), (25.0, 32.0), (40.0, 48.0)] {
        let out = laid_out(&titled_axes(label, title));
        let x_title = rects(&out, Element::AxisTitle(Side::Bottom))[0];
        let ca = RectF::from_rect(&out.chart_area);
        let bottom = ca.y + ca.height - (x_title.y + x_title.height);
        let want = f32::max(8.0, 0.5 * title);
        assert!(
            bottom >= want - 1.0 && bottom < want + 1.0,
            "{label}/{title} px: bottom {bottom}"
        );
        // The frame past the top tick, or the top y label above it.
        let da = out.data_area().expect("a data area").0;
        let tick = out.top_x.major_tick_length;
        let top = rects(&out, Element::AxisLabel(Side::Left))
            .iter()
            .map(|r| r.y)
            .fold(da.y as f32 - tick, f32::min)
            - ca.y;
        let want = f32::max(8.0, 0.5 * label);
        assert!(
            top >= want && top < want + 1.0,
            "{label}/{title} px: top {top}"
        );
        paddings.push((top, bottom));
    }
    for pair in paddings.windows(2) {
        assert!(
            pair[0].0 <= pair[1].0 && pair[0].1 < pair[1].1,
            "{paddings:?}"
        );
    }
    assert!(paddings[0].0 < paddings[3].0, "{paddings:?}");
}

// An axis with a unit (#273) prints longer labels than its data values:
// `auto_layout` measures the labels as drawn, so the y band grows to hold
// `-40000` .. `40000` where the data reads `0.0` .. `1.0`, and the title
// clears them at 1x and 2x.
#[test]
fn auto_layout_gives_converted_labels_their_room() {
    let plain = titled_axes(12.0, 14.0);
    let mut converted = plain.clone();
    converted.left_y.unit = Some(renderer::config::AxisUnit::new(80_000.0, -40_000.0));
    converted.left_y.major_spacing = 20_000.0;
    assert_eq!(converted.validate_chart(), Ok(()));
    let texts = |cfg: &Config| {
        let mut out = Vec::new();
        renderer::select::emit_element(
            cfg,
            &Element::AxisLabel(Side::Left),
            &measure(cfg),
            &mut Parts(&mut |part| {
                if let renderer::layout::DrawnPart::Text(r) = part {
                    out.push(r.width);
                }
            }),
        );
        out
    };
    let widest = |cfg: &Config| texts(cfg).into_iter().fold(0.0f32, f32::max);
    assert!(widest(&converted) > widest(&plain) + 10.0);
    let (plain_out, converted_out) = (laid_out(&plain), laid_out(&converted));
    // The format stays: five digits are not long labels.
    assert_eq!(
        converted_out.left_y.label_style.format,
        converted.left_y.label_style.format
    );
    assert!(
        converted_out.left_y.out_margin
            >= plain_out.left_y.out_margin + (widest(&converted) - widest(&plain)).floor(),
        "{} against {}",
        converted_out.left_y.out_margin,
        plain_out.left_y.out_margin
    );
    let gap = f32::max(4.0, 0.5 * 12.0);
    for (scale, doc) in [
        (1.0, converted_out.clone()),
        (2.0, converted_out.scaled(2.0)),
    ] {
        assert_eq!(title_overlaps(&doc), 0, "{scale}x");
        let clearance = title_clearance(&doc, Side::Left);
        assert!(clearance >= scale * gap - 1.0, "{scale}x: {clearance}");
    }
    // Laid out again, nothing changes.
    assert_eq!(laid_out(&converted_out), converted_out);
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
    // The band is set to exactly the labels, the gap (half the 18 px label
    // font), the title beside them and the edge padding (half the 22 px
    // title font, 11 px): the 110 px band grows to 114 px, the title moves
    // outward by its offset and ends the padding (within the pixel the
    // offset is rounded to) from the chart edge.
    assert_eq!(
        (scene.config.left_y.out_margin, out.left_y.out_margin),
        (110.0, 114.0)
    );
    assert!(out.left_y.title_option.offset_y < 0.0);
    let title = rects(&out, Element::AxisTitle(Side::Left))[0];
    let edge_gap = title.x - out.chart_area.x as f32;
    assert!((10.0..=12.0).contains(&edge_gap), "{edge_gap}");
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
        .export_png(
            renderer::ExportSource::chart(&chart, &scene.series),
            renderer::ExportOptions::document(1.0),
        )
        .block_on()
        .expect("export");
    let stamp = r.chart_render_stamp(id).expect("stamp");

    let out = laid_out(&scene.config);
    r.set_chart_config(id, out.clone()).expect("apply");
    assert_eq!(r.chart_config(id).unwrap(), &out);
    let applied = r.chart_render_stamp(id).expect("stamp");
    assert_ne!(applied, stamp, "the edit invalidates the chart");
    let after = r
        .export_png(
            renderer::ExportSource::chart(&renderer::Chart::new(out.clone()), &scene.series),
            renderer::ExportOptions::document(1.0),
        )
        .block_on()
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

/// An 800x600 chart and a 400x300 subplot beside it with a title, axis
/// titles, long tick labels and a legend: half the parent's size, so its
/// text is drawn at 0.79 of its stored sizes (#292).
fn small_cell_document() -> Config {
    let titled = |title: &str| {
        renderer::Chart::sized(800, 600)
            .with_title(title)
            .with_x_title("Time [min]")
            .with_y_title("Response mean")
            .config()
            .clone()
    };
    let mut config = titled("Parent");
    let mut cell = titled("Optical depth");
    cell.chart_area.0 = renderer::layout::Rect {
        x: 800,
        y: 0,
        width: 400,
        height: 300,
    };
    cell.left_y.min = 0.0;
    cell.left_y.max = 0.006;
    cell.left_y.major_spacing = 0.003;
    cell.legend.visible = true;
    cell.legend.content.segments = renderer::text::rich_segments_from_text("legend");
    config.subplots.push(renderer::Subplot::new(cell, &[]));
    config
}

/// The rect that holds every pixel of `image` inside `within` that is
/// darker than mid grey, in image pixels.
fn ink(image: &renderer::RasterImage, within: RectF) -> Option<RectF> {
    let (x0, y0) = (
        within.x.floor().max(0.0) as u32,
        within.y.floor().max(0.0) as u32,
    );
    let x1 = ((within.x + within.width).ceil() as u32).min(image.width);
    let y1 = ((within.y + within.height).ceil() as u32).min(image.height);
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for y in y0..y1 {
        for x in x0..x1 {
            let at = ((y * image.width + x) * 4) as usize;
            let pixel = &image.rgba[at..at + 4];
            if pixel[..3].iter().map(|c| u32::from(*c)).sum::<u32>() < 3 * 128 {
                let (l, t, r, b) = found.unwrap_or((x, y, x, y));
                found = Some((l.min(x), t.min(y), r.max(x), b.max(y)));
            }
        }
    }
    found.map(|(l, t, r, b)| RectF {
        x: l as f32,
        y: t as f32,
        width: (r - l + 1) as f32,
        height: (b - t + 1) as f32,
    })
}

// A small cell is fitted to the text it draws (#292). Its bands are the
// ones of the same cell with the drawn sizes written in, smaller than the
// stored sizes'; at 1x and 2x its titles clear its labels by half the drawn
// label font, no title overlaps a label and every text part lies inside the
// cell; the stored text sizes stay.
#[test]
fn auto_layout_fits_a_small_subplot_cell_to_its_scaled_text() {
    let doc = small_cell_document();
    let scale = doc.subplot_text_factor(0);
    assert!((scale - 0.5f32.cbrt()).abs() < 1e-6, "{scale}");
    let out = laid_out(&doc);
    assert_eq!(laid_out(&out), out, "idempotent");

    let mut written = doc.clone();
    written.subplots[0].config = doc.subplot_config(0).unwrap();
    written.subplots[0].text_scale = renderer::SubplotTextScale::Fixed;
    let mut stored = doc.clone();
    stored.subplots[0].text_scale = renderer::SubplotTextScale::Fixed;
    let (as_written, as_stored) = (laid_out(&written), laid_out(&stored));
    let bands = |c: &Config| {
        let cell = &c.subplots[0].config;
        [
            cell.chart_title.top_margin,
            cell.left_y.out_margin,
            cell.bottom_x.out_margin,
            cell.right_y.out_margin,
            cell.top_x.out_margin,
        ]
    };
    assert_eq!(bands(&out), bands(&as_written));
    for (band, wide) in bands(&out).iter().zip(bands(&as_stored)).take(3) {
        assert!(*band > 0.0 && *band < wide, "{band} against {wide}");
    }
    assert_eq!(
        out.subplot_config(0).unwrap(),
        as_written.subplot_config(0).unwrap()
    );
    // Only layout fields changed in the stored cell.
    let (cell, before) = (&out.subplots[0].config, &doc.subplots[0].config);
    assert_eq!(cell.chart_title.text, before.chart_title.text);
    assert_eq!(cell.legend.content, before.legend.content);
    for (axis, old) in [
        (&cell.left_y, &before.left_y),
        (&cell.bottom_x, &before.bottom_x),
    ] {
        assert_eq!(axis.label_style.font_size, old.label_style.font_size);
        assert_eq!(axis.title_option.text, old.title_option.text);
    }

    for export in [1.0f32, 2.0] {
        let chart = out.scaled(export).subplot_config(0).unwrap();
        let label = chart.left_y.label_style.font_size;
        assert!((label - before.left_y.label_style.font_size * scale * export).abs() < 1e-3);
        let gap = f32::max(4.0 * export, 0.5 * label);
        for side in [Side::Bottom, Side::Left] {
            let clearance = title_clearance(&chart, side.clone());
            assert!(
                clearance >= gap - 1.0 && clearance < gap + 2.0 * export,
                "{side:?} at {export}x: {clearance} against {gap}"
            );
        }
        assert_eq!(title_overlaps(&chart), 0, "at {export}x");
        let area = RectF::from_rect(&chart.chart_area.0);
        for element in [
            Element::ChartTitle,
            Element::AxisTitle(Side::Bottom),
            Element::AxisTitle(Side::Left),
            Element::AxisLabel(Side::Bottom),
            Element::AxisLabel(Side::Left),
            Element::Legend,
        ] {
            let parts = rects(&chart, element.clone());
            assert!(!parts.is_empty(), "{element:?} draws");
            for part in parts {
                assert!(
                    part.x >= area.x
                        && part.y >= area.y
                        && part.x + part.width <= area.x + area.width
                        && part.y + part.height <= area.y + area.height,
                    "{element:?} at {export}x: {part:?} outside {area:?}"
                );
            }
        }
    }
}

// A scaled subplot's decorations are hit where they are drawn and drag as
// on any chart (#292). In the PNG export at 1x and 2x the ink of the cell's
// chart title fills its hit region (the region of the drawn size, not of
// the stored one); the title, the legend, an axis, the tick labels and an
// axis title each move by the drag and keep their size; the stored text
// sizes stay; and a resize of the cell is an edit that draws its text
// again at the new factor.
#[test]
fn a_scaled_subplots_decorations_are_hit_and_dragged_where_they_are_drawn() {
    use renderer::SelectionGeometry as _;
    let mut r = renderer();
    let doc = laid_out(&small_cell_document());
    let scale = doc.subplot_text_factor(0);
    let export = |r: &mut Renderer, config: &Config, at: f32| {
        r.export_rgba(
            renderer::ExportSource::chart(&renderer::Chart::new(config.clone()), &[]),
            renderer::ExportOptions::document(at).with_clear(renderer::Color::WHITE),
        )
        .block_on()
        .expect("export")
    };
    let bounds = |config: &Config, id: &str| {
        let map = HitMap::for_active_layer(config);
        let element = map
            .get(map.find(id).unwrap_or_else(|| panic!("{id}")))
            .unwrap();
        element
            .bounds(config, &measure(config))
            .unwrap_or_else(|| panic!("{id} draws"))
    };
    let mut fixed = doc.clone();
    fixed.subplots[0].text_scale = renderer::SubplotTextScale::Fixed;
    let stored_title = bounds(&fixed, "subplot_0:chart_title");
    for at in [1.0f32, 2.0] {
        let shown = doc.scaled(at);
        let title = bounds(&shown, "subplot_0:chart_title");
        assert!(
            (title.height - stored_title.height * scale * at).abs() < 1.5 * at,
            "at {at}x: {title:?} against {stored_title:?}"
        );
        let image = export(&mut r, &doc, at);
        let cell = RectF::from_rect(&shown.subplots[0].config.chart_area.0);
        // The title band above the cell's plot, its whole width.
        let band = RectF {
            x: cell.x,
            y: cell.y,
            width: cell.width,
            height: title.y + title.height + 2.0 * at - cell.y,
        };
        let drawn = ink(&image, band).expect("the title's ink");
        let slack = 1.5 * at;
        assert!(
            drawn.x >= title.x - slack
                && drawn.y >= title.y - slack
                && drawn.x + drawn.width <= title.x + title.width + slack
                && drawn.y + drawn.height <= title.y + title.height + slack,
            "at {at}x: ink {drawn:?} outside the hit region {title:?}"
        );
        assert!(
            drawn.width >= title.width - 4.0 * at && drawn.width > 0.85 * title.width,
            "at {at}x: ink {drawn:?} narrower than the hit region {title:?}"
        );
        // A press on the ink's centre is the title's.
        let map = HitMap::for_active_layer(&shown);
        let (x, y) = drawn.center();
        let hit = map
            .hit_test(&shown, &measure(&shown), x, y)
            .and_then(|id| map.get(id))
            .map(|e| e.element_id());
        assert_eq!(hit.as_deref(), Some("subplot_0:chart_title"), "at {at}x");
    }

    let mut dragged = doc.clone();
    let map = HitMap::for_active_layer(&dragged);
    for (id, dx, dy) in [
        ("subplot_0:chart_title", 5.0, 3.0),
        ("subplot_0:legend", -6.0, 4.0),
        ("subplot_0:axis_bottom", 0.0, 4.0),
        ("subplot_0:tick_labels_left", -3.0, 0.0),
        ("subplot_0:axis_title_bottom", 0.0, 3.0),
    ] {
        let from = bounds(&dragged, id);
        let element = map.get(map.find(id).unwrap()).unwrap();
        let config = dragged.clone();
        let result =
            element
                .as_draggable()
                .unwrap()
                .drag_by(&mut dragged, &measure(&config), dx, dy);
        assert_eq!(result, renderer::layout::NudgeResult::Moved, "{id}");
        let to = bounds(&dragged, id);
        assert!(
            (to.x - from.x - dx).abs() < 0.01 && (to.y - from.y - dy).abs() < 0.01,
            "{id}: {from:?} to {to:?}"
        );
        assert!(
            (to.width - from.width).abs() < 0.01 && (to.height - from.height).abs() < 0.01,
            "{id}: {from:?} to {to:?}"
        );
    }
    let (cell, before) = (&dragged.subplots[0].config, &doc.subplots[0].config);
    assert_ne!(cell, before);
    assert_eq!(cell.chart_title.text, before.chart_title.text);
    assert_eq!(cell.legend.content, before.legend.content);
    assert_eq!(
        cell.left_y.label_style.font_size,
        before.left_y.label_style.font_size
    );
    assert_eq!(dragged.subplot_text_factor(0), scale);
    let moved = export(&mut r, &dragged, 1.0);
    assert!(moved.rgba != export(&mut r, &doc, 1.0).rgba);

    // A registered chart follows a resized cell: the edit invalidates it
    // and the stored sizes are drawn at the new factor.
    let id = r.register_chart(doc.clone(), Vec::new()).expect("register");
    let stamp = r.chart_render_stamp(id).expect("stamp");
    let mut resized = doc.clone();
    resized
        .set_subplot_area(
            0,
            renderer::layout::ChartArea(renderer::layout::Rect {
                x: 800,
                y: 0,
                width: 600,
                height: 450,
            }),
        )
        .expect("a larger cell");
    r.set_chart_config(id, resized.clone()).expect("apply");
    assert_ne!(r.chart_render_stamp(id).expect("stamp"), stamp);
    let stored = r.chart_config(id).unwrap();
    assert!((stored.subplot_text_factor(0) - 0.75f32.cbrt()).abs() < 1e-6);
    assert_eq!(
        stored.subplots[0].config.chart_title.text,
        doc.subplots[0].config.chart_title.text
    );
    let larger = bounds(&resized, "subplot_0:chart_title");
    let smaller = bounds(&doc, "subplot_0:chart_title");
    assert!(larger.height > smaller.height, "{larger:?} {smaller:?}");
}
