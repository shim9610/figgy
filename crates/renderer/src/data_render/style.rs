//! Primitive styles on the GPU: PrimitiveStyle and its uniform, the shape
//! ids, and the per-point style maps of scatter, error-bar and bar series.

use super::*;

/// Per-primitive style uniform. 80 bytes, 16-byte aligned.
///
/// One struct serves all three primitive shaders; each reads its own fields
/// and ignores the rest. Field order mirrors WGSL `Style` byte-for-byte.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PrimitiveStyle {
    pub color_premul: [f32; 4], // offset 0
    /// Line / errorbar stem thickness in pixels.
    pub line_width_px: f32, // offset 16
    /// Scatter point radius in pixels.
    pub point_radius_px: f32, // offset 20
    /// Errorbar cap half-length in pixels.
    pub cap_half_px: f32, // offset 24
    /// Errorbar cap stroke thickness in pixels.
    pub cap_width_px: f32, // offset 28
    /// Stable GPU marker-shape code — see [`shape_id`].
    pub shape_id: u32, // offset 32
    /// Number of valid scalars in `dash`; 0 = solid. Bars draw no dashes: a
    /// histogram selection outline's `Style` holds the selected bin's global
    /// index here instead (see `from_bar`).
    pub dash_len: u32, // offset 36
    /// Per-series decorrelation salt (FNV-1a of `series_id`, written by
    /// `Renderer::create_style_for_series*`). Sketch/constellation shader
    /// entries XOR it into their hash seeds so series with identical
    /// sampling don't share star/wobble patterns; precise entries ignore it.
    pub series_salt: u32, // offset 40
    /// Primitive-specific feature bits. Errorbar uses bit 0 for Y and bit 1
    /// for X; other primitive shaders ignore this field.
    pub primitive_flags: u32, // offset 44
    /// Up to 8 sequential `[on, off, ...]` pixel lengths: `dash[0]` first,
    /// then `dash[1]`.
    pub dash: [[f32; 4]; 2], // offset 48 → 80 byte
}

const _: () = assert!(std::mem::size_of::<PrimitiveStyle>() == 80);

pub(crate) const ERRORBAR_HAS_Y: u32 = 1 << 0;

pub(crate) const ERRORBAR_HAS_X: u32 = 1 << 1;

