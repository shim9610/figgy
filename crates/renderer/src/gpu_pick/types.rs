//! GPU-facing plain structs and descriptors of the point picker: the series
//! and query inputs, the uniform and storage layouts, the error type and the
//! style helpers.

use super::*;

pub(super) const GATE_MASK_BYTES: u64 = 8;
pub(super) const CANDIDATE_BYTES: u64 = 32;

pub(crate) const FLAG_SCATTER: u32 = 1;
pub(crate) const FLAG_LINE: u32 = 2;
pub(crate) const FLAG_STYLE_MAP: u32 = 4;
pub(crate) const FLAG_STYLE_INDEX: u32 = 8;
/// A `point_size_map` is bound: the exact pass resolves each point's radius
/// from its size value, and the gate extent includes the map's `max_px`.
pub(crate) const FLAG_SIZE_MAP: u32 = 16;
/// The size map scales marker area rather than radius.
pub(crate) const FLAG_SIZE_AREA: u32 = 32;
/// The scatter primitive is a point vector's arrows (`PICK_FLAG_VECTOR`).
pub(crate) const FLAG_VECTOR: u32 = 64;

/// `PICK_VECTOR_SCALE_PIXELS` in `PickQueryParams.vector.w`.
pub(crate) const VECTOR_SCALE_PIXELS: u32 = 256;
pub(super) use crate::data_render::{STYLE_MASK_RADIUS, STYLE_MASK_SHAPE};
/// The pick shader's marker draw modes (`PICK_MARKER_*`).
pub(crate) const PICK_MARKER_PRECISE: u32 = 0;
pub(crate) const PICK_MARKER_SKETCH: u32 = 1;
pub(crate) const PICK_MARKER_PLANET: u32 = 2;
pub(crate) const PICK_MARKER_STAR: u32 = 3;
/// The pick shader's line draw modes (`PICK_LINE_*`).
pub(crate) const PICK_LINE_PRECISE: u32 = 0;
pub(crate) const PICK_LINE_SKETCH: u32 = 1;
pub(crate) const PICK_LINE_RIBBON: u32 = 2;

#[derive(Clone, Debug)]
pub enum GpuPickError {
    PickerDisabled,
    UnknownChart(crate::renderer::ChartId),
    DeviceLimit {
        resource: &'static str,
        requested: u64,
        limit: u64,
    },
    AllocationFailed {
        resource: &'static str,
    },
    AsyncCompileFailed(String),
    MissingColumn(ColumnId),
    ColumnNotResident(ColumnId),
    StaleColumn {
        series_id: String,
        column_id: ColumnId,
    },
    EmptySeries(String),
    NoPickPrimitive(String),
    InvalidStyleMap {
        series_id: String,
        reason: &'static str,
    },
    TooManyValues {
        series_id: String,
        count: usize,
    },
    /// Retained for source compatibility with the former traversal picker.
    /// The streaming gate implementation never emits this error.
    TraversalOverflow,
    MapChannelClosed,
    MapFailed(wgpu::BufferAsyncError),
    InvalidGpuResult,
    InvalidSeriesIndex {
        index: usize,
        len: usize,
    },
    DuplicateSeriesIndex {
        index: usize,
    },
    ForeignColumnPool,
    RegistryGenerationExhausted,
}

