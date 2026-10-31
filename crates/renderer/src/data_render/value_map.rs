//! Point value maps: colour and size of a point resolved from a resident
//! column in the shader.

use super::*;

/// `PointValueMapParams.flags` bits.
pub const VALUE_MAP_COLOR: u32 = 1;

pub const VALUE_MAP_SIZE: u32 = 2;

pub const VALUE_MAP_LOG_Z: u32 = 4;

pub const VALUE_MAP_SIZE_AREA: u32 = 8;

/// The continuous point value maps of one scatter series.
///
/// GPU twin: `scatter_columnar.wgsl::PointValueMapParams`, 64 B. `z_min` /
/// `z_max` are the colourbar's bounds as the pool's `(hi, lo)` pair, already
/// log-transformed when [`VALUE_MAP_LOG_Z`] is set, exactly as
/// [`FieldParamsGpu`] carries them. The size domain is `PointSizeMap`'s, also
/// split hi/lo so a value column of large magnitude still resolves small
/// deltas; `size_px` is `(min_px, max_px)` multiplied by the export scale.
/// Nothing derived per point lives here: colour and radius are computed by the
/// shader on every draw from the value columns bound as vertex slots.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PointValueMapParamsGpu {
    pub z_min: [f32; 2],
    pub z_max: [f32; 2],
    pub size_domain_min: [f32; 2],
    pub size_domain_max: [f32; 2],
    pub size_px: [f32; 2],
    pub flags: u32,
    pub stop_count: u32,
    pub nan_color_premul: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<PointValueMapParamsGpu>() == 64);

impl PointValueMapParamsGpu {
    /// A series without maps: every flag clear. Bound as group 3 wherever a
    /// mapped scatter pipeline draws a series that has no value map, so the
    /// pipeline layout is satisfied and the shader reads nothing from it.
    pub const NEUTRAL: Self = Self {
        z_min: [0.0; 2],
        z_max: [0.0; 2],
        size_domain_min: [0.0; 2],
        size_domain_max: [0.0; 2],
        size_px: [0.0; 2],
        flags: 0,
        stop_count: 0,
        nan_color_premul: [0.0; 4],
    };
}

/// Bind group layout for a series' continuous value maps (group 3 in the
/// mapped precise scatter and pick-ring pipelines, which read it in the
/// vertex stage, and in the colour-mapped line pipeline, which maps the
/// value interpolated along the stroke in the fragment stage).
/// - binding 0: [`PointValueMapParamsGpu`].
/// - binding 1: the colourbar's ramp control points, straight RGBA, at least
///   one entry (a storage binding cannot be zero-sized).
pub fn create_point_value_map_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy point value map bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        },
    )
}

/// One series' value-map buffers: the params uniform (binding 0) and the
/// ramp (binding 1). Kept beside their bind group so a 3D chart can bind
/// them again with the camera in one group (#132).
pub struct PointValueMapBuffers {
    pub params: wgpu::Buffer,
    pub stops: wgpu::Buffer,
}

/// Upload one series' value-map params and ramp into the layout above.
///
/// The stops are written straight into the mapped stop buffer, one `vec4` per
/// control point; an empty ramp still gets one zero entry so the storage
/// binding is valid, with `params.stop_count` carrying the truth.
pub fn create_point_value_map_bind_group(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    layout: &wgpu::BindGroupLayout,
    params: &PointValueMapParamsGpu,
    stops: &[Color],
) -> (wgpu::BindGroup, crate::gpu_memory::SharedCharge) {
    let (buffers, charge) = create_point_value_map_buffers(device, ledger, params, stops);
    (bind_point_value_map(device, layout, &buffers), charge)
}

/// The value-map group over existing buffers (`create_point_value_map_buffers`).
pub fn bind_point_value_map(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: &PointValueMapBuffers,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy point value map bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers.params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers.stops.as_entire_binding(),
            },
        ],
    })
}

/// Upload one series' value-map params and ramp (see
/// [`create_point_value_map_bind_group`]).
pub fn create_point_value_map_buffers(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    params: &PointValueMapParamsGpu,
    stops: &[Color],
) -> (PointValueMapBuffers, crate::gpu_memory::SharedCharge) {
    use crate::gpu_memory::{ChargeTally, charged_buffer, charged_buffer_init};
    let tally = ChargeTally::new();
    let param_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy point value map params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let stop_entries = stops.len().max(1) as u64;
    let stop_buf = charged_buffer(
        &tally,
        device,
        &wgpu::BufferDescriptor {
            label: Some("figgy point value map ramp"),
            size: stop_entries * 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        },
    );
    {
        let mut view = stop_buf
            .slice(..)
            .get_mapped_range_mut()
            .expect("a buffer mapped at creation is mappable");
        let mut bytes = view.slice(..);
        bytes.fill(0);
        for (index, color) in stops.iter().enumerate() {
            let rgba = [color.r, color.g, color.b, color.a];
            bytes
                .slice(index * 16..index * 16 + 16)
                .copy_from_slice(bytemuck::bytes_of(&rgba));
        }
    }
    stop_buf.unmap();
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::Uniform,
    );
    (
        PointValueMapBuffers {
            params: param_buf,
            stops: stop_buf,
        },
        charge,
    )
}
