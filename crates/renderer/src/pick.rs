use crate::config::Config;
use crate::data_config::{
    DataLineStyleConfig, DataRenderType, DataScatterStyleConfig, SeriesConfig,
};
use crate::data_render::{self, ColumnId};

pub trait PointColumnLookup {
    fn get_f32_column(&self, id: &ColumnId) -> Option<&[f32]>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointPickOptions {
    pub max_distance_px: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickedPoint {
    pub source_id: Option<String>,
    pub series_id: String,
    pub point_index: usize,
    pub distance_px: f32,
}

/// One exact data-primitive hit.
///
/// The result contains only source identity and stable indices. Data values,
/// coordinates, bar bounds, and reconstructed contour endpoints remain in the
/// registered columns / chart configuration and are never mirrored onto the
/// CPU by the GPU picker.
#[derive(Debug, Clone, PartialEq)]
pub enum PickedData {
    Point {
        source_id: Option<String>,
        series_id: String,
        point_index: usize,
        distance_px: f32,
    },
    HistogramBin {
        source_id: Option<String>,
        series_id: String,
        bin_index: usize,
        distance_px: f32,
    },
    MatrixCell {
        source_id: Option<String>,
        series_id: String,
        x_index: usize,
        y_index: usize,
        distance_px: f32,
    },
    ContourLevel {
        source_id: Option<String>,
        series_id: String,
        level_index: usize,
        x_index: usize,
        y_index: usize,
        distance_px: f32,
    },
}

impl PickedData {
    pub fn source_id(&self) -> Option<&str> {
        match self {
            Self::Point { source_id, .. }
            | Self::HistogramBin { source_id, .. }
            | Self::MatrixCell { source_id, .. }
            | Self::ContourLevel { source_id, .. } => source_id.as_deref(),
        }
    }

    pub fn series_id(&self) -> &str {
        match self {
            Self::Point { series_id, .. }
            | Self::HistogramBin { series_id, .. }
            | Self::MatrixCell { series_id, .. }
            | Self::ContourLevel { series_id, .. } => series_id,
        }
    }

    pub fn distance_px(&self) -> f32 {
        match self {
            Self::Point { distance_px, .. }
            | Self::HistogramBin { distance_px, .. }
            | Self::MatrixCell { distance_px, .. }
            | Self::ContourLevel { distance_px, .. } => *distance_px,
        }
    }

    pub fn to_ref(&self) -> crate::config::PickedDataRef {
        match self {
            Self::Point {
                source_id,
                series_id,
                point_index,
                ..
            } => crate::config::PickedDataRef::Point {
                source_id: source_id.clone(),
                series_id: series_id.clone(),
                point_index: *point_index,
            },
            Self::HistogramBin {
                source_id,
                series_id,
                bin_index,
                ..
            } => crate::config::PickedDataRef::HistogramBin {
                source_id: source_id.clone(),
                series_id: series_id.clone(),
                bin_index: *bin_index,
            },
            Self::MatrixCell {
                source_id,
                series_id,
                x_index,
                y_index,
                ..
            } => crate::config::PickedDataRef::MatrixCell {
                source_id: source_id.clone(),
                series_id: series_id.clone(),
                x_index: *x_index,
                y_index: *y_index,
            },
            Self::ContourLevel {
                source_id,
                series_id,
                level_index,
                x_index,
                y_index,
                ..
            } => crate::config::PickedDataRef::ContourLevel {
                source_id: source_id.clone(),
                series_id: series_id.clone(),
                level_index: *level_index,
                x_index: *x_index,
                y_index: *y_index,
            },
        }
    }

    pub fn into_point(self) -> Option<PickedPoint> {
        let Self::Point {
            source_id,
            series_id,
            point_index,
            distance_px,
        } = self
        else {
            return None;
        };
        Some(PickedPoint {
            source_id,
            series_id,
            point_index,
            distance_px,
        })
    }
}

impl From<PickedPoint> for PickedData {
    fn from(point: PickedPoint) -> Self {
        Self::Point {
            source_id: point.source_id,
            series_id: point.series_id,
            point_index: point.point_index,
            distance_px: point.distance_px,
        }
    }
}

pub fn pick_nearest_point<L: PointColumnLookup>(
    config: &Config,
    series: &[SeriesConfig],
    columns: &L,
    canvas_x: f32,
    canvas_y: f32,
    options: PointPickOptions,
) -> Option<PickedPoint> {
    if !canvas_x.is_finite()
        || !canvas_y.is_finite()
        || !options.max_distance_px.is_finite()
        || options.max_distance_px < 0.0
    {
        return None;
    }

    if let Ok(data_area) = config.data_area() {
        let r = data_area.0;
        let x0 = r.x as f32;
        let y0 = r.y as f32;
        let x1 = x0 + r.width as f32;
        let y1 = y0 + r.height as f32;
        if canvas_x < x0 || canvas_x > x1 || canvas_y < y0 || canvas_y > y1 {
            return None;
        }
    }

    let transform = data_render::scatter_transform_from_config(config);
    let drawn = DrawnStyle::of(config, &transform);
    // The polar fragment clip at the cursor: every primitive is cut by it.
    let clip_distance =
        crate::polar_projection::clip_distance(&transform, config, [canvas_x, canvas_y]);
    let max_dist_sq = options.max_distance_px * options.max_distance_px;
    let use_scatter_style_mapping = config.draw_style.is_precise();
    let mut best: Option<BestPick> = None;

    for (series_index, cfg) in series.iter().enumerate() {
        let Some(xs) = columns.get_f32_column(&cfg.x_column) else {
            continue;
        };
        let Some(ys) = columns.get_f32_column(&cfg.y_column) else {
            continue;
        };

        // The styled entries' per-series hash salt (`Style.series_salt`).
        let salt = crate::sketch::fnv1a(&cfg.series_id);
        if let Some(scatter) = extract_scatter(&cfg.render_type) {
            let style_index = if use_scatter_style_mapping {
                scatter
                    .point_style_index_column
                    .as_ref()
                    .and_then(|column| columns.get_f32_column(column))
            } else {
                None
            };
            let size_values = if use_scatter_style_mapping {
                scatter
                    .point_size_map
                    .as_ref()
                    .and_then(|map| columns.get_f32_column(&map.column))
            } else {
                None
            };
            for i in 0..xs.len().min(ys.len()) {
                let Some((radius_px, shape_id)) = scatter_marker(
                    scatter,
                    style_index,
                    size_values,
                    i,
                    use_scatter_style_mapping,
                ) else {
                    continue;
                };
                pick_anchor(
                    &mut best,
                    series_index,
                    cfg,
                    config,
                    &transform,
                    i,
                    (xs[i], ys[i]),
                    &|center| {
                        styled_marker_distance(
                            drawn,
                            (shape_id, radius_px),
                            center,
                            [canvas_x, canvas_y],
                            (i as u32, salt),
                        )
                        .max(clip_distance)
                    },
                    max_dist_sq,
                );
            }
        }

        // A point vector is picked on the arrow it draws from each anchor:
        // the shaft and head the vector shader paints. It draws only in
        // precise mode, so it is picked only there.
        if let Some(style) = extract_vector_points(&cfg.render_type)
            && use_scatter_style_mapping
        {
            let DataRenderType::Vector {
                components: crate::data_config::VectorComponents::Points { u, v },
                ..
            } = &cfg.render_type
            else {
                unreachable!("extract_vector_points matched the points form");
            };
            let us = columns.get_f32_column(u).unwrap_or(&[]);
            let vs = columns.get_f32_column(v).unwrap_or(&[]);
            // The vector shader draws as many arrows as its shortest column.
            let colors = match style.color_map.as_ref() {
                Some(crate::data_config::VectorColorMap::Column(column)) => {
                    columns.get_f32_column(column).map_or(0, <[f32]>::len)
                }
                _ => usize::MAX,
            };
            let count = xs
                .len()
                .min(ys.len())
                .min(us.len())
                .min(vs.len())
                .min(colors);
            for i in 0..count {
                let Some(hit_dist) = vector_arrow_distance(
                    config,
                    &transform,
                    style,
                    [xs[i], ys[i], us[i], vs[i]],
                    [canvas_x, canvas_y],
                ) else {
                    continue;
                };
                let dist_sq = hit_dist * hit_dist;
                if !dist_sq.is_finite() || dist_sq > max_dist_sq {
                    continue;
                }
                maybe_replace_best(
                    &mut best,
                    series_index,
                    PrimitiveKind::Marker,
                    dist_sq,
                    PickedPoint {
                        source_id: cfg.source_id.clone(),
                        series_id: cfg.series_id.clone(),
                        point_index: i,
                        distance_px: hit_dist,
                    },
                );
            }
        }

        let Some(line) = extract_line(&cfg.render_type) else {
            continue;
        };
        let stroke = LineStroke {
            // The line shader's `max(line_width_px, 1) / 2`.
            half_width: line.line_width.max(1.0) * 0.5,
            // A Milkyway ribbon is never dashed.
            dash: if drawn.kind == DrawnKind::Milkyway {
                &[]
            } else {
                line.line_style.pattern()
            },
            style: drawn,
            salt,
        };
        // The cumulative arc length at the segment's first point, as the
        // GPU's `line_arc.wgsl` scan sums it: the pixel length of every
        // drawable segment so far, a gap contributing nothing.
        let mut arc = 0.0f32;
        for i in 0..xs.len().min(ys.len()).saturating_sub(1) {
            let ax = xs[i];
            let ay = ys[i];
            let bx = xs[i + 1];
            let by = ys[i + 1];
            if !ax.is_finite() || !ay.is_finite() || !bx.is_finite() || !by.is_finite() {
                continue;
            }
            // Both endpoints must be placeable before any piece is tested —
            // the same gate the GPU picker applies to the segment.
            if project_data_to_canvas_px(config, &transform, ax, ay).is_none()
                || project_data_to_canvas_px(config, &transform, bx, by).is_none()
            {
                continue;
            }

            // A step mode walks the same axis-aligned pieces the line shader
            // draws; a linear segment is its single piece.
            let mut pieces = [crate::line_step::StepPiece {
                a: [ax, ay],
                b: [bx, by],
            }; crate::line_step::MAX_LINE_PIECES];
            let piece_count = crate::line_step::line_pieces(
                [ax, ay],
                [bx, by],
                crate::line_step::mode_word_for(line.interpolation, &transform),
                &mut pieces,
            );
            let mut pieces_px = [([0.0f32; 2], [0.0f32; 2]); crate::line_step::MAX_LINE_PIECES];
            let mut projected = true;
            for (piece, out) in pieces[..piece_count].iter().zip(pieces_px.iter_mut()) {
                let Some(a) = project_data_to_canvas_px(config, &transform, piece.a[0], piece.a[1])
                else {
                    projected = false;
                    break;
                };
                let Some(b) = project_data_to_canvas_px(config, &transform, piece.b[0], piece.b[1])
                else {
                    projected = false;
                    break;
                };
                *out = ([a.0, a.1], [b.0, b.1]);
            }
            if !projected {
                continue;
            }
            let segment_px: f32 = pieces_px[..piece_count]
                .iter()
                .map(|(a, b)| (b[0] - a[0]).hypot(b[1] - a[1]))
                .sum();
            let arc_a = arc;
            if segment_px.is_finite() && segment_px < 1e30 {
                arc += segment_px;
            }
            let Some((dist, point_index)) = line_segment_pick(
                [canvas_x, canvas_y],
                &pieces_px[..piece_count],
                stroke,
                (arc_a, arc),
                i,
            ) else {
                continue;
            };
            let dist = dist.max(clip_distance);
            let dist_sq = dist * dist;
            if !dist_sq.is_finite() || dist_sq > max_dist_sq {
                continue;
            }

            maybe_replace_best(
                &mut best,
                series_index,
                PrimitiveKind::Line,
                dist_sq,
                PickedPoint {
                    source_id: cfg.source_id.clone(),
                    series_id: cfg.series_id.clone(),
                    point_index,
                    distance_px: dist,
                },
            );
        }
    }

    best.map(|(_, _, _, p)| p)
}

/// Score one anchor (a marker or a vector's tail) of `cfg` against the cursor
/// and keep it when it is the best so far: the data point `(x, y)` is
/// projected to the canvas, `hit` measures the cursor's distance from what
/// is drawn around that centre, and anything past `max_dist_sq` is dropped.
/// A non-finite or unprojectable anchor is skipped.
#[allow(clippy::too_many_arguments)]
fn pick_anchor(
    best: &mut Option<BestPick>,
    series_index: usize,
    cfg: &SeriesConfig,
    config: &Config,
    transform: &data_render::ScatterTransform,
    index: usize,
    (x, y): (f32, f32),
    hit: &dyn Fn([f32; 2]) -> f32,
    max_dist_sq: f32,
) {
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    let Some((px, py)) = project_data_to_canvas_px(config, transform, x, y) else {
        return;
    };
    let hit_dist = hit([px, py]);
    let dist_sq = hit_dist * hit_dist;
    if !dist_sq.is_finite() || dist_sq > max_dist_sq {
        return;
    }
    maybe_replace_best(
        best,
        series_index,
        PrimitiveKind::Marker,
        dist_sq,
        PickedPoint {
            source_id: cfg.source_id.clone(),
            series_id: cfg.series_id.clone(),
            point_index: index,
            distance_px: hit_dist,
        },
    );
}

/// What a candidate is, for the paint-order tie rule: within one series the
/// markers paint over the line.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PrimitiveKind {
    Marker,
    Line,
}

type BestPick = (usize, PrimitiveKind, f32, PickedPoint);

/// Keep `picked` when it is nearer than the best so far, or exactly as near
/// and painted over it. Series paint in order, a series' markers over its
/// line, and instances in index order; candidates are visited in that same
/// order, so a tie goes to the later visit unless it is a line under an
/// earlier marker of the same series. Twin of `pick_candidate_is_better`.
fn maybe_replace_best(
    best: &mut Option<BestPick>,
    series_index: usize,
    kind: PrimitiveKind,
    dist_sq: f32,
    picked: PickedPoint,
) {
    let replace = match best {
        None => true,
        Some((best_series, best_kind, best_dist_sq, _)) => {
            dist_sq < *best_dist_sq
                || (dist_sq == *best_dist_sq
                    && (series_index > *best_series
                        || (series_index == *best_series && kind <= *best_kind)))
        }
    };
    if replace {
        *best = Some((series_index, kind, dist_sq, picked));
    }
}

/// The pick radius of a vector series' anchor, in logical pixels: half the
/// head width, or half the shaft width when the shaft is wider. The GPU
/// picker and the selection ring use the same number.
pub(crate) fn vector_anchor_radius_px(style: &crate::data_config::VectorStyle) -> f32 {
    (style.head_width_px.max(style.width_px) * 0.5).max(0.0)
}

/// An arrow placed on its anchor (`common/arrow.wgsl::ArrowPlacement`).
#[derive(Clone, Copy, Debug)]
struct ArrowPlacement {
    tail: [f32; 2],
    tip: [f32; 2],
    head_length_px: f32,
    head_width_px: f32,
}

/// Twin of `common/arrow.wgsl::arrow_place`: the pivot shift and the head
/// sized to the arrow; `None` for an arrow that is not drawn.
fn arrow_place(
    tail: [f32; 2],
    tip: [f32; 2],
    style: &crate::data_config::VectorStyle,
) -> Option<ArrowPlacement> {
    let delta = [tip[0] - tail[0], tip[1] - tail[1]];
    let len = delta[0].hypot(delta[1]);
    if !len.is_finite() || len <= 0.0 || len < style.min_length_px {
        return None;
    }
    let shift = match style.pivot {
        crate::data_config::VectorPivot::Tail => 0.0,
        crate::data_config::VectorPivot::Middle => -0.5,
        crate::data_config::VectorPivot::Tip => -1.0,
    };
    let head_length = style.head_length_px.max(0.0).min(len);
    Some(ArrowPlacement {
        tail: [tail[0] + delta[0] * shift, tail[1] + delta[1] * shift],
        tip: [tip[0] + delta[0] * shift, tip[1] + delta[1] * shift],
        head_length_px: head_length,
        head_width_px: if style.head_length_px > 0.0 {
            style.head_width_px.max(0.0) * (head_length / style.head_length_px)
        } else {
            0.0
        },
    })
}

/// Twin of `common/arrow.wgsl::arrow_sd_segment`.
fn arrow_sd_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let pa = [p[0] - a[0], p[1] - a[1]];
    let ba = [b[0] - a[0], b[1] - a[1]];
    let h = ((pa[0] * ba[0] + pa[1] * ba[1]) / (ba[0] * ba[0] + ba[1] * ba[1]).max(1e-12))
        .clamp(0.0, 1.0);
    (pa[0] - ba[0] * h).hypot(pa[1] - ba[1] * h)
}

