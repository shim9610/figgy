#![doc = include_str!("../RADIAL.md")]
//! Resident categorical charts with analytic sector surfaces and procedural
//! materials. No Cartesian columns, auto-fit jobs, or stream replay are involved.
//! `prepare` returns an immutable frame; repeated identical snapshots reuse it.
use crate::gpu_memory::{GpuMemoryUsage, GpuResourceKind, TrackedBuffer, TrackedTexture};
use crate::gpu_readback::{GpuReadback, ReadbackError};
use crate::{RasterImage, RendererDevice};
pub use model::radial::*;
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// The assembled radial module.
pub(crate) const RADIAL_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/material.wgsl",
    "data_render/common/radial_sector.wgsl",
    "radial.wgsl",
);
/// The radial pick module: the same sector fragment as the draw, with a
/// compute entry that picks the sector the draw shows under one pixel.
pub(crate) const RADIAL_PICK_SHADER: crate::shader_assembly::Assembled = crate::shader_assembly::assembled!(
    "data_render/common/material.wgsl",
    "data_render/common/radial_sector.wgsl",
    "radial_pick.wgsl",
);

#[path = "radial_layout.rs"]
mod layout;

mod bind;
mod create;
mod frame;
mod prepare;
mod render;
mod vector;

use frame::*;
use prepare::*;
#[cfg(feature = "cpu-twin")]
pub use vector::*;

#[derive(Debug)]
pub enum RadialError {
    Invalid(crate::RuleKind),
    Budget { needed: u64, limit: u64 },
    Gpu(String),
}
impl std::fmt::Display for RadialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(kind) => write!(f, "{kind}"),
            Self::Budget { needed, limit } => {
                write!(f, "radial GPU budget: {needed} bytes needed, limit {limit}")
            }
            Self::Gpu(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for RadialError {}

/// Run `create` inside the device's error scopes
/// ([`crate::renderer::gpu_checked`]): an out-of-memory or validation
/// failure is a [`RadialError::Gpu`], not a panic (#152).
fn gpu_created<T>(
    device: &wgpu::Device,
    resource: &'static str,
    create: impl FnOnce() -> T,
) -> Result<T, RadialError> {
    crate::renderer::gpu_checked(device, resource, create).map_err(radial_gpu_error)
}

fn radial_gpu_error(error: crate::FiggyError) -> RadialError {
    RadialError::Gpu(error.to_string())
}
/// A readback failure as the radial chart has always reported it.
fn radial_readback_error(error: ReadbackError) -> RadialError {
    match error {
        ReadbackError::Poll(error) | ReadbackError::Health(error) => radial_gpu_error(error),
        ReadbackError::Canceled(error) => RadialError::Gpu(error.to_string()),
        ReadbackError::Map(error) => RadialError::Gpu(error.to_string()),
        ReadbackError::Range(error) => RadialError::Gpu(error.to_string()),
    }
}
/// One sector as the radial modules read it (`radial_sector.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Sector {
    pub(crate) geometry: [f32; 4],
    pub(crate) arc: [f32; 4],
    pub(crate) color: [f32; 4],
    pub(crate) material: crate::data_render::MaterialGpu,
    pub(crate) detail: [f32; 4], // explode, angular gap, bevel, reserved (0, seed moved to `material`)
    pub(crate) rounding: [f32; 4], // inner corner, outer corner, vertical lift, reserved (0, gloss moved)
    pub(crate) outline: [f32; 4],
    pub(crate) outline_color: [f32; 4],
    // reserved (0, highlight moved), reserved (0, texture angle moved),
    // inside-label flag, inside-label lift: how far the lifted top face moves
    // up the screen in physical pixels (`rounding[2] * sin(arc[2]) *
    // geometry[2]`), computed once where the sector is built
    // (`radial_layout.rs`). The layout moves the inside labels or their hit
    // map by it and the shader moves the inside glyphs by it.
    pub(crate) effects: [f32; 4],
    pub(crate) light: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    size: [f32; 4],
    background: [f32; 4],
    light: [f32; 4],
    shadow: [f32; 4],
    groups: [[f32; 4]; 2],
    meta: [f32; 4],
}
const _: () =
    assert!(std::mem::size_of::<Sector>() == 176 && std::mem::size_of::<Globals>() == 112);

