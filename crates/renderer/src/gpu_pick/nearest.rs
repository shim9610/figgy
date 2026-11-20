//! The nearest-datum pick (#381): each series' datum nearest to the
//! cursor, with no pick reach, and the panel's nearest among them.
//!
//! One compute kernel per column kind (`gpu_pick/nearest_entry.wgsl`, the
//! xy kind: markers, lines, error bars and stems by their anchors, point
//! vectors, streamlines, radar, ternary and monotonic envelopes), assembled
//! with one placement fragment per projection: `nearest_project_2d.wgsl`
//! (the draw's `pick_data_px` and `pick_ternary_px`) and
//! `nearest_project_3d.wgsl` (the shared `common/project_3d.wgsl`). The
//! kernel reads no style: there is one pipeline per projection and shader
//! set, whatever the series' styles.
//!
//! The kernel runs in the point picker's registry slots: their dispatch,
//! their per-workgroup candidates and the per-series reduction
//! (`pick_reduce_candidates`), into each slot's result buffer. Results are
//! storage buffers, so no render target is involved. The registry may hold
//! the series of several layers (the base layer and its overlays, or a
//! subplot and its overlays); each slot carries its own transform, chart
//! rectangle and, on a 3D chart, camera ([`NearestSlot`]).
//!
//! The panel's nearest datum is chosen on the CPU from the per-series
//! results (one per series, never per datum): the nearest wins whatever
//! its layer; an exact tie goes to the active layer, then to the series
//! drawn later (on top). Whether the cursor hits is not this pick's to
//! decide; the hit rule reads the drawn pixel (#381).

use super::engine::{create_bind_group_checked, create_buffer_checked, pick_gpu_error};
use super::*;
use crate::shader_assembly::{AssembledBySet, ShaderSet, assembled_by_set};

/// The xy column kind's nearest-datum module on a flat chart.
pub(crate) const NEAREST_PICK_MODULE: (&str, AssembledBySet) = (
    "figgy GPU pick nearest-datum shader",
    assembled_by_set!(
        "gpu_pick/core.wgsl",
        set,
        "data_render/common/projection.wgsl",
        "data_render/common/sample_validity.wgsl",
        "gpu_pick/nearest_project_2d.wgsl",
        "gpu_pick/nearest_entry.wgsl",
    ),
);

/// The same kernel on a 3D chart: the column kind's second projection.
pub(crate) const NEAREST_PICK_3D_MODULE: (&str, AssembledBySet) = (
    "figgy GPU pick nearest-datum 3D shader",
    assembled_by_set!(
        "gpu_pick/core.wgsl",
        set,
        "data_render/common/projection.wgsl",
        "data_render/common/sample_validity.wgsl",
        "data_render/common/project_3d.wgsl",
        "gpu_pick/nearest_project_3d.wgsl",
        "gpu_pick/nearest_entry.wgsl",
    ),
);

/// Group 3 of the 3D kernel: the camera (`project_3d.wgsl`, binding 4).
const VIEW3D_ENTRIES: [wgpu::BindGroupLayoutEntry; 1] = [wgpu::BindGroupLayoutEntry {
    binding: 4,
    visibility: wgpu::ShaderStages::COMPUTE,
    ty: wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    },
    count: None,
}];

/// The nearest-datum pipelines of one shader set, each created on first
/// use: the flat and the 3D projection of the xy column kind.
pub(crate) struct NearestPickPipelines {
    set: ShaderSet,
    flat: Option<wgpu::ComputePipeline>,
    view3d: Option<(wgpu::ComputePipeline, wgpu::BindGroupLayout)>,
}

impl NearestPickPipelines {
    pub(crate) fn new(set: ShaderSet) -> Self {
        Self {
            set,
            flat: None,
            view3d: None,
        }
    }

    /// The shader set the pipelines are built from.
    pub(crate) fn set(&self) -> ShaderSet {
        self.set
    }

