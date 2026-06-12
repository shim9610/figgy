//! Axis renderer. Takes a `Config` and draws axes, ticks, labels, and titles
//! onto the CPU raster [`Canvas`]. No data is drawn here — only axis chrome.
//!
//! Text rendering is delegated to [`crate::text_render`].

use crate::raster::{Canvas, Paint};

use crate::color::Color;
use crate::config::{AxisOptions, AxisScale, Config, LabelStyle, TickVisibility};
use crate::format::LabelFormat;
use crate::layout::{DataArea, Side};
use crate::line::LineStylePreset;
use crate::select::SelectionBox;
use crate::sketch::DecoStroker;
use crate::text::{RichSegment, RichText};
use crate::text_render::{
    draw_plain_text, draw_rich_text, measure_plain_text, measure_rich_text, FontPolicy,
};

// Public entry.

/// Which layer of the axis raster to produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisLayerKind {
    /// Grid lines + minor ticks only. Should be composited *below* data so
    /// data is not obscured.
    Grid,
    /// Axis lines, tick labels, axis titles, chart title, legend. Composited
    /// *above* data.
    Decoration,
    /// Both, in one layer (legacy single-pass). New code should use the
    /// separate Grid / Decoration layers.
    All,
}

/// Rasterize the axes into an RGBA8 premultiplied buffer matching
/// `config.chart_area` in size.
///
/// Used as the **CPU raster** that the renderer uploads to a wgpu texture.
/// The returned buffer is `width * height * 4` bytes; each pixel is
/// `[R, G, B, A]` with premultiplied alpha.
///
/// Returns [`FiggyError::InvalidChartArea`] / [`FiggyError::RasterWrapFailed`]
/// instead of panicking.
pub fn try_raster_chart_to_rgba(config: &Config) -> crate::Result<Vec<u8>> {
    try_raster_chart_layer_to_rgba(config, AxisLayerKind::All)
}

/// Rasterize a single layer — used to composite grid below and decoration
/// above the GPU data layer.
pub fn try_raster_chart_layer_to_rgba(
    config: &Config,
    layer: AxisLayerKind,
) -> crate::Result<Vec<u8>> {
    try_raster_chart_layer_to_rgba_with_selection(config, layer, &[])
}

/// [`try_raster_chart_layer_to_rgba`] plus selection highlight boxes drawn on
/// top (Decoration / All layers only — selection never sits under the data).
///
/// `selection` comes from `Selectable::selection_box(cfg, …)` and is in
/// absolute chart-surface coordinates; this function shifts it into the
/// chart_area-relative raster frame.
pub fn try_raster_chart_layer_to_rgba_with_selection(
    config: &Config,
    layer: AxisLayerKind,
    selection: &[SelectionBox],
) -> crate::Result<Vec<u8>> {
    use crate::layout::{ChartArea, Rect};

    let w = config.chart_area.0.width;
    let h = config.chart_area.0.height;
    if w == 0 || h == 0 {
        return Err(crate::FiggyError::InvalidChartArea { width: w, height: h });
    }

    let mut raster_cfg = config.clone();
    raster_cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: w,
        height: h,
    });

    let Some(mut canvas) = Canvas::new(w, h) else {
        return Err(crate::FiggyError::RasterWrapFailed {
            reason: format!("raster target allocation failed for {w}x{h}"),
        });
    };
    match layer {
        AxisLayerKind::Grid => draw_grid_layer(&mut canvas, &raster_cfg),
        AxisLayerKind::Decoration => draw_decoration_layer(&mut canvas, &raster_cfg),
        AxisLayerKind::All => draw_axes(&mut canvas, &raster_cfg),
    }
    // Constellation: the axis chrome reads as line-light — bloom it. Runs
    // BEFORE the selection overlay so interaction chrome stays crisp.
    if let crate::config::DrawStyle::Constellation(c) = &config.draw_style {
        if matches!(layer, AxisLayerKind::Decoration | AxisLayerKind::All) {
            apply_decoration_glow(&mut canvas, c.glow);
        }
    }
    if !selection.is_empty()
        && matches!(layer, AxisLayerKind::Decoration | AxisLayerKind::All)
    {
        let ox = config.chart_area.0.x as f32;
        let oy = config.chart_area.0.y as f32;
        let local: Vec<SelectionBox> = selection
            .iter()
            .map(|b| SelectionBox {
                rect: b.rect.translated(-ox, -oy),
                handles: b.handles.iter().map(|h| h.translated(-ox, -oy)).collect(),
                ..b.clone()
            })
            .collect();
        draw_selection_boxes(&mut canvas, &local);
    }

    Ok(canvas.into_rgba())
}

/// Draw selection highlight boxes — the raster realization of the model's
/// `Selectable::selection_box` policy. Box coordinates are taken as-is in the
/// canvas frame; the raster entry above handles the chart_area shift.
/// Resize handles (when present) are drawn as white squares with the
/// selection color as border, slide-editor style.
pub fn draw_selection_boxes(canvas: &mut Canvas, boxes: &[SelectionBox]) {
    for b in boxes {
        let outline = Paint::stroke(&b.color, b.stroke_width);
        canvas.draw_rect(b.rect.x, b.rect.y, b.rect.width, b.rect.height, &outline);

        if b.handles.is_empty() {
            continue;
        }
        let fill = Paint::fill(&Color::WHITE);
        for h in &b.handles {
            canvas.draw_rect(h.x, h.y, h.width, h.height, &fill);
            canvas.draw_rect(h.x, h.y, h.width, h.height, &outline);
        }
    }
}

/// Panic-on-error wrapper around [`try_raster_chart_to_rgba`]. New code
/// should prefer the fallible version; this exists for the binary demos.
pub fn raster_chart_to_rgba(config: &Config) -> Vec<u8> {
    try_raster_chart_to_rgba(config).expect("raster_chart_to_rgba failed")
}

/// Grid layer — only the parts that should sit below the data layer.
///
/// The constellation style repurposes this slot as its deep-space backdrop
/// (the compositing order already puts it under the data): grid lines are
/// not drawn in that mode — the style declares its own background instead.
pub fn draw_grid_layer(canvas: &mut Canvas, config: &Config) {
    if let crate::config::DrawStyle::Constellation(c) = &config.draw_style {
        draw_space_background(canvas, config, c);
        return;
    }
    let Ok(da) = config.data_area() else { return };
    draw_grid(canvas, config, &da);
}

