use super::*;
use crate::decoration::{DecorationMap, DecorationPainter, LabelPlacer};
use crate::raster::Canvas;
use crate::text_render::{FontPolicy, measure_plain_text};
use model::DecorationId;
use model::config::{AxisOptions, AxisScale};

pub(super) struct Layout {
    pub bars: Vec<Bar>,
    pub targets: Vec<BoxPlotPick>,
    pub annotations: Vec<u8>,
    pub decorations: DecorationMap,
    pub clip: [f32; 4],
    /// Anchor space for the annotation layer: the category index axis and
    /// the value axis, oriented by the chart direction.
    pub space: crate::annotation::PixelSpace,
}
fn measure(c: &BoxPlotChart, s: &str, size: f32) -> f32 {
    measure_plain_text(s, &c.font_family, size, false, false, FontPolicy::Standard).width
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
        let label = crate::axis_render::plain_number(value);
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
    let (rows, legend_height) = crate::axis_render::plan_bounded_legend(
        c.series
            .iter()
            .filter(|_| c.legend)
            .map(|series| series.label.as_str()),
        w,
        &c.font_family,
        fs,
    )
    .map_err(invalid)?;
    let legend: Vec<_> = rows
        .iter()
        .zip(&c.series)
        .map(|((row, x, width), series)| (*row, *x, *width, series))
        .collect();
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
    let title = crate::bounded_title::TitleLayout::new(c.resolved_title(), scale, false);
    let top = title.top;
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
    let automatic_area = crate::layout::RectF {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    };
    let area = c
        .decorations
        .resolve_data_area(automatic_area)
        .map_err(invalid)?;
    let (left, top, right, bottom) = (area.x, area.y, area.x + area.width, area.y + area.height);
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
    let mut decorations = DecorationPainter::new(&c.decorations, size, scale, annotate);
    decorations.add(DecorationId::DataArea, automatic_area);
    crate::axis_render::draw_bounded_decorations(
        &crate::axis_render::BoundedAxisDecorations {
            size,
            plot: [left, top, right, bottom],
            left_title_x: fs + 4. + left - automatic_area.x,
            horizontal,
            zero: None,
            ticks: &ticks,
            categories: &c.categories,
            title: &title,
            value_title: &c.value_title,
            category_title: &c.category_title,
            font_family: &c.font_family,
            font_size: fs,
            color: c.text_color,
            grid: c.grid,
            legend: legend
                .iter()
                .map(|(row, x, width, s)| (*row, *x, *width, s.label.as_str(), s.color))
                .collect(),
            legend_height,
        },
        &mut decorations,
        &mut back,
        &mut front,
    )
    .map_err(invalid)?;
    let mut out = Layout {
        bars: vec![],
        targets: vec![],
        annotations: vec![],
        decorations: DecorationPainter::new(&c.decorations, size, scale, false).finish(),
        clip: [left * scale, top * scale, right * scale, bottom * scale],
        space: annotation_space(
            (w, h),
            area,
            horizontal,
            c.categories.len(),
            [plan.min * unit, plan.max * unit],
        ),
    };
    let mut labels = LabelPlacer::new(area, [true, true]);
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
            {
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
                    let (x, y) =
                        decorations.place_text(&mut labels, &c.font_family, &label, x, y, fs);
                    decorations
                        .on(
                            DecorationId::DataLabel {
                                category_id: target.category_id.clone(),
                                series_id: target.series_id.clone(),
                            },
                            &mut front,
                        )
                        .plain_text(&c.font_family, &label, x, y, fs, c.text_color);
                }
            }
        }
    }
    out.decorations = decorations.finish();
    if annotate {
        out.decorations.draw_selection(&mut front, scale);
        out.annotations = back.into_rgba();
        out.annotations.extend(front.into_rgba());
    }
    Ok(out)
}

/// The annotation anchor space of a bounded bar or box chart: the value axis
/// is linear over the drawn range, the category axis counts categories (the
/// first category is centred at coordinate 0) and runs top-down when the
/// bars are horizontal.
pub(super) fn annotation_space(
    size: (f32, f32),
    area: crate::layout::RectF,
    horizontal: bool,
    categories: usize,
    value_range: [f64; 2],
) -> crate::annotation::PixelSpace {
    use crate::annotation::{AxisMap, AxisSpace, PixelSpace};
    let value = AxisSpace {
        map: AxisMap::Linear {
            min: value_range[0],
            max: value_range[1],
        },
        inverted: false,
    };
    let category = |inverted: bool| AxisSpace {
        map: AxisMap::Index { count: categories },
        inverted,
    };
    let (x, y) = if horizontal {
        (value, category(true))
    } else {
        (category(false), value)
    };
    PixelSpace {
        panel: crate::layout::RectF {
            x: 0.,
            y: 0.,
            width: size.0,
            height: size.1,
        },
        plot: area,
        x,
        y,
    }
}

