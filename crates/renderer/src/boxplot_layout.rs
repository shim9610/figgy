use super::*;
use crate::raster::{Canvas, Paint};
use crate::text_render::{FontPolicy, draw_plain_text, measure_plain_text};
use model::config::{AxisOptions, AxisScale};

pub(super) struct Layout {
    pub bars: Vec<Bar>,
    pub targets: Vec<BoxPlotPick>,
    pub annotations: Vec<u8>,
    pub clip: [f32; 4],
}
fn measure(c: &BoxPlotChart, s: &str, size: f32) -> f32 {
    measure_plain_text(s, &c.font_family, size, false, false, FontPolicy::Standard).width
}
fn text(
    canvas: &mut Canvas,
    c: &BoxPlotChart,
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
    c: &BoxPlotChart,
    size: (u32, u32),
    scale: f32,
    annotate: bool,
) -> Result<Layout, BoxPlotError> {
    let invalid = BoxPlotError::Invalid;
    let (w, h) = (size.0 as f32, size.1 as f32);
    let fs = c.font_size;
    let horizontal = c.direction == BoxPlotDirection::Horizontal;
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    for s in &c.series {
        for v in s.values.iter().flatten() {
            for value in [v.whisker_low, v.q1, v.median, v.q3, v.whisker_high]
                .into_iter()
                .chain(v.mean)
                .chain(v.median_ci.into_iter().flatten())
                .chain(v.outliers.iter().copied())
            {
                low = low.min(value);
                high = high.max(value);
            }
        }
    }
    if !low.is_finite() {
        low = 0.0;
        high = 1.0;
    }
    if let Some(r) = c.value_range {
        low = r[0];
        high = r[1];
    }
    let magnitude = low.abs().max(high.abs()).max(f64::MIN_POSITIVE);
    let unit = 10.0f64
        .powf(magnitude.log10().floor())
        .max(f64::MIN_POSITIVE);
    let mut a = low / unit;
    let mut b = high / unit;
    if a == b {
        let delta = (a.abs() * 0.05).max(0.5);
        a -= delta;
        b += delta;
    }
    if c.value_range.is_none() {
        let pad = (b - a) * 0.1;
        a -= pad;
        b += pad;
    }
    let mut plan = AxisOptions::compute_nice_ticks(AxisScale::Linear, a, b, 5)
        .map_err(|_| invalid("boxplot axis range cannot be represented"))?;
    if c.value_range.is_some() {
        plan.min = a;
        plan.max = b;
    }
    let span = plan.max - plan.min;
    if !span.is_finite() || span <= 0.0 {
        return Err(invalid("boxplot axis span is not representable"));
    }
    // Distant clipped geometry is bounded before converting to f32.
    let norm = |v: f64| ((v / unit - plan.min) / span).clamp(-10000.0, 10000.0) as f32;
    let start = (plan.min / plan.major_spacing).ceil() * plan.major_spacing;
    let tick_count = ((plan.max - start) / plan.major_spacing).floor().max(0.0) as usize;
    if tick_count > 100 {
        return Err(invalid("too many value ticks"));
    }
    let mut ticks = Vec::new();
    for i in 0..=tick_count {
        let normalized = start + i as f64 * plan.major_spacing;
        let value = normalized * unit;
        if !value.is_finite() {
            return Err(invalid("boxplot axis tick overflow"));
        }
        let label = number(value);
        if ticks.last().is_some_and(|t: &(f32, String)| t.1 == label) {
            return Err(invalid("value ticks need a wider range to remain distinct"));
        }
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
        (tick_width * 0.5 + 12.0).max(32.0)
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
            "boxplot labels leave too little plot space; enlarge canvas",
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
    let bars_per_slot = c.series.len();
    let thickness =
        (slot * c.group_width - c.box_gap * (bars_per_slot - 1) as f32) / bars_per_slot as f32;
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
        line(
            &mut back,
            (left, bottom),
            (right, bottom),
            c.text_color,
            scale,
        );
        line(&mut back, (left, top), (left, bottom), c.text_color, scale);
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
        bars: vec![],
        targets: vec![],
        annotations: vec![],
        clip: [left * scale, top * scale, right * scale, bottom * scale],
    };
    let mut text_boxes: Vec<[f32; 4]> = vec![];
    for (i, category) in c.categories.iter().enumerate() {
        for (j, series) in c.series.iter().enumerate() {
            let Some(v) = &series.values[i] else {
                continue;
            };
            let target = BoxPlotTarget::new(&category.id, &series.id);
            let edit = c.box_override(&target);
            let st = c.resolved_style(&target).unwrap();
            let color = edit.and_then(|o| o.color).unwrap_or(series.color);
            let center = if horizontal {
                top + (i as f32 + 0.5) * slot
            } else {
                left + (i as f32 + 0.5) * slot
            };
            let cross = center - slot * c.group_width * 0.5 + j as f32 * (thickness + c.box_gap);
            let middle = cross + thickness * 0.5;
            let q1 = axis(v.q1);
            let q3 = axis(v.q3);
            let med = axis(v.median);
            let wl = axis(v.whisker_low);
            let wh = axis(v.whisker_high);
            let emphasized =
                c.hovered.as_ref() == Some(&target) || c.selected.as_ref() == Some(&target);
            let seed = target
                .category_id
                .bytes()
                .chain(target.series_id.bytes())
                .fold(2166136261u32, |a, b| {
                    (a ^ u32::from(b)).wrapping_mul(16777619)
                });
            let base = Bar {
                effects: [
                    if emphasized {
                        st.emphasis_brightness
                    } else {
                        0.0
                    },
                    scale,
                    0.0,
                    0.0,
                ],
                outline: [
                    0.0,
                    0.0,
                    if horizontal { 1.0 } else { 0.0 },
                    (seed % 4096) as f32,
                ],
                ..EMPTY_BAR
            };
            let xy = |cross: f32, along: f32| {
                if horizontal {
                    [along, cross]
                } else {
                    [cross, along]
                }
            };
            let mut push = |mut b: Bar, part: BoxPlotPart| {
                // All packed coordinates, including notch endpoints, are physical.
                if b.rect[2] < out.clip[0] / scale
                    || b.rect[0] > out.clip[2] / scale
                    || b.rect[3] < out.clip[1] / scale
                    || b.rect[1] > out.clip[3] / scale
                {
                    return;
                }
                b.rect = b.rect.map(|x| x * scale);
                b.radii = b.radii.map(|x| x * scale);
                b.outline[1] *= scale;
                b.notch = b.notch.map(|x| x * scale);
                b.body = b.body.map(|x| x * scale);
                out.bars.push(b);
                out.targets.push(BoxPlotPick {
                    target: target.clone(),
                    part,
                });
            };
            let stroke = |a: [f32; 2], b: [f32; 2], width: f32, color: crate::Color| Bar {
                rect: [
                    a[0].min(b[0]) - if a[0] == b[0] { width * 0.5 } else { 0.0 },
                    a[1].min(b[1]) - if a[1] == b[1] { width * 0.5 } else { 0.0 },
                    a[0].max(b[0]) + if a[0] == b[0] { width * 0.5 } else { 0.0 },
                    a[1].max(b[1]) + if a[1] == b[1] { width * 0.5 } else { 0.0 },
                ],
                color: [color.r, color.g, color.b, color.a],
                ..base
            };
            if st.whisker_width > 0.0 {
                if wl != q1 {
                    push(
                        stroke(
                            xy(middle, wl),
                            xy(middle, q1),
                            st.whisker_width,
                            st.whisker_color,
                        ),
                        BoxPlotPart::WhiskerLow,
                    );
                }
                if wh != q3 {
                    push(
                        stroke(
                            xy(middle, q3),
                            xy(middle, wh),
                            st.whisker_width,
                            st.whisker_color,
                        ),
                        BoxPlotPart::WhiskerHigh,
                    );
                }
                if st.caps && st.cap_ratio > 0.0 {
                    for (end, part) in [(wl, BoxPlotPart::CapLow), (wh, BoxPlotPart::CapHigh)] {
                        push(
                            stroke(
                                xy(middle - thickness * st.cap_ratio * 0.5, end),
                                xy(middle + thickness * st.cap_ratio * 0.5, end),
                                st.whisker_width,
                                st.whisker_color,
                            ),
                            part,
                        );
                    }
                }
            }
            let a = xy(cross, q1.min(q3));
            let b = xy(cross + thickness, q1.max(q3));
            let rect = [a[0], a[1], b[0], b[1]];
            if q1 != q3 {
                let mut body = Bar {
                    rect,
                    radii: [st
                        .corner_radius
                        .min(thickness * 0.5)
                        .min((q1 - q3).abs() * 0.5); 4],
                    color: [color.r, color.g, color.b, color.a],
                    material: [
                        match st.material {
                            BoxPlotMaterial::Flat => 0.0,
                            BoxPlotMaterial::Matte => 1.0,
                            BoxPlotMaterial::SatinMetal => 2.0,
                        },
                        st.texture_strength,
                        st.texture_scale,
                        st.gloss,
                    ],
                    outline: [
                        if st.outline { 1.0 } else { 0.0 },
                        st.outline_width,
                        base.outline[2],
                        base.outline[3],
                    ],
                    outline_color: [
                        st.outline_color.r,
                        st.outline_color.g,
                        st.outline_color.b,
                        st.outline_color.a,
                    ],
                    ..base
                };
                if st.notched {
                    let ci = v.median_ci.unwrap();
                    let ca = axis(ci[0]);
                    let cb = axis(ci[1]);
                    body.effects[2] = 1.0;
                    body.body = [cross, q1.min(q3), cross + thickness, q1.max(q3)];
                    body.notch = [ca.min(cb), med, ca.max(cb), thickness * st.notch_depth];
                    let a = xy(cross, q1.min(q3).min(ca.min(cb)));
                    let b = xy(cross + thickness, q1.max(q3).max(ca.max(cb)));
                    body.rect = [a[0], a[1], b[0], b[1]];
                }
                push(body, BoxPlotPart::Box);
            }
            if st.median_width > 0.0 {
                let inset = if st.notched && q1 != q3 {
                    thickness * st.notch_depth
                } else {
                    0.0
                };
                push(
                    stroke(
                        xy(cross + inset, med),
                        xy(cross + thickness - inset, med),
                        st.median_width,
                        st.median_color,
                    ),
                    BoxPlotPart::Median,
                );
            }
            if st.show_mean {
                if let Some(mean) = v.mean {
                    let p = xy(middle, axis(mean));
                    let r = st.mean_size * 0.5;
                    let mut b = Bar {
                        rect: [p[0] - r, p[1] - r, p[0] + r, p[1] + r],
                        color: [
                            st.mean_color.r,
                            st.mean_color.g,
                            st.mean_color.b,
                            st.mean_color.a,
                        ],
                        ..base
                    };
                    b.effects[2] = 2.0;
                    push(b, BoxPlotPart::Mean);
                }
            }
            if st.show_outliers {
                for (index, value) in v.outliers.iter().enumerate() {
                    let p = xy(middle, axis(*value));
                    let r = st.outlier_size * 0.5;
                    push(
                        Bar {
                            rect: [p[0] - r, p[1] - r, p[0] + r, p[1] + r],
                            radii: [if st.outlier_shape == BoxOutlierShape::Circle {
                                r
                            } else {
                                0.0
                            }; 4],
                            color: [
                                st.outlier_color.r,
                                st.outlier_color.g,
                                st.outlier_color.b,
                                if st.outlier_filled {
                                    st.outlier_color.a
                                } else {
                                    0.0
                                },
                            ],
                            outline: [1.0, 1.2f32.min(r), base.outline[2], 0.0],
                            outline_color: [
                                st.outlier_color.r,
                                st.outlier_color.g,
                                st.outlier_color.b,
                                st.outlier_color.a,
                            ],
                            ..base
                        },
                        BoxPlotPart::Outlier(index),
                    );
                }
            }
            if annotate {
                if let Some(label) = c.label_text(&target) {
                    let tw = measure(c, &label, fs);
                    let label_end = v
                        .outliers
                        .iter()
                        .copied()
                        .chain([v.whisker_high])
                        .chain(v.mean)
                        .chain(v.median_ci.into_iter().flatten())
                        .fold(v.whisker_high, f64::max);
                    let end = axis(label_end);
                    let (x, y) = if horizontal {
                        (end + 10.0, middle + fs * 0.32)
                    } else {
                        (middle - tw * 0.5, end - 10.0)
                    };
                    let bounds = [x, y - fs, x + tw, y + fs * 0.2];
                    if x < left
                        || x + tw > right
                        || y - fs < top
                        || y > bottom
                        || text_boxes.iter().any(|q| {
                            bounds[0] < q[2] + 3.0
                                && bounds[2] + 3.0 > q[0]
                                && bounds[1] < q[3] + 2.0
                                && bounds[3] + 2.0 > q[1]
                        })
                    {
                        return Err(invalid(
                            "box labels do not fit; enlarge canvas, extend range, or hide labels",
                        ));
                    }
                    text_boxes.push(bounds);
                    text(&mut front, c, &label, x, y, fs, c.text_color, scale);
                }
            }
        }
    }
    if annotate {
        out.annotations = back.into_rgba();
        out.annotations.extend(front.into_rgba());
    }
    Ok(out)
}