/// Twin of `common/arrow.wgsl::arrow_sd_triangle` (negative inside).
fn arrow_sd_triangle(p: [f32; 2], p0: [f32; 2], p1: [f32; 2], p2: [f32; 2]) -> f32 {
    let sub = |a: [f32; 2], b: [f32; 2]| [a[0] - b[0], a[1] - b[1]];
    let dot = |a: [f32; 2], b: [f32; 2]| a[0] * b[0] + a[1] * b[1];
    let cross = |a: [f32; 2], b: [f32; 2]| a[0] * b[1] - a[1] * b[0];
    let edges = [sub(p1, p0), sub(p2, p1), sub(p0, p2)];
    let starts = [p0, p1, p2];
    let s = wgsl_sign(cross(edges[0], edges[2]));
    let mut d = (f32::INFINITY, f32::INFINITY);
    for (e, start) in edges.into_iter().zip(starts) {
        let v = sub(p, start);
        let h = (dot(v, e) / dot(e, e).max(1e-12)).clamp(0.0, 1.0);
        let pq = [v[0] - e[0] * h, v[1] - e[1] * h];
        d.0 = d.0.min(dot(pq, pq));
        d.1 = d.1.min(s * cross(v, e));
    }
    -d.0.sqrt() * wgsl_sign(d.1)
}

