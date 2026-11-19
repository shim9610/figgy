//! The engine, its checked buffer and bind-group helpers, its construction
//! and the reduce resources.

use super::*;

#[derive(Clone)]
pub(in crate::gpu_pick) struct ReduceResources {
    pub(in crate::gpu_pick) candidates: wgpu::Buffer,
    pub(in crate::gpu_pick) final_result: wgpu::Buffer,
    pub(in crate::gpu_pick) bind_group: wgpu::BindGroup,
    pub(in crate::gpu_pick) capacity: usize,
    /// Charge for `candidates` + `final_result`. Shared because this struct is
    /// cloned and the clones hold the same device buffers — see
    /// [`SharedCharge`].
    pub(in crate::gpu_pick) _charge: SharedCharge,
}

/// Persistent exact GPU picking engine.
pub(crate) struct GpuPickEngine {
    pub(in crate::gpu_pick) bundle: Arc<PickPipelineBundle>,
    pub(in crate::gpu_pick) registry: PickRegistry,
    #[cfg(test)]
    pub(in crate::gpu_pick) fail_next_registry_prepare: std::cell::Cell<bool>,
}

/// Every pick buffer goes through here, and the tally is not optional, so a new
/// pick allocation cannot skip the ledger.
pub(in crate::gpu_pick) fn create_buffer_checked(
    tally: &ChargeTally,
    device: &wgpu::Device,
    desc: &wgpu::BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Buffer, GpuPickError> {
    // gpu-alloc: PickScratch
    charged_buffer(tally, device, desc).map_err(|error| pick_gpu_error(resource, error))
}

/// A GPU failure met while creating a pick resource: out of memory is the
/// picker's allocation failure, anything else a GPU error.
pub(in crate::gpu_pick) fn pick_gpu_error(
    resource: &'static str,
    error: crate::FiggyError,
) -> GpuPickError {
    match error {
        crate::FiggyError::GpuResourceAllocationFailed { .. } => {
            GpuPickError::AllocationFailed { resource }
        }
        other => GpuPickError::from_figgy(other),
    }
}

pub(in crate::gpu_pick) fn create_bind_group_checked(
    device: &wgpu::Device,
    desc: &wgpu::BindGroupDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::BindGroup, GpuPickError> {
    crate::renderer::gpu_checked(device, resource, || device.create_bind_group(desc))
        .map_err(|error| pick_gpu_error(resource, error))
}

pub(in crate::gpu_pick) fn lane_base(
    handle: ColumnHandle,
    series_id: &str,
) -> Result<u32, GpuPickError> {
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

pub(in crate::gpu_pick) fn gate_dispatch_layout(
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

pub(in crate::gpu_pick) fn same_storage(a: ColumnHandle, b: ColumnHandle) -> bool {
    a.offset == b.offset && a.byte_size == b.byte_size && a.len_values == b.len_values
}

/// [`same_storage`] for an optional column: both absent, or both present at
/// the same storage.
pub(in crate::gpu_pick) fn same_optional_storage(
    a: Option<ColumnHandle>,
    b: Option<ColumnHandle>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same_storage(a, b),
        (None, None) => true,
        _ => false,
    }
}

impl GpuPickEngine {
    #[cfg(test)]
    pub(in crate::gpu_pick) fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
    ) -> Result<Self, GpuPickError> {
        let mut noop = |_| {};
        Self::new_observed(device, queue, &mut noop)
    }

    #[cfg(test)]
    pub(in crate::gpu_pick) fn new_observed(
        device: wgpu::Device,
        queue: wgpu::Queue,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Self, GpuPickError> {
        observe_result(observer, INIT_SCOPE, "limits", || {
            validate_device_limits(&device)
        })?;
        let bundle = create_pipeline_bundle_observed(
            device.clone(),
            queue,
            Arc::new(GpuLedger::new()),
            crate::renderer::DeviceHealth::polled_for_test(&device),
            crate::shader_assembly::ShaderSet::Full,
            observer,
        )?;
        let reduce = observe_result(observer, INIT_SCOPE, "initial_reduce_resources", || {
            Self::create_reduce_resources(&bundle.device, &bundle.ledger, &bundle.reduce_bgl, 1)
        })?;
        Ok(Self::from_bundle_and_reduce(bundle, reduce))
    }

    pub(crate) fn from_bundle(bundle: Arc<PickPipelineBundle>) -> Result<Self, GpuPickError> {
        let reduce =
            Self::create_reduce_resources(&bundle.device, &bundle.ledger, &bundle.reduce_bgl, 1)?;
        Ok(Self::from_bundle_and_reduce(bundle, reduce))
    }

    pub(in crate::gpu_pick) fn from_bundle_and_reduce(
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

    pub(in crate::gpu_pick) fn create_reduce_resources(
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

    pub(in crate::gpu_pick) fn prepare_reduce_resources(
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
}
