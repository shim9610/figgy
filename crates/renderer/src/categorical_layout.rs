use super::*;
use crate::data_render::{FLAT_BODY_ROUGHNESS, MaterialGpu};
use crate::decoration::{DecorationMap, DecorationPainter, LabelPlacer};
use crate::raster::Canvas;
use crate::text_render::{FontPolicy, measure_plain_text};
use model::DecorationId;
use model::config::{AxisOptions, AxisScale};
use model::format::PlainNumberFormat;
use model::tick::LinearGrid;

pub(super) struct Layout {
    pub bars: Vec<Bar>,
    pub targets: Vec<CategoryBarTarget>,
    pub annotations: Vec<u8>,
    pub decorations: DecorationMap,
    /// Anchor space for the annotation layer: the category index axis and
    /// the value axis, oriented by the chart direction.
    pub space: crate::annotation::PixelSpace,
}
fn measure(c: &CategoricalChart, s: &str, size: f32) -> f32 {
    measure_plain_text(s, &c.font_family, size, false, false, FontPolicy::Standard).width
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
    // The axes' grid walk over the widened plan: ticks at `k · step`.
    let grid = LinearGrid::new(plan.min, plan.max, plan.major_spacing, 100)
        .ok_or_else(|| invalid("categorical axis range cannot be represented"))?;
    let mut ticks = Vec::new();
    for (_, normalized) in grid.iter() {
        let value = normalized * unit;
        if !value.is_finite() {
            return Err(invalid("categorical axis tick overflow"));
        }
        let label = if percent {
            format!("{}%", PlainNumberFormat::AUTO.format(value))
        } else {
            PlainNumberFormat::AUTO.format(value)
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
        (value_width + 18.0).max(32.0)
    } else {
        24.0
    };
    let title = crate::bounded_title::TitleLayout::new(c.resolved_title(), scale);
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
            "categorical labels leave too little plot space; enlarge canvas",
        ));
    }
    let slot = if horizontal {
        (bottom - top) / c.categories.len() as f32
    } else {
        (right - left) / c.categories.len() as f32
    };
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
    let mut decorations = DecorationPainter::new(&c.decorations, size, scale, annotate);
    decorations.add(DecorationId::DataArea, automatic_area);
    crate::axis_render::draw_bounded_decorations(
        &crate::axis_render::BoundedAxisDecorations {
            size,
            plot: [left, top, right, bottom],
            left_title_x: fs + 4. + left - automatic_area.x,
            horizontal,
            zero: Some(axis(0.0)),
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
        bars: Vec::new(),
        targets: Vec::new(),
        annotations: Vec::new(),
        decorations: DecorationPainter::new(&c.decorations, size, scale, false).finish(),
        space: annotation_space(
            (w, h),
            area,
            horizontal,
            c.categories.len(),
            [plan.min * unit, plan.max * unit],
        ),
    };
    let mut labels = LabelPlacer::new(
        crate::layout::RectF {
            x: 4.,
            y: 4.,
            width: w - 8.,
            height: h - 8.,
        },
        [true, true],
    );
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
                material: MaterialGpu::pack(
                    style.material,
                    FLAT_BODY_ROUGHNESS,
                    style.texture_strength,
                    style.texture_scale,
                    style.gloss,
                    (seed % 4096) as f32,
                    0.0,
                    if emphasized {
                        style.emphasis_brightness
                    } else {
                        0.0
                    },
                ),
                outline: [
                    if style.outline { 1.0 } else { 0.0 },
                    style.outline_width * scale,
                    if horizontal { 1.0 } else { 0.0 },
                    0.0,
                ],
                outline_color: [
                    style.outline_color.r,
                    style.outline_color.g,
                    style.outline_color.b,
                    style.outline_color.a,
                ],
                effects: [0.0, scale, shared as f32, 0.0],
            });
            out.targets.push(s.target.clone());
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
        // Auto may omit values that cannot fit inside their own stack segment.
        // Explicit labels remain available through the decoration hit map.
        if mode == CategoryBarLabels::Auto
            && inside
            && (tw + 8.0 > rect[2] - rect[0] || fs * 1.4 > rect[3] - rect[1])
        {
            continue;
        }
        let (x, y) = decorations.place_text(&mut labels, &c.font_family, &label, x, y, fs);
        let on_bar = inside
            && x >= rect[0]
            && x + tw <= rect[2]
            && y - fs >= rect[1]
            && y + fs * 0.2 <= rect[3];
        let ink = if on_bar && color.r * 0.2126 + color.g * 0.7152 + color.b * 0.0722 < 0.55 {
            crate::Color::WHITE
        } else {
            c.text_color
        };
        decorations
            .on(
                DecorationId::DataLabel {
                    category_id: s.target.category_id.clone(),
                    series_id: s.target.series_id.clone(),
                },
                &mut front,
            )
            .plain_text(&c.font_family, &label, x, y, fs, ink);
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
    use model::{Color, DecorationId};
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
        let mut c = CategoricalChart {
            categories: vec![Category::new("a", "A"), Category::new("b", "B")],
            series: vec![BarSeries::new(
                "s",
                "Series",
                vec![Some(30.), Some(70.)],
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
