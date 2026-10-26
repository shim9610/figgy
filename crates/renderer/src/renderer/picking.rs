//! GPU picking on registered charts: the pick request, the per-series
//! pick descriptors, the pick plan, the picker state machine and the
//! pick_chart entry points.

use super::*;

/// Host-space input for one renderer-owned exact GPU pick.
///
/// The renderer derives the transform and data-area clip from its authoritative
/// chart config. Hosts provide only the physical canvas position and the panel
/// placement used for the current display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuPickRequest {
    pub canvas_position_px: [f32; 2],
    /// Where the whole document ([`Config::document_area`]) is displayed, as
    /// [`fit_display_panel`] places it. Without subplots this is the
    /// displayed chart panel.
    pub display_panel_px: Rect,
    pub display_scale: f32,
    pub max_distance_px: f32,
}

/// The arrows the vector shader draws for a point vector series, in the
/// picker's terms: the same columns, scale mode, pivot and sizes
/// (`Renderer::ensure_vector_scratch`), in logical pixels.
pub(super) fn gpu_pick_vector(
    components: &crate::data_config::VectorComponents,
    style: &crate::data_config::VectorStyle,
) -> Option<crate::gpu_pick::GpuPickVector> {
    use crate::data_config::{
        VectorColorMap, VectorComponents, VectorPivot, VectorScale, VectorStyle,
    };
    use data_render::vector as vector_render;
    let VectorComponents::Points { u, v } = components else {
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
pub(super) struct OwnedGpuPickSeriesDescriptor {
    pub(super) series_id: String,
    pub(super) source_id: Option<String>,
    pub(super) resource: PickSeriesResource,
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
pub(super) struct PickSeriesResource {
    pub(super) x_column: ColumnId,
    pub(super) y_column: ColumnId,
    pub(super) scatter: Option<OwnedGpuPickScatter>,
    pub(super) line_width_px: Option<f32>,
    pub(super) line_interpolation: LineInterpolation,
    pub(super) line_style: LineStylePreset,
    pub(super) vector: Option<crate::gpu_pick::GpuPickVector>,
    /// The error bars drawn with the markers; their style rows follow the
    /// marker rows in `scatter`'s.
    pub(super) errorbar: Option<OwnedGpuPickErrorBar>,
    /// The line's area fill.
    pub(super) area: Option<crate::gpu_pick::GpuPickArea>,
    /// Whether the line is a closed polygon (a radar outline).
    pub(super) line_closed: bool,
}

/// A series' error bars in the plan (`GpuPickErrorBar` without its rows,
/// which live in the series' `OwnedGpuPickScatter` after the marker rows).
#[cfg_attr(test, derive(Clone, Debug))]
pub(super) struct OwnedGpuPickErrorBar {
    pub(super) y: Option<[ColumnId; 2]>,
    pub(super) x: Option<[ColumnId; 2]>,
    pub(super) stem_width_px: f32,
    pub(super) cap_half_px: f32,
    pub(super) cap_width_px: f32,
    pub(super) paints: bool,
    /// Precise with an error-bar style map: its index column. The map's rows
    /// are the scatter's rows past the marker rows.
    pub(super) style_map: Option<Option<ColumnId>>,
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
            line_closed,
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
            line_closed: other_line_closed,
        } = other;
        line_closed == other_line_closed
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
    pub(super) fn references_column(&self, id: &str) -> bool {
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
            line_closed: _,
        } = self;
        x_column == id
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

    pub(super) fn references_any_column(&self, ids: &[&str]) -> bool {
        ids.iter().any(|id| self.references_column(id))
    }
}

/// A point vector's arrows, compared field by field with floats by bits.
fn same_pick_vector(
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
fn same_pick_errorbar(errorbar: &OwnedGpuPickErrorBar, other: &OwnedGpuPickErrorBar) -> bool {
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
fn same_pick_area(
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
fn same_pick_size_map(
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
pub(super) struct OwnedGpuPickScatter {
    pub(super) base_radius_px: f32,
    pub(super) base_shape_id: u32,
    pub(super) style_index_column: Option<ColumnId>,
    /// The marker rows (`style_meta.style_count` of them), then the error
    /// bars' slot rows.
    pub(super) style_slots: Vec<data_render::ScatterStyleSlotGpu>,
    /// The marker overrides (`style_meta.override_count`), then the error
    /// bars' override rows.
    pub(super) style_overrides: Vec<data_render::ScatterStyleOverrideGpu>,
    pub(super) style_meta: Option<data_render::ScatterStyleMapMeta>,
    pub(super) size_map: Option<crate::gpu_pick::GpuPickSizeMap>,
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

    pub(super) fn descriptor(&self) -> crate::gpu_pick::GpuPickScatter<'_> {
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
    pub(super) fn descriptor(&self) -> crate::gpu_pick::GpuPickSeriesDescriptor<'_> {
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
            line_closed,
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
            line_closed: *line_closed,
        }
    }
}

pub(super) struct PickChartPlan {
    pub(super) descriptors: Vec<OwnedGpuPickSeriesDescriptor>,
}

/// The panel and series a pick addresses: the active layer's. Picking a
/// layered document reads the active layer's internal panel (its Y range and
/// the group data area) and only the series that layer scales; the base
/// layer answers while the annotation layer is active, which has no series.
/// A document without overlays is its own panel.
///
/// The line offset members the layer scales are drawn by panels of their
/// own (`Config::expand`). They follow the layer's other series, in stacking
/// order, and `member_transforms` holds the transform of each one's panel:
/// the pick tests each member where its panel draws it, from the top of the
/// stack first, and the layer's own panel last
/// (`GpuPickEngine::pick_with_arcs`).
pub(super) struct LayerPickInputs {
    pub(super) panel: Config,
    pub(super) series: Vec<SeriesConfig>,
    pub(super) member_transforms:
        [data_render::ScatterTransform; crate::data_config::MAX_OFFSET_LINES],
    pub(super) member_count: usize,
}

impl LayerPickInputs {
    /// The transforms of the members at the end of `series`.
    pub(super) fn members(&self) -> &[data_render::ScatterTransform] {
        &self.member_transforms[..self.member_count]
    }
}

pub(super) fn active_layer_pick_inputs(
    config: &Config,
    series: &[SeriesConfig],
) -> std::result::Result<LayerPickInputs, crate::gpu_pick::GpuPickError> {
    let member_transforms = [bytemuck::Zeroable::zeroed(); crate::data_config::MAX_OFFSET_LINES];
    if !is_layered(config) {
        return Ok(LayerPickInputs {
            panel: config.clone(),
            series: series.to_vec(),
            member_transforms,
            member_count: 0,
        });
    }
    let layer = match config.active_layer() {
        crate::LayerRef::Annotations => crate::LayerRef::Base,
        layer => layer,
    };
    let panels =
        expand_for_render(config).map_err(|_| crate::gpu_pick::GpuPickError::AllocationFailed {
            resource: "renderer pick layer expansion",
        })?;
    let index = panel_index(config, panels.len(), layer);
    let layers = config.panel_layers();
    let layer = layers[index];
    let mut subset: Vec<SeriesConfig> = series
        .iter()
        .filter(|item| series_panel_index(config, panels.len(), &item.series_id) == index)
        .cloned()
        .collect();
    let mut transforms = member_transforms;
    let mut member_count = 0;
    // The layer's members, in stacking order, each with its panel's
    // transform.
    for (panel_at, panel) in panels.iter().enumerate() {
        if layers.get(panel_at) != Some(&layer) {
            continue;
        }
        let Some(member) = config.panel_member(panel_at) else {
            continue;
        };
        let Some(item) = series.iter().find(|item| item.series_id == member) else {
            continue;
        };
        let Some(slot) = transforms.get_mut(member_count) else {
            break;
        };
        *slot = data_render::scatter_transform_from_config(panel);
        member_count += 1;
        subset.push(item.clone());
    }
    let panel = panels
        .into_iter()
        .nth(index)
        .expect("expansion yields one panel per layer");
    Ok(LayerPickInputs {
        panel,
        series: subset,
        member_transforms: transforms,
        member_count,
    })
}

impl PickChartPlan {
    /// The plan for the active layer of `config`; see
    /// [`active_layer_pick_inputs`].
    pub(super) fn for_active_layer(
        config: &Config,
        series: &[SeriesConfig],
    ) -> std::result::Result<Self, crate::gpu_pick::GpuPickError> {
        let inputs = active_layer_pick_inputs(config, series)?;
        Self::new(&inputs.panel, &inputs.series)
    }

    pub(super) fn new(
        config: &Config,
        series: &[SeriesConfig],
    ) -> std::result::Result<Self, crate::gpu_pick::GpuPickError> {
        let mut descriptors = Vec::new();
        descriptors.try_reserve_exact(series.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "renderer pick descriptor plan",
            }
        })?;

        let all_series = series;
        for series in series {
            let primitives = effective_series_primitives(&config.draw_style, &series.render_type);
            // A point vector is picked on the arrows it draws, through the
            // scatter primitive slot.
            let vector = extract_vector(&series.render_type)
                .filter(|_| primitives.vector)
                .and_then(|(components, style)| gpu_pick_vector(components, style));
            let vector_points = vector.is_some();
            if !primitives.line && !primitives.scatter && !vector_points {
                continue;
            }

            let scatter = if let Some((_, style)) =
                extract_vector(&series.render_type).filter(|_| vector_points)
            {
                // The marker slot carries the anchor's highlight radius; the
                // pick itself walks the drawn arrow (`GpuPickVector`).
                let base_radius_px = crate::data_render::vector::vector_anchor_radius_px(style);
                let base_shape_id = data_render::shape_id(&ScatterShape::CircleFilled);
                Some(OwnedGpuPickScatter {
                    base_radius_px,
                    base_shape_id,
                    style_index_column: None,
                    style_slots: Vec::new(),
                    style_overrides: Vec::new(),
                    style_meta: None,
                    size_map: None,
                })
            } else if primitives.scatter {
                let scatter = extract_scatter(&series.render_type)
                    .expect("effective scatter primitive has scatter configuration");
                let scatter = scatter.as_ref();
                let base_radius_px = scatter.point_size;
                let base_shape_id = data_render::shape_id(&scatter.point_shape);
                if config.draw_style.is_precise() {
                    let style_table = scatter.point_style_table.as_deref().unwrap_or(&[]);
                    let overrides = scatter.point_style_overrides.as_deref().unwrap_or(&[]);
                    // The error bars' rows follow the markers' in the same
                    // vectors (`errorbar_style_rows` below).
                    let (bar_table, bar_overrides) = extract_errorbar_style(&series.render_type)
                        .map_or((&[][..], &[][..]), |style| {
                            (
                                style.error_bar_style_table.as_deref().unwrap_or(&[]),
                                style.error_bar_style_overrides.as_deref().unwrap_or(&[]),
                            )
                        });
                    let mut style_slots = Vec::new();
                    style_slots
                        .try_reserve_exact(style_table.len() + bar_table.len())
                        .map_err(|_| crate::gpu_pick::GpuPickError::AllocationFailed {
                            resource: "renderer pick style slots",
                        })?;
                    style_slots.extend(style_table.iter().map(pick_style_row));

                    let mut style_overrides = Vec::new();
                    style_overrides
                        .try_reserve_exact(overrides.len() + bar_overrides.len())
                        .map_err(|_| crate::gpu_pick::GpuPickError::AllocationFailed {
                            resource: "renderer pick style overrides",
                        })?;
                    for override_ in overrides {
                        let Ok(point_index) = u32::try_from(override_.index) else {
                            continue;
                        };
                        let slot = pick_style_row(&override_.style);
                        style_overrides.push(data_render::ScatterStyleOverrideGpu {
                            point_index,
                            _pad: [0; 3],
                            color_premul: slot.color_premul,
                            meta: slot.meta,
                        });
                    }

                    let style_index_column = scatter.point_style_index_column.clone();
                    let has_index = style_index_column.is_some();
                    let style_meta =
                        (has_index || !style_slots.is_empty() || !style_overrides.is_empty())
                            .then_some(data_render::ScatterStyleMapMeta {
                                style_count: style_slots.len().min(u32::MAX as usize) as u32,
                                override_count: style_overrides.len().min(u32::MAX as usize) as u32,
                                has_index: u32::from(has_index),
                                _pad: 0,
                            });
                    errorbar_style_rows(
                        &series.render_type,
                        &mut style_slots,
                        &mut style_overrides,
                    );
                    let size_map = scatter.point_size_map.as_ref().map(|map| {
                        crate::gpu_pick::GpuPickSizeMap {
                            column: map.column.clone(),
                            domain_min: map.domain_min,
                            domain_max: map.domain_max,
                            min_px: map.min_px,
                            max_px: map.max_px,
                            area: matches!(map.scale, SizeScale::Area),
                        }
                    });
                    Some(OwnedGpuPickScatter {
                        base_radius_px,
                        base_shape_id,
                        style_index_column,
                        style_slots,
                        style_overrides,
                        style_meta,
                        size_map,
                    })
                } else {
                    Some(OwnedGpuPickScatter {
                        base_radius_px,
                        base_shape_id,
                        style_index_column: None,
                        style_slots: Vec::new(),
                        style_overrides: Vec::new(),
                        style_meta: None,
                        size_map: None,
                    })
                }
            } else {
                None
            };

            let line = primitives.line.then(|| {
                extract_line(&series.render_type)
                    .expect("effective line primitive has line configuration")
            });
            let line = line.as_deref();
            let line_width_px = line.map(|line| line.line_width);
            let line_interpolation =
                line.map_or(LineInterpolation::Linear, |line| line.interpolation);
            let line_style = line.map_or(LineStylePreset::Solid, |line| line.line_style);
            let errorbar = scatter
                .as_ref()
                .filter(|_| !vector_points)
                .and_then(|_| pick_errorbar(config, &series.render_type));
            // A radar outline is closed, and so is its fill on a full
            // circle, as `build_series_layers` draws them.
            let line_closed = line.is_some() && series.render_type.radar().is_some();
            let area = line.and_then(|line| {
                pick_area(
                    line,
                    series,
                    all_series,
                    line_closed && polar_fill_closes(config),
                )
            });
            descriptors.push(OwnedGpuPickSeriesDescriptor {
                series_id: series.series_id.clone(),
                source_id: series.source_id.clone(),
                resource: PickSeriesResource {
                    x_column: series.x_column.clone(),
                    y_column: series.y_column.clone(),
                    scatter,
                    line_width_px,
                    line_interpolation,
                    line_style,
                    vector,
                    errorbar,
                    area,
                    line_closed,
                },
            });
        }

        Ok(Self { descriptors })
    }

    pub(super) fn registry_slots<'a>(
        &'a self,
        current: Option<&[OwnedGpuPickSeriesDescriptor]>,
    ) -> std::result::Result<
        Vec<crate::gpu_pick::GpuPickRegistrySlot<'a>>,
        crate::gpu_pick::GpuPickError,
    > {
        self.registry_slots_rebuilding(current, &[])
    }

    pub(super) fn registry_slots_rebuilding<'a>(
        &'a self,
        current: Option<&[OwnedGpuPickSeriesDescriptor]>,
        rebuild_columns: &[&str],
    ) -> std::result::Result<
        Vec<crate::gpu_pick::GpuPickRegistrySlot<'a>>,
        crate::gpu_pick::GpuPickError,
    > {
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(self.descriptors.len())
            .map_err(|_| crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "renderer pick registry transition",
            })?;
        for descriptor in &self.descriptors {
            let reusable = current.and_then(|current| {
                current.iter().position(|slot| {
                    !descriptor.resource.references_any_column(rebuild_columns)
                        && slot.series_id == descriptor.series_id
                        && slot.resource == descriptor.resource
                })
            });
            if let Some(current_index) = reusable {
                slots.push(crate::gpu_pick::GpuPickRegistrySlot::Reuse {
                    current_index,
                    source_id: descriptor.source_id.clone(),
                    series_id: descriptor.series_id.clone(),
                });
            } else {
                slots.push(crate::gpu_pick::GpuPickRegistrySlot::Build(
                    descriptor.descriptor(),
                ));
            }
        }
        Ok(slots)
    }
}

