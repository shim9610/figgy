//! Axis renderer. Takes a `Config` and draws axes, ticks, labels, and titles
//! onto a skia Canvas. No data is drawn here — only axis chrome.
//!
//! Text rendering is delegated to [`crate::text_render`].

use skia_safe::{
    paint::Style as PaintStyle, surfaces, AlphaType, Canvas, Color as SkColor, Color4f, ColorType,
    ImageInfo, Paint, PathEffect,
};

use crate::color::Color;
use crate::config::{
    AxisOptions, AxisScale, Config, LabelStyle, TickVisibility,
};
use crate::format::LabelFormat;
use crate::layout::{DataArea, Side};
use crate::line::LineStylePreset;
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
/// `[R, G, B, A]` with premultiplied alpha (skia default `AlphaType::Premul`).
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
    use crate::layout::{ChartArea, Rect};

    let w = config.chart_area.0.width;
    let h = config.chart_area.0.height;
    if w == 0 || h == 0 {
        return Err(crate::FiggyError::InvalidChartArea { width: w, height: h });
    }
    let wi = w as i32;
    let hi = h as i32;

    let pixel_count = (w as usize) * (h as usize);
    let byte_len = pixel_count * 4;
    let mut buffer: Vec<u8> = vec![0u8; byte_len];

    let mut raster_cfg = config.clone();
    raster_cfg.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: w,
        height: h,
    });

    {
        let info = ImageInfo::new((wi, hi), ColorType::RGBA8888, AlphaType::Premul, None);
        let Some(mut sk_surface) = surfaces::wrap_pixels(&info, &mut buffer, None, None) else {
            return Err(crate::FiggyError::RasterWrapFailed {
                reason: format!(
                    "wrap_pixels returned None for {w}x{h} (buf={byte_len} bytes)"
                ),
            });
        };
        let canvas = sk_surface.canvas();
        canvas.clear(SkColor::TRANSPARENT);
        match layer {
            AxisLayerKind::Grid => draw_grid_layer(canvas, &raster_cfg),
            AxisLayerKind::Decoration => draw_decoration_layer(canvas, &raster_cfg),
            AxisLayerKind::All => draw_axes(canvas, &raster_cfg),
        }
    }

    Ok(buffer)
}

/// Panic-on-error wrapper around [`try_raster_chart_to_rgba`]. New code
/// should prefer the fallible version; this exists for the binary demos.
pub fn raster_chart_to_rgba(config: &Config) -> Vec<u8> {
    try_raster_chart_to_rgba(config).expect("raster_chart_to_rgba failed")
}

/// Grid layer — only the parts that should sit below the data layer.
pub fn draw_grid_layer(canvas: &Canvas, config: &Config) {
    let Ok(da) = config.data_area() else { return };
    draw_grid(canvas, config, &da);
}

/// Decoration layer — axis lines, tick labels, axis titles, chart title,
/// legend. Drawn above the data so data never overlaps axis chrome.
pub fn draw_decoration_layer(canvas: &Canvas, config: &Config) {
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

    if config.legend.visible && !config.legend.entries.is_empty() {
        draw_legend(canvas, config, &da);
    }
}

/// Draw the full axis chrome (grid + decoration) into the regions specified
/// by `config`. If `data_area()` fails, this is a no-op.
pub fn draw_axes(canvas: &Canvas, config: &Config) {
    draw_grid_layer(canvas, config);
    draw_decoration_layer(canvas, config);
}

