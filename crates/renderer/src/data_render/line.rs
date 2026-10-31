//! Line and area series: pipelines, vertex counts and the line layers.

use super::*;

/// The assembled line module.
pub(crate) const LINE_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/sample_validity.wgsl",
    "data_render/common/project_3d.wgsl",
    "data_render/common/ramp.wgsl",
    "data_render/common/sketch_noise.wgsl",
    "data_render/common/arc_pair.wgsl",
    "data_render/common/line_stroke.wgsl",
    "data_render/common/line_step.wgsl",
    "data_render/common/area_piece.wgsl",
    "data_render/common/arc_walk.wgsl",
    "data_render/common/data_pick.wgsl",
    "data_render/line_columnar.wgsl",
    "data_render/parallel_columnar.wgsl",
);

/// Columnar line pipeline. slots: 0=x_a, 1=y_a, 2=x_b, 3=y_b (per-instance).
pub fn create_line_columnar_pipeline(
    device: &wgpu::Device,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let shader = ShaderModules::line(device);
    create_line_columnar_pipeline_with_sample_count(
        device,
        &shader,
        transform_bgl,
        style_bgl,
        target_format,
        1,
    )
}

pub(crate) fn create_line_columnar_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    create_line_columnar_pipeline_with_entries(
        device,
        shader,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_main",
        "fs_main",
        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        wgpu::PrimitiveTopology::TriangleStrip,
        None,
        "figgy line columnar pipeline",
    )
}

/// Strip vertices per instance (one straight segment) for the precise
/// `vs_main` line entry and the `vs_area` fill entry: one quad.
pub const LINE_PRECISE_VERTICES_PER_INSTANCE: u32 = 4;

/// Strip vertex count per instance for the sketch line pipeline:
/// `2 * (S + 1)` with the subdivision constant `S = 8` — must match
/// `SKETCH_SUBDIV` in `line_columnar.wgsl`.
pub const LINE_SKETCH_VERTICES_PER_INSTANCE: u32 = 18;

/// Strip vertices one line instance draws under `interpolation` in a
/// rectangular chart: every step piece is one strip of `per_piece` vertices
/// and consecutive pieces are joined by two duplicated vertices. Twin of
/// `strip_slot` in `line_columnar.wgsl`. A polar chart's arc pieces go
/// through the crate-private `line_step::strip_vertex_count` with its mode
/// word.
pub fn line_strip_vertex_count(
    per_piece: u32,
    interpolation: crate::data_config::LineInterpolation,
) -> u32 {
    crate::line_step::strip_vertex_count(per_piece, crate::line_step::mode_word(interpolation, 0))
}

/// The area-fill pipeline: `vs_area` / `fs_area` of the line shader, one
/// trapezoid per segment (per step piece) between the line and its baseline
/// or a second y column. Slots 0-3 are the line's; slots 4/5 carry the second
/// y column as `(hi, lo)` pairs with the same one-value shift (the y column
/// itself as inert filler for a baseline fill).
pub(crate) fn create_line_area_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy line area layout"),
            bind_group_layouts: &[Some(transform_bgl), Some(style_bgl)],
            immediate_size: 0,
        },
    );
    let column_stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;
    const ATTRS: [[wgpu::VertexAttribute; 1]; 6] = {
        let mut attrs = [[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 0,
            shader_location: 0,
        }]; 6];
        let mut i = 0;
        while i < 6 {
            attrs[i][0].shader_location = i as u32;
            i += 1;
        }
        attrs
    };
    let buffers: Vec<Option<wgpu::VertexBufferLayout<'_>>> = ATTRS
        .iter()
        .map(|attributes| {
            Some(wgpu::VertexBufferLayout {
                array_stride: column_stride,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes,
            })
        })
        .collect();
    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some("figgy line area pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_area"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: multisample_state(sample_count),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_area"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        },
    )
}