/// Twin of `common/arrow.wgsl::arrow_distance`: signed distance from `p` to
/// the shaft and head of the placed arrow.
fn arrow_distance(p: [f32; 2], placed: ArrowPlacement, width_px: f32) -> f32 {
    let delta = [
        placed.tip[0] - placed.tail[0],
        placed.tip[1] - placed.tail[1],
    ];
    let len = delta[0].hypot(delta[1]);
    let dir = [delta[0] / len.max(1e-12), delta[1] / len.max(1e-12)];
    let nrm = [-dir[1], dir[0]];
    let head_base = [
        placed.tip[0] - dir[0] * placed.head_length_px,
        placed.tip[1] - dir[1] * placed.head_length_px,
    ];
    let mut distance = 1e20f32;
    // `ARROW_SHAFT_MIN_PX`: a head as long as the arrow leaves no shaft.
    if len - placed.head_length_px > 0.01 && width_px > 0.0 {
        distance = distance.min(arrow_sd_segment(p, placed.tail, head_base) - width_px * 0.5);
    }
    if placed.head_length_px > 0.0 && placed.head_width_px > 0.0 {
        let half = [
            nrm[0] * placed.head_width_px * 0.5,
            nrm[1] * placed.head_width_px * 0.5,
        ];
        distance = distance.min(arrow_sd_triangle(
            p,
            placed.tip,
            [head_base[0] + half[0], head_base[1] + half[1]],
            [head_base[0] - half[0], head_base[1] - half[1]],
        ));
    }
    distance
}

/// Twin of `project_cartesian2d.wgsl::axis_px_per_unit`: pixels per data
/// unit along `axis` at `value`.
fn axis_px_per_unit(transform: &data_render::ScatterTransform, axis: usize, value: f32) -> f32 {
    let range = (transform.data_max[axis] - transform.data_min[axis])
        + (transform.data_max_lo[axis] - transform.data_min_lo[axis]);
    let linear = 2.0 * transform.data_to_panel_scale[axis] / (range * transform.pixel_to_ndc[axis]);
    let log_slope = linear / (value * std::f32::consts::LN_10);
    if transform.scale_log[axis] > 0.5 {
        log_slope
    } else {
        linear
    }
}

/// The pick distance from `cursor` to the arrow the vector shader draws for
/// the sample `[x, y, u, v]` (`vector_sample`, `project_arrow`, `arrow_place`
/// and `arrow_distance`), or `None` where no arrow is drawn. Canvas pixels
/// run y-down here and y-up in the shader; the mirror keeps distances.
fn vector_arrow_distance(
    config: &Config,
    transform: &data_render::ScatterTransform,
    style: &crate::data_config::VectorStyle,
    [x, y, u, v]: [f32; 4],
    cursor: [f32; 2],
) -> Option<f32> {
    if !x.is_finite() || !y.is_finite() || !u.is_finite() || !v.is_finite() {
        return None;
    }
    let (tx, ty) = project_data_to_canvas_px(config, transform, x, y)?;
    let tail = [tx, ty];
    let tip = match style.scale {
        crate::data_config::VectorScale::Pixels { px_per_unit } => {
            // y-up direction, flipped into the canvas.
            let direction = [
                axis_px_per_unit(transform, 0, x) * u,
                -(axis_px_per_unit(transform, 1, y) * v),
            ];
            let direction_len = direction[0].hypot(direction[1]);
            let len_px = px_per_unit as f32 * u.hypot(v);
            if !direction_len.is_finite() || !len_px.is_finite() {
                return None;
            }
            if direction_len > 0.0 && len_px != 0.0 {
                let k = len_px / direction_len;
                [tail[0] + direction[0] * k, tail[1] + direction[1] * k]
            } else {
                tail
            }
        }
        crate::data_config::VectorScale::Data { factor } => {
            let factor = factor as f32;
            let (px, py) =
                project_data_to_canvas_px(config, transform, x + factor * u, y + factor * v)?;
            [px, py]
        }
    };
    if !tip[0].is_finite() || !tip[1].is_finite() {
        return None;
    }
    let placed = arrow_place(tail, tip, style)?;
    Some(arrow_distance(cursor, placed, style.width_px).max(0.0))
}

/// The style of a point vector series: its anchors are picked like markers.
/// A grid vector's cells are a separate query, like a heatmap's.
fn extract_vector_points(rt: &DataRenderType) -> Option<&crate::data_config::VectorStyle> {
    match rt {
        DataRenderType::Vector {
            components: crate::data_config::VectorComponents::Points { .. },
            style,
        } => Some(style),
        _ => None,
    }
}

fn extract_line(rt: &DataRenderType) -> Option<&DataLineStyleConfig> {
    match rt {
        DataRenderType::Line { line }
        | DataRenderType::ScatterLine { line, .. }
        | DataRenderType::LineScatterErrorbarX { line, .. }
        | DataRenderType::LineScatterErrorbarY { line, .. }
        | DataRenderType::LineScatterErrorbarXY { line, .. } => Some(line),
        // Exhaustive on purpose: a new render type must fail to compile here so
        // somebody decides whether picking should hit-test a line for it.
        //
        // The four field / bar types are not point-pickable: a bar has no
        // vertex to snap to, and a matrix' `(x_column, y_column)` are grid
        // coordinates, so a "nearest point" would be a grid intersection the
        // user never plotted. Picking a cell or a level is a separate query.
        DataRenderType::Scatter { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        // A vector series is picked at its anchors through `extract_vector`.
        | DataRenderType::Vector { .. } => None,
    }
}

fn extract_scatter(rt: &DataRenderType) -> Option<&DataScatterStyleConfig> {
    match rt {
        DataRenderType::Scatter { scatter }
        | DataRenderType::ScatterLine { scatter, .. }
        | DataRenderType::ScatterErrorbarX { scatter, .. }
        | DataRenderType::ScatterErrorbarY { scatter, .. }
        | DataRenderType::ScatterErrorbarXY { scatter, .. }
        | DataRenderType::LineScatterErrorbarX { scatter, .. }
        | DataRenderType::LineScatterErrorbarY { scatter, .. }
        | DataRenderType::LineScatterErrorbarXY { scatter, .. } => Some(scatter),
        DataRenderType::Line { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. } => None,
    }
}

/// One point's marker as the scatter shader resolves it: the reference
/// radius in logical pixels and the shape code, or `None` for a point the
/// size map hides.
///
/// Precedence per channel is the shader's: sparse override > size map >
/// style table slot > series base. A size map resolves the radius from the
/// point's value column (`size_values`, the same column the GPU draws from);
/// a non-finite value hides the point unless an override sets its radius.
fn scatter_marker(
    scatter: &DataScatterStyleConfig,
    style_index: Option<&[f32]>,
    size_values: Option<&[f32]>,
    point_index: usize,
    use_style_mapping: bool,
) -> Option<(f32, u32)> {
    let mut radius = scatter.point_size;
    let mut shape = &scatter.point_shape;
    let mut hidden = false;

    if use_style_mapping {
        if let (Some(indices), Some(table)) = (style_index, scatter.point_style_table.as_deref())
            && let Some(idx) = indices
                .get(point_index)
                .copied()
                .and_then(valid_style_index)
                .filter(|idx| *idx < table.len())
        {
            let slot = &table[idx];
            if let Some(size) = slot.point_size {
                radius = size;
            }
            if let Some(slot_shape) = slot.point_shape.as_ref() {
                shape = slot_shape;
            }
        }

        if let Some(map) = scatter.point_size_map.as_ref() {
            match size_values
                .and_then(|values| values.get(point_index))
                .and_then(|value| map.radius_px(f64::from(*value)))
            {
                Some(mapped) => radius = mapped,
                None => hidden = true,
            }
        }

        if let Some(overrides) = scatter.point_style_overrides.as_deref() {
            for ov in overrides {
                if ov.index == point_index {
                    if let Some(size) = ov.style.point_size {
                        radius = size;
                        hidden = false;
                    }
                    if let Some(ov_shape) = ov.style.point_shape.as_ref() {
                        shape = ov_shape;
                    }
                }
            }
        }
    }

    if hidden || radius.is_nan() {
        return None;
    }
    Some((radius.max(0.0), data_render::shape_id(shape)))
}

fn valid_style_index(v: f32) -> Option<usize> {
    if (0.0..=16_777_216.0).contains(&v) && (v - v.round()).abs() <= 0.001 {
        Some(v.round() as usize)
    } else {
        None
    }
}

// ───── CPU twin of the shared marker shape (SHADER_COMMON.md §5.5) ─────
//
// A line-for-line port of the WGSL functions the scatter shader draws with
// and the pick shader picks with, in f32 like the GPU. `wgsl_sign` keeps
// WGSL's `sign(0) == 0`, which `f32::signum` does not.

/// `QUAD_MARGIN_PX`: nothing a marker draws lies farther than its visual
/// radius plus this margin from its centre on either axis.
pub(crate) const MARKER_QUAD_MARGIN_PX: f32 = 2.0;
/// `STROKE_HALF_PX`: half-width of the outline stroke.
pub(crate) const MARKER_STROKE_HALF_PX: f32 = 0.75;
const MARKER_PI: f32 = 3.141_592_7;
const MARKER_HALF_PI: f32 = 1.570_796_4;
const STAR_INNER_RATIO: f32 = 0.5;

fn wgsl_sign(v: f32) -> f32 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        0.0
    }
}