// ── Constellation backdrop + glow (style-specific CPU post-processing) ──
//
// LEGIBILITY CONTRACT: this is a science-presentation tool — the backdrop
// must never compete with data ink. Hard rules encoded below:
//   - nebula peak adds ≤ ~12/255 luminance over the base, low-frequency only
//   - a soft vignette keeps the panel CENTER (where data lives) the cleanest
//   - background dust stars are 1 px and far dimmer than any data star
//     (data stars have PSF cores + halos and sit on series ribbons)

/// Deep-space base color (premultiplied; alpha 255 — the panel owns its
/// background in this style, no host compositing needed).
const SPACE_BASE: [f32; 3] = [11.0, 15.0, 23.0];

fn draw_space_background(
    canvas: &mut Canvas,
    _config: &Config,
    c: &crate::config::ConstellationOptions,
) {
    use crate::data_render::fbm2;

    let (w, h) = canvas.size();
    if w == 0 || h == 0 {
        return;
    }
    let seed = c.seed;

    // Nebula sampled on a quarter-res lattice (it is low-frequency by
    // design) and bilinearly upsampled — 16× cheaper than per-pixel fBm.
    let gw = (w / 4 + 2) as usize;
    let gh = (h / 4 + 2) as usize;
    let mut cool = vec![0.0f32; gw * gh];
    let mut warm = vec![0.0f32; gw * gh];
    for gy in 0..gh {
        for gx in 0..gw {
            let x = gx as f64 * 4.0 / w.max(1) as f64 * 3.0;
            let y = gy as f64 * 4.0 / h.max(1) as f64 * 2.0;
            cool[gy * gw + gx] =
                ((fbm2(x + 7.1, y + 3.7, 5, seed ^ 0x0EB1) - 0.42).max(0.0) * 2.0).min(1.0) as f32;
            warm[gy * gw + gx] =
                ((fbm2(x * 0.7 + 21.0, y * 0.7 + 9.0, 4, seed ^ 0x0EB2) - 0.50).max(0.0) * 2.2)
                    .min(1.0) as f32;
        }
    }
    let sample = |grid: &[f32], px: u32, py: u32| -> f32 {
        let fx = px as f32 / 4.0;
        let fy = py as f32 / 4.0;
        let x0 = fx.floor() as usize;
        let y0 = fy.floor() as usize;
        let (tx, ty) = (fx.fract(), fy.fract());
        let i = |x: usize, y: usize| grid[(y.min(gh - 1)) * gw + x.min(gw - 1)];
        let a = i(x0, y0) * (1.0 - tx) + i(x0 + 1, y0) * tx;
        let b = i(x0, y0 + 1) * (1.0 - tx) + i(x0 + 1, y0 + 1) * tx;
        a * (1.0 - ty) + b * ty
    };

    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let max_r = (cx * cx + cy * cy).sqrt().max(1.0);
    let data = canvas.pixels_mut();
    for py in 0..h {
        for px in 0..w {
            // Vignette: nebula fades toward the panel center so the data
            // region stays the cleanest part of the frame.
            let dx = px as f32 - cx;
            let dy = py as f32 - cy;
            let edge = ((dx * dx + dy * dy).sqrt() / max_r).clamp(0.0, 1.0);
            let vig = 0.35 + 0.65 * edge * edge;

            // Peak nebula contribution stays ≤ ~12/255 per channel at the
            // default `nebula = 1.0`; the slider scales within the same
            // legibility-bounded design.
            let neb = c.nebula.clamp(0.0, 2.0);
            let nc = sample(&cool, px, py) * vig * neb;
            let nw = sample(&warm, px, py) * vig * neb;
            let r = SPACE_BASE[0] + nc * 4.0 + nw * 9.0;
            let g = SPACE_BASE[1] + nc * 6.0 + nw * 5.0;
            let b = SPACE_BASE[2] + nc * 12.0 + nw * 4.0;

            let i = ((py * w + px) * 4) as usize;
            data[i] = r.min(255.0) as u8;
            data[i + 1] = g.min(255.0) as u8;
            data[i + 2] = b.min(255.0) as u8;
            data[i + 3] = 255;
        }
    }

    // Background dust: sparse, dim, 1 px — unmistakably "behind" the data.
    let n_dust =
        (((w as u64 * h as u64) / 1400) as f32 * c.dust.clamp(0.0, 4.0)).max(0.0) as u32;
    for k in 0..n_dust {
        let hx = crate::sketch::hash01(k, seed ^ 0xD057_0001);
        let hy = crate::sketch::hash01(k, seed ^ 0xD057_0002);
        let hb = crate::sketch::hash01(k, seed ^ 0xD057_0003);
        let px = (hx * w as f32) as u32 % w;
        let py = (hy * h as f32) as u32 % h;
        let add = 14.0 + 52.0 * hb * hb;
        let i = ((py * w + px) * 4) as usize;
        data[i] = (data[i] as f32 + add).min(255.0) as u8;
        data[i + 1] = (data[i + 1] as f32 + add).min(255.0) as u8;
        data[i + 2] = (data[i + 2] as f32 + add * 1.06).min(255.0) as u8;
    }
}

// Glow bloom for the decoration layer (axes / ticks / labels / titles read
// as line-light sources): one blurred copy added back under the crisp
// original. Premultiplied-additive with clamp; selection boxes are drawn
// AFTER this in the raster entry, so the interaction overlay stays crisp.
const GLOW_PASSES: u32 = 3;
const GLOW_RADIUS: usize = 3;