/// Entry-point-parameterized line pipeline builder. Styled variants share
/// the precise pipeline's six instance slots and differ in entry points,
/// blend state (constellation is additive), topology (star quads are a
/// TriangleList), and an optional third bind group (style textures) — the
/// renderer's style table supplies all of it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_line_columnar_pipeline_with_entries(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    fs_entry: &str,
    blend: wgpu::BlendState,
    topology: wgpu::PrimitiveTopology,
    texture_bgl: Option<&wgpu::BindGroupLayout>,
    label: &str,
) -> wgpu::RenderPipeline {
    let layouts = [Some(transform_bgl), Some(style_bgl), texture_bgl];
    let groups = if texture_bgl.is_some() { 3 } else { 2 };
    line_slot_pipeline(
        device,
        shader,
        &layouts[..groups],
        LINE_SLOTS,
        target_format,
        sample_count,
        vs_entry,
        fs_entry,
        blend,
        topology,
        label,
        None,
    )
}

/// The colour-mapped stroke of a streamline series: `vs_main_mapped` /
/// `fs_main_mapped` over the six line slots plus the value column in
/// slots 6/7 (eight, the WebGPU default limit), with the point value map
/// (SHADER_COMMON.md §5.2) as group 3. The shader reads no group 2; the
/// layout holds the value map's layout there too, bound to the renderer's
/// neutral value map, so no new layout is needed.
pub(crate) fn create_line_mapped_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    value_map_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    line_slot_pipeline(
        device,
        shader,
        &[
            Some(transform_bgl),
            Some(style_bgl),
            Some(value_map_bgl),
            Some(value_map_bgl),
        ],
        LINE_MAPPED_SLOTS,
        target_format,
        sample_count,
        "vs_main_mapped",
        "fs_main_mapped",
        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        wgpu::PrimitiveTopology::TriangleStrip,
        "figgy line mapped pipeline",
        None,
    )
}

/// Vertex slots of the line entries: x_a, y_a, x_b, y_b and the arc prefix
/// at A and B.
const LINE_SLOTS: usize = 6;
/// Vertex slots of the colour-mapped stroke: the line's and the colour
/// value at A and B; the ternary stroke's are the same with the third
/// component at A and B.
const LINE_MAPPED_SLOTS: usize = 8;

/// The stroke of a ternary series (`vs_ternary` / `fs_main`): the six line
/// slots plus the third component at slots 6/7, over the line's groups.
pub(crate) fn create_line_ternary_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    line_slot_pipeline(
        device,
        shader,
        &[Some(transform_bgl), Some(style_bgl)],
        LINE_MAPPED_SLOTS,
        target_format,
        sample_count,
        "vs_ternary",
        "fs_main",
        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        wgpu::PrimitiveTopology::TriangleStrip,
        "figgy line ternary pipeline",
        None,
    )
}

/// The stroke of a 3D chart's line series (`vs_3d` / `fs_main`, #132): the
/// six line slots plus the height at slots 6/7, over the line's groups and
/// the camera group as group 3, drawn into the panel composite with the
/// depth test; or its 1×1 ID pick (`vs_3d_pick` / `fs_3d_pick`) into the
/// integer pick target.
pub(crate) fn create_line_view3d_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    camera_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    pass: super::View3dPass,
) -> wgpu::RenderPipeline {
    let (vs, fs, label) = match pass {
        super::View3dPass::Pick => ("vs_3d_pick", "fs_3d_pick", "figgy line 3D pick pipeline"),
        _ => ("vs_3d", "fs_main", "figgy line 3D pipeline"),
    };
    line_slot_pipeline(
        device,
        shader,
        &[Some(transform_bgl), Some(style_bgl), None, Some(camera_bgl)],
        LINE_MAPPED_SLOTS,
        target_format,
        sample_count,
        vs,
        fs,
        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        wgpu::PrimitiveTopology::TriangleStrip,
        label,
        Some(pass),
    )
}

