//! Stylized draws: the Milkyway and Constellation texture sets, their baked
//! atlases and lookups, and the star layer.

use super::*;

/// Milkyway ribbon strip vertices per instance — `2·(S+1)`, twin of
/// `CONS_RIBBON_SUBDIV` in `line_columnar.wgsl`.
pub const MILKYWAY_RIBBON_VERTICES: u32 = 18;

pub(super) const PSF_SIZE: u32 = 128;

pub(super) const ATLAS_TILE: u32 = 128;

pub(super) const STYLE_STRIP_SIZE: u32 = 256;

/// Exact Rgba8Unorm payload bytes of either textured style set: one PSF,
/// one 2x2 planet atlas, and the blackbody and ring strips (one mip each).
pub(crate) const STYLED_TEXTURE_BYTES: u64 = 4
    * (PSF_SIZE as u64 * PSF_SIZE as u64
        + (ATLAS_TILE as u64 * 2) * (ATLAS_TILE as u64 * 2)
        + 2 * STYLE_STRIP_SIZE as u64);

/// Milkyway pipelines and baked style textures for one target format, cached
/// inside the renderer's lazy style set. The bind group keeps the textures
/// alive for as long as the pipelines can sample them.
pub(crate) struct MilkywaySet {
    pub(crate) ribbon: wgpu::RenderPipeline,
    pub(crate) stars: wgpu::RenderPipeline,
    /// Ringed-planet scatter pass uses premultiplied blending so planet bodies
    /// occlude the additive star field behind them.
    pub(crate) planets: wgpu::RenderPipeline,
    /// Bipolar-jet errorbars — additive beams + terminal shock knots over
    /// the precise errorbar geometry.
    pub(crate) jets: wgpu::RenderPipeline,
    pub(crate) star_tex_bg: wgpu::BindGroup,
}

/// Point-constellation pipelines + baked star textures for one target format.
/// The style intentionally supports only `ScatterLine`: `line` connects the
/// data points and `stars` renders PSF sprites at the scatter positions.
pub(crate) struct PointConstellationSet {
    pub(crate) line: wgpu::RenderPipeline,
    pub(crate) stars: wgpu::RenderPipeline,
    pub(crate) star_tex_bg: wgpu::BindGroup,
}