/// The radial pick's query (`RadialPickQuery` of `radial_pick.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct RadialPickQueryGpu {
    pub(crate) pixel: [f32; 2],
    pub(crate) count: u32,
    pub(crate) rest: u32,
}
/// The radial pick's result (`RadialPickResult` of `radial_pick.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct RadialPickResultGpu {
    pub(crate) index: u32,
    pub(crate) hit: u32,
    pub(crate) t: f32,
    pub(crate) reserved: u32,
}
const _: () = assert!(
    std::mem::size_of::<RadialPickQueryGpu>() == 16
        && std::mem::size_of::<RadialPickResultGpu>() == 16
);
const RADIAL_PICK_RESULT_BYTES: u64 = std::mem::size_of::<RadialPickResultGpu>() as u64;

/// What a radial raster export draws under the chart (#313): an option of
/// the export alone. The chart, its selection and hover, and every frame
/// already prepared (the one on screen included) are not changed by it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum RadialExportBackground {
    /// The chart's own `background` over [`crate::ExportOptions::clear`],
    /// as [`RadialRenderer::export_rgba`] draws it.
    #[default]
    Document,
    /// No background: the slices, the shadows they cast, the labels and the
    /// annotations over transparent pixels, whatever the chart's background
    /// and `ExportOptions::clear`.
    Transparent,
    /// This straight-alpha colour (each component in `0..=1`) in place of
    /// the chart's background, over `ExportOptions::clear`.
    Color(crate::Color),
}

/// Immutable GPU snapshot. Its shader draws straight into a same-format,
/// single-sample target; four analytic subpixel samples provide edge AA.
/// Labels are rasterized at the requested output resolution, never upscaled.
pub struct RadialFrame {
    decorations: crate::HitMap<RadialChart>,
    gpu: RendererDevice,
    size: (u32, u32),
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    pick_pipeline: wgpu::ComputePipeline,
    pick_layout: wgpu::BindGroupLayout,
    _uniform: TrackedBuffer,
    /// The frame's globals as its uniform holds them: an export with
    /// another background ([`RadialExportBackground`]) binds a copy with
    /// only the background replaced, next to this frame's sectors and labels.
    globals: Globals,
    sectors: TrackedBuffer,
    _annotations: Arc<TrackedTexture>,
    /// The CPU sectors, kept only for the CPU twin's agreement test.
    #[cfg(feature = "cpu-twin")]
    geometry: Vec<Sector>,
    /// The slice each sector of the buffer draws, in buffer order.
    targets: Vec<RadialTarget>,
    scale: f32,
    space: crate::annotation::PixelSpace,
}
/// An asynchronous radial pick, from [`RadialFrame::pick`] or
/// [`RadialFrame::pick_at_rest`]: the request is submitted, and
/// [`Self::resolve`] reads the GPU's answer back. The ticket keeps its frame
/// alive until it resolves or is dropped.
pub struct RadialPickTicket {
    pending: Option<PendingRadialPick>,
}
struct PendingRadialPick {
    frame: Arc<RadialFrame>,
    /// The result, mapped once the pick completes.
    readback: GpuReadback,
    /// The `MAP_READ` buffer's charge, credited when the pick resolves.
    _readback: TrackedBuffer,
    /// The query and result buffers, credited when the pick resolves.
    _scratch: [TrackedBuffer; 2],
}
/// A radial export's submitted render: the target copied into the
/// readback buffer, its map requested.
struct RadialRgbaJob {
    readback: GpuReadback,
    size: (u32, u32),
    stride: u64,
    _target: TrackedTexture,
    _buffer: TrackedBuffer,
    #[cfg(target_arch = "wasm32")]
    scopes: Option<crate::renderer::BrowserErrorScopes>,
}

/// A radial vector export's frame, bound for its one image render.
struct RadialVectorFrame {
    size: (u32, u32),
    layout: layout::Layout,
    bind_group: wgpu::BindGroup,
    _labels: TrackedTexture,
    _uniform: TrackedBuffer,
    _sectors: TrackedBuffer,
}

