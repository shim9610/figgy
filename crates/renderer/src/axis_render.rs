//! Axis renderer. Takes a `Config` and draws axes, ticks, labels, and titles
//! onto the CPU raster [`Canvas`]. No data is drawn here — only axis chrome.
//!
//! Text rendering is delegated to [`crate::text_render`].

use crate::raster::{Canvas, Paint};

use crate::color::Color;
use crate::config::{AxisOptions, AxisScale, Config, LabelStyle, TickVisibility};
use crate::format::LabelFormat;
use crate::layout::{
    DataArea, RectF, Side, TitleBand, TitlePlacement, axis_anchor, axis_offset,
    axis_title_placement, chart_title_placement, colorbar_rect, colorbar_title_placement,
    label_origin, legend_rect, point_on_rect_side,
};
use crate::line::LineStylePreset;
use crate::select::SelectionBox;
use crate::sketch::DecoStroker;
use crate::text::{RichSegment, RichText};
use crate::text_render::{
    FontPolicy, draw_plain_text, draw_rich_text, measure_plain_text, measure_rich_text,
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
/// Returns [`crate::FiggyError::InvalidChartArea`] /
/// [`crate::FiggyError::RasterWrapFailed`] instead of panicking.
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
        return Err(crate::FiggyError::InvalidChartArea {
            width: w,
            height: h,
        });
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
    // Milkyway: the axis chrome reads as line-light — bloom it. Runs
    // BEFORE the selection overlay so interaction chrome stays crisp.
    if let crate::config::DrawStyle::Milkyway(c) = &config.draw_style
        && matches!(layer, AxisLayerKind::Decoration | AxisLayerKind::All)
    {
        apply_decoration_glow(&mut canvas, c.glow);
    }
    if !selection.is_empty() && matches!(layer, AxisLayerKind::Decoration | AxisLayerKind::All) {
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
    if let crate::config::DrawStyle::Milkyway(c) = &config.draw_style {
        draw_space_background(canvas, config, c);
        return;
    }
    let Ok(da) = config.data_area() else { return };
    let fp = FontPolicy::for_style(&config.draw_style);
    draw_grid(canvas, config, &da, fp);
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
    c: &crate::config::MilkywayOptions,
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
    let n_dust = (((w as u64 * h as u64) / 1400) as f32 * c.dust.clamp(0.0, 4.0)).max(0.0) as u32;
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
// original. This runs on every decoration re-raster (= every pan / zoom /
// config commit), so it must stay light:
//   - the halo lives at QUARTER resolution (a soft halo can't show the
//     difference; box radius 1 × 2 passes there ≈ the old half-res blur),
//   - the downsample tests each 4×4 block's alpha first — the deco layer is
//     transparent everywhere but the chrome, so most blocks skip the sum,
//   - the upsample-add walks 4-px spans and skips spans whose four
//     contributing quarter-cells are all zero, with 8-bit fixed-point math
//     on the spans that remain.
// Premultiplied-additive with clamp; selection boxes are drawn AFTER this in
// the raster entry, so the interaction overlay stays crisp.
const GLOW_PASSES: u32 = 2;
const GLOW_RADIUS: usize = 1;

pub(crate) fn apply_decoration_glow(canvas: &mut Canvas, gain: f32) {
    let gain = gain.clamp(0.0, 2.0);
    if gain <= 0.0 {
        return;
    }
    let (w, h) = canvas.size();
    if w < 4 || h < 4 {
        return;
    }
    let (w, h) = (w as usize, h as usize);
    let (qw, qh) = (w.div_ceil(4), h.div_ceil(4));
    let src = canvas.pixels_mut();

    // Downsample 4×4 average into the quarter-res halo buffer. Blocks whose
    // alpha is all zero keep the buffer's zero fill without summing.
    let mut halo = vec![0u8; qw * qh * 4];
    let mut tmp = vec![0u8; qw * qh * 4];
    for qy in 0..qh {
        for qx in 0..qw {
            let (x_lo, y_lo) = (qx * 4, qy * 4);
            let (x_hi, y_hi) = ((x_lo + 4).min(w), (y_lo + 4).min(h));
            let mut any = false;
            'probe: for y in y_lo..y_hi {
                let row = y * w * 4;
                for x in x_lo..x_hi {
                    if src[row + x * 4 + 3] != 0 {
                        any = true;
                        break 'probe;
                    }
                }
            }
            if !any {
                continue;
            }
            let mut sum = [0u32; 4];
            let mut n = 0u32;
            for y in y_lo..y_hi {
                let row = y * w * 4;
                for x in x_lo..x_hi {
                    for c in 0..4 {
                        sum[c] += src[row + x * 4 + c] as u32;
                    }
                    n += 1;
                }
            }
            for c in 0..4 {
                halo[(qy * qw + qx) * 4 + c] = (sum[c] / n.max(1)) as u8;
            }
        }
    }

    // Separable box blur ×2 ≈ gaussian, at quarter res.
    for _ in 0..GLOW_PASSES {
        for y in 0..qh {
            let row = y * qw * 4;
            let mut acc = [0u32; 4];
            for x in 0..qw.min(GLOW_RADIUS + 1) {
                for c in 0..4 {
                    acc[c] += halo[row + x * 4 + c] as u32;
                }
            }
            let mut count = qw.min(GLOW_RADIUS + 1) as u32;
            for x in 0..qw {
                for c in 0..4 {
                    tmp[row + x * 4 + c] = (acc[c] / count.max(1)) as u8;
                }
                if x + GLOW_RADIUS + 1 < qw {
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
        for x in 0..qw {
            let mut acc = [0u32; 4];
            for y in 0..qh.min(GLOW_RADIUS + 1) {
                for c in 0..4 {
                    acc[c] += tmp[(y * qw + x) * 4 + c] as u32;
                }
            }
            let mut count = qh.min(GLOW_RADIUS + 1) as u32;
            for y in 0..qh {
                for c in 0..4 {
                    halo[(y * qw + x) * 4 + c] = (acc[c] / count.max(1)) as u8;
                }
                if y + GLOW_RADIUS + 1 < qh {
                    for c in 0..4 {
                        acc[c] += tmp[((y + GLOW_RADIUS + 1) * qw + x) * 4 + c] as u32;
                    }
                    count += 1;
                }
                if y >= GLOW_RADIUS {
                    for c in 0..4 {
                        acc[c] -= tmp[((y - GLOW_RADIUS) * qw + x) * 4 + c] as u32;
                    }
                    count -= 1;
                }
            }
        }
    }

    // Per-cell occupancy of the blurred halo — the span skip below reads
    // this instead of re-testing four bytes per cell per span.
    let mut occupied = vec![false; qw * qh];
    for (cell, occ) in halo.chunks_exact(4).zip(occupied.iter_mut()) {
        *occ = cell.iter().any(|&b| b != 0);
    }

    // Bilinear upsample + additive merge, in 8-bit fixed point. Output is
    // walked in 4-px spans sharing one quarter-cell column pair; a span whose
    // four contributing cells are all zero adds nothing and is skipped — on a
    // chart-sized canvas that's most of the data area.
    let gain_fp = (gain * 256.0) as u32;
    for y in 0..h {
        let y0 = (y / 4).min(qh - 1);
        let y1 = (y0 + 1).min(qh - 1);
        let ty = ((y % 4) * 64) as u32; // fraction in /256
        let (row0, row1) = (y0 * qw, y1 * qw);
        let out_row = y * w * 4;
        let mut x = 0usize;
        while x < w {
            let x0 = (x / 4).min(qw - 1);
            let x1 = (x0 + 1).min(qw - 1);
            let span_end = (x0 * 4 + 4).min(w);
            if !occupied[row0 + x0]
                && !occupied[row0 + x1]
                && !occupied[row1 + x0]
                && !occupied[row1 + x1]
            {
                x = span_end;
                continue;
            }
            let (c00, c01) = ((row0 + x0) * 4, (row0 + x1) * 4);
            let (c10, c11) = ((row1 + x0) * 4, (row1 + x1) * 4);
            while x < span_end {
                let tx = ((x % 4) * 64) as u32;
                for c in 0..4 {
                    let a = halo[c00 + c] as u32 * (256 - tx) + halo[c01 + c] as u32 * tx;
                    let b = halo[c10 + c] as u32 * (256 - tx) + halo[c11 + c] as u32 * tx;
                    let v = (a * (256 - ty) + b * ty) >> 16; // back to 0..=255
                    let i = out_row + x * 4 + c;
                    src[i] = (src[i] as u32 + ((v * gain_fp) >> 8)).min(255) as u8;
                }
                x += 1;
            }
        }
    }
}

/// Decoration layer — axis lines, tick labels, axis titles, chart title,
/// legend. Drawn above the data so data never overlaps axis chrome.
pub fn draw_decoration_layer(canvas: &mut Canvas, config: &Config) {
    let Ok(da) = config.data_area() else { return };

    // The decoration stroke strategy is derived once per layer entry and
    // threaded down: `Precise` keeps every draw below on
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

    draw_colorbar(canvas, config, &da, &stroker, fp);

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

// Colourbar.
//
// No GPU pipeline: the strip is a CPU ramp and its ticks, labels, and title go
// through the *same* helpers the four chart axes use (`draw_tick`,
// `draw_tick_label`, `colorbar_title_placement`, `major_tick_values`,
// `format_tick_value`, `format_tick_power`). That reuse is the point — the bar's
// axis is an `AxisOptions`, so a log colourbar gets decade ticks and 10ⁿ labels
// from the code that already does it for a log axis, instead of a second
// implementation that drifts.

/// Draw the colourbar: strip, border, axis line, ticks, tick labels, title.
///
/// A no-op when there is no colourbar or it is hidden — and `colorbar_parts` has
/// then reserved no band for it, so there is nothing to leave blank.
fn draw_colorbar(
    canvas: &mut Canvas,
    config: &Config,
    da: &DataArea,
    stroker: &DecoStroker,
    fp: FontPolicy,
) {
    let Some(bar) = config.colorbar.as_ref() else {
        return;
    };
    if !bar.visible {
        return;
    }
    let rect = colorbar_rect(&config.chart_area, da, config.chart_title.top_margin, bar);
    if !(rect.width > 0.0 && rect.height > 0.0) {
        return;
    }

    draw_colorbar_strip(canvas, bar, &rect);

    // Border. Precise fill, wobbled outline — the same split as the legend box,
    // for the same reason: perturbing the fill too would visibly disagree with
    // the independently wobbled border.
    if bar.border_width > 0.0 {
        let paint = Paint::stroke(&bar.border_color, bar.border_width);
        stroker.stroke_rect_outline(
            canvas,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            &paint,
            "colorbar_strip",
        );
    }

    let axis = &bar.axis;
    let side = bar.side.clone();

    // Like a chart axis, the colourbar axis chrome may detach perpendicular to
    // its direction. The strip and title stay put; line, ticks, and tick labels
    // share this one transform so they cannot drift from one another.
    let (off_x, off_y) = axis_offset(side.clone(), axis.line_offset);
    let detached = off_x != 0.0 || off_y != 0.0;
    if detached {
        canvas.save();
        canvas.translate(off_x, off_y);
    }

    // The bar's axis runs along the strip's outer long edge — the same side of
    // the strip as the bar is of the chart, which is what makes `draw_tick`'s
    // "outward = away from the data area" come out right.
    let low = point_on_rect_side(0.0, &side, &rect);
    let high = point_on_rect_side(1.0, &side, &rect);
    if axis.line_visible {
        let paint = stroke_paint(&axis.line_color, axis.line_width, &axis.line_style);
        stroker.stroke_segment(canvas, low, high, &paint, "axis_colorbar");
    }

    // Ticks. `major_tick_values` / `minor_tick_values` are the axis walkers, so a
    // logarithmic bar gets decades and 2a..9a minors with no extra code here.
    let majors = major_tick_values(axis);
    let minors = minor_tick_values(axis);
    if axis.tick != TickVisibility::None {
        let tick_paint = stroke_paint(&axis.line_color, axis.line_width, &axis.line_style);
        for (i, value) in majors.iter().chain(minors.iter()).enumerate() {
            let length = if i < majors.len() {
                axis.major_tick_length
            } else {
                axis.minor_tick_length
            };
            let pos = point_on_rect_side(axis_fraction(*value, axis), &side, &rect);
            let tag = format!("tick_colorbar_{i}");
            draw_tick(
                canvas,
                pos,
                side.clone(),
                length,
                &axis.tick,
                &tick_paint,
                stroker,
                &tag,
            );
        }
    }

    // Tick labels. Timestamps are not offered: a z value is a magnitude, and the
    // timestamp planner needs an axis pixel length and collision pruning that
    // belong to a time axis. `LabelFormat::Timestamp` on a colourbar therefore
    // formats through the numeric path rather than being silently dropped.
    let ls = &axis.label_style;
    if ls.visible && ls.label_visible {
        for value in &majors {
            let pos = point_on_rect_side(axis_fraction(*value, axis), &side, &rect);
            match ls.format {
                LabelFormat::Power => {
                    let rt = format_tick_power(*value, ls.significant_digits, ls);
                    draw_tick_label_rich(canvas, &rt, pos, side.clone(), axis, fp);
                }
                _ => {
                    let text = format_tick_value(
                        *value,
                        &ls.format,
                        ls.significant_digits,
                        &axis.scale,
                        effective_major_spacing(axis),
                        axis.min,
                        axis.max,
                    );
                    draw_tick_label(canvas, &text, pos, side.clone(), axis, fp);
                }
            }
        }
    }

    if detached {
        canvas.restore();
    }

    // Title. Its anchor is the painted strip rather than the full data area,
    // so shortening, aligning, dragging, or resizing the bar moves the title
    // with it. The title keeps its own offset and does not follow a separately
    // detached axis line.
    let to = &axis.title_option;
    if to.visible {
        let m = measure_rich_text(&to.text, fp);
        let placement = colorbar_title_placement(side, &rect, axis, (to.offset_x, to.offset_y), m);
        draw_placed_title(canvas, &to.text, placement, fp);
    }
}

/// Paint the ramp into the strip rect.
///
/// One band per device pixel along the bar, coloured by `ColorMap::sample` at
/// the band's centre — the ramp is a continuous function, so there is no reason
/// for the strip to show coarser steps than the display can.
///
/// `field_columnar.wgsl` reimplements `ColorMap::sample` over the same stops and
/// takes its `t` from the same `ColorBarOptions` range, so a field cell and the
/// strip position naming its value come out the *same colour* rather than nearly
/// so. `heatmap_render.rs` asserts that against `color_for_z` directly.
///
/// `t` runs from the `axis.min` end of the bar. `axis.inverted` swaps which
/// screen end that is — it does not reverse the ramp, because inverting an axis
/// changes where a value is drawn and not what colour it has.
fn draw_colorbar_strip(canvas: &mut Canvas, bar: &crate::config::ColorBarOptions, rect: &RectF) {
    let vertical = matches!(bar.side, Side::Left | Side::Right);
    let length = if vertical { rect.height } else { rect.width };
    // One band per device pixel, sized so no band is *thinner* than a pixel:
    // `floor`, not `ceil`, spreads the remainder over the bands instead of
    // leaving sub-pixel slivers to be anti-aliased into their neighbours. At
    // least one band, so a sub-pixel strip still shows a colour.
    let bands = (length.floor() as usize).max(1);
    let step = length / bands as f32;

    for i in 0..bands {
        // Screen fraction of this band's centre, measured from the low end of
        // the axis (bottom for a vertical bar, left for a horizontal one).
        let along = (i as f32 + 0.5) / bands as f32;
        let screen_fraction = if vertical { 1.0 - along } else { along };
        let value_fraction = if bar.axis.inverted {
            1.0 - screen_fraction
        } else {
            screen_fraction
        };
        let paint = Paint::fill(&bar.colormap.sample(value_fraction));
        let offset = i as f32 * step;
        if vertical {
            canvas.draw_rect(rect.x, rect.y + offset, rect.width, step, &paint);
        } else {
            canvas.draw_rect(rect.x + offset, rect.y, step, rect.height, &paint);
        }
    }
}

// Contour labels.
//
// The *placement* of an inline level label is a GPU fact — the anchor pass
// projects seed points onto the same implicit field the line uses — but the
// label's *content* is `levels[i]`, which is config. The CPU therefore bakes the
// strings once per signature and the GPU only places the resulting images.
//
// Baking whole strings rather than glyphs is what keeps this in the existing text
// stack: `measure_plain_text` / `draw_plain_text` / `FontPolicy` handle
// per-character fallback and the sketch face exactly as they do for a tick label.
// There is no second text stack and no GPU glyph rasterizer.
//
// The label is drawn *over* the line. matplotlib erases the stroke under an
// inline label; here `bg_color` paints behind the text instead, which needs no
// change to the contour draw at all — and unlike the chart background, the
// label's own background is something the label knows (design B.4.7).

/// One level's cell in the baked label atlas.
pub(crate) struct LabelAtlasCell {
    /// Content width in texels. The quad is drawn this wide and samples only
    /// this far into the cell, so a short string is not stretched.
    pub width: f32,
}

/// A baked contour-label atlas: one uniformly-strided grid cell per level.
pub(crate) struct BakedLabelAtlas {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Drawn label height inside each cell, excluding the transparent gutter.
    pub cell_h: f32,
    pub cell_stride_w: u32,
    pub cell_stride_h: u32,
    pub columns: u32,
    pub rows: u32,
    pub gutter: u32,
    pub cells: Vec<LabelAtlasCell>,
}

/// Why a requested contour-label atlas could not be baked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContourLabelAtlasError {
    /// The uniform cell grid cannot fit in the device's 2D texture limit.
    DeviceLimit {
        level_count: u64,
        capacity: u64,
        cell_stride_w: u32,
        cell_stride_h: u32,
        max_dimension: u32,
    },
    /// The dimensions passed the device limit, but the CPU raster allocation
    /// failed before upload.
    AllocationFailed { width: u32, height: u32 },
}

impl std::fmt::Display for ContourLabelAtlasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DeviceLimit {
                level_count,
                capacity,
                cell_stride_w,
                cell_stride_h,
                max_dimension,
            } => write!(
                f,
                "{level_count} contour label cells of stride {cell_stride_w}x{cell_stride_h} \
                 exceed a {max_dimension}x{max_dimension} texture (capacity {capacity})"
            ),
            Self::AllocationFailed { width, height } => {
                write!(
                    f,
                    "could not allocate a {width}x{height} contour label atlas"
                )
            }
        }
    }
}

impl std::error::Error for ContourLabelAtlasError {}

const CONTOUR_LABEL_ATLAS_GUTTER: u32 = 1;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct LabelAtlasGrid {
    columns: u32,
    rows: u32,
    width: u32,
    height: u32,
}

fn contour_label_atlas_grid(
    level_count: usize,
    cell_stride_w: u32,
    cell_stride_h: u32,
    max_dimension: u32,
) -> Result<LabelAtlasGrid, ContourLabelAtlasError> {
    let levels = level_count as u64;
    let max_columns = max_dimension.checked_div(cell_stride_w).unwrap_or(0);
    let max_rows = max_dimension.checked_div(cell_stride_h).unwrap_or(0);
    let capacity = u64::from(max_columns) * u64::from(max_rows);
    if levels == 0 || levels > capacity {
        return Err(ContourLabelAtlasError::DeviceLimit {
            level_count: levels,
            capacity,
            cell_stride_w,
            cell_stride_h,
            max_dimension,
        });
    }

    // Choose the valid grid whose larger texel dimension is smallest. The
    // search is bounded by the device dimension (normally <= 16384), not by the
    // number of labels, and runs only when the atlas signature changes.
    let mut best: Option<(u32, u64, u32, u32)> = None;
    let candidate_columns = u64::from(max_columns).min(levels) as u32;
    for columns in 1..=candidate_columns {
        let rows = levels.div_ceil(u64::from(columns));
        if rows > u64::from(max_rows) {
            continue;
        }
        let rows = rows as u32;
        let width = columns * cell_stride_w;
        let height = rows * cell_stride_h;
        let score = width.max(height);
        let area = u64::from(width) * u64::from(height);
        if best
            .as_ref()
            .is_none_or(|(best_score, best_area, _, _)| (score, area) < (*best_score, *best_area))
        {
            best = Some((score, area, columns, rows));
        }
    }
    let (_, _, columns, rows) = best.expect("capacity check guarantees a valid label grid");
    Ok(LabelAtlasGrid {
        columns,
        rows,
        width: columns * cell_stride_w,
        height: rows * cell_stride_h,
    })
}

/// The text of every level's label, formatted the way a tick label is.
///
/// Separated from the bake so the signature can be compared against the strings
/// themselves: two configs that format to the same numbers need no new atlas.
pub(crate) fn contour_label_texts(
    config: &Config,
    labels: &crate::data_config::ContourLabelConfig,
    levels: &[f64],
) -> Vec<String> {
    let mut finite_levels: Vec<f64> = levels
        .iter()
        .copied()
        .filter(|level| level.is_finite())
        .collect();
    finite_levels.sort_by(f64::total_cmp);
    let level_spacing = finite_levels
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|spacing| spacing.is_finite() && *spacing > 0.0)
        .min_by(f64::total_cmp);
    let colorbar_axis = config.colorbar.as_ref().map(|bar| &bar.axis);
    let spacing = level_spacing
        .or_else(|| colorbar_axis.map(effective_major_spacing))
        .unwrap_or_else(|| {
            finite_levels
                .first()
                .copied()
                .unwrap_or(1.0)
                .abs()
                .max(f64::MIN_POSITIVE)
        });
    let range_min = finite_levels
        .first()
        .copied()
        .or_else(|| colorbar_axis.map(|axis| axis.min))
        .unwrap_or(0.0);
    let range_max = finite_levels
        .last()
        .copied()
        .or_else(|| colorbar_axis.map(|axis| axis.max))
        .unwrap_or(range_min);
    levels
        .iter()
        .map(|level| {
            format_tick_value(
                *level,
                &labels.format,
                labels.significant_digits,
                &AxisScale::Linear,
                spacing,
                range_min,
                range_max,
            )
        })
        .collect()
}

