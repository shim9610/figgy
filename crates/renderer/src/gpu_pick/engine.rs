//! The point picker's execution path: the engine, its gate, exact and
//! reduce passes, the query parameters and the readback ticket.

use super::*;

/// Logical point/segment indices represented by one gate-mask word.
pub(super) const GPU_PICK_GATE_WORD_POINTS: u32 = 32;
/// Compute workgroup width used by gating, exact testing, and reduction.
pub(super) const GPU_PICK_WORKGROUP_SIZE: u32 = 64;
/// Up to 32 workgroups scan directly; larger series use the X/Y gates first.
pub(super) const GPU_PICK_DIRECT_SCAN_POINTS: u32 =
    GPU_PICK_GATE_WORD_POINTS * GPU_PICK_WORKGROUP_SIZE * 32;

/// `QUAD_MARGIN_PX` of the shared marker shape (SHADER_COMMON.md §5.5): a
/// marker draws nothing farther than its visual radius plus this margin from
/// its centre on either axis.
pub(super) const MARKER_QUAD_MARGIN_PX: f32 = crate::data_render::MARKER_QUAD_MARGIN_PX;

/// `CONSTELLATION_POINT_STAR_GAIN` times the largest per-point size jitter
/// of `constellation_star_radius` (`0.86 + 0.30`).
pub(super) const CONSTELLATION_STAR_MAX_SCALE: f32 = 1.16;
/// `PLANET_QUAD_EXTENT`: a planet's quad half-extent per unit radius.
pub(crate) const PLANET_QUAD_EXTENT: f32 = 2.5;

/// The pick shader's per-query uniform for one registered series. `data` is
/// `(point count, x lane base, y lane base, style-index lane base)` and
/// `stream` the `(global offset, owned scatter count, owned segment count,
/// row-map base)` of the chunk; a resident series owns every point. `arc`
/// says whether the exact pass binds this series' arc prefix.
#[allow(clippy::too_many_arguments)]
pub(super) fn query_params(
    series: &PickSeriesGpu,
    series_id: &str,
    query: &GpuPickQuery,
    display_scale: f32,
    series_order: u32,
    data: [u32; 4],
    stream: [u32; 4],
    arc: bool,
) -> PickQueryParamsGpu {
    let [cursor_x, cursor_y] = query.canvas_position_px;
    let [chart_x, chart_y, chart_width, chart_height] = query.chart_rect_px;
    let style = query.draw_style;
    let params = query.transform.style_params;
    // Nothing a marker draws lies outside its quad: the visual radius plus
    // the quad margin on either axis, grown by the styled entry's own quad.
    let base_radius = series.base_radius_px * display_scale;
    let marker_reach = match style.marker {
        PICK_MARKER_SKETCH => {
            series.max_extent_px * display_scale
                + MARKER_QUAD_MARGIN_PX
                + params[0][0].max(0.0) * 0.5
        }
        PICK_MARKER_PLANET => base_radius.max(1.0) * PLANET_QUAD_EXTENT + MARKER_QUAD_MARGIN_PX,
        PICK_MARKER_STAR => {
            (base_radius * CONSTELLATION_STAR_MAX_SCALE).max(0.5) * 4.0 + MARKER_QUAD_MARGIN_PX
        }
        _ => series.max_extent_px * display_scale + MARKER_QUAD_MARGIN_PX,
    };
    // The line shader's `max(line_width_px, 1) / 2`.
    let half_width = (2.0 * series.line_half_width_px * display_scale).max(1.0) * 0.5;
    let ribbon = milkyway_line_half_extent(&params);
    let line_extent = match style.line {
        PICK_LINE_SKETCH => {
            half_width + crate::data_render::MARKER_STROKE_HALF_PX + params[0][0].max(0.0)
        }
        PICK_LINE_RIBBON => ribbon,
        _ => half_width + crate::data_render::MARKER_STROKE_HALF_PX,
    };
    let mut line_dash = [[0.0f32; 4]; 2];
    for (index, length) in series.line_dash.iter().enumerate() {
        line_dash[index / 4][index % 4] = length * display_scale;
    }
    // A milkyway ribbon is never dashed.
    let dash_count = if style.line == PICK_LINE_RIBBON {
        0
    } else {
        series.line_dash_len
    };
    PickQueryParamsGpu {
        transform: query.transform,
        cursor_chart: [cursor_x, cursor_y, chart_x, chart_y],
        chart_limits: [
            chart_width,
            chart_height,
            query.max_distance_px,
            marker_reach,
        ],
        scatter_line: [
            series.base_radius_px,
            series.line_half_width_px,
            display_scale,
            if series.direct_scan { 1.0 } else { 0.0 },
        ],
        data,
        style: [
            series.flags,
            series.style_count,
            series.override_count,
            series.style_index_len,
        ],
        series: [
            series.gate_word_count,
            series_order,
            series.base_shape_id,
            series.dispatch_x,
        ],
        stream,
        line: [
            series.line_step_mode,
            u32::from(arc),
            u32::from(series.line_closed),
            0,
        ],
        size_map: [series.size_base, series.size_len, 0, 0],
        size_domain: series.size_domain,
        size_px: [series.size_px[0], series.size_px[1], 0.0, 0.0],
        // A square cap's corner reaches √2 times the half extent past the
        // centreline on either axis.
        line_px: [
            half_width,
            ribbon,
            line_extent * std::f32::consts::SQRT_2,
            0.0,
        ],
        line_dash,
        styled: [
            style.marker,
            style.line,
            crate::sketch::fnv1a(series_id),
            dash_count,
        ],
        vector: [
            series.vector_base[0],
            series.vector_base[1],
            series.vector_count,
            series.vector.as_ref().map_or(0, |vector| {
                vector.pivot
                    | if vector.scale_pixels {
                        VECTOR_SCALE_PIXELS
                    } else {
                        0
                    }
            }),
        ],
        vector_px: series.vector.as_ref().map_or([0.0; 4], |vector| {
            [
                if vector.scale_pixels {
                    vector.scale * display_scale
                } else {
                    vector.scale
                },
                vector.width_px * display_scale,
                vector.head_length_px * display_scale,
                vector.head_width_px * display_scale,
            ]
        }),
        vector_min: [
            series
                .vector
                .as_ref()
                .map_or(0.0, |vector| vector.min_length_px * display_scale),
            0.0,
            0.0,
            0.0,
        ],
        errorbar: series.errorbar.as_ref().map_or([0; 4], |bars| bars.params),
        errorbar_lanes: series.errorbar.as_ref().map_or([0; 4], |bars| bars.lanes),
        errorbar_rows: series.errorbar.as_ref().map_or([0; 4], |bars| bars.rows),
        errorbar_px: series.errorbar.as_ref().map_or([0.0; 4], |bars| bars.px),
        area: series.area.as_ref().map_or([0; 4], |fill| fill.params),
        area_baseline: series.area.as_ref().map_or([0.0; 4], |fill| fill.baseline),
    }
}

/// The half extent of what a Milkyway line draws around its centreline, in
/// canvas pixels, from the packed style parameters (`pack_milkyway_params`):
/// the ribbon strip reaches one full ribbon width either side
/// (`vs_ribbon`'s `max(ribbon_width_px, 1)`), and a star of the arc-driven
/// star pass (`vs_stars`) lies within its perpendicular scatter
/// (`1.7 * spread_px`), a binary companion's separation (`4 *
/// structure_scale`) and its quad (`4 * star_scale * 4.3` at full
/// brightness) of the path. The star positions are hashed per slot, so this
/// envelope, not each star, is what picks.
pub(crate) fn milkyway_line_half_extent(params: &[[f32; 4]; 3]) -> f32 {
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

#[derive(Clone)]
pub(super) struct ReduceResources {
    pub(super) candidates: wgpu::Buffer,
    pub(super) final_result: wgpu::Buffer,
    pub(super) bind_group: wgpu::BindGroup,
    pub(super) capacity: usize,
    /// Charge for `candidates` + `final_result`. Shared because this struct is
    /// cloned and the clones hold the same device buffers — see
    /// [`SharedCharge`].
    pub(super) _charge: SharedCharge,
}

/// Persistent exact GPU picking engine.
pub(crate) struct GpuPickEngine {
    pub(super) bundle: Arc<PickPipelineBundle>,
    pub(super) registry: PickRegistry,
    #[cfg(test)]
    pub(super) fail_next_registry_prepare: std::cell::Cell<bool>,
}

/// Every pick buffer goes through here, and the tally is not optional, so a new
/// pick allocation cannot skip the ledger.
pub(super) fn create_buffer_checked(
    tally: &ChargeTally,
    device: &wgpu::Device,
    desc: &wgpu::BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Buffer, GpuPickError> {
    // gpu-alloc: PickScratch
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        charged_buffer(tally, device, desc)
    }))
    .map_err(|_| GpuPickError::AllocationFailed { resource })
}