/// Shares the host device/queue. Owns one pipeline and at most one cached frame.
/// Separate from the numeric-data/Cartesian renderer.
pub struct RadialRenderer {
    gpu: RendererDevice,
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    pick_layout: wgpu::BindGroupLayout,
    pick_pipeline: wgpu::ComputePipeline,
    annotations_cached: Option<(FrameKey, (u32, u32), f32, u64, Arc<TrackedTexture>)>,
    /// The dedicated annotation layer, drawn over the frame by its own pass.
    annotation_layer: crate::annotation::RadialAnnotationCache,
    cached: Option<(FrameKey, (u32, u32), f32, u64, Arc<RadialFrame>)>,
}
/// The browser device outlives the radial renderer's submitted work, and a
/// device figgy requested is destroyed once no one else holds it, as for
/// [`crate::Renderer`] (`release_after_submitted_work`, #322).
#[cfg(target_arch = "wasm32")]
impl Drop for RadialRenderer {
    fn drop(&mut self) {
        crate::renderer::release_after_submitted_work(
            self.gpu.health(),
            self.gpu.device(),
            self.gpu.queue(),
        );
    }
}

#[cfg(test)]
mod pick_module_tests {
    use super::*;
    /// The draw and the pick splice one sector fragment after the material,
    /// so the pick answers with the draw's own ray-march.
    #[test]
    fn draw_and_pick_splice_the_same_sector_fragment() {
        let shared = [
            "data_render/common/material.wgsl",
            "data_render/common/radial_sector.wgsl",
        ];
        assert_eq!(RADIAL_SHADER.fragments[..2], shared);
        assert_eq!(RADIAL_PICK_SHADER.fragments[..2], shared);
        assert_eq!(RADIAL_SHADER.fragments.len(), 3);
        assert_eq!(RADIAL_PICK_SHADER.fragments.len(), 3);
        // The draw's own file keeps no sector definition, only the markers
        // of the blocks the fragment holds.
        let draw = include_str!("radial.wgsl");
        for definition in ["struct Sector", "fn hit_pixel", "fn footprint", "fn solid"] {
            assert!(
                !draw.lines().any(|line| line.starts_with(definition)),
                "{definition}"
            );
        }
        assert_eq!(
            draw.lines()
                .filter(|line| line.starts_with("// [data_render/common/radial_sector.wgsl:"))
                .count(),
            2
        );
    }

    /// The pick, the draw's shaded winner (#349) and the moving inside
    /// glyphs count their winner from 1: Metal compiles a 0-based
    /// `winner=i` as a constant 0 (#63).
    #[test]
    fn sector_index_loops_count_the_winner_from_one() {
        for (name, source, loops) in [
            ("radial.wgsl", include_str!("radial.wgsl"), 2),
            ("radial_pick.wgsl", include_str!("radial_pick.wgsl"), 1),
        ] {
            assert!(!source.contains("winner=i;"), "{name}");
            assert_eq!(source.matches("winner=i+1u;").count(), loops, "{name}");
        }
    }
}