#[cfg(test)]
mod title_tests {
    use super::*;
    use model::{Category, Color, DecorationId};
    fn bounds(map: &DecorationMap, title: bool) -> Option<crate::layout::RectF> {
        map.entries()
            .find(|(_, e)| {
                if title {
                    e.id == DecorationId::ChartTitle
                } else {
                    matches!(e.id, DecorationId::AxisLabel(crate::layout::Side::Bottom))
                }
            })
            .and_then(|(id, _)| map.bounds(id))
    }
    fn crop(bytes: &[u8], r: crate::layout::RectF, scale: f32) -> Vec<u8> {
        let width = (800. * scale) as usize;
        let height = (520. * scale) as usize;
        let front = &bytes[bytes.len() - width * height * 4..];
        let x0 = (r.x * scale).floor().max(0.) as usize;
        let x1 = ((r.x + r.width) * scale).ceil().min(width as f32) as usize;
        let y0 = (r.y * scale).floor().max(0.) as usize;
        let y1 = ((r.y + r.height) * scale).ceil().min(height as f32) as usize;
        (y0..y1)
            .flat_map(|y| {
                front[(y * width + x0) * 4..(y * width + x1) * 4]
                    .iter()
                    .copied()
            })
            .collect()
    }
    #[test]
    fn title_and_label_styles_render_independently() {
        let mut c = BoxPlotChart {
            categories: vec![Category::new("a", "A"), Category::new("b", "B")],
            series: vec![BoxPlotSeries::new(
                "s",
                "Series",
                vec![
                    Some(BoxSummary::new(10., 20., 30., 40., 50.)),
                    Some(BoxSummary::new(20., 30., 40., 50., 60.)),
                ],
                Color::BLACK,
            )],
            ..Default::default()
        };
        c.title = "Independent title".into();
        for scale in [1., 2.] {
            let legacy = layout(&c, (800, 520), scale, true).unwrap();
            c.normalize_title();
            let original = layout(&c, (800, 520), scale, true).unwrap();
            assert_eq!(
                legacy.annotations, original.annotations,
                "normalization must preserve appearance"
            );
            let tb = bounds(&original.decorations, true).unwrap();
            let lb = bounds(&original.decorations, false).unwrap();
            let title_ink = crop(&original.annotations, tb, scale);
            assert!(title_ink.chunks_exact(4).any(|p| p[3] != 0));
            let saved = c.clone();
            c.font_size *= 1.25;
            c.text_color = Color::new(0.8, 0.1, 0.1, 1.);
            let labels = layout(&c, (800, 520), scale, true).unwrap();
            assert_eq!(bounds(&labels.decorations, true), Some(tb));
            assert_eq!(
                crop(&labels.annotations, tb, scale),
                title_ink,
                "label edits changed title pixels"
            );
            assert!(bounds(&labels.decorations, false).unwrap().height > lb.height);
            c = saved.clone();
            c.chart_title.as_mut().unwrap().text.font_size += 6.;
            let sized = layout(&c, (800, 520), scale, true).unwrap();
            assert!(bounds(&sized.decorations, true).unwrap().height > tb.height);
            let label = bounds(&sized.decorations, false).unwrap();
            assert!(
                (label.height - lb.height).abs() < 0.01 && (label.width - lb.width).abs() < 0.01,
                "title edit resized label glyphs"
            );
            c = saved.clone();
            c.chart_title.as_mut().unwrap().text.color = Color::new(0.1, 0.7, 0.1, 1.);
            let colored = layout(&c, (800, 520), scale, true).unwrap();
            assert_ne!(crop(&colored.annotations, tb, scale), title_ink);
            assert_eq!(
                crop(&colored.annotations, lb, scale),
                crop(&original.annotations, lb, scale),
                "title color changed label pixels"
            );
            c = saved.clone();
            c.chart_title.as_mut().unwrap().text.segments[0].bold = true;
            c.chart_title.as_mut().unwrap().text.segments[1].superscript = true;
            let rich = layout(&c, (800, 520), scale, true).unwrap();
            assert_ne!(
                crop(&rich.annotations, tb, scale),
                title_ink,
                "rich text is actually painted"
            );
            c.chart_title.as_mut().unwrap().visible = false;
            assert!(
                bounds(
                    &layout(&c, (800, 520), scale, true).unwrap().decorations,
                    true
                )
                .is_none()
            );
            c = saved;
        }
    }
}