impl fmt::Display for GpuPickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PickerDisabled => write!(f, "GPU picking is not enabled for this renderer"),
            Self::UnknownChart(id) => write!(f, "unknown GPU pick chart id: {id:?}"),
            Self::DeviceLimit {
                resource,
                requested,
                limit,
            } => write!(
                f,
                "GPU pick {resource} exceeds a device limit: requested {requested}, limit {limit}"
            ),
            Self::AllocationFailed { resource } => {
                write!(f, "GPU pick could not allocate {resource}")
            }
            Self::AsyncCompileFailed(reason) => {
                write!(f, "GPU pick pipeline compile failed: {reason}")
            }
            Self::MissingColumn(id) => write!(f, "GPU pick column is missing: {id}"),
            Self::ColumnNotResident(id) => {
                write!(f, "GPU pick column is registered but not resident: {id}")
            }
            Self::StaleColumn {
                series_id,
                column_id,
            } => write!(
                f,
                "GPU picker for series {series_id:?} is stale because column {column_id:?} moved, was removed, or changed length"
            ),
            Self::EmptySeries(id) => write!(f, "GPU pick series {id:?} has no paired values"),
            Self::NoPickPrimitive(id) => {
                write!(
                    f,
                    "GPU pick series {id:?} has neither scatter nor line geometry"
                )
            }
            Self::InvalidStyleMap { series_id, reason } => {
                write!(
                    f,
                    "GPU pick style map for series {series_id:?} is invalid: {reason}"
                )
            }
            Self::TooManyValues { series_id, count } => write!(
                f,
                "GPU pick series {series_id:?} has {count} values; the WebGPU storage index limit is {}",
                u32::MAX
            ),
            Self::TraversalOverflow => write!(f, "legacy GPU pick traversal overflow"),
            Self::MapChannelClosed => write!(f, "GPU pick readback callback was dropped"),
            Self::MapFailed(error) => write!(f, "GPU pick readback mapping failed: {error:?}"),
            Self::InvalidGpuResult => write!(f, "GPU pick returned an invalid scalar record"),
            Self::InvalidSeriesIndex { index, len } => write!(
                f,
                "GPU pick series index {index} is out of bounds for {len} registered series"
            ),
            Self::DuplicateSeriesIndex { index } => {
                write!(
                    f,
                    "GPU pick series index {index} occurs more than once in a batch"
                )
            }
            Self::ForeignColumnPool => {
                write!(f, "GPU picker cannot use a different ColumnPool instance")
            }
            Self::RegistryGenerationExhausted => {
                write!(f, "GPU pick registry generation is exhausted")
            }
        }
    }
}

impl std::error::Error for GpuPickError {}

/// Exact per-point scatter-style inputs.
///
/// These are the small arrays already derived from `SeriesConfig` for the
/// precise render path.  They are uploaded once and cached with the series.
/// Passing `None` for [`GpuPickSeriesDescriptor::scatter`] disables scatter
/// picking.  Passing `style_map: None` selects the production picker's
/// non-mapped/base-style semantics (used by non-precise draw styles).
pub(crate) struct GpuPickScatterStyle<'a> {
    pub(crate) style_index_column: Option<ColumnId>,
    pub(crate) style_slots: &'a [ScatterStyleSlotGpu],
    pub(crate) style_overrides: &'a [ScatterStyleOverrideGpu],
    pub(crate) style_meta: ScatterStyleMapMeta,
}

/// A series' `point_size_map`, as the pick shader needs it: the column the
/// size values come from and the explicit domain → pixel mapping. Twin of
/// `PointSizeMap`; the radius is re-derived per point in the exact pass.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GpuPickSizeMap {
    pub(crate) column: ColumnId,
    pub(crate) domain_min: f64,
    pub(crate) domain_max: f64,
    pub(crate) min_px: f32,
    pub(crate) max_px: f32,
    pub(crate) area: bool,
}

/// Scatter inputs for one registered series.
pub(crate) struct GpuPickScatter<'a> {
    pub(crate) base_radius_px: f32,
    pub(crate) base_shape_id: u32,
    /// `Some` only when current precise-mode style mapping is active.
    pub(crate) style_map: Option<GpuPickScatterStyle<'a>>,
    /// `Some` only in precise mode with a `point_size_map`.
    pub(crate) size_map: Option<GpuPickSizeMap>,
}