/// The attribute of each per-instance vertex slot the line entries read, in
/// slot order: the X / Y columns bound twice (the second pair starts one
/// logical value later, so instance i sees points [i] and [i+1] together),
/// the arc prefix bound twice with the same one-value shift (arc pairs,
/// `(low, high)` words of a fixed-point arc, SHADER_COMMON.md §5.6), and for
/// the colour-mapped stroke the value column bound twice like X / Y. Every
/// slot is 8 bytes per value.
const LINE_SLOT_ATTRIBUTES: [[wgpu::VertexAttribute; 1]; LINE_MAPPED_SLOTS] = {
    let mut attrs = [[wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    }]; LINE_MAPPED_SLOTS];
    let mut i = 0;
    while i < LINE_MAPPED_SLOTS {
        attrs[i][0].shader_location = i as u32;
        if i == 4 || i == 5 {
            attrs[i][0].format = wgpu::VertexFormat::Uint32x2;
        }
        i += 1;
    }
    attrs
};

/// A line pipeline over the first `slots` vertex slots of
/// [`LINE_SLOT_ATTRIBUTES`] and the bind group `layouts`.
#[allow(clippy::too_many_arguments)]
fn line_slot_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    slots: usize,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    vs_entry: &str,
    fs_entry: &str,
    blend: wgpu::BlendState,
    topology: wgpu::PrimitiveTopology,
    label: &str,
    three_d: Option<super::View3dPass>,
) -> wgpu::RenderPipeline {
    let (depth_stencil, color_target) = match three_d {
        Some(pass) => (pass.depth_stencil(), pass.color_target(target_format)),
        None => (
            None,
            wgpu::ColorTargetState {
                format: target_format,
                blend: Some(blend),
                write_mask: wgpu::ColorWrites::ALL,
            },
        ),
    };
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy line columnar layout"),
            bind_group_layouts: layouts,
            immediate_size: 0,
        },
    );
    const _: () = assert!(
        crate::data_render::line_arc::ARC_PAIR_BYTES == crate::data::COLUMN_VALUE_BYTES as u64
    );
    let column_stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;
    let buffers: [Option<wgpu::VertexBufferLayout<'_>>; LINE_MAPPED_SLOTS] =
        std::array::from_fn(|slot| {
            Some(wgpu::VertexBufferLayout {
                // Every slot is 8 bytes per value: (hi, lo) column pairs, and
                // the arc prefix's 4/5 are arc pairs.
                array_stride: column_stride,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &LINE_SLOT_ATTRIBUTES[slot],
            })
        });
    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vs_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &buffers[..slots],
            },
            primitive: wgpu::PrimitiveState {
                // Strip per instance for the line entries (4 vertices for
                // `vs_main`, 2·(S+1) for `vs_sketch`/`vs_ribbon`), TriangleList
                // for `vs_stars` quads — the caller picks.
                topology,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil,
            multisample: three_d.map_or_else(
                || multisample_state(sample_count),
                |pass| pass.multisample(sample_count),
            ),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fs_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(color_target)],
            }),
            multiview_mask: None,
            cache: None,
        },
    )
}

/// Bundle of handles for drawing one columnar line series. `pool_buffer` is
/// passed in separately so it can be shared across series.
pub struct ColumnLineLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub pool_buffer: &'a wgpu::Buffer,
    pub x: ColumnHandle,
    pub y: ColumnHandle,
    /// Cumulative arc-length prefix (px) for dash phase — `(buffer, length
    /// in bytes)`. `None` for solid lines: the X column is bound as inert
    /// filler instead. `Arc` because the buffer lives in the renderer's
    /// per-series cache while the layer is a per-frame view.
    pub arc: Option<(std::sync::Arc<wgpu::Buffer>, u64)>,
    /// Strip vertices per instance — must match `pipeline`'s vertex entry:
    /// 4 for the precise `vs_main`, [`LINE_SKETCH_VERTICES_PER_INSTANCE`]
    /// for the sketch `vs_sketch`, [`MILKYWAY_RIBBON_VERTICES`] for
    /// the milkyway ribbon — each per step piece, joined per
    /// [`line_strip_vertex_count`].
    pub verts_per_instance: u32,
    /// Style textures (group 2) when `pipeline`'s layout includes them —
    /// the milkyway/constellation PSF/LUT bind group. `None` for precise/sketch.
    pub texture_bg: Option<&'a wgpu::BindGroup>,
    /// Whether the line is a closed polygon (a radar series): one more
    /// segment, from the last point back to the first, drawn with the same
    /// pipeline (`draw_line_layer`).
    pub closed: bool,
    /// A colour-mapped stroke (a streamline series with a colour column):
    /// `pipeline` is the mapped line pipeline, the value column is bound as
    /// slots 6/7 and the [`LineValueMap`] groups as 2 and 3. `None` for every
    /// other line.
    pub value_map: Option<LineValueMap<'a>>,
    /// A ternary series' third component, or a 3D series' height (#132):
    /// `pipeline` is the ternary (`vs_ternary`) or 3D (`vs_3d`) line
    /// pipeline, the column bound as slots 6/7 like the X / Y columns.
    /// `None` for every other line.
    pub third: Option<ColumnHandle>,
    /// A 3D series' camera group, bound as group 3 (#132): `pipeline` is the
    /// 3D line pipeline, drawn into the panel composite with the depth test
    /// by `issue_series_surfaces`. `None` for every other line.
    pub view3d: Option<super::surface_mesh::View3dBinding>,
}

