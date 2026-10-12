use super::*;
use crate::decoration::{DecorationMap, DecorationPainter};
use crate::raster::{Canvas, Paint};
use crate::text_render::{FontPolicy, measure_plain_text};
use model::DecorationId;
use std::f32::consts::TAU;

pub(super) struct Layout {
    pub sectors: Vec<Sector>,
    pub groups: [[f32; 4]; 2],
    pub holes: [f32; 2],
    pub annotations: Vec<u8>,
    pub decorations: DecorationMap,
    pub targets: Vec<RadialTarget>,
}
fn width(c: &RadialChart, text: &str, size: f32) -> f32 {
    measure_plain_text(
        text,
        &c.font_family,
        size,
        false,
        false,
        FontPolicy::Standard,
    )
    .width
}
fn projected(s: &Sector, angle: f32, r: f32) -> (f32, f32) {
    let mid = s.arc[0] + s.arc[1] * 0.5;
    let cs = s.arc[2].cos();
    let sn = s.arc[2].sin();
    (
        s.geometry[0] + s.geometry[2] * (r * angle.cos() + s.detail[0] * mid.cos()),
        s.geometry[1]
            + s.geometry[2] * ((r * angle.sin() + s.detail[0] * mid.sin()) * cs - s.arc[3] * sn),
    )
}
pub(super) fn layout(
    chart: &RadialChart,
    size: (u32, u32),
    scale: f32,
    annotate: bool,
) -> Result<Layout, RadialError> {
    let w = size.0 as f32;
    let h = size.1 as f32;
    let mut canvas = Canvas::new(
        if annotate {
            (w * scale).round() as u32
        } else {
            1
        },
        if annotate {
            (h * scale).round() as u32
        } else {
            1
        },
    )
    .ok_or(RadialError::Invalid("could not allocate label canvas"))?;
    let mut decorations = DecorationPainter::new(&chart.decorations, size, scale, annotate);
    let mut out = Layout {
        sectors: Vec::new(),
        groups: [[0.0; 4]; 2],
        holes: [0.0; 2],
        annotations: Vec::new(),
        decorations: DecorationPainter::new(&chart.decorations, size, scale, false).finish(),
        targets: Vec::new(),
    };
    let top = if chart.title.is_empty() { 24.0 } else { 70.0 };
    let bottom = h - 24.0;
    let automatic_area = crate::layout::RectF {
        x: 24.,
        y: top,
        width: w - 48.,
        height: bottom - top,
    };
    let area = chart
        .decorations
        .resolve_data_area(automatic_area)
        .map_err(RadialError::Invalid)?;
    decorations.add(DecorationId::DataArea, automatic_area);
    let (top, bottom) = (area.y, area.y + area.height);

    if !chart.title.is_empty() {
        let title_size = chart.font_size * 1.5 * scale;
        let tw = width(chart, &chart.title, title_size);
        if tw > (w - 32.0) * scale {
            return Err(RadialError::Invalid(
                "title does not fit; increase chart width or shorten it",
            ));
        }
        decorations.text(
            DecorationId::ChartTitle,
            &mut canvas,
            &chart.font_family,
            &chart.title,
            (w * scale - tw) * 0.5 / scale,
            42.0,
            title_size / scale,
            chart.label_color,
        );
    }
    let mut groups = vec![(
        &chart.slices,
        chart.kind,
        chart.start_angle_degrees.rem_euclid(360.0).to_radians(),
    )];
    if let Some(split) = &chart.split {
        groups.push((&split.children, split.kind, -std::f32::consts::FRAC_PI_2));
    }
    for (g, (slices, kind, start)) in groups.into_iter().enumerate() {
        let total = slices.iter().map(|s| s.value).sum::<f64>();
        let (left, right) = if chart.split.is_none() {
            (area.x, area.x + area.width)
        } else if g == 0 {
            (area.x, area.x + (area.width + 48.) * 0.6 - 48.)
        } else {
            (area.x + (area.width + 48.) * 0.6, area.x + area.width)
        };
        let max_explode = slices
            .iter()
            .filter(|s| s.value > 0.0)
            .map(|s| s.explode)
            .fold(0.0, f32::max);
        // The automatic area already reserves 24 logical pixels at both
        // outer edges (and either side of the split). Keep the original
        // default geometry while leaving room for all eight resize handles.
        let mut margin = 0.0;
        let outside =
            |slice: &RadialSlice| slice.labels.unwrap_or(chart.labels) == RadialLabels::Outside;
        if slices.iter().any(outside) {
            let max_label = slices
                .iter()
                .filter(|s| s.value > 0.0 && outside(s))
                .flat_map(|s| chart.label_lines(s, total))
                .map(|line| width(chart, &line, chart.font_size))
                .fold(0.0, f32::max);
            margin = max_label + 12.0;
        }
        let tilt = chart.style.tilt_degrees.to_radians();
        let cs = tilt.cos();
        let sn = tilt.sin();
        // Reserve the maximum lift even at rest: interaction must not resize the chart.
        let max_lift = slices
            .iter()
            .map(|s| s.style.as_ref().unwrap_or(&chart.style).hover_lift)
            .fold(0.0, f32::max);
        let max_depth = slices
            .iter()
            .map(|s| s.style.as_ref().unwrap_or(&chart.style).depth)
            .fold(chart.style.depth, f32::max);
        let shadow_margin = if slices
            .iter()
            .any(|s| s.style.as_ref().unwrap_or(&chart.style).shadow)
        {
            0.55
        } else {
            0.25
        };
        let extent = slices
            .iter()
            .filter(|s| s.value > 0.0)
            .map(|s| {
                let style = s.style.as_ref().unwrap_or(&chart.style);
                let tilt = style.tilt_degrees.to_radians();
                2.0 * tilt.cos() * (1.0 + max_explode)
                    + (max_depth + max_lift) * tilt.sin()
                    + shadow_margin
            })
            .fold(0.0, f32::max);
        // Labels cannot consume the entire chart. Keep the existing geometry
        // when it fits, and cap their reserved margin on a crowded canvas.
        margin = margin.min(((right - left) * 0.5 - 40.0 * (1.0 + max_explode)).max(0.));
        let radius = ((right - left - 2.0 * margin) / (2.0 * (1.0 + max_explode)))
            .min((bottom - top) / extent);
        let radius = if g == 1 { radius * 0.76 } else { radius };
        if radius < 32.0 {
            return Err(RadialError::Invalid("radial data area is too small"));
        }
        let center = [
            (left + right) * 0.5,
            (top + bottom) * 0.5 + chart.style.depth * sn * radius * 0.5,
        ];
        out.groups[g] = [center[0] * scale, center[1] * scale, radius * scale, cs];
        out.holes[g] = kind.inner_radius();
        let mut cumulative = 0.0f64;
        let first = out.sectors.len();
        for (index, slice) in slices.iter().enumerate() {
            if slice.value == 0.0 {
                continue;
            }
            let a = start + (cumulative / total) as f32 * TAU;
            cumulative += slice.value;
            // Derive both endpoints from the same cumulative sum; no accumulated
            // f32 wedge increments and no final-slice rounding hole.
            let b = start + (cumulative / total) as f32 * TAU;
            let style = slice.style.as_ref().unwrap_or(&chart.style);
            let target = RadialTarget { group: g, index };
            let amount = chart.interaction.amount(target);
            out.targets.push(target);
            out.sectors.push(Sector {
                geometry: [
                    center[0] * scale,
                    center[1] * scale,
                    radius * scale,
                    kind.inner_radius(),
                ],
                arc: [a, b - a, style.tilt_degrees.to_radians(), style.depth],
                color: [slice.color.r, slice.color.g, slice.color.b, 1.0],
                material: [
                    match style.material {
                        RadialMaterial::Flat => 0.0,
                        RadialMaterial::Matte => 1.0,
                        RadialMaterial::Ceramic => 2.0,
                        RadialMaterial::BrushedMetal => 3.0,
                        RadialMaterial::Paper => 4.0,
                        RadialMaterial::Wood => 5.0,
                        RadialMaterial::SatinMetal => 6.0,
                        RadialMaterial::Toon => 7.0,
                        RadialMaterial::Enamel => 8.0,
                        RadialMaterial::Hatch => 9.0,
                        RadialMaterial::Pearl => 10.0,
                    },
                    style.roughness,
                    style.texture_strength,
                    style.texture_scale,
                ],
                rounding: [
                    style.inner_corner,
                    style.outer_corner,
                    style.hover_lift * amount,
                    style.gloss,
                ],
                outline: [
                    if style.outline.rim { 1.0 } else { 0.0 },
                    if style.outline.separators { 1.0 } else { 0.0 },
                    if style.outline.emphasis { amount } else { 0.0 },
                    style.outline.width * scale,
                ],
                outline_color: [
                    style.outline.color.r,
                    style.outline.color.g,
                    style.outline.color.b,
                    style.outline.color.a,
                ],
                light: [
                    style.light[0],
                    style.light[1],
                    style.light[2],
                    if style.shadow { 1.0 } else { 0.0 },
                ],
                effects: [
                    amount * style.hover_brightness,
                    style.texture_angle_degrees.rem_euclid(360.0).to_radians(),
                    if slice.labels.unwrap_or(chart.labels) == RadialLabels::Inside
                        && !raster_moves_inside_labels(chart)
                    {
                        1.0
                    } else {
                        0.0
                    },
                    0.0,
                ],
                detail: [
                    slice.explode,
                    style.gap_degrees.to_radians().min((b - a) * 0.2),
                    style.bevel,
                    index as f32 * 13.17,
                ],
            });
        }
        let sectors = &out.sectors[first..];
        let positive: Vec<_> = slices.iter().filter(|s| s.value > 0.0).collect();
        {
            for ((s, slice), target) in sectors.iter().zip(&positive).zip(&out.targets[first..]) {
                if slice.labels.unwrap_or(chart.labels) != RadialLabels::Inside {
                    continue;
                }
                let mid = s.arc[0] + s.arc[1] * 0.5;
                let r = (s.geometry[3] + 1.0) * 0.5;
                let (x, mut y) = projected(s, mid, r.max(0.6));
                let lift = s.rounding[2] * s.arc[2].sin() * s.geometry[2];
                if raster_moves_inside_labels(chart) {
                    y -= lift;
                }
                let fs = chart.font_size * scale;
                let lines = chart.label_lines(slice, total);
                let text_width = lines
                    .iter()
                    .map(|s| width(chart, s, fs))
                    .fold(0.0, f32::max);
                // Keep inside glyphs anchored to their slice, so the cached
                // hover translation and measured hit map still agree. Fit the
                // text to the available face instead of rejecting the chart.
                let available = (s.arc[1] * r * s.geometry[2] * s.arc[2].cos())
                    .min(s.geometry[2] * (1.0 - s.geometry[3]));
                let fit = (available * 1.05 / text_width.max(1.))
                    .min(available / (fs * lines.len() as f32 * 1.2))
                    .min(1.);
                let fs = (fs * fit).max(scale);
                let luminance = s.color[0] * 0.2126 + s.color[1] * 0.7152 + s.color[2] * 0.0722;
                let color = slice.label_color.unwrap_or(if luminance < 0.5 {
                    crate::Color::WHITE
                } else {
                    chart.label_color
                });
                for (i, line) in lines.iter().enumerate() {
                    decorations.text(
                        DecorationId::RadialLabel(*target),
                        &mut canvas,
                        &chart.font_family,
                        line,
                        (x - width(chart, line, fs) * 0.5) / scale,
                        (y + (i as f32 * 1.2 - (lines.len() - 1) as f32 * 0.6 + 0.45) * fs) / scale,
                        fs / scale,
                        color,
                    );
                }
                if !raster_moves_inside_labels(chart) {
                    decorations.shift_hit(&DecorationId::RadialLabel(*target), 0., -lift / scale);
                }
            }
        }
        {
            for left_side in [true, false] {
                let mut labels: Vec<_> = sectors
                    .iter()
                    .zip(&positive)
                    .zip(&out.targets[first..])
                    .filter(|((_, slice), _)| outside(slice))
                    .filter(|((s, _), _)| ((s.arc[0] + s.arc[1] * 0.5).cos() < 0.0) == left_side)
                    .map(|((s, slice), target)| {
                        let mid = s.arc[0] + s.arc[1] * 0.5;
                        let mut anchor = projected(s, mid, 1.015);
                        // The front silhouette is the bottom of the extruded wall,
                        // not the top rim: leaders must not cross the visible face.
                        if mid.sin() > 0.0 {
                            anchor.1 += s.arc[3] * s.arc[2].sin() * s.geometry[2];
                        }
                        (anchor, slice, *target)
                    })
                    .collect();
                labels.sort_by(|a, b| a.0.1.total_cmp(&b.0.1));
                let intervals = labels.len().saturating_sub(1) as f32;
                let fs = (chart.font_size * scale)
                    .min((bottom - top) * scale / (2.4 + 2.7 * intervals))
                    .max(scale);
                let min = top * scale + fs;
                let max = (bottom * scale - fs * 1.4).max(min);
                let step = (fs * 2.7).min((max - min) / intervals.max(1.));
                let mut ys = Vec::new();
                let mut prev = min - step;
                for (anchor, _, _) in &labels {
                    let y = anchor.1.clamp(min, max).max(prev + step);
                    ys.push(y);
                    prev = y;
                }
                if let Some(last) = ys.last().copied() {
                    let shift = (last - max).max(0.0);
                    for y in &mut ys {
                        *y -= shift;
                    }
                }
                let elbow = (center[0]
                    + if left_side {
                        -radius * (1.0 + max_explode) - 12.0
                    } else {
                        radius * (1.0 + max_explode) + 12.0
                    })
                    * scale;
                let rule = Paint::stroke(&crate::Color::new(0.48, 0.53, 0.6, 0.75), scale);
                for ((anchor, slice, target), y) in labels.into_iter().zip(ys) {
                    let tx = elbow + if left_side { -8.0 * scale } else { 8.0 * scale };
                    let offset = decorations
                        .offset(&DecorationId::RadialLabel(target))
                        .map(|v| v * scale);
                    if annotate {
                        let elbow = (elbow + offset[0], y + offset[1]);
                        canvas.draw_line(anchor, elbow, &rule);
                        canvas.draw_line(elbow, (tx + offset[0], y + offset[1]), &rule);
                    }
                    let lines = chart.label_lines(slice, total);
                    for (i, line) in lines.iter().enumerate() {
                        let x = if left_side {
                            tx - width(chart, line, fs)
                        } else {
                            tx
                        };
                        decorations.text(
                            DecorationId::RadialLabel(target),
                            &mut canvas,
                            &chart.font_family,
                            line,
                            x / scale,
                            (y + (i as f32 * 1.2 - 0.15) * fs) / scale,
                            fs / scale,
                            slice.label_color.unwrap_or(chart.label_color),
                        );
                    }
                }
            }
        }
    }
    if let Some(split) = &chart.split {
        let parent = chart.slices[..split.slice_index]
            .iter()
            .filter(|s| s.value > 0.0)
            .count();
        let sector = &out.sectors[parent];
        let other = out.groups[1];
        let paint = Paint::stroke(&crate::Color::new(0.55, 0.58, 0.62, 0.65), scale);
        for (a, dy) in [(sector.arc[0], -0.8), (sector.arc[0] + sector.arc[1], 0.8)] {
            canvas.draw_line(
                projected(sector, a, 1.025),
                (
                    other[0] - other[2] * 0.6,
                    other[1] + other[2] * other[3] * dy,
                ),
                &paint,
            );
        }
    }
    out.decorations = decorations.finish();
    if annotate {
        out.decorations.draw_selection(&mut canvas, scale);
        out.annotations = canvas.into_rgba();
    }
    Ok(out)
}
