use super::*;
use crate::raster::{Canvas, Paint};
use crate::text_render::{FontPolicy, draw_plain_text, measure_plain_text};
use std::f32::consts::TAU;

pub(super) struct Layout {
    pub sectors: Vec<Sector>,
    pub groups: [[f32; 4]; 2],
    pub holes: [f32; 2],
    pub annotations: Vec<u8>,
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
fn text(
    canvas: &mut Canvas,
    c: &RadialChart,
    s: &str,
    x: f32,
    y: f32,
    size: f32,
    color: crate::Color,
) {
    draw_plain_text(
        canvas,
        s,
        (x, y),
        color,
        &c.font_family,
        size,
        false,
        false,
        FontPolicy::Standard,
    );
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
    let mut out = Layout {
        sectors: Vec::new(),
        groups: [[0.0; 4]; 2],
        holes: [0.0; 2],
        annotations: Vec::new(),
        targets: Vec::new(),
    };
    let top = if chart.title.is_empty() { 24.0 } else { 70.0 };
    let bottom = h - 24.0;
    if annotate && !chart.title.is_empty() {
        let title_size = chart.font_size * 1.5 * scale;
        let tw = width(chart, &chart.title, title_size);
        if tw > (w - 32.0) * scale {
            return Err(RadialError::Invalid(
                "title does not fit; increase chart width or shorten it",
            ));
        }
        text(
            &mut canvas,
            chart,
            &chart.title,
            (w * scale - tw) * 0.5,
            42.0 * scale,
            title_size,
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
            (0.0, w)
        } else if g == 0 {
            (0.0, w * 0.6)
        } else {
            (w * 0.6, w)
        };
        let max_explode = slices
            .iter()
            .filter(|s| s.value > 0.0)
            .map(|s| s.explode)
            .fold(0.0, f32::max);
        let mut margin = 24.0;
        let outside =
            |slice: &RadialSlice| slice.labels.unwrap_or(chart.labels) == RadialLabels::Outside;
        if slices.iter().any(outside) {
            let max_label = slices
                .iter()
                .filter(|s| s.value > 0.0 && outside(s))
                .flat_map(|s| chart.label_lines(s, total))
                .map(|line| width(chart, &line, chart.font_size))
                .fold(0.0, f32::max);
            margin = max_label + 36.0;
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
        let radius = ((right - left - 2.0 * margin) / (2.0 * (1.0 + max_explode)))
            .min((bottom - top) / extent);
        let radius = if g == 1 { radius * 0.76 } else { radius };
        if radius < 32.0 {
            return Err(RadialError::Invalid(
                "radial labels do not fit; enlarge the canvas or use fewer/shorter labels",
            ));
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
                    if slice.labels.unwrap_or(chart.labels) == RadialLabels::Inside {
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
        if annotate {
            for (s, slice) in sectors.iter().zip(&positive) {
                if slice.labels.unwrap_or(chart.labels) != RadialLabels::Inside {
                    continue;
                }
                let mid = s.arc[0] + s.arc[1] * 0.5;
                let r = (s.geometry[3] + 1.0) * 0.5;
                let (x, y) = projected(s, mid, r.max(0.6));
                let fs = chart.font_size * scale;
                let lines = chart.label_lines(slice, total);
                // Fail visibly instead of hiding or silently truncating categories.
                let available = (s.arc[1] * r * s.geometry[2] * s.arc[2].cos())
                    .min(s.geometry[2] * (1.0 - s.geometry[3]));
                if lines
                    .iter()
                    .map(|s| width(chart, s, fs))
                    .fold(0.0, f32::max)
                    > available * 1.05
                    || fs * (lines.len() as f32 * 1.2) > available
                {
                    return Err(RadialError::Invalid(
                        "inside labels do not fit; use outside labels or a larger canvas",
                    ));
                }
                let luminance = s.color[0] * 0.2126 + s.color[1] * 0.7152 + s.color[2] * 0.0722;
                let color = slice.label_color.unwrap_or(if luminance < 0.5 {
                    crate::Color::WHITE
                } else {
                    chart.label_color
                });
                for (i, line) in lines.iter().enumerate() {
                    text(
                        &mut canvas,
                        chart,
                        line,
                        x - width(chart, line, fs) * 0.5,
                        y + (i as f32 * 1.2 - (lines.len() - 1) as f32 * 0.6 + 0.45) * fs,
                        fs,
                        color,
                    );
                }
            }
        }
        if annotate {
            for left_side in [true, false] {
                let mut labels: Vec<_> = sectors
                    .iter()
                    .zip(&positive)
                    .filter(|(_, slice)| outside(slice))
                    .filter(|(s, _)| ((s.arc[0] + s.arc[1] * 0.5).cos() < 0.0) == left_side)
                    .map(|(s, slice)| {
                        let mid = s.arc[0] + s.arc[1] * 0.5;
                        let mut anchor = projected(s, mid, 1.015);
                        // The front silhouette is the bottom of the extruded wall,
                        // not the top rim: leaders must not cross the visible face.
                        if mid.sin() > 0.0 {
                            anchor.1 += s.arc[3] * s.arc[2].sin() * s.geometry[2];
                        }
                        (anchor, slice)
                    })
                    .collect();
                labels.sort_by(|a, b| a.0.1.total_cmp(&b.0.1));
                let fs = chart.font_size * scale;
                let step = fs * 2.7;
                let min = (top + chart.font_size) * scale;
                let max = (bottom - chart.font_size * 1.4) * scale;
                if (labels.len().saturating_sub(1)) as f32 * step > max - min {
                    return Err(RadialError::Invalid(
                        "too many outside labels for this height",
                    ));
                }
                let mut ys = Vec::new();
                let mut prev = min - step;
                for (anchor, _) in &labels {
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
                for ((anchor, slice), y) in labels.into_iter().zip(ys) {
                    let tx = elbow + if left_side { -8.0 * scale } else { 8.0 * scale };
                    canvas.draw_line(anchor, (elbow, y), &rule);
                    canvas.draw_line((elbow, y), (tx, y), &rule);
                    let lines = chart.label_lines(slice, total);
                    for (i, line) in lines.iter().enumerate() {
                        let x = if left_side {
                            tx - width(chart, line, fs)
                        } else {
                            tx
                        };
                        text(
                            &mut canvas,
                            chart,
                            line,
                            x,
                            y + (i as f32 * 1.2 - 0.15) * fs,
                            fs,
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
    if annotate {
        out.annotations = canvas.into_rgba();
    }
    Ok(out)
}