pub(crate) fn apply_decoration_glow(canvas: &mut Canvas, gain: f32) {
    let gain = gain.clamp(0.0, 2.0);
    if gain <= 0.0 {
        return;
    }
    let (w, h) = canvas.size();
    if w == 0 || h == 0 {
        return;
    }
    let (w, h) = (w as usize, h as usize);
    let src = canvas.pixels_mut();
    let mut halo: Vec<u8> = src.to_vec();
    let mut tmp = vec![0u8; halo.len()];

    // Separable box blur ×3 ≈ gaussian. Premultiplied RGBA blurs channel-wise.
    let window = (2 * GLOW_RADIUS + 1) as u32;
    for _ in 0..GLOW_PASSES {
        // Horizontal.
        for y in 0..h {
            let row = y * w * 4;
            let mut acc = [0u32; 4];
            for x in 0..w.min(GLOW_RADIUS + 1) {
                for c in 0..4 {
                    acc[c] += halo[row + x * 4 + c] as u32;
                }
            }
            let mut count = w.min(GLOW_RADIUS + 1) as u32;
            for x in 0..w {
                for c in 0..4 {
                    tmp[row + x * 4 + c] = (acc[c] / count.max(1)) as u8;
                }
                let _ = window;
                if x + GLOW_RADIUS + 1 < w {
                    for c in 0..4 {
                        acc[c] += halo[row + (x + GLOW_RADIUS + 1) * 4 + c] as u32;
                    }
                    count += 1;
                }
                if x >= GLOW_RADIUS {
                    for c in 0..4 {
                        acc[c] -= halo[row + (x - GLOW_RADIUS) * 4 + c] as u32;
                    }
                    count -= 1;
                }
            }
        }
        // Vertical.
        for x in 0..w {
            let mut acc = [0u32; 4];
            for y in 0..h.min(GLOW_RADIUS + 1) {
                for c in 0..4 {
                    acc[c] += tmp[(y * w + x) * 4 + c] as u32;
                }
            }
            let mut count = h.min(GLOW_RADIUS + 1) as u32;
            for y in 0..h {
                for c in 0..4 {
                    halo[(y * w + x) * 4 + c] = (acc[c] / count.max(1)) as u8;
                }
                if y + GLOW_RADIUS + 1 < h {
                    for c in 0..4 {
                        acc[c] += tmp[((y + GLOW_RADIUS + 1) * w + x) * 4 + c] as u32;
                    }
                    count += 1;
                }
                if y >= GLOW_RADIUS {
                    for c in 0..4 {
                        acc[c] -= tmp[((y - GLOW_RADIUS) * w + x) * 4 + c] as u32;
                    }
                    count -= 1;
                }
            }
        }
    }

    for (d, s) in src.iter_mut().zip(halo.iter()) {
        *d = (*d as f32 + *s as f32 * gain).min(255.0) as u8;
    }
}

/// Decoration layer — axis lines, tick labels, axis titles, chart title,
/// legend. Drawn above the data so data never overlaps axis chrome.
pub fn draw_decoration_layer(canvas: &mut Canvas, config: &Config) {
    let Ok(da) = config.data_area() else { return };

    // The decoration stroke strategy is derived once per layer entry and
    // threaded down (STYLE_REGISTRY §4): `Precise` keeps every draw below on
    // the plain pre-stroker canvas calls. The font policy is its text twin —
    // sketch mode forces the bundled handwritten face (with per-character
    // fallback for glyphs it lacks), threaded through every measure + draw so
    // layout and raster always agree.
    let stroker = DecoStroker::from_style(&config.draw_style);
    let fp = FontPolicy::for_style(&config.draw_style);

    draw_axis(canvas, &config.top_x, Side::Top, &da, &stroker, fp);
    draw_axis(canvas, &config.bottom_x, Side::Bottom, &da, &stroker, fp);
    draw_axis(canvas, &config.left_y, Side::Left, &da, &stroker, fp);
    draw_axis(canvas, &config.right_y, Side::Right, &da, &stroker, fp);

    draw_axis_title(canvas, config, &da, Side::Top, fp);
    draw_axis_title(canvas, config, &da, Side::Bottom, fp);
    draw_axis_title(canvas, config, &da, Side::Left, fp);
    draw_axis_title(canvas, config, &da, Side::Right, fp);

    if config.chart_title.visible {
        draw_chart_title(canvas, config, fp);
    }

    if config.legend.visible && !config.legend.content.segments.is_empty() {
        draw_legend(canvas, config, &da, &stroker, fp);
    }
}

/// Draw the full axis chrome (grid + decoration) into the regions specified
/// by `config`. If `data_area()` fails, this is a no-op.
pub fn draw_axes(canvas: &mut Canvas, config: &Config) {
    draw_grid_layer(canvas, config);
    draw_decoration_layer(canvas, config);
}

/// Draw the legend box in one corner of the data area.
///
/// The whole legend is **one rich document** (`legend.content`): `'\n'`
/// segments break lines, symbols are inline segments with per-segment color
/// overrides, and the document's font/font_size apply at draw time. The box
/// is the measured envelope plus `padding`, with the same formulas as the
/// model's `LegendElement` bounds (`model::select`) — change them together.
fn draw_legend(
    canvas: &mut Canvas,
    config: &Config,
    da: &crate::layout::DataArea,
    stroker: &DecoStroker,
    fp: FontPolicy,
) {
    use crate::config::LegendCorner;

    let lg = &config.legend;
    if !lg.visible || lg.content.segments.is_empty() { return; }

    let m = measure_rich_text(&lg.content, fp);
    let box_w = m.width + lg.padding * 2.0;
    let box_h = m.height() + lg.padding * 2.0;

    // Corner position — inset from the data_area inner corner, then the
    // user's drag offset on top.
    let inset = 6.0;
    let (box_x, box_y) = match lg.corner {
        LegendCorner::TopLeft => (da.x as f32 + inset, da.y as f32 + inset),
        LegendCorner::TopRight => (
            (da.x + da.width) as f32 - box_w - inset,
            da.y as f32 + inset,
        ),
        LegendCorner::BottomLeft => (
            da.x as f32 + inset,
            (da.y + da.height) as f32 - box_h - inset,
        ),
        LegendCorner::BottomRight => (
            (da.x + da.width) as f32 - box_w - inset,
            (da.y + da.height) as f32 - box_h - inset,
        ),
    };
    let (box_x, box_y) = (box_x + lg.offset_x, box_y + lg.offset_y);

    // Box background + border. The sketch stroker wobbles only the border
    // outline; the fill stays a precise rect (§5a) — perturbing the fill too
    // would visibly disagree with the independently wobbled border.
    canvas.draw_rect(box_x, box_y, box_w, box_h, &Paint::fill(&lg.bg_color));
    let border = Paint::stroke(&lg.border_color, 1.0);
    stroker.stroke_rect_outline(canvas, box_x, box_y, box_w, box_h, &border, "legend_box");

    // Content: first baseline sits `ascent` below the padded top-left corner.
    draw_rich_text(
        canvas,
        &lg.content,
        (box_x + lg.padding, box_y + lg.padding + m.ascent),
        fp,
    );
}

// Decoration stroke plumbing — STYLE_REGISTRY §4.
//
// Each layer entry (`draw_grid` / `draw_decoration_layer`) derives one
// [`DecoStroker`] from `config.draw_style` and threads it down by reference.
// `Precise` executes the exact pre-stroker code path (plain `draw_line` /
// `draw_rect`); the sketch arm mixes stable kind+index element tags
// ("axis_left", "tick_left_3", "grid_major_x_2", "legend_box") into the
// global seed so every element wobbles differently but identically across
// re-rasters. The selection overlay and all text stay off the stroker —
// unconditionally precise.