/// The colour value of a colour-mapped stroke (`vs_main_mapped`): the value
/// column, index-aligned with x and y, and the bind groups of the mapped
/// line pipeline's groups 2 (the renderer's neutral value map, which the
/// shader does not read) and 3 (the series' point value map).
#[derive(Clone, Copy)]
pub struct LineValueMap<'a> {
    pub column: ColumnHandle,
    pub neutral_bg: &'a wgpu::BindGroup,
    pub value_map_bg: &'a wgpu::BindGroup,
}

/// The area fill of one columnar line series: under the line to a baseline,
/// or between the line and a second series' y column (`base`), index-aligned
/// over the same x column. Drawn by `issue_series_fill` before every series
/// body, so no fill can cover another series' line.
pub struct ColumnAreaLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub pool_buffer: &'a wgpu::Buffer,
    pub x: ColumnHandle,
    pub y: ColumnHandle,
    /// Second y column for a fill between two lines; the y column itself for
    /// a baseline fill, bound as inert filler.
    pub base: ColumnHandle,
    /// Strip vertices per instance: [`LINE_PRECISE_VERTICES_PER_INSTANCE`] per
    /// step piece through [`line_strip_vertex_count`].
    pub verts_per_instance: u32,
    /// Whether the fill closes its polygon (a radar series on a full
    /// circle): one more trapezoid under the segment from the last point
    /// back to the first.
    pub closed: bool,
}