/// Registration-time metadata for one exact GPU-picked series.
pub(crate) struct GpuPickSeriesDescriptor<'a> {
    pub(crate) source_id: Option<String>,
    pub(crate) series_id: String,
    pub(crate) x_column: ColumnId,
    pub(crate) y_column: ColumnId,
    pub(crate) scatter: Option<GpuPickScatter<'a>>,
    /// Full production picker line width.  The engine applies `max(0) / 2`.
    pub(crate) line_width_px: Option<f32>,
    /// How the line connects its samples. A step mode makes the picker walk
    /// the same axis-aligned pieces the line shader draws.
    pub(crate) line_interpolation: crate::data_config::LineInterpolation,
    /// The line's dash preset: the picker walks the same dash phase the line
    /// shader draws, so a dash gap does not pick.
    pub(crate) line_style: crate::line::LineStylePreset,
    /// A point vector series: its markers are the arrows drawn from each
    /// anchor (`scatter` must be set too, for the scatter primitive slot).
    pub(crate) vector: Option<GpuPickVector>,
}

/// The arrows of a point vector series, as the vector shader draws them
/// (`vector.rs::VectorParamsGpu`): the component columns, the columns that
/// bound how many arrows are drawn, and the style in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GpuPickVector {
    pub(crate) u_column: ColumnId,
    pub(crate) v_column: ColumnId,
    /// A colour column: the vector shader draws only as many arrows as it
    /// has values.
    pub(crate) color_column: Option<ColumnId>,
    /// `VectorScale::Pixels` (else `Data`).
    pub(crate) scale_pixels: bool,
    /// The `Data` factor, or `px_per_unit` in logical pixels.
    pub(crate) scale: f32,
    /// `VECTOR_PIVOT_*`.
    pub(crate) pivot: u32,
    pub(crate) width_px: f32,
    pub(crate) head_length_px: f32,
    pub(crate) head_width_px: f32,
    pub(crate) min_length_px: f32,
}

/// What the chart's draw style draws for markers and lines, as the pick
/// shader's `styled` modes: the precise shapes and quads, the sketch wobble,
/// Milkyway planets and ribbons, or Constellation stars over precise lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GpuPickDrawStyle {
    pub(super) marker: u32,
    pub(super) line: u32,
}

impl GpuPickDrawStyle {
    pub(crate) const PRECISE: Self = Self {
        marker: PICK_MARKER_PRECISE,
        line: PICK_LINE_PRECISE,
    };

    /// The marker mode, as the scatter shader's picked ring reads it
    /// (`RING_MARKER_*`, numbered like `PICK_MARKER_*`).
    pub(crate) fn marker_mode(self) -> u32 {
        self.marker
    }

    pub(crate) fn of(style: &crate::config::DrawStyle) -> Self {
        use crate::config::DrawStyle;
        match style {
            DrawStyle::Precise => Self::PRECISE,
            DrawStyle::Sketch(_) => Self {
                marker: PICK_MARKER_SKETCH,
                line: PICK_LINE_SKETCH,
            },
            DrawStyle::Milkyway(_) => Self {
                marker: PICK_MARKER_PLANET,
                line: PICK_LINE_RIBBON,
            },
            DrawStyle::Constellation(_) => Self {
                marker: PICK_MARKER_STAR,
                line: PICK_LINE_PRECISE,
            },
        }
    }
}

/// Query state which integration can derive directly from `Config`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GpuPickQuery {
    pub(crate) transform: ScatterTransform,
    /// `(x, y, width, height)` in canvas pixels.  Width/height must be the
    /// same chart-area values used to build `transform`.
    pub(crate) chart_rect_px: [f32; 4],
    /// Current data-area clip in canvas pixels.  Like the CPU picker, cursor
    /// positions outside it return `None` before GPU work is submitted.
    pub(crate) data_area_px: Option<[f32; 4]>,
    pub(crate) canvas_position_px: [f32; 2],
    pub(crate) max_distance_px: f32,
    /// The draw style the chart is drawn with.
    pub(crate) draw_style: GpuPickDrawStyle,
}