/// Stable side name used in stroker element tags.
fn side_tag(side: &Side) -> &'static str {
    match side {
        Side::Top => "top",
        Side::Bottom => "bottom",
        Side::Left => "left",
        Side::Right => "right",
    }
}

// Grid rendering.
//
// Vertical grid lines use bottom_x tick positions; horizontal lines use
// left_y. Lines are confined to the data_area (just inside the axis lines).

fn draw_grid(canvas: &mut Canvas, config: &Config, da: &DataArea) {
    let g = &config.grid;
    let stroker = DecoStroker::from_style(&config.draw_style);
    let x_top = da.y as f32;
    let x_bot = (da.y + da.height) as f32;
    let y_left = da.x as f32;
    let y_right = (da.x + da.width) as f32;

    // Draw minor first so major can paint over it.
    if g.show_minor_x {
        let paint = stroke_paint(&g.minor_x_color, g.minor_x_width, &g.minor_x_style);
        for (i, v) in minor_tick_values(&config.bottom_x).into_iter().enumerate() {
            let pos = value_to_screen(v, &config.bottom_x, Side::Bottom, da);
            let tag = format!("grid_minor_x_{i}");
            stroker.stroke_segment(canvas, (pos.0, x_top), (pos.0, x_bot), &paint, &tag);
        }
    }
    if g.show_minor_y {
        let paint = stroke_paint(&g.minor_y_color, g.minor_y_width, &g.minor_y_style);
        for (i, v) in minor_tick_values(&config.left_y).into_iter().enumerate() {
            let pos = value_to_screen(v, &config.left_y, Side::Left, da);
            let tag = format!("grid_minor_y_{i}");
            stroker.stroke_segment(canvas, (y_left, pos.1), (y_right, pos.1), &paint, &tag);
        }
    }

    if g.show_major_x {
        let paint = stroke_paint(&g.major_x_color, g.major_x_width, &g.major_x_style);
        for (i, v) in major_tick_values(&config.bottom_x).into_iter().enumerate() {
            let pos = value_to_screen(v, &config.bottom_x, Side::Bottom, da);
            let tag = format!("grid_major_x_{i}");
            stroker.stroke_segment(canvas, (pos.0, x_top), (pos.0, x_bot), &paint, &tag);
        }
    }
    if g.show_major_y {
        let paint = stroke_paint(&g.major_y_color, g.major_y_width, &g.major_y_style);
        for (i, v) in major_tick_values(&config.left_y).into_iter().enumerate() {
            let pos = value_to_screen(v, &config.left_y, Side::Left, da);
            let tag = format!("grid_major_y_{i}");
            stroker.stroke_segment(canvas, (y_left, pos.1), (y_right, pos.1), &paint, &tag);
        }
    }
}

fn draw_axis(
    canvas: &mut Canvas,
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    stroker: &DecoStroker,
    fp: FontPolicy,
) {
    // Detached-axis offset: shift the whole axis chrome (line + ticks +
    // labels) perpendicular to the axis. The data area and grid stay put;
    // tick positions along the axis are unaffected.
    let (off_x, off_y) = match side {
        Side::Left | Side::Right => (axis.line_offset, 0.0),
        Side::Top | Side::Bottom => (0.0, axis.line_offset),
    };
    let detached = off_x != 0.0 || off_y != 0.0;
    if detached {
        canvas.save();
        canvas.translate(off_x, off_y);
    }

    let (p0, p1) = axis_endpoints(side.clone(), da);

    // 1) Axis line.
    if axis.line_visible {
        let paint = stroke_paint(&axis.line_color, axis.line_width, &axis.line_style);
        let tag = format!("axis_{}", side_tag(&side));
        stroker.stroke_segment(canvas, p0, p1, &paint, &tag);
    }

    // 2) Major / minor ticks
    let majors = major_tick_values(axis);
    let minors = minor_tick_values(axis);

    if axis.tick != TickVisibility::None {
        let tick_paint = stroke_paint(&axis.line_color, axis.line_width, &LineStylePreset::Solid);
        // Tick stroker tags share one running index per side (majors first,
        // minors after) so every tick gets its own wobble shape.
        for (i, v) in majors.iter().enumerate() {
            let pos = value_to_screen(*v, axis, side.clone(), da);
            let tag = format!("tick_{}_{i}", side_tag(&side));
            draw_tick(canvas, pos, side.clone(), axis.major_tick_length, &axis.tick, &tick_paint, stroker, &tag);
        }
        for (i, v) in minors.iter().enumerate() {
            let pos = value_to_screen(*v, axis, side.clone(), da);
            let tag = format!("tick_{}_{}", side_tag(&side), majors.len() + i);
            draw_tick(canvas, pos, side.clone(), axis.minor_tick_length, &axis.tick, &tick_paint, stroker, &tag);
        }
    }

    // 3) Major tick labels
    let ls = &axis.label_style;
    if ls.visible && ls.label_visible {
        for v in &majors {
            let pos = value_to_screen(*v, axis, side.clone(), da);
            match ls.format {
                LabelFormat::Power => {
                    let rt = format_tick_power(*v, ls.significant_digits, ls);
                    draw_tick_label_rich(canvas, &rt, pos, side.clone(), axis, fp);
                }
                _ => {
                    let text = format_tick_value(
                        *v,
                        &ls.format,
                        ls.significant_digits,
                        &axis.scale,
                        axis.major_spacing,
                    );
                    draw_tick_label(canvas, &text, pos, side.clone(), axis, fp);
                }
            }
        }
    }

    if detached {
        canvas.restore();
    }
}

// Axis-line / tick-position helpers.

fn axis_endpoints(side: Side, da: &DataArea) -> ((f32, f32), (f32, f32)) {
    // For a 1px AA stroke to land on a single row/column at full alpha, its
    // coordinate must be a pixel center (integer + 0.5). Integer coordinates
    // would split the line over two rows at 50% alpha each, blending the
    // edge with the data drawn underneath and giving the four sides
    // different apparent colors.
    //
    // top/left: center of the first inside pixel of the data area.
    // bottom/right: center of the last inside pixel.
    let x0 = da.x as f32 + 0.5;
    let y0 = da.y as f32 + 0.5;
    let x1 = (da.x + da.width) as f32 - 0.5;
    let y1 = (da.y + da.height) as f32 - 0.5;
    match side {
        Side::Top => ((x0, y0), (x1, y0)),
        Side::Bottom => ((x0, y1), (x1, y1)),
        Side::Left => ((x0, y0), (x0, y1)),
        Side::Right => ((x1, y0), (x1, y1)),
    }
}

