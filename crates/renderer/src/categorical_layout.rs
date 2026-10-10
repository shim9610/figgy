use super::*;
use crate::raster::{Canvas, Paint};
use crate::text_render::{FontPolicy, draw_plain_text, measure_plain_text};
use model::config::{AxisOptions, AxisScale};

pub(super) struct Layout {
    pub bars: Vec<Bar>,
    pub targets: Vec<CategoryBarTarget>,
    pub annotations: Vec<u8>,
}
fn measure(c: &CategoricalChart, s: &str, size: f32) -> f32 {
    measure_plain_text(s, &c.font_family, size, false, false, FontPolicy::Standard).width
}
fn text(
    canvas: &mut Canvas,
    c: &CategoricalChart,
    s: &str,
    x: f32,
    y: f32,
    size: f32,
    color: crate::Color,
    scale: f32,
) {
    draw_plain_text(
        canvas,
        s,
        (x * scale, y * scale),
        color,
        &c.font_family,
        size * scale,
        false,
        false,
        FontPolicy::Standard,
    );
}
fn line(canvas: &mut Canvas, a: (f32, f32), b: (f32, f32), color: crate::Color, scale: f32) {
    canvas.draw_line(
        (a.0 * scale, a.1 * scale),
        (b.0 * scale, b.1 * scale),
        &Paint::stroke(&color, scale),
    );
}
fn number(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    if v.abs() >= 1e6 || v.abs() < 0.001 {
        format!("{v:.2e}")
    } else {
        format!("{v:.6}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned()
    }
}
pub(super) fn layout(
    c: &CategoricalChart,
    size: (u32, u32),
    scale: f32,
    annotate: bool,
) -> Result<Layout, CategoricalError> {
    let invalid = CategoricalError::Invalid;
    let segments = c.segments().map_err(invalid)?;
    let (w, h) = (size.0 as f32, size.1 as f32);
    let fs = c.font_size;
    let horizontal = c.direction == CategoryBarDirection::Horizontal;
    let percent = c.mode == CategoryBarMode::PercentStacked;
    let mut low = 0.0f64;
    let mut high = 0.0f64;
    for s in &segments {
        low = low.min(s.start).min(s.end);
        high = high.max(s.start).max(s.end);
    }
    if percent {
        high = 100.0;
    } else if low == high {
        high = 1.0;
    }
    let magnitude = low.abs().max(high.abs());
    let unit = 10.0f64
        .powf(magnitude.log10().floor())
        .max(f64::MIN_POSITIVE);
    let outside_labels = c.labels == CategoryBarLabels::Outside
        || (c.labels == CategoryBarLabels::Auto && c.mode == CategoryBarMode::Grouped)
        || c.overrides
            .iter()
            .any(|o| o.labels == Some(CategoryBarLabels::Outside));
    let padding = if outside_labels && !percent {
        0.12 * (high / unit - low / unit)
    } else {
        0.0
    };
    let axis_low = low / unit - if low < 0.0 { padding } else { 0.0 };
    let axis_high = high / unit + if high > 0.0 { padding } else { 0.0 };
    let plan = AxisOptions::compute_nice_ticks(AxisScale::Linear, axis_low, axis_high, 5)
        .map_err(|_| invalid("categorical axis range cannot be represented"))?;
    let span = plan.max - plan.min;
    let norm = |v: f64| ((v / unit - plan.min) / span) as f32;
    let tick_count = ((plan.max - plan.min) / plan.major_spacing).round() as usize;
    let mut ticks = Vec::new();
    for i in 0..=tick_count {
        let normalized = plan.min + i as f64 * plan.major_spacing;
        let value = normalized * unit;
        if !value.is_finite() {
            return Err(invalid("categorical axis tick overflow"));
        }
        let label = if percent {
            format!("{}%", number(value))
        } else {
            number(value)
        };
        ticks.push((((normalized - plan.min) / span) as f32, label));
    }
    let tick_width = ticks
        .iter()
        .map(|(_, s)| measure(c, s, fs))
        .fold(0.0, f32::max);
    let category_width = c
        .categories
        .iter()
        .map(|x| measure(c, &x.label, fs))
        .fold(0.0, f32::max);
    let value_width = segments
        .iter()
        .map(|s| measure(c, &c.label_text(s), fs))
        .fold(0.0, f32::max);
    let mut legend = Vec::new();
    let mut row_width = 0.0;
    let mut row = 0usize;
    if c.legend {
        for series in &c.series {
            let width = measure(c, &series.label, fs) + 34.0;
            if width > w - 40.0 {
                return Err(invalid("legend label does not fit"));
            }
            if row_width + width > w - 40.0 && row_width > 0.0 {
                row += 1;
                row_width = 0.0;
            }
            legend.push((row, row_width, width, series));
            row_width += width;
        }
    }
    let legend_height = if legend.is_empty() {
        0.0
    } else {
        (row + 1) as f32 * (fs + 12.0) + 12.0
    };
    let left = if horizontal {
        category_width
            + 24.0
            + if c.category_title.is_empty() {
                0.0
            } else {
                fs + 12.0
            }
    } else {
        tick_width
            + 24.0
            + if c.value_title.is_empty() {
                0.0
            } else {
                fs + 12.0
            }
    };
    let right = w - if horizontal {
        (value_width + 18.0).max(32.0)
    } else {
        24.0
    };
    let top = if c.title.is_empty() {
        32.0
    } else {
        fs * 1.5 + 42.0
    };
    let axis_title = if horizontal {
        &c.value_title
    } else {
        &c.category_title
    };
    let bottom = h
        - legend_height
        - fs
        - 28.0
        - if axis_title.is_empty() {
            0.0
        } else {
            fs + 12.0
        };
    if right - left < 80.0 || bottom - top < 60.0 {
        return Err(invalid(
            "categorical labels leave too little plot space; enlarge canvas",
        ));
    }
    let slot = if horizontal {
        (bottom - top) / c.categories.len() as f32
    } else {
        (right - left) / c.categories.len() as f32
    };
    if (!horizontal && category_width > slot - 8.0) || (horizontal && fs + 8.0 > slot) {
        return Err(invalid(
            "category labels overlap; enlarge canvas or shorten labels",
        ));
    }
    if horizontal
        && ticks.windows(2).any(|t| {
            (t[1].0 - t[0].0) * (right - left)
                < (measure(c, &t[0].1, fs) + measure(c, &t[1].1, fs)) * 0.5 + 8.0
        })
    {
        return Err(invalid("value ticks overlap; enlarge canvas"));
    }
    if !horizontal && (bottom - top) / (tick_count.max(1) as f32) < fs + 4.0 {
        return Err(invalid("value ticks overlap; enlarge canvas"));
    }
    let group = c.mode == CategoryBarMode::Grouped;
    let bars_per_slot = if group { c.series.len() } else { 1 };
    let thickness =
        (slot * c.group_width - c.bar_gap * (bars_per_slot - 1) as f32) / bars_per_slot as f32;
    if thickness < 1.0 {
        return Err(invalid("bars do not fit the category slots"));
    }
    let axis = |v: f64| {
        if horizontal {
            left + norm(v) * (right - left)
        } else {
            bottom - norm(v) * (bottom - top)
        }
    };
    let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
    let mut back = Canvas::new(if annotate { pw } else { 1 }, if annotate { ph } else { 1 })
        .ok_or(invalid("annotation allocation failed"))?;
    let mut front = Canvas::new(if annotate { pw } else { 1 }, if annotate { ph } else { 1 })
        .ok_or(invalid("annotation allocation failed"))?;
    if annotate {
        if !c.title.is_empty() {
            let tw = measure(c, &c.title, fs * 1.5);
            if tw > w - 32.0 {
                return Err(invalid("title does not fit"));
            }
            text(
                &mut front,
                c,
                &c.title,
                (w - tw) * 0.5,
                fs * 1.5 + 12.0,
                fs * 1.5,
                c.text_color,
                scale,
            );
        }
        let grid = crate::Color::from_rgb8(224, 229, 235);
        for (t, label) in &ticks {
            if horizontal {
                let x = left + t * (right - left);
                if c.grid {
                    line(&mut back, (x, top), (x, bottom), grid, scale);
                }
                text(
                    &mut front,
                    c,
                    label,
                    x - measure(c, label, fs) * 0.5,
                    bottom + fs + 8.0,
                    fs,
                    c.text_color,
                    scale,
                );
            } else {
                let y = bottom - t * (bottom - top);
                if c.grid {
                    line(&mut back, (left, y), (right, y), grid, scale);
                }
                text(
                    &mut front,
                    c,
                    label,
                    left - measure(c, label, fs) - 10.0,
                    y + fs * 0.32,
                    fs,
                    c.text_color,
                    scale,
                );
            }
        }
        if horizontal {
            line(
                &mut back,
                (axis(0.0), top),
                (axis(0.0), bottom),
                c.text_color,
                scale,
            );
        } else {
            line(
                &mut back,
                (left, axis(0.0)),
                (right, axis(0.0)),
                c.text_color,
                scale,
            );
        }
        for (i, cat) in c.categories.iter().enumerate() {
            let center = if horizontal {
                top + (i as f32 + 0.5) * slot
            } else {
                left + (i as f32 + 0.5) * slot
            };
            let tw = measure(c, &cat.label, fs);
            if horizontal {
                text(
                    &mut front,
                    c,
                    &cat.label,
                    left - tw - 10.0,
                    center + fs * 0.32,
                    fs,
                    c.text_color,
                    scale,
                );
            } else {
                text(
                    &mut front,
                    c,
                    &cat.label,
                    center - tw * 0.5,
                    bottom + fs + 8.0,
                    fs,
                    c.text_color,
                    scale,
                );
            }
        }
        if !axis_title.is_empty() {
            let tw = measure(c, axis_title, fs);
            if tw > right - left {
                return Err(invalid("axis title does not fit"));
            }
            text(
                &mut front,
                c,
                axis_title,
                (left + right - tw) * 0.5,
                bottom + 2.0 * fs + 22.0,
                fs,
                c.text_color,
                scale,
            );
        }
        let side_title = if horizontal {
            &c.category_title
        } else {
            &c.value_title
        };
        if !side_title.is_empty() {
            let tw = measure(c, side_title, fs);
            if tw > bottom - top {
                return Err(invalid("axis title does not fit"));
            }
            let x = fs + 4.0;
            let y = (top + bottom) * 0.5;
            front.save();
            front.rotate_at(-90.0, x * scale, y * scale);
            text(
                &mut front,
                c,
                side_title,
                x - tw * 0.5,
                y,
                fs,
                c.text_color,
                scale,
            );
            front.restore();
        }
        for (r, x, _, series) in &legend {
            let total = legend
                .iter()
                .filter(|(rr, _, _, _)| rr == r)
                .map(|(_, _, v, _)| v)
                .sum::<f32>();
            let lx = (w - total) * 0.5 + x;
            let ly = h - legend_height + 12.0 + *r as f32 * (fs + 12.0);
            back.draw_rect(
                lx * scale,
                (ly - fs * 0.65) * scale,
                fs * scale,
                fs * 0.65 * scale,
                &Paint::fill(&series.color),
            );
            text(
                &mut front,
                c,
                &series.label,
                lx + fs + 7.0,
                ly,
                fs,
                c.text_color,
                scale,
            );
        }
    }
    let mut out = Layout {
        bars: Vec::new(),
        targets: Vec::new(),
        annotations: Vec::new(),
    };
    let mut text_boxes: Vec<[f32; 4]> = Vec::new();
    for s in &segments {
        let series = &c.series[s.series_index];
        let edit = c.bar_override(&s.target);
        let style = edit
            .and_then(|o| o.style.as_ref())
            .or(series.style.as_ref())
            .unwrap_or(&c.style);
        let color = edit.and_then(|o| o.color).unwrap_or(series.color);
        let center = if horizontal {
            top + (s.category_index as f32 + 0.5) * slot
        } else {
            left + (s.category_index as f32 + 0.5) * slot
        };
        let cross = center - slot * c.group_width * 0.5
            + if group {
                s.series_index as f32 * (thickness + c.bar_gap)
            } else {
                0.0
            };
        let a = axis(s.start);
        let b = axis(s.end);
        let rect = if horizontal {
            [a.min(b), cross, a.max(b), cross + thickness]
        } else {
            [cross, a.min(b), cross + thickness, a.max(b)]
        };
        let mut radii = [0.0; 4];
        let r = style
            .corner_radius
            .min(thickness * 0.5)
            .min((a - b).abs() * 0.5)
            * scale;
        if s.exposed_end {
            if horizontal {
                if s.value >= 0.0 {
                    radii[1] = r;
                    radii[2] = r;
                } else {
                    radii[0] = r;
                    radii[3] = r;
                }
            } else if s.value >= 0.0 {
                radii[0] = r;
                radii[1] = r;
            } else {
                radii[2] = r;
                radii[3] = r;
            }
        }
        if s.value != 0.0 && rect[2] > rect[0] && rect[3] > rect[1] {
            let material = match style.material {
                CategoryBarMaterial::Flat => 0.0,
                CategoryBarMaterial::Matte => 1.0,
                CategoryBarMaterial::SatinMetal => 2.0,
                CategoryBarMaterial::Enamel => 3.0,
                CategoryBarMaterial::Paper => 4.0,
            };
            let emphasized =
                c.hovered.as_ref() == Some(&s.target) || c.selected.as_ref() == Some(&s.target);
            // Stable grain seed depends on identity rather than array order.
            let seed = s
                .target
                .category_id
                .bytes()
                .chain(s.target.series_id.bytes())
                .fold(2166136261u32, |a, b| {
                    (a ^ u32::from(b)).wrapping_mul(16777619)
                });
            // Shared stack edges use a single pixel owner. Independently blending
            // complementary AA coverages would leak the background through a seam.
            let (start_edge, end_edge) = match (horizontal, s.value >= 0.0) {
                (true, true) => (1u32, 2u32),
                (true, false) => (2, 1),
                (false, true) => (8, 4),
                (false, false) => (4, 8),
            };
            let shared = if group {
                0
            } else {
                (if s.start != 0.0 { start_edge } else { 0 })
                    | (if s.exposed_end { 0 } else { end_edge })
            };
            out.bars.push(Bar {
                rect: rect.map(|v| v * scale),
                radii,
                color: [color.r, color.g, color.b, color.a],
                material: [
                    material,
                    style.texture_strength,
                    style.texture_scale,
                    style.gloss,
                ],
                outline: [
                    if style.outline { 1.0 } else { 0.0 },
                    style.outline_width * scale,
                    if horizontal { 1.0 } else { 0.0 },
                    (seed % 4096) as f32,
                ],
                outline_color: [
                    style.outline_color.r,
                    style.outline_color.g,
                    style.outline_color.b,
                    style.outline_color.a,
                ],
                effects: [
                    if emphasized {
                        style.emphasis_brightness
                    } else {
                        0.0
                    },
                    scale,
                    shared as f32,
                    0.0,
                ],
            });
            out.targets.push(s.target.clone());
        }
        if !annotate {
            continue;
        }
        let mode = edit.and_then(|o| o.labels).unwrap_or(c.labels);
        if mode == CategoryBarLabels::None {
            continue;
        }
        let label = c.label_text(s);
        let tw = measure(c, &label, fs);
        let inside =
            mode == CategoryBarLabels::Inside || (mode == CategoryBarLabels::Auto && !group);
        if !inside && !group && !s.exposed_end && s.value != 0.0 {
            return Err(invalid(
                "outside labels require grouped bars or an exposed stack end",
            ));
        }
        let (x, y) = if inside {
            (
                (rect[0] + rect[2] - tw) * 0.5,
                (rect[1] + rect[3]) * 0.5 + fs * 0.32,
            )
        } else if horizontal {
            (
                if s.value >= 0.0 {
                    b + 6.0
                } else {
                    b - tw - 6.0
                },
                cross + thickness * 0.5 + fs * 0.32,
            )
        } else {
            (
                cross + (thickness - tw) * 0.5,
                if s.value >= 0.0 {
                    b - 6.0
                } else {
                    b + fs + 6.0
                },
            )
        };
        let bounds = [x, y - fs, x + tw, y + fs * 0.2];
        let fits = (!inside || (tw + 8.0 <= rect[2] - rect[0] && fs * 1.4 <= rect[3] - rect[1]))
            && x >= 0.0
            && x + tw <= w
            && y - fs >= 0.0
            && y < h;
        let overlaps = text_boxes.iter().any(|q| {
            bounds[0] < q[2] + 3.0
                && bounds[2] + 3.0 > q[0]
                && bounds[1] < q[3] + 2.0
                && bounds[3] + 2.0 > q[1]
        });
        if !fits || overlaps {
            if mode == CategoryBarLabels::Auto {
                continue;
            } else {
                return Err(invalid(
                    "bar labels do not fit; enlarge canvas, reduce text or use Auto",
                ));
            }
        }
        text_boxes.push(bounds);
        let ink = if inside && color.r * 0.2126 + color.g * 0.7152 + color.b * 0.0722 < 0.55 {
            crate::Color::WHITE
        } else {
            c.text_color
        };
        text(&mut front, c, &label, x, y, fs, ink, scale);
    }
    if annotate {
        out.annotations = back.into_rgba();
        out.annotations.extend(front.into_rgba());
    }
    Ok(out)
}
