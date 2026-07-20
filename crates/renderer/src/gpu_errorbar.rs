//! Exact GPU reduction for pairwise errorbar fit extents.
//!
//! The column pool remains the sole per-value source.  This module binds
//! three pool slices containing `(hi, lo)` f32 pairs, exhaustively evaluates
//! `value - lower_error` and `value + upper_error`, and reads back one 72-byte
//! record.  No value-sized CPU allocation or shadow column is created.
//!
//! [`GpuErrorbarExtentTicket`] is detached from the engine.  A caller may
//! submit a query, release its mutable renderer borrow, and await the owned
//! ticket.  Native resolution polls only the ticket's submission; wasm
//! resolution yields while the browser drives WebGPU callbacks.

use std::num::NonZeroU64;

use futures_channel::oneshot;
use wgpu::util::DeviceExt;

use crate::data::COLUMN_VALUE_BYTES;
use crate::data_render::ColumnHandle;

const WORKGROUP_SIZE: u32 = 128;
const ENDPOINT_INVALID: u32 = 0;
const ENDPOINT_LOWER: u32 = 1;
const ENDPOINT_UPPER: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct EndpointGpu {
    value: [f32; 2],
    error: [f32; 2],
    kind: u32,
    source_index: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ExtentStateGpu {
    minimum: EndpointGpu,
    maximum: EndpointGpu,
    minimum_positive: EndpointGpu,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ParamsGpu {
    input_len: u32,
    lower_len: u32,
    upper_len: u32,
    dispatch_groups: u32,
}

const STATE_BYTES: u64 = std::mem::size_of::<ExtentStateGpu>() as u64;

const _: [(); 24] = [(); std::mem::size_of::<EndpointGpu>()];
const _: [(); 72] = [(); std::mem::size_of::<ExtentStateGpu>()];
const _: [(); 16] = [(); std::mem::size_of::<ParamsGpu>()];

/// The scalar result reconstructed from the winning `(hi, lo)` endpoint
/// lanes.  It can be converted directly to `chart::FitExtent` by the future
/// Renderer integration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuErrorbarExtent {
    pub min: f64,
    pub max: f64,
    pub min_positive: Option<f64>,
}

/// Validation, submission, or detached-readback failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuErrorbarError {
    UnknownColumn {
        role: &'static str,
        id: String,
    },
    StaleHandle {
        role: &'static str,
        id: String,
        generation: u32,
        current: u32,
    },
    EmptyColumn {
        role: &'static str,
    },
    ValueCountTooLarge {
        role: &'static str,
        len: usize,
    },
    InvalidColumnRange {
        role: &'static str,
        offset: u64,
        required: u64,
        available: u64,
        pool_size: u64,
    },
    MisalignedColumnOffset {
        role: &'static str,
        offset: u64,
        alignment: u32,
    },
    StorageBindingTooLarge {
        role: &'static str,
        requested: u64,
        limit: u64,
    },
    NoDispatchCapacity,
    ReadbackSenderDropped,
    ReadbackMapFailed(String),
    DevicePollFailed(String),
    CorruptResult,
}

impl std::fmt::Display for GpuErrorbarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownColumn { role, id } => {
                write!(f, "unknown errorbar {role} column id: {id}")
            }
            Self::StaleHandle {
                role,
                id,
                generation,
                current,
            } => write!(
                f,
                "stale errorbar {role} column handle for {id} \
                 (handle generation {generation}, pool generation {current})"
            ),
            Self::EmptyColumn { role } => write!(f, "errorbar {role} column is empty"),
            Self::ValueCountTooLarge { role, len } => {
                write!(
                    f,
                    "errorbar {role} column length {len} exceeds u32 indexing"
                )
            }
            Self::InvalidColumnRange {
                role,
                offset,
                required,
                available,
                pool_size,
            } => write!(
                f,
                "errorbar {role} pool range is invalid: offset={offset}, required={required}, \
                 handle_bytes={available}, pool_bytes={pool_size}"
            ),
            Self::MisalignedColumnOffset {
                role,
                offset,
                alignment,
            } => write!(
                f,
                "errorbar {role} pool offset {offset} is not aligned to {alignment} bytes"
            ),
            Self::StorageBindingTooLarge {
                role,
                requested,
                limit,
            } => write!(
                f,
                "errorbar {role} storage binding requires {requested} bytes, device limit is {limit}"
            ),
            Self::NoDispatchCapacity => write!(f, "device exposes no usable errorbar dispatch"),
            Self::ReadbackSenderDropped => {
                write!(f, "errorbar readback callback sender was dropped")
            }
            Self::ReadbackMapFailed(reason) => {
                write!(f, "errorbar readback map failed: {reason}")
            }
            Self::DevicePollFailed(reason) => {
                write!(f, "errorbar submission poll failed: {reason}")
            }
            Self::CorruptResult => write!(f, "GPU returned an invalid errorbar extent record"),
        }
    }
}