pub(super) enum PickerPipelineState {
    Disabled,
    Ready {
        point: Arc<crate::gpu_pick::PickPipelineBundle>,
        data: Arc<crate::gpu_data_pick::DataPickPipelineBundle>,
    },
    Failed(crate::gpu_pick::GpuPickError),
}

pub(super) struct ActivePickRegistry {
    pub(super) chart_id: ChartId,
    /// The plan's descriptors the registry was built from, one per slot in
    /// slot order: the next plan reuses a slot whose descriptor it equals.
    pub(super) descriptors: Vec<OwnedGpuPickSeriesDescriptor>,
    pub(super) engine: crate::gpu_pick::GpuPickEngine,
}

impl ActivePickRegistry {
    pub(super) fn references_any_column(&self, ids: &[&str]) -> bool {
        self.descriptors
            .iter()
            .any(|descriptor| descriptor.resource.references_any_column(ids))
    }
}

pub(super) enum PreparedActivePickerMutation<'a> {
    Clear(&'a mut Option<ActivePickRegistry>),
    Update {
        transition: crate::gpu_pick::PreparedPickRegistryTransition<'a>,
        active_chart_id: &'a mut ChartId,
        active_descriptors: &'a mut Vec<OwnedGpuPickSeriesDescriptor>,
        next_chart_id: ChartId,
        next_descriptors: Vec<OwnedGpuPickSeriesDescriptor>,
    },
}