/// Issue one line-slot draw (the shared 6-slot binding scheme). Used for the
/// main line layer and the constellation star pass.
pub(crate) fn draw_line_layer(pass: &mut wgpu::RenderPass<'_>, l: &ColumnLineLayer<'_>) {
    // A colour column is index-aligned with the coordinates and bounds the
    // segments too.
    let count = l
        .value_map
        .map_or(usize::MAX, |map| map.column.len_values)
        .min(l.third.map_or(usize::MAX, |c| c.len_values))
        .min(l.x.len_values)
        .min(l.y.len_values) as u32;
    if count < 2 {
        return;
    }
    pass.set_pipeline(l.pipeline);
    pass.set_bind_group(0, l.transform_bg, &[]);
    pass.set_bind_group(1, l.style_bg, &[]);
    if let Some(tex) = l.texture_bg {
        pass.set_bind_group(2, tex, &[]);
    }
    if let Some(map) = l.value_map {
        pass.set_bind_group(2, map.neutral_bg, &[]);
        pass.set_bind_group(3, map.value_map_bg, &[]);
        let value = map.column.byte_range();
        let value_next = (value.start + crate::data::COLUMN_VALUE_BYTES as u64)..value.end;
        pass.set_vertex_buffer(6, l.pool_buffer.slice(value));
        pass.set_vertex_buffer(7, l.pool_buffer.slice(value_next));
    }
    if let Some(c) = l.third {
        let c = c.byte_range();
        let c_next = (c.start + crate::data::COLUMN_VALUE_BYTES as u64)..c.end;
        pass.set_vertex_buffer(6, l.pool_buffer.slice(c));
        pass.set_vertex_buffer(7, l.pool_buffer.slice(c_next));
    }
    if let Some(view3d) = &l.view3d {
        pass.set_bind_group(3, &view3d.bind_group, &[]);
    }
    let x_full = l.x.byte_range();
    let y_full = l.y.byte_range();
    let x_next = (x_full.start + crate::data::COLUMN_VALUE_BYTES as u64)..x_full.end;
    let y_next = (y_full.start + crate::data::COLUMN_VALUE_BYTES as u64)..y_full.end;
    pass.set_vertex_buffer(0, l.pool_buffer.slice(x_full.clone()));
    pass.set_vertex_buffer(1, l.pool_buffer.slice(y_full.clone()));
    pass.set_vertex_buffer(2, l.pool_buffer.slice(x_next.clone()));
    pass.set_vertex_buffer(3, l.pool_buffer.slice(y_next));
    // Arc-length prefix (dash phase / constellation arc); solid precise lines
    // reuse the X column as filler (read by the VS, ignored by the FS).
    match l.arc.as_ref() {
        Some((buf, len_bytes)) => {
            let pair = crate::data_render::line_arc::ARC_PAIR_BYTES;
            pass.set_vertex_buffer(4, buf.slice(0..*len_bytes));
            pass.set_vertex_buffer(5, buf.slice(pair..*len_bytes));
        }
        None => {
            pass.set_vertex_buffer(4, l.pool_buffer.slice(x_full.clone()));
            pass.set_vertex_buffer(5, l.pool_buffer.slice(x_next));
        }
    }
    // Per-instance vertex count decided where the pipeline variant was
    // picked: 4 (precise) / 18 (sketch, constellation ribbon) per step piece,
    // through `line_strip_vertex_count`.
    pass.draw(0..l.verts_per_instance, 0..(count - 1));
    if l.closed {
        // The closing segment: the same instance with A at the last point
        // and B at the first. A closed line is solid (a radar outline), so
        // its arc slots hold the inert filler; a bound prefix gives the
        // segment the last point's arc at both ends.
        let last = u64::from(count - 1);
        let value = crate::data::COLUMN_VALUE_BYTES as u64;
        pass.set_vertex_buffer(
            0,
            l.pool_buffer
                .slice((x_full.start + last * value)..x_full.end),
        );
        pass.set_vertex_buffer(
            1,
            l.pool_buffer
                .slice((y_full.start + last * value)..y_full.end),
        );
        pass.set_vertex_buffer(2, l.pool_buffer.slice(x_full.clone()));
        pass.set_vertex_buffer(3, l.pool_buffer.slice(y_full.clone()));
        if let Some((buf, len_bytes)) = l.arc.as_ref() {
            let at = last * crate::data_render::line_arc::ARC_PAIR_BYTES;
            pass.set_vertex_buffer(4, buf.slice(at..*len_bytes));
            pass.set_vertex_buffer(5, buf.slice(at..*len_bytes));
        }
        pass.draw(0..l.verts_per_instance, 0..1);
    }
}

/// Issue the closing trapezoid of a closed area fill: the area instance with
/// A at the last point and B at the first (`issue_series_fill` draws the
/// others). `count` is the drawn point count.
pub(super) fn draw_area_closing(
    pass: &mut wgpu::RenderPass<'_>,
    a: &ColumnAreaLayer<'_>,
    count: u32,
) {
    let last = u64::from(count.saturating_sub(1));
    let value = crate::data::COLUMN_VALUE_BYTES as u64;
    let at_last = |range: std::ops::Range<u64>| (range.start + last * value)..range.end;
    let (x, y, base) = (a.x.byte_range(), a.y.byte_range(), a.base.byte_range());
    pass.set_vertex_buffer(0, a.pool_buffer.slice(at_last(x.clone())));
    pass.set_vertex_buffer(1, a.pool_buffer.slice(at_last(y.clone())));
    pass.set_vertex_buffer(2, a.pool_buffer.slice(x));
    pass.set_vertex_buffer(3, a.pool_buffer.slice(y));
    pass.set_vertex_buffer(4, a.pool_buffer.slice(at_last(base.clone())));
    pass.set_vertex_buffer(5, a.pool_buffer.slice(base));
    pass.draw(0..a.verts_per_instance, 0..1);
}