impl std::error::Error for GpuErrorbarError {}

/// Pipelines and layouts intended to be created once and owned by Renderer.
pub struct GpuErrorbarExtentEngine {
    values_layout: wgpu::BindGroupLayout,
    states_layout: wgpu::BindGroupLayout,
    reduce_values: wgpu::ComputePipeline,
    reduce_states: wgpu::ComputePipeline,
}

impl GpuErrorbarExtentEngine {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figgy exact errorbar extent shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_errorbar.wgsl").into()),
        });

        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        let values_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy exact errorbar values layout"),
            entries: &[
                storage(0, true),
                storage(1, true),
                storage(2, true),
                storage(3, false),
                uniform(4),
            ],
        });
        let states_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy exact errorbar states layout"),
            entries: &[storage(0, true), storage(1, false), uniform(2)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("figgy exact errorbar pipeline layout"),
            bind_group_layouts: &[&values_layout, &states_layout],
            push_constant_ranges: &[],
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("figgy exact errorbar extent pipeline"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let reduce_values = pipeline("reduce_values");
        let reduce_states = pipeline("reduce_states");

        Self {
            values_layout,
            states_layout,
            reduce_values,
            reduce_states,
        }
    }

    /// Submit an exhaustive pairwise extent query and return its detached
    /// readback ticket.
    ///
    /// `values`, `lower_errors`, and `upper_errors` must still name live
    /// ranges in `pool_buffer`.  Renderer integration must therefore validate
    /// handle generation before calling this method.  Error columns may be
    /// shorter than `values`; missing lanes are exactly zero.  A non-finite
    /// anchor pair is skipped, while a non-finite error pair is exactly zero.
    pub fn begin(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool_buffer: &wgpu::Buffer,
        values: ColumnHandle,
        lower_errors: ColumnHandle,
        upper_errors: ColumnHandle,
    ) -> Result<GpuErrorbarExtentTicket, GpuErrorbarError> {
        let value_len = checked_u32_len("value", values.len_values)?;
        if value_len == 0 {
            return Err(GpuErrorbarError::EmptyColumn { role: "value" });
        }
        let lower_bound_len = lower_errors.len_values.min(values.len_values);
        let upper_bound_len = upper_errors.len_values.min(values.len_values);
        let lower_len = checked_u32_len("lower error", lower_bound_len)?;
        let upper_len = checked_u32_len("upper error", upper_bound_len)?;
        if lower_len == 0 {
            return Err(GpuErrorbarError::EmptyColumn {
                role: "lower error",
            });
        }
        if upper_len == 0 {
            return Err(GpuErrorbarError::EmptyColumn {
                role: "upper error",
            });
        }

        let limits = device.limits();
        let value_binding =
            checked_binding(pool_buffer, &values, values.len_values, "value", &limits)?;
        let lower_binding = checked_binding(
            pool_buffer,
            &lower_errors,
            lower_bound_len,
            "lower error",
            &limits,
        )?;
        let upper_binding = checked_binding(
            pool_buffer,
            &upper_errors,
            upper_bound_len,
            "upper error",
            &limits,
        )?;

        let max_scratch_groups = u64::from(limits.max_storage_buffer_binding_size) / STATE_BYTES;
        let dispatch_cap = u64::from(limits.max_compute_workgroups_per_dimension)
            .min(max_scratch_groups)
            .min(u64::from(u32::MAX)) as u32;
        if dispatch_cap == 0 {
            return Err(GpuErrorbarError::NoDispatchCapacity);
        }
        let first_groups = div_ceil(value_len, WORKGROUP_SIZE).min(dispatch_cap);
        let first_scratch_bytes = u64::from(first_groups) * STATE_BYTES;
        let second_capacity = div_ceil(first_groups, WORKGROUP_SIZE).max(1);
        let second_scratch_bytes = u64::from(second_capacity) * STATE_BYTES;

        let scratch_a = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("figgy exact errorbar scratch A"),
            size: first_scratch_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let scratch_b = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("figgy exact errorbar scratch B"),
            size: second_scratch_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("figgy exact errorbar detached readback"),
            size: STATE_BYTES,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        // wgpu validates every bind-group slot in an explicit pipeline layout,
        // including the group not reachable from the selected entry point.
        // Keep that inactive group's writable binding disjoint from every
        // live reduction buffer; the shader never reads or writes it.
        let inactive_output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("figgy exact errorbar inactive binding"),
            size: STATE_BYTES,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });

        let initial_params = params_buffer(
            device,
            "figgy exact errorbar value params",
            ParamsGpu {
                input_len: value_len,
                lower_len,
                upper_len,
                dispatch_groups: first_groups,
            },
        );
        let initial_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy exact errorbar value bindings"),
            layout: &self.values_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(value_binding.clone()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(lower_binding.clone()),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(upper_binding.clone()),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: scratch_a.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: initial_params.as_entire_binding(),
                },
            ],
        });
        let inactive_values_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy exact errorbar inactive values bindings"),
            layout: &self.values_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(value_binding),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Buffer(lower_binding),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(upper_binding),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: inactive_output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: initial_params.as_entire_binding(),
                },
            ],
        });
        let inactive_states_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy exact errorbar inactive states bindings"),
            layout: &self.states_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: scratch_b.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: inactive_output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: initial_params.as_entire_binding(),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy exact errorbar extent encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact errorbar values pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.reduce_values);
            pass.set_bind_group(0, &initial_bind_group, &[]);
            pass.set_bind_group(1, &inactive_states_bind_group, &[]);
            pass.dispatch_workgroups(first_groups, 1, 1);
        }

        let mut current_len = first_groups;
        let mut current_is_a = true;
        while current_len > 1 {
            let groups = div_ceil(current_len, WORKGROUP_SIZE);
            let params = params_buffer(
                device,
                "figgy exact errorbar state params",
                ParamsGpu {
                    input_len: current_len,
                    lower_len: 0,
                    upper_len: 0,
                    dispatch_groups: groups,
                },
            );
            let (input, output) = if current_is_a {
                (&scratch_a, &scratch_b)
            } else {
                (&scratch_b, &scratch_a)
            };
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("figgy exact errorbar state bindings"),
                layout: &self.states_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: input.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: params.as_entire_binding(),
                    },
                ],
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("figgy exact errorbar state pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.reduce_states);
                pass.set_bind_group(0, &inactive_values_bind_group, &[]);
                pass.set_bind_group(1, &bind_group, &[]);
                pass.dispatch_workgroups(groups, 1, 1);
            }
            current_len = groups;
            current_is_a = !current_is_a;
        }

        let final_buffer = if current_is_a { &scratch_a } else { &scratch_b };
        encoder.copy_buffer_to_buffer(final_buffer, 0, &readback, 0, STATE_BYTES);

        let (sender, receiver) = oneshot::channel();
        encoder.map_buffer_on_submit(
            &readback,
            wgpu::MapMode::Read,
            0..STATE_BYTES,
            move |result| {
                let _ = sender.send(result);
            },
        );
        let submission = queue.submit(std::iter::once(encoder.finish()));

        Ok(GpuErrorbarExtentTicket {
            device: device.clone(),
            readback,
            receiver,
            submission,
        })
    }
}