/// Draw the legend box in one corner of the data area.
fn draw_legend(canvas: &Canvas, config: &Config, da: &crate::layout::DataArea) {
    use crate::config::{LegendCorner, LegendEntryKind};

    let lg = &config.legend;
    let n = lg.entries.len();
    if n == 0 { return; }

    // Measure label widths to size the box.
    let mut label_w_max: f32 = 0.0;
    for e in &lg.entries {
        let label = legend_label_text(e.label.clone(), lg.font_size);
        let m = measure_rich_text(&label);
        if m.width > label_w_max { label_w_max = m.width; }
    }
    let inner_w = lg.sample_width + lg.sample_text_gap + label_w_max;
    let box_w = inner_w + lg.padding * 2.0;
    let box_h = (n as f32) * lg.line_height + lg.padding * 2.0;

    // Corner position — inset from the data_area inner corner.
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

    // Box background + border.
    let bg_rect = skia_safe::Rect::from_xywh(box_x, box_y, box_w, box_h);
    let mut bg = Paint::new(
        Color4f::new(
            lg.bg_color.r * lg.bg_color.a,
            lg.bg_color.g * lg.bg_color.a,
            lg.bg_color.b * lg.bg_color.a,
            lg.bg_color.a,
        ),
        None,
    );
    bg.set_anti_alias(true);
    bg.set_style(PaintStyle::Fill);
    canvas.draw_rect(bg_rect, &bg);

    let mut border = Paint::new(
        Color4f::new(
            lg.border_color.r, lg.border_color.g, lg.border_color.b, lg.border_color.a,
        ),
        None,
    );
    border.set_anti_alias(true);
    border.set_style(PaintStyle::Stroke);
    border.set_stroke_width(1.0);
    canvas.draw_rect(bg_rect, &border);

    // One row per entry: sample + label.
    for (i, e) in lg.entries.iter().enumerate() {
        let row_y_center =
            box_y + lg.padding + (i as f32 + 0.5) * lg.line_height;

        let sample_x0 = box_x + lg.padding;
        let sample_x1 = sample_x0 + lg.sample_width;
        let sample_x_mid = (sample_x0 + sample_x1) * 0.5;

        let series_paint = {
            let mut p = Paint::new(
                Color4f::new(e.color.r, e.color.g, e.color.b, e.color.a),
                None,
            );
            p.set_anti_alias(true);
            p
        };

        match e.kind {
            LegendEntryKind::Line | LegendEntryKind::LineScatter => {
                let mut sp = series_paint.clone();
                sp.set_style(PaintStyle::Stroke);
                sp.set_stroke_width(e.line_width.max(1.0));
                canvas.draw_line(
                    (sample_x0, row_y_center),
                    (sample_x1, row_y_center),
                    &sp,
                );
            }
            _ => {}
        }
        match e.kind {
            LegendEntryKind::Scatter | LegendEntryKind::LineScatter => {
                let mut sp = series_paint.clone();
                sp.set_style(PaintStyle::Fill);
                canvas.draw_circle((sample_x_mid, row_y_center), 4.0, &sp);
            }
            _ => {}
        }

        // Label — baseline-aligned.
        let label = legend_label_text(e.label.clone(), lg.font_size);
        let m = measure_rich_text(&label);
        let label_x = sample_x1 + lg.sample_text_gap;
        let label_y = row_y_center + (m.ascent - m.descent) * 0.5;
        draw_rich_text(canvas, &label, (label_x, label_y));
    }
}

fn legend_label_text(mut label: RichText, font_size: f32) -> RichText {
    label.font_size = font_size;
    label
}

// Grid rendering.
//
// Vertical grid lines use bottom_x tick positions; horizontal lines use
// left_y. Lines are confined to the data_area (just inside the axis lines).