/// Renderer policy for histogram width ratios. Invalid host values inherit the
/// full-bin default instead of poisoning bar geometry with NaN.
pub(crate) fn sanitize_bar_width_ratio(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

impl PrimitiveStyle {
    /// Convert a straight-RGBA `Color` to the premultiplied form expected by
    /// the `PREMULTIPLIED_ALPHA_BLENDING` pipeline.
    pub fn from_color(c: Color) -> Self {
        Self::from_color_with_width(c, 1.0)
    }

    /// `line_width_px` is the pixel thickness for line series. Every other
    /// option gets a neutral default: 4 px point radius, 3 px cap half-length,
    /// 1 px cap stroke, filled circle, solid line.
    pub fn from_color_with_width(c: Color, line_width_px: f32) -> Self {
        let a = c.a.clamp(0.0, 1.0);
        Self {
            color_premul: [c.r * a, c.g * a, c.b * a, a],
            line_width_px,
            point_radius_px: 4.0,
            cap_half_px: 3.0,
            cap_width_px: 1.0,
            shape_id: shape_id(&ScatterShape::CircleFilled),
            dash_len: 0,
            series_salt: 0,
            primitive_flags: 0,
            dash: [[0.0; 4]; 2],
        }
    }

    /// A bar's style — the `Style` reinterpretation documented in
    /// **SHADER_COMMON.md**'s bar `Style` reinterpretation table, which is the
    /// SSoT for this mapping. Change
    /// one without the other and a bar's colour, border, or baseline goes
    /// silently wrong.
    ///
    /// `scale` multiplies the pixel dimensions for high-DPI export, exactly as
    /// the other `create_style_for_series_scaled` paths do.
    ///
    /// A selected bin's outline starts from this style and then reinterprets
    /// three lanes (`selection_prepare.rs`, `vs_bar_selection*`):
    /// `color_premul` holds the highlight colour, `line_width_px` the outline
    /// width (unscaled) and `dash_len` the bin's global index, the key the
    /// mapped entry resolves the bin's overrides by.
    pub fn from_bar(bar: &crate::data_config::DataBarStyleConfig, scale: f32) -> Self {
        let crate::data_config::DataBarStyleConfig {
            fill_color,
            border_color,
            border_width,
            baseline,
            gap_px,
            width_ratio,
            orientation,
            // Per-bin overrides go to the bar style map, which the mapped
            // draw and selection entries resolve on the GPU.
            bar_style_overrides: _,
        } = bar;
        let premul = |c: Color| {
            let a = c.a.clamp(0.0, 1.0);
            [c.r * a, c.g * a, c.b * a, a]
        };
        // The baseline crosses as the pool's (hi, lo) f32 pair so a large
        // absolute base keeps its small deltas — a single f32 would not.
        let (base_hi, base_lo) = crate::data::split_f64_to_f32_pair(*baseline);
        Self {
            color_premul: premul(*fill_color),
            line_width_px: border_width.max(0.0) * scale,
            point_radius_px: 0.0,
            cap_half_px: gap_px.max(0.0) * scale,
            cap_width_px: sanitize_bar_width_ratio(*width_ratio),
            shape_id: match orientation {
                crate::data_config::BarOrientation::Vertical => 0,
                crate::data_config::BarOrientation::Horizontal => 1,
            },
            dash_len: 0,
            series_salt: 0,
            primitive_flags: 0,
            dash: [premul(*border_color), [base_hi, base_lo, 0.0, 0.0]],
        }
    }

    pub(crate) fn pack_dash_pattern(&mut self, pattern: &[f32], scale: f32) {
        let capacity: usize = self.dash.iter().map(|lane| lane.len()).sum();
        self.dash.iter_mut().flatten().for_each(|slot| *slot = 0.0);
        for (slot, length) in self.dash.iter_mut().flatten().zip(pattern.iter()) {
            *slot = *length * scale;
        }
        self.dash_len = pattern.len().min(capacity) as u32;
    }
}

/// Map a [`ScatterShape`] to the stable `Style.shape_id` uniform value.
pub fn shape_id(shape: &ScatterShape) -> u32 {
    match shape {
        ScatterShape::Circle => 0,
        ScatterShape::Square => 1,
        ScatterShape::Triangle => 2,
        ScatterShape::Diamond => 3,
        ScatterShape::Cross => 4,
        ScatterShape::CircleFilled => 5,
        ScatterShape::SquareFilled => 6,
        ScatterShape::TriangleFilled => 7,
        ScatterShape::DiamondFilled => 8,
        ScatterShape::TriangleDown => 9,
        ScatterShape::TriangleLeft => 10,
        ScatterShape::TriangleRight => 11,
        ScatterShape::Plus => 12,
        ScatterShape::Pentagon => 13,
        ScatterShape::Hexagon => 14,
        ScatterShape::Octagon => 15,
        ScatterShape::Star => 16,
        ScatterShape::TriangleDownFilled => 17,
        ScatterShape::TriangleLeftFilled => 18,
        ScatterShape::TriangleRightFilled => 19,
        ScatterShape::PlusFilled => 20,
        ScatterShape::CrossFilled => 21,
        ScatterShape::PentagonFilled => 22,
        ScatterShape::HexagonFilled => 23,
        ScatterShape::OctagonFilled => 24,
        ScatterShape::StarFilled => 25,
    }
}

/// Bind-group layout for the style uniform. `VERTEX_FRAGMENT` because the
/// line vertex shader reads `line_width_px`.
pub fn create_style_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy primitive style bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
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

pub fn create_style_uniform_buffer(device: &wgpu::Device, style: &PrimitiveStyle) -> wgpu::Buffer {
    // gpu-alloc: caller
    // ChartStyle owns one aggregate, budget-checked charge.
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy primitive style uniform"),
        contents: bytemuck::bytes_of(style),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

pub fn create_style_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy primitive style bg"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}

pub fn update_style(queue: &wgpu::Queue, buffer: &wgpu::Buffer, style: &PrimitiveStyle) {
    queue.write_buffer(buffer, 0, bytemuck::bytes_of(style));
}

/// `ScatterStyleSlotGpu::meta[2]` / `ScatterStyleOverrideGpu::meta[2]` bit:
/// the slot sets the colour. The scatter shader's `STYLE_MASK_COLOR`.
pub(crate) const STYLE_MASK_COLOR: u32 = 1;
/// The slot sets the radius (`STYLE_MASK_RADIUS`; the picker's
/// `PICK_STYLE_MASK_RADIUS`).
pub(crate) const STYLE_MASK_RADIUS: u32 = 2;
/// The slot sets the shape (`STYLE_MASK_SHAPE`; the picker's
/// `PICK_STYLE_MASK_SHAPE`).
pub(crate) const STYLE_MASK_SHAPE: u32 = 4;

/// GPU kinds of `model::Material`, in variant order. The WGSL constants of
/// `common/material.wgsl` (SHADER_COMMON.md §5.9).
pub(crate) const MATERIAL_FLAT: u32 = 0;
pub(crate) const MATERIAL_MATTE: u32 = 1;
pub(crate) const MATERIAL_CERAMIC: u32 = 2;
pub(crate) const MATERIAL_BRUSHED_METAL: u32 = 3;
pub(crate) const MATERIAL_PAPER: u32 = 4;
pub(crate) const MATERIAL_WOOD: u32 = 5;
pub(crate) const MATERIAL_SATIN_METAL: u32 = 6;
pub(crate) const MATERIAL_TOON: u32 = 7;
pub(crate) const MATERIAL_ENAMEL: u32 = 8;
pub(crate) const MATERIAL_HATCH: u32 = 9;
pub(crate) const MATERIAL_PEARL: u32 = 10;

/// The roughness a bar or a box is shaded with: `RadialStyle`'s default.
/// Bar and box styles have no roughness of their own.
pub(crate) const FLAT_BODY_ROUGHNESS: f32 = 0.35;

/// Per-instance material of a bar, a box or a sector. 32 bytes; mirrors
/// WGSL `Material` of `common/material.wgsl` (SHADER_COMMON.md §5.9).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct MaterialGpu {
    /// Kind, roughness, texture strength, texture scale.
    pub(crate) params: [f32; 4],
    /// Gloss, seed, texture angle (radians), emphasis brightness.
    pub(crate) finish: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<MaterialGpu>() == 32);

impl MaterialGpu {
    pub(crate) const EMPTY: MaterialGpu = MaterialGpu {
        params: [0.0; 4],
        finish: [0.0; 4],
    };

    /// Pack a material and its parameters. `angle` is in radians;
    /// `emphasis` is the hover or selection brightness, added after shading.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pack(
        material: model::Material,
        roughness: f32,
        strength: f32,
        scale: f32,
        gloss: f32,
        seed: f32,
        angle: f32,
        emphasis: f32,
    ) -> Self {
        use model::Material;
        let kind = match material {
            Material::Flat => MATERIAL_FLAT,
            Material::Matte => MATERIAL_MATTE,
            Material::Ceramic => MATERIAL_CERAMIC,
            Material::BrushedMetal => MATERIAL_BRUSHED_METAL,
            Material::Paper => MATERIAL_PAPER,
            Material::Wood => MATERIAL_WOOD,
            Material::SatinMetal => MATERIAL_SATIN_METAL,
            Material::Toon => MATERIAL_TOON,
            Material::Enamel => MATERIAL_ENAMEL,
            Material::Hatch => MATERIAL_HATCH,
            Material::Pearl => MATERIAL_PEARL,
        };
        Self {
            params: [kind as f32, roughness, strength, scale],
            finish: [gloss, seed, angle, emphasis],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScatterStyleSlotGpu {
    pub color_premul: [f32; 4],
    /// `(radius_px, shape_id as f32, mask_bits as f32, 0)`.
    pub meta: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<ScatterStyleSlotGpu>() == 32);

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScatterStyleOverrideGpu {
    pub point_index: u32,
    pub _pad: [u32; 3],
    pub color_premul: [f32; 4],
    /// `(radius_px, shape_id as f32, mask_bits as f32, 0)`.
    pub meta: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<ScatterStyleOverrideGpu>() == 48);

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ScatterStyleMapMeta {
    pub style_count: u32,
    pub override_count: u32,
    pub has_index: u32,
    pub _pad: u32,
}

const _: () = assert!(std::mem::size_of::<ScatterStyleMapMeta>() == 16);

/// GPU-side per-point style map. The bind group keeps all backing buffers alive.
///
/// Layout (group 2 in the mapped precise scatter/errorbar pipelines). Bindings
/// 0..4 are already used by the constellation scatter entries in the same WGSL
/// module, so mapped precise entries use bindings 5..7.
/// - binding 5: [`ScatterStyleSlotGpu`] array.
/// - binding 6: [`ScatterStyleOverrideGpu`] array.
/// - binding 7: [`ScatterStyleMapMeta`].
pub struct ScatterStyleMap {
    pub bind_group: wgpu::BindGroup,
    pub has_index: bool,
    pub(super) style_buf: wgpu::Buffer,
    pub(super) override_buf: wgpu::Buffer,
    pub(super) meta: ScatterStyleMapMeta,
}

impl ScatterStyleMap {
    pub(crate) fn stream_bind_group(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        global_base: u32,
        tally: &crate::gpu_memory::ChargeTally,
    ) -> wgpu::BindGroup {
        let stream_meta = ScatterStyleMapMeta {
            _pad: global_base,
            ..self.meta
        };
        let meta_buf = crate::gpu_memory::charged_buffer_init(
            tally,
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("figgy streamed point style map meta"),
                contents: bytemuck::bytes_of(&stream_meta),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy streamed point style map bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: self.style_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: self.override_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: meta_buf.as_entire_binding(),
                },
            ],
        })
    }
}