/// Shading once per surface (#349) is an appearance change: it moves no
/// coverage, no data and no pick, and the image stays within a pinned
/// colour difference of the draw that shaded every sample.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod shading_tolerance_tests {
    use super::*;
    use crate::Material;
    use crate::test_gpu::BlockOn as _;

    /// The draw before #349, as test support: the module the renderer
    /// compiles with the entry `fs_per_sample` appended, which shades every
    /// sample at its own position with the same functions.
    fn per_sample_pipeline(r: &RadialRenderer) -> wgpu::RenderPipeline {
        let device = r.gpu.device();
        let source = format!(
            "{}\n{}",
            RADIAL_SHADER.source,
            include_str!("../tests/support/radial_per_sample.wgsl")
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("radial per-sample reference"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&r.layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("radial per-sample reference"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_per_sample"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: r.format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    }

    fn renderer() -> RadialRenderer {
        let (device, queue) = crate::test_gpu::device();
        RadialRenderer::try_new(
            RendererDevice::new(device, queue).expect("figgy poll thread"),
            wgpu::TextureFormat::Rgba8Unorm,
        )
        .unwrap()
    }

    const PALETTE: [[u8; 3]; 6] = [
        [217, 76, 64],
        [64, 128, 217],
        [76, 178, 89],
        [242, 204, 51],
        [150, 90, 200],
        [60, 180, 190],
    ];
    fn donut(values: &[f64], style: RadialStyle) -> RadialChart {
        let mut c = RadialChart {
            kind: RadialKind::Donut { inner_radius: 0.5 },
            start_angle_degrees: -90.0,
            labels: RadialLabels::Outside,
            font_size: 13.0,
            label_color: crate::Color::new(0.2, 0.4, 0.65, 1.0),
            background: crate::Color::WHITE,
            style,
            slices: values
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let [r, g, b] = PALETTE[i % PALETTE.len()];
                    RadialSlice::new(format!("{}", i + 1), *v, crate::Color::from_rgb8(r, g, b))
                })
                .collect(),
            ..Default::default()
        };
        c.format_labels();
        c
    }
    /// The Ceramic donut of the issue (#349).
    fn ceramic() -> RadialStyle {
        RadialStyle {
            material: Material::Ceramic,
            tilt_degrees: 38.0,
            depth: 0.10,
            bevel: 0.012,
            shadow: true,
            inner_corner: 0.008,
            outer_corner: 0.008,
            gloss: 0.3,
            roughness: 0.48,
            texture_strength: 0.08,
            gap_degrees: 1.3,
            ..RadialStyle::default()
        }
    }
    fn outlined(mut style: RadialStyle) -> RadialStyle {
        style.outline.rim = true;
        style.outline.separators = true;
        style.outline.width = 1.5;
        style.outline.color = crate::Color::new(0.1, 0.1, 0.1, 1.0);
        style
    }
    /// Eleven slices, one per material.
    fn materials(style: impl Fn(RadialStyle) -> RadialStyle) -> RadialChart {
        let base = style(RadialStyle {
            tilt_degrees: 25.0,
            depth: 0.08,
            bevel: 0.012,
            shadow: true,
            inner_corner: 0.008,
            outer_corner: 0.008,
            gap_degrees: 1.3,
            material: Material::Ceramic,
            ..RadialStyle::default()
        });
        let mut c = donut(&[1.0; 11], base.clone());
        let all = [
            Material::Matte,
            Material::Ceramic,
            Material::BrushedMetal,
            Material::Paper,
            Material::Wood,
            Material::SatinMetal,
            Material::Toon,
            Material::Enamel,
            Material::Hatch,
            Material::Pearl,
            Material::Flat,
        ];
        for (slice, material) in c.slices.iter_mut().zip(all) {
            slice.style = Some(RadialStyle {
                material,
                ..base.clone()
            });
        }
        c
    }
    fn charts() -> Vec<(&'static str, RadialChart)> {
        const SIX: [f64; 6] = [31.0, 23.0, 20.0, 12.0, 8.0, 6.0];
        let textured = |material| RadialStyle {
            material,
            tilt_degrees: 30.0,
            depth: 0.12,
            texture_strength: 0.8,
            gloss: 0.6,
            ..RadialStyle::default()
        };
        let mut v = vec![
            ("flat", donut(&SIX, RadialStyle::default())),
            (
                "flat shadow",
                donut(
                    &SIX,
                    RadialStyle {
                        shadow: true,
                        ..RadialStyle::default()
                    },
                ),
            ),
            (
                "flat outlined",
                donut(&SIX, outlined(RadialStyle::default())),
            ),
            ("ceramic", donut(&SIX, ceramic())),
            ("ceramic outlined", donut(&SIX, outlined(ceramic()))),
            ("materials", materials(|s| s)),
            ("materials outlined", materials(outlined)),
            ("brushed", donut(&SIX, textured(Material::BrushedMetal))),
            ("wood", donut(&SIX, textured(Material::Wood))),
            (
                "ceramic depth 0",
                donut(
                    &SIX,
                    RadialStyle {
                        depth: 0.0,
                        ..ceramic()
                    },
                ),
            ),
            (
                "ceramic tilt 0",
                donut(
                    &SIX,
                    RadialStyle {
                        tilt_degrees: 0.0,
                        ..ceramic()
                    },
                ),
            ),
            ("ceramic 3 slices", donut(&[50.0, 30.0, 20.0], ceramic())),
        ];
        let mut c = donut(&SIX, ceramic());
        c.slices[1].explode = 0.15;
        c.slices[3].explode = 0.25;
        v.push(("ceramic explode", c));
        let mut c = donut(&SIX, ceramic());
        c.labels = RadialLabels::Inside;
        c.style.hover_lift = 0.06;
        c.interaction.hovered = Some(RadialTarget::main(1));
        c.interaction.hover_progress = 1.0;
        c.format_labels();
        v.push(("ceramic hover lift, inside labels", c));
        let mut c = donut(&SIX, outlined(ceramic()));
        c.style.outline.emphasis = true;
        c.interaction.selected = Some(RadialTarget::main(2));
        c.interaction.hover_progress = 1.0;
        v.push(("ceramic selected, emphasis", c));
        let mut c = donut(&SIX, ceramic());
        c.split = Some(RadialSplit {
            slice_index: 1,
            kind: RadialKind::Pie,
            children: vec![
                RadialSlice::new("D", 10.0, crate::Color::from_rgb8(0, 200, 150)),
                RadialSlice::new("E", 13.0, crate::Color::from_rgb8(220, 180, 80)),
            ],
        });
        c.format_labels();
        v.push(("ceramic detail", c));
        // A sunburst stays level (`RuleKind::SunburstTilt`).
        let mut c = donut(
            &SIX,
            RadialStyle {
                tilt_degrees: 0.0,
                depth: 0.0,
                ..ceramic()
            },
        );
        c.kind = RadialKind::Pie;
        c.slices[0].children = vec![
            RadialSlice::new("a", 10.0, crate::Color::from_rgb8(200, 100, 50)),
            RadialSlice::new("b", 21.0, crate::Color::from_rgb8(50, 100, 200)),
        ];
        c.slices[2].children = vec![RadialSlice::new(
            "c",
            12.0,
            crate::Color::from_rgb8(90, 200, 90),
        )];
        c.format_labels();
        v.push(("ceramic sunburst", c));
        v
    }

    /// `chart` drawn as one opaque Flat colour on white, with no outline or
    /// hover brightness, and with its shadow and labels, which keep the
    /// layout: a pixel of that colour has every sample on a sector, a grey
    /// one every sample on the background (white, or shadowed), and a
    /// label's pixels are neither.
    fn coverage_chart(chart: &RadialChart) -> RadialChart {
        let ink = crate::Color::from_rgb8(255, 0, 255);
        let plain = |s: &mut RadialStyle| {
            s.material = Material::Flat;
            s.outline.rim = false;
            s.outline.separators = false;
            s.outline.emphasis = false;
            s.hover_brightness = 0.0;
        };
        fn paint(slices: &mut [RadialSlice], ink: crate::Color, plain: &dyn Fn(&mut RadialStyle)) {
            for s in slices {
                s.color = ink;
                if let Some(style) = s.style.as_mut() {
                    plain(style);
                }
                paint(&mut s.children, ink, plain);
            }
        }
        let mut c = chart.clone();
        c.background = crate::Color::WHITE;
        plain(&mut c.style);
        paint(&mut c.slices, ink, &plain);
        if let Some(split) = c.split.as_mut() {
            paint(&mut split.children, ink, &plain);
        }
        c
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Cover {
        Background,
        Edge,
        Interior,
    }

    #[test]
    fn shading_once_per_surface_stays_within_the_pinned_difference() {
        let mut r = renderer();
        let reference = per_sample_pipeline(&r);
        let shipped = r.pipeline.clone();
        let size = (600, 400);
        let mut report = String::new();
        for (name, chart) in charts() {
            for scale in [1.0f32, 2.0] {
                let export =
                    |r: &mut RadialRenderer, pipeline: &wgpu::RenderPipeline, c: &RadialChart| {
                        r.pipeline = pipeline.clone();
                        r.cached = None;
                        let image = r
                            .export_rgba(c, size, crate::ExportOptions::document(scale))
                            .block_on()
                            .unwrap();
                        r.end_frame();
                        image
                    };
                let new = export(&mut r, &shipped, &chart);
                let old = export(&mut r, &reference, &chart);
                let mask = export(&mut r, &shipped, &coverage_chart(&chart));
                let (w, h) = (new.width as usize, new.height as usize);
                let cover: Vec<Cover> = mask
                    .rgba
                    .chunks_exact(4)
                    .map(|p| match p {
                        [255, 0, 255, 255] => Cover::Interior,
                        [r, g, b, 255] if r == g && g == b => Cover::Background,
                        _ => Cover::Edge,
                    })
                    .collect();
                // The largest channel difference per pixel, in 8-bit sRGB.
                let mut differences: Vec<u8> = new
                    .rgba
                    .chunks_exact(4)
                    .zip(old.rgba.chunks_exact(4))
                    .map(|(a, b)| (0..4).map(|i| a[i].abs_diff(b[i])).max().unwrap())
                    .collect();
                let mean = differences.iter().map(|&d| d as f64).sum::<f64>() / (w * h) as f64;
                let flat = chart.style.material == Material::Flat
                    && !chart.style.shadow
                    && chart.slices.iter().all(|s| s.style.is_none());
                let textured_interior = |i: usize| !flat && cover[i] == Cover::Interior;
                let edge_max = (0..w * h)
                    .filter(|&i| cover[i] == Cover::Edge)
                    .map(|i| differences[i])
                    .max()
                    .unwrap_or(0);
                let background_max = (0..w * h)
                    .filter(|&i| cover[i] == Cover::Background)
                    .map(|i| differences[i])
                    .max()
                    .unwrap_or(0);
                let outside_max = (0..w * h)
                    .filter(|&i| !textured_interior(i))
                    .map(|i| differences[i])
                    .max()
                    .unwrap_or(0);
                differences.sort_unstable();
                let p99 = differences[(w * h * 99) / 100];
                let max = differences[w * h - 1];
                report.push_str(&format!(
                    "{name} @{scale}x: mean {mean:.3}, p99 {p99}, max {max}, \
                     edge max {edge_max}, background max {background_max}, \
                     outside textured interiors {outside_max}\n"
                ));
                // Measured on lavapipe and SwiftShader (#349): a mean of at
                // most 0.22, a p99 of at most 3, at most 3 at edges and on
                // the background (its shadow, at most 2) and at most 14 in a
                // textured interior. A Flat chart without a shadow is exact.
                if flat {
                    assert_eq!(max, 0, "{name} @{scale}x: a Flat chart is exact\n{report}");
                }
                assert!(mean <= 0.35, "{name} @{scale}x: mean {mean}\n{report}");
                assert!(p99 <= 4, "{name} @{scale}x: p99 {p99}\n{report}");
                assert!(
                    outside_max <= 5,
                    "{name} @{scale}x: outside textured interiors {outside_max}\n{report}"
                );
                assert!(max <= 20, "{name} @{scale}x: max {max}\n{report}");
                if scale != 1.0 {
                    continue;
                }
                // The pick, at points a pixel or more from any edge, finds a
                // sector exactly where every sample does, from frames of
                // both draws.
                // A label's pixels count as edges, so a point in a label
                // is skipped.
                let far_from_edges = |x: usize, y: usize| {
                    let c = cover[y * w + x];
                    (c != Cover::Edge
                        && (y - 1..=y + 1).all(|v| (x - 1..=x + 1).all(|u| cover[v * w + u] == c)))
                    .then_some(c)
                };
                let frames = [&shipped, &reference].map(|pipeline| {
                    r.pipeline = pipeline.clone();
                    r.cached = None;
                    r.prepare(&chart, size, scale).unwrap()
                });
                let mut points = 0;
                for y in (2..h - 2).step_by(13) {
                    for x in (2..w - 2).step_by(13) {
                        let Some(c) = far_from_edges(x, y) else {
                            continue;
                        };
                        points += 1;
                        let at = [x as f32 + 0.5, y as f32 + 0.5];
                        let picked = frames
                            .each_ref()
                            .map(|f| f.pick(at).unwrap().resolve().block_on().unwrap());
                        assert_eq!(picked[0], picked[1], "{name} at {at:?}");
                        assert_eq!(
                            picked[0].is_some(),
                            c == Cover::Interior,
                            "{name} at {at:?}"
                        );
                    }
                }
                assert!(points > 500, "{name}: {points} points");
                r.end_frame();
            }
        }
        r.pipeline = shipped;
        println!("{report}");
    }
}
