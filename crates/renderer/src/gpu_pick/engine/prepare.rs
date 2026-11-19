//! Preparing one series' query slot.

use super::*;

impl GpuPickEngine {
    /// Prepare one persistent exact query slot from `pool`.
    ///
    /// Every later [`Self::pick`] must receive that same pool instance. Replace
    /// this slot after any referenced column content replacement. Allocation
    /// epochs distinguish remove+readd even when the replacement reuses the
    /// same offset and length.
    #[cfg(test)]
    pub(in crate::gpu_pick) fn add_series(
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
    pub(in crate::gpu_pick) fn prepare_series(
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

    pub(in crate::gpu_pick) fn prepare_series_buffer(
        &self,
        buffer: &wgpu::Buffer,
        descriptor: GpuPickSeriesDescriptor<'_>,
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

        let (descriptor, mut flags, vector_handles, vector_epochs, vector_base, vector_count) =
            Self::prepare_vector_arrows(descriptor, columns, resident, paired_len, flags)?;

        let (descriptor, bar_slots, bar_overrides, errorbar, area, stem, ternary) =
            Self::prepare_resident_inputs(
                descriptor,
                buffer,
                resident,
                paired_len,
                marker_slots,
                marker_overrides,
            )?;

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
        // A region fill is picked by the crossings of the cursor's ray with
        // the polygons' edges, placed as the draw places them.
        let region = descriptor.ternary_region && ternary.is_some();
        if region {
            flags |= FLAG_REGION;
        }
        if flags & (FLAG_SCATTER | FLAG_LINE | FLAG_REGION) == 0 {
            return Err(GpuPickError::NoPickPrimitive(descriptor.series_id));
        }

        lane_base(x_handle, &descriptor.series_id)?;
        lane_base(y_handle, &descriptor.series_id)?;
        let (gate_word_count, workgroup_count, dispatch_x, dispatch_y) =
            gate_dispatch_layout(point_count, limits.max_compute_workgroups_per_dimension)?;
        // An arrow reaches as far from its anchor as its component says, an
        // error bar as far as its error and a fill to its base, so no
        // per-axis gate bounds them: such a series scans every point.
        // A closed line's closing segment joins its last word to its first,
        // which the per-word gates do not see: it scans every point too.
        // A ternary placement is not axis-separable either: the gates see
        // one axis at a time.
        let direct_scan = point_count <= GPU_PICK_DIRECT_SCAN_POINTS
            || flags & FLAG_VECTOR != 0
            || errorbar.is_some()
            || stem.is_some()
            || area.is_some()
            || ternary.is_some()
            || descriptor.line_closed;
        // A region series' exact pass writes a region entry (its crossing
        // parity) per workgroup after the best candidates.
        let candidate_slots = if region { 2 } else { 1 };
        let allocated_gate_words = if direct_scan { 1 } else { gate_word_count };
        let gate_mask_bytes = u64::from(allocated_gate_words)
            .checked_mul(GATE_MASK_BYTES)
            .ok_or(GpuPickError::DeviceLimit {
                resource: "GPU pick gate-mask buffer",
                requested: u64::MAX,
                limit: limits.max_buffer_size,
            })?;
        let workgroup_candidate_bytes = u64::from(workgroup_count)
            .checked_mul(CANDIDATE_BYTES * candidate_slots)
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

        let (
            series_tally,
            gate_masks,
            workgroup_candidates,
            query_params,
            result,
            query_data_bg,
            query_work_bg,
            reduce_bg,
            style_rows,
        ) = self.create_slot_resources(
            buffer,
            gate_mask_bytes,
            workgroup_candidate_bytes,
            marker_slots,
            bar_slots,
            marker_overrides,
            bar_overrides,
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
                style_rows,
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
                path_arrows: descriptor
                    .path_arrows
                    .filter(|_| descriptor.line_width_px.is_some()),
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
                    errorbar.is_some() || stem.is_some(),
                    area.is_some(),
                    descriptor.path_arrows.is_some() && descriptor.line_width_px.is_some(),
                    descriptor.category_jitter.is_some() && descriptor.scatter.is_some(),
                    ternary.is_some(),
                ),
                errorbar,
                area,
                stem,
                category_jitter: descriptor
                    .category_jitter
                    .filter(|_| descriptor.scatter.is_some()),
                ternary,
            },
        })
    }

    /// [`Self::prepare_series_buffer`]'s point vector arrows.
    fn prepare_vector_arrows<'a>(
        descriptor: GpuPickSeriesDescriptor<'a>,
        columns: GpuStreamPickColumns,
        resident: Option<&ColumnPool>,
        paired_len: usize,
        mut flags: u32,
    ) -> Result<
        (
            GpuPickSeriesDescriptor<'a>,
            u32,
            [Option<ColumnHandle>; 3],
            [Option<u64>; 3],
            [u32; 2],
            u32,
        ),
        GpuPickError,
    > {
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
        Ok((
            descriptor,
            flags,
            vector_handles,
            vector_epochs,
            vector_base,
            vector_count,
        ))
    }

    /// [`Self::prepare_series_buffer`]'s resident-only inputs: the error bars,
    /// the line's fill, the stems and a ternary series' third component.
    fn prepare_resident_inputs<'a>(
        mut descriptor: GpuPickSeriesDescriptor<'a>,
        buffer: &wgpu::Buffer,
        resident: Option<&ColumnPool>,
        paired_len: usize,
        marker_slots: &[ScatterStyleSlotGpu],
        marker_overrides: &[ScatterStyleOverrideGpu],
    ) -> Result<
        (
            GpuPickSeriesDescriptor<'a>,
            &'a [ScatterStyleSlotGpu],
            &'a [ScatterStyleOverrideGpu],
            Option<PickErrorBarGpu>,
            Option<PickAreaGpu>,
            Option<GpuPickStem>,
            Option<PickTernaryGpu>,
        ),
        GpuPickError,
    > {
        // The error bars and the line's fill read pool columns: resident
        // series only. A streamed chunk does not carry them (streaming is not
        // extended, rule 8; `encode_chunk_impl` drops them).
        let errorbar_input = descriptor.errorbar.take();
        let area_input = descriptor.area.take();
        // Stems read only the anchors, but like the error bars they are
        // resident only (streaming is not extended to them, rule 8) and part
        // of the scatter primitive (#206).
        let stem = descriptor
            .stem
            .take()
            .filter(|_| resident.is_some() && descriptor.scatter.is_some());
        let ternary_input = descriptor.ternary_c.take();
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

        // A ternary series' third component: a resident column, read at
        // each sample by the exact pass.
        let ternary = match ternary_input {
            Some(id) => {
                let (handle, epoch) = resolve(&id)?;
                Some(PickTernaryGpu {
                    base: lane_base(handle, &descriptor.series_id)?,
                    len: u32::try_from(handle.len_values).unwrap_or(u32::MAX),
                    column: id,
                    handle,
                    epoch,
                })
            }
            None => None,
        };
        Ok((
            descriptor,
            bar_slots,
            bar_overrides,
            errorbar,
            area,
            stem,
            ternary,
        ))
    }

    /// [`Self::prepare_series_buffer`]'s owned buffers and bind groups.
    #[allow(clippy::too_many_arguments)]
    fn create_slot_resources(
        &self,
        buffer: &wgpu::Buffer,
        gate_mask_bytes: u64,
        workgroup_candidate_bytes: u64,
        marker_slots: &[ScatterStyleSlotGpu],
        bar_slots: &[ScatterStyleSlotGpu],
        marker_overrides: &[ScatterStyleOverrideGpu],
        bar_overrides: &[ScatterStyleOverrideGpu],
    ) -> Result<
        (
            ChargeTally,
            wgpu::Buffer,
            wgpu::Buffer,
            wgpu::Buffer,
            wgpu::Buffer,
            wgpu::BindGroup,
            wgpu::BindGroup,
            wgpu::BindGroup,
            [wgpu::Buffer; 2],
        ),
        GpuPickError,
    > {
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
        Ok((
            series_tally,
            gate_masks,
            workgroup_candidates,
            query_params,
            result,
            query_data_bg,
            query_work_bg,
            reduce_bg,
            [style_slots, style_overrides],
        ))
    }
}
