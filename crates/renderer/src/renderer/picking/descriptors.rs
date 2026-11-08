//! The owned pick descriptors of each series kind and their equality.

use super::*;

/// The arrows the vector shader draws for a point vector series, in the
/// picker's terms: the same columns, scale mode, pivot and sizes
/// (`Renderer::ensure_vector_scratch`), in logical pixels.
pub(in crate::renderer) fn gpu_pick_vector(
    components: &crate::data_config::VectorComponents,
    style: &crate::data_config::VectorStyle,
) -> Option<crate::gpu_pick::GpuPickVector> {
    use crate::data_config::{
        VectorColorMap, VectorComponents, VectorPivot, VectorScale, VectorStyle,
    };
    use data_render::vector as vector_render;
    let VectorComponents::Points { u, v, .. } = components else {
        return None;
    };
    let VectorStyle {
        // The hit shape does not depend on the arrow colour.
        color: _,
        width_px,
        scale,
        pivot,
        head_length_px,
        head_width_px,
        min_length_px,
        color_map,
    } = style;
    let (scale_pixels, scale) = match scale {
        VectorScale::Data { factor } => (false, *factor as f32),
        VectorScale::Pixels { px_per_unit } => (true, *px_per_unit as f32),
    };
    Some(crate::gpu_pick::GpuPickVector {
        u_column: u.clone(),
        v_column: v.clone(),
        color_column: match color_map.as_ref() {
            Some(VectorColorMap::Column(column)) => Some(column.clone()),
            _ => None,
        },
        scale_pixels,
        scale,
        pivot: match pivot {
            VectorPivot::Tail => vector_render::VECTOR_PIVOT_TAIL,
            VectorPivot::Middle => vector_render::VECTOR_PIVOT_MIDDLE,
            VectorPivot::Tip => vector_render::VECTOR_PIVOT_TIP,
        },
        width_px: *width_px,
        head_length_px: *head_length_px,
        head_width_px: *head_width_px,
        min_length_px: *min_length_px,
    })
}

/// A series' pick slot as the plan builds it: its identity and the
/// resource the slot is built from.
#[derive(PartialEq)]
#[cfg_attr(test, derive(Clone, Debug))]
pub(in crate::renderer) struct OwnedGpuPickSeriesDescriptor {
    pub(in crate::renderer) series_id: String,
    pub(in crate::renderer) source_id: Option<String>,
    pub(in crate::renderer) resource: PickSeriesResource,
}

/// What a series' pick slot is built from, besides its identity. The
/// engine builds a slot from its descriptor and the column pool alone
/// (`GpuPickEngine::prepare_series`), so this is the slot's cache key and
/// the build's only series input (#66, tier 1): the plan reuses a slot
/// exactly when its series id and resource are equal. Floats compare by
/// bits and GPU rows by bytes. The pick never reads colour, so the style
/// rows carry none (`pick_style_row`) and a colour-only edit reuses the
/// slot.
#[cfg_attr(test, derive(Clone, Debug))]
pub(in crate::renderer) struct PickSeriesResource {
    pub(in crate::renderer) x_column: ColumnId,
    pub(in crate::renderer) y_column: ColumnId,
    pub(in crate::renderer) scatter: Option<OwnedGpuPickScatter>,
    pub(in crate::renderer) line_width_px: Option<f32>,
    pub(in crate::renderer) line_interpolation: LineInterpolation,
    pub(in crate::renderer) line_style: LineStylePreset,
    pub(in crate::renderer) vector: Option<crate::gpu_pick::GpuPickVector>,
    /// The error bars drawn with the markers; their style rows follow the
    /// marker rows in `scatter`'s.
    pub(in crate::renderer) errorbar: Option<OwnedGpuPickErrorBar>,
    /// The line's area fill.
    pub(in crate::renderer) area: Option<crate::gpu_pick::GpuPickArea>,
    /// The stems drawn with the markers (#206).
    pub(in crate::renderer) stem: Option<crate::gpu_pick::GpuPickStem>,
    /// Whether the line is a closed polygon (a radar outline).
    pub(in crate::renderer) line_closed: bool,
    /// A streamline series' arrowheads along its line.
    pub(in crate::renderer) path_arrows: Option<crate::data_config::StreamlineArrows>,
    /// A box plot's raw points, placed in their category slot.
    pub(in crate::renderer) category_jitter: Option<crate::gpu_pick::GpuPickCategoryJitter>,
    /// A ternary series' third component.
    pub(in crate::renderer) ternary_c: Option<ColumnId>,
    /// Whether the ternary series fills the regions its samples outline.
    pub(in crate::renderer) ternary_region: bool,
}