    fn create(
        bundle: &PickPipelineBundle,
        set: ShaderSet,
        module: (&str, AssembledBySet),
        layout: &wgpu::PipelineLayout,
    ) -> wgpu::ComputePipeline {
        let device = &bundle.device;
        let shader = crate::gpu_compile::shader_module(
            device,
            wgpu::ShaderModuleDescriptor {
                label: Some(module.0),
                source: wgpu::ShaderSource::Wgsl(module.1.of(set).source.into()),
            },
        );
        let base = crate::gpu_compile::workgroup_options(device);
        crate::gpu_compile::compute_pipeline(
            device,
            &wgpu::ComputePipelineDescriptor {
                label: Some("figgy GPU pick nearest-datum pipeline"),
                layout: Some(layout),
                module: &shader,
                entry_point: Some("pick_nearest_points"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[],
                    zero_initialize_workgroup_memory: base.zero_initialize_workgroup_memory,
                },
                cache: None,
            },
        )
    }

    /// Whether the flat chart's pipeline has been created.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn has_flat(&self) -> bool {
        self.flat.is_some()
    }

    /// Whether the 3D chart's pipeline has been created.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn has_3d(&self) -> bool {
        self.view3d.is_some()
    }

    /// The flat chart's pipeline.
    pub(crate) fn ensure(&mut self, bundle: &PickPipelineBundle) -> &wgpu::ComputePipeline {
        let set = self.set;
        self.flat.get_or_insert_with(|| {
            Self::create(bundle, set, NEAREST_PICK_MODULE, &bundle.exact_layout)
        })
    }

    /// The 3D chart's pipeline and its camera group layout.
    pub(crate) fn ensure_3d(
        &mut self,
        bundle: &PickPipelineBundle,
    ) -> &(wgpu::ComputePipeline, wgpu::BindGroupLayout) {
        let set = self.set;
        self.view3d.get_or_insert_with(|| {
            let device = &bundle.device;
            let view_bgl = crate::gpu_compile::bind_group_layout(
                device,
                &wgpu::BindGroupLayoutDescriptor {
                    label: Some("figgy GPU pick nearest-datum camera bgl"),
                    entries: &VIEW3D_ENTRIES,
                },
            );
            let layout = crate::gpu_compile::pipeline_layout(
                device,
                &wgpu::PipelineLayoutDescriptor {
                    label: Some("figgy GPU pick nearest-datum 3D layout"),
                    bind_group_layouts: &[
                        Some(&bundle.query_data_bgl),
                        Some(&bundle.query_work_bgl),
                        Some(&bundle.arc_bgl),
                        Some(&view_bgl),
                    ],
                    immediate_size: 0,
                },
            );
            (
                Self::create(bundle, set, NEAREST_PICK_3D_MODULE, &layout),
                view_bgl,
            )
        })
    }
}

/// How one registry slot is placed: the query of the panel that draws the
/// series (its transform and chart rectangle, the cursor), the layer it is
/// drawn on, and on a 3D chart the panel's camera mapped onto the cursor's
/// pixel with the slot's height column as (f32-lane base, length).
#[derive(Clone, Copy)]
pub(crate) struct NearestSlot {
    pub(crate) query: GpuPickQuery,
    pub(crate) layer: crate::LayerRef,
    pub(crate) view3d: Option<(crate::data_render::surface_mesh::View3dGpu, (u32, u32))>,
}

/// One series' datum nearest to the cursor.
#[derive(Clone, Debug, PartialEq)]
pub struct NearestDatum {
    /// The layer the series is drawn on.
    pub layer: crate::LayerRef,
    pub source_id: Option<String>,
    pub series_id: String,
    /// The datum's logical index (a monotonic series' sample index).
    pub point_index: usize,
    /// Its screen distance to the cursor in canvas pixels.
    pub distance_px: f32,
}

/// A resolved nearest-datum pick: every series' nearest datum, in drawing
/// order, and the nearest of them (an exact tie to the active layer, then
/// to the series drawn later). A series the draw places nothing of has no
/// entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NearestPick {
    pub nearest: Option<NearestDatum>,
    pub per_series: Vec<NearestDatum>,
}