pub type ErrorBarStyleSlotGpu = ScatterStyleSlotGpu;

pub type ErrorBarStyleOverrideGpu = ScatterStyleOverrideGpu;

pub type ErrorBarStyleMapMeta = ScatterStyleMapMeta;

pub type ErrorBarStyleMap = ScatterStyleMap;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BarStyleSlotGpu {
    pub fill_color_premul: [f32; 4],
    pub border_color_premul: [f32; 4],
    /// `(border_width_px, gap_px, width_ratio, mask_bits as f32)`.
    pub params: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<BarStyleSlotGpu>() == 48);

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BarStyleOverrideGpu {
    pub bin_index: u32,
    pub _pad: [u32; 3],
    pub fill_color_premul: [f32; 4],
    pub border_color_premul: [f32; 4],
    /// `(border_width_px, gap_px, width_ratio, mask_bits as f32)`.
    pub params: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<BarStyleOverrideGpu>() == 64);

pub type BarStyleMapMeta = ScatterStyleMapMeta;

/// GPU-side sparse histogram-bin style map. It deliberately shares the
/// generic mapped-style bind-group layout (bindings 5..7), while its record
/// bytes are bar-specific and consumed only by `bar_columnar.wgsl`.
pub struct BarStyleMap {
    pub bind_group: wgpu::BindGroup,
}