/// A series' error bars in the plan (`GpuPickErrorBar` without its rows,
/// which live in the series' `OwnedGpuPickScatter` after the marker rows).
#[cfg_attr(test, derive(Clone, Debug))]
pub(in crate::renderer) struct OwnedGpuPickErrorBar {
    pub(in crate::renderer) y: Option<[ColumnId; 2]>,
    pub(in crate::renderer) x: Option<[ColumnId; 2]>,
    pub(in crate::renderer) stem_width_px: f32,
    pub(in crate::renderer) cap_half_px: f32,
    pub(in crate::renderer) cap_width_px: f32,
    pub(in crate::renderer) paints: bool,
    /// Precise with an error-bar style map: its index column. The map's rows
    /// are the scatter's rows past the marker rows.
    pub(in crate::renderer) style_map: Option<Option<ColumnId>>,
}

impl PartialEq for PickSeriesResource {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            x_column,
            y_column,
            scatter,
            line_width_px,
            line_interpolation,
            line_style,
            vector,
            errorbar,
            area,
            stem,
            line_closed,
            path_arrows,
            category_jitter,
            ternary_c,
            ternary_region,
        } = self;
        let Self {
            x_column: other_x_column,
            y_column: other_y_column,
            scatter: other_scatter,
            line_width_px: other_line_width_px,
            line_interpolation: other_line_interpolation,
            line_style: other_line_style,
            vector: other_vector,
            errorbar: other_errorbar,
            area: other_area,
            stem: other_stem,
            line_closed: other_line_closed,
            path_arrows: other_path_arrows,
            category_jitter: other_category_jitter,
            ternary_c: other_ternary_c,
            ternary_region: other_ternary_region,
        } = other;
        let jitter_bits = |jitter: &Option<crate::gpu_pick::GpuPickCategoryJitter>| {
            jitter.map(|j| {
                (
                    [j.member, j.members, u32::from(j.horizontal), j.seed],
                    [
                        j.group_width.to_bits(),
                        j.gap_px.to_bits(),
                        j.jitter.to_bits(),
                        j.offset.to_bits(),
                    ],
                )
            })
        };
        let arrow_bits = |arrows: &Option<crate::data_config::StreamlineArrows>| {
            arrows.map(|arrows| {
                [
                    arrows.spacing_px.to_bits(),
                    arrows.head_length_px.to_bits(),
                    arrows.head_width_px.to_bits(),
                ]
            })
        };
        let stem_bits = |stem: &Option<crate::gpu_pick::GpuPickStem>| {
            stem.as_ref().map(|stem| {
                (
                    stem.baseline.to_bits(),
                    stem.horizontal,
                    stem.width_px.to_bits(),
                    stem.dash,
                    stem.paints,
                )
            })
        };
        line_closed == other_line_closed
            && stem_bits(stem) == stem_bits(other_stem)
            && arrow_bits(path_arrows) == arrow_bits(other_path_arrows)
            && jitter_bits(category_jitter) == jitter_bits(other_category_jitter)
            && ternary_c == other_ternary_c
            && ternary_region == other_ternary_region
            && x_column == other_x_column
            && y_column == other_y_column
            && scatter == other_scatter
            && line_width_px.map(f32::to_bits) == other_line_width_px.map(f32::to_bits)
            && line_interpolation == other_line_interpolation
            && line_style == other_line_style
            && match (vector, other_vector) {
                (Some(vector), Some(other)) => same_pick_vector(vector, other),
                (None, None) => true,
                _ => false,
            }
            && match (errorbar, other_errorbar) {
                (Some(errorbar), Some(other)) => same_pick_errorbar(errorbar, other),
                (None, None) => true,
                _ => false,
            }
            && match (area, other_area) {
                (Some(area), Some(other)) => same_pick_area(area, other),
                (None, None) => true,
                _ => false,
            }
    }
}