pub(super) fn create_bind_group_checked(
    device: &wgpu::Device,
    desc: &wgpu::BindGroupDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::BindGroup, GpuPickError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        device.create_bind_group(desc)
    }))
    .map_err(|_| GpuPickError::AllocationFailed { resource })
}

pub(super) fn lane_base(handle: ColumnHandle, series_id: &str) -> Result<u32, GpuPickError> {
    if !handle.offset.is_multiple_of(4) {
        return Err(GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "column byte offset is not f32-aligned",
        });
    }
    u32::try_from(handle.offset / 4).map_err(|_| GpuPickError::DeviceLimit {
        resource: "column word offset",
        requested: handle.offset / 4,
        limit: u64::from(u32::MAX),
    })
}

pub(super) fn gate_dispatch_layout(
    point_count: u32,
    max_workgroups_per_dimension: u32,
) -> Result<(u32, u32, u32, u32), GpuPickError> {
    if max_workgroups_per_dimension == 0 {
        return Err(GpuPickError::DeviceLimit {
            resource: "max_compute_workgroups_per_dimension",
            requested: 1,
            limit: 0,
        });
    }
    let gate_word_count = point_count.div_ceil(GPU_PICK_GATE_WORD_POINTS);
    let workgroup_count = gate_word_count.div_ceil(GPU_PICK_WORKGROUP_SIZE);
    let dispatch_x = workgroup_count.min(max_workgroups_per_dimension);
    let dispatch_y = workgroup_count.div_ceil(dispatch_x);
    if dispatch_y > max_workgroups_per_dimension {
        return Err(GpuPickError::DeviceLimit {
            resource: "GPU pick gate dispatch",
            requested: u64::from(workgroup_count),
            limit: u64::from(max_workgroups_per_dimension)
                * u64::from(max_workgroups_per_dimension),
        });
    }
    Ok((gate_word_count, workgroup_count, dispatch_x, dispatch_y))
}

pub(super) fn same_storage(a: ColumnHandle, b: ColumnHandle) -> bool {
    a.offset == b.offset && a.byte_size == b.byte_size && a.len_values == b.len_values
}

/// [`same_storage`] for an optional column: both absent, or both present at
/// the same storage.
pub(super) fn same_optional_storage(a: Option<ColumnHandle>, b: Option<ColumnHandle>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same_storage(a, b),
        (None, None) => true,
        _ => false,
    }
}