impl PreparedActivePickerMutation<'_> {
    pub(super) fn commit(self) {
        match self {
            Self::Clear(active) => *active = None,
            Self::Update {
                transition,
                active_chart_id,
                active_descriptors,
                next_chart_id,
                next_descriptors,
            } => {
                transition.commit();
                *active_chart_id = next_chart_id;
                *active_descriptors = next_descriptors;
            }
        }
    }
}

pub(super) struct RendererPicker {
    pub(super) pipeline_state: PickerPipelineState,
    pub(super) active: Option<ActivePickRegistry>,
    /// The exact pipelines created so far, by module and variant: compiled on
    /// the first pick that needs each (`PickExactPipelines`).
    pub(super) exact: crate::gpu_pick::PickExactPipelines,
    #[cfg(test)]
    pub(super) injected_activation_failure: Option<crate::gpu_pick::GpuPickError>,
}

impl RendererPicker {
    pub(super) fn disabled() -> Self {
        Self {
            pipeline_state: PickerPipelineState::Disabled,
            active: None,
            exact: crate::gpu_pick::PickExactPipelines::default(),
            #[cfg(test)]
            injected_activation_failure: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn enable_observed(
        &mut self,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        gpu_ledger: Arc<GpuLedger>,
        shaders: crate::gpu_data_pick::DataPickShaders<'_>,
        transform_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        bar_style_bgl: &wgpu::BindGroupLayout,
        vector_bgl: &wgpu::BindGroupLayout,
        category_bgl: &wgpu::BindGroupLayout,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        match &self.pipeline_state {
            PickerPipelineState::Ready { .. } => return Ok(()),
            PickerPipelineState::Failed(error) => return Err(error.clone()),
            PickerPipelineState::Disabled => {}
        }

        #[cfg(test)]
        if let Some(error) = self.injected_activation_failure.take() {
            self.pipeline_state = PickerPipelineState::Failed(error.clone());
            return Err(error);
        }

        match crate::gpu_pick::PickPipelineBundle::new_observed(
            Arc::clone(&device),
            Arc::clone(&queue),
            Arc::clone(&gpu_ledger),
            observer,
        ) {
            Ok(point) => {
                let data = crate::gpu_data_pick::DataPickPipelineBundle::new_observed(
                    device,
                    queue,
                    gpu_ledger,
                    shaders,
                    transform_bgl,
                    field_bgl,
                    bar_style_bgl,
                    vector_bgl,
                    category_bgl,
                    observer,
                );
                self.pipeline_state = PickerPipelineState::Ready { point, data };
                Ok(())
            }
            Err(error) => {
                self.pipeline_state = PickerPipelineState::Failed(error.clone());
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn enable_observed_async(
        &mut self,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        gpu_ledger: Arc<GpuLedger>,
        shaders: crate::gpu_data_pick::DataPickShaders<'_>,
        transform_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        bar_style_bgl: &wgpu::BindGroupLayout,
        vector_bgl: &wgpu::BindGroupLayout,
        category_bgl: &wgpu::BindGroupLayout,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        match &self.pipeline_state {
            PickerPipelineState::Ready { .. } => return Ok(()),
            PickerPipelineState::Failed(error) => return Err(error.clone()),
            PickerPipelineState::Disabled => {}
        }

        #[cfg(test)]
        if let Some(error) = self.injected_activation_failure.take() {
            self.pipeline_state = PickerPipelineState::Failed(error.clone());
            return Err(error);
        }

        match crate::gpu_pick::PickPipelineBundle::new_observed_async(
            Arc::clone(&device),
            Arc::clone(&queue),
            Arc::clone(&gpu_ledger),
            observer,
        )
        .await
        {
            Ok(point) => {
                match crate::gpu_data_pick::DataPickPipelineBundle::new_observed_async(
                    device,
                    queue,
                    gpu_ledger,
                    shaders,
                    transform_bgl,
                    field_bgl,
                    bar_style_bgl,
                    vector_bgl,
                    category_bgl,
                    observer,
                )
                .await
                {
                    Ok(data) => {
                        self.pipeline_state = PickerPipelineState::Ready { point, data };
                        Ok(())
                    }
                    Err(error) => {
                        self.pipeline_state = PickerPipelineState::Failed(error.clone());
                        Err(error)
                    }
                }
            }
            Err(error) => {
                self.pipeline_state = PickerPipelineState::Failed(error.clone());
                Err(error)
            }
        }
    }

    pub(super) fn ready_bundle(
        &self,
    ) -> std::result::Result<Arc<crate::gpu_pick::PickPipelineBundle>, crate::gpu_pick::GpuPickError>
    {
        match &self.pipeline_state {
            PickerPipelineState::Disabled => Err(crate::gpu_pick::GpuPickError::PickerDisabled),
            PickerPipelineState::Ready { point, .. } => Ok(Arc::clone(point)),
            PickerPipelineState::Failed(error) => Err(error.clone()),
        }
    }

    /// The exact pipelines, shared by resident and streamed picks: a
    /// streamed chunk scans with its resident series' variant.
    pub(super) fn exact_pipelines(&mut self) -> &mut crate::gpu_pick::PickExactPipelines {
        &mut self.exact
    }

    pub(super) fn ready_data_bundle(
        &self,
    ) -> std::result::Result<
        Arc<crate::gpu_data_pick::DataPickPipelineBundle>,
        crate::gpu_pick::GpuPickError,
    > {
        match &self.pipeline_state {
            PickerPipelineState::Disabled => Err(crate::gpu_pick::GpuPickError::PickerDisabled),
            PickerPipelineState::Ready { data, .. } => Ok(Arc::clone(data)),
            PickerPipelineState::Failed(error) => Err(error.clone()),
        }
    }

    #[cfg(test)]
    pub(super) fn fail_activation_once(&mut self, error: crate::gpu_pick::GpuPickError) {
        self.injected_activation_failure = Some(error);
    }

    pub(super) fn prepare_chart(
        &mut self,
        pool: &ColumnPool,
        chart_id: ChartId,
        config: &Config,
        series: &[SeriesConfig],
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let bundle = self.ready_bundle()?;
        let plan = PickChartPlan::for_active_layer(config, series)?;
        if self.active.as_ref().is_some_and(|active| {
            active.chart_id == chart_id && active.descriptors == plan.descriptors
        }) {
            return Ok(());
        }

        if let Some(active) = self.active.as_mut() {
            let slots = plan.registry_slots(Some(&active.descriptors))?;
            active
                .engine
                .prepare_registry_transition(pool, slots)?
                .commit();
            active.chart_id = chart_id;
            active.descriptors = plan.descriptors;
        } else {
            let mut engine = crate::gpu_pick::GpuPickEngine::from_bundle(bundle)?;
            let slots = plan.registry_slots(None)?;
            engine.prepare_registry_transition(pool, slots)?.commit();
            self.active = Some(ActivePickRegistry {
                chart_id,
                descriptors: plan.descriptors,
                engine,
            });
        }
        Ok(())
    }

    pub(super) fn prepare_active_chart_mutation<'picker>(
        &'picker mut self,
        pool: &ColumnPool,
        chart_id: ChartId,
        config: &Config,
        series: &[SeriesConfig],
    ) -> std::result::Result<
        Option<PreparedActivePickerMutation<'picker>>,
        crate::gpu_pick::GpuPickError,
    > {
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.chart_id == chart_id)
        {
            let mut nonresident = false;
            for item in series {
                visit_series_columns(item, &mut |id| nonresident |= pool.slot(id).is_none());
            }
            if nonresident {
                return Ok(Some(PreparedActivePickerMutation::Clear(&mut self.active)));
            }
        }
        let plan = PickChartPlan::for_active_layer(config, series)?;
        self.prepare_active_plan(pool, chart_id, plan, &[], false)
    }

    pub(super) fn prepare_active_pool_mutation<'picker>(
        &'picker mut self,
        pool: &ColumnPool,
        chart_id: ChartId,
        plan: PickChartPlan,
        rebuild_columns: &[&str],
    ) -> std::result::Result<
        Option<PreparedActivePickerMutation<'picker>>,
        crate::gpu_pick::GpuPickError,
    > {
        self.prepare_active_plan(pool, chart_id, plan, rebuild_columns, true)
    }

    pub(super) fn prepare_active_plan<'picker>(
        &'picker mut self,
        pool: &ColumnPool,
        chart_id: ChartId,
        plan: PickChartPlan,
        rebuild_columns: &[&str],
        force: bool,
    ) -> std::result::Result<
        Option<PreparedActivePickerMutation<'picker>>,
        crate::gpu_pick::GpuPickError,
    > {
        let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.chart_id == chart_id)
        else {
            return Ok(None);
        };
        if !force && active.descriptors == plan.descriptors {
            return Ok(None);
        }

        let ActivePickRegistry {
            chart_id: active_chart_id,
            descriptors: active_descriptors,
            engine,
        } = active;
        let slots = plan.registry_slots_rebuilding(Some(active_descriptors), rebuild_columns)?;
        let transition = engine.prepare_registry_transition(pool, slots)?;
        Ok(Some(PreparedActivePickerMutation::Update {
            transition,
            active_chart_id,
            active_descriptors,
            next_chart_id: chart_id,
            next_descriptors: plan.descriptors,
        }))
    }
}

