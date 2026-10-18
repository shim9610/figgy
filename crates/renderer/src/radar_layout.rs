use super::*;
use crate::decoration::{DecorationMap, DecorationPainter, LabelPlacer};
use crate::layout::{RectF, Side};
use crate::raster::{Canvas, Paint};
use crate::text_render::{FontPolicy, measure_plain_text};
use model::DecorationId;
use model::config::{AxisOptions, AxisScale};

const TAU: f32 = std::f32::consts::TAU;
/// Segments of a circular ring; a polygon ring has one per category.
const CIRCLE_SEGMENTS: usize = 96;

pub(super) struct Layout {
    pub prims: Vec<Prim>,
    pub vertices: Vec<[f32; 2]>,
    pub targets: Vec<RadarPick>,
    pub annotations: Vec<u8>,
    pub decorations: DecorationMap,
    /// The plot area the series are clipped to, physical pixels.
    pub clip: [f32; 4],
    /// The outer ring, physical pixels.
    pub center: [f32; 2],
    pub radius: f32,
    /// Anchor space for the annotation layer: ratio anchors over the square
    /// around the outer ring; there is no separable data axis.
    pub space: crate::annotation::PixelSpace,
}
fn measure(c: &RadarChart, s: &str, size: f32) -> f32 {
    measure_plain_text(s, &c.font_family, size, false, false, FontPolicy::Standard).width
}

/// The value axis every spoke shares: its range and the ring values.
struct ValueAxis {
    min: f64,
    max: f64,
    rings: Vec<f64>,
}
fn value_axis(c: &RadarChart) -> Result<ValueAxis, RadarError> {
    let invalid = RadarError::Invalid;
    let finite = c
        .series
        .iter()
        .flat_map(|s| s.values.iter().copied())
        .filter(|v| v.is_finite());
    let (mut lo, mut hi) = finite.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
        (lo.min(v), hi.max(v))
    });
    if !lo.is_finite() {
        lo = 0.0;
        hi = 1.0;
    }
    // A radar reads from the centre out, so the fitted axis starts at zero
    // unless the data goes below it.
    lo = lo.min(0.0);
    if hi <= lo {
        hi = lo + 1.0;
    }
    let (min, max, spacing) = match (c.value_range, c.ring_spacing) {
        (Some([min, max]), Some(spacing)) => (min, max, spacing),
        (Some([min, max]), None) => {
            let plan = AxisOptions::compute_nice_ticks(AxisScale::Linear, min, max, 5)
                .map_err(|_| invalid("radar value range cannot be represented"))?;
            (min, max, plan.major_spacing)
        }
        (None, spacing) => {
            let plan = AxisOptions::compute_nice_ticks(AxisScale::Linear, lo, hi, 5)
                .map_err(|_| invalid("radar value range cannot be represented"))?;
            let spacing = spacing.unwrap_or(plan.major_spacing);
            // The outer ring is the first ring at or past the data.
            let max = (hi / spacing).ceil() * spacing;
            let min = (lo / spacing).floor() * spacing;
            (min, if max > min { max } else { min + spacing }, spacing)
        }
    };
    let span = max - min;
    if !span.is_finite() || span <= 0.0 || !spacing.is_finite() || spacing <= 0.0 {
        return Err(invalid("radar axis span is not representable"));
    }
    if span / spacing > 100.0 {
        return Err(invalid("too many radar rings; widen ring_spacing"));
    }
    let first = (min / spacing).ceil() as i64;
    let last = (max / spacing).floor() as i64;
    let rings: Vec<f64> = (first..=last)
        .map(|k| k as f64 * spacing)
        .filter(|v| *v > min + span * 1e-9)
        .collect();
    Ok(ValueAxis { min, max, rings })
}