impl PickSeriesResource {
    pub(in crate::renderer) fn references_column(&self, id: &str) -> bool {
        let Self {
            x_column,
            y_column,
            scatter,
            line_width_px: _,
            line_interpolation: _,
            line_style: _,
            vector,
            errorbar,
            area,
            stem: _,
            line_closed: _,
            path_arrows: _,
            category_jitter: _,
            ternary_c,
            ternary_region: _,
        } = self;
        x_column == id
            || ternary_c.as_deref() == Some(id)
            || y_column == id
            || scatter.as_ref().is_some_and(|scatter| {
                scatter.style_index_column.as_deref() == Some(id)
                    || scatter
                        .size_map
                        .as_ref()
                        .is_some_and(|size_map| size_map.column == id)
            })
            || vector.as_ref().is_some_and(|vector| {
                vector.u_column == id
                    || vector.v_column == id
                    || vector.color_column.as_deref() == Some(id)
            })
            || errorbar.as_ref().is_some_and(|errorbar| {
                errorbar
                    .y
                    .iter()
                    .chain(errorbar.x.iter())
                    .flatten()
                    .any(|column| column == id)
                    || errorbar
                        .style_map
                        .as_ref()
                        .is_some_and(|index| index.as_deref() == Some(id))
            })
            || area
                .as_ref()
                .is_some_and(|area| area.to_series.as_deref() == Some(id))
    }

    pub(in crate::renderer) fn references_any_column(&self, ids: &[&str]) -> bool {
        ids.iter().any(|id| self.references_column(id))
    }
}

/// A point vector's arrows, compared field by field with floats by bits.
pub(super) fn same_pick_vector(
    vector: &crate::gpu_pick::GpuPickVector,
    other: &crate::gpu_pick::GpuPickVector,
) -> bool {
    let crate::gpu_pick::GpuPickVector {
        u_column,
        v_column,
        color_column,
        scale_pixels,
        scale,
        pivot,
        width_px,
        head_length_px,
        head_width_px,
        min_length_px,
    } = vector;
    let crate::gpu_pick::GpuPickVector {
        u_column: other_u_column,
        v_column: other_v_column,
        color_column: other_color_column,
        scale_pixels: other_scale_pixels,
        scale: other_scale,
        pivot: other_pivot,
        width_px: other_width_px,
        head_length_px: other_head_length_px,
        head_width_px: other_head_width_px,
        min_length_px: other_min_length_px,
    } = other;
    u_column == other_u_column
        && v_column == other_v_column
        && color_column == other_color_column
        && scale_pixels == other_scale_pixels
        && scale.to_bits() == other_scale.to_bits()
        && pivot == other_pivot
        && width_px.to_bits() == other_width_px.to_bits()
        && head_length_px.to_bits() == other_head_length_px.to_bits()
        && head_width_px.to_bits() == other_head_width_px.to_bits()
        && min_length_px.to_bits() == other_min_length_px.to_bits()
}

/// A series' error bars, compared field by field with floats by bits.
pub(super) fn same_pick_errorbar(
    errorbar: &OwnedGpuPickErrorBar,
    other: &OwnedGpuPickErrorBar,
) -> bool {
    let OwnedGpuPickErrorBar {
        y,
        x,
        stem_width_px,
        cap_half_px,
        cap_width_px,
        paints,
        style_map,
    } = errorbar;
    let OwnedGpuPickErrorBar {
        y: other_y,
        x: other_x,
        stem_width_px: other_stem_width_px,
        cap_half_px: other_cap_half_px,
        cap_width_px: other_cap_width_px,
        paints: other_paints,
        style_map: other_style_map,
    } = other;
    y == other_y
        && x == other_x
        && stem_width_px.to_bits() == other_stem_width_px.to_bits()
        && cap_half_px.to_bits() == other_cap_half_px.to_bits()
        && cap_width_px.to_bits() == other_cap_width_px.to_bits()
        && paints == other_paints
        && style_map == other_style_map
}

/// A line's fill, compared field by field with the baseline by bits.
pub(super) fn same_pick_area(
    area: &crate::gpu_pick::GpuPickArea,
    other: &crate::gpu_pick::GpuPickArea,
) -> bool {
    let crate::gpu_pick::GpuPickArea {
        to_series,
        baseline,
        paints,
        closed,
    } = area;
    let crate::gpu_pick::GpuPickArea {
        to_series: other_to_series,
        baseline: other_baseline,
        paints: other_paints,
        closed: other_closed,
    } = other;
    to_series == other_to_series
        && baseline.to_bits() == other_baseline.to_bits()
        && paints == other_paints
        && closed == other_closed
}