/// An owned, one-shot GPU readback.  It borrows neither Renderer nor the
/// column pool and may safely outlive the engine that submitted it.
pub struct GpuErrorbarExtentTicket {
    device: wgpu::Device,
    readback: wgpu::Buffer,
    receiver: oneshot::Receiver<Result<(), wgpu::BufferAsyncError>>,
    submission: wgpu::SubmissionIndex,
}

impl GpuErrorbarExtentTicket {
    pub async fn resolve(self) -> Result<Option<GpuErrorbarExtent>, GpuErrorbarError> {
        let Self {
            device,
            readback,
            receiver,
            submission,
        } = self;

        #[cfg(not(target_arch = "wasm32"))]
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission.clone()),
                timeout: None,
            })
            .map_err(|error| GpuErrorbarError::DevicePollFailed(format!("{error:?}")))?;
        #[cfg(target_arch = "wasm32")]
        let _ = (device, submission);

        receiver
            .await
            .map_err(|_| GpuErrorbarError::ReadbackSenderDropped)?
            .map_err(|error| GpuErrorbarError::ReadbackMapFailed(format!("{error:?}")))?;

        let slice = readback.slice(0..STATE_BYTES);
        let mapped = slice.get_mapped_range();
        let state = bytemuck::pod_read_unaligned::<ExtentStateGpu>(&mapped);
        drop(mapped);
        readback.unmap();
        decode_state(state)
    }
}

