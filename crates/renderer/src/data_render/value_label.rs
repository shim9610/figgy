//! Value labels of the category bar, box plot and radar series: the CPU
//! half of `common/value_label.wgsl` (SHADER_COMMON.md §5.17).
//!
//! A series' strings are SSOT configuration (`labels.text`), so the CPU
//! bakes them into one label atlas with the contour labels' builder
//! (`bake_contour_label_atlas`: the one text stack, `FontPolicy`) and the
//! GPU places each label from the pool on every draw, through the bar
//! module's value label entries. What the CPU writes per label is
//! configuration only: the measured width of its string's atlas cell, its
//! placement and its drag offset (`labels.offsets`), in one fixed-size
//! uniform. Nothing is computed from the data on the CPU and nothing per
//! label is stored beyond the atlas.

use super::*;
use crate::data_config::{BoxPlotLabels, CategoryBarLabels, DataRenderType, RadarLabels};

/// Labels one series draws: one per category of its axis, for the first
/// `VALUE_LABEL_CAPACITY` categories. A categorical axis has no category
/// cap (#107), so an axis with more categories draws its series in full
/// and labels the first 64 only; the labels past the uniform's cells move
/// to a storage buffer sized by the labels in a later change (#107 R11).
/// GPU twin: `value_label.wgsl::VALUE_LABEL_CAPACITY`.
pub const VALUE_LABEL_CAPACITY: usize = 64;

/// Vertices per label: one quad (`VALUE_LABEL_VERTICES`).
pub const VALUE_LABEL_VERTICES: u32 = 6;

/// `ValueLabelParamsGpu::shape[0]`: the series kind whose geometry the
/// labels follow (`VALUE_LABEL_KIND_*`).
pub(crate) const VALUE_LABEL_KIND_BARS: u32 = 0;
pub(crate) const VALUE_LABEL_KIND_BOXES: u32 = 1;
pub(crate) const VALUE_LABEL_KIND_RADAR: u32 = 2;
/// A label's placement (`VALUE_LABEL_OUTSIDE` / `_INSIDE` / `_AUTO`).
pub(crate) const VALUE_LABEL_OUTSIDE: u32 = 0;
pub(crate) const VALUE_LABEL_INSIDE: u32 = 1;
pub(crate) const VALUE_LABEL_AUTO: u32 = 2;

/// Logical pixels between a bar's value end and its outside label's
/// baseline (or text top below a falling end), as the former categorical
/// chart placed them.
pub(crate) const BAR_LABEL_GAP: f32 = 6.0;
/// Logical pixels between the top of a box and its label's baseline.
pub(crate) const BOX_LABEL_GAP: f32 = 10.0;
/// Logical pixels between a radar vertex and its label along the spoke.
pub(crate) const RADAR_LABEL_GAP: f32 = 9.0;

/// Everything the bar module's value label entries read besides the
/// transform, the series' bar style map and its category bar group 3: the
/// atlas geometry, the kind, and per label its content width, placement
/// and drag offset.
///
/// GPU twin: `value_label.wgsl::ValueLabelParams`, 1120 B.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ValueLabelParamsGpu {
    pub atlas: crate::gpu_contour::LabelParamsGpu,
    /// `[kind, labels, 0, 0]`.
    pub shape: [u32; 4],
    /// `[gap px, baseline above the content bottom px, font px, scale]`.
    pub metrics: [f32; 4],
    /// The selection box colour, premultiplied.
    pub selection_color: [f32; 4],
    /// `[stroke width px, padding px, 0, 0]`.
    pub selection: [f32; 4],
    /// Per label `[content width px, placement, offset x px, offset y px]`.
    pub cells: [[f32; 4]; VALUE_LABEL_CAPACITY],
}

const _: () = assert!(std::mem::size_of::<ValueLabelParamsGpu>() == 1120);