/// Bake one contour series' labels into an atlas.
///
/// `scale` is the render scale — the same DPI multiplier every other pixel
/// dimension gets — so an export at 2x re-bakes at 2x rather than magnifying a
/// window-resolution atlas.
///
/// Returns `Ok(None)` when there is nothing to draw: no visible label, a
/// non-positive font size, or every string empty. Device-limit and allocation
/// failures are explicit so the renderer can fail the frame with context rather
/// than silently drawing an unlabelled series.
pub(crate) fn bake_contour_label_atlas(
    config: &Config,
    labels: &crate::data_config::ContourLabelConfig,
    texts: &[String],
    scale: f32,
    max_dimension: u32,
) -> Result<Option<BakedLabelAtlas>, ContourLabelAtlasError> {
    if !labels.visible || labels.font_size <= 0.0 || texts.is_empty() {
        return Ok(None);
    }
    let fp = FontPolicy::for_style(&config.draw_style);
    // Contour labels are data annotations, so they take the chart's tick-label
    // face; `FontPolicy` still swaps it in sketch mode.
    let family = config.bottom_x.label_style.label_font.clone();
    let font_size = labels.font_size * scale;
    // Padding also defines the transparent line break around a label, so it is
    // geometric even when no background colour is painted.
    let pad = labels.bg_padding_px.max(0.0) * scale;

    // One measure pass first: the cell height is uniform across levels (the
    // tallest), so it cannot be known before every string has been measured.
    let measured: Vec<_> = texts
        .iter()
        .map(|text| measure_plain_text(text, &family, font_size, false, false, fp))
        .collect();
    let mut cell_w = 0.0f32;
    let mut ascent = 0.0f32;
    let mut descent = 0.0f32;
    for m in &measured {
        cell_w = cell_w.max(m.width);
        ascent = ascent.max(m.ascent);
        descent = descent.max(m.descent);
    }
    if cell_w <= 0.0 {
        return Ok(None);
    }
    let cell_h = (ascent + descent + 2.0 * pad).ceil().max(1.0);
    let content_w = (cell_w + 2.0 * pad).ceil().max(1.0) as u32;
    let content_h = cell_h as u32;
    let gutter = CONTOUR_LABEL_ATLAS_GUTTER;
    let cell_stride_w = content_w.saturating_add(2 * gutter);
    let cell_stride_h = content_h.saturating_add(2 * gutter);
    let grid = contour_label_atlas_grid(texts.len(), cell_stride_w, cell_stride_h, max_dimension)?;
    let mut canvas =
        Canvas::new(grid.width, grid.height).ok_or(ContourLabelAtlasError::AllocationFailed {
            width: grid.width,
            height: grid.height,
        })?;

    let mut cells = Vec::with_capacity(texts.len());
    for (index, (text, m)) in texts.iter().zip(measured.iter()).enumerate() {
        let content = m.width + 2.0 * pad;
        cells.push(LabelAtlasCell { width: content });
        if text.is_empty() {
            continue;
        }
        let column = index as u32 % grid.columns;
        let row = index as u32 / grid.columns;
        let left = (column * cell_stride_w + gutter) as f32;
        let top = (row * cell_stride_h + gutter) as f32;
        if let Some(bg) = labels.bg_color.as_ref() {
            canvas.draw_rect(left, top, content, cell_h, &Paint::fill(bg));
        }
        draw_plain_text(
            &mut canvas,
            text,
            (left + pad, top + pad + ascent),
            labels.color,
            &family,
            font_size,
            false,
            false,
            fp,
        );
    }

    Ok(Some(BakedLabelAtlas {
        rgba: canvas.into_rgba(),
        width: grid.width,
        height: grid.height,
        cell_h,
        cell_stride_w,
        cell_stride_h,
        columns: grid.columns,
        rows: grid.rows,
        gutter,
        cells,
    }))
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
    let lg = &config.legend;
    if !lg.visible || lg.content.segments.is_empty() {
        return;
    }

    let m = measure_rich_text(&lg.content, fp);
    let rect = legend_rect(da, lg.corner, lg.padding, (lg.offset_x, lg.offset_y), m);
    let (box_x, box_y, box_w, box_h) = (rect.x, rect.y, rect.width, rect.height);

    // Box background + border. The sketch stroker wobbles only the border
    // outline; the fill stays a precise rect — perturbing the fill too
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

// Decoration stroke plumbing.
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

fn draw_grid(canvas: &mut Canvas, config: &Config, da: &DataArea, fp: FontPolicy) {
    let g = &config.grid;
    let stroker = DecoStroker::from_style(&config.draw_style);
    let x_top = da.y as f32;
    let x_bot = (da.y + da.height) as f32;
    let y_left = da.x as f32;
    let y_right = (da.x + da.width) as f32;

    // Draw minor first so major can paint over it.
    if g.show_minor_x {
        let paint = stroke_paint(&g.minor_x_color, g.minor_x_width, &g.minor_x_style);
        for (i, v) in minor_tick_values_for_axis(&config.bottom_x, Side::Bottom, da, fp)
            .into_iter()
            .enumerate()
        {
            let pos = value_to_screen(v, &config.bottom_x, Side::Bottom, da);
            let tag = format!("grid_minor_x_{i}");
            stroker.stroke_segment(canvas, (pos.0, x_top), (pos.0, x_bot), &paint, &tag);
        }
    }
    if g.show_minor_y {
        let paint = stroke_paint(&g.minor_y_color, g.minor_y_width, &g.minor_y_style);
        for (i, v) in minor_tick_values_for_axis(&config.left_y, Side::Left, da, fp)
            .into_iter()
            .enumerate()
        {
            let pos = value_to_screen(v, &config.left_y, Side::Left, da);
            let tag = format!("grid_minor_y_{i}");
            stroker.stroke_segment(canvas, (y_left, pos.1), (y_right, pos.1), &paint, &tag);
        }
    }

    if g.show_major_x {
        let paint = stroke_paint(&g.major_x_color, g.major_x_width, &g.major_x_style);
        for (i, v) in major_tick_values_for_axis(&config.bottom_x, Side::Bottom, da, fp)
            .into_iter()
            .enumerate()
        {
            let pos = value_to_screen(v, &config.bottom_x, Side::Bottom, da);
            let tag = format!("grid_major_x_{i}");
            stroker.stroke_segment(canvas, (pos.0, x_top), (pos.0, x_bot), &paint, &tag);
        }
    }
    if g.show_major_y {
        let paint = stroke_paint(&g.major_y_color, g.major_y_width, &g.major_y_style);
        for (i, v) in major_tick_values_for_axis(&config.left_y, Side::Left, da, fp)
            .into_iter()
            .enumerate()
        {
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
    let (off_x, off_y) = axis_offset(side.clone(), axis.line_offset);
    let detached = off_x != 0.0 || off_y != 0.0;
    if detached {
        canvas.save();
        canvas.translate(off_x, off_y);
    }

    let (p0, p1) = axis_anchor(side.clone(), da);

    // 1) Axis line.
    if axis.line_visible {
        let paint = stroke_paint(&axis.line_color, axis.line_width, &axis.line_style);
        let tag = format!("axis_{}", side_tag(&side));
        stroker.stroke_segment(canvas, p0, p1, &paint, &tag);
    }

    // 2) Major / minor ticks
    let majors = major_tick_values_for_axis(axis, side.clone(), da, fp);
    let minors = minor_tick_values_for_axis(axis, side.clone(), da, fp);

    if axis.tick != TickVisibility::None {
        let tick_paint = stroke_paint(&axis.line_color, axis.line_width, &axis.line_style);
        // Tick stroker tags share one running index per side (majors first,
        // minors after) so every tick gets its own wobble shape.
        for (i, v) in majors.iter().enumerate() {
            let pos = value_to_screen(*v, axis, side.clone(), da);
            let tag = format!("tick_{}_{i}", side_tag(&side));
            draw_tick(
                canvas,
                pos,
                side.clone(),
                axis.major_tick_length,
                &axis.tick,
                &tick_paint,
                stroker,
                &tag,
            );
        }
        for (i, v) in minors.iter().enumerate() {
            let pos = value_to_screen(*v, axis, side.clone(), da);
            let tag = format!("tick_{}_{}", side_tag(&side), majors.len() + i);
            draw_tick(
                canvas,
                pos,
                side.clone(),
                axis.minor_tick_length,
                &axis.tick,
                &tick_paint,
                stroker,
                &tag,
            );
        }
    }

    // 3) Major tick labels
    let ls = &axis.label_style;
    if ls.visible && ls.label_visible {
        if let LabelFormat::Timestamp(_) = &ls.format {
            for label in visible_timestamp_labels(axis, side.clone(), da, fp, &majors) {
                let pos = value_to_screen(label.value, axis, side.clone(), da);
                draw_tick_label(canvas, &label.text, pos, side.clone(), axis, fp);
            }
        } else {
            for v in &majors {
                let pos = value_to_screen(*v, axis, side.clone(), da);
                match ls.format {
                    LabelFormat::Power => {
                        let rt = format_tick_power(*v, ls.significant_digits, ls);
                        draw_tick_label_rich(canvas, &rt, pos, side.clone(), axis, fp);
                    }
                    _ => {
                        // Decimals must derive from the spacing the walker
                        // actually used, or a guarded fallback would emit ticks
                        // at 0.5 steps with 0-decimal labels.
                        let text = format_tick_value(
                            *v,
                            &ls.format,
                            ls.significant_digits,
                            &axis.scale,
                            effective_major_spacing(axis),
                            axis.min,
                            axis.max,
                        );
                        draw_tick_label(canvas, &text, pos, side.clone(), axis, fp);
                    }
                }
            }
        }
    }

    if detached {
        canvas.restore();
    }
}

/// Where `value` falls along its axis, as a fraction of the axis' screen length
/// measured from the axis' visual start, `inverted` included.
///
/// **Not clamped**: a tick outside the range lands outside the axis, which is
/// what puts a stale tick visibly off the end instead of piled at it. The
/// colourbar's `ColorBarOptions::normalized_z` is the clamped z→colour twin, and
/// `colorbar_ticks_match_the_strip_ramp` pins the two to agree in range.
fn axis_fraction(value: f64, axis: &AxisOptions) -> f32 {
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
            let (min, max) = crate::chart::guarded_log_range(axis.min, axis.max);
            let log_min = min.log10();
            let log_max = max.log10();
            let range = log_max - log_min;
            if range == 0.0 || value <= 0.0 {
                0.0
            } else {
                (value.log10() - log_min) / range
            }
        }
    };
    let t = if axis.inverted { 1.0 - t } else { t };
    t as f32
}

fn data_area_rect(da: &DataArea) -> RectF {
    RectF {
        x: da.x as f32,
        y: da.y as f32,
        width: da.width as f32,
        height: da.height as f32,
    }
}

fn value_to_screen(value: f64, axis: &AxisOptions, side: Side, da: &DataArea) -> (f32, f32) {
    point_on_rect_side(axis_fraction(value, axis), &side, &data_area_rect(da))
}

/// Hard ceiling on majors the linear walker may emit. A finite-but-tiny
/// spacing (a mis-typed magnitude arriving through the SSoT) would otherwise
/// walk the whole range in near-zero steps — millions of pushes on the
/// raster path, which reads as a frozen chart.
const MAX_MAJOR_INTERVALS: usize = 1000;
const MAX_MINOR_SUBDIVISIONS: usize = 100;
const MAX_LOG_DECADES: usize = 1024;
const GRID_EPSILON: f64 = 1.0e-9;

fn range_in_spacing_units(axis: &AxisOptions, spacing: f64) -> Option<f64> {
    if !walkable_range(axis) || !spacing.is_finite() || spacing <= 0.0 {
        return None;
    }
    let span = axis.max - axis.min;
    let units = if span.is_finite() {
        span / spacing
    } else {
        // Divide before subtracting so `-MAX..MAX` can still be measured when
        // the requested spacing is large enough to make the walk bounded.
        axis.max / spacing - axis.min / spacing
    };
    (units.is_finite() && units >= 0.0).then_some(units)
}

fn spacing_advances_range(axis: &AxisOptions, spacing: f64) -> bool {
    let forward = axis.min + spacing;
    let backward = axis.max - spacing;
    (forward.is_finite() && forward > axis.min) || (backward.is_finite() && backward < axis.max)
}

fn automatic_linear_spacing(axis: &AxisOptions) -> f64 {
    if !walkable_range(axis) {
        return 1.0;
    }
    let span = axis.max - axis.min;
    let mut spacing = if span.is_finite() {
        crate::chart::nice_spacing(span)
    } else {
        let scale = axis.min.abs().max(axis.max.abs());
        let normalized_span = axis.max / scale - axis.min / scale;
        scale * (normalized_span / 8.0)
    };
    if !spacing.is_finite() || spacing <= 0.0 {
        spacing = if span.is_finite() && span > 0.0 {
            span
        } else {
            1.0
        };
    }
    if !spacing_advances_range(axis, spacing) && span.is_finite() && span > 0.0 {
        spacing = span;
    }
    spacing
}

/// The major spacing the raster actually walks. The SSoT accepts whatever
/// the host sends (a live spacing input passes through 0 mid-edit), so the
/// guard lives here — in the one place shared by tick marks, grid lines,
/// minors, and label decimals, which therefore can never disagree. A
/// non-finite or ≤ 0 spacing, or one that would emit an absurd tick count,
/// falls back to the auto spacing for the current range instead of blanking
/// the axis. Callers must have validated the range as finite and non-empty.
pub(crate) fn effective_major_spacing(axis: &AxisOptions) -> f64 {
    let sp = axis.major_spacing;
    if range_in_spacing_units(axis, sp).is_some_and(|units| units <= MAX_MAJOR_INTERVALS as f64)
        && spacing_advances_range(axis, sp)
    {
        return sp;
    }
    automatic_linear_spacing(axis)
}

/// Range a tick walker may iterate: finite ends, positive span. NaN/±inf
/// ends otherwise slip past plain `max <= min` comparisons and saturate the
/// integer step bounds (`inf as i64` = i64::MAX — an unbounded walk).
fn walkable_range(axis: &AxisOptions) -> bool {
    axis.min.is_finite() && axis.max.is_finite() && axis.max > axis.min
}

fn linear_grid_values(axis: &AxisOptions, spacing: f64, max_intervals: usize) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    if !walkable_range(axis)
        || !spacing.is_finite()
        || spacing <= 0.0
        || !spacing_advances_range(axis, spacing)
    {
        return out;
    }
    let first = ((axis.min / spacing) - GRID_EPSILON).ceil();
    let last = ((axis.max / spacing) + GRID_EPSILON).floor();
    if !first.is_finite() || !last.is_finite() || last < first {
        return out;
    }

    for offset in 0..=max_intervals {
        let index = first + offset as f64;
        if !index.is_finite() || index > last {
            break;
        }
        let value = index * spacing;
        if !value.is_finite() {
            continue;
        }
        if out
            .last()
            .is_some_and(|(_, previous): &(f64, f64)| *previous == value)
        {
            continue;
        }
        out.push((index, value));
    }
    out
}

