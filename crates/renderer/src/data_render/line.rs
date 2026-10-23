//! Line and area series: pipelines, vertex counts and the line layers.

use super::*;

/// The assembled line module.
pub(crate) const LINE_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/transform.wgsl",
    "data_render/common/style.wgsl",
    "data_render/common/projection.wgsl",
    "data_render/common/sample_validity.wgsl",
    "data_render/common/sketch_noise.wgsl",
    "data_render/common/line_stroke.wgsl",
    "data_render/common/line_step.wgsl",
    "data_render/line_columnar.wgsl",
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
    let mut bgls: Vec<Option<&wgpu::BindGroupLayout>> = vec![Some(transform_bgl), Some(style_bgl)];
    if let Some(t) = texture_bgl {
        bgls.push(Some(t));
    }
    let layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy line columnar layout"),
            bind_group_layouts: &bgls,
            immediate_size: 0,
        },
    );

    let f32_stride = std::mem::size_of::<f32>() as wgpu::BufferAddress;
    let column_stride = crate::data::COLUMN_VALUE_BYTES as wgpu::BufferAddress;

    crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some(vs_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                // 4 per-instance f32 slots: x_a, y_a, x_b, y_b. The same X/Y
                // columns are bound twice; the second pair starts one logical
                // value later, so instance i sees points [i] and [i+1] together.
                // Each instance emits a 4-vertex quad strip for one segment.
                buffers: &[
                    // slot 0: x_a (X column from offset 0)
                    Some(wgpu::VertexBufferLayout {
                        array_stride: column_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        }],
                    }),
                    // slot 1: y_a
                    Some(wgpu::VertexBufferLayout {
                        array_stride: column_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 1,
                        }],
                    }),
                    // slot 2: x_b (X column from the next value)
                    Some(wgpu::VertexBufferLayout {
                        array_stride: column_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 2,
                        }],
                    }),
                    // slot 3: y_b
                    Some(wgpu::VertexBufferLayout {
                        array_stride: column_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 3,
                        }],
                    }),
                    // slots 4/5: cumulative arc length (px) at A and B — the
                    // same prefix buffer bound twice with a one-f32 shift, like
                    // x/y. Solid lines bind the X column here as inert filler
                    // (the fragment stage ignores it when dash_len == 0).
                    Some(wgpu::VertexBufferLayout {
                        array_stride: f32_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: 0,
                            shader_location: 4,
                        }],
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: f32_stride,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32,
                            offset: 0,
                            shader_location: 5,
                        }],
                    }),
                ],
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
            depth_stencil: None,
            multisample: multisample_state(sample_count),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fs_entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
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
}

/// Issue one line-slot draw (the shared 6-slot binding scheme). Used for the
/// main line layer and the constellation star pass.
pub(super) fn draw_line_layer(pass: &mut wgpu::RenderPass<'_>, l: &ColumnLineLayer<'_>) {
    let count = l.x.len_values.min(l.y.len_values) as u32;
    if count < 2 {
        return;
    }
    pass.set_pipeline(l.pipeline);
    pass.set_bind_group(0, l.transform_bg, &[]);
    pass.set_bind_group(1, l.style_bg, &[]);
    if let Some(tex) = l.texture_bg {
        pass.set_bind_group(2, tex, &[]);
    }
    let x_full = l.x.byte_range();
    let y_full = l.y.byte_range();
    let x_next = (x_full.start + crate::data::COLUMN_VALUE_BYTES as u64)..x_full.end;
    let y_next = (y_full.start + crate::data::COLUMN_VALUE_BYTES as u64)..y_full.end;
    let x_f32_shift = (x_full.start + 4)..x_full.end;
    pass.set_vertex_buffer(0, l.pool_buffer.slice(x_full.clone()));
    pass.set_vertex_buffer(1, l.pool_buffer.slice(y_full));
    pass.set_vertex_buffer(2, l.pool_buffer.slice(x_next));
    pass.set_vertex_buffer(3, l.pool_buffer.slice(y_next));
    // Arc-length prefix (dash phase / constellation arc); solid precise lines
    // reuse the X column as filler (read by the VS, ignored by the FS).
    match l.arc.as_ref() {
        Some((buf, len_bytes)) => {
            pass.set_vertex_buffer(4, buf.slice(0..*len_bytes));
            pass.set_vertex_buffer(5, buf.slice(4..*len_bytes));
        }
        None => {
            pass.set_vertex_buffer(4, l.pool_buffer.slice(x_full));
            pass.set_vertex_buffer(5, l.pool_buffer.slice(x_f32_shift));
        }
    }
    // Per-instance vertex count decided where the pipeline variant was
    // picked: 4 (precise) / 18 (sketch, constellation ribbon) per step piece,
    // through `line_strip_vertex_count`.
    pass.draw(0..l.verts_per_instance, 0..(count - 1));
}
