//! Value labels of the category bar, box plot and radar series: the CPU
//! half of `common/value_label.wgsl` (SHADER_COMMON.md §5.17).
//!
//! A series' strings are SSOT configuration (`labels.text`), so the CPU
//! bakes them into one label atlas with the contour labels' builder
//! (`bake_contour_label_atlas`: the one text stack, `FontPolicy`) and the
//! GPU places each label from the pool on every draw, through the bar
//! module's value label entries. What the CPU writes per label is
//! configuration only: the measured width of its string's atlas cell, its
//! placement and its drag offset (`labels.offsets`), one cell per label
//! written straight into the mapping of a storage buffer sized by the
//! labels (group 1 binding 7, #107 R11). Nothing is computed from the data
//! on the CPU and nothing per label is stored beyond the atlas and its
//! bake's cell widths.

use super::*;
use crate::data_config::{BoxPlotLabels, CategoryBarLabels, DataRenderType, RadarLabels};

/// Vertices per label: one quad (`VALUE_LABEL_VERTICES`).
pub const VALUE_LABEL_VERTICES: u32 = 6;

/// `ValueLabelParamsGpu::shape[0]`: the series kind whose geometry the
/// labels follow (`VALUE_LABEL_KIND_*`).
pub(crate) const VALUE_LABEL_KIND_BARS: u32 = 0;
pub(crate) const VALUE_LABEL_KIND_BOXES: u32 = 1;
pub(crate) const VALUE_LABEL_KIND_RADAR: u32 = 2;
pub(crate) const VALUE_LABEL_KIND_RANGE: u32 = 3;
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
/// transform, the series' bar style map, its category bar group 3 and its
/// label cells (one per label: content width, placement and drag offset,
/// `value_label_cells`): the atlas geometry, the kind and the label
/// count.
///
/// GPU twin: `value_label.wgsl::ValueLabelParams`, 96 B.
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
}

const _: () = assert!(std::mem::size_of::<ValueLabelParamsGpu>() == 96);

/// One series' baked value label atlas on the GPU: the texture its labels
/// sample, its cell geometry, and each string's measured cell width.
pub struct ValueLabelAtlas {
    pub(crate) view: wgpu::TextureView,
    pub(crate) texture: crate::gpu_memory::TrackedTexture,
    pub(crate) geometry: crate::gpu_contour::LabelParamsGpu,
    /// Content width of each string's cell in texels (render pixels), one
    /// per string: the bake's own cells, kept with the atlas they measure
    /// (configuration, not data).
    pub(crate) cells: Vec<LabelAtlasCell>,
    /// The text baseline's height above the cell content's bottom edge.
    pub(crate) baseline_px: f32,
}

impl ValueLabelAtlas {
    /// Upload `baked`, the series' strings as `bake_contour_label_atlas`
    /// baked them; the atlas keeps the bake's cell widths.
    pub(crate) fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
        max_texture_dimension_2d: u32,
        baked: BakedLabelAtlas,
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
            baseline_px: baked.baseline,
            cells: baked.cells,
        })
    }
}

/// The value labels a series draws: its kind and its strings. `None` for a
/// series that is not a category bar, box plot, range bar or radar series,
/// or that
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
        DataRenderType::RangeBars { labels, .. } => (labels.placement
            != CategoryBarLabels::None)
            .then_some((VALUE_LABEL_KIND_RANGE, labels.text.as_slice())),
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