/// A `point_size_map`, compared field by field with floats by bits.
pub(super) fn same_pick_size_map(
    size_map: &crate::gpu_pick::GpuPickSizeMap,
    other: &crate::gpu_pick::GpuPickSizeMap,
) -> bool {
    let crate::gpu_pick::GpuPickSizeMap {
        column,
        domain_min,
        domain_max,
        min_px,
        max_px,
        area,
    } = size_map;
    let crate::gpu_pick::GpuPickSizeMap {
        column: other_column,
        domain_min: other_domain_min,
        domain_max: other_domain_max,
        min_px: other_min_px,
        max_px: other_max_px,
        area: other_area,
    } = other;
    column == other_column
        && domain_min.to_bits() == other_domain_min.to_bits()
        && domain_max.to_bits() == other_domain_max.to_bits()
        && min_px.to_bits() == other_min_px.to_bits()
        && max_px.to_bits() == other_max_px.to_bits()
        && area == other_area
}

#[cfg_attr(test, derive(Clone, Debug))]
pub(in crate::renderer) struct OwnedGpuPickScatter {
    pub(in crate::renderer) base_radius_px: f32,
    pub(in crate::renderer) base_shape_id: u32,
    pub(in crate::renderer) style_index_column: Option<ColumnId>,
    /// The marker rows (`style_meta.style_count` of them), then the error
    /// bars' slot rows.
    pub(in crate::renderer) style_slots: Vec<data_render::ScatterStyleSlotGpu>,
    /// The marker overrides (`style_meta.override_count`), then the error
    /// bars' override rows.
    pub(in crate::renderer) style_overrides: Vec<data_render::ScatterStyleOverrideGpu>,
    pub(in crate::renderer) style_meta: Option<data_render::ScatterStyleMapMeta>,
    pub(in crate::renderer) size_map: Option<crate::gpu_pick::GpuPickSizeMap>,
}

impl PartialEq for OwnedGpuPickScatter {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            base_radius_px,
            base_shape_id,
            style_index_column,
            style_slots,
            style_overrides,
            style_meta,
            size_map,
        } = self;
        let Self {
            base_radius_px: other_base_radius_px,
            base_shape_id: other_base_shape_id,
            style_index_column: other_style_index_column,
            style_slots: other_style_slots,
            style_overrides: other_style_overrides,
            style_meta: other_style_meta,
            size_map: other_size_map,
        } = other;
        base_radius_px.to_bits() == other_base_radius_px.to_bits()
            && base_shape_id == other_base_shape_id
            && style_index_column == other_style_index_column
            && bytemuck::cast_slice::<_, u8>(style_slots)
                == bytemuck::cast_slice::<_, u8>(other_style_slots)
            && bytemuck::cast_slice::<_, u8>(style_overrides)
                == bytemuck::cast_slice::<_, u8>(other_style_overrides)
            && style_meta.as_ref().map(bytemuck::bytes_of)
                == other_style_meta.as_ref().map(bytemuck::bytes_of)
            && match (size_map, other_size_map) {
                (Some(size_map), Some(other)) => same_pick_size_map(size_map, other),
                (None, None) => true,
                _ => false,
            }
    }
}

impl OwnedGpuPickScatter {
    /// How many of `style_slots` and `style_overrides` are the markers'; the
    /// rest are the error bars'.
    fn marker_rows(&self) -> (usize, usize) {
        self.style_meta.map_or((0, 0), |meta| {
            (meta.style_count as usize, meta.override_count as usize)
        })
    }

    pub(in crate::renderer) fn descriptor(&self) -> crate::gpu_pick::GpuPickScatter<'_> {
        let Self {
            base_radius_px,
            base_shape_id,
            style_index_column,
            style_slots,
            style_overrides,
            style_meta,
            size_map,
        } = self;
        let (marker_slots, marker_overrides) = self.marker_rows();
        crate::gpu_pick::GpuPickScatter {
            base_radius_px: *base_radius_px,
            base_shape_id: *base_shape_id,
            style_map: style_meta.map(|style_meta| crate::gpu_pick::GpuPickScatterStyle {
                style_index_column: style_index_column.clone(),
                style_slots: &style_slots[..marker_slots],
                style_overrides: &style_overrides[..marker_overrides],
                style_meta,
            }),
            size_map: size_map.clone(),
        }
    }
}

