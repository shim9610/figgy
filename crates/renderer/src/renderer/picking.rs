//! GPU picking on registered charts: the pick request, the per-series
//! resource signatures, the pick plan, the picker state machine and the
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PickPointStyleSignature {
    pub(super) radius_bits: Option<u32>,
    pub(super) shape_id: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PickPointOverrideSignature {
    pub(super) point_index: u32,
    pub(super) style: PickPointStyleSignature,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PickPreciseStyleMappingSignature {
    pub(super) style_index_column: Option<ColumnId>,
    pub(super) style_slots: Vec<PickPointStyleSignature>,
    pub(super) style_overrides: Vec<PickPointOverrideSignature>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PickStyleMappingSignature {
    Inactive,
    Precise(PickPreciseStyleMappingSignature),
}

/// A `point_size_map` as the pick plan compares it: the column and the
/// explicit mapping, by bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PickSizeMapSignature {
    pub(super) column: ColumnId,
    pub(super) domain_min_bits: u64,
    pub(super) domain_max_bits: u64,
    pub(super) min_px_bits: u32,
    pub(super) max_px_bits: u32,
    pub(super) area: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PickScatterSignature {
    pub(super) base_radius_bits: u32,
    pub(super) base_shape_id: u32,
    pub(super) style_mapping: PickStyleMappingSignature,
    pub(super) size_map: Option<PickSizeMapSignature>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PickResourceSignature {
    pub(super) x_column: ColumnId,
    pub(super) y_column: ColumnId,
    pub(super) primitives: EffectiveSeriesPrimitives,
    pub(super) line_width_bits: Option<u32>,
    /// The line's step mode: a stepped path is picked along its pieces.
    pub(super) line_step_mode: u32,
    /// The line's dash preset: a dashed path is picked on its dashes.
    pub(super) line_style: LineStylePreset,
    pub(super) scatter: Option<PickScatterSignature>,
    /// A point vector's arrows: picked on the drawn shaft and head.
    pub(super) vector: Option<PickVectorSignature>,
}

/// A point vector's arrows as the pick plan compares them: the component
/// and colour columns and the style, by bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PickVectorSignature {
    pub(super) u_column: ColumnId,
    pub(super) v_column: ColumnId,
    pub(super) color_column: Option<ColumnId>,
    pub(super) scale_pixels: bool,
    pub(super) scale_bits: u32,
    pub(super) pivot: u32,
    pub(super) width_bits: u32,
    pub(super) head_length_bits: u32,
    pub(super) head_width_bits: u32,
    pub(super) min_length_bits: u32,
}

impl PickVectorSignature {
    pub(super) fn of(vector: &crate::gpu_pick::GpuPickVector) -> Self {
        Self {
            u_column: vector.u_column.clone(),
            v_column: vector.v_column.clone(),
            color_column: vector.color_column.clone(),
            scale_pixels: vector.scale_pixels,
            scale_bits: vector.scale.to_bits(),
            pivot: vector.pivot,
            width_bits: vector.width_px.to_bits(),
            head_length_bits: vector.head_length_px.to_bits(),
            head_width_bits: vector.head_width_px.to_bits(),
            min_length_bits: vector.min_length_px.to_bits(),
        }
    }

    pub(super) fn references_column(&self, id: &str) -> bool {
        self.u_column == id || self.v_column == id || self.color_column.as_deref() == Some(id)
    }
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

impl PickResourceSignature {
    pub(super) fn references_column(&self, id: &str) -> bool {
        self.x_column == id
            || self.y_column == id
            || self.scatter.as_ref().is_some_and(|scatter| {
                matches!(
                    &scatter.style_mapping,
                    PickStyleMappingSignature::Precise(mapping)
                        if mapping.style_index_column.as_deref() == Some(id)
                ) || scatter
                    .size_map
                    .as_ref()
                    .is_some_and(|size_map| size_map.column == id)
            })
            || self
                .vector
                .as_ref()
                .is_some_and(|vector| vector.references_column(id))
    }

    pub(super) fn references_any_column(&self, ids: &[&str]) -> bool {
        ids.iter().any(|id| self.references_column(id))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PickSeriesSignature {
    pub(super) series_id: String,
    pub(super) source_id: Option<String>,
    pub(super) resource: PickResourceSignature,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PickChartSignature {
    pub(super) slots: Vec<PickSeriesSignature>,
}

impl PickChartSignature {
    pub(super) fn references_column(&self, id: &str) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.resource.references_column(id))
    }

    pub(super) fn references_any_column(&self, ids: &[&str]) -> bool {
        ids.iter().any(|id| self.references_column(id))
    }
}

pub(super) struct OwnedGpuPickScatter {
    pub(super) base_radius_px: f32,
    pub(super) base_shape_id: u32,
    pub(super) style_index_column: Option<ColumnId>,
    pub(super) style_slots: Vec<data_render::ScatterStyleSlotGpu>,
    pub(super) style_overrides: Vec<data_render::ScatterStyleOverrideGpu>,
    pub(super) style_meta: Option<data_render::ScatterStyleMapMeta>,
    pub(super) size_map: Option<crate::gpu_pick::GpuPickSizeMap>,
}

impl OwnedGpuPickScatter {
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
        crate::gpu_pick::GpuPickScatter {
            base_radius_px: *base_radius_px,
            base_shape_id: *base_shape_id,
            style_map: style_meta.map(|style_meta| crate::gpu_pick::GpuPickScatterStyle {
                style_index_column: style_index_column.clone(),
                style_slots,
                style_overrides,
                style_meta,
            }),
            size_map: size_map.clone(),
        }
    }
}

pub(super) struct OwnedGpuPickSeriesDescriptor {
    pub(super) signature: PickSeriesSignature,
    pub(super) scatter: Option<OwnedGpuPickScatter>,
    pub(super) line_width_px: Option<f32>,
    pub(super) line_interpolation: LineInterpolation,
    pub(super) line_style: LineStylePreset,
    pub(super) vector: Option<crate::gpu_pick::GpuPickVector>,
}

impl OwnedGpuPickSeriesDescriptor {
    pub(super) fn descriptor(&self) -> crate::gpu_pick::GpuPickSeriesDescriptor<'_> {
        let Self {
            signature,
            scatter,
            line_width_px,
            line_interpolation,
            line_style,
            vector,
        } = self;
        let PickSeriesSignature {
            series_id,
            source_id,
            resource,
        } = signature;
        let PickResourceSignature {
            x_column,
            y_column,
            // The rest of the signature restates the owned fields above in
            // comparable form; the descriptor reads those.
            primitives: _,
            line_width_bits: _,
            line_step_mode: _,
            line_style: _,
            scatter: _,
            vector: _,
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
        }
    }
}

pub(super) struct PickChartPlan {
    pub(super) descriptors: Vec<OwnedGpuPickSeriesDescriptor>,
    pub(super) signature: PickChartSignature,
}

/// The panel and series a pick addresses: the active layer's. Picking a
/// layered document reads the active layer's internal panel (its Y range and
/// the group data area) and only the series that layer scales; the base
/// layer answers while the annotation layer is active, which has no series.
/// A document without overlays is its own panel.
pub(super) fn active_layer_pick_inputs(
    config: &Config,
    series: &[SeriesConfig],
) -> std::result::Result<(Config, Vec<SeriesConfig>), crate::gpu_pick::GpuPickError> {
    if !is_layered(config) {
        return Ok((config.clone(), series.to_vec()));
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
    let layer = config.panel_layers()[index];
    let panel = panels
        .into_iter()
        .nth(index)
        .expect("expansion yields one panel per layer");
    let subset = series
        .iter()
        .filter(|item| config.layer_of_series(&item.series_id) == layer)
        .cloned()
        .collect();
    Ok((panel, subset))
}

impl PickChartPlan {
    /// The plan for the active layer of `config`; see
    /// [`active_layer_pick_inputs`].
    pub(super) fn for_active_layer(
        config: &Config,
        series: &[SeriesConfig],
    ) -> std::result::Result<Self, crate::gpu_pick::GpuPickError> {
        let (panel, subset) = active_layer_pick_inputs(config, series)?;
        Self::new(&panel, &subset)
    }

    pub(super) fn new(
        config: &Config,
        series: &[SeriesConfig],
    ) -> std::result::Result<Self, crate::gpu_pick::GpuPickError> {
        const MASK_COLOR: u32 = 1;
        const MASK_RADIUS: u32 = 2;
        const MASK_SHAPE: u32 = 4;

        let mut descriptors = Vec::new();
        descriptors.try_reserve_exact(series.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "renderer pick descriptor plan",
            }
        })?;

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

            let (scatter, scatter_signature) = if let Some((_, style)) =
                extract_vector(&series.render_type).filter(|_| vector_points)
            {
                // The marker slot carries the anchor's highlight radius; the
                // pick itself walks the drawn arrow (`GpuPickVector`).
                let base_radius_px = crate::pick::vector_anchor_radius_px(style);
                let base_shape_id = data_render::shape_id(&ScatterShape::CircleFilled);
                (
                    Some(OwnedGpuPickScatter {
                        base_radius_px,
                        base_shape_id,
                        style_index_column: None,
                        style_slots: Vec::new(),
                        style_overrides: Vec::new(),
                        style_meta: None,
                        size_map: None,
                    }),
                    Some(PickScatterSignature {
                        base_radius_bits: base_radius_px.to_bits(),
                        base_shape_id,
                        style_mapping: PickStyleMappingSignature::Inactive,
                        size_map: None,
                    }),
                )
            } else if primitives.scatter {
                let scatter = extract_scatter(&series.render_type)
                    .expect("effective scatter primitive has scatter configuration");
                let base_radius_px = scatter.point_size;
                let base_shape_id = data_render::shape_id(&scatter.point_shape);
                if config.draw_style.is_precise() {
                    let style_table = scatter.point_style_table.as_deref().unwrap_or(&[]);
                    let overrides = scatter.point_style_overrides.as_deref().unwrap_or(&[]);
                    let mut style_slots = Vec::new();
                    let mut style_signatures = Vec::new();
                    style_slots
                        .try_reserve_exact(style_table.len())
                        .map_err(|_| crate::gpu_pick::GpuPickError::AllocationFailed {
                            resource: "renderer pick style slots",
                        })?;
                    style_signatures
                        .try_reserve_exact(style_table.len())
                        .map_err(|_| crate::gpu_pick::GpuPickError::AllocationFailed {
                            resource: "renderer pick style signatures",
                        })?;
                    for slot in style_table {
                        style_slots.push(scatter_style_slot_gpu(
                            slot,
                            1.0,
                            MASK_COLOR,
                            MASK_RADIUS,
                            MASK_SHAPE,
                        ));
                        style_signatures.push(pick_point_style_signature(slot));
                    }

                    let mut style_overrides = Vec::new();
                    let mut override_signatures = Vec::new();
                    style_overrides
                        .try_reserve_exact(overrides.len())
                        .map_err(|_| crate::gpu_pick::GpuPickError::AllocationFailed {
                            resource: "renderer pick style overrides",
                        })?;
                    override_signatures
                        .try_reserve_exact(overrides.len())
                        .map_err(|_| crate::gpu_pick::GpuPickError::AllocationFailed {
                            resource: "renderer pick override signatures",
                        })?;
                    for override_ in overrides {
                        let Ok(point_index) = u32::try_from(override_.index) else {
                            continue;
                        };
                        let slot = scatter_style_slot_gpu(
                            &override_.style,
                            1.0,
                            MASK_COLOR,
                            MASK_RADIUS,
                            MASK_SHAPE,
                        );
                        style_overrides.push(data_render::ScatterStyleOverrideGpu {
                            point_index,
                            _pad: [0; 3],
                            color_premul: slot.color_premul,
                            meta: slot.meta,
                        });
                        override_signatures.push(PickPointOverrideSignature {
                            point_index,
                            style: pick_point_style_signature(&override_.style),
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
                    let style_mapping = if style_meta.is_some() {
                        PickStyleMappingSignature::Precise(PickPreciseStyleMappingSignature {
                            style_index_column: style_index_column.clone(),
                            style_slots: style_signatures,
                            style_overrides: override_signatures,
                        })
                    } else {
                        PickStyleMappingSignature::Inactive
                    };
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
                    let size_map_signature = size_map.as_ref().map(|map| PickSizeMapSignature {
                        column: map.column.clone(),
                        domain_min_bits: map.domain_min.to_bits(),
                        domain_max_bits: map.domain_max.to_bits(),
                        min_px_bits: map.min_px.to_bits(),
                        max_px_bits: map.max_px.to_bits(),
                        area: map.area,
                    });
                    (
                        Some(OwnedGpuPickScatter {
                            base_radius_px,
                            base_shape_id,
                            style_index_column: style_index_column.clone(),
                            style_slots,
                            style_overrides,
                            style_meta,
                            size_map,
                        }),
                        Some(PickScatterSignature {
                            base_radius_bits: base_radius_px.to_bits(),
                            base_shape_id,
                            style_mapping,
                            size_map: size_map_signature,
                        }),
                    )
                } else {
                    (
                        Some(OwnedGpuPickScatter {
                            base_radius_px,
                            base_shape_id,
                            style_index_column: None,
                            style_slots: Vec::new(),
                            style_overrides: Vec::new(),
                            style_meta: None,
                            size_map: None,
                        }),
                        Some(PickScatterSignature {
                            base_radius_bits: base_radius_px.to_bits(),
                            base_shape_id,
                            style_mapping: PickStyleMappingSignature::Inactive,
                            size_map: None,
                        }),
                    )
                }
            } else {
                (None, None)
            };

            let line = primitives.line.then(|| {
                extract_line(&series.render_type)
                    .expect("effective line primitive has line configuration")
            });
            let line_width_px = line.map(|line| line.line_width);
            let line_interpolation =
                line.map_or(LineInterpolation::Linear, |line| line.interpolation);
            let line_style = line.map_or(LineStylePreset::Solid, |line| line.line_style);
            let resource = PickResourceSignature {
                x_column: series.x_column.clone(),
                y_column: series.y_column.clone(),
                primitives,
                line_width_bits: line_width_px.map(f32::to_bits),
                line_step_mode: crate::line_step::step_mode(line_interpolation),
                line_style,
                scatter: scatter_signature,
                vector: vector.as_ref().map(PickVectorSignature::of),
            };
            descriptors.push(OwnedGpuPickSeriesDescriptor {
                signature: PickSeriesSignature {
                    series_id: series.series_id.clone(),
                    source_id: series.source_id.clone(),
                    resource,
                },
                scatter,
                line_width_px,
                line_interpolation,
                line_style,
                vector,
            });
        }

        let mut signature_slots = Vec::new();
        signature_slots
            .try_reserve_exact(descriptors.len())
            .map_err(|_| crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "renderer pick chart signature",
            })?;
        signature_slots.extend(
            descriptors
                .iter()
                .map(|descriptor| descriptor.signature.clone()),
        );
        Ok(Self {
            descriptors,
            signature: PickChartSignature {
                slots: signature_slots,
            },
        })
    }

    pub(super) fn registry_slots<'a>(
        &'a self,
        current: Option<&PickChartSignature>,
    ) -> std::result::Result<
        Vec<crate::gpu_pick::GpuPickRegistrySlot<'a>>,
        crate::gpu_pick::GpuPickError,
    > {
        self.registry_slots_rebuilding(current, &[])
    }

    pub(super) fn registry_slots_rebuilding<'a>(
        &'a self,
        current: Option<&PickChartSignature>,
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
            let reusable = current.and_then(|signature| {
                signature.slots.iter().position(|slot| {
                    !descriptor
                        .signature
                        .resource
                        .references_any_column(rebuild_columns)
                        && slot.series_id == descriptor.signature.series_id
                        && slot.resource == descriptor.signature.resource
                })
            });
            if let Some(current_index) = reusable {
                slots.push(crate::gpu_pick::GpuPickRegistrySlot::Reuse {
                    current_index,
                    source_id: descriptor.signature.source_id.clone(),
                    series_id: descriptor.signature.series_id.clone(),
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
    pub(super) signature: PickChartSignature,
    pub(super) engine: crate::gpu_pick::GpuPickEngine,
}

pub(super) enum PreparedActivePickerMutation<'a> {
    Clear(&'a mut Option<ActivePickRegistry>),
    Update {
        transition: crate::gpu_pick::PreparedPickRegistryTransition<'a>,
        active_chart_id: &'a mut ChartId,
        active_signature: &'a mut PickChartSignature,
        next_chart_id: ChartId,
        next_signature: PickChartSignature,
    },
}

impl PreparedActivePickerMutation<'_> {
    pub(super) fn commit(self) {
        match self {
            Self::Clear(active) => *active = None,
            Self::Update {
                transition,
                active_chart_id,
                active_signature,
                next_chart_id,
                next_signature,
            } => {
                transition.commit();
                *active_chart_id = next_chart_id;
                *active_signature = next_signature;
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
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.chart_id == chart_id && active.signature == plan.signature)
        {
            return Ok(());
        }

        if let Some(active) = self.active.as_mut() {
            let slots = plan.registry_slots(Some(&active.signature))?;
            active
                .engine
                .prepare_registry_transition(pool, slots)?
                .commit();
            active.chart_id = chart_id;
            active.signature = plan.signature;
        } else {
            let mut engine = crate::gpu_pick::GpuPickEngine::from_bundle(bundle)?;
            let slots = plan.registry_slots(None)?;
            engine.prepare_registry_transition(pool, slots)?.commit();
            self.active = Some(ActivePickRegistry {
                chart_id,
                signature: plan.signature,
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
        if !force && active.signature == plan.signature {
            return Ok(None);
        }

        let ActivePickRegistry {
            chart_id: active_chart_id,
            signature: active_signature,
            engine,
        } = active;
        let slots = plan.registry_slots_rebuilding(Some(active_signature), rebuild_columns)?;
        let transition = engine.prepare_registry_transition(pool, slots)?;
        Ok(Some(PreparedActivePickerMutation::Update {
            transition,
            active_chart_id,
            active_signature,
            next_chart_id: chart_id,
            next_signature: plan.signature,
        }))
    }
}

pub(super) fn pick_point_style_signature(
    style: &DataScatterPointStyleConfig,
) -> PickPointStyleSignature {
    PickPointStyleSignature {
        radius_bits: style.point_size.map(f32::to_bits),
        shape_id: style.point_shape.as_ref().map(data_render::shape_id),
    }
}

impl Renderer {
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
        let (display_config, layer_series) = active_layer_pick_inputs(&displayed, &state.series)?;
        let transform = data_render::scatter_transform_from_config(&display_config);
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
            let series = layer_series
                .iter()
                .find(|series| series.series_id == series_id);
            let arc = series
                .filter(|series| line_pick_walks_arc(&display_config.draw_style, series))
                .and_then(|series| {
                    let interpolation = extract_line(&series.render_type)
                        .map_or(LineInterpolation::Linear, |line| line.interpolation);
                    self.preparation_parts().1.ensure_arc_prefix(
                        &series.series_id,
                        &series.x_column,
                        &series.y_column,
                        &transform,
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
        let (config, series, display_config) = {
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
            let (config, series) = active_layer_pick_inputs(&state.config, &state.series)?;
            let displayed = place_document(
                &state.config,
                request.display_scale,
                request.display_panel_px,
            );
            let (display_config, _) = active_layer_pick_inputs(&displayed, &state.series)?;
            (config, series, display_config)
        };

        // Submit the established point/line query first. It stays point-only,
        // so the compatibility API and its registry lifecycle are unchanged.
        let point_ticket = self.pick_chart(id, request)?;
        let data_bundle = self.picker.ready_data_bundle()?;

        let mut data_series = Vec::new();
        data_series
            .try_reserve_exact(series.len())
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
        ))
    }
}