fn log_exponent_bounds(axis: &AxisOptions) -> (i32, i32) {
    let (min, max) = crate::chart::guarded_log_range(axis.min, axis.max);
    let first = min
        .log10()
        .ceil()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    let last = max
        .log10()
        .floor()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    (first, last)
}

fn effective_log_decade_step(axis: &AxisOptions) -> i32 {
    let step = if axis.major_spacing.is_finite() {
        axis.major_spacing.max(1.0).floor()
    } else {
        1.0
    };
    step.min(f64::from(i32::MAX)) as i32
}

fn major_tick_values_for_axis(
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    fp: FontPolicy,
) -> Vec<f64> {
    if let Some(plan) = timestamp_tick_plan(axis, side, da, fp) {
        return plan.majors.into_iter().map(|tick| tick.value).collect();
    }
    major_tick_values(axis)
}

fn minor_tick_values_for_axis(
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    fp: FontPolicy,
) -> Vec<f64> {
    if let Some(plan) = timestamp_tick_plan(axis, side, da, fp) {
        return plan.minor_values;
    }
    minor_tick_values(axis)
}

const LABEL_COLLISION_GAP: f32 = 8.0;

fn timestamp_tick_plan(
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    fp: FontPolicy,
) -> Option<crate::time_axis::TimestampTickPlan> {
    let ls = &axis.label_style;
    crate::time_axis::build_timestamp_tick_plan(
        axis,
        axis_pixel_len(side.clone(), da),
        LABEL_COLLISION_GAP,
        |text| {
            let m = measure_plain_text(text, &ls.label_font, ls.font_size, false, false, fp);
            match side {
                Side::Top | Side::Bottom => m.width,
                Side::Left | Side::Right => m.height(),
            }
        },
    )
}