pub(super) fn layout(
    c: &RadarChart,
    size: (u32, u32),
    scale: f32,
    annotate: bool,
) -> Result<Layout, RadarError> {
    let invalid = RadarError::Invalid;
    let (w, h) = (size.0 as f32, size.1 as f32);
    let fs = c.font_size;
    let n = c.categories.len();
    let axis = value_axis(c)?;
    let norm = |v: f64| {
        if v.is_finite() {
            Some((((v - axis.min) / (axis.max - axis.min)).clamp(0.0, 1.0)) as f32)
        } else {
            None
        }
    };
    let direction = if c.style.clockwise { 1.0 } else { -1.0 };
    let angle =
        |i: usize| c.style.start_angle_degrees.to_radians() + direction * i as f32 * TAU / n as f32;

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
        .map(|((row, x, width), series)| (*row, *x, *width, series.label.as_str(), series.color))
        .collect();
    let title = crate::bounded_title::TitleLayout::new(c.resolved_title(), scale, false);
    let automatic_area = RectF {
        x: 16.0,
        y: title.top,
        width: w - 32.0,
        height: h - legend_height - 16.0 - title.top,
    };
    let area = c
        .decorations
        .resolve_data_area(automatic_area)
        .map_err(invalid)?;
    // Category labels sit outside the outer ring: the widest one must fit
    // beside the ring and one line of text above and below it.
    let radius = (area.width * 0.5 - category_width - 14.0)
        .min(area.height * 0.5 - fs - 14.0)
        .floor();
    if radius < 40.0 {
        return Err(invalid(
            "radar labels leave too little room for the rings; enlarge canvas",
        ));
    }
    let center = [area.x + area.width * 0.5, area.y + area.height * 0.5];
    let point = |i: usize, t: f32| {
        let a = angle(i);
        [
            center[0] + radius * t * a.cos(),
            center[1] + radius * t * a.sin(),
        ]
    };

    let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
    let mut back = Canvas::new(if annotate { pw } else { 1 }, if annotate { ph } else { 1 })
        .ok_or(invalid("annotation allocation failed"))?;
    let mut front = Canvas::new(if annotate { pw } else { 1 }, if annotate { ph } else { 1 })
        .ok_or(invalid("annotation allocation failed"))?;
    let mut decorations = DecorationPainter::new(&c.decorations, size, scale, annotate);
    decorations.add(DecorationId::DataArea, automatic_area);
    title
        .draw(&mut decorations, &mut front, w)
        .map_err(invalid)?;

    // Rings and spokes: plain back-raster lines (not draggable), like a
    // Cartesian grid.
    let physical = |p: [f32; 2]| (p[0] * scale, p[1] * scale);
    if c.grid {
        let ring_paint = Paint::stroke(&c.style.ring_color, scale);
        for value in &axis.rings {
            let t = norm(*value).unwrap_or(1.0);
            let points: Vec<(f32, f32)> = match c.style.grid_shape {
                RadarGridShape::Polygon => (0..=n).map(|i| physical(point(i % n, t))).collect(),
                RadarGridShape::Circle => (0..=CIRCLE_SEGMENTS)
                    .map(|k| {
                        let a = k as f32 * TAU / CIRCLE_SEGMENTS as f32;
                        physical([
                            center[0] + radius * t * a.cos(),
                            center[1] + radius * t * a.sin(),
                        ])
                    })
                    .collect(),
            };
            back.draw_polyline(&points, &ring_paint);
        }
        let spoke_paint = Paint::stroke(&c.style.spoke_color, scale);
        for i in 0..n {
            back.draw_line(physical(center), physical(point(i, 1.0)), &spoke_paint);
        }
    }
    // Value labels along the first spoke, beside the rings they name.
    let mut labels = LabelPlacer::new(
        RectF {
            x: 4.0,
            y: 4.0,
            width: w - 8.0,
            height: h - 8.0,
        },
        [true, true],
    );
    {
        let a = angle(0);
        let (nx, ny) = (-a.sin(), a.cos());
        for value in &axis.rings {
            let t = norm(*value).unwrap_or(1.0);
            let p = point(0, t);
            let label = crate::axis_render::plain_number(*value);
            let tw = measure(c, &label, fs);
            let x = p[0] + nx * 6.0 - if nx < 0.0 { tw } else { 0.0 };
            let y = p[1] + ny * 6.0 + fs * 0.35;
            let (x, y) = decorations.place_text(&mut labels, &c.font_family, &label, x, y, fs);
            decorations.text(
                DecorationId::AxisLabel(Side::Left),
                &mut front,
                &c.font_family,
                &label,
                x,
                y,
                fs,
                c.text_color,
            );
        }
    }
    // Category labels outside the outer ring, aligned away from the centre.
    for (i, category) in c.categories.iter().enumerate() {
        let a = angle(i);
        let p = [
            center[0] + (radius + 10.0) * a.cos(),
            center[1] + (radius + 10.0) * a.sin(),
        ];
        let tw = measure(c, &category.label, fs);
        let x = p[0] - tw * (1.0 - a.cos()) * 0.5;
        let y = p[1] + fs * 0.35 + a.sin() * fs * 0.45;
        let (x, y) = decorations.place_text(&mut labels, &c.font_family, &category.label, x, y, fs);
        decorations.text(
            DecorationId::SpokeLabel {
                category_id: category.id.clone(),
            },
            &mut front,
            &c.font_family,
            &category.label,
            x,
            y,
            fs,
            c.text_color,
        );
    }
    crate::axis_render::draw_bounded_legend(
        &crate::axis_render::BoundedLegend {
            size,
            entries: &legend,
            height: legend_height,
            font_family: &c.font_family,
            font_size: fs,
            color: c.text_color,
        },
        &mut decorations,
        &mut back,
        &mut front,
    );

    let clip = [
        area.x * scale,
        area.y * scale,
        (area.x + area.width) * scale,
        (area.y + area.height) * scale,
    ];
    let mut out = Layout {
        prims: vec![],
        vertices: vec![],
        targets: vec![],
        annotations: vec![],
        decorations: DecorationPainter::new(&c.decorations, size, scale, false).finish(),
        clip,
        center: [center[0] * scale, center[1] * scale],
        radius: radius * scale,
        space: annotation_space((w, h), center, radius),
    };
    // Series primitives: every fill first, then every outline, then every
    // marker, so a later series' fill never covers an earlier outline.
    let mut push = |prim: Prim, target: RadarTarget, part: RadarPart| {
        out.prims.push(prim);
        out.targets.push(RadarPick { target, part });
    };
    let positions: Vec<Vec<Option<[f32; 2]>>> = c
        .series
        .iter()
        .map(|s| {
            s.values
                .iter()
                .enumerate()
                .map(|(i, v)| norm(*v).map(|t| point(i, t)))
                .collect()
        })
        .collect();
    let rgba = |color: crate::Color| [color.r, color.g, color.b, color.a];
    let bbox = |points: &[[f32; 2]], pad: f32| {
        let (mut l, mut t, mut r, mut b) = (
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        );
        for p in points {
            l = l.min(p[0]);
            t = t.min(p[1]);
            r = r.max(p[0]);
            b = b.max(p[1]);
        }
        [
            (l - pad) * scale,
            (t - pad) * scale,
            (r + pad) * scale,
            (b + pad) * scale,
        ]
    };
    let emphasis = |series: &RadarSeries| {
        if c.is_emphasized(&series.id) {
            c.style.emphasis_brightness
        } else {
            0.0
        }
    };
    for (series, pts) in c.series.iter().zip(&positions) {
        let (_, fill, _, _) = c.resolved_style(series);
        if fill.a <= 0.0 || pts.iter().any(Option::is_none) {
            continue;
        }
        let polygon: Vec<[f32; 2]> = pts.iter().map(|p| p.unwrap()).collect();
        let offset = out.vertices.len();
        out.vertices
            .extend(polygon.iter().map(|p| [p[0] * scale, p[1] * scale]));
        push(
            Prim {
                kind: [
                    KIND_POLYGON,
                    offset as f32,
                    polygon.len() as f32,
                    emphasis(series),
                ],
                rect: bbox(&polygon, 1.0),
                points: [0.0; 4],
                color: rgba(fill),
                stroke: [0.0; 4],
            },
            RadarTarget::series(&series.id),
            RadarPart::Fill,
        );
    }
    for (series, pts) in c.series.iter().zip(&positions) {
        let (width, _, _, _) = c.resolved_style(series);
        if width <= 0.0 {
            continue;
        }
        for i in 0..n {
            let (Some(a), Some(b)) = (pts[i], pts[(i + 1) % n]) else {
                continue;
            };
            push(
                Prim {
                    kind: [KIND_SEGMENT, 0.0, 0.0, emphasis(series)],
                    rect: bbox(&[a, b], width * 0.5 + 1.0),
                    points: [a[0] * scale, a[1] * scale, b[0] * scale, b[1] * scale],
                    color: rgba(series.color),
                    stroke: [width * 0.5 * scale, 0.0, 0.0, 0.0],
                },
                RadarTarget::series(&series.id),
                RadarPart::Outline,
            );
        }
    }
    for (series, pts) in c.series.iter().zip(&positions) {
        let (_, _, marker, marker_size) = c.resolved_style(series);
        if marker == RadarMarker::None {
            continue;
        }
        for (i, p) in pts.iter().enumerate() {
            let Some(p) = p else {
                continue;
            };
            let target = RadarTarget::vertex(&series.id, &c.categories[i].id);
            let picked =
                c.hovered.as_ref() == Some(&target) || c.selected.as_ref() == Some(&target);
            let r = marker_size * 0.5 + if picked { 2.0 } else { 0.0 };
            push(
                Prim {
                    kind: [
                        if marker == RadarMarker::Square {
                            KIND_SQUARE
                        } else {
                            KIND_CIRCLE
                        },
                        0.0,
                        0.0,
                        emphasis(series),
                    ],
                    rect: bbox(&[*p], r + 1.0),
                    points: [p[0] * scale, p[1] * scale, r * scale, 0.0],
                    color: rgba(series.color),
                    stroke: [0.0; 4],
                },
                target,
                RadarPart::Marker,
            );
        }
    }
    // Data labels at the vertices, pushed outward along the spoke.
    if c.labels != RadarLabels::None {
        let mut data_labels = LabelPlacer::new(area, [true, true]);
        for (series, pts) in c.series.iter().zip(&positions) {
            if c.labels == RadarLabels::Emphasized && !c.is_emphasized(&series.id) {
                continue;
            }
            for (i, p) in pts.iter().enumerate() {
                let Some(p) = p else {
                    continue;
                };
                let label = c.label_text(series.values[i]);
                let a = angle(i);
                let tw = measure(c, &label, fs);
                let x = p[0] + 9.0 * a.cos() - tw * (1.0 - a.cos()) * 0.5;
                let y = p[1] + 9.0 * a.sin() + fs * 0.35;
                let (x, y) =
                    decorations.place_text(&mut data_labels, &c.font_family, &label, x, y, fs);
                decorations.text(
                    DecorationId::DataLabel {
                        category_id: c.categories[i].id.clone(),
                        series_id: series.id.clone(),
                    },
                    &mut front,
                    &c.font_family,
                    &label,
                    x,
                    y,
                    fs,
                    c.text_color,
                );
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

/// Ratio anchors over the square around the outer ring; a data anchor has no
/// separable axis to land on and is reported as a skip.
pub(super) fn annotation_space(
    size: (f32, f32),
    center: [f32; 2],
    radius: f32,
) -> crate::annotation::PixelSpace {
    use crate::annotation::{AxisSpace, PixelSpace};
    PixelSpace {
        panel: RectF {
            x: 0.,
            y: 0.,
            width: size.0,
            height: size.1,
        },
        plot: RectF {
            x: center[0] - radius,
            y: center[1] - radius,
            width: radius * 2.0,
            height: radius * 2.0,
        },
        x: AxisSpace::NONE,
        y: AxisSpace::NONE,
    }
}

#[cfg(test)]
mod axis_tests {
    use super::*;
    use model::{Category, Color};
    fn chart(values: Vec<f64>) -> RadarChart {
        RadarChart {
            categories: (0..values.len())
                .map(|i| Category::new(i.to_string(), format!("C{i}")))
                .collect(),
            series: vec![RadarSeries::new("s", "S", values, Color::BLACK)],
            ..Default::default()
        }
    }
    #[test]
    fn fitted_axis_starts_at_zero_and_ends_on_a_ring() {
        let axis = value_axis(&chart(vec![1.0, 7.5, 3.0])).unwrap();
        assert_eq!(axis.min, 0.0);
        assert!(axis.max >= 7.5);
        assert_eq!(*axis.rings.last().unwrap(), axis.max);
        assert!(axis.rings.iter().all(|r| *r > 0.0));
    }
    #[test]
    fn explicit_range_and_spacing_are_used_as_given() {
        let mut c = chart(vec![1.0, 7.5, 3.0]);
        c.value_range = Some([0.0, 10.0]);
        c.ring_spacing = Some(2.5);
        let axis = value_axis(&c).unwrap();
        assert_eq!((axis.min, axis.max), (0.0, 10.0));
        assert_eq!(axis.rings, vec![2.5, 5.0, 7.5, 10.0]);
        c.ring_spacing = Some(0.01);
        assert!(value_axis(&c).is_err(), "a thousand rings is refused");
    }
    #[test]
    fn negative_values_extend_the_axis_below_zero() {
        let axis = value_axis(&chart(vec![-2.0, 4.0, 1.0])).unwrap();
        assert!(axis.min <= -2.0);
        assert!(axis.max >= 4.0);
    }
}