fn length2(p: [f32; 2]) -> f32 {
    (p[0] * p[0] + p[1] * p[1]).sqrt()
}

/// Twin of `shape_is_filled`.
pub(crate) fn marker_shape_is_filled(shape_id: u32) -> bool {
    (5..=8).contains(&shape_id) || shape_id >= 17
}

/// Twin of `base_shape_id`.
pub(crate) fn marker_base_shape_id(shape_id: u32) -> u32 {
    match shape_id {
        5 => 0,
        6 => 1,
        7 => 2,
        8 => 3,
        9 | 17 => 5,
        10 | 18 => 6,
        11 | 19 => 7,
        12 | 20 => 8,
        13 | 22 => 9,
        14 | 23 => 10,
        15 | 24 => 11,
        16 | 25 => 12,
        21 => 4,
        other => other,
    }
}

/// The factor `visual_shape_radius` applies to the reference radius.
pub(crate) fn marker_visual_scale(base: u32) -> f32 {
    match base {
        1 => 0.886_226_95,
        2 | 5 | 6 | 7 => 1.555_120_3,
        3 => 1.253_314_1,
        9 => 1.149_139_9,
        10 => 1.099_636_1,
        11 => 1.053_907_4,
        12 => 1.462_850_3,
        _ => 1.0,
    }
}

/// Twin of `visual_shape_radius`.
pub(crate) fn marker_visual_shape_radius(base: u32, r: f32) -> f32 {
    r * marker_visual_scale(base)
}

fn rotate2(p: [f32; 2], a: f32) -> [f32; 2] {
    let c = a.cos();
    let s = a.sin();
    [c * p[0] - s * p[1], s * p[0] + c * p[1]]
}

fn sd_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let pa = [p[0] - a[0], p[1] - a[1]];
    let ba = [b[0] - a[0], b[1] - a[1]];
    let h = ((pa[0] * ba[0] + pa[1] * ba[1]) / (ba[0] * ba[0] + ba[1] * ba[1]).max(1e-12))
        .clamp(0.0, 1.0);
    length2([pa[0] - ba[0] * h, pa[1] - ba[1] * h])
}

fn sd_box(p: [f32; 2], b: [f32; 2]) -> f32 {
    let q = [p[0].abs() - b[0], p[1].abs() - b[1]];
    length2([q[0].max(0.0), q[1].max(0.0)]) + q[0].max(q[1]).min(0.0)
}

fn regular_vertex(i: u32, n: u32, r: f32, rot: f32) -> [f32; 2] {
    let a = rot + i as f32 * (2.0 * MARKER_PI / n as f32);
    [a.cos() * r, a.sin() * r]
}

fn star_vertex(i: u32, r: f32) -> [f32; 2] {
    let outer = i % 2 == 0;
    let rr = if outer { r } else { r * STAR_INNER_RATIO };
    let a = MARKER_HALF_PI + i as f32 * MARKER_PI / 5.0;
    [a.cos() * rr, a.sin() * rr]
}

fn sd_closed_polygon(p: [f32; 2], n: u32, vertex: impl Fn(u32) -> [f32; 2]) -> f32 {
    let mut min_d = 1e20f32;
    let mut inside = false;
    let mut prev = vertex(n - 1);
    for i in 0..n {
        let curr = vertex(i);
        min_d = min_d.min(sd_segment(p, prev, curr));
        if (curr[1] > p[1]) != (prev[1] > p[1]) {
            let x_at_y = (prev[0] - curr[0]) * (p[1] - curr[1]) / (prev[1] - curr[1]) + curr[0];
            if p[0] < x_at_y {
                inside = !inside;
            }
        }
        prev = curr;
    }
    if inside { -min_d } else { min_d }
}

fn sd_plus_filled(p: [f32; 2], r: f32) -> f32 {
    let arm = (r * 0.32).max(MARKER_STROKE_HALF_PX);
    let h = sd_box(p, [r, arm]);
    let v = sd_box(p, [arm, r]);
    h.min(v)
}

fn sd_triangle(p_in: [f32; 2], r: f32) -> f32 {
    let k = 3.0f32.sqrt();
    let half_w = 0.5 * k * r;
    let mut p = [p_in[0].abs() - half_w, p_in[1] + half_w / k];
    if p[0] + k * p[1] > 0.0 {
        p = [(p[0] - k * p[1]) * 0.5, (-k * p[0] - p[1]) * 0.5];
    }
    p[0] -= p[0].clamp(-2.0 * half_w, 0.0);
    -length2(p) * wgsl_sign(p[1])
}

/// Twin of `shape_distance`: signed pixel distance from `p` (y up) to the
/// contour of base shape `base` with visual radius `r`.
pub(crate) fn marker_shape_distance(base: u32, p: [f32; 2], r: f32, filled: bool) -> f32 {
    match base {
        0 => length2(p) - r,
        1 => p[0].abs().max(p[1].abs()) - r,
        2 => sd_triangle(p, r),
        3 => p[0].abs() + p[1].abs() - r,
        4 => {
            let c = r;
            if filled {
                sd_plus_filled(rotate2(p, MARKER_PI * 0.25), c)
            } else {
                let d1 = sd_segment(p, [-c, -c], [c, c]);
                let d2 = sd_segment(p, [-c, c], [c, -c]);
                d1.min(d2)
            }
        }
        5 => sd_triangle([-p[0], -p[1]], r),
        6 => sd_triangle(rotate2(p, -MARKER_HALF_PI), r),
        7 => sd_triangle(rotate2(p, MARKER_HALF_PI), r),
        8 => {
            let c = r;
            if filled {
                sd_plus_filled(p, c)
            } else {
                let d1 = sd_segment(p, [-c, 0.0], [c, 0.0]);
                let d2 = sd_segment(p, [0.0, -c], [0.0, c]);
                d1.min(d2)
            }
        }
        9 => sd_closed_polygon(p, 5, |i| regular_vertex(i, 5, r, MARKER_HALF_PI)),
        10 => sd_closed_polygon(p, 6, |i| regular_vertex(i, 6, r, MARKER_HALF_PI)),
        11 => sd_closed_polygon(p, 8, |i| regular_vertex(i, 8, r, MARKER_HALF_PI)),
        _ => sd_closed_polygon(p, 10, |i| star_vertex(i, r)),
    }
}

/// Twin of `marker_edge`.
pub(crate) fn marker_edge(d: f32, filled: bool) -> f32 {
    if filled {
        d
    } else {
        d.abs() - MARKER_STROKE_HALF_PX
    }
}

// ───── CPU twin of the styled markers and strokes (SHADER_COMMON.md §5.6,
// §5.7) ─────