/// One series' baked value label atlas on the GPU: the texture its labels
/// sample, its cell geometry, and each string's measured cell width.
pub struct ValueLabelAtlas {
    pub(crate) view: wgpu::TextureView,
    pub(crate) texture: crate::gpu_memory::TrackedTexture,
    pub(crate) geometry: crate::gpu_contour::LabelParamsGpu,
    /// Content width of each string's cell in texels (render pixels).
    pub(crate) widths: [f32; VALUE_LABEL_CAPACITY],
    /// The text baseline's height above the cell content's bottom edge.
    pub(crate) baseline_px: f32,
}

impl ValueLabelAtlas {
    /// Upload `baked`, the series' strings as `bake_contour_label_atlas`
    /// baked them.
    pub(crate) fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
        max_texture_dimension_2d: u32,
        baked: &BakedLabelAtlas,
    ) -> crate::Result<Self> {
        let texture = crate::gpu_memory::TrackedTexture::new(
            ledger,
            crate::gpu_memory::GpuResourceKind::ContourScratch,
            upload_rgba_texture(
                device,
                queue,
                max_texture_dimension_2d,
                baked.width,
                baked.height,
                &baked.rgba,
            )?,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut widths = [0.0; VALUE_LABEL_CAPACITY];
        for (width, cell) in widths.iter_mut().zip(&baked.cells) {
            *width = cell.width;
        }
        Ok(Self {
            view,
            texture,
            geometry: crate::gpu_contour::LabelParamsGpu {
                atlas_w: baked.width as f32,
                atlas_h: baked.height as f32,
                cell_stride_w: baked.cell_stride_w as f32,
                cell_stride_h: baked.cell_stride_h as f32,
                columns: baked.columns,
                rows: baked.rows,
                gutter: baked.gutter as f32,
                level_count: u32::try_from(baked.cells.len()).unwrap_or(u32::MAX),
            },
            widths,
            baseline_px: baked.baseline,
        })
    }
}

/// The value labels a series draws: its kind and its strings. `None` for a
/// series that is not a category bar, box plot or radar series, or that
/// draws no labels: placement `None` (the series' and every override's),
/// no non-empty string, or a radar series whose `Emphasized` labels have no
/// selection. `selected` is whether the series has a selected vertex or
/// label (radar `Emphasized`).
pub(crate) fn value_label_kind(
    render_type: &DataRenderType,
    selected: bool,
) -> Option<(u32, &[String])> {
    let kind = match render_type {
        DataRenderType::CategoryBars {
            labels, overrides, ..
        } => {
            let any = labels.placement != CategoryBarLabels::None
                || overrides.as_deref().unwrap_or(&[]).iter().any(|o| {
                    o.labels
                        .is_some_and(|placement| placement != CategoryBarLabels::None)
                });
            any.then_some((VALUE_LABEL_KIND_BARS, labels.text.as_slice()))
        }
        DataRenderType::BoxPlot {
            labels, overrides, ..
        } => {
            let any = labels.placement != BoxPlotLabels::None
                || overrides.as_deref().unwrap_or(&[]).iter().any(|o| {
                    o.labels
                        .is_some_and(|placement| placement != BoxPlotLabels::None)
                });
            any.then_some((VALUE_LABEL_KIND_BOXES, labels.text.as_slice()))
        }
        DataRenderType::Radar { labels, .. } => {
            let shown = match labels.placement {
                RadarLabels::None => false,
                RadarLabels::Values => true,
                RadarLabels::Emphasized => selected,
            };
            shown.then_some((VALUE_LABEL_KIND_RADAR, labels.text.as_slice()))
        }
        _ => None,
    };
    kind.filter(|(_, text)| text.iter().any(|t| !t.is_empty()))
}