fn axis_pixel_len(side: Side, da: &DataArea) -> f32 {
    match side {
        Side::Top | Side::Bottom => da.width as f32,
        Side::Left | Side::Right => da.height as f32,
    }
}

fn major_tick_values(axis: &AxisOptions) -> Vec<f64> {
    let mut out = Vec::new();
    match axis.scale {
        AxisScale::Linear => {
            if !walkable_range(axis) {
                return out;
            }
            // Anchor ticks to ABSOLUTE multiples of the spacing, not to
            // axis.min: uniform-margin fits make the range ends arbitrary
            // (0.137…), but tick values must stay nice regardless.
            out.extend(
                linear_grid_values(axis, effective_major_spacing(axis), MAX_MAJOR_INTERVALS)
                    .into_iter()
                    .map(|(_, value)| value),
            );
        }
        AxisScale::Logarithmic => {
            // major_spacing = decade step.
            let (start_exp, end_exp) = log_exponent_bounds(axis);
            let step = effective_log_decade_step(axis);
            let mut exponent = start_exp;
            for _ in 0..MAX_LOG_DECADES {
                if exponent > end_exp {
                    break;
                }
                let value = 10f64.powi(exponent);
                if value.is_finite() && value > 0.0 {
                    out.push(value);
                }
                let Some(next) = exponent.checked_add(step) else {
                    break;
                };
                if next <= exponent {
                    break;
                }
                exponent = next;
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
    if axis.minor_count == 0 {
        return out;
    }
    match axis.scale {
        AxisScale::Linear => {
            // The subdivision count is SSoT input too — clamp it so the
            // walk stays O(majors × subdivisions) no matter what arrives.
            if !walkable_range(axis) {
                return out;
            }
            let subdivisions = axis.minor_count.min(MAX_MINOR_SUBDIVISIONS - 1) + 1;
            let step = effective_major_spacing(axis) / subdivisions as f64;
            let max_intervals = MAX_MAJOR_INTERVALS
                .saturating_mul(subdivisions)
                .saturating_add(1);
            for (index, value) in linear_grid_values(axis, step, max_intervals) {
                // Multiples of the major spacing are the majors themselves.
                if index.rem_euclid(subdivisions as f64) == 0.0 {
                    continue;
                }
                out.push(value);
            }
        }
        AxisScale::Logarithmic => {
            // 2a..9a for EVERY decade overlapping the range, anchored at the
            // decade powers — not at the majors, so partial edge decades
            // (and multi-decade major steps) keep their minors.
            let (min, max) = crate::chart::guarded_log_range(axis.min, axis.max);
            let mut exponent =
                min.log10()
                    .floor()
                    .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
            let last_decade =
                max.log10()
                    .floor()
                    .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
            for _ in 0..MAX_LOG_DECADES {
                if exponent > last_decade {
                    break;
                }
                let a = 10f64.powi(exponent);
                for k in 2..=9 {
                    let v = a * (k as f64);
                    if v.is_finite() && v >= min && v <= max {
                        out.push(v);
                    }
                }
                let Some(next) = exponent.checked_add(1) else {
                    break;
                };
                exponent = next;
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
    d.min(15)
}

/// Decimal places needed to make `significant_digits` observable without
/// mixing a different precision at every tick. The largest magnitude in the
/// label set chooses one shared decimal count; spacing may raise it further so
/// adjacent values can never collapse to the same string.
fn decimals_from_significant_digits(sig_digits: u8, range_min: f64, range_max: f64) -> usize {
    let sig = sig_digits.clamp(1, 15) as i32;
    let max_abs = range_min.abs().max(range_max.abs());
    if !max_abs.is_finite() || max_abs == 0.0 {
        return sig.saturating_sub(1) as usize;
    }
    let order = max_abs.log10().floor() as i32;
    (sig - 1 - order).clamp(0, 15) as usize
}

fn format_tick_value(
    value: f64,
    format: &LabelFormat,
    sig_digits: u8,
    scale: &AxisScale,
    major_spacing: f64,
    range_min: f64,
    range_max: f64,
) -> String {
    // Log scale: ignore sig_digits padding and use a minimal form.
    if matches!(scale, AxisScale::Logarithmic) {
        if value == 0.0 {
            return "0".into();
        }
        return match format {
            LabelFormat::Decimal | LabelFormat::Power => format!("{}", value),
            LabelFormat::Scientific => format!("{:e}", value),
            LabelFormat::Timestamp(_) => format!("{}", value),
        };
    }

    let sig = sig_digits.clamp(1, 15) as usize;
    match format {
        LabelFormat::Scientific => format!("{:.*e}", sig.saturating_sub(1), value),
        LabelFormat::Timestamp(cfg) => crate::time_axis::format_timestamp_numeric_tick(
            value,
            cfg,
            value - major_spacing,
            value + major_spacing,
            major_spacing,
        )
        .unwrap_or_else(|| format!("{}", value)),
        // Power uses the RichText path; if we end up here, treat as Decimal.
        // One uniform form for the whole axis/level set. `significant_digits`
        // controls requested precision while spacing is the correctness floor:
        // a 0.5 level interval must never format both neighbours as integers.
        LabelFormat::Decimal | LabelFormat::Power => {
            if value == 0.0 {
                return "0".into();
            }
            let decimals = decimals_from_spacing(major_spacing).max(
                decimals_from_significant_digits(sig_digits, range_min, range_max),
            );
            format!("{:.*}", decimals, value)
        }
    }
}

#[derive(Debug, Clone)]
struct PlainTickLabel {
    value: f64,
    text: String,
}

fn visible_timestamp_labels(
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    fp: FontPolicy,
    majors: &[f64],
) -> Vec<PlainTickLabel> {
    let Some(cfg) = crate::time_axis::timestamp_format(axis) else {
        return Vec::new();
    };
    let labels: Vec<PlainTickLabel> =
        if let Some(plan) = timestamp_tick_plan(axis, side.clone(), da, fp) {
            plan.majors
                .into_iter()
                .map(|tick| PlainTickLabel {
                    value: tick.value,
                    text: tick.label,
                })
                .collect()
        } else {
            majors
                .iter()
                .filter_map(|v| {
                    crate::time_axis::format_timestamp_numeric_tick(
                        *v,
                        cfg,
                        axis.min,
                        axis.max,
                        effective_major_spacing(axis),
                    )
                    .map(|text| PlainTickLabel { value: *v, text })
                })
                .collect()
        };

    prune_overlapping_plain_labels(axis, side, da, fp, labels)
}

fn prune_overlapping_plain_labels(
    axis: &AxisOptions,
    side: Side,
    da: &DataArea,
    fp: FontPolicy,
    labels: Vec<PlainTickLabel>,
) -> Vec<PlainTickLabel> {
    let mut intervals: Vec<(f32, f32, PlainTickLabel)> = labels
        .into_iter()
        .map(|label| {
            let pos = value_to_screen(label.value, axis, side.clone(), da);
            let (start, end) = plain_tick_label_interval(&label.text, pos, side.clone(), axis, fp);
            (start, end, label)
        })
        .filter(|(start, end, _)| start.is_finite() && end.is_finite())
        .collect();
    intervals.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut out = Vec::new();
    let mut last_end = f32::NEG_INFINITY;
    for (start, end, label) in intervals {
        if start >= last_end + LABEL_COLLISION_GAP {
            last_end = end;
            out.push(label);
        }
    }
    out
}

fn plain_tick_label_interval(
    text: &str,
    tick_pos: (f32, f32),
    side: Side,
    axis: &AxisOptions,
    fp: FontPolicy,
) -> (f32, f32) {
    let ls: &LabelStyle = &axis.label_style;
    let m = measure_plain_text(text, &ls.label_font, ls.font_size, false, false, fp);
    let (origin_x, origin_y) = label_origin(
        side.clone(),
        tick_pos,
        axis.major_tick_length,
        (ls.label_offset_x, ls.label_offset_y),
        m,
    );
    match side {
        Side::Top | Side::Bottom => (origin_x, origin_x + m.width),
        Side::Left | Side::Right => (origin_y - m.ascent, origin_y + m.descent),
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

    let (origin_x, origin_y) = label_origin(
        side,
        tick_pos,
        axis.major_tick_length,
        (ls.label_offset_x, ls.label_offset_y),
        m,
    );

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
    let ls = &axis.label_style;
    let origin = label_origin(
        side,
        tick_pos,
        axis.major_tick_length,
        (ls.label_offset_x, ls.label_offset_y),
        m,
    );

    draw_rich_text(canvas, rt, origin, fp);
}

// Axis title (RichText).

fn draw_axis_title(
    canvas: &mut Canvas,
    config: &Config,
    da: &DataArea,
    side: Side,
    fp: FontPolicy,
) {
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
    let placement = axis_title_placement(
        side.clone(),
        &config.chart_area,
        da,
        TitleBand {
            out_margin: axis.out_margin,
            chart_title_margin: config.chart_title.top_margin,
            edge_inset: config.colorbar_band(&side),
        },
        (to.offset_x, to.offset_y),
        m,
    );
    draw_placed_title(canvas, &to.text, placement, fp);
}

fn draw_placed_title(
    canvas: &mut Canvas,
    rt: &crate::text::RichText,
    placement: TitlePlacement,
    fp: FontPolicy,
) {
    if placement.rotation_degrees == 0.0 {
        draw_rich_text(canvas, rt, placement.origin, fp);
    } else {
        canvas.save();
        canvas.rotate_at(
            placement.rotation_degrees,
            placement.rotation_center.0,
            placement.rotation_center.1,
        );
        draw_rich_text(canvas, rt, placement.origin, fp);
        canvas.restore();
    }
}

// Chart title (RichText).

fn draw_chart_title(canvas: &mut Canvas, config: &Config, fp: FontPolicy) {
    let ct = &config.chart_title;
    let m = measure_rich_text(&ct.text, fp);
    let placement = chart_title_placement(
        &config.chart_area,
        ct.top_margin,
        (ct.offset_x, ct.offset_y),
        m,
    );
    draw_placed_title(canvas, &ct.text, placement, fp);
}

// Skia Paint helpers.

fn stroke_paint(color: &Color, width: f32, style: &LineStylePreset) -> Paint {
    Paint::stroke(color, width).with_dash(style.pattern())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DrawStyle, SketchOptions};
    use crate::data_config::ContourLabelConfig;
    use crate::default::default_config;
    use crate::layout::LABEL_GAP;

    fn contour_label_fixture() -> (Config, ContourLabelConfig) {
        let config = default_config();
        let labels = ContourLabelConfig {
            visible: true,
            font_size: 12.0,
            color: Color::BLACK,
            format: LabelFormat::Decimal,
            significant_digits: 3,
            spacing_px: 80.0,
            anchors: Vec::new(),
            bg_color: Some(Color::WHITE),
            bg_padding_px: 0.0,
        };
        (config, labels)
    }

    #[test]
    fn a_thousand_and_twenty_four_labels_pack_into_a_2d_atlas() {
        let (config, labels) = contour_label_fixture();
        let texts = vec!["1".to_owned(); 1024];
        let atlas = bake_contour_label_atlas(&config, &labels, &texts, 1.0, 4096)
            .expect("the device limit admits the grid")
            .expect("visible non-empty labels bake an atlas");

        assert_eq!(atlas.cells.len(), 1024);
        assert!(atlas.columns > 1, "1024 labels must not remain one column");
        assert!(atlas.rows > 1, "1024 labels must not remain one row");
        assert!(atlas.columns * atlas.rows >= 1024);
        assert!(atlas.width <= 4096 && atlas.height <= 4096);
        assert_eq!(atlas.width, atlas.columns * atlas.cell_stride_w);
        assert_eq!(atlas.height, atlas.rows * atlas.cell_stride_h);
        assert_eq!(atlas.gutter, 1);
        assert_eq!(
            atlas.rgba.len(),
            atlas.width as usize * atlas.height as usize * 4
        );
    }

    #[test]
    fn every_label_cell_has_a_transparent_texel_gutter() {
        let (config, labels) = contour_label_fixture();
        let texts = vec!["A".to_owned(); 4];
        let atlas = bake_contour_label_atlas(&config, &labels, &texts, 1.0, 128)
            .expect("the device limit admits the grid")
            .expect("visible non-empty labels bake an atlas");
        let alpha = |x: u32, y: u32| {
            let offset = ((y * atlas.width + x) * 4 + 3) as usize;
            atlas.rgba[offset]
        };

        for index in 0..texts.len() as u32 {
            let column = index % atlas.columns;
            let row = index / atlas.columns;
            let left = column * atlas.cell_stride_w;
            let top = row * atlas.cell_stride_h;
            assert_eq!(alpha(left, top), 0, "cell {index} top-left gutter");
            assert_eq!(
                alpha(left + atlas.cell_stride_w - 1, top + atlas.gutter),
                0,
                "cell {index} trailing gutter"
            );
            assert_eq!(
                alpha(left + atlas.gutter, top + atlas.gutter),
                255,
                "cell {index} opaque background begins inside its gutter"
            );
        }
    }

    #[test]
    fn an_atlas_past_the_device_limit_is_an_explicit_error() {
        let error = contour_label_atlas_grid(5, 34, 18, 64).expect_err("capacity is only three");
        assert_eq!(
            error,
            ContourLabelAtlasError::DeviceLimit {
                level_count: 5,
                capacity: 3,
                cell_stride_w: 34,
                cell_stride_h: 18,
                max_dimension: 64,
            }
        );
        assert!(error.to_string().contains("capacity 3"));
    }

    #[test]
    fn disabled_or_empty_labels_remain_a_no_op() {
        let (config, mut labels) = contour_label_fixture();
        labels.visible = false;
        assert!(
            bake_contour_label_atlas(&config, &labels, &["1".into()], 1.0, 1)
                .expect("disabled labels do not consult the limit")
                .is_none()
        );
        labels.visible = true;
        assert!(
            bake_contour_label_atlas(&config, &labels, &[], 1.0, 1)
                .expect("empty labels do not consult the limit")
                .is_none()
        );
    }

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
        assert!(
            minors.iter().all(|v| (v / 200.0).fract().abs() > 1e-9),
            "majors leaked into minors: {minors:?}"
        );

        // Arbitrary (uniform-margin) range ends: tick VALUES stay nice.
        axis.min = 0.137;
        axis.max = 2.63;
        axis.major_spacing = 0.5;
        let majors = major_tick_values(&axis);
        assert_eq!(majors, vec![0.5, 1.0, 1.5, 2.0, 2.5]);
    }

    /// Spacing and minor_count are host SSoT input — 0 (a live input
    /// passing through mid-edit), a mis-typed magnitude, or a non-finite
    /// range must neither blank the axis nor let a walker run away.
    #[test]
    fn tick_walkers_guard_hostile_spacing() {
        let mut axis = default_config().bottom_x.clone();
        axis.min = 0.0;
        axis.max = 10.0;
        axis.minor_count = 4;

        // Zero spacing falls back to the auto spacing, not a blank axis.
        axis.major_spacing = 0.0;
        let majors = major_tick_values(&axis);
        assert_eq!(majors, vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);

        // A tiny positive spacing would walk ~10^10 steps — capped to auto.
        axis.major_spacing = 1e-9;
        let majors = major_tick_values(&axis);
        assert!(
            !majors.is_empty() && majors.len() <= 12,
            "tiny spacing must cap to auto, got {} ticks",
            majors.len()
        );

        // Dense but sane custom spacing is honored verbatim.
        axis.major_spacing = 0.05;
        assert_eq!(major_tick_values(&axis).len(), 201);

        // Hostile minor_count stays bounded (and must not overflow).
        axis.major_spacing = 1.0;
        axis.minor_count = usize::MAX - 1;
        let minors = minor_tick_values(&axis);
        assert!(
            !minors.is_empty() && minors.len() <= 1_100,
            "minor walk must stay bounded, got {}",
            minors.len()
        );

        // Non-finite range ends: nothing to walk — `inf as i64` saturation
        // must never reach the loop bounds.
        axis.minor_count = 4;
        axis.max = f64::INFINITY;
        assert!(major_tick_values(&axis).is_empty());
        assert!(minor_tick_values(&axis).is_empty());
        axis.min = f64::NAN;
        axis.max = 10.0;
        assert!(major_tick_values(&axis).is_empty());
        assert!(minor_tick_values(&axis).is_empty());
    }

    #[test]
    fn linear_tick_walkers_bound_every_finite_extreme_range() {
        let minimum_subnormal = f64::from_bits(1);
        let below_max = f64::from_bits(f64::MAX.to_bits() - 1);
        let mut axis = default_config().bottom_x.clone();
        axis.scale = AxisScale::Linear;
        axis.minor_count = usize::MAX;

        assert!(crate::chart::nice_spacing(minimum_subnormal).is_finite());
        assert!(crate::chart::nice_spacing(minimum_subnormal) > 0.0);
        assert!(crate::chart::nice_spacing(f64::MAX).is_finite());
        assert!(crate::chart::nice_spacing(f64::MAX) > 0.0);

        for (min, max, requested_spacing) in [
            (0.0, minimum_subnormal, 0.0),
            (-f64::MAX, f64::MAX, 0.0),
            (below_max, f64::MAX, minimum_subnormal),
            (-f64::MAX, -below_max, minimum_subnormal),
        ] {
            axis.min = min;
            axis.max = max;
            axis.major_spacing = requested_spacing;

            let spacing = effective_major_spacing(&axis);
            assert!(
                spacing.is_finite() && spacing > 0.0,
                "invalid spacing {spacing} for {min:?}..{max:?}"
            );

            let majors = major_tick_values(&axis);
            assert!(majors.len() <= MAX_MAJOR_INTERVALS + 1, "{majors:?}");
            assert!(majors.iter().all(|value| value.is_finite()), "{majors:?}");
            assert!(
                majors.windows(2).all(|pair| pair[0] < pair[1]),
                "{majors:?}"
            );

            let minors = minor_tick_values(&axis);
            assert!(
                minors.len() <= MAX_MAJOR_INTERVALS * MAX_MINOR_SUBDIVISIONS + 1,
                "{} minor ticks for {min:?}..{max:?}",
                minors.len()
            );
            assert!(minors.iter().all(|value| value.is_finite()));
            assert!(minors.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }

    #[test]
    fn log_tick_walkers_guard_manual_ranges() {
        let close = |a: f64, b: f64| (a - b).abs() <= b.abs() * 1.0e-9 + 1.0e-30;
        let mut axis = default_config().bottom_x.clone();
        axis.scale = AxisScale::Logarithmic;
        axis.major_spacing = 1.0;
        axis.minor_count = 8;
        axis.min = 0.0;
        axis.max = 1000.0;

        let majors = major_tick_values(&axis);
        assert!(majors.iter().all(|v| v.is_finite() && *v > 0.0));
        assert!(close(majors[0], 1.0e-12), "{majors:?}");
        assert!(majors.iter().any(|v| close(*v, 1.0)), "{majors:?}");

        let minors = minor_tick_values(&axis);
        assert!(minors.iter().all(|v| v.is_finite() && *v > 0.0));
        assert!(minors.iter().any(|v| close(*v, 2.0e-12)), "{minors:?}");

        let da = DataArea(crate::layout::Rect {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        });
        let (x, _) = value_to_screen(1.0, &axis, Side::Bottom, &da);
        assert!(x.is_finite() && x > 10.0 && x < 110.0, "x={x}");

        axis.min = 1.0e-15;
        axis.max = 1.0e-12;
        let majors = major_tick_values(&axis);
        assert!(close(majors[0], 1.0e-15), "{majors:?}");
        assert!(close(*majors.last().unwrap(), 1.0e-12), "{majors:?}");

        axis.min = 10.0;
        axis.max = 1.0;
        let majors = major_tick_values(&axis);
        assert_eq!(majors.len(), 2, "{majors:?}");
        assert!(close(majors[0], 10.0), "{majors:?}");
        assert!(close(majors[1], 100.0), "{majors:?}");
    }

    #[test]
    fn log_tick_walkers_bound_every_finite_extreme_range() {
        let mut axis = default_config().bottom_x.clone();
        axis.scale = AxisScale::Logarithmic;
        axis.minor_count = usize::MAX;

        for (min, max, requested_spacing) in [
            (f64::from_bits(1), f64::MAX, 1.0),
            (10.0, 100.0, f64::MAX),
            (f64::MAX, f64::MAX, f64::MAX),
            (f64::MAX, 1.0, f64::MAX),
            (0.0, f64::MAX, f64::NAN),
        ] {
            axis.min = min;
            axis.max = max;
            axis.major_spacing = requested_spacing;

            let (guarded_min, guarded_max) = crate::chart::guarded_log_range(min, max);
            assert!(guarded_min.is_finite() && guarded_min > 0.0);
            assert!(guarded_max.is_finite() && guarded_max > guarded_min);
            assert!(effective_log_decade_step(&axis) > 0);

            let majors = major_tick_values(&axis);
            assert!(majors.len() <= MAX_LOG_DECADES, "{majors:?}");
            assert!(
                majors.iter().all(|value| value.is_finite() && *value > 0.0),
                "{majors:?}"
            );
            assert!(
                majors.windows(2).all(|pair| pair[0] < pair[1]),
                "{majors:?}"
            );

            let minors = minor_tick_values(&axis);
            assert!(minors.len() <= MAX_LOG_DECADES * 8);
            assert!(minors.iter().all(|value| value.is_finite() && *value > 0.0));
            assert!(minors.windows(2).all(|pair| pair[0] < pair[1]));
        }
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
            format_tick_value(
                v,
                &LabelFormat::Decimal,
                3,
                &AxisScale::Linear,
                sp,
                0.0,
                150.0,
            )
        };
        assert_eq!(f(50.0, 50.0), "50");
        assert_eq!(f(100.0, 50.0), "100");
        assert_eq!(f(150.0, 50.0), "150");
        assert_eq!(f(-0.5, 0.5), "-0.5");
        assert_eq!(f(1.5, 0.5), "1.5");
    }

    #[test]
    fn decimal_significant_digits_control_uniform_precision() {
        let f = |digits| {
            format_tick_value(
                1.0,
                &LabelFormat::Decimal,
                digits,
                &AxisScale::Linear,
                1.0,
                0.0,
                10.0,
            )
        };
        assert_eq!(f(1), "1");
        assert_eq!(f(3), "1.0");
        assert_eq!(
            format_tick_value(
                0.5,
                &LabelFormat::Decimal,
                1,
                &AxisScale::Linear,
                0.5,
                0.0,
                1.0,
            ),
            "0.5",
            "spacing remains a floor so adjacent values cannot collapse"
        );
    }

    #[test]
    fn contour_decimal_format_uses_level_spacing_not_bottom_x() {
        let (mut config, mut labels) = contour_label_fixture();
        config.bottom_x.major_spacing = 2.0;
        config.colorbar = Some(crate::default::default_colorbar_options());
        labels.significant_digits = 1;
        let texts = contour_label_texts(&config, &labels, &[-1.0, -0.5, 0.0, 0.5, 1.0]);
        assert_eq!(texts, ["-1.0", "-0.5", "0", "0.5", "1.0"]);
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
        assert!(
            any_opaque,
            "expected at least some non-transparent pixel (axes drawn)"
        );
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

        let plain = try_raster_chart_layer_to_rgba(&config, AxisLayerKind::Decoration).unwrap();
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
        let grid =
            try_raster_chart_layer_to_rgba_with_selection(&config, AxisLayerKind::Grid, &[sel])
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
        cfg.draw_style = DrawStyle::Sketch(SketchOptions {
            seed,
            ..SketchOptions::default()
        });
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
            assert_ne!(
                a, b,
                "{layer:?} raster must change when sketch mode is enabled"
            );
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

    /// The selection overlay never wobbles. The box is placed mid data
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
            rect: RectF {
                x: 300.0,
                y: 350.0,
                width: 120.0,
                height: 80.0,
            },
            color: Color {
                r: 0.0,
                g: 0.4,
                b: 1.0,
                a: 1.0,
            },
            stroke_width: 2.0,
            handles: vec![RectF {
                x: 296.0,
                y: 346.0,
                width: 8.0,
                height: 8.0,
            }],
        };

        let layer = AxisLayerKind::Decoration;
        let base_p = try_raster_chart_layer_to_rgba(&precise, layer).unwrap();
        let with_p = try_raster_chart_layer_to_rgba_with_selection(
            &precise,
            layer,
            std::slice::from_ref(&sel),
        )
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
        assert_eq!(
            fp_p, fp_s,
            "selection footprint must not move in sketch mode"
        );
        for &i in &fp_p {
            assert_eq!(
                with_p[i], with_s[i],
                "selection ink must be byte-identical in sketch mode (byte {i})"
            );
        }
    }

    // ── Colourbar ──────────────────────────────────────────────────────────

    const BAR_W: u32 = 480;
    const BAR_H: u32 = 360;

    /// A chart whose only chrome of interest is the colourbar. The chart area
    /// starts at the origin because the raster entry shifts it there anyway, so
    /// the rect computed here is the rect that was drawn.
    fn colorbar_config(side: Side) -> Config {
        let mut cfg = default_config();
        cfg.chart_area = crate::layout::ChartArea(crate::layout::Rect {
            x: 0,
            y: 0,
            width: BAR_W,
            height: BAR_H,
        });
        cfg.chart_title.visible = false;
        let mut bar = crate::default::default_colorbar_options();
        bar.side = side;
        bar.length_frac = 1.0;
        // No border and no axis line: the strip's interior is then pure ramp,
        // so a sampled pixel is the ramp's answer and nothing else.
        bar.border_width = 0.0;
        bar.axis.min = 0.0;
        bar.axis.max = 100.0;
        bar.axis.major_spacing = 25.0;
        cfg.colorbar = Some(bar);
        cfg
    }

    fn strip_rect(cfg: &Config) -> RectF {
        let da = cfg.data_area().expect("data area");
        colorbar_rect(
            &cfg.chart_area,
            &da,
            cfg.chart_title.top_margin,
            cfg.colorbar.as_ref().expect("colourbar"),
        )
    }

    fn raster(cfg: &Config) -> Vec<u8> {
        try_raster_chart_to_rgba(cfg).expect("colourbar raster")
    }

    fn pixel_at(rgba: &[u8], x: f32, y: f32) -> [u8; 4] {
        let (px, py) = (x.floor() as u32, y.floor() as u32);
        assert!(px < BAR_W && py < BAR_H, "sample ({x}, {y}) is off-canvas");
        let i = ((py * BAR_W + px) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    fn magenta_pixels(rgba: &[u8], rect: RectF) -> usize {
        let x0 = rect.x.floor().max(0.0) as u32;
        let y0 = rect.y.floor().max(0.0) as u32;
        let x1 = (rect.x + rect.width).ceil().min(BAR_W as f32) as u32;
        let y1 = (rect.y + rect.height).ceil().min(BAR_H as f32) as u32;
        let mut count = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                let p = pixel_at(rgba, x as f32, y as f32);
                if p[0] > 180 && p[1] < 100 && p[2] > 180 && p[3] > 180 {
                    count += 1;
                }
            }
        }
        count
    }

    fn assert_close(found: [u8; 4], want: Color, tolerance: i32, what: &str) {
        let expected = [
            (want.r * 255.0).round() as i32,
            (want.g * 255.0).round() as i32,
            (want.b * 255.0).round() as i32,
            (want.a * 255.0).round() as i32,
        ];
        for channel in 0..4 {
            let delta = found[channel] as i32 - expected[channel];
            assert!(
                delta.abs() <= tolerance,
                "{what}: channel {channel} is {} , expected {} (+-{tolerance}); \
                 found {found:?} want {expected:?}",
                found[channel],
                expected[channel]
            );
        }
    }

    /// The strip runs from the ramp's low end at `axis.min` to its high end at
    /// `axis.max`, and the screen direction follows the side: a vertical bar has
    /// min at the bottom, like a y axis.
    ///
    /// Tolerance is 3/255: a band's colour is sampled at its centre, so the
    /// outermost band is half a band short of t = 0 / t = 1.
    #[test]
    fn the_colorbar_strip_runs_from_min_to_max_along_its_side() {
        for side in [Side::Right, Side::Left] {
            let cfg = colorbar_config(side.clone());
            let bar = cfg.colorbar.clone().expect("colourbar");
            let rect = strip_rect(&cfg);
            let rgba = raster(&cfg);
            let mid_x = rect.x + rect.width * 0.5;

            assert_close(
                pixel_at(&rgba, mid_x, rect.y + 0.5),
                bar.colormap.sample(1.0),
                3,
                &format!("{side:?} top of a vertical bar is axis.max"),
            );
            assert_close(
                pixel_at(&rgba, mid_x, rect.y + rect.height - 0.5),
                bar.colormap.sample(0.0),
                3,
                &format!("{side:?} bottom of a vertical bar is axis.min"),
            );
        }

        for side in [Side::Top, Side::Bottom] {
            let cfg = colorbar_config(side.clone());
            let bar = cfg.colorbar.clone().expect("colourbar");
            let rect = strip_rect(&cfg);
            let rgba = raster(&cfg);
            let mid_y = rect.y + rect.height * 0.5;

            assert_close(
                pixel_at(&rgba, rect.x + 0.5, mid_y),
                bar.colormap.sample(0.0),
                3,
                &format!("{side:?} left of a horizontal bar is axis.min"),
            );
            assert_close(
                pixel_at(&rgba, rect.x + rect.width - 0.5, mid_y),
                bar.colormap.sample(1.0),
                3,
                &format!("{side:?} right of a horizontal bar is axis.max"),
            );
        }
    }

    /// The §B.5 pin: the colour drawn where a value's tick lands is the colour
    /// that value's z normalization asks for. The strip and the tick positions
    /// come from two functions with deliberately different out-of-range
    /// behaviour (`normalized_z` clamps and rejects, `axis_fraction` does
    /// neither), and in range they must agree exactly — that agreement is what
    /// the GPU field sampler will also be held to.
    #[test]
    fn colorbar_ticks_match_the_strip_ramp() {
        for (scale, min, max, probes) in [
            (
                AxisScale::Linear,
                0.0f64,
                100.0f64,
                vec![0.0, 12.5, 25.0, 50.0, 99.0, 100.0],
            ),
            (
                AxisScale::Logarithmic,
                1.0e-3,
                1.0e3,
                vec![1.0e-3, 1.0e-2, 1.0, 1.0e2, 1.0e3],
            ),
        ] {
            for inverted in [false, true] {
                let mut cfg = colorbar_config(Side::Right);
                {
                    let bar = cfg.colorbar.as_mut().expect("colourbar");
                    bar.axis.scale = scale.clone();
                    bar.axis.min = min;
                    bar.axis.max = max;
                    bar.axis.inverted = inverted;
                }
                let bar = cfg.colorbar.as_ref().expect("colourbar");
                for z in &probes {
                    let normalized = bar
                        .normalized_z(*z)
                        .unwrap_or_else(|| panic!("{z} is inside [{min}, {max}]"));
                    let mut on_screen = axis_fraction(*z, &bar.axis);
                    if inverted {
                        on_screen = 1.0 - on_screen;
                    }
                    assert!(
                        (normalized - on_screen).abs() < 1e-6,
                        "{scale:?} inverted={inverted} z={z}: colour t {normalized} vs \
                         position t {on_screen}"
                    );
                }
                // Out of range, the two part ways on purpose.
                assert_eq!(bar.normalized_z(f64::NAN), None);
                assert_eq!(bar.normalized_z(max * 10.0), Some(1.0));
            }
        }
    }

    /// `inverted` moves where a value is drawn. It does not reverse the ramp:
    /// the same z keeps the same colour, at the other end of the bar.
    #[test]
    fn an_inverted_colorbar_axis_swaps_the_ends_and_not_the_ramp() {
        let mut cfg = colorbar_config(Side::Right);
        cfg.colorbar.as_mut().expect("colourbar").axis.inverted = true;
        let bar = cfg.colorbar.clone().expect("colourbar");
        let rect = strip_rect(&cfg);
        let rgba = raster(&cfg);
        let mid_x = rect.x + rect.width * 0.5;

        // Inverted: axis.max is now at the bottom, so t = 1 is the bottom.
        assert_close(
            pixel_at(&rgba, mid_x, rect.y + 0.5),
            bar.colormap.sample(0.0),
            3,
            "inverted top is axis.min's colour",
        );
        assert_close(
            pixel_at(&rgba, mid_x, rect.y + rect.height - 0.5),
            bar.colormap.sample(1.0),
            3,
            "inverted bottom is axis.max's colour",
        );
        // The colour a value has did not change — only where it sits.
        assert_eq!(bar.normalized_z(bar.axis.max), Some(1.0));
    }

    /// `visible: false` is byte-identical to having no colourbar at all: the
    /// band was never reserved, so there is nothing drawn and nothing blank.
    #[test]
    fn a_hidden_colorbar_draws_exactly_nothing() {
        let mut hidden = colorbar_config(Side::Right);
        hidden.colorbar.as_mut().expect("colourbar").visible = false;
        let mut absent = hidden.clone();
        absent.colorbar = None;

        assert_eq!(raster(&hidden), raster(&absent));

        // And a visible one does change the picture — otherwise the comparison
        // above would pass for the wrong reason.
        let shown = colorbar_config(Side::Right);
        assert_ne!(raster(&shown), raster(&absent));
    }

    /// The colourbar's ticks and labels are the axis helpers, so a logarithmic
    /// bar gets decade majors and 10ⁿ labels with no second implementation.
    #[test]
    fn a_logarithmic_colorbar_ticks_by_decade() {
        let mut cfg = colorbar_config(Side::Right);
        {
            let bar = cfg.colorbar.as_mut().expect("colourbar");
            bar.axis.scale = AxisScale::Logarithmic;
            bar.axis.min = 1.0e-2;
            bar.axis.max = 1.0e2;
            bar.axis.major_spacing = 1.0;
            bar.axis.label_style.format = LabelFormat::Power;
        }
        let bar = cfg.colorbar.as_ref().expect("colourbar");
        let majors = major_tick_values(&bar.axis);
        assert_eq!(majors, vec![1.0e-2, 1.0e-1, 1.0, 1.0e1, 1.0e2]);

        // The Power label is a RichText with a superscript exponent, which is
        // what `format_tick_power` exists to produce.
        let label = format_tick_power(1.0e-2, 3, &bar.axis.label_style);
        assert!(
            label.segments.iter().any(|seg| seg.superscript),
            "a decade label must carry a superscript exponent: {label:?}"
        );

        // Ink lands in the label margin outside the strip.
        let rect = strip_rect(&cfg);
        let rgba = raster(&cfg);
        let label_band_x = rect.x + rect.width + bar.axis.major_tick_length + 2.0;
        let inked = (0..BAR_H).any(|y| {
            (0..(BAR_W - label_band_x as u32))
                .any(|dx| pixel_at(&rgba, label_band_x + dx as f32, y as f32)[3] != 0)
        });
        assert!(inked, "no tick label ink outside the strip");
    }

    #[test]
    fn colorbar_ticks_honor_direction_and_configured_line_style() {
        let configured = |tick, style| {
            let mut cfg = colorbar_config(Side::Right);
            let bar = cfg.colorbar.as_mut().expect("colourbar");
            bar.axis.min = 0.0;
            bar.axis.max = 1.0;
            bar.axis.major_spacing = 0.5;
            bar.axis.minor_count = 0;
            bar.axis.tick = tick;
            bar.axis.major_tick_length = 12.0;
            bar.axis.line_width = 2.0;
            bar.axis.line_color = Color::from_rgb8(255, 0, 255);
            bar.axis.line_style = style;
            bar.axis.line_visible = false;
            bar.axis.label_style.visible = false;
            cfg
        };

        let outside = configured(TickVisibility::Outside, LineStylePreset::Solid);
        let outside_rect = strip_rect(&outside);
        let outside_rgba = raster(&outside);
        let middle_y = outside_rect.y + outside_rect.height * 0.5;
        let outside_probe = RectF {
            x: outside_rect.x + outside_rect.width + 1.0,
            y: middle_y - 2.0,
            width: 10.0,
            height: 4.0,
        };
        let inside_probe = RectF {
            x: outside_rect.x + outside_rect.width - 11.0,
            y: middle_y - 2.0,
            width: 10.0,
            height: 4.0,
        };
        assert!(magenta_pixels(&outside_rgba, outside_probe) > 0);
        assert_eq!(magenta_pixels(&outside_rgba, inside_probe), 0);

        let inside = configured(TickVisibility::Inside, LineStylePreset::Solid);
        let inside_rgba = raster(&inside);
        assert!(magenta_pixels(&inside_rgba, inside_probe) > 0);
        assert_eq!(magenta_pixels(&inside_rgba, outside_probe), 0);

        let dotted = configured(TickVisibility::Outside, LineStylePreset::ShortDot);
        let dotted_count = magenta_pixels(&raster(&dotted), outside_probe);
        let solid_count = magenta_pixels(&outside_rgba, outside_probe);
        assert!(
            dotted_count < solid_count,
            "tick dash style was ignored: dotted={dotted_count}, solid={solid_count}"
        );
    }

    #[test]
    fn detached_colorbar_axis_paint_stays_inside_its_selection_bounds() {
        use crate::select::{ColorBarAxisElement, Selectable};
        use crate::text_render::CpuTextMeasure;

        let mut cfg = colorbar_config(Side::Right);
        {
            let bar = cfg.colorbar.as_mut().expect("colourbar");
            bar.axis.min = 0.0;
            bar.axis.max = 1.0;
            bar.axis.major_spacing = 0.5;
            bar.axis.minor_count = 0;
            bar.axis.tick = TickVisibility::Outside;
            bar.axis.major_tick_length = 12.0;
            bar.axis.line_width = 2.0;
            bar.axis.line_color = Color::from_rgb8(255, 0, 255);
            bar.axis.line_visible = false;
            bar.axis.label_style.visible = false;
            bar.axis.line_offset = -9.0;
        }
        let bounds = ColorBarAxisElement
            .bounds(&cfg, &CpuTextMeasure::for_style(&cfg.draw_style))
            .expect("axis selection bounds");
        assert!(
            magenta_pixels(&raster(&cfg), bounds) > 0,
            "detached tick paint and selection bounds diverged"
        );
    }

    /// A colourbar and a visible axis on the same side must not be drawn on top
    /// of each other: the axis band hugs the data area and the bar is outside
    /// it, so the axis' outermost ink stays clear of the strip.
    #[test]
    fn a_colorbar_does_not_cover_the_axis_labels_on_its_side() {
        let mut cfg = colorbar_config(Side::Right);
        cfg.right_y.label_style.visible = true;
        cfg.right_y.label_style.label_visible = true;
        cfg.right_y.out_margin = 90.0;
        let da = cfg.data_area().expect("data area");
        let rect = strip_rect(&cfg);

        let axis_reach = (da.x + da.width) as f32 + cfg.right_y.major_tick_length + LABEL_GAP;
        assert!(
            rect.x > axis_reach,
            "strip starts at {} but the axis' ticks reach {axis_reach}",
            rect.x
        );
        // The whole axis label band fits before the strip.
        assert!(
            rect.x >= (da.x + da.width) as f32 + cfg.right_y.out_margin,
            "strip at {} overlaps the {} px axis label margin",
            rect.x,
            cfg.right_y.out_margin
        );
    }

    /// A degenerate strip is skipped rather than drawn as a zero-area rect or a
    /// division by zero in the band walk.
    #[test]
    fn a_degenerate_colorbar_is_skipped() {
        for (frac, thickness) in [(0.0, 18.0), (0.5, 0.0)] {
            let mut cfg = colorbar_config(Side::Right);
            {
                let bar = cfg.colorbar.as_mut().expect("colourbar");
                bar.length_frac = frac;
                bar.thickness_px = thickness;
            }
            // Draws without panicking, and paints no strip.
            let rgba = raster(&cfg);
            assert_eq!(rgba.len(), (BAR_W * BAR_H * 4) as usize);
        }
    }

    /// The offset moves what is painted, not just what is selectable: the strip
    /// is drawn at `ColorBarElement`'s bounds, so a drag lands the highlight box
    /// and the handles on the bar the user sees.
    #[test]
    fn a_colorbar_offset_moves_the_painted_strip() {
        use crate::select::{ColorBarElement, Selectable};
        use crate::text_render::CpuTextMeasure;

        let mut cfg = colorbar_config(Side::Right);
        let base = strip_rect(&cfg);
        {
            let bar = cfg.colorbar.as_mut().expect("colourbar");
            bar.offset_x = -40.0;
            bar.offset_y = 12.0;
        }
        let moved = strip_rect(&cfg);
        assert_eq!(moved.x, base.x - 40.0);
        assert_eq!(moved.y, base.y + 12.0);

        // Painted where the offset says, and nowhere else.
        let bar = cfg.colorbar.clone().expect("colourbar");
        let rgba = raster(&cfg);
        assert_close(
            pixel_at(&rgba, moved.x + moved.width * 0.5, moved.y + 0.5),
            bar.colormap.sample(1.0),
            3,
            "the strip's high end moved with the offset",
        );
        assert_eq!(
            pixel_at(&rgba, base.x + base.width * 0.5, base.y + base.height * 0.5)[3],
            0,
            "nothing is painted at the un-offset position"
        );

        // Interaction bounds are the same rect the paint used — no second
        // geometry to drift.
        assert_eq!(
            ColorBarElement.bounds(&cfg, &CpuTextMeasure::for_style(&cfg.draw_style)),
            Some(moved)
        );
    }
}