fn draw_grid(canvas: &Canvas, config: &Config, da: &DataArea) {
    let g = &config.grid;
    let x_top = da.y as f32;
    let x_bot = (da.y + da.height) as f32;
    let y_left = da.x as f32;
    let y_right = (da.x + da.width) as f32;

    // Draw minor first so major can paint over it.
    if g.show_minor_x {
        let paint = stroke_paint(&g.minor_x_color, g.minor_x_width, &g.minor_x_style);
        let majors = major_tick_values(&config.bottom_x);
        for v in minor_tick_values(&config.bottom_x, &majors) {
            let pos = value_to_screen(v, &config.bottom_x, Side::Bottom, da);
            canvas.draw_line((pos.0, x_top), (pos.0, x_bot), &paint);
        }
    }
    if g.show_minor_y {
        let paint = stroke_paint(&g.minor_y_color, g.minor_y_width, &g.minor_y_style);
        let majors = major_tick_values(&config.left_y);
        for v in minor_tick_values(&config.left_y, &majors) {
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

fn draw_axis(canvas: &Canvas, axis: &AxisOptions, side: Side, da: &DataArea) {
    let (p0, p1) = axis_endpoints(side.clone(), da);

    // 1) Axis line.
    if axis.line_visible {
        let paint = stroke_paint(&axis.line_color, axis.line_width, &axis.line_style);
        canvas.draw_line(p0, p1, &paint);
    }

    // 2) Major / minor ticks
    let majors = major_tick_values(axis);
    let minors = minor_tick_values(axis, &majors);

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
                    let text =
                        format_tick_value(*v, &ls.format, ls.significant_digits, &axis.scale);
                    draw_tick_label(canvas, &text, pos, side.clone(), axis);
                }
            }
        }
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
            let mut v = axis.min;
            // Step by major_spacing using an index to avoid float drift.
            let n = ((axis.max - axis.min) / axis.major_spacing).round() as i64;
            for i in 0..=n {
                v = axis.min + (i as f64) * axis.major_spacing;
                if v > axis.max + axis.major_spacing * 1e-9 {
                    break;
                }
                out.push(v);
            }
            let _ = v;
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

fn minor_tick_values(axis: &AxisOptions, majors: &[f64]) -> Vec<f64> {
    let mut out = Vec::new();
    if axis.minor_count == 0 || majors.len() < 2 {
        return out;
    }
    match axis.scale {
        AxisScale::Linear => {
            let subdivisions = axis.minor_count + 1;
            for win in majors.windows(2) {
                let a = win[0];
                let b = win[1];
                let step = (b - a) / subdivisions as f64;
                for k in 1..subdivisions {
                    out.push(a + (k as f64) * step);
                }
            }
        }
        AxisScale::Logarithmic => {
            // 2..=9 within each decade.
            for win in majors.windows(2) {
                let a = win[0];
                let _b = win[1];
                // If a == 10^k, minors are at 2a, 3a, …, 9a.
                for k in 2..=9 {
                    let v = a * (k as f64);
                    if v < axis.max {
                        out.push(v);
                    }
                }
            }
        }
    }
    out
}

fn draw_tick(
    canvas: &Canvas,
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

fn format_tick_value(
    value: f64,
    format: &LabelFormat,
    sig_digits: u8,
    scale: &AxisScale,
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

    // Linear scale: pad to `sig_digits`.
    let sig = sig_digits.max(1) as usize;
    match format {
        LabelFormat::Scientific => format!("{:.*e}", sig.saturating_sub(1), value),
        // Power uses the RichText path; if we end up here, treat as Decimal.
        LabelFormat::Decimal | LabelFormat::Power => {
            if value == 0.0 {
                return "0".into();
            }
            let order = value.abs().log10().floor() as i32;
            let decimals = ((sig as i32) - 1 - order).max(0) as usize;
            format!("{:.*}", decimals, value)
        }
    }
}

// Power format (RichText with superscript exponent).

fn plain_seg(c: char) -> RichSegment {
    RichSegment {
        text: c,
        bold: false,
        italic: false,
        underline: false,
        superscript: false,
        subscript: false,
        greek: false,
    }
}

fn sup_seg(c: char) -> RichSegment {
    RichSegment {
        text: c,
        bold: false,
        italic: false,
        underline: false,
        superscript: true,
        subscript: false,
        greek: false,
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
    canvas: &Canvas,
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
    canvas: &Canvas,
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

fn draw_axis_title(canvas: &Canvas, config: &Config, da: &DataArea, side: Side) {
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
    canvas: &Canvas,
    rt: &crate::text::RichText,
    center: (f32, f32),
    degrees: f32,
    to: &crate::config::AxisTitleOptions,
    m: &crate::text_render::TextMetrics,
) {
    let (cx, cy) = center;
    canvas.save();
    canvas.rotate(degrees, Some(skia_safe::Point::new(cx, cy)));

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

fn draw_chart_title(canvas: &Canvas, config: &Config) {
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
    let c4 = Color4f::new(color.r, color.g, color.b, color.a);
    let mut paint = Paint::new(c4, None);
    paint.set_anti_alias(true);
    paint.set_style(PaintStyle::Stroke);
    paint.set_stroke_width(width.max(1.0));

    let pattern: &[f32] = match style {
        LineStylePreset::Solid => &[],
        LineStylePreset::Dash => &[8.0, 4.0],
        LineStylePreset::Dot => &[2.0, 3.0],
        LineStylePreset::DashDot => &[8.0, 4.0, 2.0, 4.0],
        LineStylePreset::DashDotDot => &[8.0, 4.0, 2.0, 4.0, 2.0, 4.0],
        LineStylePreset::ShortDash => &[4.0, 3.0],
        LineStylePreset::ShortDot => &[1.0, 2.0],
        LineStylePreset::ShortDashDot => &[4.0, 3.0, 1.0, 3.0],
        LineStylePreset::LongDash => &[14.0, 4.0],
        LineStylePreset::LongDashDot => &[14.0, 4.0, 2.0, 4.0],
        LineStylePreset::LongDashDotDot => &[14.0, 4.0, 2.0, 4.0, 2.0, 4.0],
    };
    if !pattern.is_empty()
        && let Some(effect) = PathEffect::dash(pattern, 0.0)
    {
        paint.set_path_effect(effect);
    }
    paint
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;

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
}