impl GpuPickEngine {
    #[cfg(test)]
    pub(super) fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
    ) -> Result<Self, GpuPickError> {
        let mut noop = |_| {};
        Self::new_observed(device, queue, &mut noop)
    }

    #[cfg(test)]
    pub(super) fn new_observed(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Self, GpuPickError> {
        observe_result(observer, INIT_SCOPE, "limits", || {
            validate_device_limits(&device)
        })?;
        let bundle =
            create_pipeline_bundle_observed(device, queue, Arc::new(GpuLedger::new()), observer);
        let reduce = observe_result(observer, INIT_SCOPE, "initial_reduce_resources", || {
            Self::create_reduce_resources(&bundle.device, &bundle.ledger, &bundle.reduce_bgl, 1)
        })?;
        Ok(Self::from_bundle_and_reduce(bundle, reduce))
    }

    #[allow(dead_code)] // Consumed by renderer ownership integration in P-02.
    pub(crate) fn from_bundle(bundle: Arc<PickPipelineBundle>) -> Result<Self, GpuPickError> {
        let reduce =
            Self::create_reduce_resources(&bundle.device, &bundle.ledger, &bundle.reduce_bgl, 1)?;
        Ok(Self::from_bundle_and_reduce(bundle, reduce))
    }

    pub(super) fn from_bundle_and_reduce(
        bundle: Arc<PickPipelineBundle>,
        reduce: ReduceResources,
    ) -> Self {
        Self {
            bundle,
            registry: PickRegistry {
                slots: Vec::new(),
                identities: Arc::from([]),
                reduce,
                pool_identity: None,
                pool_layout_generation: 0,
                generation: 0,
            },
            #[cfg(test)]
            fail_next_registry_prepare: std::cell::Cell::new(false),
        }
    }

    #[cfg(test)]
    pub(crate) fn registry_generation(&self) -> u64 {
        self.registry.generation
    }

    #[cfg(test)]
    pub(crate) fn registered_series_count(&self) -> usize {
        self.registry.slots.len()
    }

    #[cfg(test)]
    pub(crate) fn query_params_buffer(&self, index: usize) -> Option<wgpu::Buffer> {
        self.registry
            .slots
            .get(index)
            .map(|slot| slot.gpu.query_params.clone())
    }

    #[cfg(test)]
    pub(crate) fn identity_snapshot(&self) -> Vec<(Option<String>, String)> {
        self.registry
            .identities
            .iter()
            .map(|identity| (identity.source_id.clone(), identity.series_id.clone()))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn fail_next_registry_prepare(&self) {
        self.fail_next_registry_prepare.set(true);
    }

    pub(super) fn create_reduce_resources(
        device: &wgpu::Device,
        ledger: &Arc<GpuLedger>,
        layout: &wgpu::BindGroupLayout,
        candidate_count: usize,
    ) -> Result<ReduceResources, GpuPickError> {
        let capacity = candidate_count.max(1);
        let tally = ChargeTally::new();
        let count = capacity as u64;
        let candidate_size =
            count
                .checked_mul(CANDIDATE_BYTES)
                .ok_or(GpuPickError::DeviceLimit {
                    resource: "candidate buffer",
                    requested: u64::MAX,
                    limit: device.limits().max_buffer_size,
                })?;
        let candidates = create_buffer_checked(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick per-series candidates"),
                size: candidate_size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "per-series candidate buffer",
        )?;
        let final_result = create_buffer_checked(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick final scalar"),
                size: CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
            "final pick scalar buffer",
        )?;
        let bind_group = create_bind_group_checked(
            device,
            &wgpu::BindGroupDescriptor {
                label: Some("figgy GPU pick reduction bg"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: candidates.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: final_result.as_entire_binding(),
                    },
                ],
            },
            "reduction bind group",
        )?;
        Ok(ReduceResources {
            candidates,
            final_result,
            bind_group,
            capacity,
            _charge: shared_charge(tally, ledger, GpuResourceKind::PickScratch),
        })
    }

    pub(super) fn prepare_reduce_resources(
        &self,
        candidate_count: usize,
    ) -> Result<ReduceResources, GpuPickError> {
        if candidate_count <= self.registry.reduce.capacity {
            Ok(self.registry.reduce.clone())
        } else {
            Self::create_reduce_resources(
                &self.bundle.device,
                &self.bundle.ledger,
                &self.bundle.reduce_bgl,
                candidate_count,
            )
        }
    }

    /// Prepare one persistent exact query slot from `pool`.
    ///
    /// Every later [`Self::pick`] must receive that same pool instance. Replace
    /// this slot after any referenced column content replacement. Allocation
    /// epochs distinguish remove+readd even when the replacement reuses the
    /// same offset and length.
    #[cfg(test)]
    pub(super) fn add_series(
        &mut self,
        pool: &ColumnPool,
        descriptor: GpuPickSeriesDescriptor<'_>,
    ) -> Result<GpuPickSeriesId, GpuPickError> {
        let id = GpuPickSeriesId(self.registry.slots.len() as u32);
        let mut final_slots = self.current_reuse_slots(None);
        final_slots.push(GpuPickRegistrySlot::Build(descriptor));
        self.prepare_registry_transition(pool, final_slots)?
            .commit();
        Ok(id)
    }

    /// Build one resident series' slot from `descriptor` and `pool` alone:
    /// no other series state reaches the build. So the descriptor the
    /// renderer's pick plan keeps (`renderer::picking::PickSeriesResource`
    /// with the series id) is this build's complete key, and reusing a slot
    /// whose descriptor is equal is exact by construction (#66, tier 1).
    pub(super) fn prepare_series(
        &self,
        pool: &ColumnPool,
        descriptor: GpuPickSeriesDescriptor<'_>,
    ) -> Result<PickSeriesSlot, GpuPickError> {
        let x_handle = pool
            .handle_for(&descriptor.x_column)
            .ok_or_else(|| GpuPickError::MissingColumn(descriptor.x_column.clone()))?;
        let y_handle = pool
            .handle_for(&descriptor.y_column)
            .ok_or_else(|| GpuPickError::MissingColumn(descriptor.y_column.clone()))?;
        let style_index_handle = descriptor
            .scatter
            .as_ref()
            .and_then(|scatter| scatter.style_map.as_ref())
            .and_then(|map| map.style_index_column.as_ref())
            .map(|id| {
                pool.handle_for(id)
                    .ok_or_else(|| GpuPickError::MissingColumn(id.clone()))
            })
            .transpose()?;
        let size_value_handle = descriptor
            .scatter
            .as_ref()
            .and_then(|scatter| scatter.size_map.as_ref())
            .map(|map| {
                pool.handle_for(&map.column)
                    .ok_or_else(|| GpuPickError::MissingColumn(map.column.clone()))
            })
            .transpose()?;
        let mut vector = [None; 3];
        if let Some(arrows) = descriptor.vector.as_ref() {
            for (slot, id) in vector.iter_mut().zip([
                Some(&arrows.u_column),
                Some(&arrows.v_column),
                arrows.color_column.as_ref(),
            ]) {
                if let Some(id) = id {
                    *slot = Some(
                        pool.handle_for(id)
                            .ok_or_else(|| GpuPickError::MissingColumn(id.clone()))?,
                    );
                }
            }
        }
        self.prepare_series_buffer(
            pool.buffer(),
            descriptor,
            GpuStreamPickColumns {
                x: x_handle,
                y: y_handle,
                style_index: style_index_handle,
                size_value: size_value_handle,
                vector,
            },
            Some(pool),
        )
    }

    pub(super) fn prepare_series_buffer(
        &self,
        buffer: &wgpu::Buffer,
        mut descriptor: GpuPickSeriesDescriptor<'_>,
        columns: GpuStreamPickColumns,
        resident: Option<&ColumnPool>,
    ) -> Result<PickSeriesSlot, GpuPickError> {
        let limits = self.bundle.device.limits();
        if buffer.size() > limits.max_storage_buffer_binding_size {
            return Err(GpuPickError::DeviceLimit {
                resource: "column-pool storage binding",
                requested: buffer.size(),
                limit: limits.max_storage_buffer_binding_size,
            });
        }
        let x_handle = columns.x;
        let y_handle = columns.y;
        for handle in [
            Some(x_handle),
            Some(y_handle),
            columns.style_index,
            columns.size_value,
            columns.vector[0],
            columns.vector[1],
            columns.vector[2],
        ]
        .into_iter()
        .flatten()
        {
            validate_stream_handle(buffer, handle)?;
        }
        let x_allocation_epoch = resident
            .and_then(|pool| pool.allocation_epoch(&descriptor.x_column))
            .unwrap_or(0);
        let y_allocation_epoch = resident
            .and_then(|pool| pool.allocation_epoch(&descriptor.y_column))
            .unwrap_or(0);
        let paired_len = x_handle.len_values.min(y_handle.len_values);
        if paired_len == 0 {
            return Err(GpuPickError::EmptySeries(descriptor.series_id));
        }
        let point_count = u32::try_from(paired_len).map_err(|_| GpuPickError::TooManyValues {
            series_id: descriptor.series_id.clone(),
            count: paired_len,
        })?;

        let mut flags = 0u32;
        let mut base_radius_px = 0.0;
        let mut base_shape_id = 0u32;
        let mut max_extent_px = 0.0;
        let mut style_count = 0u32;
        let mut override_count = 0u32;
        let mut style_index_column = None;
        let mut style_index_handle = None;
        let mut style_index_allocation_epoch = None;
        let mut style_index_base = 0u32;
        let mut style_index_len = 0u32;
        let mut size_column = None;
        let mut size_handle = None;
        let mut size_allocation_epoch = None;
        let mut size_base = 0u32;
        let mut size_len = 0u32;
        let mut size_domain = [0.0f32; 4];
        let mut size_px = [0.0f32; 2];

        // The marker rows; the error-bar rows follow them in the same
        // buffers.
        let mut marker_slots: &[ScatterStyleSlotGpu] = &[];
        let mut marker_overrides: &[ScatterStyleOverrideGpu] = &[];

        if let Some(scatter) = descriptor.scatter.as_ref() {
            flags |= FLAG_SCATTER;
            base_radius_px = nonnegative(scatter.base_radius_px);
            base_shape_id = scatter.base_shape_id;
            max_extent_px = style_max_extent(&descriptor.series_id, scatter)?;
            if let Some(map) = scatter.style_map.as_ref() {
                flags |= FLAG_STYLE_MAP;
                style_count = map.style_meta.style_count;
                override_count = map.style_meta.override_count;
                marker_slots = map.style_slots;
                marker_overrides = map.style_overrides;
                if let Some(column_id) = map.style_index_column.as_ref() {
                    flags |= FLAG_STYLE_INDEX;
                    let handle = columns
                        .style_index
                        .ok_or_else(|| GpuPickError::MissingColumn(column_id.clone()))?;
                    let allocation_epoch = resident
                        .and_then(|pool| pool.allocation_epoch(column_id))
                        .unwrap_or(0);
                    style_index_base = lane_base(handle, &descriptor.series_id)?;
                    style_index_len = u32::try_from(handle.len_values).unwrap_or(u32::MAX);
                    style_index_column = Some(column_id.clone());
                    style_index_handle = Some(handle);
                    style_index_allocation_epoch = Some(allocation_epoch);
                }
            }
            if let Some(size_map) = scatter.size_map.as_ref() {
                flags |= FLAG_SIZE_MAP;
                if size_map.area {
                    flags |= FLAG_SIZE_AREA;
                }
                let handle = columns
                    .size_value
                    .ok_or_else(|| GpuPickError::MissingColumn(size_map.column.clone()))?;
                let allocation_epoch = resident
                    .and_then(|pool| pool.allocation_epoch(&size_map.column))
                    .unwrap_or(0);
                size_base = lane_base(handle, &descriptor.series_id)?;
                size_len = u32::try_from(handle.len_values).unwrap_or(u32::MAX);
                size_column = Some(size_map.column.clone());
                size_handle = Some(handle);
                size_allocation_epoch = Some(allocation_epoch);
                let (min_hi, min_lo) = crate::data::split_f64_to_f32_pair(size_map.domain_min);
                let (max_hi, max_lo) = crate::data::split_f64_to_f32_pair(size_map.domain_max);
                size_domain = [min_hi, min_lo, max_hi, max_lo];
                size_px = [nonnegative(size_map.min_px), nonnegative(size_map.max_px)];
            }
        }

        // A point vector's arrows: every component column is resident and
        // the arrow count is the shortest column the vector shader reads.
        let mut vector_handles = [None; 3];
        let mut vector_epochs = [None; 3];
        let mut vector_base = [0u32; 2];
        let mut vector_count = 0u32;
        if let Some(arrows) = descriptor.vector.as_ref() {
            if descriptor.scatter.is_none() {
                return Err(GpuPickError::NoPickPrimitive(descriptor.series_id));
            }
            flags |= FLAG_VECTOR;
            let ids = [
                Some(&arrows.u_column),
                Some(&arrows.v_column),
                arrows.color_column.as_ref(),
            ];
            let mut count = paired_len;
            for (index, id) in ids.into_iter().enumerate() {
                let Some(id) = id else {
                    continue;
                };
                let handle =
                    columns.vector[index].ok_or_else(|| GpuPickError::MissingColumn(id.clone()))?;
                count = count.min(handle.len_values);
                vector_handles[index] = Some(handle);
                vector_epochs[index] = Some(
                    resident
                        .and_then(|pool| pool.allocation_epoch(id))
                        .unwrap_or(0),
                );
                if index < 2 {
                    vector_base[index] = lane_base(handle, &descriptor.series_id)?;
                }
            }
            vector_count = u32::try_from(count).unwrap_or(u32::MAX);
        }

        // The error bars and the line's fill read pool columns: resident
        // series only. A streamed chunk does not carry them (streaming is not
        // extended, rule 8; `encode_chunk_impl` drops them).
        let errorbar_input = descriptor.errorbar.take();
        let area_input = descriptor.area.take();
        let resolve = |id: &ColumnId| -> Result<(ColumnHandle, u64), GpuPickError> {
            let pool = resident.ok_or_else(|| GpuPickError::ColumnNotResident(id.clone()))?;
            let handle = pool
                .handle_for(id)
                .ok_or_else(|| GpuPickError::MissingColumn(id.clone()))?;
            validate_stream_handle(buffer, handle)?;
            Ok((handle, pool.allocation_epoch(id).unwrap_or(0)))
        };
        let mut bar_slots: &[ScatterStyleSlotGpu] = &[];
        let mut bar_overrides: &[ScatterStyleOverrideGpu] = &[];
        let mut errorbar = None;
        if let Some(bars) = errorbar_input.filter(|_| resident.is_some()) {
            if descriptor.scatter.is_none() {
                return Err(GpuPickError::NoPickPrimitive(descriptor.series_id));
            }
            let GpuPickErrorBar {
                y,
                x,
                stem_width_px,
                cap_half_px,
                cap_width_px,
                paints,
                style_map,
            } = bars;
            let directions = if y.is_some() {
                crate::data_render::ERRORBAR_HAS_Y
            } else {
                0
            } | if x.is_some() {
                crate::data_render::ERRORBAR_HAS_X
            } else {
                0
            };
            let [y_lo, y_hi] = y.map_or([None, None], |[lo, hi]| [Some(lo), Some(hi)]);
            let [x_lo, x_hi] = x.map_or([None, None], |[lo, hi]| [Some(lo), Some(hi)]);
            let mut index_column = None;
            if let Some(map) = style_map {
                let GpuPickErrorBarStyle {
                    style_index_column,
                    style_slots,
                    style_overrides,
                } = map;
                index_column = style_index_column;
                bar_slots = style_slots;
                bar_overrides = style_overrides;
            }
            let columns = [y_lo, y_hi, x_lo, x_hi, index_column];
            let mut handles = [None; ERRORBAR_COLUMNS];
            let mut epochs = [None; ERRORBAR_COLUMNS];
            // The bar count is the shortest column the error-bar draw binds:
            // the anchor's and those of the directions it draws.
            let mut count = paired_len;
            for (index, id) in columns.iter().enumerate() {
                let Some(id) = id else {
                    continue;
                };
                let (handle, epoch) = resolve(id)?;
                if index < 4 {
                    count = count.min(handle.len_values);
                }
                handles[index] = Some(handle);
                epochs[index] = Some(epoch);
            }
            let rows = |len: usize| u32::try_from(len).unwrap_or(u32::MAX);
            let mut bars = PickErrorBarGpu {
                columns,
                handles,
                epochs,
                params: [
                    u32::try_from(count).unwrap_or(u32::MAX),
                    directions,
                    0,
                    handles[4].map_or(0, |handle| rows(handle.len_values)),
                ],
                lanes: [0; 4],
                rows: [
                    rows(marker_slots.len()),
                    rows(bar_slots.len()),
                    rows(marker_overrides.len()),
                    rows(bar_overrides.len()),
                ],
                // As declared: the draw takes the style's sizes unclamped
                // (a quad's sign does not change its area).
                px: [
                    stem_width_px,
                    cap_half_px,
                    cap_width_px,
                    if paints { 1.0 } else { 0.0 },
                ],
            };
            bars.rebase(handles, &descriptor.series_id)?;
            errorbar = Some(bars);
        }
        let mut area = None;
        if let Some(fill) = area_input.filter(|fill| fill.paints && resident.is_some()) {
            if descriptor.line_width_px.is_none() {
                return Err(GpuPickError::NoPickPrimitive(descriptor.series_id));
            }
            let GpuPickArea {
                to_series,
                baseline,
                paints: _,
                closed,
            } = fill;
            // The fill's instance count is the shortest of the x, y and
            // second y columns it binds; one trapezoid strip per segment.
            let mut count = paired_len;
            let (handle, epoch, lane) = match to_series.as_ref() {
                Some(id) => {
                    let (handle, epoch) = resolve(id)?;
                    count = count.min(handle.len_values);
                    (
                        Some(handle),
                        Some(epoch),
                        lane_base(handle, &descriptor.series_id)?,
                    )
                }
                None => (None, None, 0),
            };
            let (hi, lo) = crate::data::split_f64_to_f32_pair(baseline);
            area = Some(PickAreaGpu {
                // A closed fill has one more trapezoid: under the segment
                // from the last point back to the first.
                params: [
                    lane,
                    u32::try_from(if closed && count > 1 {
                        count
                    } else {
                        count.saturating_sub(1)
                    })
                    .unwrap_or(u32::MAX),
                    u32::from(to_series.is_some()),
                    0,
                ],
                column: to_series,
                handle,
                epoch,
                baseline: [hi, lo, 0.0, 0.0],
            });
        }

        let line_half_width_px = descriptor
            .line_width_px
            .map(|width| nonnegative(width) * 0.5)
            .unwrap_or(0.0);
        if descriptor.line_width_px.is_some() {
            flags |= FLAG_LINE;
            max_extent_px = max_extent_px.max(line_half_width_px);
        }
        let line_step_mode = crate::line_step::step_mode(descriptor.line_interpolation);
        let mut line_dash = [0.0f32; 8];
        let pattern = descriptor.line_style.pattern();
        for (slot, length) in line_dash.iter_mut().zip(pattern) {
            *slot = *length;
        }
        let line_dash_len = pattern.len().min(line_dash.len()) as u32;
        if flags & (FLAG_SCATTER | FLAG_LINE) == 0 {
            return Err(GpuPickError::NoPickPrimitive(descriptor.series_id));
        }

        let _ = lane_base(x_handle, &descriptor.series_id)?;
        let _ = lane_base(y_handle, &descriptor.series_id)?;
        let (gate_word_count, workgroup_count, dispatch_x, dispatch_y) =
            gate_dispatch_layout(point_count, limits.max_compute_workgroups_per_dimension)?;
        // An arrow reaches as far from its anchor as its component says, an
        // error bar as far as its error and a fill to its base, so no
        // per-axis gate bounds them: such a series scans every point.
        // A closed line's closing segment joins its last word to its first,
        // which the per-word gates do not see: it scans every point too.
        let direct_scan = point_count <= GPU_PICK_DIRECT_SCAN_POINTS
            || flags & FLAG_VECTOR != 0
            || errorbar.is_some()
            || area.is_some()
            || descriptor.line_closed;
        let allocated_gate_words = if direct_scan { 1 } else { gate_word_count };
        let gate_mask_bytes = u64::from(allocated_gate_words)
            .checked_mul(GATE_MASK_BYTES)
            .ok_or(GpuPickError::DeviceLimit {
                resource: "GPU pick gate-mask buffer",
                requested: u64::MAX,
                limit: limits.max_buffer_size,
            })?;
        let workgroup_candidate_bytes = u64::from(workgroup_count)
            .checked_mul(CANDIDATE_BYTES)
            .ok_or(GpuPickError::DeviceLimit {
                resource: "GPU pick workgroup-candidate buffer",
                requested: u64::MAX,
                limit: limits.max_buffer_size,
            })?;
        let storage_limit = limits.max_storage_buffer_binding_size;
        if gate_mask_bytes > limits.max_buffer_size || gate_mask_bytes > storage_limit {
            return Err(GpuPickError::DeviceLimit {
                resource: "GPU pick gate-mask buffer",
                requested: gate_mask_bytes,
                limit: limits.max_buffer_size.min(storage_limit),
            });
        }
        if workgroup_candidate_bytes > limits.max_buffer_size
            || workgroup_candidate_bytes > storage_limit
        {
            return Err(GpuPickError::DeviceLimit {
                resource: "GPU pick workgroup-candidate buffer",
                requested: workgroup_candidate_bytes,
                limit: limits.max_buffer_size.min(storage_limit),
            });
        }

        // Every buffer this slot owns is tallied by the creation helpers, so the
        // charge below cannot disagree with what was allocated.
        let series_tally = ChargeTally::new();
        let gate_masks = create_buffer_checked(
            &series_tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick XY gate masks"),
                size: gate_mask_bytes,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            },
            "XY gate-mask buffer",
        )?;
        let workgroup_candidates = create_buffer_checked(
            &series_tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick exact workgroup candidates"),
                size: workgroup_candidate_bytes,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            },
            "exact workgroup-candidate buffer",
        )?;
        let query_params = create_buffer_checked(
            &series_tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick query params"),
                size: std::mem::size_of::<PickQueryParamsGpu>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "query uniform",
        )?;
        let style_slots = create_rows_buffer_checked(
            &series_tally,
            &self.bundle.device,
            "figgy GPU pick style slots",
            [
                bytemuck::cast_slice(marker_slots),
                bytemuck::cast_slice(bar_slots),
            ],
            std::mem::size_of::<ScatterStyleSlotGpu>() as u64,
            "style slot buffer",
        )?;
        let style_overrides = create_rows_buffer_checked(
            &series_tally,
            &self.bundle.device,
            "figgy GPU pick style overrides",
            [
                bytemuck::cast_slice(marker_overrides),
                bytemuck::cast_slice(bar_overrides),
            ],
            std::mem::size_of::<ScatterStyleOverrideGpu>() as u64,
            "style override buffer",
        )?;
        let result = create_buffer_checked(
            &series_tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick series scalar"),
                size: CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
            "series pick scalar",
        )?;

        let query_data_bg = create_bind_group_checked(
            &self.bundle.device,
            &wgpu::BindGroupDescriptor {
                label: Some("figgy GPU pick query data bg"),
                layout: &self.bundle.query_data_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: gate_masks.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: workgroup_candidates.as_entire_binding(),
                    },
                ],
            },
            "query data bind group",
        )?;
        let query_work_bg = create_bind_group_checked(
            &self.bundle.device,
            &wgpu::BindGroupDescriptor {
                label: Some("figgy GPU pick query work bg"),
                layout: &self.bundle.query_work_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: query_params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: style_slots.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: style_overrides.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: result.as_entire_binding(),
                    },
                ],
            },
            "query work bind group",
        )?;
        let reduce_bg = create_bind_group_checked(
            &self.bundle.device,
            &wgpu::BindGroupDescriptor {
                label: Some("figgy GPU pick per-series reduction bg"),
                layout: &self.bundle.reduce_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: workgroup_candidates.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: result.as_entire_binding(),
                    },
                ],
            },
            "per-series reduction bind group",
        )?;

        let identity = PickIdentity {
            source_id: descriptor.source_id,
            series_id: descriptor.series_id,
        };
        // One charge for the whole slot: the style rows and params live inside
        // the bind groups above, so per-buffer tracking cannot see them. The
        // tally already holds the sizes the device was handed, so there is no
        // second list of terms to keep in step with the creations.
        Ok(PickSeriesSlot {
            identity,
            gpu: PickSeriesGpu {
                _charge: shared_charge(
                    series_tally,
                    &self.bundle.ledger,
                    GpuResourceKind::PickScratch,
                ),
                pool_identity: resident.map(ColumnPool::identity),
                x_column: descriptor.x_column,
                y_column: descriptor.y_column,
                style_index_column,
                x_handle,
                y_handle,
                style_index_handle,
                pool_layout_generation: resident.map_or(0, ColumnPool::layout_generation),
                x_allocation_epoch,
                y_allocation_epoch,
                style_index_allocation_epoch,
                size_column,
                size_handle,
                size_allocation_epoch,
                vector: descriptor.vector,
                vector_handles,
                vector_epochs,
                vector_base,
                vector_count,
                size_base,
                size_len,
                size_domain,
                size_px,
                query_params,
                gate_masks,
                workgroup_candidates,
                query_data_bg,
                query_work_bg,
                reduce_bg,
                result,
                point_count,
                gate_word_count,
                dispatch_x,
                dispatch_y,
                direct_scan,
                flags,
                base_radius_px,
                base_shape_id,
                line_half_width_px,
                line_step_mode,
                line_closed: descriptor.line_closed && descriptor.line_width_px.is_some(),
                line_dash,
                line_dash_len,
                max_extent_px,
                style_count,
                override_count,
                style_index_base,
                style_index_len,
                exact_key: series_exact_key(
                    flags,
                    line_dash_len,
                    base_shape_id,
                    errorbar.is_some(),
                    area.is_some(),
                ),
                errorbar,
                area,
            },
        })
    }

    #[cfg(test)]
    pub(super) fn current_reuse_slots<'a>(
        &self,
        skip: Option<usize>,
    ) -> Vec<GpuPickRegistrySlot<'a>> {
        self.registry
            .slots
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != skip)
            .map(|(current_index, slot)| GpuPickRegistrySlot::Reuse {
                current_index,
                source_id: slot.identity.source_id.clone(),
                series_id: slot.identity.series_id.clone(),
            })
            .collect()
    }

    pub(super) fn check_pool_identity(
        &self,
        pool: &ColumnPool,
    ) -> Result<PoolIdentity, GpuPickError> {
        let target = pool.identity();
        if let Some(current) = self.registry.pool_identity.as_ref()
            && !current.same_instance(&target)
        {
            return Err(GpuPickError::ForeignColumnPool);
        }
        Ok(target)
    }

    pub(super) fn prepare_reused_gpu(
        &self,
        pool: &ColumnPool,
        target_pool_identity: &PoolIdentity,
        current: &PickSeriesSlot,
        next_series_id: &str,
    ) -> Result<PickSeriesGpu, GpuPickError> {
        let series = &current.gpu;
        if !series
            .pool_identity
            .as_ref()
            .is_some_and(|identity| identity.same_instance(target_pool_identity))
        {
            return Err(GpuPickError::ForeignColumnPool);
        }

        let relocated = |id: &ColumnId,
                         expected: ColumnHandle,
                         expected_epoch: u64|
         -> Result<ColumnHandle, GpuPickError> {
            let Some(current) = pool.handle_for(id) else {
                return Err(GpuPickError::StaleColumn {
                    series_id: next_series_id.to_owned(),
                    column_id: id.clone(),
                });
            };
            if pool.allocation_epoch(id) != Some(expected_epoch)
                || current.len_values != expected.len_values
                || current.byte_size != expected.byte_size
            {
                return Err(GpuPickError::StaleColumn {
                    series_id: next_series_id.to_owned(),
                    column_id: id.clone(),
                });
            }
            Ok(current)
        };

        let x_handle = relocated(&series.x_column, series.x_handle, series.x_allocation_epoch)?;
        let y_handle = relocated(&series.y_column, series.y_handle, series.y_allocation_epoch)?;
        let _ = lane_base(x_handle, next_series_id)?;
        let _ = lane_base(y_handle, next_series_id)?;
        let (style_index_handle, style_index_base, style_index_len) =
            if let Some(id) = series.style_index_column.as_ref() {
                let expected = series
                    .style_index_handle
                    .ok_or(GpuPickError::InvalidGpuResult)?;
                let expected_epoch = series
                    .style_index_allocation_epoch
                    .ok_or(GpuPickError::InvalidGpuResult)?;
                let current = relocated(id, expected, expected_epoch)?;
                let base = lane_base(current, next_series_id)?;
                let len =
                    u32::try_from(current.len_values).map_err(|_| GpuPickError::TooManyValues {
                        series_id: next_series_id.to_owned(),
                        count: current.len_values,
                    })?;
                (Some(current), base, len)
            } else {
                (None, 0, 0)
            };

        let (size_handle, size_base, size_len) = if let Some(id) = series.size_column.as_ref() {
            let expected = series.size_handle.ok_or(GpuPickError::InvalidGpuResult)?;
            let expected_epoch = series
                .size_allocation_epoch
                .ok_or(GpuPickError::InvalidGpuResult)?;
            let current = relocated(id, expected, expected_epoch)?;
            let base = lane_base(current, next_series_id)?;
            let len =
                u32::try_from(current.len_values).map_err(|_| GpuPickError::TooManyValues {
                    series_id: next_series_id.to_owned(),
                    count: current.len_values,
                })?;
            (Some(current), base, len)
        } else {
            (None, 0, 0)
        };

        let mut vector_handles = [None; 3];
        let mut vector_base = [0u32; 2];
        if let Some(arrows) = series.vector.as_ref() {
            let ids = [
                Some(&arrows.u_column),
                Some(&arrows.v_column),
                arrows.color_column.as_ref(),
            ];
            for (index, id) in ids.into_iter().enumerate() {
                let Some(id) = id else {
                    continue;
                };
                let expected =
                    series.vector_handles[index].ok_or(GpuPickError::InvalidGpuResult)?;
                let expected_epoch =
                    series.vector_epochs[index].ok_or(GpuPickError::InvalidGpuResult)?;
                let current = relocated(id, expected, expected_epoch)?;
                if index < 2 {
                    vector_base[index] = lane_base(current, next_series_id)?;
                }
                vector_handles[index] = Some(current);
            }
        }

        let mut errorbar_handles = [None; ERRORBAR_COLUMNS];
        if let Some(bars) = series.errorbar.as_ref() {
            for (index, id) in bars.columns.iter().enumerate() {
                let Some(id) = id else {
                    continue;
                };
                let expected = bars.handles[index].ok_or(GpuPickError::InvalidGpuResult)?;
                let expected_epoch = bars.epochs[index].ok_or(GpuPickError::InvalidGpuResult)?;
                errorbar_handles[index] = Some(relocated(id, expected, expected_epoch)?);
            }
        }
        let mut area_handle = None;
        if let Some(fill) = series.area.as_ref()
            && let Some(id) = fill.column.as_ref()
        {
            let expected = fill.handle.ok_or(GpuPickError::InvalidGpuResult)?;
            let expected_epoch = fill.epoch.ok_or(GpuPickError::InvalidGpuResult)?;
            area_handle = Some(relocated(id, expected, expected_epoch)?);
        }

        let storage_unchanged = pool.layout_generation() == series.pool_layout_generation
            && same_storage(x_handle, series.x_handle)
            && same_storage(y_handle, series.y_handle)
            && style_index_handle
                .zip(series.style_index_handle)
                .is_none_or(|(current, expected)| same_storage(current, expected))
            && style_index_handle.is_some() == series.style_index_handle.is_some()
            && size_handle
                .zip(series.size_handle)
                .is_none_or(|(current, expected)| same_storage(current, expected))
            && size_handle.is_some() == series.size_handle.is_some()
            && vector_handles
                .iter()
                .zip(&series.vector_handles)
                .all(|(current, expected)| same_optional_storage(*current, *expected))
            && series.errorbar.as_ref().is_none_or(|bars| {
                errorbar_handles
                    .iter()
                    .zip(&bars.handles)
                    .all(|(current, expected)| same_optional_storage(*current, *expected))
            })
            && series
                .area
                .as_ref()
                .is_none_or(|fill| same_optional_storage(area_handle, fill.handle));
        if storage_unchanged {
            return Ok(series.clone());
        }

        let query_data_bg = create_bind_group_checked(
            &self.bundle.device,
            &wgpu::BindGroupDescriptor {
                label: Some("figgy GPU pick rebound query data bg"),
                layout: &self.bundle.query_data_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: pool.buffer().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: series.gate_masks.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: series.workgroup_candidates.as_entire_binding(),
                    },
                ],
            },
            "rebound query data bind group",
        )?;

        let mut rebound = series.clone();
        rebound.pool_identity = Some(target_pool_identity.clone());
        rebound.x_handle = x_handle;
        rebound.y_handle = y_handle;
        rebound.style_index_handle = style_index_handle;
        rebound.pool_layout_generation = pool.layout_generation();
        rebound.style_index_base = style_index_base;
        rebound.style_index_len = style_index_len;
        rebound.size_handle = size_handle;
        rebound.size_base = size_base;
        rebound.size_len = size_len;
        rebound.vector_handles = vector_handles;
        rebound.vector_base = vector_base;
        if let Some(bars) = rebound.errorbar.as_mut() {
            bars.rebase(errorbar_handles, next_series_id)?;
        }
        if let Some(fill) = rebound.area.as_mut()
            && let Some(handle) = area_handle
        {
            fill.params[0] = lane_base(handle, next_series_id)?;
            fill.handle = Some(handle);
        }
        rebound.query_data_bg = query_data_bg;
        Ok(rebound)
    }

    pub(super) fn prepare_next_registry<'a>(
        &self,
        pool: &ColumnPool,
        final_slots: impl IntoIterator<Item = GpuPickRegistrySlot<'a>>,
    ) -> Result<PickRegistry, GpuPickError> {
        #[cfg(test)]
        if self.fail_next_registry_prepare.replace(false) {
            return Err(GpuPickError::AllocationFailed {
                resource: "injected registry prepare failure",
            });
        }
        let storage_limit = self.bundle.device.limits().max_storage_buffer_binding_size;
        if pool.capacity() > storage_limit {
            return Err(GpuPickError::DeviceLimit {
                resource: "column-pool storage binding",
                requested: pool.capacity(),
                limit: storage_limit,
            });
        }
        let target_pool_identity = self.check_pool_identity(pool)?;

        let iterator = final_slots.into_iter();
        let mut plans = Vec::new();
        plans
            .try_reserve(iterator.size_hint().0)
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "registry transition plan",
            })?;
        plans.extend(iterator);

        let mut seen = Vec::new();
        seen.try_reserve_exact(self.registry.slots.len())
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "registry transition index map",
            })?;
        seen.resize(self.registry.slots.len(), false);

        let mut slots = Vec::new();
        slots
            .try_reserve_exact(plans.len())
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "registry successor slots",
            })?;
        for plan in plans {
            let slot = match plan {
                GpuPickRegistrySlot::Reuse {
                    current_index,
                    source_id,
                    series_id,
                } => {
                    let len = self.registry.slots.len();
                    let Some(current) = self.registry.slots.get(current_index) else {
                        return Err(GpuPickError::InvalidSeriesIndex {
                            index: current_index,
                            len,
                        });
                    };
                    if std::mem::replace(&mut seen[current_index], true) {
                        return Err(GpuPickError::DuplicateSeriesIndex {
                            index: current_index,
                        });
                    }
                    PickSeriesSlot {
                        gpu: self.prepare_reused_gpu(
                            pool,
                            &target_pool_identity,
                            current,
                            &series_id,
                        )?,
                        identity: PickIdentity {
                            source_id,
                            series_id,
                        },
                    }
                }
                GpuPickRegistrySlot::Build(descriptor) => self.prepare_series(pool, descriptor)?,
            };
            slots.push(slot);
        }

        let mut identities = Vec::new();
        identities
            .try_reserve_exact(slots.len())
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "registry identity snapshot",
            })?;
        identities.extend(slots.iter().map(|slot| slot.identity.clone()));
        let reduce = self.prepare_reduce_resources(slots.len())?;
        let generation = self
            .registry
            .generation
            .checked_add(1)
            .ok_or(GpuPickError::RegistryGenerationExhausted)?;

        Ok(PickRegistry {
            slots,
            identities: identities.into(),
            reduce,
            pool_identity: Some(target_pool_identity),
            pool_layout_generation: pool.layout_generation(),
            generation,
        })
    }

    pub(crate) fn prepare_registry_transition<'engine, 'a>(
        &'engine mut self,
        pool: &ColumnPool,
        final_slots: impl IntoIterator<Item = GpuPickRegistrySlot<'a>>,
    ) -> Result<PreparedPickRegistryTransition<'engine>, GpuPickError> {
        let next = self.prepare_next_registry(pool, final_slots)?;
        Ok(PreparedPickRegistryTransition {
            engine: self,
            next: Some(next),
        })
    }

    /// Replace one registry slot without recreating any other series resources.
    ///
    /// The replacement is built first, so a build/allocation error leaves the
    /// existing slot untouched. The returned id is the unchanged positional
    /// slot; later-series equal-distance precedence is therefore preserved.
    #[cfg(test)]
    pub(super) fn replace_series_at(
        &mut self,
        index: usize,
        pool: &ColumnPool,
        descriptor: GpuPickSeriesDescriptor<'_>,
    ) -> Result<GpuPickSeriesId, GpuPickError> {
        let len = self.registry.slots.len();
        if index >= len {
            return Err(GpuPickError::InvalidSeriesIndex { index, len });
        }
        let mut final_slots = self.current_reuse_slots(Some(index));
        final_slots.insert(index, GpuPickRegistrySlot::Build(descriptor));
        self.prepare_registry_transition(pool, final_slots)?
            .commit();
        Ok(GpuPickSeriesId(index as u32))
    }

    /// Insert one registry slot without recreating existing series resources.
    ///
    /// The new series is built completely at the tail before it is moved into
    /// position, so a build/allocation error leaves the registry and its tie
    /// order untouched. An index equal to the registry length appends it.
    #[cfg(test)]
    pub(super) fn insert_series_at(
        &mut self,
        index: usize,
        pool: &ColumnPool,
        descriptor: GpuPickSeriesDescriptor<'_>,
    ) -> Result<GpuPickSeriesId, GpuPickError> {
        let len = self.registry.slots.len();
        if index > len {
            return Err(GpuPickError::InvalidSeriesIndex { index, len });
        }
        let mut final_slots = self.current_reuse_slots(None);
        final_slots.insert(index, GpuPickRegistrySlot::Build(descriptor));
        self.prepare_registry_transition(pool, final_slots)?
            .commit();
        Ok(GpuPickSeriesId(index as u32))
    }

    /// Remove one positional registry slot without recreating surviving slots.
    /// Later slots shift left, matching their new production traversal order.
    #[cfg(test)]
    pub(super) fn remove_series_at(&mut self, index: usize) -> Result<(), GpuPickError> {
        let len = self.registry.slots.len();
        if index >= len {
            return Err(GpuPickError::InvalidSeriesIndex { index, len });
        }
        let generation = self
            .registry
            .generation
            .checked_add(1)
            .ok_or(GpuPickError::RegistryGenerationExhausted)?;
        let mut slots = self.registry.slots.clone();
        slots.remove(index);
        let identities = slots
            .iter()
            .map(|slot| slot.identity.clone())
            .collect::<Vec<_>>()
            .into();
        let next = PickRegistry {
            slots,
            identities,
            reduce: self.registry.reduce.clone(),
            pool_identity: self.registry.pool_identity.clone(),
            pool_layout_generation: self.registry.pool_layout_generation,
            generation,
        };
        PreparedPickRegistryTransition {
            engine: self,
            next: Some(next),
        }
        .commit();
        Ok(())
    }

    /// Drop every registered series while retaining reusable pipelines and
    /// reduction storage.
    #[cfg(test)]
    pub(super) fn clear_series(&mut self) {
        let generation = self
            .registry
            .generation
            .checked_add(1)
            .expect("GPU pick registry generation exhausted while clearing");
        let next = PickRegistry {
            slots: Vec::new(),
            identities: Arc::from([]),
            reduce: self.registry.reduce.clone(),
            pool_identity: self.registry.pool_identity.clone(),
            pool_layout_generation: self.registry.pool_layout_generation,
            generation,
        };
        PreparedPickRegistryTransition {
            engine: self,
            next: Some(next),
        }
        .commit();
    }

    /// Rebind registered columns after an in-place [`ColumnPool::defragment`].
    ///
    /// Defragmentation changes the backing buffer, offsets, and generations,
    /// but preserves every value. This method verifies every referenced column
    /// still exists with exactly its registered logical length, recreates the
    /// whole-pool data bind groups, and updates all handles/bases without
    /// reallocating gate or reduction storage.
    ///
    /// Use this only with the same `ColumnPool` after a relocation-only
    /// defragment. It is not valid after replacing column contents, even when
    /// ids and lengths happen to match; call replace_series_at/rebuild instead.
    #[cfg(test)]
    pub(super) fn rebind_columns(&mut self, pool: &ColumnPool) -> Result<(), GpuPickError> {
        let final_slots = self.current_reuse_slots(None);
        self.prepare_registry_transition(pool, final_slots)?
            .commit();
        Ok(())
    }

    pub(super) fn validate_series_columns(
        pool: &ColumnPool,
        slot: &PickSeriesSlot,
    ) -> Result<(), GpuPickError> {
        let series = &slot.gpu;
        if !series
            .pool_identity
            .as_ref()
            .is_some_and(|identity| identity.same_instance(&pool.identity()))
        {
            return Err(GpuPickError::ForeignColumnPool);
        }
        if pool.layout_generation() != series.pool_layout_generation {
            return Err(GpuPickError::StaleColumn {
                series_id: slot.identity.series_id.clone(),
                column_id: series.x_column.clone(),
            });
        }

        for (id, expected, expected_epoch) in [
            (&series.x_column, series.x_handle, series.x_allocation_epoch),
            (&series.y_column, series.y_handle, series.y_allocation_epoch),
        ] {
            let Some(current) = pool.handle_for(id) else {
                return Err(GpuPickError::StaleColumn {
                    series_id: slot.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            };
            if pool.allocation_epoch(id) != Some(expected_epoch) || !same_storage(current, expected)
            {
                return Err(GpuPickError::StaleColumn {
                    series_id: slot.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            }
        }
        for (id, expected, expected_epoch) in [
            (
                series.style_index_column.as_ref(),
                series.style_index_handle,
                series.style_index_allocation_epoch,
            ),
            (
                series.size_column.as_ref(),
                series.size_handle,
                series.size_allocation_epoch,
            ),
            (
                series.vector.as_ref().map(|vector| &vector.u_column),
                series.vector_handles[0],
                series.vector_epochs[0],
            ),
            (
                series.vector.as_ref().map(|vector| &vector.v_column),
                series.vector_handles[1],
                series.vector_epochs[1],
            ),
            (
                series
                    .vector
                    .as_ref()
                    .and_then(|vector| vector.color_column.as_ref()),
                series.vector_handles[2],
                series.vector_epochs[2],
            ),
        ]
        .into_iter()
        .chain(series.errorbar.iter().flat_map(|bars| {
            (0..ERRORBAR_COLUMNS).map(move |index| {
                (
                    bars.columns[index].as_ref(),
                    bars.handles[index],
                    bars.epochs[index],
                )
            })
        }))
        .chain(
            series
                .area
                .iter()
                .map(|fill| (fill.column.as_ref(), fill.handle, fill.epoch)),
        )
        .filter_map(|(id, expected, epoch)| Some((id?, expected?, epoch?)))
        {
            let Some(current) = pool.handle_for(id) else {
                return Err(GpuPickError::StaleColumn {
                    series_id: slot.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            };
            if pool.allocation_epoch(id) != Some(expected_epoch) || !same_storage(current, expected)
            {
                return Err(GpuPickError::StaleColumn {
                    series_id: slot.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            }
        }
        Ok(())
    }

    /// Submit one exact pick query and return an owned async readback ticket.
    #[cfg(test)]
    pub(super) fn pick(
        &self,
        pool: &ColumnPool,
        query: GpuPickQuery,
    ) -> Result<GpuPickTicket, GpuPickError> {
        self.pick_with_display_scale(pool, query, 1.0)
    }

    /// Submit one exact pick query using the display's uniform pixel scale.
    ///
    /// Series descriptors remain in logical-document pixels. This scale is
    /// applied once by the query shader to scatter radii, line half-widths,
    /// and the matching conservative gate widths. Cursor coordinates,
    /// `max_distance_px`, and returned distances remain physical canvas pixels.
    /// The registered series, in the order a pick addresses them.
    pub(crate) fn series_identities(&self) -> GpuPickSeriesIdentities {
        GpuPickSeriesIdentities(Arc::clone(&self.registry.identities))
    }

    #[cfg(test)]
    pub(crate) fn pick_with_display_scale(
        &self,
        pool: &ColumnPool,
        query: GpuPickQuery,
        display_scale: f32,
    ) -> Result<GpuPickTicket, GpuPickError> {
        let mut exact = PickExactPipelines::default();
        for key in self.exact_variants(&query) {
            exact.ensure(&self.bundle, key)?;
        }
        self.pick_with_arcs(pool, query, display_scale, &[], &exact)
    }

    /// The exact module and variant each registered series needs under
    /// `query`, in registry order: create them
    /// ([`PickExactPipelines::ensure`]) before [`Self::pick_with_arcs`].
    pub(crate) fn exact_variants<'a>(
        &'a self,
        query: &'a GpuPickQuery,
    ) -> impl Iterator<Item = PickExactKey> + 'a {
        self.registry
            .slots
            .iter()
            .map(move |slot| series_exact_variant(&slot.gpu, query))
    }

    /// The bundle the exact pipelines are created against.
    pub(crate) fn bundle(&self) -> &PickPipelineBundle {
        &self.bundle
    }

    /// [`Self::pick_with_display_scale`] with each registered series' arc
    /// prefix, by registry position: the per-point cumulative arc length the
    /// line shader drew the series with (`line_arc.wgsl`) for the query's
    /// transform, which places its dash phase and sketch wobble. A missing
    /// entry, or a buffer too short for the series, picks without it.
    /// `exact` must hold every variant [`Self::exact_variants`] names.
    pub(crate) fn pick_with_arcs(
        &self,
        pool: &ColumnPool,
        query: GpuPickQuery,
        display_scale: f32,
        arcs: &[Option<Arc<wgpu::Buffer>>],
        exact: &PickExactPipelines,
    ) -> Result<GpuPickTicket, GpuPickError> {
        let [cursor_x, cursor_y] = query.canvas_position_px;
        if !display_scale.is_finite()
            || display_scale <= 0.0
            || !cursor_x.is_finite()
            || !cursor_y.is_finite()
            || !query.max_distance_px.is_finite()
            || query.max_distance_px < 0.0
        {
            return Ok(GpuPickTicket::ready_none());
        }
        let [chart_x, chart_y, chart_width, chart_height] = query.chart_rect_px;
        if !chart_x.is_finite()
            || !chart_y.is_finite()
            || !chart_width.is_finite()
            || !chart_height.is_finite()
            || chart_width <= 0.0
            || chart_height <= 0.0
        {
            return Ok(GpuPickTicket::ready_none());
        }
        if let Some([x, y, width, height]) = query.data_area_px {
            let x1 = x + width;
            let y1 = y + height;
            if !x.is_finite()
                || !y.is_finite()
                || !width.is_finite()
                || !height.is_finite()
                || cursor_x < x
                || cursor_x > x1
                || cursor_y < y
                || cursor_y > y1
            {
                return Ok(GpuPickTicket::ready_none());
            }
        }
        if let Some(expected) = self.registry.pool_identity.as_ref()
            && !expected.same_instance(&pool.identity())
        {
            return Err(GpuPickError::ForeignColumnPool);
        }
        if self.registry.slots.is_empty() {
            return Ok(GpuPickTicket::ready_none());
        }
        if self.registry.pool_layout_generation != pool.layout_generation() {
            let first = &self.registry.slots[0];
            return Err(GpuPickError::StaleColumn {
                series_id: first.identity.series_id.clone(),
                column_id: first.gpu.x_column.clone(),
            });
        }

        for (series_order, slot) in self.registry.slots.iter().enumerate() {
            Self::validate_series_columns(pool, slot)?;
            let series = &slot.gpu;
            let params = query_params(
                series,
                &slot.identity.series_id,
                &query,
                display_scale,
                series_order as u32,
                [
                    series.point_count,
                    lane_base(series.x_handle, &slot.identity.series_id)?,
                    lane_base(series.y_handle, &slot.identity.series_id)?,
                    series.style_index_base,
                ],
                [
                    0,
                    series.point_count,
                    // A closed line owns its closing segment too.
                    if series.line_closed {
                        series.point_count
                    } else {
                        series.point_count.saturating_sub(1)
                    },
                    0,
                ],
                series_arc(arcs, series_order, series.point_count).is_some(),
            );
            self.bundle
                .queue
                .write_buffer(&series.query_params, 0, bytemuck::bytes_of(&params));
        }

        let readback_tally = ChargeTally::new();
        let readback = create_buffer_checked(
            &readback_tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick detached readback"),
                size: CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "detached pick readback",
        )?;
        let mut encoder =
            self.bundle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy exact GPU pick encoder"),
                });
        // Reduction storage may intentionally be larger than the live registry
        // after replace/remove. Clear every slot so a prior query's candidate
        // can never participate as a stale tail entry.
        encoder.clear_buffer(&self.registry.reduce.candidates, 0, None);
        let has_gated_series = self.registry.slots.iter().any(|slot| !slot.gpu.direct_scan);
        if has_gated_series {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact GPU pick X gate"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.gate_x);
            for slot in &self.registry.slots {
                let series = &slot.gpu;
                if series.direct_scan {
                    continue;
                }
                pass.set_bind_group(0, &series.query_data_bg, &[]);
                pass.set_bind_group(1, &series.query_work_bg, &[]);
                pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
            }
        }
        if has_gated_series {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact GPU pick Y gate"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.gate_y);
            for slot in &self.registry.slots {
                let series = &slot.gpu;
                if series.direct_scan {
                    continue;
                }
                pass.set_bind_group(0, &series.query_data_bg, &[]);
                pass.set_bind_group(1, &series.query_work_bg, &[]);
                pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact GPU pick candidate scan"),
                timestamp_writes: None,
            });
            for (series_order, slot) in self.registry.slots.iter().enumerate() {
                let series = &slot.gpu;
                let key = series_exact_variant(series, &query);
                pass.set_pipeline(exact.get(key).ok_or(GpuPickError::InvalidGpuResult)?);
                pass.set_bind_group(0, &series.query_data_bg, &[]);
                pass.set_bind_group(1, &series.query_work_bg, &[]);
                // The pass keeps the bind group alive until it is submitted.
                let arc = series_arc(arcs, series_order, series.point_count)
                    .map(|buffer| self.bundle.arc_bind_group(buffer));
                pass.set_bind_group(2, arc.as_ref().unwrap_or(&self.bundle.arc_placeholder), &[]);
                pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact GPU pick per-series reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.reduce);
            for slot in &self.registry.slots {
                let series = &slot.gpu;
                pass.set_bind_group(0, &series.reduce_bg, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
        for (index, slot) in self.registry.slots.iter().enumerate() {
            let series = &slot.gpu;
            encoder.copy_buffer_to_buffer(
                &series.result,
                0,
                &self.registry.reduce.candidates,
                index as u64 * CANDIDATE_BYTES,
                CANDIDATE_BYTES,
            );
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy GPU pick cross-series reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.reduce);
            pass.set_bind_group(0, &self.registry.reduce.bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(
            &self.registry.reduce.final_result,
            0,
            &readback,
            0,
            CANDIDATE_BYTES,
        );
        self.bundle.queue.submit(std::iter::once(encoder.finish()));

        let slice = readback.slice(..CANDIDATE_BYTES);
        let (sender, receiver) = oneshot::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        Ok(GpuPickTicket {
            state: GpuPickTicketState::Pending {
                device: Arc::clone(&self.bundle.device),
                readback,
                receiver,
                identities: Arc::clone(&self.registry.identities),
                _charge: readback_tally.into_charge(&self.bundle.ledger, GpuResourceKind::Readback),
                _scratch_charge: None,
            },
        })
    }
}

pub(super) enum GpuPickTicketState {
    ReadyNone,
    Pending {
        device: Arc<wgpu::Device>,
        readback: wgpu::Buffer,
        receiver: oneshot::Receiver<Result<(), wgpu::BufferAsyncError>>,
        identities: Arc<[PickIdentity]>,
        /// Credited when the ticket resolves or is abandoned — either way the
        /// MAP_READ buffer goes with it.
        _charge: GpuByteCharge,
        _scratch_charge: Option<SharedCharge>,
    },
}

/// Owned, detached asynchronous scalar readback.
pub struct GpuPickTicket {
    pub(super) state: GpuPickTicketState,
}

pub(crate) struct GpuIndexedPick {
    pub(crate) source_id: Option<String>,
    pub(crate) series_id: String,
    pub(crate) series_order: u32,
    pub(crate) point_index: u32,
    pub(crate) distance_px: f32,
}

impl GpuPickTicket {
    pub(super) fn ready_none() -> Self {
        Self {
            state: GpuPickTicketState::ReadyNone,
        }
    }

    pub async fn resolve(self) -> Result<Option<PickedPoint>, GpuPickError> {
        Ok(self.resolve_indexed().await?.map(|picked| PickedPoint {
            source_id: picked.source_id,
            series_id: picked.series_id,
            point_index: picked.point_index as usize,
            distance_px: picked.distance_px,
        }))
    }

    pub(crate) async fn resolve_indexed(self) -> Result<Option<GpuIndexedPick>, GpuPickError> {
        let GpuPickTicketState::Pending {
            device: _device,
            readback,
            receiver,
            identities,
            // Dropped here: resolving consumes the readback buffer.
            _charge,
            _scratch_charge,
        } = self.state
        else {
            return Ok(None);
        };

        #[cfg(not(target_arch = "wasm32"))]
        let _ = _device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        receiver
            .await
            .map_err(|_| GpuPickError::MapChannelClosed)?
            .map_err(GpuPickError::MapFailed)?;
        let slice = readback.slice(..CANDIDATE_BYTES);
        let mapped = slice
            .get_mapped_range()
            .expect("GPU pick readback is mapped after map_async");
        let candidate = *bytemuck::from_bytes::<PickCandidateGpu>(&mapped);
        drop(mapped);
        readback.unmap();

        match candidate.valid {
            0 => Ok(None),
            2 => Err(GpuPickError::TraversalOverflow),
            1 => {
                let identity = identities
                    .get(candidate.series_order as usize)
                    .ok_or(GpuPickError::InvalidGpuResult)?;
                if !candidate.distance_px.is_finite() {
                    return Err(GpuPickError::InvalidGpuResult);
                }
                Ok(Some(GpuIndexedPick {
                    source_id: identity.source_id.clone(),
                    series_id: identity.series_id.clone(),
                    series_order: candidate.series_order,
                    point_index: candidate.point_index,
                    distance_px: candidate.distance_px,
                }))
            }
            _ => Err(GpuPickError::InvalidGpuResult),
        }
    }
}