fn checked_u32_len(role: &'static str, len: usize) -> Result<u32, GpuErrorbarError> {
    u32::try_from(len).map_err(|_| GpuErrorbarError::ValueCountTooLarge { role, len })
}

fn checked_binding<'a>(
    pool_buffer: &'a wgpu::Buffer,
    handle: &ColumnHandle,
    bound_values: usize,
    role: &'static str,
    limits: &wgpu::Limits,
) -> Result<wgpu::BufferBinding<'a>, GpuErrorbarError> {
    let required = (bound_values as u64)
        .checked_mul(COLUMN_VALUE_BYTES as u64)
        .ok_or(GpuErrorbarError::ValueCountTooLarge {
            role,
            len: bound_values,
        })?;
    if required == 0 {
        return Err(GpuErrorbarError::EmptyColumn { role });
    }
    let pool_size = pool_buffer.size();
    let end = handle.offset.checked_add(required);
    if required > handle.byte_size || end.is_none_or(|end| end > pool_size) {
        return Err(GpuErrorbarError::InvalidColumnRange {
            role,
            offset: handle.offset,
            required,
            available: handle.byte_size,
            pool_size,
        });
    }
    let alignment = limits.min_storage_buffer_offset_alignment;
    if !handle.offset.is_multiple_of(u64::from(alignment)) {
        return Err(GpuErrorbarError::MisalignedColumnOffset {
            role,
            offset: handle.offset,
            alignment,
        });
    }
    let binding_limit = u64::from(limits.max_storage_buffer_binding_size);
    if required > binding_limit {
        return Err(GpuErrorbarError::StorageBindingTooLarge {
            role,
            requested: required,
            limit: binding_limit,
        });
    }
    let size = NonZeroU64::new(required).ok_or(GpuErrorbarError::EmptyColumn { role })?;
    Ok(wgpu::BufferBinding {
        buffer: pool_buffer,
        offset: handle.offset,
        size: Some(size),
    })
}

fn div_ceil(value: u32, divisor: u32) -> u32 {
    value / divisor + u32::from(!value.is_multiple_of(divisor))
}