pub fn create_per_point_style_map_bind_group_layout(
    device: &wgpu::Device,
) -> wgpu::BindGroupLayout {
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy per-point style map bgl"),
            entries: &[
                storage(5),
                storage(6),
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        },
    )
}

pub fn create_scatter_style_map_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    create_per_point_style_map_bind_group_layout(device)
}

pub fn create_scatter_style_map(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    style_slots: &[ScatterStyleSlotGpu],
    overrides: &[ScatterStyleOverrideGpu],
    meta: ScatterStyleMapMeta,
) -> ScatterStyleMap {
    let dummy_style = [ScatterStyleSlotGpu {
        color_premul: [0.0; 4],
        meta: [0.0; 4],
    }];
    let dummy_override = [ScatterStyleOverrideGpu {
        point_index: 0,
        _pad: [0; 3],
        color_premul: [0.0; 4],
        meta: [0.0; 4],
    }];
    let style_slots = if style_slots.is_empty() {
        &dummy_style[..]
    } else {
        style_slots
    };
    let overrides = if overrides.is_empty() {
        &dummy_override[..]
    } else {
        overrides
    };
    // gpu-alloc: caller
    let style_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy scatter style rows"),
        contents: bytemuck::cast_slice(style_slots),
        usage: wgpu::BufferUsages::STORAGE,
    });
    // gpu-alloc: caller
    let override_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy scatter style override rows"),
        contents: bytemuck::cast_slice(overrides),
        usage: wgpu::BufferUsages::STORAGE,
    });
    // gpu-alloc: caller
    let meta_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy scatter style map meta"),
        contents: bytemuck::bytes_of(&meta),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy scatter style map bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 5,
                resource: style_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: override_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: meta_buf.as_entire_binding(),
            },
        ],
    });
    ScatterStyleMap {
        bind_group,
        has_index: meta.has_index != 0,
        style_buf,
        override_buf,
        meta,
    }
}

pub fn create_errorbar_style_map(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    style_slots: &[ErrorBarStyleSlotGpu],
    overrides: &[ErrorBarStyleOverrideGpu],
    meta: ErrorBarStyleMapMeta,
) -> ErrorBarStyleMap {
    create_scatter_style_map(device, bgl, style_slots, overrides, meta)
}

pub fn create_bar_style_map(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    overrides: &[BarStyleOverrideGpu],
) -> BarStyleMap {
    let dummy_style = [BarStyleSlotGpu {
        fill_color_premul: [0.0; 4],
        border_color_premul: [0.0; 4],
        params: [0.0; 4],
    }];
    let dummy_override = [BarStyleOverrideGpu {
        bin_index: 0,
        _pad: [0; 3],
        fill_color_premul: [0.0; 4],
        border_color_premul: [0.0; 4],
        params: [0.0; 4],
    }];
    let override_rows = if overrides.is_empty() {
        &dummy_override[..]
    } else {
        overrides
    };
    let meta = BarStyleMapMeta {
        style_count: 0,
        override_count: overrides.len().min(u32::MAX as usize) as u32,
        has_index: 0,
        _pad: 0,
    };
    // gpu-alloc: caller
    let style_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy histogram style padding row"),
        contents: bytemuck::cast_slice(&dummy_style),
        usage: wgpu::BufferUsages::STORAGE,
    });
    // gpu-alloc: caller
    let override_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy histogram style override rows"),
        contents: bytemuck::cast_slice(override_rows),
        usage: wgpu::BufferUsages::STORAGE,
    });
    // gpu-alloc: caller
    let meta_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("figgy histogram style map meta"),
        contents: bytemuck::bytes_of(&meta),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy histogram style map bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 5,
                resource: style_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: override_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: meta_buf.as_entire_binding(),
            },
        ],
    });
    BarStyleMap { bind_group }
}