// ───────────── parallel coordinates (#141) ─────────────

/// What a parallel coordinates series draws with beside `Transform` and
/// `Style`: its row and axis counts and its colour column's lane. The axis
/// lanes and ranges are the group's tables, sized by the axis count, so no
/// field here bounds the number of axes (#107 R3).
///
/// GPU twin: `parallel_columnar.wgsl::ParallelParams`, 16 B.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParallelParamsGpu {
    /// Rows drawn: the shortest axis column (and colour column).
    pub rows: u32,
    /// Axes: the lane table's entries. At least two.
    pub axes: u32,
    /// The colour column's f32 lane base when [`PARALLEL_COLOR`] is set.
    pub color_lane: u32,
    pub flags: u32,
}

const _: () = assert!(std::mem::size_of::<ParallelParamsGpu>() == 16);

/// `ParallelParamsGpu::flags`: a row's colour is its colour column's value
/// through the colourbar ramp.
pub const PARALLEL_COLOR: u32 = 1;

/// One parallel axis's range as the shader places a value on it: `lo` is
/// the axis minimum (`log10` of it on a logarithmic axis) as a `(hi, lo)`
/// pair, `span_inv` the reciprocal of the span in the same space, `mode`
/// the [`PARALLEL_AXIS_INVERTED`] and [`PARALLEL_AXIS_LOG`] bits.
///
/// GPU twin: `parallel_columnar.wgsl::ParallelAxis`, 16 B.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ParallelAxisGpu {
    pub lo: [f32; 2],
    pub span_inv: f32,
    pub mode: u32,
}

const _: () = assert!(std::mem::size_of::<ParallelAxisGpu>() == 16);

/// `ParallelAxisGpu::mode`: values grow downward.
pub const PARALLEL_AXIS_INVERTED: u32 = 1;
/// `ParallelAxisGpu::mode`: values are placed in `log10`.
pub const PARALLEL_AXIS_LOG: u32 = 2;

impl ParallelAxisGpu {
    /// The GPU range of a model axis (`ParallelAxis`), which validated its
    /// range as finite with a smaller minimum (positive when logarithmic).
    pub fn from_axis(axis: &crate::parallel::ParallelAxis) -> Self {
        let log = axis.scale == crate::config::AxisScale::Logarithmic;
        let (min, max) = if log {
            (axis.min.log10(), axis.max.log10())
        } else {
            (axis.min, axis.max)
        };
        let (hi, lo) = crate::data::split_f64_to_f32_pair(min);
        let span = max - min;
        Self {
            lo: [hi, lo],
            span_inv: if span.is_finite() && span != 0.0 {
                (1.0 / span) as f32
            } else {
                0.0
            },
            mode: (if axis.inverted {
                PARALLEL_AXIS_INVERTED
            } else {
                0
            }) | (if log { PARALLEL_AXIS_LOG } else { 0 }),
        }
    }
}

/// Bind group layout of a parallel coordinates series' own data (group 3
/// of the line module's parallel entries): the point value map the row
/// colours read (bindings 0 and 1, the line module's `line_value_map` /
/// `line_value_stops`), the column pool as read-only storage (binding 2),
/// its params (binding 3), its axis lane table (binding 4, `array<u32>`)
/// and its axis range table (binding 5, `array<ParallelAxis>`). The pick
/// (`pick_parallel`) binds the same group.
pub fn create_parallel_data_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let visibility = wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE;
    let uniform = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    crate::gpu_compile::bind_group_layout(
        device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy parallel data bgl"),
            entries: &[
                uniform(0),
                storage(1),
                storage(2),
                uniform(3),
                storage(4),
                storage(5),
            ],
        },
    )
}