/// The error bars a series draws with its markers under `config`'s draw
/// style (`prepare/layers.rs`): every style that draws the markers but
/// Constellation, which drops error-bar series. The sizes are the logical
/// style's; whether the colour paints is the precise and sketch rule (alpha
/// above zero), while a Milkyway jet's white core paints at any alpha.
fn pick_errorbar(config: &Config, render_type: &DataRenderType) -> Option<OwnedGpuPickErrorBar> {
    let style = extract_errorbar_style(render_type)?;
    if matches!(config.draw_style, DrawStyle::Constellation(_)) {
        return None;
    }
    let columns = |error: &ErrorRef| match error {
        ErrorRef::Symmetric { column } => [column.clone(), column.clone()],
        ErrorRef::Asymmetric { lower, upper } => [lower.clone(), upper.clone()],
    };
    let y = extract_err_y(render_type).map(columns);
    let x = extract_err_x(render_type).map(columns);
    if y.is_none() && x.is_none() {
        return None;
    }
    let DataErrorBarStyleConfig {
        error_bar_color,
        error_bar_width,
        error_bar_cap_size,
        cap_width,
        error_bar_style_table,
        error_bar_style_index_column,
        error_bar_style_overrides,
    } = style;
    // The precise draw maps per-point styles exactly when the renderer builds
    // an error-bar style map (`create_errorbar_style_map_for`).
    let mapped = config.draw_style.is_precise()
        && (error_bar_style_index_column.is_some()
            || error_bar_style_table
                .as_ref()
                .is_some_and(|table| !table.is_empty())
            || error_bar_style_overrides
                .as_ref()
                .is_some_and(|overrides| !overrides.is_empty()));
    Some(OwnedGpuPickErrorBar {
        y,
        x,
        stem_width_px: *error_bar_width,
        cap_half_px: *error_bar_cap_size,
        cap_width_px: *cap_width,
        paints: matches!(config.draw_style, DrawStyle::Milkyway(_)) || error_bar_color.a > 0.0,
        style_map: mapped.then(|| error_bar_style_index_column.clone()),
    })
}