pub(super) fn bake_hash2(ix: i64, iy: i64, seed: u32) -> f64 {
    let mut h = (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (seed as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// 2D value noise, [0,1], smoothstep-interpolated.
pub(crate) fn vnoise2(x: f64, y: f64, seed: u32) -> f64 {
    let (ix, iy) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x.floor(), y - y.floor());
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = bake_hash2(ix, iy, seed);
    let b = bake_hash2(ix + 1, iy, seed);
    let c = bake_hash2(ix, iy + 1, seed);
    let d = bake_hash2(ix + 1, iy + 1, seed);
    a + (b - a) * ux + (c - a) * uy + (a - b - c + d) * ux * uy
}

/// Fractal Brownian motion over `vnoise2`, [0,1]-ish.
pub(crate) fn fbm2(x: f64, y: f64, octaves: u32, seed: u32) -> f64 {
    let mut acc = 0.0;
    let mut amp = 0.5;
    let (mut fx, mut fy) = (x, y);
    for o in 0..octaves {
        acc += amp * vnoise2(fx, fy, seed.wrapping_add(o * 131));
        amp *= 0.5;
        fx *= 2.03;
        fy *= 2.03;
    }
    acc
}

/// Planet albedo atlas: 2×2 archetype tiles (each `tile`² px, equirect).
/// Longitude-seamless: noise is sampled on the unit cylinder (cos θ, sin θ).
/// Archetypes: 0 gas giant (domain-warped bands), 1 ice giant, 2 rocky,
/// 3 cratered gray.
pub(super) fn bake_planet_atlas(tile: u32) -> Vec<u8> {
    let size = tile * 2;
    let mut out = vec![0u8; (size * size * 4) as usize];
    let mix3 = |a: [f64; 3], b: [f64; 3], t: f64| -> [f64; 3] {
        let t = t.clamp(0.0, 1.0);
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    for ty in 0..2u32 {
        for tx in 0..2u32 {
            let arch = ty * 2 + tx;
            for py in 0..tile {
                for px in 0..tile {
                    let u = px as f64 / (tile - 1) as f64; // longitude 0..1
                    let v = py as f64 / (tile - 1) as f64; // latitude 0..1
                    let th = u * std::f64::consts::TAU;
                    let (cx, sx) = (th.cos(), th.sin());

                    let rgb: [f64; 3] = match arch {
                        // Gas giant: latitude bands, domain-warped by
                        // cylinder-sampled fBm — the Jupiter look. Warp is
                        // kept mild so the bands stay BANDS (strong warp
                        // reads as marble, not a gas giant).
                        0 => {
                            let warp = fbm2(cx * 2.2 + 11.0, sx * 2.2 + v * 5.0, 5, 7) - 0.5;
                            let band_t = v * 9.0 + warp * 1.1;
                            let s = 0.5 + 0.5 * (band_t * std::f64::consts::TAU * 0.5).sin();
                            let turb = fbm2(cx * 5.0, sx * 5.0 + v * 14.0, 5, 23) - 0.5;
                            let cream = [0.88, 0.80, 0.66];
                            let rust = [0.58, 0.36, 0.24];
                            let mut c = mix3(cream, rust, s * 0.9 + turb * 0.18);
                            // One dark belt accent.
                            let belt = (-(v - 0.62).powi(2) / 0.002).exp();
                            c = mix3(c, [0.42, 0.26, 0.18], belt * 0.55);
                            c
                        }
                        // Ice giant: smooth teal with faint streaks.
                        1 => {
                            let s = fbm2(cx * 1.6, sx * 1.6 + v * 7.0, 4, 41) - 0.5;
                            let base =
                                mix3([0.34, 0.52, 0.86], [0.55, 0.72, 0.95], v * 0.5 + s * 0.25);
                            let streak = (-(v - 0.35).powi(2) / 0.004).exp();
                            mix3(base, [0.85, 0.92, 1.0], streak * 0.35)
                        }
                        // Rocky: ochre terrain patches + polar caps.
                        2 => {
                            let t1 = fbm2(cx * 3.0, sx * 3.0 + v * 6.0, 6, 67);
                            let mut c =
                                mix3([0.72, 0.46, 0.28], [0.44, 0.27, 0.17], (t1 - 0.35) * 2.0);
                            let polar = ((v - 0.5).abs() * 2.0 - 0.78).max(0.0) / 0.22;
                            c = mix3(c, [0.92, 0.90, 0.86], polar.min(1.0) * 0.8);
                            c
                        }
                        // Cratered gray: maria blotches over regolith noise.
                        _ => {
                            let t1 = fbm2(cx * 3.4, sx * 3.4 + v * 7.0, 6, 97);
                            let t2 = fbm2(cx * 1.4 + 5.0, sx * 1.4 + v * 3.0, 4, 113);
                            let g = 0.58 + (t1 - 0.5) * 0.30 - if t2 > 0.62 { 0.18 } else { 0.0 };
                            [g, g, g * 1.02]
                        }
                    };

                    let x = tx * tile + px;
                    let y = ty * tile + py;
                    let i = ((y * size + x) * 4) as usize;
                    out[i] = (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8;
                    out[i + 1] = (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8;
                    out[i + 2] = (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8;
                    out[i + 3] = 255;
                }
            }
        }
    }
    out
}

/// Ring radial strip (256×1): C ring (faint) → B ring (bright) → Cassini
/// gap → A ring, with fine radial density noise. RGB is the straight ring
/// color; A is the density the shader composes with.
pub(super) fn bake_ring_strip() -> Vec<u8> {
    let mut out = vec![0u8; STYLE_STRIP_SIZE as usize * 4];
    for i in 0..STYLE_STRIP_SIZE as usize {
        let u = i as f64 / (STYLE_STRIP_SIZE - 1) as f64;
        let base = if u < 0.16 {
            0.22
        } else if u < 0.52 {
            0.85
        } else if u < 0.60 {
            0.04
        } else if u < 0.93 {
            0.60 * (1.0 - (u - 0.60) / 0.33 * 0.35)
        } else {
            0.0
        };
        let fine = (fbm2(u * 60.0, 0.5, 4, 151) - 0.5) * 0.35;
        let a = (base * (1.0 + fine)).clamp(0.0, 1.0);
        let rgb: [f64; 3] = [0.80, 0.74, 0.63];
        out[i * 4] = (rgb[0] * 255.0).round() as u8;
        out[i * 4 + 1] = (rgb[1] * 255.0).round() as u8;
        out[i * 4 + 2] = (rgb[2] * 255.0).round() as u8;
        out[i * 4 + 3] = (a * 255.0).round() as u8;
    }
    out
}

/// Bake the star PSF sprite (R = saturating core, G = halo wings + one faint
/// Airy-style ring). This runs once per style-set creation, so the render loop
/// only samples the baked texture.
pub(super) fn bake_psf_rgba(size: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    let half = (size as f32 - 1.0) * 0.5;
    for y in 0..size {
        for x in 0..size {
            let dx = (x as f32 - half) / half; // -1..1
            let dy = (y as f32 - half) / half;
            let r = (dx * dx + dy * dy).sqrt();
            let edge_t = ((r - 0.82) / (0.98 - 0.82)).clamp(0.0, 1.0);
            let aperture = 1.0 - edge_t * edge_t * (3.0 - 2.0 * edge_t);
            // Flat saturated core with a steep gaussian shoulder.
            let core = (-(r / 0.11).powf(2.6)).exp().min(1.0) * aperture;
            // Exponential halo wings + one faint ring at 0.45.
            let halo =
                (0.85 * (-r / 0.28).exp() + 0.08 * (-((r - 0.45) / 0.06).powi(2)).exp()) * aperture;
            let i = ((y * size + x) * 4) as usize;
            out[i] = (core.clamp(0.0, 1.0) * 255.0).round() as u8;
            out[i + 1] = (halo.clamp(0.0, 1.0) * 255.0).round() as u8;
            out[i + 2] = 0;
            out[i + 3] = 255;
        }
    }
    out
}

/// Bake the 256×1 blackbody LUT, 2,500 K → 12,000 K (Tanner Helland's
/// piecewise fit — visually faithful Planckian locus, never green).
pub(super) fn bake_blackbody_lut() -> Vec<u8> {
    let mut out = vec![0u8; STYLE_STRIP_SIZE as usize * 4];
    for i in 0..STYLE_STRIP_SIZE as usize {
        let kelvin = 2500.0 + 9500.0 * (i as f64 / (STYLE_STRIP_SIZE - 1) as f64);
        let t = kelvin / 100.0;
        let r = if t <= 66.0 {
            255.0
        } else {
            329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2)
        };
        let g = if t <= 66.0 {
            99.470_802_586_1 * t.ln() - 161.119_568_166_1
        } else {
            288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2)
        };
        let b = if t >= 66.0 {
            255.0
        } else if t <= 19.0 {
            0.0
        } else {
            138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7
        };
        out[i * 4] = r.clamp(0.0, 255.0).round() as u8;
        out[i * 4 + 1] = g.clamp(0.0, 255.0).round() as u8;
        out[i * 4 + 2] = b.clamp(0.0, 255.0).round() as u8;
        out[i * 4 + 3] = 255;
    }
    out
}

/// Build the milkyway style set: bake PSF + blackbody LUT, upload them,
/// and compile the additive ribbon/star pipelines (group 2 = the textures).
// Milkyway assembly combines both texture resources and both line/star layouts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_milkyway_set(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    shaders: &ShaderModules,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    star_data_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> MilkywaySet {
    let make_tex = |label: &str, w: u32, h: u32, data: &[u8]| {
        // gpu-alloc: uncharged(baked inside a lazily-compiled style set)
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    };
    let psf_view = make_tex(
        "figgy milkyway psf",
        PSF_SIZE,
        PSF_SIZE,
        &bake_psf_rgba(PSF_SIZE),
    );
    let lut_view = make_tex(
        "figgy milkyway blackbody lut",
        STYLE_STRIP_SIZE,
        1,
        &bake_blackbody_lut(),
    );
    let atlas_view = make_tex(
        "figgy milkyway planet atlas",
        ATLAS_TILE * 2,
        ATLAS_TILE * 2,
        &bake_planet_atlas(ATLAS_TILE),
    );
    let ring_view = make_tex(
        "figgy milkyway ring strip",
        STYLE_STRIP_SIZE,
        1,
        &bake_ring_strip(),
    );

    let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let tex_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy milkyway texture bgl"),
            entries: &[
                tex_entry(0), // PSF (stars)
                tex_entry(1), // blackbody LUT (stars)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex_entry(3), // planet atlas (planets)
                tex_entry(4), // ring strip (planets)
            ],
        },
    );
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("figgy milkyway sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let star_tex_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy milkyway texture bg"),
        layout: &tex_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&psf_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&lut_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&ring_view),
            },
        ],
    });

    let additive = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    };
    // MAX, not ADD: the ribbon seals curve joints by overlapping square-cap
    // extensions (vs_ribbon), and max() keeps that overlap from
    // double-brightening — the haze is a field, not an accumulation.
    let max_blend = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Max,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Max,
        },
    };
    let ribbon = create_line_columnar_pipeline_with_entries(
        device,
        &shaders.line,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_ribbon",
        "fs_ribbon",
        max_blend,
        wgpu::PrimitiveTopology::TriangleStrip,
        Some(&tex_bgl),
        "figgy milkyway ribbon pipeline",
    );
    // Arc-driven star pass: NO vertex buffers — the VS reads the arc-length
    // prefix and the column pool as storage (group 3) and is drawn via
    // DrawIndirect args computed on the GPU from the polyline's total arc.
    let star_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy milkyway star layout"),
            bind_group_layouts: &[
                Some(transform_bgl),
                Some(style_bgl),
                Some(&tex_bgl),
                Some(star_data_bgl),
            ],
            immediate_size: 0,
        },
    );
    let stars = crate::gpu_compile::render_pipeline(
        &device,
        &wgpu::RenderPipelineDescriptor {
            label: Some("figgy milkyway stars pipeline"),
            layout: Some(&star_layout),
            vertex: wgpu::VertexState {
                module: &shaders.line,
                entry_point: Some("vs_stars"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
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
                module: &shaders.line,
                entry_point: Some("fs_stars"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(additive),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        },
    );
    // Planets keep the scatter builder's premultiplied blend — bodies
    // occlude the additive star field behind them.
    let planets = create_scatter_columnar_pipeline_full(
        device,
        &shaders.scatter,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_planet",
        "fs_planet",
        Some(&tex_bgl),
        "figgy milkyway planets pipeline",
    );
    let jets = create_errorbar_columnar_pipeline_full(
        device,
        &shaders.errorbar,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_jet",
        "fs_jet",
        additive,
        "figgy milkyway jets pipeline",
    );

    MilkywaySet {
        ribbon,
        stars,
        planets,
        jets,
        star_tex_bg,
    }
}

/// Build the lightweight constellation style set: PSF stars at scatter
/// positions plus a translucent connecting line. No arc-prefix star pass,
/// planets, or jets are compiled for this mode.
pub(crate) fn create_point_constellation_set(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    shaders: &ShaderModules,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> PointConstellationSet {
    let make_tex = |label: &str, w: u32, h: u32, data: &[u8]| {
        // gpu-alloc: uncharged(baked inside a lazily-compiled style set)
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    };
    let psf_view = make_tex(
        "figgy point constellation psf",
        PSF_SIZE,
        PSF_SIZE,
        &bake_psf_rgba(PSF_SIZE),
    );
    let lut_view = make_tex(
        "figgy point constellation blackbody lut",
        STYLE_STRIP_SIZE,
        1,
        &bake_blackbody_lut(),
    );
    // These two are unused by point constellation entries, but keeping the
    // five-binding texture layout identical lets the shared scatter WGSL
    // module declare both star and planet resources.
    let atlas_view = make_tex(
        "figgy point constellation planet atlas",
        ATLAS_TILE * 2,
        ATLAS_TILE * 2,
        &bake_planet_atlas(ATLAS_TILE),
    );
    let ring_view = make_tex(
        "figgy point constellation ring strip",
        STYLE_STRIP_SIZE,
        1,
        &bake_ring_strip(),
    );

    let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let tex_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy point constellation texture bgl"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex_entry(3),
                tex_entry(4),
            ],
        },
    );
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("figgy point constellation sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let star_tex_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy point constellation texture bg"),
        layout: &tex_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&psf_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&lut_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&ring_view),
            },
        ],
    });

    let line = create_line_columnar_pipeline_with_entries(
        device,
        &shaders.line,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_main",
        "fs_constellation_line",
        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
        wgpu::PrimitiveTopology::TriangleStrip,
        None,
        "figgy point constellation line pipeline",
    );
    let stars = create_scatter_columnar_pipeline_full(
        device,
        &shaders.scatter,
        transform_bgl,
        style_bgl,
        target_format,
        sample_count,
        "vs_constellation_star",
        "fs_constellation_star",
        Some(&tex_bgl),
        "figgy point constellation stars pipeline",
    );

    PointConstellationSet {
        line,
        stars,
        star_tex_bg,
    }
}

/// Bind group layout for the Milkyway star pass's per-series data
/// (group 3 of the stars pipeline): the arc-length prefix and the column
/// pool as read-only storage plus a small offsets uniform. Read-only storage
/// in the vertex stage is core WebGPU; the GL-downlevel adapters that lack
/// it are already rejected at renderer construction.
pub fn create_star_data_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let storage = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
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
            label: Some("figgy star data bgl"),
            entries: &[
                storage(0), // arc-length prefix
                storage(1), // column pool (x/y bases in the uniform)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
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

/// Constellation star pass — an arc-driven indirect draw. No vertex
/// buffers: the vertex shader walks the arc-length prefix (binary search)
/// and fetches segment endpoints from the pool, both bound as read-only
/// storage in `star_bg`; `indirect` holds the GPU-computed DrawIndirect
/// args (instance count = candidate slots over the total arc).
pub struct ColumnStarLayer<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub transform_bg: &'a wgpu::BindGroup,
    pub style_bg: &'a wgpu::BindGroup,
    pub texture_bg: &'a wgpu::BindGroup,
    pub star_bg: &'a wgpu::BindGroup,
    pub indirect: &'a wgpu::Buffer,
}