/// What a parallel coordinates series draws with besides its style: its
/// group 3, the charge of its params and tables, and the counts the draw
/// and the pick walk.
#[derive(Clone)]
pub struct ParallelData {
    pub bind_group: wgpu::BindGroup,
    pub charge: crate::gpu_memory::SharedCharge,
    /// Rows drawn (`ParallelParamsGpu::rows`).
    pub rows: u32,
    /// Axes (`ParallelParamsGpu::axes`).
    pub axes: u32,
}

impl ParallelData {
    /// Segments one row draws.
    pub fn segments(&self) -> u32 {
        self.axes.max(2) - 1
    }

    /// Instances the draw issues: every row's segments.
    pub fn instances(&self) -> u32 {
        self.rows.saturating_mul(self.segments())
    }
}

/// Bind the value map, the pool, `params` and the series' axis tables into
/// the parallel data layout.
///
/// `write` pushes one `(lane, range)` per axis in order; they go straight
/// into the mappings of the two tables at creation, so nothing is staged on
/// the CPU and the tables are sized by the axis count. Pushes past
/// `params.axes` are dropped; an axis `write` does not push reads as lane 0
/// over an empty range. An error from `write` is returned as is. The
/// buffers are owned by the bind group and charged as a uniform.
#[allow(clippy::too_many_arguments)]
pub fn create_parallel_data_bind_group(
    device: &wgpu::Device,
    ledger: &std::sync::Arc<crate::gpu_memory::GpuLedger>,
    layout: &wgpu::BindGroupLayout,
    pool_buffer: &wgpu::Buffer,
    value_map: &PointValueMapBuffers,
    params: &ParallelParamsGpu,
    write: impl FnOnce(&mut dyn FnMut(u32, ParallelAxisGpu)) -> crate::Result<()>,
) -> crate::Result<ParallelData> {
    use crate::gpu_memory::{ChargeTally, charged_buffer, charged_buffer_init};
    const LANE: usize = std::mem::size_of::<u32>();
    const RANGE: usize = std::mem::size_of::<ParallelAxisGpu>();
    let axes = params.axes as usize;
    let tally = ChargeTally::new();
    let params_buf = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy parallel params"),
            contents: bytemuck::bytes_of(params),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    );
    let table = |label, entry: usize| -> crate::Result<wgpu::Buffer> {
        let bytes = (axes.max(1) as u64).checked_mul(entry as u64).ok_or(
            crate::FiggyError::GpuResourceLimit {
                resource: "parallel axis table",
                requested: axes as u64,
                limit: u64::MAX / entry as u64,
            },
        )?;
        Ok(charged_buffer(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: true,
            },
        ))
    };
    let lane_buf = table("figgy parallel lane table", LANE)?;
    let range_buf = table("figgy parallel axis range table", RANGE)?;
    let written = {
        let mut lanes = lane_buf
            .slice(..)
            .get_mapped_range_mut()
            .expect("the lane table is mapped at creation");
        let mut ranges = range_buf
            .slice(..)
            .get_mapped_range_mut()
            .expect("the axis range table is mapped at creation");
        let mut written = 0usize;
        let result = write(&mut |lane, range| {
            if written < axes {
                lanes
                    .slice(written * LANE..(written + 1) * LANE)
                    .copy_from_slice(bytemuck::bytes_of(&lane));
                ranges
                    .slice(written * RANGE..(written + 1) * RANGE)
                    .copy_from_slice(bytemuck::bytes_of(&range));
                written += 1;
            }
        });
        result.map(|()| written)
    };
    lane_buf.unmap();
    range_buf.unmap();
    written?;
    let charge = crate::gpu_memory::shared_charge(
        tally,
        ledger,
        crate::gpu_memory::GpuResourceKind::Uniform,
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy parallel data bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: value_map.params.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: value_map.stops.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: pool_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: params_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: lane_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: range_buf.as_entire_binding(),
            },
        ],
    });
    Ok(ParallelData {
        bind_group,
        charge,
        rows: params.rows,
        axes: params.axes,
    })
}