/// The placement of label `index` (category `category_id`) of a series
/// [`value_label_kind`] accepted, or `None` when the label is hidden.
fn label_placement(render_type: &DataRenderType, category_id: &str) -> Option<u32> {
    match render_type {
        DataRenderType::CategoryBars {
            labels, overrides, ..
        } => {
            let placement = overrides
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .find(|o| o.target.category_id == category_id)
                .and_then(|o| o.labels)
                .unwrap_or(labels.placement);
            match placement {
                CategoryBarLabels::None => None,
                CategoryBarLabels::Inside => Some(VALUE_LABEL_INSIDE),
                CategoryBarLabels::Outside => Some(VALUE_LABEL_OUTSIDE),
                CategoryBarLabels::Auto => Some(VALUE_LABEL_AUTO),
            }
        }
        DataRenderType::BoxPlot {
            labels, overrides, ..
        } => {
            let placement = overrides
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .find(|o| o.target.category_id == category_id)
                .and_then(|o| o.labels)
                .unwrap_or(labels.placement);
            (placement != BoxPlotLabels::None).then_some(VALUE_LABEL_OUTSIDE)
        }
        DataRenderType::Radar { .. } => Some(VALUE_LABEL_OUTSIDE),
        _ => None,
    }
}

/// The selection box policy the model's selection sink draws decorations
/// with, in render pixels at `scale`: colour, stroke width, padding.
fn selection_policy(scale: f32) -> ([f32; 4], f32, f32) {
    let c = crate::select::SELECTION_COLOR;
    (
        [c.r * c.a, c.g * c.a, c.b * c.a, c.a],
        crate::select::SELECTION_STROKE_WIDTH * scale,
        crate::select::SELECTION_PADDING * scale,
    )
}

/// The value label uniform of one series: the atlas geometry, its kind,
/// and per label the width of its string's cell (zero for an empty string
/// or a label its placement hides), its placement and its drag offset at
/// `scale` render pixels per logical pixel. `categories` are the category
/// ids of the series' axis in order.
pub(crate) fn value_label_params<'c>(
    render_type: &DataRenderType,
    kind: u32,
    text: &[String],
    offsets: &[[f32; 2]],
    categories: impl IntoIterator<Item = &'c str>,
    atlas: &ValueLabelAtlas,
    font_px: f32,
    scale: f32,
) -> ValueLabelParamsGpu {
    let gap = match kind {
        VALUE_LABEL_KIND_BARS => BAR_LABEL_GAP,
        VALUE_LABEL_KIND_BOXES => BOX_LABEL_GAP,
        _ => RADAR_LABEL_GAP,
    };
    let (color, stroke, padding) = selection_policy(scale);
    let mut params = ValueLabelParamsGpu {
        atlas: atlas.geometry,
        shape: [kind, 0, 0, 0],
        metrics: [gap * scale, atlas.baseline_px, font_px, scale],
        selection_color: color,
        selection: [stroke, padding, 0.0, 0.0],
        cells: [[0.0; 4]; VALUE_LABEL_CAPACITY],
    };
    let mut count = 0;
    for (index, category) in categories
        .into_iter()
        .enumerate()
        .take(text.len().min(VALUE_LABEL_CAPACITY))
    {
        count = index + 1;
        let Some(placement) = label_placement(render_type, category) else {
            continue;
        };
        if text[index].is_empty() {
            continue;
        }
        let [dx, dy] = offsets.get(index).copied().unwrap_or([0.0; 2]);
        params.cells[index] = [
            atlas.widths[index],
            placement as f32,
            dx * scale,
            dy * scale,
        ];
    }
    params.shape[1] = count as u32;
    params
}

/// Group 1 of the value label draw and selection pipelines: the label
/// uniform (binding 4), the atlas (5) and its sampler (6). Bindings 0..3
/// of group 1 are the bar module's `Style` and typed pick query.
pub fn create_value_label_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    crate::gpu_compile::bind_group_layout(
        device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy value label bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        },
    )
}

/// One series' value label uniform, charged as a uniform, and the group 1
/// that binds it with the series' atlas.
#[derive(Clone)]
pub struct ValueLabelData {
    pub bind_group: wgpu::BindGroup,
    /// The uniform alone: the typed pick binds it into its own group 1.
    pub params: std::sync::Arc<wgpu::Buffer>,
    pub count: u32,
    pub charge: crate::gpu_memory::SharedCharge,
    /// Keeps the atlas texture's ledger charge alive with the bind group.
    pub atlas_charge: crate::gpu_memory::SharedCharge,
}