/// A pending nearest-datum pick: each series' result, read back.
pub struct NearestPickTicket {
    readback: Option<GpuReadback>,
    identities: Arc<[PickIdentity]>,
    /// Each slot's layer, by registry position.
    layers: Vec<crate::LayerRef>,
    active: crate::LayerRef,
    pub(crate) monotonic: Vec<crate::renderer::MonotonicPickRows>,
    _charge: Option<GpuByteCharge>,
}

impl NearestPickTicket {
    fn none() -> Self {
        Self {
            readback: None,
            identities: Arc::from([]),
            layers: Vec::new(),
            active: crate::LayerRef::Base,
            monotonic: Vec::new(),
            _charge: None,
        }
    }

    /// Wait for the pick, then decode it.
    pub async fn resolve(mut self) -> Result<NearestPick, GpuPickError> {
        if let Some(readback) = self.readback.as_mut() {
            readback.wait().await?;
        }
        self.finish()
    }

    /// [`Self::resolve`], polling the device on this thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve_blocking(mut self) -> Result<NearestPick, GpuPickError> {
        if let Some(readback) = self.readback.as_mut() {
            readback.wait_blocking()?;
        }
        self.finish()
    }

    /// The pick if it has completed, or the ticket back, without blocking.
    pub fn try_resolve(mut self) -> Result<crate::TryResolve<NearestPick, Self>, GpuPickError> {
        if let Some(readback) = self.readback.as_mut()
            && !readback.poll_ready()?
        {
            return Ok(crate::TryResolve::Pending(self));
        }
        Ok(crate::TryResolve::Ready(self.finish()?))
    }

    pub(crate) async fn wait(&mut self) -> Result<(), GpuPickError> {
        if let Some(readback) = self.readback.as_mut() {
            readback.wait().await?;
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn wait_blocking(&mut self) -> Result<(), GpuPickError> {
        if let Some(readback) = self.readback.as_mut() {
            readback.wait_blocking()?;
        }
        Ok(())
    }

    pub(crate) fn poll_ready(&mut self) -> Result<bool, GpuPickError> {
        match self.readback.as_mut() {
            Some(readback) => Ok(readback.poll_ready()?),
            None => Ok(true),
        }
    }

    /// The series order (registry identity position) of `datum`.
    pub(crate) fn series_order(&self, datum: &NearestDatum) -> u32 {
        self.identities
            .iter()
            .position(|identity| {
                identity.series_id == datum.series_id && identity.source_id == datum.source_id
            })
            .map_or(0, |order| order as u32)
    }

    pub(crate) fn decode(&mut self) -> Result<NearestPick, GpuPickError> {
        self.finish()
    }

    fn finish(&mut self) -> Result<NearestPick, GpuPickError> {
        let series = self.layers.len();
        let Some(readback) = self.readback.as_mut() else {
            return Ok(NearestPick::default());
        };
        let mut per_series = Vec::new();
        per_series
            .try_reserve_exact(series)
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "nearest pick result",
            })?;
        let identities = &self.identities;
        let layers = &self.layers;
        let monotonic = &self.monotonic;
        let size = CANDIDATE_BYTES as usize;
        readback.finish_checked(|mapped| -> Result<(), GpuPickError> {
            for (slot, layer) in layers.iter().enumerate() {
                let candidate =
                    *bytemuck::from_bytes::<PickCandidateGpu>(&mapped[slot * size..][..size]);
                if candidate.valid != 1 {
                    continue;
                }
                if !candidate.distance_px.is_finite() {
                    return Err(GpuPickError::InvalidGpuResult);
                }
                let identity = identities
                    .get(candidate.series_order as usize)
                    .ok_or(GpuPickError::InvalidGpuResult)?;
                // A monotonic series' row maps back to the logical index of
                // the kept point it was uploaded from (#271).
                let point_index = match monotonic
                    .iter()
                    .find(|rows| rows.series_id == identity.series_id)
                {
                    Some(rows) => match rows.logical(candidate.point_index as usize) {
                        Some(index) => index,
                        None => continue,
                    },
                    None => candidate.point_index as usize,
                };
                per_series.push(NearestDatum {
                    layer: *layer,
                    source_id: identity.source_id.clone(),
                    series_id: identity.series_id.clone(),
                    point_index,
                    distance_px: candidate.distance_px,
                });
            }
            Ok(())
        })??;
        let active = self.active;
        let nearest = per_series
            .iter()
            .fold(None::<&NearestDatum>, |best, datum| match best {
                None => Some(datum),
                Some(best) if datum.distance_px != best.distance_px => {
                    Some(if datum.distance_px < best.distance_px {
                        datum
                    } else {
                        best
                    })
                }
                // An exact tie: the active layer, then the later series.
                Some(best) if best.layer == active && datum.layer != active => Some(best),
                Some(_) => Some(datum),
            })
            .cloned();
        Ok(NearestPick {
            nearest,
            per_series,
        })
    }
}

