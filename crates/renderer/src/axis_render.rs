//! Axis renderer. Takes a `Config` and draws axes, ticks, labels, and titles
//! onto the CPU raster [`Canvas`]. No data is drawn here — only axis chrome.
//!
//! Text rendering is delegated to [`crate::text_render`].

use crate::raster::{Canvas, Paint};

use crate::color::Color;
use crate::config::{
    AxisOptions, AxisScale, Config, LabelStyle, TickVisibility,
};
use crate::format::LabelFormat;
use crate::layout::{DataArea, Side};
use crate::line::LineStylePreset;
use crate::select::SelectionBox;
use crate::text::{RichSegment, RichText};
use crate::text_render::{
    draw_plain_text, draw_rich_text, measure_plain_text, measure_rich_text,
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
pub fn draw_grid_layer(canvas: &mut Canvas, config: &Config) {
    let Ok(da) = config.data_area() else { return };
    draw_grid(canvas, config, &da);
}

/// Decoration layer — axis lines, tick labels, axis titles, chart title,
/// legend. Drawn above the data so data never overlaps axis chrome.
pub fn draw_decoration_layer(canvas: &mut Canvas, config: &Config) {
    let Ok(da) = config.data_area() else { return };

    draw_axis(canvas, &config.top_x, Side::Top, &da);
    draw_axis(canvas, &config.bottom_x, Side::Bottom, &da);
    draw_axis(canvas, &config.left_y, Side::Left, &da);
    draw_axis(canvas, &config.right_y, Side::Right, &da);

    draw_axis_title(canvas, config, &da, Side::Top);
    draw_axis_title(canvas, config, &da, Side::Bottom);
    draw_axis_title(canvas, config, &da, Side::Left);
    draw_axis_title(canvas, config, &da, Side::Right);

    if config.chart_title.visible {
        draw_chart_title(canvas, config);
    }

    if config.legend.visible && !config.legend.content.segments.is_empty() {
        draw_legend(canvas, config, &da);
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
fn draw_legend(canvas: &mut Canvas, config: &Config, da: &crate::layout::DataArea) {
    use crate::config::LegendCorner;

    let lg = &config.legend;
    if !lg.visible || lg.content.segments.is_empty() { return; }

    let m = measure_rich_text(&lg.content);
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

    // Box background + border.
    canvas.draw_rect(box_x, box_y, box_w, box_h, &Paint::fill(&lg.bg_color));
    canvas.draw_rect(box_x, box_y, box_w, box_h, &Paint::stroke(&lg.border_color, 1.0));

    // Content: first baseline sits `ascent` below the padded top-left corner.
    draw_rich_text(
        canvas,
        &lg.content,
        (box_x + lg.padding, box_y + lg.padding + m.ascent),
    );
}

// Grid rendering.
//
// Vertical grid lines use bottom_x tick positions; horizontal lines use
// left_y. Lines are confined to the data_area (just inside the axis lines).

fn draw_grid(canvas: &mut Canvas, config: &Config, da: &DataArea) {
    let g = &config.grid;
    let x_top = da.y as f32;
    let x_bot = (da.y + da.height) as f32;
    let y_left = da.x as f32;
    let y_right = (da.x + da.width) as f32;

    // Draw minor first so major can paint over it.
    if g.show_minor_x {
        let paint = stroke_paint(&g.minor_x_color, g.minor_x_width, &g.minor_x_style);
        for v in minor_tick_values(&config.bottom_x) {
            let pos = value_to_screen(v, &config.bottom_x, Side::Bottom, da);
            canvas.draw_line((pos.0, x_top), (pos.0, x_bot), &paint);
        }
    }
    if g.show_minor_y {
        let paint = stroke_paint(&g.minor_y_color, g.minor_y_width, &g.minor_y_style);
        for v in minor_tick_values(&config.left_y) {
            let pos = value_to_screen(v, &config.left_y, Side::Left, da);
            canvas.draw_line((y_left, pos.1), (y_right, pos.1), &paint);
        }
    }

    if g.show_major_x {
        let paint = stroke_paint(&g.major_x_color, g.major_x_width, &g.major_x_style);
        for v in major_tick_values(&config.bottom_x) {
            let pos = value_to_screen(v, &config.bottom_x, Side::Bottom, da);
            canvas.draw_line((pos.0, x_top), (pos.0, x_bot), &paint);
        }
    }
    if g.show_major_y {
        let paint = stroke_paint(&g.major_y_color, g.major_y_width, &g.major_y_style);
        for v in major_tick_values(&config.left_y) {
            let pos = value_to_screen(v, &config.left_y, Side::Left, da);
            canvas.draw_line((y_left, pos.1), (y_right, pos.1), &paint);
        }
    }
}

fn draw_axis(canvas: &mut Canvas, axis: &AxisOptions, side: Side, da: &DataArea) {
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
        canvas.draw_line(p0, p1, &paint);
    }

    // 2) Major / minor ticks
    let majors = major_tick_values(axis);
    let minors = minor_tick_values(axis);

    if axis.tick != TickVisibility::None {
        let tick_paint = stroke_paint(&axis.line_color, axis.line_width, &LineStylePreset::Solid);
        for v in &majors {
            let pos = value_to_screen(*v, axis, side.clone(), da);
            draw_tick(canvas, pos, side.clone(), axis.major_tick_length, &axis.tick, &tick_paint);
        }
        for v in &minors {
            let pos = value_to_screen(*v, axis, side.clone(), da);
            draw_tick(canvas, pos, side.clone(), axis.minor_tick_length, &axis.tick, &tick_paint);
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
                    draw_tick_label_rich(canvas, &rt, pos, side.clone(), axis);
                }
                _ => {
                    let text = format_tick_value(
                        *v,
                        &ls.format,
                        ls.significant_digits,
                        &axis.scale,
                        axis.major_spacing,
                    );
                    draw_tick_label(canvas, &text, pos, side.clone(), axis);
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

fn draw_tick(
    canvas: &mut Canvas,
    pos: (f32, f32),
    side: Side,
    length: f32,
    visibility: &TickVisibility,
    paint: &Paint,
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
            canvas.draw_line(pos, outside, paint);
        }
        TickVisibility::Inside => {
            canvas.draw_line(pos, inside, paint);
        }
        TickVisibility::Both => {
            canvas.draw_line(inside, outside, paint);
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
) {
    let ls: &LabelStyle = &axis.label_style;
    let m = measure_plain_text(text, &ls.label_font, ls.font_size, false, false);

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
) {
    let m = measure_rich_text(rt);
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

    draw_rich_text(canvas, rt, origin);
}

// Axis title (RichText).

fn draw_axis_title(canvas: &mut Canvas, config: &Config, da: &DataArea, side: Side) {
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

    let m = measure_rich_text(&to.text);
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
            );
        }
        // Vertical text: Left (-90° CCW), Right (+90° CW).
        // Rotate around the band center and center the text on that point.
        Side::Left => {
            let center_x = ca.x as f32 + axis.out_margin * 0.5;
            let center_y = da.y as f32 + da.height as f32 * 0.5;
            draw_rotated_centered(canvas, &to.text, (center_x, center_y), -90.0, to, &m);
        }
        Side::Right => {
            let center_x = (ca.x + ca.width) as f32 - axis.out_margin * 0.5;
            let center_y = da.y as f32 + da.height as f32 * 0.5;
            draw_rotated_centered(canvas, &to.text, (center_x, center_y), 90.0, to, &m);
        }
    }
}

// Rotate text by `degrees` around `(cx, cy)` and center it on that point.
// `to.offset_{x,y}` is applied in the pre-rotation local frame (so it stays
// aligned with the text direction after the rotation).
fn draw_rotated_centered(
    canvas: &mut Canvas,
    rt: &crate::text::RichText,
    center: (f32, f32),
    degrees: f32,
    to: &crate::config::AxisTitleOptions,
    m: &crate::text_render::TextMetrics,
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
    );
    canvas.restore();
}

// Chart title (RichText).

fn draw_chart_title(canvas: &mut Canvas, config: &Config) {
    let ct = &config.chart_title;
    let m = measure_rich_text(&ct.text);
    let ca = &config.chart_area;

    // Vertically centered inside the top_margin band.
    let baseline = ca.y as f32 + (ct.top_margin - m.height()) * 0.5 + m.ascent;
    let x = ca.x as f32 + ca.width as f32 * 0.5 - m.width * 0.5;

    let origin_x = x + ct.offset_x;
    let origin_y = baseline + ct.offset_y;

    draw_rich_text(canvas, &ct.text, (origin_x, origin_y));
}

// Skia Paint helpers.

fn stroke_paint(color: &Color, width: f32, style: &LineStylePreset) -> Paint {
    Paint::stroke(color, width).with_dash(style.pattern())
}

#[cfg(test)]
mod tests {
    use super::*;
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
            .selection_box(&config, &CpuTextMeasure)
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
            .selection_box(&config, &CpuTextMeasure)
            .unwrap();
        let grid = try_raster_chart_layer_to_rgba_with_selection(
            &config,
            AxisLayerKind::Grid,
            &[sel],
        )
        .unwrap();
        assert!(!has_selection_blue(&grid));
    }
}