impl OwnedGpuPickSeriesDescriptor {
    pub(in crate::renderer) fn descriptor(&self) -> crate::gpu_pick::GpuPickSeriesDescriptor<'_> {
        let Self {
            series_id,
            source_id,
            resource,
        } = self;
        let PickSeriesResource {
            x_column,
            y_column,
            scatter,
            line_width_px,
            line_interpolation,
            line_style,
            vector,
            errorbar,
            area,
            stem,
            line_closed,
            path_arrows,
            category_jitter,
            ternary_c,
            ternary_region,
        } = resource;
        crate::gpu_pick::GpuPickSeriesDescriptor {
            source_id: source_id.clone(),
            series_id: series_id.clone(),
            x_column: x_column.clone(),
            y_column: y_column.clone(),
            scatter: scatter.as_ref().map(OwnedGpuPickScatter::descriptor),
            line_width_px: *line_width_px,
            line_interpolation: *line_interpolation,
            line_style: *line_style,
            vector: vector.clone(),
            errorbar: errorbar.as_ref().map(|errorbar| {
                let OwnedGpuPickErrorBar {
                    y,
                    x,
                    stem_width_px,
                    cap_half_px,
                    cap_width_px,
                    paints,
                    style_map,
                } = errorbar;
                crate::gpu_pick::GpuPickErrorBar {
                    y: y.clone(),
                    x: x.clone(),
                    stem_width_px: *stem_width_px,
                    cap_half_px: *cap_half_px,
                    cap_width_px: *cap_width_px,
                    paints: *paints,
                    style_map: style_map.as_ref().map(|style_index_column| {
                        // The rows past the markers' in the scatter's rows.
                        let (slots, overrides) =
                            scatter.as_ref().map_or((&[][..], &[][..]), |scatter| {
                                let (marker_slots, marker_overrides) = scatter.marker_rows();
                                (
                                    &scatter.style_slots[marker_slots..],
                                    &scatter.style_overrides[marker_overrides..],
                                )
                            });
                        crate::gpu_pick::GpuPickErrorBarStyle {
                            style_index_column: style_index_column.clone(),
                            style_slots: slots,
                            style_overrides: overrides,
                        }
                    }),
                }
            }),
            area: area.clone(),
            stem: stem.clone(),
            line_closed: *line_closed,
            path_arrows: *path_arrows,
            category_jitter: *category_jitter,
            ternary_c: ternary_c.clone(),
            ternary_region: *ternary_region,
        }
    }
}

/// The pick slot of a box plot series' raw points (`vs_box_points`): its
/// points' columns as the descriptor's x and y (the category positions on
/// the category axis' side), its markers and its place in the box group of
/// `config`'s `series`, as `box_plot_params` places its boxes.
pub(super) fn box_points_pick_descriptor(
    config: &Config,
    series: &[SeriesConfig],
    item: &SeriesConfig,
    points: &crate::data_config::BoxPoints,
) -> OwnedGpuPickSeriesDescriptor {
    use crate::data_config::{BoxOutlierShape, CategoryBarDirection};
    let groups = config.bar_groups.clone().unwrap_or_default();
    let horizontal = groups.direction == CategoryBarDirection::Horizontal;
    let boxes = || series.iter().filter(|s| s.render_type.box_plot().is_some());
    let member = boxes()
        .position(|s| s.series_id == item.series_id)
        .unwrap_or(0);
    let members = boxes().count();
    let style = &points.style;
    let shape = match (style.shape, style.filled) {
        (BoxOutlierShape::Circle, false) => ScatterShape::Circle,
        (BoxOutlierShape::Circle, true) => ScatterShape::CircleFilled,
        (BoxOutlierShape::Square, false) => ScatterShape::Square,
        (BoxOutlierShape::Square, true) => ScatterShape::SquareFilled,
    };
    let (x_column, y_column) = if horizontal {
        (points.values.clone(), points.positions.clone())
    } else {
        (points.positions.clone(), points.values.clone())
    };
    OwnedGpuPickSeriesDescriptor {
        series_id: item.series_id.clone(),
        source_id: item.source_id.clone(),
        resource: PickSeriesResource {
            x_column,
            y_column,
            scatter: Some(OwnedGpuPickScatter {
                base_radius_px: style.size * 0.5,
                base_shape_id: data_render::shape_id(&shape),
                style_index_column: None,
                style_slots: Vec::new(),
                style_overrides: Vec::new(),
                style_meta: None,
                size_map: None,
            }),
            line_width_px: None,
            line_interpolation: LineInterpolation::Linear,
            line_style: LineStylePreset::Solid,
            vector: None,
            errorbar: None,
            area: None,
            stem: None,
            line_closed: false,
            path_arrows: None,
            category_jitter: Some(crate::gpu_pick::GpuPickCategoryJitter {
                member: member as u32,
                members: members as u32,
                horizontal,
                group_width: groups.group_width.clamp(0.0, 1.0),
                gap_px: groups.bar_gap.max(0.0),
                jitter: style.jitter,
                offset: style.offset,
                seed: crate::sketch::fnv1a(&item.series_id),
            }),
            ternary_c: None,
            ternary_region: false,
        },
    }
}