/// The placement of a label of a series [`value_label_kind`] accepted (of
/// category `category_id`, which a range bar series' row labels have
/// none of), or `None` when the label is hidden.
fn label_placement(render_type: &DataRenderType, category_id: Option<&str>) -> Option<u32> {
    let category_id = category_id.unwrap_or_default();
    let bar_placement = |placement: CategoryBarLabels| match placement {
        CategoryBarLabels::None => None,
        CategoryBarLabels::Inside => Some(VALUE_LABEL_INSIDE),
        CategoryBarLabels::Outside => Some(VALUE_LABEL_OUTSIDE),
        CategoryBarLabels::Auto => Some(VALUE_LABEL_AUTO),
    };
    match render_type {
        DataRenderType::RangeBars { labels, .. } => bar_placement(labels.placement),
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
            bar_placement(placement)
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

/// How many labels a series of `kind` with the strings `text` draws over
/// `categories` (the series' axis in order): one per category that has a
/// string, or for range bars one per string (a label per row, whatever
/// the position axis), any number (#107 R11).
pub(crate) fn value_label_count(
    kind: u32,
    text: &[String],
    categories: &[crate::config::Category],
) -> usize {
    if kind == VALUE_LABEL_KIND_RANGE {
        text.len()
    } else {
        categories.len().min(text.len())
    }
}

/// The value label uniform of one series: the atlas geometry, its kind and
/// its label count ([`value_label_count`]) at `scale` render pixels per
/// logical pixel. The labels' cells are [`value_label_cells`].
pub(crate) fn value_label_params(
    kind: u32,
    count: usize,
    atlas: &ValueLabelAtlas,
    font_px: f32,
    scale: f32,
) -> ValueLabelParamsGpu {
    let gap = match kind {
        VALUE_LABEL_KIND_BARS | VALUE_LABEL_KIND_RANGE => BAR_LABEL_GAP,
        VALUE_LABEL_KIND_BOXES => BOX_LABEL_GAP,
        _ => RADAR_LABEL_GAP,
    };
    let (color, stroke, padding) = selection_policy(scale);
    ValueLabelParamsGpu {
        atlas: atlas.geometry,
        shape: [kind, u32::try_from(count).unwrap_or(u32::MAX), 0, 0],
        metrics: [gap * scale, atlas.baseline_px, font_px, scale],
        selection_color: color,
        selection: [stroke, padding, 0.0, 0.0],
    }
}

/// Push the label cells of one series, the first `count` labels in
/// category (or, for range bars, row) order: the width of its string's
/// cell (zero for an empty string or a label its placement hides), its
/// placement and its drag offset (`offsets`) at `scale` render pixels per
/// logical pixel. [`create_value_label_data`] writes them into the cells'
/// mapping.
#[allow(clippy::too_many_arguments)]
pub(crate) fn value_label_cells(
    render_type: &DataRenderType,
    text: &[String],
    offsets: &[[f32; 2]],
    categories: &[crate::config::Category],
    count: usize,
    atlas: &ValueLabelAtlas,
    scale: f32,
    push: &mut dyn FnMut([f32; 4]),
) {
    for (index, text) in text.iter().take(count).enumerate() {
        let category = categories.get(index).map(|c| c.id.as_str());
        let placement = label_placement(render_type, category);
        let width = atlas.cells.get(index).map_or(0.0, |cell| cell.width);
        let cell = match placement {
            Some(placement) if !text.is_empty() => {
                let [dx, dy] = offsets.get(index).copied().unwrap_or([0.0; 2]);
                [width, placement as f32, dx * scale, dy * scale]
            }
            _ => [0.0; 4],
        };
        push(cell);
    }
}

/// Group 1 of the value label draw and selection pipelines: the label
/// uniform (binding 4), the atlas (5), its sampler (6) and the label cells
/// as read-only storage (7, `array<vec4<f32>>`, one per label). Bindings
/// 0..3 of group 1 are the bar module's `Style` and typed pick query.
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
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
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

/// One series' value label uniform and cells, charged as a uniform, and
/// the group 1 that binds them with the series' atlas.
#[derive(Clone)]
pub struct ValueLabelData {
    pub bind_group: wgpu::BindGroup,
    /// The uniform alone: the typed pick binds it into its own group 1.
    pub params: std::sync::Arc<wgpu::Buffer>,
    /// The label cells alone, which the typed pick binds beside it.
    pub cells: std::sync::Arc<wgpu::Buffer>,
    pub count: u32,
    pub charge: crate::gpu_memory::SharedCharge,
    /// Keeps the atlas texture's ledger charge alive with the bind group.
    pub atlas_charge: crate::gpu_memory::SharedCharge,
}

/// Upload `params` and the series' label cells and bind them with `atlas`
/// and `sampler` into `layout` ([`create_value_label_bind_group_layout`]).
///
/// The cells buffer holds `params.shape[1]` cells, sized by the labels:
/// `write` pushes them in order ([`value_label_cells`]) and they go
/// straight into its mapping at creation, nothing staged on the CPU; cells
/// it does not push read as zero (no label), pushes past the count are
/// dropped.
pub(crate) fn create_value_label_data(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    layout: &wgpu::BindGroupLayout,
    atlas: &ValueLabelAtlas,
    sampler: &wgpu::Sampler,
    params: &ValueLabelParamsGpu,
    write: impl FnOnce(&mut dyn FnMut([f32; 4])),
) -> crate::Result<ValueLabelData> {
    use crate::gpu_memory::{ChargeTally, charged_buffer, charged_buffer_init};
    const CELL: usize = std::mem::size_of::<[f32; 4]>();
    let tally = ChargeTally::new();
    let count = params.shape[1] as usize;
    let cell_bytes = (count.max(1) as u64).checked_mul(CELL as u64).ok_or(
        crate::FiggyError::GpuResourceLimit {
            resource: "value label cells",
            requested: count as u64,
            limit: u64::MAX / CELL as u64,
        },
    )?;
    let cells = charged_buffer(
        &tally,
        device,
        &wgpu::BufferDescriptor {
            label: Some("figgy value label cells"),
            size: cell_bytes,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: true,
        },
    );
    {
        let mut view = cells
            .slice(..)
            .get_mapped_range_mut()
            .expect("the value label cells are mapped at creation");
        let mut written = 0usize;
        write(&mut |cell| {
            if written < count {
                let at = written * CELL;
                view.slice(at..at + CELL)
                    .copy_from_slice(bytemuck::bytes_of(&cell));
                written += 1;
            }
        });
    }
    cells.unmap();
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
            wgpu::BindGroupEntry {
                binding: 7,
                resource: cells.as_entire_binding(),
            },
        ],
    });
    Ok(ValueLabelData {
        bind_group,
        params: std::sync::Arc::new(buffer),
        cells: std::sync::Arc::new(cells),
        count: params.shape[1],
        charge,
        atlas_charge: atlas.texture.shared_charge(),
    })
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
/// category column as `positions`, one label per row, a group of one; the
/// value column is the lane table's one entry. The radar entries read
/// nothing else.
pub(crate) fn radar_label_params(positions: u32, count: u32, scale: f32) -> CategoryBarParamsGpu {
    let mut params: CategoryBarParamsGpu = bytemuck::Zeroable::zeroed();
    params.positions = positions;
    params.count = count;
    params.members = 1;
    params.mode = CATEGORY_BAR_MODE_GROUPED;
    params.scale = scale;
    params
}