/// One mask's part of an ink read: the texels of the window that fall on
/// the panel, copied at `offset` of the readback, `bytes_per_row` apart.
pub(crate) struct InkWindow {
    /// The canvas pixel of the copy's first texel.
    pub(crate) origin: [i64; 2],
    pub(crate) size: [u32; 2],
    pub(crate) offset: u64,
    pub(crate) bytes_per_row: u32,
}

/// The ink around the cursor (#381): every pickable panel's ink mask
/// texels within the pick's reach, read back. The cursor hits when any
/// texel with ink lies within `reach` canvas pixels of it (the distance
/// from the cursor to the texel's square; a reach of 0 is the texel under
/// the cursor).
pub(crate) struct InkRead {
    readback: GpuReadback,
    windows: Vec<InkWindow>,
    cursor: [f32; 2],
    reach: f32,
    _charge: GpuByteCharge,
}

impl InkRead {
    /// The texel window of a pick at `cursor` with `reach`, as canvas pixel
    /// bounds `[x0, y0, x1, y1)`.
    pub(crate) fn bounds(cursor: [f32; 2], reach: f32) -> [i64; 4] {
        let [x, y] = cursor.map(f64::from);
        let r = f64::from(reach);
        [
            (x - r).floor() as i64,
            (y - r).floor() as i64,
            (x + r).floor() as i64 + 1,
            (y + r).floor() as i64 + 1,
        ]
    }

    /// Copy each `(mask, canvas origin of its texel 0)` texel window that
    /// meets the pick's bounds, and request the read.
    pub(crate) fn submit(
        bundle: &PickEngineHandles<'_>,
        masks: &[(&wgpu::Texture, [i64; 2])],
        cursor: [f32; 2],
        reach: f32,
    ) -> Result<Option<Self>, GpuPickError> {
        let [bx0, by0, bx1, by1] = Self::bounds(cursor, reach);
        let mut windows = Vec::new();
        windows
            .try_reserve_exact(masks.len())
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "ink read windows",
            })?;
        let mut size = 0u64;
        let mut copies = Vec::new();
        copies
            .try_reserve_exact(masks.len())
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "ink read windows",
            })?;
        for (index, (texture, origin)) in masks.iter().enumerate() {
            let x0 = bx0.max(origin[0]);
            let y0 = by0.max(origin[1]);
            let x1 = bx1.min(origin[0] + i64::from(texture.width()));
            let y1 = by1.min(origin[1] + i64::from(texture.height()));
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            let width = (x1 - x0) as u32;
            let height = (y1 - y0) as u32;
            let bytes_per_row = width.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            windows.push(InkWindow {
                origin: [x0, y0],
                size: [width, height],
                offset: size,
                bytes_per_row,
            });
            copies.push(index);
            size += u64::from(bytes_per_row) * u64::from(height);
        }
        if windows.is_empty() {
            return Ok(None);
        }
        let tally = ChargeTally::new();
        let buffer = create_buffer_checked(
            &tally,
            bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick ink readback"),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "ink readback",
        )?;
        let mut encoder = bundle
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy GPU pick ink read"),
            });
        for (window, &index) in windows.iter().zip(&copies) {
            let (texture, origin) = masks[index];
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: (window.origin[0] - origin[0]) as u32,
                        y: (window.origin[1] - origin[1]) as u32,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: window.offset,
                        bytes_per_row: Some(window.bytes_per_row),
                        rows_per_image: Some(window.size[1]),
                    },
                },
                wgpu::Extent3d {
                    width: window.size[0],
                    height: window.size[1],
                    depth_or_array_layers: 1,
                },
            );
        }
        bundle.queue.submit(std::iter::once(encoder.finish()));
        Ok(Some(Self {
            readback: GpuReadback::map(bundle.device, bundle.health, &buffer, None),
            windows,
            cursor,
            reach,
            _charge: tally.into_charge(bundle.ledger, GpuResourceKind::Readback),
        }))
    }

    pub(crate) async fn wait(&mut self) -> Result<(), GpuPickError> {
        Ok(self.readback.wait().await?)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn wait_blocking(&mut self) -> Result<(), GpuPickError> {
        Ok(self.readback.wait_blocking()?)
    }

    pub(crate) fn poll_ready(&mut self) -> Result<bool, GpuPickError> {
        Ok(self.readback.poll_ready()?)
    }

    /// After a wait: whether any texel with ink lies within the reach.
    pub(crate) fn hit(&mut self) -> Result<bool, GpuPickError> {
        let windows = &self.windows;
        let [cx, cy] = self.cursor;
        let reach = self.reach;
        Ok(self.readback.finish_checked(|mapped| {
            windows.iter().any(|window| {
                (0..window.size[1]).any(|row| {
                    let start =
                        window.offset as usize + row as usize * window.bytes_per_row as usize;
                    let texels = &mapped[start..start + window.size[0] as usize];
                    let y = (window.origin[1] + i64::from(row)) as f32;
                    texels.iter().enumerate().any(|(column, &ink)| {
                        let x = (window.origin[0] + column as i64) as f32;
                        let dx = (x - cx).max(cx - (x + 1.0)).max(0.0);
                        let dy = (y - cy).max(cy - (y + 1.0)).max(0.0);
                        ink != 0 && dx * dx + dy * dy <= reach * reach
                    })
                })
            })
        })?)
    }
}