/// Upload `params` and bind it with `atlas` and `sampler` into `layout`
/// ([`create_value_label_bind_group_layout`]).
pub(crate) fn create_value_label_data(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    layout: &wgpu::BindGroupLayout,
    atlas: &ValueLabelAtlas,
    sampler: &wgpu::Sampler,
    params: &ValueLabelParamsGpu,
) -> ValueLabelData {
    use crate::gpu_memory::{ChargeTally, charged_buffer_init};
    let tally = ChargeTally::new();
    let buffer = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy value label params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::Uniform,
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy value label bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 4,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(&atlas.view),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    ValueLabelData {
        bind_group,
        params: std::sync::Arc::new(buffer),
        count: params.shape[1],
        charge,
        atlas_charge: atlas.texture.shared_charge(),
    }
}

/// A value label pipeline of the bar module: groups 0 transform, 1 the
/// series' value label group, 2 its bar style map, 3 its category bar data;
/// no vertex buffers. `vs_value_label` / `fs_value_label` draw the labels,
/// `vs_value_label_selection` / `fs_value_label_selection` the selected
/// label's box.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_value_label_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    value_label_bgl: &wgpu::BindGroupLayout,
    style_map_bgl: &wgpu::BindGroupLayout,
    data_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    selection: bool,
) -> wgpu::RenderPipeline {
    let entries = if selection {
        (
            "vs_value_label_selection",
            "fs_value_label_selection",
            "figgy value label selection pipeline",
        )
    } else {
        (
            "vs_value_label",
            "fs_value_label",
            "figgy value label pipeline",
        )
    };
    create_category_axis_pipeline(
        device,
        shader,
        [transform_bgl, value_label_bgl, style_map_bgl, data_bgl],
        target_format,
        sample_count,
        entries,
    )
}

/// One series' value labels: `count` instances of [`VALUE_LABEL_VERTICES`]
/// over its category bar group 3 and style map, and, when one of them is
/// selected, its selection box.
pub struct ColumnValueLabelLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub labels: ValueLabelData,
    pub data: CategoryBarData,
    /// The selected label and the selection box pipeline.
    pub selected: Option<(u32, &'a wgpu::RenderPipeline)>,
}

/// Issue one series' value labels, then its selected label's box. The
/// caller has set the viewport (panel) and the scissor (data area).
pub(crate) fn issue_value_labels(
    pass: &mut wgpu::RenderPass<'_>,
    layer: &ColumnValueLabelLayer<'_>,
) {
    if layer.labels.count == 0 {
        return;
    }
    pass.set_pipeline(layer.pipeline);
    pass.set_bind_group(0, layer.transform_bg, &[]);
    pass.set_bind_group(1, &layer.labels.bind_group, &[]);
    pass.set_bind_group(2, &layer.data.style_map, &[]);
    pass.set_bind_group(3, &layer.data.bind_group, &[]);
    pass.draw(0..VALUE_LABEL_VERTICES, 0..layer.labels.count);
    if let Some((row, pipeline)) = layer.selected
        && row < layer.labels.count
    {
        pass.set_pipeline(pipeline);
        pass.draw(0..VALUE_LABEL_VERTICES, row..row + 1);
    }
}

/// The category bar group 3 params of a radar series' value labels: the
/// category column as `positions` and the value column as `values[0][0]`,
/// one label per row, a group of one. The radar entries read nothing else.
pub(crate) fn radar_label_params(
    positions: u32,
    values: u32,
    count: u32,
    scale: f32,
) -> CategoryBarParamsGpu {
    let mut params: CategoryBarParamsGpu = bytemuck::Zeroable::zeroed();
    params.values[0][0] = values;
    params.positions = positions;
    params.count = count;
    params.members = 1;
    params.mode = CATEGORY_BAR_MODE_GROUPED;
    params.scale = scale;
    params
}
