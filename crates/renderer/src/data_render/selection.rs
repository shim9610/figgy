//! The data selection uniform: which bin, cell or level is picked.

use super::*;

pub(crate) const DATA_SELECTION_KIND_HISTOGRAM_BIN: u32 = 1;

pub(crate) const DATA_SELECTION_KIND_MATRIX_CELL: u32 = 2;

pub(crate) const DATA_SELECTION_KIND_CONTOUR_LEVEL: u32 = 3;

/// Small typed-selection uniform shared by the bar and field overlay entries.
///
/// GPU twin: `common/data_pick.wgsl::DataSelection` (the bar and field
/// modules splice it), 48 bytes. It contains provenance
/// indices plus visual scalars, never reconstructed data coordinates.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct DataSelectionGpu {
    pub color_premul: [f32; 4],
    pub metrics: [f32; 4],
    pub indices: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<DataSelectionGpu>() == 48);

impl DataSelectionGpu {
    pub(crate) fn from_color(color: Color) -> Self {
        let alpha = color.a.clamp(0.0, 1.0);
        Self {
            color_premul: [color.r * alpha, color.g * alpha, color.b * alpha, alpha],
            metrics: [0.0; 4],
            indices: [0; 4],
        }
    }
}

pub(crate) fn create_data_selection_bind_group_layout(
    device: &wgpu::Device,
) -> wgpu::BindGroupLayout {
    crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy typed data selection bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        },
    )
}

pub(crate) fn create_data_selection_bind_group(
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    selection: &DataSelectionGpu,
) -> (wgpu::BindGroup, crate::gpu_memory::SharedCharge) {
    let tally = crate::gpu_memory::ChargeTally::new();
    let buffer = crate::gpu_memory::charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy typed data selection uniform"),
            contents: bytemuck::bytes_of(selection),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy typed data selection bg"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 4,
            resource: buffer.as_entire_binding(),
        }],
    });
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::Uniform,
    );
    (bind_group, charge)
}