/// Handles address only the currently uploaded bounded pair buffer.
#[derive(Clone, Copy)]
pub(crate) struct GpuStreamPickColumns {
    pub(crate) x: ColumnHandle,
    pub(crate) y: ColumnHandle,
    pub(crate) style_index: Option<ColumnHandle>,
    /// The `point_size_map` value column, resident series only.
    pub(crate) size_value: Option<ColumnHandle>,
    /// A point vector's `u`, `v` and colour columns, resident series only.
    pub(crate) vector: [Option<ColumnHandle>; 3],
}

pub(crate) fn validate_stream_handle(
    buffer: &wgpu::Buffer,
    handle: ColumnHandle,
) -> Result<(), GpuPickError> {
    let end = (handle.len_values as u64)
        .checked_mul(8)
        .and_then(|bytes| handle.offset.checked_add(bytes));
    if handle.offset % 8 != 0 || end.is_none_or(|end| end > buffer.size()) {
        return Err(GpuPickError::DeviceLimit {
            resource: "pick column pair range",
            requested: end.unwrap_or(u64::MAX),
            limit: buffer.size(),
        });
    }
    Ok(())
}

/// Constant-size running GPU answer. Source bytes remain owned by the stream
/// upload ring; every encoded chunk is reduced before that slot is reusable.
pub(crate) struct GpuStreamPick {
    pub(super) engine: GpuPickEngine,
    pub(super) query: GpuPickQuery,
    pub(super) display_scale: f32,
    pub(super) identities: Arc<[PickIdentity]>,
    pub(super) has_chunks: bool,
    pub(super) enabled: bool,
}

impl GpuStreamPick {
    pub(crate) const PERSISTENT_BYTES: u64 = 3 * CANDIDATE_BYTES;
    pub(crate) const READBACK_BYTES: u64 = CANDIDATE_BYTES;

    /// Exact chunk scratch allocation, excluding the caller-owned uploaded
    /// pair buffer and the constant running accumulator.
    pub(crate) fn chunk_bytes(
        &self,
        descriptor: &GpuPickSeriesDescriptor<'_>,
        paired_count: usize,
    ) -> Result<u64, GpuPickError> {
        let count = u32::try_from(paired_count).map_err(|_| GpuPickError::TooManyValues {
            series_id: descriptor.series_id.clone(),
            count: paired_count,
        })?;
        if count == 0 || !self.enabled {
            return Ok(0);
        }
        let (words, workgroups, _, _) = gate_dispatch_layout(
            count,
            self.engine
                .bundle
                .device
                .limits()
                .max_compute_workgroups_per_dimension,
        )?;
        let map = descriptor
            .scatter
            .as_ref()
            .and_then(|scatter| scatter.style_map.as_ref());
        let slots = map.map_or(1, |map| map.style_slots.len().max(1)) as u64;
        let overrides = map.map_or(1, |map| map.style_overrides.len().max(1)) as u64;
        let gate_words = if count <= GPU_PICK_DIRECT_SCAN_POINTS {
            1
        } else {
            words
        };
        u64::from(gate_words)
            .checked_mul(GATE_MASK_BYTES)
            .and_then(|bytes| bytes.checked_add(u64::from(workgroups) * CANDIDATE_BYTES))
            .and_then(|bytes| {
                bytes
                    .checked_add(std::mem::size_of::<PickQueryParamsGpu>() as u64 + CANDIDATE_BYTES)
            })
            .and_then(|bytes| {
                slots
                    .checked_mul(std::mem::size_of::<ScatterStyleSlotGpu>() as u64)
                    .and_then(|slots| bytes.checked_add(slots))
            })
            .and_then(|bytes| {
                overrides
                    .checked_mul(std::mem::size_of::<ScatterStyleOverrideGpu>() as u64)
                    .and_then(|overrides| bytes.checked_add(overrides))
            })
            .ok_or(GpuPickError::AllocationFailed {
                resource: "stream pick chunk scratch bytes",
            })
    }