/// What the chart's draw style draws, for the CPU twin of the pick shader's
/// `styled` modes, with the packed style parameters the draw reads
/// (`StyleVariant::pack_params`).
#[derive(Clone, Copy)]
struct DrawnStyle {
    kind: DrawnKind,
    params: [[f32; 4]; 3],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DrawnKind {
    Precise,
    Sketch,
    Milkyway,
    Constellation,
}

impl DrawnStyle {
    fn of(config: &Config, transform: &data_render::ScatterTransform) -> Self {
        use crate::config::DrawStyle;
        let kind = match &config.draw_style {
            DrawStyle::Precise => DrawnKind::Precise,
            DrawStyle::Sketch(_) => DrawnKind::Sketch,
            DrawStyle::Milkyway(_) => DrawnKind::Milkyway,
            DrawStyle::Constellation(_) => DrawnKind::Constellation,
        };
        Self {
            kind,
            params: transform.style_params,
        }
    }
}

/// `SKETCH_TAU` / `SKETCH_CONTOUR_WOBBLES`.
const SKETCH_TAU: f32 = 6.283_185_5;
const SKETCH_CONTOUR_WOBBLES: f32 = 6.0;
/// `SKETCH_SUBDIV` / `LINE_AA_EXTENT_PX`.
const SKETCH_SUBDIV: u32 = 8;
const LINE_AA_EXTENT_PX: f32 = 0.5;
/// The planet constants of §5.7.
const RING_INNER: f32 = 1.45;
const RING_OUTER: f32 = 2.3;
const RING_INCL: f32 = 0.32;
const PLANET_QUAD_EXTENT: f32 = 2.5;
const PLANET_ALPHA_FLOOR: f32 = 0.003;

/// Twin of `sketch_hash01`.
fn sketch_hash01(i: u32, seed: u32) -> f32 {
    let mut h = i.wrapping_mul(0x9E37_79B9) ^ seed.wrapping_mul(0x85EB_CA6B);
    h = (h ^ (h >> 16)).wrapping_mul(0x045D_9F3B);
    h ^= h >> 16;
    h as f32 / 4_294_967_296.0
}

/// Twin of `sketch_noise`.
fn sketch_noise(t: f32, seed: u32) -> f32 {
    let floor = t.floor();
    let i = (floor as i32) as u32;
    let f = t - floor;
    let u = f * f * (3.0 - 2.0 * f);
    let a = sketch_hash01(i, seed);
    let b = sketch_hash01(i.wrapping_add(1), seed);
    (a * (1.0 - u) + b * u) * 2.0 - 1.0
}

/// Twin of `sketch_line_offset`.
fn sketch_line_offset(arc_px: f32, amplitude_px: f32, wavelength_px: f32, seed: u32) -> f32 {
    if wavelength_px > 0.0 {
        amplitude_px.max(0.0) * sketch_noise(arc_px / wavelength_px.max(1e-6), seed)
    } else {
        0.0
    }
}

/// Twin of `cons_ring_angle`.
fn cons_ring_angle(shape_id: u32) -> f32 {
    match shape_id {
        0 => 0.0,
        1 => 0.52,
        2 => 1.05,
        3 => -0.52,
        4 => -1.05,
        5 => 0.26,
        6 => 0.79,
        7 => -0.26,
        8 => -0.79,
        other => -1.2 + (other % 11) as f32 * 0.24,
    }
}

/// The hit distance of a marker drawn at `center` (canvas pixels, y down)
/// for `cursor` under `style`: the GPU picker's `pick_marker_distance`.
/// `index` is the point index the styled entries hash and `salt` the
/// series' `fnv1a(series_id)`.
fn styled_marker_distance(
    style: DrawnStyle,
    (shape_id, radius_px): (u32, f32),
    center: [f32; 2],
    cursor: [f32; 2],
    (index, salt): (u32, u32),
) -> f32 {
    let p = [cursor[0] - center[0], -(cursor[1] - center[1])];
    let params = style.params;
    match style.kind {
        DrawnKind::Milkyway => {
            let r = radius_px;
            let r_planet = r.max(1.0);
            let quad = r * PLANET_QUAD_EXTENT + MARKER_QUAD_MARGIN_PX;
            let outside_quad = p[0].abs().max(p[1].abs()) - quad;
            let scale = (r_planet * PLANET_QUAD_EXTENT + MARKER_QUAD_MARGIN_PX) / quad;
            let p = [p[0] * scale, p[1] * scale];
            let dist = length2(p);
            let rim_gain = params[1][3].clamp(0.0, 2.0);
            let falloff = (r_planet * 0.10).max(1.0);
            let rim = rim_gain * (-(dist - r_planet).max(0.0) / falloff).exp();
            let mut d = dist - r_planet;
            if rim > PLANET_ALPHA_FLOOR {
                d = d.min(0.0);
            } else if rim_gain > PLANET_ALPHA_FLOOR {
                d = d.min(dist - r_planet - falloff * (rim_gain / PLANET_ALPHA_FLOOR).ln());
            }
            let seed_inst = ((params[0][3] as u32) ^ salt).wrapping_add(index);
            let phi = cons_ring_angle(shape_id) + (sketch_hash01(seed_inst, 0x717A) - 0.5) * 0.12;
            let (c, s) = (phi.cos(), phi.sin());
            let q = [p[0] * c + p[1] * s, -p[0] * s + p[1] * c];
            let rho = length2([q[0], q[1] / RING_INCL]) / r_planet;
            d = d.min((RING_INNER - rho).max(rho - RING_OUTER) * r_planet * RING_INCL);
            d.max(outside_quad).max(0.0)
        }
        DrawnKind::Constellation => {
            if !(params[0][0] > 0.0) {
                return f32::MAX;
            }
            let jitter = 0.86 + 0.30 * sketch_hash01(index, salt ^ 0x51A7);
            let star = (radius_px * jitter).max(0.5);
            (length2(p) - (star * 4.0 + MARKER_QUAD_MARGIN_PX)).max(0.0)
        }
        DrawnKind::Precise | DrawnKind::Sketch => {
            let filled = marker_shape_is_filled(shape_id);
            let base = marker_base_shape_id(shape_id);
            let visual_r = marker_visual_shape_radius(base, radius_px);
            let mut d = marker_shape_distance(base, p, visual_r, filled);
            if style.kind == DrawnKind::Sketch {
                let wobble = (params[0][0].max(0.0) * 0.5).min(radius_px * 0.35);
                let seed = ((params[0][2] as u32) ^ salt).wrapping_add(index);
                let theta = p[1].atan2(p[0]);
                d += wobble * sketch_noise(theta / SKETCH_TAU * SKETCH_CONTOUR_WOBBLES, seed);
            }
            marker_edge(d, filled).max(0.0)
        }
    }
}

/// Twin of `gpu_pick.rs::milkyway_line_half_extent`.
fn milkyway_line_half_extent(params: &[[f32; 4]; 3]) -> f32 {
    let ribbon = params[0][1].max(1.0);
    let star_scale = params[1][0].max(0.0);
    let spread = params[1][1].max(0.0);
    let structure = params[2][0].max(1e-3);
    let stars = if params[0][0] > 0.0 {
        spread * 1.7 + 4.0 * structure + star_scale * (0.9 + 3.4) * 4.0
    } else {
        0.0
    };
    ribbon.max(stars)
}

/// How a line series' stroke is drawn, for the CPU twin of the line
/// shader's extrusion: the drawn half-width (`max(width, 1) / 2`), the dash
/// pattern in pixels (empty for a solid line), the draw style and the
/// series' hash salt.
#[derive(Clone, Copy)]
struct LineStroke {
    half_width: f32,
    dash: &'static [f32],
    style: DrawnStyle,
    salt: u32,
}

/// Twin of `line_stroke_distance`: signed distance from `(along, side)` to
/// the square-capped rectangle `[-half_w, len + half_w] x [-half_w, half_w]`.
fn line_stroke_distance(along: f32, side: f32, len: f32, half_w: f32) -> f32 {
    let q = [
        (along - len * 0.5).abs() - (len * 0.5 + half_w),
        side.abs() - half_w,
    ];
    q[0].max(0.0).hypot(q[1].max(0.0)) + q[0].max(q[1]).min(0.0)
}

/// Twin of `dash_pattern_on` and the pick shader's `pick_dash_gap`: whether
/// the dash is on at arc position `dist_px` and, when it is off, the arc
/// distances back to the previous dash and forward to the next.
fn dash_at(dist_px: f32, dash: &[f32]) -> Result<(), (f32, f32)> {
    let spans = &dash[..dash.len().min(8)];
    if spans.is_empty() {
        return Ok(());
    }
    let period: f32 = spans.iter().map(|span| span.max(0.0)).sum();
    if period <= 0.0 {
        return Ok(());
    }
    let phase = ((dist_px % period) + period) % period;
    let mut acc = 0.0f32;
    for (index, span) in spans.iter().enumerate() {
        let start = acc;
        let end = acc + span.max(0.0);
        acc = end;
        if phase <= end || index == spans.len() - 1 {
            return if index % 2 == 0 {
                Ok(())
            } else {
                Err((phase - start, end - phase))
            };
        }
    }
    Ok(())
}

/// Twin of the pick shader's `pick_stroke_piece_distance`: the hit distance
/// of one drawn piece `a → b` (canvas pixels) whose ends sit at arc
/// positions `arc_start` / `arc_end`. `None` for a piece the vertex stage
/// collapses or a stroke drawn at zero opacity.
fn stroke_piece_distance(
    cursor: [f32; 2],
    (a, b): ([f32; 2], [f32; 2]),
    stroke: LineStroke,
    arc_start: f32,
    arc_end: f32,
) -> Option<f32> {
    // The draw's pixel frame is y up.
    let (a, b, cursor) = ([a[0], -a[1]], [b[0], -b[1]], [cursor[0], -cursor[1]]);
    let delta = [b[0] - a[0], b[1] - a[1]];
    let len = (delta[0] * delta[0] + delta[1] * delta[1]).sqrt();
    if !(len > 1e-6) {
        return None;
    }
    let dir = [delta[0] / len, delta[1] / len];
    let normal = [-dir[1], dir[0]];
    let rel = [cursor[0] - a[0], cursor[1] - a[1]];
    let along = rel[0] * dir[0] + rel[1] * dir[1];
    let mut side = rel[0] * normal[0] + rel[1] * normal[1];
    let params = stroke.style.params;
    match stroke.style.kind {
        DrawnKind::Milkyway => {
            let half = milkyway_line_half_extent(&params);
            return Some(line_stroke_distance(along, side, len, half).max(0.0));
        }
        DrawnKind::Constellation if !(params[0][1] > 0.0) => return None,
        _ => {}
    }
    let half_w = stroke.half_width;
    let geom_half_w = half_w
        + if stroke.dash.is_empty() {
            LINE_AA_EXTENT_PX
        } else {
            0.0
        };
    let (dist_px, slope) = if stroke.style.kind == DrawnKind::Sketch {
        // The strip's knots sit at `t_k * len` along the piece, the outer two
        // pushed out by the square cap, each displaced along the normal by
        // the wobble at its arc position.
        let s = SKETCH_SUBDIV as f32;
        let k = (along / len * s).floor().clamp(0.0, s - 1.0) as u32;
        let t0 = k as f32 / s;
        let t1 = (k + 1) as f32 / s;
        let a0 = if k == 0 { -geom_half_w } else { t0 * len };
        let a1 = if k + 1 == SKETCH_SUBDIV {
            len + geom_half_w
        } else {
            t1 * len
        };
        let w = ((along - a0) / (a1 - a0)).clamp(0.0, 1.0);
        let seed = (params[0][2] as u32) ^ stroke.salt;
        let arc0 = arc_start + (arc_end - arc_start) * t0;
        let arc1 = arc_start + (arc_end - arc_start) * t1;
        let d0 = sketch_line_offset(arc0, params[0][0], params[0][1], seed);
        let d1 = sketch_line_offset(arc1, params[0][0], params[0][1], seed);
        side -= d0 + (d1 - d0) * w;
        let dist0 = arc0 + if k == 0 { -half_w } else { 0.0 };
        let dist1 = arc1 + if k + 1 == SKETCH_SUBDIV { half_w } else { 0.0 };
        (dist0 + (dist1 - dist0) * w, (dist1 - dist0) / (a1 - a0))
    } else {
        // `vs_main` puts `arc_start - half_w` on the A-side corners and
        // `arc_end + half_w` on the B-side ones.
        let slope = (arc_end - arc_start + 2.0 * half_w) / (len + 2.0 * geom_half_w);
        ((arc_start - half_w) + (along + geom_half_w) * slope, slope)
    };
    let stroke_distance = line_stroke_distance(along, side, len, half_w).max(0.0);
    let Err((back, forward)) = dash_at(dist_px, stroke.dash) else {
        return Some(stroke_distance);
    };
    let scale = slope.abs().max(1e-6);
    let across = (side.abs() - half_w).max(0.0);
    let mut best = f32::MAX;
    let before = along - back / scale;
    if before >= -half_w {
        best = best.min((along - before.min(len + half_w)).hypot(across));
    }
    let after = along + forward / scale;
    if after <= len + half_w {
        best = best.min((after.max(-half_w) - along).hypot(across));
    }
    Some(best.max(f32::EPSILON))
}

/// Distance from `cursor` to the stroke of one segment drawn as `pieces`
/// (projected canvas pixels, in draw order), and the source index the hit
/// snaps to. `arc` is the cumulative arc length at the segment's two
/// samples, which places the dash pattern.
///
/// Every piece is tested and the nearest wins (the earlier piece on a tie).
/// The snapped index follows the position of the hit along the whole stepped
/// path — 0 at the segment's first sample, 1 at its second — so a hit on a
/// riser resolves to the nearer of the two source indices, never to a corner
/// that is not a sample. A linear segment is its single piece and keeps the
/// straight-segment rule bit for bit. Twin of `pick_line_candidate` in
/// `gpu_pick/line.wgsl`.
fn line_segment_pick(
    cursor: [f32; 2],
    pieces: &[([f32; 2], [f32; 2])],
    stroke: LineStroke,
    arc: (f32, f32),
    point_index: usize,
) -> Option<(f32, usize)> {
    let [x, y] = cursor;
    let mut coordinate_scale = 1.0f32;
    let mut total_len = 0.0f32;
    let mut prefix_len = 0.0f32;
    let mut best: Option<(f32, f32, f32, f32)> = None; // (dist, t, prefix, len)
    for (index, &([ax, ay], [bx, by])) in pieces.iter().enumerate() {
        coordinate_scale = coordinate_scale
            .max(ax.abs())
            .max(ay.abs())
            .max(bx.abs())
            .max(by.abs());
        let sx = bx - ax;
        let sy = by - ay;
        let len_sq = sx.mul_add(sx, sy * sy);
        if !len_sq.is_finite() {
            return None;
        }
        let len = len_sq.sqrt();
        let piece_prefix = prefix_len;
        let arc_start = arc.0 + piece_prefix;
        let arc_end = if index + 1 == pieces.len() {
            arc.1
        } else {
            arc_start + len
        };
        prefix_len += len;
        total_len += len;
        // A zero-length piece (a riser between equal values, or a degenerate
        // linear segment) has no stroke to hit.
        let Some(dist) =
            stroke_piece_distance(cursor, ([ax, ay], [bx, by]), stroke, arc_start, arc_end)
        else {
            continue;
        };
        if dist.is_finite() && best.is_none_or(|(best_dist, ..)| dist < best_dist) {
            let t = (((x - ax) * sx + (y - ay) * sy) / len_sq).clamp(0.0, 1.0);
            best = Some((dist, t, piece_prefix, len));
        }
    }
    let (dist, t, prefix, len) = best?;
    // Position along the whole stepped path. A linear segment keeps `t`
    // itself: `(t · len) / len` is not bit-exact.
    let position = if pieces.len() > 1 {
        (prefix + t * len) / total_len
    } else {
        t
    };
    // Match the GPU's lower-index tie rule within projected f32 uncertainty.
    let midpoint_tolerance = (f32::EPSILON * coordinate_scale / total_len).min(0.25);
    let snapped_index = if position <= 0.5 + midpoint_tolerance {
        point_index
    } else {
        point_index + 1
    };
    Some((dist, snapped_index))
}

/// A sample outside a polar sector keeps its projection: the fragment clip
/// cuts what it draws there, which `polar_projection::clip_distance` applies
/// at the cursor.
fn project_data_to_canvas_px(
    config: &Config,
    transform: &data_render::ScatterTransform,
    x: f32,
    y: f32,
) -> Option<(f32, f32)> {
    if (transform.scale_log[0] != 0.0 && x <= 0.0) || (transform.scale_log[1] != 0.0 && y <= 0.0) {
        return None;
    }
    let axis_t = |value: f32, axis: usize| {
        let min_hi = transform.data_min[axis];
        let max_hi = transform.data_max[axis];
        let min_lo = transform.data_min_lo[axis];
        let max_lo = transform.data_max_lo[axis];
        let linear_num = (value - min_hi) - min_lo;
        let range = (max_hi - min_hi) + (max_lo - min_lo);
        let log_num = (maybe_log(value, transform.scale_log[axis]) - min_hi) - min_lo;
        // Chosen exactly, as the shaders' early return does (SHADER_COMMON §3).
        let data_t = if transform.scale_log[axis] > 0.5 {
            log_num / range
        } else {
            linear_num / range
        };
        transform.data_to_panel_offset[axis] + data_t * transform.data_to_panel_scale[axis]
    };
    let tx = axis_t(x, 0);
    let ty = axis_t(y, 1);
    let [ndc_x, ndc_y] = if crate::polar_projection::is_polar(transform) {
        if !crate::polar_projection::sample_valid(transform, ty) {
            return None;
        }
        crate::polar_projection::ndc(transform, x, ty)
    } else {
        [tx * 2.0 - 1.0, ty * 2.0 - 1.0]
    };
    if !ndc_x.is_finite() || !ndc_y.is_finite() {
        return None;
    }

    let ca = config.chart_area.0;
    let chart_w = ca.width.max(1) as f32;
    let chart_h = ca.height.max(1) as f32;
    let px = ca.x as f32 + (ndc_x + 1.0) * 0.5 * chart_w;
    let py = ca.y as f32 + (1.0 - ndc_y) * 0.5 * chart_h;
    if px.is_finite() && py.is_finite() {
        Some((px, py))
    } else {
        None
    }
}

/// CPU twin of the shaders' `maybe_log` (SHADER_COMMON §3).
fn maybe_log(v: f32, is_log: f32) -> f32 {
    if is_log > 0.5 {
        v.max(1e-30).log10()
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::color::Color;
    use crate::config::AxisScale;
    use crate::data_config::{
        DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType, DataScatterPointStyleConfig,
        DataScatterStyleConfig, ErrorRef, ScatterShape,
    };
    use crate::layout::{ChartArea, Rect};
    use crate::line::LineStylePreset;

    const PRECISE: DrawnStyle = DrawnStyle {
        kind: DrawnKind::Precise,
        params: [[0.0; 4]; 3],
    };

    fn solid(half_width: f32) -> LineStroke {
        LineStroke {
            half_width,
            dash: &[],
            style: PRECISE,
            salt: 0,
        }
    }

    #[test]
    fn segment_distance_preserves_centerlines_and_finite_endpoints() {
        for (a, b, cursor, expected, expected_index) in [
            ([10.0, 50.0], [90.0, 50.0], [50.0, 50.0], 0.0, 0),
            ([10.0, 50.0], [90.0, 50.0], [70.0, 50.0], 0.0, 1),
            ([10.0, 50.0], [90.0, 50.0], [5.0, 50.0], 5.0, 0),
            ([10.0, 50.0], [90.0, 50.0], [95.0, 50.0], 5.0, 1),
            ([10.0, 50.0], [90.0, 50.0], [50.0, 53.0], 3.0, 0),
            ([50.0, 10.0], [50.0, 90.0], [50.0, 30.0], 0.0, 0),
            ([0.0, 0.0], [30.0, 40.0], [11.0, 23.0], 5.0, 0),
        ] {
            let (distance, index) =
                line_segment_pick(cursor, &[(a, b)], solid(0.0), (0.0, 0.0), 0).unwrap();
            assert!(
                (distance - expected).abs() < 0.0001,
                "{distance} vs {expected}"
            );
            assert_eq!(index, expected_index);
        }
        assert_eq!(
            line_segment_pick(
                [50.0, 53.0],
                &[([10.0, 50.0], [90.0, 50.0])],
                solid(1.0),
                (0.0, 0.0),
                0
            ),
            Some((2.0, 0))
        );
        assert_eq!(
            line_segment_pick(
                [1.0, 1.0],
                &[([0.0, 0.0], [0.0, 0.0])],
                solid(0.0),
                (0.0, 0.0),
                0
            ),
            None
        );
    }

    /// The stroke has square caps: the cap's corners are drawn and pick at
    /// distance zero, where a round cap would not reach.
    #[test]
    fn segment_caps_are_square_like_the_drawn_stroke() {
        let segment = [([10.0, 50.0], [90.0, 50.0])];
        let corner = line_segment_pick([8.5, 51.5], &segment, solid(2.0), (0.0, 0.0), 0);
        assert_eq!(corner, Some((0.0, 0)));
        let (beyond, _) =
            line_segment_pick([7.0, 53.0], &segment, solid(2.0), (0.0, 0.0), 0).unwrap();
        assert!((beyond - 2.0f32.sqrt()).abs() < 1e-5, "{beyond}");
    }

    /// A dashed stroke picks on its dashes; in a gap the distance runs along
    /// the stroke to the nearer dash.
    #[test]
    fn dashed_segment_picks_its_dashes_and_not_its_gaps() {
        let dashed = LineStroke {
            half_width: 1.0,
            dash: &[8.0, 4.0],
            style: PRECISE,
            salt: 0,
        };
        let segment = [([0.0, 50.0], [100.0, 50.0])];
        // The arc position runs with the stroke from 0 at the first sample.
        let on = line_segment_pick([5.0, 50.0], &segment, dashed, (0.0, 100.0), 0);
        assert_eq!(on, Some((0.0, 0)));
        let (gap, _) = line_segment_pick([10.0, 50.0], &segment, dashed, (0.0, 100.0), 0).unwrap();
        assert!((gap - 2.0).abs() < 1e-4, "{gap}");
        let (gap, _) = line_segment_pick([11.0, 50.0], &segment, dashed, (0.0, 100.0), 0).unwrap();
        assert!((gap - 1.0).abs() < 1e-4, "{gap}");
        // The same cursor with the segment starting 4 px further along the
        // pattern lands on the next dash.
        let shifted = line_segment_pick([10.0, 50.0], &segment, dashed, (4.0, 104.0), 0);
        assert_eq!(shifted, Some((0.0, 0)));
    }

    // A stepped segment is picked along its pieces: the nearest piece wins,
    // a zero-length riser is skipped rather than invalidating the segment,
    // and the snapped index follows the position along the whole path.
    #[test]
    fn stepped_segment_distance_walks_every_piece_and_snaps_by_path_position() {
        // step-post from (10, 10) to (90, 50): run at y = 10, riser at x = 90.
        let post = [([10.0, 10.0], [90.0, 10.0]), ([90.0, 10.0], [90.0, 50.0])];
        let pick = |cursor: [f32; 2], pieces: &[([f32; 2], [f32; 2])], index| {
            line_segment_pick(cursor, pieces, solid(0.0), (0.0, 0.0), index)
        };
        // On the run, a quarter along the path: index 0.
        assert_eq!(pick([30.0, 10.0], &post, 4), Some((0.0, 4)));
        // On the riser: past the midpoint of the 120 px path, index 1.
        assert_eq!(pick([90.0, 40.0], &post, 4), Some((0.0, 5)));
        // Far from the run but near the riser: the riser's distance wins.
        let (dist, index) = pick([95.0, 45.0], &post, 4).unwrap();
        assert!((dist - 5.0).abs() < 1e-4);
        assert_eq!(index, 5);
        // The straight chord between the endpoints is NOT part of the path.
        let (dist, _) = pick([50.0, 30.0], &post, 4).unwrap();
        assert!((dist - 20.0).abs() < 1e-3);
        // A riser between equal values has zero length and is skipped.
        let flat = [([10.0, 10.0], [90.0, 10.0]), ([90.0, 10.0], [90.0, 10.0])];
        assert_eq!(pick([90.0, 10.0], &flat, 0), Some((0.0, 1)));
        // Every piece degenerate: nothing to hit.
        let point = [([5.0, 5.0], [5.0, 5.0]), ([5.0, 5.0], [5.0, 5.0])];
        assert_eq!(pick([5.0, 5.0], &point, 0), None);
    }

    struct Columns(HashMap<ColumnId, Vec<f32>>);

    impl Columns {
        fn new(entries: &[(&str, &[f32])]) -> Self {
            Self(
                entries
                    .iter()
                    .map(|(id, values)| ((*id).to_string(), values.to_vec()))
                    .collect(),
            )
        }
    }

    impl PointColumnLookup for Columns {
        fn get_f32_column(&self, id: &ColumnId) -> Option<&[f32]> {
            self.0.get(id).map(Vec::as_slice)
        }
    }

    fn config() -> Config {
        let mut c = crate::default::default_config();
        c.chart_area = ChartArea(Rect {
            x: 10,
            y: 20,
            width: 100,
            height: 100,
        });
        c.bottom_x.min = 0.0;
        c.bottom_x.max = 10.0;
        c.left_y.min = 0.0;
        c.left_y.max = 10.0;
        c.chart_title.top_margin = 0.0;
        for axis in [&mut c.top_x, &mut c.bottom_x, &mut c.left_y, &mut c.right_y] {
            axis.out_margin = 0.0;
            axis.major_tick_length = 0.0;
        }
        c
    }

    fn line_series(id: &str, x: &str, y: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::BLACK,
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        }
    }

    fn scatter_series(id: &str, x: &str, y: &str, source_id: Option<&str>) -> SeriesConfig {
        scatter_series_with_size(id, x, y, source_id, 3.0)
    }

    fn scatter_series_with_size(
        id: &str,
        x: &str,
        y: &str,
        source_id: Option<&str>,
        point_size: f32,
    ) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: source_id.map(str::to_string),
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Scatter {
                scatter: DataScatterStyleConfig {
                    point_color: Color::BLACK,
                    point_shape: ScatterShape::CircleFilled,
                    point_size,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                },
            },
        }
    }

    fn scatter_errorbar_y_series(
        id: &str,
        x: &str,
        y: &str,
        err_y: &str,
        point_size: f32,
    ) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::ScatterErrorbarY {
                scatter: DataScatterStyleConfig {
                    point_color: Color::BLACK,
                    point_shape: ScatterShape::CircleFilled,
                    point_size,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                },
                err_y: ErrorRef::Symmetric {
                    column: err_y.into(),
                },
                err_style: DataErrorBarStyleConfig {
                    error_bar_color: Color::BLACK,
                    error_bar_width: 1.0,
                    error_bar_cap_size: 6.0,
                    cap_width: 1.0,
                    error_bar_style_table: None,
                    error_bar_style_index_column: None,
                    error_bar_style_overrides: None,
                },
            },
        }
    }

    fn pick(
        config: &Config,
        series: &[SeriesConfig],
        columns: &Columns,
        x: f32,
        y: f32,
        max_distance_px: f32,
    ) -> Option<PickedPoint> {
        pick_nearest_point(
            config,
            series,
            columns,
            x,
            y,
            PointPickOptions { max_distance_px },
        )
    }

    #[test]
    fn picks_nearest_point_across_series() {
        let config = config();
        let columns = Columns::new(&[
            ("x0", &[1.0, 5.0]),
            ("y0", &[1.0, 5.0]),
            ("x1", &[7.0]),
            ("y1", &[7.0]),
        ]);
        let series = [
            scatter_series("farther", "x0", "y0", Some("src-a")),
            scatter_series("nearest", "x1", "y1", None),
        ];

        let picked = pick(&config, &series, &columns, 81.0, 50.0, 20.0).unwrap();

        assert_eq!(picked.series_id, "nearest");
        assert_eq!(picked.source_id, None);
        assert_eq!(picked.point_index, 0);
    }

    #[test]
    fn single_point_line_series_is_not_pickable_without_visible_segment() {
        let config = config();
        let columns = Columns::new(&[("x", &[5.0]), ("y", &[5.0])]);
        let series = [line_series("single-line", "x", "y")];

        assert_eq!(pick(&config, &series, &columns, 60.0, 70.0, 0.0), None);
    }

    #[test]
    fn max_distance_miss_returns_none() {
        let config = config();
        let columns = Columns::new(&[("x", &[5.0]), ("y", &[5.0])]);
        let series = [scatter_series("s", "x", "y", None)];

        assert_eq!(pick(&config, &series, &columns, 70.0, 70.0, 5.0), None);
    }

    #[test]
    fn line_series_picks_nearest_endpoint_from_segment_hit() {
        let config = config();
        let columns = Columns::new(&[("x", &[0.0, 10.0]), ("y", &[0.0, 10.0])]);
        let series = [line_series("line", "x", "y")];

        let picked = pick(&config, &series, &columns, 80.0, 50.0, 0.0).unwrap();

        assert_eq!(picked.series_id, "line");
        assert_eq!(picked.point_index, 1);
        assert_eq!(picked.distance_px, 0.0);
    }

    #[test]
    fn line_width_counts_as_pickable_stroke_area() {
        let mut config = config();
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 10.0;
        config.left_y.min = 0.0;
        config.left_y.max = 10.0;
        let columns = Columns::new(&[("x", &[0.0, 10.0]), ("y", &[5.0, 5.0])]);
        let mut series = line_series("wide", "x", "y");
        let DataRenderType::Line { line } = &mut series.render_type else {
            unreachable!();
        };
        line.line_width = 10.0;

        let picked = pick(&config, &[series], &columns, 80.0, 74.0, 0.0).unwrap();

        assert_eq!(picked.series_id, "wide");
        assert_eq!(picked.point_index, 1);
        assert_eq!(picked.distance_px, 0.0);
    }

    #[test]
    fn scatter_series_does_not_pick_between_points() {
        let config = config();
        let columns = Columns::new(&[("x", &[0.0, 10.0]), ("y", &[0.0, 10.0])]);
        let series = [scatter_series("scatter", "x", "y", None)];

        assert_eq!(pick(&config, &series, &columns, 80.0, 50.0, 0.0), None);
    }

    #[test]
    fn scatter_pick_uses_marker_radius_when_distance_limit_is_zero() {
        let config = config();
        let columns = Columns::new(&[("x", &[5.0]), ("y", &[5.0])]);
        let series = [scatter_series_with_size("big", "x", "y", None, 8.0)];

        let picked = pick(&config, &series, &columns, 67.0, 70.0, 0.0).unwrap();

        assert_eq!(picked.series_id, "big");
        assert_eq!(picked.point_index, 0);
        assert_eq!(picked.distance_px, 0.0);
    }

    #[test]
    fn zero_size_marker_picks_the_dot_it_draws() {
        // A zero-size marker still draws: the scatter shader evaluates the
        // shape at radius zero, a dot of the outline stroke's half-width (or
        // a sub-pixel point when filled). Picking follows the drawing; only
        // a point the size map hides is not drawn and not picked.
        let config = config();
        let columns = Columns::new(&[("x", &[5.0]), ("y", &[5.0]), ("ey", &[2.0])]);
        let mut series = scatter_errorbar_y_series("err", "x", "y", "ey", 0.0);
        let DataRenderType::ScatterErrorbarY { scatter, .. } = &mut series.render_type else {
            unreachable!();
        };
        scatter.point_shape = ScatterShape::Circle;
        let series = [series];

        let dot = pick(&config, &series, &columns, 60.5, 70.0, 0.0).unwrap();
        assert_eq!((dot.point_index, dot.distance_px), (0, 0.0));
        let near = pick(&config, &series, &columns, 62.0, 70.0, 2.0).unwrap();
        assert!((near.distance_px - (2.0 - MARKER_STROKE_HALF_PX)).abs() < 1e-5);
        assert_eq!(pick(&config, &series, &columns, 61.0, 70.0, 0.0), None);
    }

    #[test]
    fn scatter_pick_uses_per_point_size_overrides() {
        let config = config();
        let columns = Columns::new(&[
            ("x", &[5.0, 8.0]),
            ("y", &[5.0, 5.0]),
            ("style", &[0.0, 1.0]),
        ]);
        let mut series = scatter_series_with_size("mapped", "x", "y", None, 0.0);
        let DataRenderType::Scatter { scatter } = &mut series.render_type else {
            unreachable!();
        };
        scatter.point_style_index_column = Some("style".into());
        scatter.point_style_table = Some(vec![
            DataScatterPointStyleConfig {
                point_size: Some(0.0),
                ..Default::default()
            },
            DataScatterPointStyleConfig {
                point_size: Some(10.0),
                ..Default::default()
            },
        ]);

        // Point 0's table size is zero: a filled marker of radius zero is a
        // point, so one pixel beside it nothing is drawn or picked.
        assert_eq!(
            pick(&config, &[series.clone()], &columns, 61.0, 70.0, 0.0),
            None
        );
        let picked = pick(&config, &[series], &columns, 93.0, 70.0, 0.0).unwrap();

        assert_eq!(picked.series_id, "mapped");
        assert_eq!(picked.point_index, 1);
    }

    fn vector_points_series(id: &str, u: &str, v: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Vector {
                components: crate::data_config::VectorComponents::Points {
                    u: u.into(),
                    v: v.into(),
                },
                style: crate::data_config::VectorStyle {
                    color: Color::BLACK,
                    width_px: 4.0,
                    scale: crate::data_config::VectorScale::Pixels { px_per_unit: 1.0 },
                    pivot: crate::data_config::VectorPivot::Tail,
                    head_length_px: 10.0,
                    head_width_px: 10.0,
                    min_length_px: 0.0,
                    color_map: None,
                },
            },
        }
    }

    /// A point vector is picked on the arrow it draws: the shaft (4 px wide,
    /// round-ended like the draw's segment distance) and the head, not a disc
    /// at the anchor. An arrow that is not drawn — a zero vector, or one past
    /// the shortest component column — does not pick.
    #[test]
    fn vector_points_pick_on_the_drawn_arrow() {
        let config = config();
        let columns = Columns::new(&[
            ("x", &[2.0, 8.0]),
            ("y", &[5.0, 5.0]),
            ("u", &[20.0, 0.0]),
            ("v", &[0.0, 0.0]),
            ("u_short", &[20.0]),
            ("u_long", &[20.0, 20.0]),
        ]);
        let series = [vector_points_series("arrows", "u", "v")];

        // Anchor 0 at (30, 70): a 20 px arrow to (50, 70) whose 10 px head
        // starts at x = 40.
        let shaft = pick(&config, &series, &columns, 35.0, 71.5, 0.0).unwrap();
        assert_eq!((shaft.point_index, shaft.distance_px), (0, 0.0));
        assert_eq!(pick(&config, &series, &columns, 35.0, 73.0, 0.0), None);
        let beside = pick(&config, &series, &columns, 35.0, 73.0, 2.0).unwrap();
        assert!((beside.distance_px - 1.0).abs() < 1e-4);
        let head = pick(&config, &series, &columns, 45.0, 72.0, 0.0).unwrap();
        assert_eq!((head.point_index, head.distance_px), (0, 0.0));
        // Past the tip, and behind the tail beyond the shaft's half width.
        assert_eq!(pick(&config, &series, &columns, 52.0, 70.0, 0.0), None);
        let behind = pick(&config, &series, &columns, 27.0, 70.0, 2.0).unwrap();
        assert!((behind.distance_px - 1.0).abs() < 1e-4);

        // Anchor 1 at (90, 70) is a zero vector: nothing is drawn there.
        assert_eq!(pick(&config, &series, &columns, 90.0, 70.0, 0.0), None);
        let long = [vector_points_series("arrows", "u_long", "v")];
        assert_eq!(
            pick(&config, &long, &columns, 95.0, 70.0, 0.0)
                .unwrap()
                .point_index,
            1
        );

        // A component column shorter than the anchors hides the surplus.
        let short = [vector_points_series("arrows", "u_short", "v")];
        assert_eq!(
            pick(&config, &short, &columns, 35.0, 70.0, 0.0)
                .unwrap()
                .point_index,
            0
        );
        assert_eq!(pick(&config, &short, &columns, 95.0, 70.0, 0.0), None);
    }

    #[test]
    fn outside_valid_data_area_returns_none() {
        let config = config();
        let columns = Columns::new(&[("x", &[0.0]), ("y", &[0.0])]);
        let series = [scatter_series("s", "x", "y", None)];

        assert_eq!(pick(&config, &series, &columns, 9.0, 20.0, 100.0), None);
    }

    #[test]
    fn log_transform_matches_visible_position() {
        let mut config = config();
        config.bottom_x.scale = AxisScale::Logarithmic;
        config.bottom_x.min = 1.0;
        config.bottom_x.max = 100.0;
        let columns = Columns::new(&[("x", &[10.0]), ("y", &[5.0])]);
        let series = [scatter_series("log", "x", "y", None)];

        let picked = pick(&config, &series, &columns, 60.0, 70.0, 0.0).unwrap();

        assert_eq!(picked.series_id, "log");
        assert_eq!(picked.point_index, 0);
        assert_eq!(picked.distance_px, 0.0);
    }

    #[test]
    fn inverted_axes_match_visible_position() {
        let mut config = config();
        config.bottom_x.inverted = true;
        config.left_y.inverted = true;
        let columns = Columns::new(&[("x", &[2.0]), ("y", &[8.0])]);
        let series = [scatter_series("inverted", "x", "y", Some("src-inv"))];

        let picked = pick(&config, &series, &columns, 90.0, 100.0, 0.0).unwrap();

        assert_eq!(picked.source_id, Some("src-inv".into()));
        assert_eq!(picked.series_id, "inverted");
        assert_eq!(picked.point_index, 0);
        assert_eq!(picked.distance_px, 0.0);
    }

    /// An exact tie goes to the marker painted on top: a later series paints
    /// over an earlier one, and within a series instance 1 paints over
    /// instance 0.
    #[test]
    fn exact_tie_prefers_the_marker_painted_on_top() {
        let config = config();
        let columns = Columns::new(&[
            ("x0", &[5.0]),
            ("y0", &[5.0]),
            ("x1", &[5.0, 5.0]),
            ("y1", &[5.0, 5.0]),
        ]);
        let series = [
            scatter_series("early", "x0", "y0", None),
            scatter_series("late", "x1", "y1", Some("src-b")),
        ];

        let picked = pick(&config, &series, &columns, 60.0, 70.0, 0.0).unwrap();

        assert_eq!(picked.series_id, "late");
        assert_eq!(picked.source_id, Some("src-b".into()));
        assert_eq!(picked.point_index, 1);
    }
}
