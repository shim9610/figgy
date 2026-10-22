//! Contour series: the level lookup, the labelled contour pipelines and the
//! contour layers.

use super::*;

/// Contour lookup granularity. Each block fits in one shader `u32` candidate
/// mask, so the original declaration order can be restored without a second
/// per-fragment table or an O(level_count) scan.
pub(crate) const CONTOUR_LEVEL_BLOCK_SIZE: usize = 32;

pub(crate) const CONTOUR_LEVEL_BLOCK_COUNT: usize = 32;

pub(crate) const CONTOUR_LEVEL_LOOKUP_CAPACITY: usize =
    CONTOUR_LEVEL_BLOCK_SIZE * CONTOUR_LEVEL_BLOCK_COUNT;
const _: () = assert!(crate::data_config::MAX_CONTOUR_LEVELS == CONTOUR_LEVEL_LOOKUP_CAPACITY);

/// Per-32-level lookup bounds uploaded at field group-2 binding 6.
///
/// GPU twin: `field_columnar.wgsl::ContourLookupMetadata`, 8 B. Counts are
/// derived from the actual f32 search keys, after the declared f64 levels have
/// crossed the renderer upload boundary.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ContourLookupMetadataGpu {
    pub finite_count: u32,
    pub negative_infinity_count: u32,
}

const _: () = assert!(std::mem::size_of::<ContourLookupMetadataGpu>() == 8);

/// The selected-field highlight for a labelled contour: group 3 is the
/// series' label-gap group, which `fs_data_selection_labelled` reads.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_field_selection_labelled_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    selection_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    label_gap_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_field_columnar_pipeline_with_optional_extra_layout(
        device,
        shader,
        transform_bgl,
        selection_bgl,
        field_bgl,
        Some(label_gap_bgl),
        target_format,
        sample_count,
        "fs_data_selection_labelled",
        "figgy selected labelled field data pipeline",
    )
}

/// Labelled contours add group 3 containing the exact selected-anchor buffers.
/// No-label contours keep the original layout and entry point, so merely adding
/// contour support does not compile or bind label machinery.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_labelled_contour_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    field_bgl: &wgpu::BindGroupLayout,
    label_gap_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_field_columnar_pipeline_with_optional_extra_layout(
        device,
        shader,
        transform_bgl,
        style_bgl,
        field_bgl,
        Some(label_gap_bgl),
        target_format,
        sample_count,
        "fs_contour_labelled",
        "figgy labelled field contour pipeline",
    )
}

/// One contour series' draw.
///
/// The **same quad as the field**, through `fs_contour`: the isolines are the
/// level set of the same bilinear interpolation, so there is no segment list, no
/// indirect draw and no per-level draw call. Per-level colour comes from the
/// group-2 `level_colors` table. Stroke coverage comes from the quadratic root
/// of the current cell's bilinear polynomial along the fragment's gradient
/// normal; it is not a claim of globally shortest Euclidean distance.
pub struct ColumnContourLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    /// group(1): the fallback colour for an unplaceable z. Unread by
    /// `fs_contour`, but the layout is shared with the fill.
    pub style_bg: wgpu::BindGroup,
    /// group(2): pool, grid table, levels, stops, params, level colours.
    pub field_bg: wgpu::BindGroup,
    /// Keeps the group-2 lump charge alive for as long as this layer can draw.
    pub charge: std::sync::Arc<crate::gpu_memory::GpuByteCharge>,
    /// False when the grid has no cell to interpolate across.
    pub drawable: bool,
    /// `None` when the series draws no labels.
    pub label: Option<ColumnContourLabel<'a>>,
}

/// The inline level labels for one contour series.
///
/// One quad per anchor, textured from the CPU-baked atlas. The serial
/// `anchor_select` pass writes the indirect instance count; the CPU neither
/// learns it nor needs to. A host override writes the same records and argument
/// quad, so there is exactly one draw path.
pub struct ColumnContourLabel<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    /// Immutable atlas plus this occurrence's explicit or exact-key automatic
    /// placement resources and their ledger charges.
    pub(crate) snapshot: crate::gpu_contour::ContourLabelSnapshot,
}