/// What an ink read borrows from the picker's device bundle.
pub(crate) struct PickEngineHandles<'a> {
    pub(crate) device: &'a wgpu::Device,
    pub(crate) queue: &'a wgpu::Queue,
    pub(crate) health: &'a crate::renderer::DeviceHealth,
    pub(crate) ledger: &'a Arc<GpuLedger>,
}

impl GpuPickEngine {
    /// The device handles an ink read submits with.
    pub(crate) fn handles(&self) -> PickEngineHandles<'_> {
        PickEngineHandles {
            device: &self.bundle.device,
            queue: &self.bundle.queue,
            health: &self.bundle.health,
            ledger: &self.bundle.ledger,
        }
    }
}

impl GpuPickEngine {
    /// The nearest-datum pick of the registered series, each placed by its
    /// [`NearestSlot`] (one per registry slot, in registry order), with no
    /// reach: every datum the draw places is a candidate. `active` is the
    /// layer an exact tie between layers goes to.
    pub(crate) fn pick_nearest(
        &self,
        pipelines: &mut NearestPickPipelines,
        pool: &ColumnPool,
        display_scale: f32,
        slots: &[NearestSlot],
        active: crate::LayerRef,
    ) -> Result<NearestPickTicket, GpuPickError> {
        if !display_scale.is_finite() || display_scale <= 0.0 {
            return Ok(NearestPickTicket::none());
        }
        if let Some(expected) = self.registry.pool_identity.as_ref()
            && !expected.same_instance(&pool.identity())
        {
            return Err(GpuPickError::ForeignColumnPool);
        }
        if self.registry.slots.is_empty() {
            return Ok(NearestPickTicket::none());
        }
        if slots.len() != self.registry.slots.len() {
            return Err(GpuPickError::Internal(
                "a nearest-datum pick places every registered slot",
            ));
        }
        if self.registry.pool_layout_generation != pool.layout_generation() {
            let first = &self.registry.slots[0];
            return Err(GpuPickError::StaleColumn {
                series_id: first.identity.series_id.clone(),
                column_id: first.gpu.x_column.clone(),
            });
        }
        for slot in slots {
            let [x, y] = slot.query.canvas_position_px;
            let [_, _, width, height] = slot.query.chart_rect_px;
            if !x.is_finite() || !y.is_finite() || !(width > 0.0 && height > 0.0) {
                return Ok(NearestPickTicket::none());
            }
        }
        let device = &self.bundle.device;
        let tally = ChargeTally::new();
        // Each 3D slot's camera group: the camera is per panel, so per
        // slot; flat slots bind none.
        let mut cameras = Vec::new();
        cameras
            .try_reserve_exact(slots.len())
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "nearest pick cameras",
            })?;
        for slot in slots {
            let camera = match &slot.view3d {
                None => None,
                Some((view, _)) => {
                    let view_bgl = pipelines.ensure_3d(&self.bundle).1.clone();
                    let buffer = charged_buffer_init(
                        &tally,
                        device,
                        &wgpu::util::BufferInitDescriptor {
                            label: Some("figgy GPU pick nearest-datum camera"),
                            contents: bytemuck::bytes_of(view),
                            usage: wgpu::BufferUsages::UNIFORM,
                        },
                    )
                    .map_err(|error| pick_gpu_error("nearest pick camera", error))?;
                    Some(create_bind_group_checked(
                        device,
                        &wgpu::BindGroupDescriptor {
                            label: Some("figgy GPU pick nearest-datum camera bg"),
                            layout: &view_bgl,
                            entries: &[wgpu::BindGroupEntry {
                                binding: 4,
                                resource: buffer.as_entire_binding(),
                            }],
                        },
                        "nearest pick camera bind group",
                    )?)
                }
            };
            cameras.push(camera);
        }
        let flat = slots
            .iter()
            .any(|slot| slot.view3d.is_none())
            .then(|| pipelines.ensure(&self.bundle).clone());
        let view3d = slots
            .iter()
            .any(|slot| slot.view3d.is_some())
            .then(|| pipelines.ensure_3d(&self.bundle).0.clone());
        let mut layers = Vec::new();
        layers
            .try_reserve_exact(slots.len())
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "nearest pick layers",
            })?;
        for ((series_order, registered), slot) in self.registry.slots.iter().enumerate().zip(slots)
        {
            Self::validate_series_columns(pool, registered)?;
            let series = &registered.gpu;
            let mut params = query_params(
                series,
                &registered.identity.series_id,
                &slot.query,
                display_scale,
                series_order as u32,
                [
                    series.point_count,
                    lane_base(series.x_handle, &registered.identity.series_id)?,
                    lane_base(series.y_handle, &registered.identity.series_id)?,
                    series.style_index_base,
                ],
                [0, series.point_count, series.point_count, 0],
                false,
            );
            // A 3D slot's heights, where a flat ternary series reads its
            // third component (a 3D chart is never ternary).
            if let Some((_, (base, len))) = slot.view3d {
                params.size_map[2] = base;
                params.size_map[3] = len;
            }
            self.bundle
                .queue
                .write_buffer(&series.query_params, 0, bytemuck::bytes_of(&params));
            layers.push(slot.layer);
        }
        let readback = create_buffer_checked(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick nearest-datum readback"),
                size: CANDIDATE_BYTES * slots.len() as u64,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "nearest pick readback",
        )?;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy GPU pick nearest-datum encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy GPU pick nearest-datum scan"),
                timestamp_writes: None,
            });
            for ((registered, slot), camera) in self.registry.slots.iter().zip(slots).zip(&cameras)
            {
                let series = &registered.gpu;
                let pipeline = match (&slot.view3d, &flat, &view3d) {
                    (None, Some(flat), _) => flat,
                    (Some(_), _, Some(view3d)) => view3d,
                    _ => return Err(GpuPickError::InvalidGpuResult),
                };
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &series.query_data_bg, &[]);
                pass.set_bind_group(1, &series.query_work_bg, &[]);
                pass.set_bind_group(2, &self.bundle.arc_placeholder, &[]);
                if let Some(camera) = camera {
                    pass.set_bind_group(3, camera, &[]);
                }
                pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy GPU pick nearest-datum per-series reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.reduce);
            for registered in &self.registry.slots {
                pass.set_bind_group(0, &registered.gpu.reduce_bg, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
        for (index, registered) in self.registry.slots.iter().enumerate() {
            encoder.copy_buffer_to_buffer(
                &registered.gpu.result,
                0,
                &readback,
                index as u64 * CANDIDATE_BYTES,
                CANDIDATE_BYTES,
            );
        }
        self.bundle.queue.submit(std::iter::once(encoder.finish()));
        Ok(NearestPickTicket {
            readback: Some(GpuReadback::map(
                device,
                &self.bundle.health,
                &readback,
                None,
            )),
            identities: Arc::clone(&self.registry.identities),
            layers,
            active,
            monotonic: Vec::new(),
            _charge: Some(tally.into_charge(&self.bundle.ledger, GpuResourceKind::Readback)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transform(max: f32) -> ScatterTransform {
        ScatterTransform {
            data_min: [0.0, 0.0],
            data_max: [max, max],
            data_min_lo: [0.0; 2],
            data_max_lo: [0.0; 2],
            scale_log: [0.0; 2],
            pixel_to_ndc: [0.02; 2],
            data_to_panel_scale: [1.0; 2],
            data_to_panel_offset: [0.0; 2],
            style_params: [[0.0; 4]; 3],
            polar_angle: [0.0; 4],
            polar_extent: [0.0; 4],
            ternary_lt: [0.0; 4],
            ternary_r: [0.0; 4],
            breaks: crate::data_render::NO_BREAKS,
            break_info: crate::data_render::NO_BREAK_INFO,
            scale_prob: crate::data_render::NO_PROBABILITY,
        }
    }

    fn slot(transform: ScatterTransform, cursor: [f32; 2]) -> NearestSlot {
        NearestSlot {
            query: GpuPickQuery {
                transform,
                chart_rect_px: [0.0, 0.0, 100.0, 100.0],
                data_area_px: None,
                canvas_position_px: cursor,
                max_distance_px: 0.0,
                draw_style: GpuPickDrawStyle::PRECISE,
            },
            layer: crate::LayerRef::Base,
            view3d: None,
        }
    }

    fn markers(id: &str, x: &str, y: &str) -> GpuPickSeriesDescriptor<'static> {
        GpuPickSeriesDescriptor {
            source_id: None,
            series_id: id.to_owned(),
            x_column: x.into(),
            y_column: y.into(),
            scatter: Some(GpuPickScatter {
                base_radius_px: 3.0,
                base_shape_id: 5,
                style_map: None,
                size_map: None,
            }),
            line_width_px: None,
            line_closed: false,
            path_arrows: None,
            line_interpolation: crate::data_config::LineInterpolation::Linear,
            line_style: crate::line::LineStylePreset::Solid,
            vector: None,
            errorbar: None,
            area: None,
            category_jitter: None,
            ternary_c: None,
            ternary_region: false,
            stem: None,
        }
    }

    fn pool_with(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ledger: &Arc<GpuLedger>,
        columns: &[(&str, Vec<f32>)],
    ) -> ColumnPool {
        let ctx = || crate::data_render::GpuAllocCtx::new(device, queue, ledger);
        let bytes: usize = columns.iter().map(|(_, v)| v.len() * 8 + 1024).sum();
        let mut pool = ColumnPool::new(ctx(), bytes as u64).unwrap();
        for (id, values) in columns {
            pool.add_column(
                (*id).into(),
                &crate::data::Column {
                    data: values.clone(),
                },
                ctx(),
            )
            .unwrap();
        }
        pool
    }

    fn pick(engine: &GpuPickEngine, pool: &ColumnPool, slots: &[NearestSlot]) -> NearestPick {
        let mut pipelines = NearestPickPipelines::new(ShaderSet::Full);
        engine
            .pick_nearest(&mut pipelines, pool, 1.0, slots, crate::LayerRef::Base)
            .unwrap()
            .resolve_blocking()
            .unwrap()
    }

    /// Rule 10: a series past the size the exact pass scans directly
    /// (`GPU_PICK_DIRECT_SCAN_POINTS`), dispatched over two dimensions,
    /// answers its nearest datum by its original index; nothing caps it.
    #[test]
    fn a_series_past_the_direct_scan_size_answers_its_original_index() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let ledger = Arc::new(GpuLedger::new());
        let count = GPU_PICK_DIRECT_SCAN_POINTS as usize * 3 + 17;
        let xs: Vec<f32> = (0..count).map(|i| ((i * 2_053) % count) as f32).collect();
        let ys: Vec<f32> = (0..count).map(|i| ((i * 12_337) % count) as f32).collect();
        let target = count / 3;
        let max = (count - 1) as f32;
        let cursor = [xs[target] / max * 100.0, (1.0 - ys[target] / max) * 100.0];
        let pool = pool_with(&device, &queue, &ledger, &[("bx", xs), ("by", ys)]);
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(&pool, markers("big", "bx", "by"))
            .unwrap();
        // Over two dimensions, as a device with a small per-dimension limit
        // dispatches it.
        let groups = engine.registry.slots[0]
            .gpu
            .gate_word_count
            .div_ceil(GPU_PICK_WORKGROUP_SIZE);
        engine.registry.slots[0].gpu.dispatch_x = 1;
        engine.registry.slots[0].gpu.dispatch_y = groups;
        let picked = pick(&engine, &pool, &[slot(transform(max), cursor)]);
        let nearest = picked.nearest.expect("an answer");
        assert_eq!(nearest.point_index, target);
        // The datum's centre, up to f32 rounding of the projection.
        assert!(nearest.distance_px < 1.0e-3, "{nearest:?}");
    }

    /// A datum the draw does not place (non-finite, or non-positive on a
    /// logarithmic axis) is never the answer, however near; within a
    /// series an exact tie goes to the later index (drawn on top); and
    /// every series answers, nearest first across them.
    #[test]
    fn unplaced_data_never_answer_and_ties_go_on_top() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let ledger = Arc::new(GpuLedger::new());
        let pool = pool_with(
            &device,
            &queue,
            &ledger,
            &[
                ("ax", vec![50.0, f32::NAN, 20.0, 20.0]),
                ("ay", vec![50.0, 50.0, 80.0, 80.0]),
                ("bx", vec![10.0, 90.0]),
                ("by", vec![10.0, 90.0]),
            ],
        );
        let mut engine = GpuPickEngine::new(device.clone(), queue.clone()).unwrap();
        engine.add_series(&pool, markers("a", "ax", "ay")).unwrap();
        engine.add_series(&pool, markers("b", "bx", "by")).unwrap();
        let t = transform(100.0);
        // On the NaN's would-be place, (x=?, y=50): index 0 at x=50 is
        // the nearest placed datum of `a`.
        let picked = pick(
            &engine,
            &pool,
            &[slot(t, [49.0, 50.0]), slot(t, [49.0, 50.0])],
        );
        let a = &picked.per_series[0];
        assert_eq!((a.series_id.as_str(), a.point_index), ("a", 0));
        assert_eq!(picked.per_series.len(), 2);
        // On the twin points (20, 80): the later index.
        let picked = pick(
            &engine,
            &pool,
            &[slot(t, [20.0, 20.0]), slot(t, [20.0, 20.0])],
        );
        let nearest = picked.nearest.expect("an answer");
        assert_eq!((nearest.series_id.as_str(), nearest.point_index), ("a", 3));
        // A logarithmic axis does not place a non-positive value.
        let mut log = transform(2.0);
        log.data_min = [0.0, -1.0];
        log.scale_log = [0.0, 1.0];
        let pool_log = pool_with(
            &device,
            &queue,
            &ledger,
            &[("lx", vec![1.0, 1.0]), ("ly", vec![-5.0, 10.0])],
        );
        let mut engine_log = GpuPickEngine::new(device.clone(), queue.clone()).unwrap();
        engine_log
            .add_series(&pool_log, markers("log", "lx", "ly"))
            .unwrap();
        let picked = pick(&engine_log, &pool_log, &[slot(log, [50.0, 100.0])]);
        assert_eq!(picked.nearest.map(|d| d.point_index), Some(1));
    }
}