/// Appends a series' error-bar style rows after the marker rows, in logical
/// pixels. The pick reads no colour, only whether a row's colour paints:
/// `color_premul.w` is 1 where the row sets a colour with alpha above zero.
fn errorbar_style_rows(
    render_type: &DataRenderType,
    slots: &mut Vec<data_render::ScatterStyleSlotGpu>,
    overrides: &mut Vec<data_render::ScatterStyleOverrideGpu>,
) {
    let Some(style) = extract_errorbar_style(render_type) else {
        return;
    };
    let row = |slot: &DataErrorBarPointStyleConfig| {
        // The draw's mask bits (`create_errorbar_style_map_for`).
        const MASK_COLOR: u32 = 1;
        const MASK_STEM_WIDTH: u32 = 2;
        const MASK_CAP_HALF: u32 = 4;
        const MASK_CAP_WIDTH: u32 = 8;
        let row = errorbar_style_slot_gpu(
            slot,
            1.0,
            MASK_COLOR,
            MASK_STEM_WIDTH,
            MASK_CAP_HALF,
            MASK_CAP_WIDTH,
        );
        data_render::ScatterStyleSlotGpu {
            color_premul: [
                0.0,
                0.0,
                0.0,
                if row.color_premul[3] > 0.0 { 1.0 } else { 0.0 },
            ],
            meta: row.meta,
        }
    };
    slots.extend(
        style
            .error_bar_style_table
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(row),
    );
    for override_ in style.error_bar_style_overrides.as_deref().unwrap_or(&[]) {
        let Ok(point_index) = u32::try_from(override_.index) else {
            continue;
        };
        let slot = row(&override_.style);
        overrides.push(data_render::ScatterStyleOverrideGpu {
            point_index,
            _pad: [0; 3],
            color_premul: slot.color_premul,
            meta: slot.meta,
        });
    }
}

/// A line's fill as `prepare/layers.rs` draws it: to a baseline, or to the
/// y column of the series it names in the same series list with the same x
/// column (otherwise the draw rejects the chart and nothing is picked).
/// `paints` is whether the fill colour's alpha is above zero.
fn pick_area(
    line: &DataLineStyleConfig,
    series: &SeriesConfig,
    all_series: &[SeriesConfig],
    closed: bool,
) -> Option<crate::gpu_pick::GpuPickArea> {
    let paints = line.effective_fill_color().a > 0.0;
    match line.fill.as_ref()? {
        LineFill::ToBaseline(baseline) => Some(crate::gpu_pick::GpuPickArea {
            to_series: None,
            baseline: *baseline,
            paints,
            closed,
        }),
        LineFill::ToSeries(target) => {
            let target = all_series
                .iter()
                .find(|other| other.series_id == *target)
                .filter(|other| other.x_column == series.x_column)?;
            Some(crate::gpu_pick::GpuPickArea {
                to_series: Some(target.y_column.clone()),
                baseline: 0.0,
                paints,
                closed,
            })
        }
    }
}

/// A style table row or override as the pick reads it: its radius and
/// shape at scale 1. The pick never reads colour, so the colour lanes and
/// the colour mask bit stay zero and a colour-only edit leaves the row's
/// bytes, and with them the slot's resource, equal.
fn pick_style_row(style: &DataScatterPointStyleConfig) -> data_render::ScatterStyleSlotGpu {
    let mut row = scatter_style_slot_gpu(
        style,
        1.0,
        0,
        data_render::STYLE_MASK_RADIUS,
        data_render::STYLE_MASK_SHAPE,
    );
    row.color_premul = [0.0; 4];
    row
}