fn params_buffer(device: &wgpu::Device, label: &'static str, params: ParamsGpu) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::bytes_of(&params),
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

fn decode_endpoint(endpoint: EndpointGpu) -> Result<Option<f64>, GpuErrorbarError> {
    if endpoint.kind == ENDPOINT_INVALID {
        return Ok(None);
    }
    if endpoint.kind != ENDPOINT_LOWER && endpoint.kind != ENDPOINT_UPPER {
        return Err(GpuErrorbarError::CorruptResult);
    }
    if !endpoint.value.into_iter().all(f32::is_finite)
        || !endpoint.error.into_iter().all(f32::is_finite)
    {
        return Err(GpuErrorbarError::CorruptResult);
    }
    let value = endpoint.value[0] as f64 + endpoint.value[1] as f64;
    let error = endpoint.error[0] as f64 + endpoint.error[1] as f64;
    Ok(Some(if endpoint.kind == ENDPOINT_LOWER {
        value - error
    } else {
        value + error
    }))
}

fn decode_state(state: ExtentStateGpu) -> Result<Option<GpuErrorbarExtent>, GpuErrorbarError> {
    let Some(min) = decode_endpoint(state.minimum)? else {
        if state.maximum.kind == ENDPOINT_INVALID && state.minimum_positive.kind == ENDPOINT_INVALID
        {
            return Ok(None);
        }
        return Err(GpuErrorbarError::CorruptResult);
    };
    let max = decode_endpoint(state.maximum)?.ok_or(GpuErrorbarError::CorruptResult)?;
    let min_positive = decode_endpoint(state.minimum_positive)?;
    if min > max || min_positive.is_some_and(|value| value <= 0.0) {
        return Err(GpuErrorbarError::CorruptResult);
    }
    Ok(Some(GpuErrorbarExtent {
        min,
        max,
        min_positive,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Column, split_f64_to_f32_pair};
    use crate::data_render::ColumnPool;

    fn f32_column(values: Vec<f32>) -> Column<f32> {
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for &value in &values {
            if value < min {
                min = value;
            }
            if value > max {
                max = value;
            }
        }
        Column {
            data: values,
            min,
            max,
        }
    }

    fn f64_column(values: Vec<f64>) -> Column<f64> {
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for &value in &values {
            if value < min {
                min = value;
            }
            if value > max {
                max = value;
            }
        }
        Column {
            data: values,
            min,
            max,
        }
    }

    #[test]
    fn gpu_extent_matches_nan_and_short_error_oracle() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(&device, 64 * 1024).unwrap();
        let values = pool
            .add_column(
                "gpu-eb-values".into(),
                &f32_column(vec![f32::NAN, 2.0, 4.0]),
                &device,
                &queue,
            )
            .unwrap();
        let lower = pool
            .add_column(
                "gpu-eb-lower".into(),
                &f32_column(vec![100.0, f32::NAN]),
                &device,
                &queue,
            )
            .unwrap();
        let upper = pool
            .add_column(
                "gpu-eb-upper".into(),
                &f32_column(vec![100.0, f32::NAN]),
                &device,
                &queue,
            )
            .unwrap();

        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = pollster::block_on(
            engine
                .begin(&device, &queue, pool.buffer(), values, lower, upper)
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(extent.min, 2.0);
        assert_eq!(extent.max, 4.0);
        assert_eq!(extent.min_positive, Some(2.0));
    }

    #[test]
    fn gpu_extent_keeps_f32_overflow_out_of_f32_arithmetic() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(&device, 64 * 1024).unwrap();
        let values = pool
            .add_column(
                "gpu-eb-max-values".into(),
                &f32_column(vec![f32::MAX]),
                &device,
                &queue,
            )
            .unwrap();
        let errors = pool
            .add_column(
                "gpu-eb-max-errors".into(),
                &f32_column(vec![f32::MAX]),
                &device,
                &queue,
            )
            .unwrap();

        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = pollster::block_on(
            engine
                .begin(&device, &queue, pool.buffer(), values, errors, errors)
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(extent.min, 0.0);
        assert_eq!(extent.max, f32::MAX as f64 + f32::MAX as f64);
        assert_eq!(extent.min_positive, Some(extent.max));
    }

    #[test]
    fn gpu_extent_preserves_hilo_source_residuals() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(&device, 64 * 1024).unwrap();
        let raw_value = 1_700_000_000_000.125_f64;
        let raw_lower = 0.25_f64;
        let raw_upper = 0.75_f64;
        let values = pool
            .add_hilo_column(
                "gpu-eb-hilo-values".into(),
                &f64_column(vec![raw_value]),
                &device,
                &queue,
            )
            .unwrap();
        let lower = pool
            .add_hilo_column(
                "gpu-eb-hilo-lower".into(),
                &f64_column(vec![raw_lower]),
                &device,
                &queue,
            )
            .unwrap();
        let upper = pool
            .add_hilo_column(
                "gpu-eb-hilo-upper".into(),
                &f64_column(vec![raw_upper]),
                &device,
                &queue,
            )
            .unwrap();

        let pair_sum = |value: f64| {
            let (hi, lo) = split_f64_to_f32_pair(value);
            hi as f64 + lo as f64
        };
        let expected_min = pair_sum(raw_value) - pair_sum(raw_lower);
        let expected_max = pair_sum(raw_value) + pair_sum(raw_upper);

        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = pollster::block_on(
            engine
                .begin(&device, &queue, pool.buffer(), values, lower, upper)
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(extent.min, expected_min);
        assert_eq!(extent.max, expected_max);
        assert_eq!(extent.min_positive, Some(expected_min));
    }

    #[test]
    fn gpu_extent_reduces_across_workgroups() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(&device, 64 * 1024).unwrap();
        let values = pool
            .add_column(
                "gpu-eb-many-values".into(),
                &f32_column((0..1000).map(|value| value as f32).collect()),
                &device,
                &queue,
            )
            .unwrap();
        let lower = pool
            .add_column(
                "gpu-eb-many-lower".into(),
                &f32_column(vec![5.0; 1000]),
                &device,
                &queue,
            )
            .unwrap();
        let upper = pool
            .add_column(
                "gpu-eb-many-upper".into(),
                &f32_column(vec![7.0; 1000]),
                &device,
                &queue,
            )
            .unwrap();

        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = pollster::block_on(
            engine
                .begin(&device, &queue, pool.buffer(), values, lower, upper)
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(extent.min, -5.0);
        assert_eq!(extent.max, 1006.0);
        assert_eq!(extent.min_positive, Some(1.0));
    }
}