    /// The caller retains this charge until the final referencing submission
    /// completes, including cancellation with recorded commands still alive.
    #[cfg(test)]
    pub(crate) fn charge(&self) -> SharedCharge {
        self.engine.registry.reduce._charge.clone()
    }

    pub(crate) fn result_buffer(&self) -> Option<&wgpu::Buffer> {
        self.has_chunks
            .then_some(&self.engine.registry.reduce.final_result)
    }

    /// All owned indices begin at local zero. A line chunk includes one right
    /// halo point; `owned_points` excludes that halo and `owned_segments` names
    /// exactly the starts owned by this chunk. Style override rows keep global
    /// indices, whereas the style-index handle addresses this chunk's lane.
    /// Returned scratch charge must live through the chunk's GPU completion.
    /// `exact` is the renderer's exact pipelines, shared with resident picks.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_chunk(
        &mut self,
        exact: &mut PickExactPipelines,
        encoder: &mut wgpu::CommandEncoder,
        buffer: &wgpu::Buffer,
        descriptor: GpuPickSeriesDescriptor<'_>,
        columns: GpuStreamPickColumns,
        series_order: u32,
        global_point_start: u32,
        owned_points: u32,
        owned_segments: u32,
    ) -> Result<Option<SharedCharge>, GpuPickError> {
        self.encode_chunk_impl(
            exact,
            encoder,
            buffer,
            descriptor,
            columns,
            series_order,
            global_point_start,
            owned_points,
            owned_segments,
            0,
        )
    }

    /// The final u32 lane of the same packed GPU buffer maps local row slots
    /// to original source rows. No source replay or CPU row mirror is needed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_indexed_chunk(
        &mut self,
        exact: &mut PickExactPipelines,
        encoder: &mut wgpu::CommandEncoder,
        buffer: &wgpu::Buffer,
        descriptor: GpuPickSeriesDescriptor<'_>,
        columns: GpuStreamPickColumns,
        series_order: u32,
        original_index_word_base: u32,
        owned_points: u32,
        owned_segments: u32,
    ) -> Result<Option<SharedCharge>, GpuPickError> {
        self.encode_chunk_impl(
            exact,
            encoder,
            buffer,
            descriptor,
            columns,
            series_order,
            0,
            owned_points,
            owned_segments,
            original_index_word_base,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn encode_chunk_impl(
        &mut self,
        exact: &mut PickExactPipelines,
        encoder: &mut wgpu::CommandEncoder,
        buffer: &wgpu::Buffer,
        descriptor: GpuPickSeriesDescriptor<'_>,
        columns: GpuStreamPickColumns,
        series_order: u32,
        global_point_start: u32,
        owned_points: u32,
        owned_segments: u32,
        original_index_word_base: u32,
    ) -> Result<Option<SharedCharge>, GpuPickError> {
        if !self.enabled {
            return Ok(None);
        }
        let identity =
            self.identities
                .get(series_order as usize)
                .ok_or(GpuPickError::InvalidSeriesIndex {
                    index: series_order as usize,
                    len: self.identities.len(),
                })?;
        if identity.series_id != descriptor.series_id || identity.source_id != descriptor.source_id
        {
            return Err(GpuPickError::InvalidGpuResult);
        }
        let paired = columns.x.len_values.min(columns.y.len_values);
        if owned_points as usize > paired
            || owned_segments as usize > paired.saturating_sub(1)
            || global_point_start
                .checked_add(u32::try_from(paired.saturating_sub(1)).unwrap_or(u32::MAX))
                .is_none()
        {
            return Err(GpuPickError::TooManyValues {
                series_id: descriptor.series_id,
                count: paired,
            });
        }
        if owned_points == 0 && owned_segments == 0 {
            return Ok(None);
        }
        if original_index_word_base != 0 {
            let required = u64::from(original_index_word_base)
                .checked_add(paired as u64)
                .and_then(|words| words.checked_mul(4))
                .ok_or(GpuPickError::InvalidGpuResult)?;
            if required > buffer.size() {
                return Err(GpuPickError::InvalidGpuResult);
            }
        }
        let slot = self
            .engine
            .prepare_series_buffer(buffer, descriptor, columns, None)?;
        let series = &slot.gpu;
        // Streaming is not extended: a streamed line is picked without its
        // arc prefix, so its dash gaps pick like its dashes.
        let params = query_params(
            series,
            &slot.identity.series_id,
            &self.query,
            self.display_scale,
            series_order,
            [
                series.point_count,
                lane_base(columns.x, &slot.identity.series_id)?,
                lane_base(columns.y, &slot.identity.series_id)?,
                series.style_index_base,
            ],
            [
                global_point_start,
                owned_points,
                owned_segments,
                original_index_word_base,
            ],
            false,
        );
        self.engine
            .bundle
            .queue
            .write_buffer(&series.query_params, 0, bytemuck::bytes_of(&params));
        let bundle = &self.engine.bundle;
        let exact = exact.ensure(bundle, stream_exact_variant(series, &self.query))?;
        for (pipeline, enabled) in [
            (&bundle.gate_x, !series.direct_scan),
            (&bundle.gate_y, !series.direct_scan),
            (exact, true),
        ] {
            if !enabled {
                continue;
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy stream pick chunk scan"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &series.query_data_bg, &[]);
            pass.set_bind_group(1, &series.query_work_bg, &[]);
            pass.set_bind_group(2, &bundle.arc_placeholder, &[]);
            pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy stream pick chunk reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&bundle.reduce);
            pass.set_bind_group(0, &series.reduce_bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        let reduce = &self.engine.registry.reduce;
        if self.has_chunks {
            encoder.copy_buffer_to_buffer(
                &reduce.final_result,
                0,
                &reduce.candidates,
                0,
                CANDIDATE_BYTES,
            );
        } else {
            encoder.clear_buffer(&reduce.candidates, 0, None);
        }
        encoder.copy_buffer_to_buffer(
            &series.result,
            0,
            &reduce.candidates,
            CANDIDATE_BYTES,
            CANDIDATE_BYTES,
        );
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy stream pick running reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&bundle.reduce);
            pass.set_bind_group(0, &reduce.bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        self.has_chunks = true;
        Ok(Some(series._charge.clone()))
    }

    /// Call only after every required chunk has been encoded and submitted.
    pub(crate) fn finish(self) -> Result<GpuPickTicket, GpuPickError> {
        if !self.has_chunks {
            return Ok(GpuPickTicket::ready_none());
        }
        let bundle = &self.engine.bundle;
        let tally = ChargeTally::new();
        let readback = create_buffer_checked(
            &tally,
            &bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy stream pick scalar readback"),
                size: CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "stream pick readback",
        )?;
        let mut encoder = bundle
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy stream pick finish"),
            });
        encoder.copy_buffer_to_buffer(
            &self.engine.registry.reduce.final_result,
            0,
            &readback,
            0,
            CANDIDATE_BYTES,
        );
        bundle.queue.submit([encoder.finish()]);
        let (sender, receiver) = oneshot::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        Ok(GpuPickTicket {
            state: GpuPickTicketState::Pending {
                device: Arc::clone(&bundle.device),
                readback,
                receiver,
                identities: self.identities,
                _charge: tally.into_charge(&bundle.ledger, GpuResourceKind::Readback),
                _scratch_charge: Some(self.engine.registry.reduce._charge.clone()),
            },
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PickQueryParamsGpu {
    pub(crate) transform: ScatterTransform,
    pub(crate) cursor_chart: [f32; 4],
    pub(crate) chart_limits: [f32; 4],
    pub(crate) scatter_line: [f32; 4],
    pub(crate) data: [u32; 4],
    pub(crate) style: [u32; 4],
    pub(crate) series: [u32; 4],
    pub(crate) stream: [u32; 4],
    pub(crate) line: [u32; 4],
    /// `(size lane base, size logical length, 0, 0)`.
    pub(crate) size_map: [u32; 4],
    /// `(domain_min hi, domain_min lo, domain_max hi, domain_max lo)`.
    pub(crate) size_domain: [f32; 4],
    /// `(min_px, max_px, 0, 0)`, logical pixels.
    pub(crate) size_px: [f32; 4],
    /// `(stroke half-width, milkyway half extent, line gate reach, 0)`,
    /// canvas pixels.
    pub(crate) line_px: [f32; 4],
    /// The dash pattern in canvas pixels.
    pub(crate) line_dash: [[f32; 4]; 2],
    /// `(marker mode, line mode, series salt, dash count)`.
    pub(crate) styled: [u32; 4],
    /// `(u lane base, v lane base, arrow count, pivot | scale-pixels bit)`.
    pub(crate) vector: [u32; 4],
    /// `(scale, shaft width, head length, head width)`, canvas pixels.
    pub(crate) vector_px: [f32; 4],
    /// `(minimum length, 0, 0, 0)`, canvas pixels.
    pub(crate) vector_min: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PickCandidateGpu {
    pub(crate) valid: u32,
    pub(crate) series_order: u32,
    pub(crate) point_index: u32,
    pub(crate) primitive_kind: u32,
    pub(crate) primitive_index: u32,
    pub(crate) distance_sq: f32,
    pub(crate) distance_px: f32,
    pub(crate) _pad: u32,
}

const _: () = assert!(std::mem::size_of::<PickQueryParamsGpu>() == 432);
const _: () = assert!(std::mem::size_of::<PickCandidateGpu>() == CANDIDATE_BYTES as usize);

#[derive(Clone)]
pub(super) struct PickSeriesGpu {
    /// Charge for this series' gate masks, workgroup candidates, params, style
    /// rows, and scalar result. Shared across clones like `ReduceResources`.
    pub(super) _charge: SharedCharge,
    pub(super) pool_identity: Option<PoolIdentity>,
    pub(super) x_column: ColumnId,
    pub(super) y_column: ColumnId,
    pub(super) style_index_column: Option<ColumnId>,
    pub(super) x_handle: ColumnHandle,
    pub(super) y_handle: ColumnHandle,
    pub(super) style_index_handle: Option<ColumnHandle>,
    pub(super) pool_layout_generation: u64,
    pub(super) x_allocation_epoch: u64,
    pub(super) y_allocation_epoch: u64,
    pub(super) style_index_allocation_epoch: Option<u64>,
    pub(super) size_column: Option<ColumnId>,
    pub(super) size_handle: Option<ColumnHandle>,
    pub(super) size_allocation_epoch: Option<u64>,
    /// A point vector's arrows: the style, the `u` / `v` / colour columns
    /// with their handles and allocation epochs, the component lane bases
    /// and the arrow count.
    pub(super) vector: Option<GpuPickVector>,
    pub(super) vector_handles: [Option<ColumnHandle>; 3],
    pub(super) vector_epochs: [Option<u64>; 3],
    pub(super) vector_base: [u32; 2],
    pub(super) vector_count: u32,
    pub(super) size_base: u32,
    pub(super) size_len: u32,
    pub(super) size_domain: [f32; 4],
    pub(super) size_px: [f32; 2],
    pub(super) query_params: wgpu::Buffer,
    pub(super) gate_masks: wgpu::Buffer,
    pub(super) workgroup_candidates: wgpu::Buffer,
    pub(super) query_data_bg: wgpu::BindGroup,
    pub(super) query_work_bg: wgpu::BindGroup,
    pub(super) reduce_bg: wgpu::BindGroup,
    pub(super) result: wgpu::Buffer,
    pub(super) point_count: u32,
    pub(super) gate_word_count: u32,
    pub(super) dispatch_x: u32,
    pub(super) dispatch_y: u32,
    pub(super) direct_scan: bool,
    pub(super) flags: u32,
    pub(super) base_radius_px: f32,
    pub(super) base_shape_id: u32,
    pub(super) line_half_width_px: f32,
    pub(super) line_step_mode: u32,
    /// The dash pattern in logical pixels and how many of its lengths count.
    pub(super) line_dash: [f32; 8],
    pub(super) line_dash_len: u32,
    pub(super) max_extent_px: f32,
    pub(super) style_count: u32,
    pub(super) override_count: u32,
    pub(super) style_index_base: u32,
    pub(super) style_index_len: u32,
}

pub(super) fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub(super) fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub(super) fn nonnegative(v: f32) -> f32 {
    if !v.is_nan() && v > 0.0 { v } else { 0.0 }
}

/// The factor `visual_shape_radius` gives shape `shape_id` (SHADER_COMMON.md
/// §5.5): its drawn radius per unit of reference radius.
pub(super) fn shape_scale(shape_id: u32) -> f32 {
    crate::data_render::marker_visual_scale(crate::data_render::marker_base_shape_id(shape_id))
}

pub(super) fn gpu_meta_u32(v: f32) -> Option<u32> {
    if !v.is_finite() || !(0.0..4_294_967_296.0).contains(&v) || v.fract() != 0.0 {
        None
    } else {
        Some(v as u32)
    }
}

pub(super) fn style_max_extent(
    series_id: &str,
    scatter: &GpuPickScatter<'_>,
) -> Result<f32, GpuPickError> {
    let mut maximum_radius = nonnegative(scatter.base_radius_px);
    let mut maximum_scale = shape_scale(scatter.base_shape_id);
    // A size map clamps to its pixel range, so `max_px` bounds every mapped
    // radius; an override row may still exceed it and is visited below.
    if let Some(size_map) = scatter.size_map.as_ref() {
        maximum_radius = maximum_radius.max(nonnegative(size_map.max_px));
    }
    let Some(map) = scatter.style_map.as_ref() else {
        return Ok(maximum_radius * maximum_scale);
    };

    if map.style_meta.style_count as usize > map.style_slots.len() {
        return Err(GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "style_count exceeds the supplied slot array",
        });
    }
    if map.style_meta.override_count as usize > map.style_overrides.len() {
        return Err(GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "override_count exceeds the supplied override array",
        });
    }
    let has_index = map.style_meta.has_index != 0;
    if map.style_meta.has_index > 1 || has_index != map.style_index_column.is_some() {
        return Err(GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "has_index and style_index_column disagree",
        });
    }

    let mut visit = |meta: [f32; 4]| -> Result<(), GpuPickError> {
        let mask = gpu_meta_u32(meta[2]).ok_or_else(|| GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "a style mask is not a finite integer",
        })?;
        if mask & STYLE_MASK_RADIUS != 0 {
            maximum_radius = maximum_radius.max(nonnegative(meta[0]));
        }
        if mask & STYLE_MASK_SHAPE != 0 {
            let shape = gpu_meta_u32(meta[1]).ok_or_else(|| GpuPickError::InvalidStyleMap {
                series_id: series_id.to_owned(),
                reason: "a mapped shape id is not a finite integer",
            })?;
            maximum_scale = maximum_scale.max(shape_scale(shape));
        }
        Ok(())
    };
    for slot in &map.style_slots[..map.style_meta.style_count as usize] {
        visit(slot.meta)?;
    }
    for override_ in &map.style_overrides[..map.style_meta.override_count as usize] {
        visit(override_.meta)?;
    }

    // Radius and shape changes may originate in different table/override
    // rows. Their product is deliberately a cross-product upper bound; the
    // final exact pass still applies rows in production order.
    Ok(maximum_radius * maximum_scale)
}