fn value_to_screen(value: f64, axis: &AxisOptions, side: Side, da: &DataArea) -> (f32, f32) {
    let t = match axis.scale {
        AxisScale::Linear => {
            let range = axis.max - axis.min;
            if range == 0.0 {
                0.0
            } else {
                (value - axis.min) / range
            }
        }
        AxisScale::Logarithmic => {
            let log_min = axis.min.log10();
            let log_max = axis.max.log10();
            let range = log_max - log_min;
            if range == 0.0 || value <= 0.0 {
                0.0
            } else {
                (value.log10() - log_min) / range
            }
        }
    };
    let t = if axis.inverted { 1.0 - t } else { t };
    let t_f32 = t as f32;

    match side {
        Side::Top => {
            let x = da.x as f32 + t_f32 * da.width as f32;
            (x, da.y as f32)
        }
        Side::Bottom => {
            let x = da.x as f32 + t_f32 * da.width as f32;
            (x, (da.y + da.height) as f32)
        }
        // Y axis: increasing value moves up the screen (decreasing y).
        Side::Left => {
            let y = (da.y + da.height) as f32 - t_f32 * da.height as f32;
            (da.x as f32, y)
        }
        Side::Right => {
            let y = (da.y + da.height) as f32 - t_f32 * da.height as f32;
            ((da.x + da.width) as f32, y)
        }
    }
}

fn major_tick_values(axis: &AxisOptions) -> Vec<f64> {
    let mut out = Vec::new();
    if axis.major_spacing <= 0.0 || axis.max <= axis.min {
        return out;
    }
    match axis.scale {
        AxisScale::Linear => {
            // Anchor ticks to ABSOLUTE multiples of the spacing, not to
            // axis.min: uniform-margin fits make the range ends arbitrary
            // (0.137…), but tick values must stay nice regardless.
            let sp = axis.major_spacing;
            let first = ((axis.min / sp) - 1e-9).ceil() as i64;
            let last = ((axis.max / sp) + 1e-9).floor() as i64;
            for i in first..=last {
                out.push(i as f64 * sp);
            }
        }
        AxisScale::Logarithmic => {
            // major_spacing = decade step.
            let step = axis.major_spacing.max(1.0);
            let start_exp = axis.min.log10().ceil() as i64;
            let end_exp = axis.max.log10().floor() as i64;
            let step_i = step as i64;
            let mut e = start_exp;
            while e <= end_exp {
                out.push(10f64.powi(e as i32));
                e += step_i;
            }
        }
    }
    out
}