impl Renderer {
    /// The category bar group 3 series `cfg`'s value labels are placed over
    /// at the pick's display transform: the bar or box group's params and
    /// style map (as the draw resolves them), or a radar series' category
    /// and value columns. `None` for a series whose bars, boxes or polygon
    /// are not drawn.
    fn value_label_pick_group(
        &mut self,
        config: &Config,
        display_config: &Config,
        series: &[SeriesConfig],
        cfg: &SeriesConfig,
        scale: f32,
    ) -> Result<Option<data_render::CategoryBarData>> {
        let primitives = effective_series_primitives(&config.draw_style, &cfg.render_type);
        let lookup = |pool: &crate::data_render::ColumnPool, id: &str| {
            pool.handle_for(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })
        };
        if primitives.category_bars
            && let Some(bars) = cfg.render_type.category_bars()
        {
            let params = category_bar_params(
                display_config,
                series.iter().filter(|s| {
                    effective_series_primitives(&config.draw_style, &s.render_type).category_bars
                }),
                cfg,
                scale,
                |id| lookup(&self.pool, id),
                |id| self.pool.slot(id).map(|slot| slot.min),
            )?;
            let style_map = crate::renderer::style::category_bar_style_map(
                &self.device,
                &self.per_point_style_map_bgl,
                category_bar_categories(display_config).unwrap_or(&[]),
                bars,
                scale,
            );
            return Ok(Some(data_render::create_category_bar_data_bind_group(
                &self.device,
                &self.gpu_ledger,
                &self.category_bar_data_bgl,
                self.pool.buffer(),
                &params,
                style_map,
            )));
        }
        if primitives.box_plot
            && let Some(boxes) = cfg.render_type.box_plot()
        {
            let (params, _) = box_plot_params(
                display_config,
                series.iter().filter(|s| {
                    effective_series_primitives(&config.draw_style, &s.render_type).box_plot
                }),
                cfg,
                scale,
                &self.pool,
                |id| lookup(&self.pool, id),
            )?;
            let style_map = crate::renderer::style::box_plot_style_map(
                &self.device,
                &self.per_point_style_map_bgl,
                category_bar_categories(display_config).unwrap_or(&[]),
                boxes,
                scale,
            );
            return Ok(Some(data_render::create_category_bar_data_bind_group(
                &self.device,
                &self.gpu_ledger,
                &self.category_bar_data_bgl,
                self.pool.buffer(),
                &params,
                style_map,
            )));
        }
        if primitives.line && cfg.render_type.radar().is_some() {
            let pool_buffer = self.pool.buffer().clone();
            let (_, preparation) = self.preparation_parts();
            let mut lookup = |id: &str| lookup(preparation.pool, id);
            return preparation
                .radar_label_group(cfg, &pool_buffer, scale, &mut lookup)
                .map(Some);
        }
        Ok(None)
    }

    /// Enable exact GPU picking for this renderer.
    ///
    /// This is idempotent. The immutable compute pipeline bundle is created
    /// only by the first successful call; chart registries remain lazy.
    pub fn enable_gpu_picking(&mut self) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let mut noop = |_| {};
        self.enable_gpu_picking_observed(&mut noop)
    }

    /// Enable exact GPU picking while reporting timestamp-free pipeline stages.
    pub fn enable_gpu_picking_observed(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.picker.enable_observed(
            Arc::clone(&self.device),
            Arc::clone(&self.queue),
            Arc::clone(&self.gpu_ledger),
            crate::gpu_data_pick::DataPickShaders {
                bar: &self.pipelines.shaders.bar,
                field: &self.pipelines.shaders.field,
                vector_grid: &self.pipelines.shaders.vector_grid,
            },
            &self.transform_bgl,
            &self.field_bgl,
            &self.per_point_style_map_bgl,
            &self.vector_bgl,
            &self.category_bar_data_bgl,
            observer,
        )
    }

    pub async fn enable_gpu_picking_async(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let mut noop = |_| {};
        self.enable_gpu_picking_observed_async(&mut noop).await
    }

    pub async fn enable_gpu_picking_observed_async(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        self.picker
            .enable_observed_async(
                Arc::clone(&self.device),
                Arc::clone(&self.queue),
                Arc::clone(&self.gpu_ledger),
                crate::gpu_data_pick::DataPickShaders {
                    bar: &self.pipelines.shaders.bar,
                    field: &self.pipelines.shaders.field,
                    vector_grid: &self.pipelines.shaders.vector_grid,
                },
                &self.transform_bgl,
                &self.field_bgl,
                &self.per_point_style_map_bgl,
                &self.vector_bgl,
                &self.category_bar_data_bgl,
                observer,
            )
            .await
    }

    /// Prepare the renderer-owned picker cache for one chart.
    pub fn prepare_gpu_picking_for_chart(
        &mut self,
        id: ChartId,
    ) -> std::result::Result<(), crate::gpu_pick::GpuPickError> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?;
        for series in &state.series {
            if let Err(FiggyError::ColumnNotResident { id }) = self.require_resident_series(series)
            {
                return Err(crate::gpu_pick::GpuPickError::ColumnNotResident(id));
            }
        }
        self.picker
            .prepare_chart(&self.pool, id, &state.config, &state.series)
    }

    /// Submit one exact pick against renderer-owned chart state.
    pub fn pick_chart(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> std::result::Result<crate::gpu_pick::GpuPickTicket, crate::gpu_pick::GpuPickError> {
        self.prepare_gpu_picking_for_chart(id)?;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(crate::gpu_pick::GpuPickError::UnknownChart(id))?;
        let displayed = place_document(
            &state.config,
            request.display_scale,
            request.display_panel_px,
        );
        let inputs = active_layer_pick_inputs(&displayed, &state.series)?;
        let (display_config, layer_series) = (&inputs.panel, &inputs.series);
        let transform = data_render::scatter_transform_from_config(display_config);
        // The line offset members are the last registered series, each
        // drawn (and its arc prefix scanned) at its own panel's transform.
        let members = inputs.members();
        let first_member = layer_series.len() - members.len();
        // The arc prefix each registered line was drawn with at this
        // transform, by registry position: its dash phase and sketch wobble
        // walk it. The draw's own cache key, so a pick after a frame reuses
        // the frame's scan and a pick before one runs the same scan.
        let slots = self
            .picker
            .active
            .as_ref()
            .expect("successful chart preparation publishes an active picker registry")
            .engine
            .series_identities();
        let mut arcs: Vec<Option<Arc<wgpu::Buffer>>> = Vec::new();
        arcs.try_reserve_exact(slots.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "renderer pick arc prefixes",
            }
        })?;
        for series_id in slots.series_ids() {
            let position = layer_series
                .iter()
                .position(|series| series.series_id == series_id);
            let series_transform = position
                .and_then(|position| position.checked_sub(first_member))
                .map_or(&transform, |member| &members[member]);
            let series = position.map(|position| &layer_series[position]);
            let arc = series
                .filter(|series| line_pick_walks_arc(&display_config.draw_style, series))
                .and_then(|series| {
                    let interpolation = extract_line(&series.render_type)
                        .map_or(LineInterpolation::Linear, |line| line.interpolation);
                    self.preparation_parts().1.ensure_arc_prefix(
                        &series.series_id,
                        &series.x_column,
                        &series.y_column,
                        series_transform,
                        None,
                        interpolation,
                    )
                })
                .map(|arc| arc.prefix.0);
            arcs.push(arc);
        }
        let chart_rect = display_config.chart_area.0;
        let data_area_px = display_config.data_area().ok().map(|area| {
            let rect = area.0;
            [
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
            ]
        });
        let query = crate::gpu_pick::GpuPickQuery {
            transform,
            chart_rect_px: [
                chart_rect.x as f32,
                chart_rect.y as f32,
                chart_rect.width as f32,
                chart_rect.height as f32,
            ],
            data_area_px,
            canvas_position_px: request.canvas_position_px,
            max_distance_px: request.max_distance_px,
            draw_style: crate::gpu_pick::GpuPickDrawStyle::of(&display_config.draw_style),
        };
        let picker = &mut self.picker;
        let active = picker
            .active
            .as_ref()
            .expect("successful chart preparation publishes an active picker registry");
        debug_assert_eq!(active.chart_id, id);
        // The exact pipelines these series need, compiled on the first pick
        // that needs each.
        for key in active.engine.exact_variants(&query) {
            picker.exact.ensure(active.engine.bundle(), key)?;
        }
        active.engine.pick_with_arcs(
            &self.pool,
            query,
            request.display_scale,
            &arcs,
            members,
            &picker.exact,
        )
    }

    /// Submit one typed data pick across points/lines, histogram bins, heatmap
    /// cells, and contour levels.
    ///
    /// Source values stay in `ColumnPool`. The CPU supplies only current
    /// render metadata (lane bases, counts, styles and identity indices), while
    /// the bar/field render shaders themselves compute the exact hit geometry.
    pub fn pick_chart_data(
        &mut self,
        id: ChartId,
        request: GpuPickRequest,
    ) -> Result<crate::GpuDataPickTicket> {
        let (config, series, display_config, members) = {
            let state = self
                .chart_states
                .get(&id)
                .ok_or(FiggyError::UnknownChart { id })?;
            for series in &state.series {
                self.require_resident_series(series)?;
            }
            // The typed query answers for the active layer only, with that
            // layer's panel (its Y range) and the series it scales, so its
            // paint orders agree with the point query's registry. The query
            // geometry is the panel as displayed: the document is scaled and
            // placed first and expanded after, exactly as it is drawn.
            let inputs = active_layer_pick_inputs(&state.config, &state.series)?;
            let displayed = place_document(
                &state.config,
                request.display_scale,
                request.display_panel_px,
            );
            let display_config = active_layer_pick_inputs(&displayed, &state.series)?.panel;
            let members = inputs.member_count;
            (inputs.panel, inputs.series, display_config, members)
        };

        // Submit the established point/line query first. It stays point-only,
        // so the compatibility API and its registry lifecycle are unchanged.
        let point_ticket = self.pick_chart(id, request)?;
        let data_bundle = self.picker.ready_data_bundle()?;

        let mut data_series = Vec::new();
        // One entry per series and one per series' value labels.
        data_series
            .try_reserve_exact(series.len().saturating_mul(2))
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "typed data pick series",
                reason: error.to_string(),
            })?;
        let mut point_orders = Vec::new();
        point_orders
            .try_reserve_exact(series.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "typed point pick paint order",
                reason: error.to_string(),
            })?;

        for (index, cfg) in series.iter().enumerate() {
            let paint_order =
                u32::try_from(index).map_err(|_| FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    reason: "series paint order exceeds the GPU u32 index space".into(),
                })?;
            point_orders.push(crate::gpu_data_pick::PointPaintOrder {
                source_id: cfg.source_id.clone(),
                series_id: cfg.series_id.clone(),
                paint_order,
            });

            let primitives = effective_series_primitives(&config.draw_style, &cfg.render_type);
            if primitives.bar {
                let bar = extract_bar(&cfg.render_type)
                    .expect("effective histogram primitive has a bar config");
                let (edges_id, values_id, horizontal) = match bar.orientation {
                    crate::data_config::BarOrientation::Vertical => {
                        (&cfg.x_column, &cfg.y_column, false)
                    }
                    crate::data_config::BarOrientation::Horizontal => {
                        (&cfg.y_column, &cfg.x_column, true)
                    }
                };
                let edges =
                    self.pool
                        .handle_for(edges_id)
                        .ok_or_else(|| FiggyError::UnknownColumn {
                            id: edges_id.clone(),
                        })?;
                let values =
                    self.pool
                        .handle_for(values_id)
                        .ok_or_else(|| FiggyError::UnknownColumn {
                            id: values_id.clone(),
                        })?;
                let bin_count = edges.len_values.saturating_sub(1).min(values.len_values);
                if bin_count != 0 {
                    let bin_count =
                        u32::try_from(bin_count).map_err(|_| FiggyError::InvalidSeriesConfig {
                            series_id: cfg.series_id.clone(),
                            reason: "histogram bin count exceeds the GPU u32 index space".into(),
                        })?;
                    let (baseline_hi, baseline_lo) =
                        crate::data::split_f64_to_f32_pair(bar.baseline);
                    let style_map = self
                        .create_bar_style_map_for(bar, request.display_scale)
                        .map(|map| map.bind_group);
                    data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                        source_id: cfg.source_id.clone(),
                        series_id: cfg.series_id.clone(),
                        paint_order,
                        geometry: crate::gpu_data_pick::GpuDataPickGeometry::Histogram {
                            edges,
                            values,
                            bin_count,
                            baseline: [baseline_hi, baseline_lo],
                            gap_px: bar.gap_px.max(0.0) * request.display_scale,
                            width_ratio: data_render::sanitize_bar_width_ratio(bar.width_ratio),
                            horizontal,
                            style_map,
                        },
                    });
                }
                continue;
            }

            if primitives.box_plot {
                // The boxes the box plot entries draw at the pick's display
                // transform, from the same params and style map as the draw.
                let boxes = cfg
                    .render_type
                    .box_plot()
                    .expect("the box plot primitive has a box plot config");
                let (params, _) = box_plot_params(
                    &display_config,
                    series.iter().filter(|s| {
                        effective_series_primitives(&config.draw_style, &s.render_type).box_plot
                    }),
                    cfg,
                    request.display_scale,
                    &self.pool,
                    |id| {
                        self.pool
                            .handle_for(id)
                            .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })
                    },
                )?;
                if params.count != 0 {
                    let style_map = crate::renderer::style::box_plot_style_map(
                        &self.device,
                        &self.per_point_style_map_bgl,
                        category_bar_categories(&display_config).unwrap_or(&[]),
                        boxes,
                        request.display_scale,
                    );
                    data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                        source_id: cfg.source_id.clone(),
                        series_id: cfg.series_id.clone(),
                        paint_order,
                        geometry: crate::gpu_data_pick::GpuDataPickGeometry::BoxPlot {
                            data: data_render::create_category_bar_data_bind_group(
                                &self.device,
                                &self.gpu_ledger,
                                &self.category_bar_data_bgl,
                                self.pool.buffer(),
                                &params,
                                style_map,
                            ),
                        },
                    });
                }
                continue;
            }

            if primitives.category_bars {
                // The bars the category bar module draws at the pick's
                // display transform, resolved like the draw's.
                let bars = cfg
                    .render_type
                    .category_bars()
                    .expect("the category bar primitive has a category bar config");
                let params = category_bar_params(
                    &display_config,
                    series.iter().filter(|s| {
                        effective_series_primitives(&config.draw_style, &s.render_type)
                            .category_bars
                    }),
                    cfg,
                    request.display_scale,
                    |id| {
                        self.pool
                            .handle_for(id)
                            .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })
                    },
                    |id| self.pool.slot(id).map(|slot| slot.min),
                )?;
                if params.count != 0 {
                    let style_map = crate::renderer::style::category_bar_style_map(
                        &self.device,
                        &self.per_point_style_map_bgl,
                        category_bar_categories(&display_config).unwrap_or(&[]),
                        bars,
                        request.display_scale,
                    );
                    data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                        source_id: cfg.source_id.clone(),
                        series_id: cfg.series_id.clone(),
                        paint_order,
                        geometry: crate::gpu_data_pick::GpuDataPickGeometry::CategoryBars {
                            data: data_render::create_category_bar_data_bind_group(
                                &self.device,
                                &self.gpu_ledger,
                                &self.category_bar_data_bgl,
                                self.pool.buffer(),
                                &params,
                                style_map,
                            ),
                        },
                    });
                }
                continue;
            }

            if primitives.vector {
                // A grid vector picks the arrows it draws, as the cells they
                // are drawn for: the vector shader's own pick entry walks
                // them with the draw's group 2 at the pick's display scale.
                // A point vector is a point pick (`pick_chart`).
                self.ensure_vector_scratch(&config, cfg, request.display_scale)?;
                let scratch = self
                    .vector_cache
                    .get(cfg.series_id.as_str())
                    .expect("vector scratch was just ensured");
                if scratch.input == data_render::vector::VectorInput::Grid && scratch.count != 0 {
                    data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                        source_id: cfg.source_id.clone(),
                        series_id: cfg.series_id.clone(),
                        paint_order,
                        geometry: crate::gpu_data_pick::GpuDataPickGeometry::VectorGrid {
                            vector_bg: scratch.vector_bg.clone(),
                            charge: Arc::clone(&scratch.charge),
                        },
                    });
                }
                continue;
            }
            if !(primitives.field || primitives.contour) {
                continue;
            }
            let has_fill = primitives.field;
            let has_contour = primitives.contour;
            let mut nan_color_premul = [0.0; 4];
            let (field_bg, charge, drawable) = if has_fill {
                self.ensure_field_scratch(&config, cfg)?;
                let scratch = self
                    .field_cache
                    .get(cfg.series_id.as_str())
                    .expect("field scratch was just ensured");
                // The fill's `nan_color`, premultiplied as its style uniform
                // holds it (`PrimitiveStyle::from_color`).
                let [r, g, b, a] = scratch.signature.nan_color.map(f32::from_bits);
                nan_color_premul = PrimitiveStyle::from_color(Color { r, g, b, a }).color_premul;
                (
                    scratch.field_bg.clone(),
                    Arc::clone(&scratch.charge),
                    scratch.drawable,
                )
            } else {
                self.ensure_contour_scratch(cfg, &config, request.display_scale)?;
                let scratch = self
                    .contour_cache
                    .get(cfg.series_id.as_str())
                    .and_then(ScratchEntry::contour)
                    .expect("contour scratch was just ensured");
                (
                    scratch.field_bg.clone(),
                    Arc::clone(&scratch.charge),
                    scratch.drawable,
                )
            };
            if !drawable {
                continue;
            }
            let contour_width_px = extract_contour(&cfg.render_type)
                .map_or(0.0, |contour| contour.line.line_width.max(0.0))
                * request.display_scale;
            // The label placement the contour draws its gaps from at this
            // display transform: the draw's own snapshot (reused after a frame
            // at the same transform, dispatched once otherwise).
            let label = if has_contour && series_wants_contour_labels(&cfg.render_type) {
                if has_fill {
                    self.ensure_contour_scratch(cfg, &config, request.display_scale)?;
                }
                self.preparation_parts().1.contour_pick_label(
                    cfg,
                    &display_config,
                    request.display_scale,
                )?
            } else {
                None
            };
            data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                source_id: cfg.source_id.clone(),
                series_id: cfg.series_id.clone(),
                paint_order,
                geometry: crate::gpu_data_pick::GpuDataPickGeometry::Field {
                    field_bg,
                    charge,
                    has_fill,
                    has_contour,
                    contour_width_px,
                    nan_color_premul,
                    label,
                },
            });
        }

        // Value labels: drawn over every series of the panel, so they pick
        // with a paint order after every series and win the ties against
        // the data under them. The same atlas, uniform and group 3 the draw
        // places them with, at the pick's display transform and scale.
        for (index, cfg) in series.iter().enumerate() {
            let Some(atlas) = self
                .preparation_parts()
                .1
                .ensure_value_label_scratch(&display_config, cfg)?
            else {
                continue;
            };
            let Some(data) = self.value_label_pick_group(
                &config,
                &display_config,
                &series,
                cfg,
                request.display_scale,
            )?
            else {
                continue;
            };
            let Some(labels) = self.preparation_parts().1.value_label_data(
                &display_config,
                cfg,
                &atlas,
                request.display_scale,
            ) else {
                continue;
            };
            if labels.count == 0 {
                continue;
            }
            let paint_order = u32::try_from(series.len() + index).map_err(|_| {
                FiggyError::InvalidSeriesConfig {
                    series_id: cfg.series_id.clone(),
                    reason: "value label paint order exceeds the GPU u32 index space".into(),
                }
            })?;
            data_series.push(crate::gpu_data_pick::GpuDataPickSeries {
                source_id: cfg.source_id.clone(),
                series_id: cfg.series_id.clone(),
                paint_order,
                geometry: crate::gpu_data_pick::GpuDataPickGeometry::ValueLabels { data, labels },
            });
        }

        let chart_rect = display_config.chart_area.0;
        let data_area_px = display_config.data_area().ok().map(|area| {
            let rect = area.0;
            [
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
            ]
        });
        let data_ticket = data_bundle.submit(
            &self.pool,
            crate::gpu_data_pick::GpuDataPickQuery {
                transform: data_render::scatter_transform_from_config(&display_config),
                chart_rect_px: [
                    chart_rect.x as f32,
                    chart_rect.y as f32,
                    chart_rect.width as f32,
                    chart_rect.height as f32,
                ],
                data_area_px,
                canvas_position_px: request.canvas_position_px,
                max_distance_px: request.max_distance_px,
            },
            data_series,
        )?;
        Ok(crate::gpu_data_pick::GpuDataPickTicket::new(
            point_ticket,
            data_ticket,
            point_orders,
            members,
        ))
    }
}