/// The rows of a parallel coordinates series (`vs_parallel` /
/// `fs_parallel`), or the picked row's redraw in the selection's style
/// (`vs_parallel_selected` / `fs_parallel`): groups 0 transform, 1 style,
/// 3 the series' parallel data; no vertex buffers, one quad per segment
/// instance.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_line_parallel_pipeline_with_sample_count(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    parallel_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
    selection: bool,
) -> wgpu::RenderPipeline {
    let (vs, label) = if selection {
        (
            "vs_parallel_selected",
            "figgy selected parallel row pipeline",
        )
    } else {
        ("vs_parallel", "figgy line parallel pipeline")
    };
    line_slot_pipeline(
        device,
        shader,
        &[
            Some(transform_bgl),
            Some(style_bgl),
            None,
            Some(parallel_bgl),
        ],
        0,
        target_format,
        sample_count,
        vs,
        "fs_parallel",
        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        wgpu::PrimitiveTopology::TriangleStrip,
        label,
        None,
    )
}

/// The picked row of a parallel coordinates series, redrawn over every
/// series body in the selection's colour and width.
pub struct ParallelSelection<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub style_bg: wgpu::BindGroup,
    pub row: u32,
}

/// Bundle of handles for drawing one parallel coordinates series.
pub struct ColumnParallelLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub data: ParallelData,
    pub selected: Option<ParallelSelection<'a>>,
}

/// Issue the rows of one parallel coordinates series: one quad per
/// segment instance over the series' group 3.
pub(crate) fn draw_parallel_layer(pass: &mut wgpu::RenderPass<'_>, l: &ColumnParallelLayer<'_>) {
    let instances = l.data.instances();
    if instances == 0 {
        return;
    }
    pass.set_pipeline(l.pipeline);
    pass.set_bind_group(0, l.transform_bg, &[]);
    pass.set_bind_group(1, l.style_bg, &[]);
    pass.set_bind_group(3, &l.data.bind_group, &[]);
    pass.draw(0..LINE_PRECISE_VERTICES_PER_INSTANCE, 0..instances);
}

/// Issue the picked row's segments of one parallel coordinates series in
/// the selection's style: the row's instances only.
pub(crate) fn draw_parallel_selection(
    pass: &mut wgpu::RenderPass<'_>,
    l: &ColumnParallelLayer<'_>,
) {
    let Some(selected) = &l.selected else {
        return;
    };
    if selected.row >= l.data.rows {
        return;
    }
    let segments = l.data.segments();
    let first = selected.row * segments;
    pass.set_pipeline(selected.pipeline);
    pass.set_bind_group(0, l.transform_bg, &[]);
    pass.set_bind_group(1, &selected.style_bg, &[]);
    pass.set_bind_group(3, &l.data.bind_group, &[]);
    pass.draw(
        0..LINE_PRECISE_VERTICES_PER_INSTANCE,
        first..first + segments,
    );
}

#[cfg(test)]
mod arc_walk_tests {
    /// The star pass's slot search moved, unchanged, into the shared
    /// `common/arc_walk.wgsl`: the line module splices it once, before the
    /// line shader, and the line shader declares no group-3 binding and no
    /// search of its own.
    #[test]
    fn star_pass_walks_the_shared_arc_slots() {
        let walk = include_str!("common/arc_walk.wgsl");
        let line = include_str!("line_columnar.wgsl");
        let module = super::LINE_SHADER;
        assert_eq!(module.source.matches(walk).count(), 1);
        let at = |name: &str| module.fragments.iter().position(|f| f.ends_with(name));
        assert!(at("common/arc_walk.wgsl") < at("line_columnar.wgsl"));
        assert!(walk.contains("fn arc_walk_segment(n: u32, arc_here: vec2<u32>) -> u32"));
        assert!(line.contains("let lo = arc_walk_segment(n, arc_here);"));
        assert!(!line.contains("var<storage, read> star_arc"));
        assert!(!line.contains("loop {\n        if (hi - lo <= 1u)"));
    }
}