/// Minor tick positions on the same ABSOLUTE grid the majors use — including
/// the partial intervals before the first and after the last major (range
/// ends are arbitrary under uniform-margin fits, and subdividing only
/// between consecutive majors left those edge strips empty).
fn minor_tick_values(axis: &AxisOptions) -> Vec<f64> {
    let mut out = Vec::new();
    if axis.minor_count == 0 || axis.max <= axis.min {
        return out;
    }
    match axis.scale {
        AxisScale::Linear => {
            let subdivisions = (axis.minor_count + 1) as i64;
            let step = axis.major_spacing / subdivisions as f64;
            if !step.is_finite() || step <= 0.0 {
                return out;
            }
            let first = ((axis.min / step) - 1e-9).ceil() as i64;
            let last = ((axis.max / step) + 1e-9).floor() as i64;
            for i in first..=last {
                // Multiples of the major spacing are the majors themselves.
                if i.rem_euclid(subdivisions) == 0 {
                    continue;
                }
                out.push(i as f64 * step);
            }
        }
        AxisScale::Logarithmic => {
            // 2a..9a for EVERY decade overlapping the range, anchored at the
            // decade powers — not at the majors, so partial edge decades
            // (and multi-decade major steps) keep their minors.
            let lo = axis.min.max(f64::MIN_POSITIVE);
            let first_decade = lo.log10().floor() as i32;
            let last_decade = axis.max.log10().floor() as i32;
            for e in first_decade..=last_decade {
                let a = 10f64.powi(e);
                for k in 2..=9 {
                    let v = a * (k as f64);
                    if v >= axis.min && v <= axis.max {
                        out.push(v);
                    }
                }
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn draw_tick(
    canvas: &mut Canvas,
    pos: (f32, f32),
    side: Side,
    length: f32,
    visibility: &TickVisibility,
    paint: &Paint,
    stroker: &DecoStroker,
    tag: &str,
) {
    // outward direction = away from the data area.
    let (dx_out, dy_out) = match side {
        Side::Top => (0.0, -1.0),
        Side::Bottom => (0.0, 1.0),
        Side::Left => (-1.0, 0.0),
        Side::Right => (1.0, 0.0),
    };
    let outside = (pos.0 + dx_out * length, pos.1 + dy_out * length);
    let inside = (pos.0 - dx_out * length, pos.1 - dy_out * length);
    match visibility {
        TickVisibility::None => {}
        TickVisibility::Outside => {
            stroker.stroke_segment(canvas, pos, outside, paint, tag);
        }
        TickVisibility::Inside => {
            stroker.stroke_segment(canvas, pos, inside, paint, tag);
        }
        TickVisibility::Both => {
            stroker.stroke_segment(canvas, inside, outside, paint, tag);
        }
    }
}

// Tick labels.

/// Decimal places every tick on an axis shares, derived from the major
/// spacing: just enough digits to represent one step exactly. Per-value
/// significant digits would mix forms on one axis ("0 / 50.0 / 100 / 150"),
/// which reads as a bug.
fn decimals_from_spacing(spacing: f64) -> usize {
    if !spacing.is_finite() || spacing <= 0.0 {
        return 0;
    }
    let mut d = (-spacing.log10().floor()).max(0.0) as usize;
    // Custom spacings off the 1·2·5 grid (e.g. 2.5) need one more place
    // when one step still doesn't land on a whole number of that grid.
    while d < 6 {
        let scaled = spacing * 10f64.powi(d as i32);
        if (scaled - scaled.round()).abs() < 1e-9 {
            break;
        }
        d += 1;
    }
    d
}

fn format_tick_value(
    value: f64,
    format: &LabelFormat,
    sig_digits: u8,
    scale: &AxisScale,
    major_spacing: f64,
) -> String {
    // Log scale: ignore sig_digits padding and use a minimal form.
    if matches!(scale, AxisScale::Logarithmic) {
        if value == 0.0 {
            return "0".into();
        }
        return match format {
            LabelFormat::Decimal | LabelFormat::Power => format!("{}", value),
            LabelFormat::Scientific => format!("{:e}", value),
        };
    }

    let sig = sig_digits.max(1) as usize;
    match format {
        LabelFormat::Scientific => format!("{:.*e}", sig.saturating_sub(1), value),
        // Power uses the RichText path; if we end up here, treat as Decimal.
        // Uniform per-axis decimals from the spacing — sig_digits stays in
        // charge of the Scientific form only.
        LabelFormat::Decimal | LabelFormat::Power => {
            if value == 0.0 {
                return "0".into();
            }
            format!("{:.*}", decimals_from_spacing(major_spacing), value)
        }
    }
}

// Power format (RichText with superscript exponent).

fn plain_seg(c: char) -> RichSegment {
    RichSegment::plain(c)
}

fn sup_seg(c: char) -> RichSegment {
    RichSegment {
        superscript: true,
        ..RichSegment::plain(c)
    }
}

fn trim_trailing_fraction_zeros(s: &str) -> String {
    if !s.contains('.') {
        return s.to_string();
    }
    let trimmed = s.trim_end_matches('0');
    trimmed.trim_end_matches('.').to_string()
}

/// Decompose `value` into mantissa × 10^exp and produce a RichText with the
/// exponent as superscript. If the mantissa is very close to 1, only "10^exp"
/// is shown (the conventional log major-tick form).
fn format_tick_power(value: f64, sig_digits: u8, ls: &LabelStyle) -> RichText {
    let style_from = |segments: Vec<RichSegment>| RichText {
        segments,
        color: ls.color,
        font_size: ls.font_size,
        font: ls.label_font.clone(),
    };

    if value == 0.0 {
        return style_from(vec![plain_seg('0')]);
    }

    let abs = value.abs();
    let exp = abs.log10().floor() as i32;
    let mantissa = value / 10f64.powi(exp);
    let mantissa_close_to_one = (mantissa.abs() - 1.0).abs() < 1e-9;

    let mut segs: Vec<RichSegment> = Vec::new();

    if mantissa_close_to_one {
        if mantissa < 0.0 {
            segs.push(plain_seg('-'));
        }
    } else {
        let sig = sig_digits.max(1) as usize;
        let raw = format!("{:.*}", sig.saturating_sub(1), mantissa);
        let trimmed = trim_trailing_fraction_zeros(&raw);
        for c in trimmed.chars() {
            segs.push(plain_seg(c));
        }
        segs.push(plain_seg('×'));
    }

    // The "10".
    segs.push(plain_seg('1'));
    segs.push(plain_seg('0'));

    // Exponent (every char is superscript).
    let exp_str = format!("{}", exp);
    for c in exp_str.chars() {
        segs.push(sup_seg(c));
    }

    style_from(segs)
}

/// Minimum gap (px) between the end of a tick and its label, applied on top
/// of any user-provided `label_offset_{x,y}`.
const LABEL_GAP: f32 = 4.0;

fn draw_tick_label(
    canvas: &mut Canvas,
    text: &str,
    tick_pos: (f32, f32),
    side: Side,
    axis: &AxisOptions,
    fp: FontPolicy,
) {
    let ls: &LabelStyle = &axis.label_style;
    let m = measure_plain_text(text, &ls.label_font, ls.font_size, false, false, fp);

    // The label's natural anchor sits one tick-length outward (plus a gap).
    // `label_offset_{x,y}` is added on top in screen coordinates.
    let outward = axis.major_tick_length;

    // Pre-offset baseline anchor.
    let (base_x, base_y) = match side {
        Side::Top => (
            tick_pos.0 - m.width * 0.5,
            tick_pos.1 - outward - LABEL_GAP - m.descent,
        ),
        Side::Bottom => (
            tick_pos.0 - m.width * 0.5,
            tick_pos.1 + outward + LABEL_GAP + m.ascent,
        ),
        Side::Left => (
            tick_pos.0 - outward - LABEL_GAP - m.width,
            tick_pos.1 + (m.ascent - m.descent) * 0.5,
        ),
        Side::Right => (
            tick_pos.0 + outward + LABEL_GAP,
            tick_pos.1 + (m.ascent - m.descent) * 0.5,
        ),
    };

    // label_offset is a screen-space translation (same convention for all sides).
    let origin_x = base_x + ls.label_offset_x;
    let origin_y = base_y + ls.label_offset_y;

    draw_plain_text(
        canvas,
        text,
        (origin_x, origin_y),
        ls.color,
        &ls.label_font,
        ls.font_size,
        false,
        false,
        fp,
    );
}

/// Draw a Power-format RichText label, following the same placement rules as
/// `draw_tick_label` but with `measure_rich_text` measurements.
fn draw_tick_label_rich(
    canvas: &mut Canvas,
    rt: &RichText,
    tick_pos: (f32, f32),
    side: Side,
    axis: &AxisOptions,
    fp: FontPolicy,
) {
    let m = measure_rich_text(rt, fp);
    let outward = axis.major_tick_length;

    let (base_x, base_y) = match side {
        Side::Top => (
            tick_pos.0 - m.width * 0.5,
            tick_pos.1 - outward - LABEL_GAP - m.descent,
        ),
        Side::Bottom => (
            tick_pos.0 - m.width * 0.5,
            tick_pos.1 + outward + LABEL_GAP + m.ascent,
        ),
        Side::Left => (
            tick_pos.0 - outward - LABEL_GAP - m.width,
            tick_pos.1 + (m.ascent - m.descent) * 0.5,
        ),
        Side::Right => (
            tick_pos.0 + outward + LABEL_GAP,
            tick_pos.1 + (m.ascent - m.descent) * 0.5,
        ),
    };

    let ls = &axis.label_style;
    let origin = (
        base_x + ls.label_offset_x,
        base_y + ls.label_offset_y,
    );

    draw_rich_text(canvas, rt, origin, fp);
}

// Axis title (RichText).

fn draw_axis_title(canvas: &mut Canvas, config: &Config, da: &DataArea, side: Side, fp: FontPolicy) {
    let axis = match side {
        Side::Top => &config.top_x,
        Side::Bottom => &config.bottom_x,
        Side::Left => &config.left_y,
        Side::Right => &config.right_y,
    };
    let to = &axis.title_option;
    if !to.visible {
        return;
    }

    let m = measure_rich_text(&to.text, fp);
    let ca = &config.chart_area;

    match side {
        // Horizontal text (Top/Bottom): centered horizontally, vertically
        // centered within the band.
        Side::Top => {
            let band_top = ca.y as f32 + config.chart_title.top_margin;
            let baseline = band_top + (axis.out_margin - m.height()) * 0.5 + m.ascent;
            let x = da.x as f32 + da.width as f32 * 0.5 - m.width * 0.5;
            draw_rich_text(
                canvas,
                &to.text,
                (x + to.offset_x, baseline + to.offset_y),
                fp,
            );
        }
        Side::Bottom => {
            let band_top = (ca.y + ca.height) as f32 - axis.out_margin;
            let baseline = band_top + (axis.out_margin - m.height()) * 0.5 + m.ascent;
            let x = da.x as f32 + da.width as f32 * 0.5 - m.width * 0.5;
            draw_rich_text(
                canvas,
                &to.text,
                (x + to.offset_x, baseline + to.offset_y),
                fp,
            );
        }
        // Vertical text: Left (-90° CCW), Right (+90° CW).
        // Rotate around the band center and center the text on that point.
        Side::Left => {
            let center_x = ca.x as f32 + axis.out_margin * 0.5;
            let center_y = da.y as f32 + da.height as f32 * 0.5;
            draw_rotated_centered(canvas, &to.text, (center_x, center_y), -90.0, to, &m, fp);
        }
        Side::Right => {
            let center_x = (ca.x + ca.width) as f32 - axis.out_margin * 0.5;
            let center_y = da.y as f32 + da.height as f32 * 0.5;
            draw_rotated_centered(canvas, &to.text, (center_x, center_y), 90.0, to, &m, fp);
        }
    }
}

// Rotate text by `degrees` around `(cx, cy)` and center it on that point.
// `to.offset_{x,y}` is applied in the pre-rotation local frame (so it stays
// aligned with the text direction after the rotation).
#[allow(clippy::too_many_arguments)]
fn draw_rotated_centered(
    canvas: &mut Canvas,
    rt: &crate::text::RichText,
    center: (f32, f32),
    degrees: f32,
    to: &crate::config::AxisTitleOptions,
    m: &crate::text_render::TextMetrics,
    fp: FontPolicy,
) {
    let (cx, cy) = center;
    canvas.save();
    canvas.rotate_at(degrees, cx, cy);

    // After the rotate, place the text centered on (cx, cy) in the pre-rotation frame.
    let text_x = cx - m.width * 0.5;
    let text_baseline = cy + (m.ascent - m.descent) * 0.5;
    draw_rich_text(
        canvas,
        rt,
        (text_x + to.offset_x, text_baseline + to.offset_y),
        fp,
    );
    canvas.restore();
}

// Chart title (RichText).

fn draw_chart_title(canvas: &mut Canvas, config: &Config, fp: FontPolicy) {
    let ct = &config.chart_title;
    let m = measure_rich_text(&ct.text, fp);
    let ca = &config.chart_area;

    // Vertically centered inside the top_margin band.
    let baseline = ca.y as f32 + (ct.top_margin - m.height()) * 0.5 + m.ascent;
    let x = ca.x as f32 + ca.width as f32 * 0.5 - m.width * 0.5;

    let origin_x = x + ct.offset_x;
    let origin_y = baseline + ct.offset_y;

    draw_rich_text(canvas, &ct.text, (origin_x, origin_y), fp);
}

// Skia Paint helpers.

fn stroke_paint(color: &Color, width: f32, style: &LineStylePreset) -> Paint {
    Paint::stroke(color, width).with_dash(style.pattern())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DrawStyle, SketchOptions};
    use crate::default::default_config;

    /// Majors anchor to absolute spacing multiples (nice values even when
    /// the range ends are arbitrary), and minors cover the partial edge
    /// intervals beyond the outermost majors.
    #[test]
    fn ticks_anchor_to_grid_and_minors_cover_edges() {
        let mut axis = default_config().bottom_x.clone();
        axis.min = 1200.0;
        axis.max = 1950.0;
        axis.major_spacing = 200.0;
        axis.minor_count = 3; // minors every 50

        let majors = major_tick_values(&axis);
        assert_eq!(majors, vec![1200.0, 1400.0, 1600.0, 1800.0]);

        let minors = minor_tick_values(&axis);
        for m in [1850.0, 1900.0, 1950.0] {
            assert!(
                minors.iter().any(|v| (v - m).abs() < 1e-6),
                "edge minor {m} missing: {minors:?}"
            );
        }
        assert!(minors.iter().all(|v| (v / 200.0).fract().abs() > 1e-9),
            "majors leaked into minors: {minors:?}");

        // Arbitrary (uniform-margin) range ends: tick VALUES stay nice.
        axis.min = 0.137;
        axis.max = 2.63;
        axis.major_spacing = 0.5;
        let majors = major_tick_values(&axis);
        assert_eq!(majors, vec![0.5, 1.0, 1.5, 2.0, 2.5]);
    }

    /// One axis, one decimal form: every tick label shares the spacing-derived
    /// decimal count instead of per-value significant digits ("0 / 50.0 /
    /// 100" mixing was the bug).
    #[test]
    fn tick_decimals_follow_spacing_uniformly() {
        assert_eq!(decimals_from_spacing(50.0), 0);
        assert_eq!(decimals_from_spacing(0.5), 1);
        assert_eq!(decimals_from_spacing(0.02), 2);
        assert_eq!(decimals_from_spacing(2.5), 1);

        let f = |v: f64, sp: f64| {
            format_tick_value(v, &LabelFormat::Decimal, 3, &AxisScale::Linear, sp)
        };
        assert_eq!(f(50.0, 50.0), "50");
        assert_eq!(f(100.0, 50.0), "100");
        assert_eq!(f(150.0, 50.0), "150");
        assert_eq!(f(-0.5, 0.5), "-0.5");
        assert_eq!(f(1.5, 0.5), "1.5");
    }

    /// `raster_chart_to_rgba` returns a buffer of the expected size and is
    /// not entirely empty. Checking that anything was actually drawn would
    /// require visual inspection, but a non-zero pixel proves the path ran.
    #[test]
    fn raster_produces_expected_size_and_nonzero() {
        let config = default_config();
        let w = config.chart_area.0.width as usize;
        let h = config.chart_area.0.height as usize;

        let rgba = raster_chart_to_rgba(&config);
        assert_eq!(rgba.len(), w * h * 4, "buffer size must be w*h*4");

        // Background is transparent, so any axis/label pixel must lift alpha above 0.
        let any_opaque = rgba.chunks_exact(4).any(|px| px[3] > 0);
        assert!(any_opaque, "expected at least some non-transparent pixel (axes drawn)");
    }

    /// Selection-blue (b dominant over r and g) — axis chrome is black/gray
    /// (r == g == b), so a blue-dominant pixel can only come from the
    /// selection overlay.
    fn has_selection_blue(rgba: &[u8]) -> bool {
        rgba.chunks_exact(4).any(|px| {
            px[3] > 0 && px[2] > px[0].saturating_add(40) && px[2] > px[1].saturating_add(30)
        })
    }

    #[test]
    fn selection_overlay_rasters_blue_box() {
        use crate::select::{DataAreaElement, Selectable};
        use crate::text_render::CpuTextMeasure;

        let config = default_config();
        let sel = DataAreaElement
            .selection_box(&config, &CpuTextMeasure::for_style(&config.draw_style))
            .expect("data area selection box");

        let plain =
            try_raster_chart_layer_to_rgba(&config, AxisLayerKind::Decoration).unwrap();
        assert!(
            !has_selection_blue(&plain),
            "no blue-dominant pixels expected without a selection overlay"
        );

        let selected = try_raster_chart_layer_to_rgba_with_selection(
            &config,
            AxisLayerKind::Decoration,
            &[sel],
        )
        .unwrap();
        assert!(
            has_selection_blue(&selected),
            "selection overlay must contribute blue-dominant pixels"
        );
    }

    /// The grid layer sits below the data — selection must never draw there.
    #[test]
    fn selection_overlay_skips_grid_layer() {
        use crate::select::{DataAreaElement, Selectable};
        use crate::text_render::CpuTextMeasure;

        let config = default_config();
        let sel = DataAreaElement
            .selection_box(&config, &CpuTextMeasure::for_style(&config.draw_style))
            .unwrap();
        let grid = try_raster_chart_layer_to_rgba_with_selection(
            &config,
            AxisLayerKind::Grid,
            &[sel],
        )
        .unwrap();
        assert!(!has_selection_blue(&grid));
    }

    // Sketch (hand-drawn) mode — CPU deco layer only, no GPU adapter needed.

    /// Sketch-mode test config: legend and minor grid switched on so the
    /// sketched legend border and the dashed (Dot) minor grid lines are
    /// exercised alongside axis lines, ticks, and major grid lines.
    fn sketch_test_config(seed: u32) -> Config {
        let mut cfg = default_config();
        cfg.legend.visible = true;
        cfg.legend.content = RichText::plain("series A", Color::BLACK, 14.0, "");
        cfg.grid.show_minor_x = true;
        cfg.grid.show_minor_y = true;
        cfg.draw_style = DrawStyle::Sketch(SketchOptions { seed, ..SketchOptions::default() });
        cfg
    }

    /// (a) Divergence: enabling sketch mode changes both deco rasters.
    #[test]
    fn sketch_mode_diverges_from_precise() {
        let sketched = sketch_test_config(0);
        let mut precise = sketched.clone();
        precise.draw_style = DrawStyle::Precise;
        for layer in [AxisLayerKind::Grid, AxisLayerKind::Decoration] {
            let a = try_raster_chart_layer_to_rgba(&precise, layer).unwrap();
            let b = try_raster_chart_layer_to_rgba(&sketched, layer).unwrap();
            assert_ne!(a, b, "{layer:?} raster must change when sketch mode is enabled");
        }
    }

    /// (b) Determinism: identical sketch config twice → byte-identical raster.
    #[test]
    fn sketch_mode_is_deterministic() {
        let cfg = sketch_test_config(0);
        for layer in [AxisLayerKind::Grid, AxisLayerKind::Decoration] {
            let a = try_raster_chart_layer_to_rgba(&cfg, layer).unwrap();
            let b = try_raster_chart_layer_to_rgba(&cfg, layer).unwrap();
            assert_eq!(a, b, "{layer:?} sketch raster must be deterministic");
        }
    }

    /// (c) Seed separation: seed 0 vs seed 1 → different wobble pixels.
    #[test]
    fn sketch_seed_changes_raster() {
        for layer in [AxisLayerKind::Grid, AxisLayerKind::Decoration] {
            let a = try_raster_chart_layer_to_rgba(&sketch_test_config(0), layer).unwrap();
            let b = try_raster_chart_layer_to_rgba(&sketch_test_config(1), layer).unwrap();
            assert_ne!(a, b, "{layer:?} raster must depend on the sketch seed");
        }
    }

    /// (§2) The selection overlay never wobbles. The box is placed mid data
    /// area, over pixels that are fully transparent in both modes (deco ink
    /// hugs the data-area border, the legend sits top-right), so its rendered
    /// pixels — footprint AND values — must be byte-identical whether sketch
    /// mode is on or off.
    #[test]
    fn selection_overlay_stays_precise_in_sketch_mode() {
        use crate::layout::RectF;

        let sketched = sketch_test_config(0);
        let mut precise = sketched.clone();
        precise.draw_style = DrawStyle::Precise;

        let sel = SelectionBox {
            rect: RectF { x: 300.0, y: 350.0, width: 120.0, height: 80.0 },
            color: Color { r: 0.0, g: 0.4, b: 1.0, a: 1.0 },
            stroke_width: 2.0,
            handles: vec![RectF { x: 296.0, y: 346.0, width: 8.0, height: 8.0 }],
        };

        let layer = AxisLayerKind::Decoration;
        let base_p = try_raster_chart_layer_to_rgba(&precise, layer).unwrap();
        let with_p =
            try_raster_chart_layer_to_rgba_with_selection(&precise, layer, &[sel.clone()])
                .unwrap();
        let base_s = try_raster_chart_layer_to_rgba(&sketched, layer).unwrap();
        let with_s =
            try_raster_chart_layer_to_rgba_with_selection(&sketched, layer, &[sel]).unwrap();

        // Bytes the selection overlay touched (with vs without selection).
        let footprint = |base: &[u8], with: &[u8]| -> Vec<usize> {
            base.iter()
                .zip(with)
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .map(|(i, _)| i)
                .collect()
        };
        let fp_p = footprint(&base_p, &with_p);
        let fp_s = footprint(&base_s, &with_s);
        assert!(!fp_p.is_empty(), "selection overlay must draw something");
        assert_eq!(fp_p, fp_s, "selection footprint must not move in sketch mode");
        for &i in &fp_p {
            assert_eq!(
                with_p[i], with_s[i],
                "selection ink must be byte-identical in sketch mode (byte {i})"
            );
        }
    }
}
