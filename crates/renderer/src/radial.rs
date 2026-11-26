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
        reference_pipeline(r, "fs_per_sample")
    }
    /// The renderer's module with the test-support entries of
    /// `tests/support/radial_per_sample.wgsl` appended, drawn by `entry`.
    fn reference_pipeline(r: &RadialRenderer, entry: &str) -> wgpu::RenderPipeline {
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
                entry_point: Some(entry),
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
        // The supersampled draw that shades once per surface: the shipped
        // draw takes it wherever one ray per pixel cannot settle a pixel
        // (#454), and `one_ray_material_pixels_stay_within_the_pinned_difference`
        // compares the two.
        let restore = r.pipeline.clone();
        let shipped = reference_pipeline(&r, "fs_material_supersampled");
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
        r.pipeline = restore;
        println!("{report}");
    }

    /// The look of the gallery sunburst (`examples/support/looks.rs`
    /// `sunburst`): Flat, level, with a 0.6 degree gap and small corners.
    fn flat_look() -> crate::theme::RadialTheme {
        let mut theme = crate::theme::RadialTheme::default();
        theme.palette = vec![
            crate::Color::from_rgb8(38, 140, 112),
            crate::Color::from_rgb8(196, 112, 64),
            crate::Color::from_rgb8(128, 110, 196),
        ];
        theme.child_tint = Some(0.22);
        theme.label_color = crate::Color::from_rgb8(40, 44, 52);
        theme.material = Material::Flat;
        theme.gloss = 0.15;
        theme.gap_degrees = 0.6;
        theme.inner_corner = 0.012;
        theme.outer_corner = 0.012;
        // A sunburst stays level whatever the theme (`RuleKind::SunburstTilt`);
        // named so the chart takes the Flat path.
        theme.tilt_degrees = 0.0;
        theme.depth = 0.0;
        theme.bevel = 0.0;
        theme.shadow = false;
        theme
    }
    /// The gallery sunburst (`examples/radial_gallery.rs` `sunburst`), a
    /// donut and a pie of six slices in the host default style, and the
    /// Flat fast path's own cases.
    fn flat_charts() -> Vec<(&'static str, RadialChart)> {
        let leaf = |label: &str, value: f64| RadialSlice::new(label, value, crate::Color::BLACK);
        let node = |label: &str, value: f64, children: Vec<RadialSlice>| RadialSlice {
            children,
            ..leaf(label, value)
        };
        let theme = flat_look();
        let mut sunburst = RadialChart {
            kind: RadialKind::Donut { inner_radius: 0.14 },
            labels: RadialLabels::Outside,
            label_format: RadialLabelFormat::NamePercent,
            label_decimals: 0,
            slices: vec![
                node(
                    "Renewables",
                    46.0,
                    vec![
                        node(
                            "Wind",
                            20.0,
                            vec![leaf("Onshore", 13.0), leaf("Offshore", 7.0)],
                        ),
                        node(
                            "Solar",
                            12.0,
                            vec![leaf("Utility", 8.0), leaf("Rooftop", 4.0)],
                        ),
                        leaf("Hydro", 10.0),
                        leaf("Bio", 4.0),
                    ],
                ),
                node(
                    "Fossil",
                    38.0,
                    vec![
                        node(
                            "Gas",
                            26.0,
                            vec![leaf("Combined cycle", 21.0), leaf("Peakers", 5.0)],
                        ),
                        leaf("Coal", 9.0),
                        leaf("Oil", 3.0),
                    ],
                ),
                leaf("Nuclear", 16.0),
            ],
            ..Default::default()
        }
        .with_title("Electricity generation / group, source, technology");
        theme.apply(&mut sunburst);
        sunburst.format_labels();
        // The host default style: Flat, level, every slice outlined in a
        // 1 px white rim and white separators, no gap.
        let six = |kind| {
            let mut c = RadialChart {
                kind,
                labels: RadialLabels::Outside,
                label_format: RadialLabelFormat::NamePercent,
                slices: [31.0, 23.0, 20.0, 12.0, 8.0, 6.0]
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
        };
        let mut v = vec![
            ("gallery sunburst", sunburst),
            ("donut", six(RadialKind::Donut { inner_radius: 0.5 })),
            ("pie", six(RadialKind::Pie)),
        ];
        // The Flat fast path's cases (#41, `tests/radial_render.rs`): no
        // outline, many slices, a gap with rounded corners, explode with a
        // hover lift, moving inside labels on a translucent background and a
        // detail chart.
        let plain = |n: usize, kind: RadialKind| {
            let mut c = six(kind);
            c.labels = RadialLabels::None;
            c.style.outline.rim = false;
            c.style.outline.separators = false;
            c.slices = (0..n)
                .map(|i| {
                    let [r, g, b] = PALETTE[i % 4];
                    RadialSlice::new(
                        format!("S{i}"),
                        1.0 + (i * 7 % 5) as f64,
                        crate::Color::from_rgb8(r, g, b),
                    )
                })
                .collect();
            c.format_labels();
            c
        };
        let donut = RadialKind::Donut { inner_radius: 0.55 };
        v.push(("one slice", plain(1, RadialKind::Pie)));
        v.push(("three slices", plain(3, RadialKind::Pie)));
        v.push(("12 slices", plain(12, donut)));
        v.push(("40 slices", plain(40, donut)));
        let mut c = plain(5, donut);
        c.style.gap_degrees = 3.0;
        c.style.inner_corner = 0.05;
        c.style.outer_corner = 0.08;
        v.push(("gap and corners", c));
        let mut c = plain(6, RadialKind::Pie);
        c.slices[1].explode = 0.2;
        c.slices[4].explode = 0.35;
        c.interaction.hovered = Some(RadialTarget::main(2));
        c.interaction.hover_progress = 0.6;
        v.push(("explode and hover", c));
        let mut c = plain(6, donut);
        c.labels = RadialLabels::Inside;
        c.font_size = 12.0;
        c.style.hover_lift = 0.06;
        c.interaction.hovered = Some(RadialTarget::main(1));
        c.interaction.hover_progress = 1.0;
        c.background = crate::Color::new(0.1, 0.2, 0.3, 0.5);
        c.format_labels();
        v.push(("inside labels, translucent", c));
        // The detail chart splits a slice of 75 into 25 and 50.
        let mut c = plain(2, donut);
        c.slices[0].value = 25.0;
        c.slices[1].value = 75.0;
        c.split = Some(RadialSplit {
            slice_index: 1,
            kind: RadialKind::Pie,
            children: vec![
                RadialSlice::new("D", 25.0, crate::Color::from_rgb8(0, 200, 150)),
                RadialSlice::new("E", 50.0, crate::Color::from_rgb8(220, 180, 80)),
            ],
        });
        c.format_labels();
        v.push(("detail", c));
        v
    }

    fn export_with(
        r: &mut RadialRenderer,
        pipeline: &wgpu::RenderPipeline,
        chart: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> crate::RasterImage {
        r.pipeline = pipeline.clone();
        r.cached = None;
        let image = r
            .export_rgba(chart, size, crate::ExportOptions::document(scale))
            .block_on()
            .unwrap();
        r.end_frame();
        image
    }

    /// Analytic Flat edges change appearance only: against the 2 x 2
    /// supersampled draw they replace, the difference stays within a pinned
    /// mean and high percentile, it lies within a pixel of an edge, and
    /// every pixel farther from one, inside a sector or on the background,
    /// is byte-identical, as is the pick there.
    #[test]
    fn analytic_flat_edges_stay_within_the_pinned_difference() {
        let mut r = renderer();
        let reference = reference_pipeline(&r, "fs_supersampled");
        let shipped = r.pipeline.clone();
        let mut report = String::new();
        for (index, (name, chart)) in flat_charts().into_iter().enumerate() {
            // And an odd size at 2x, as the fast path's own test.
            for (size, scale) in [((600, 400), 1.0f32), ((600, 400), 2.0), ((333, 257), 2.0)] {
                let new = export_with(&mut r, &shipped, &chart, size, scale);
                let old = export_with(&mut r, &reference, &chart, size, scale);
                // Which pixels the old draw covers partly: its own coverage
                // mask, so the band is the edge it drew.
                let mask = export_with(&mut r, &reference, &coverage_chart(&chart), size, scale);
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
                // Within `r` pixels of an edge: its (2r + 1)-square
                // neighbourhood holds a partly covered pixel of the mask, or
                // two colours of the old draw (a seam between two sectors,
                // or an ink line).
                let pixel = |image: &crate::RasterImage, i: usize| {
                    <[u8; 4]>::try_from(&image.rgba[i * 4..i * 4 + 4]).unwrap()
                };
                let within = |i: usize, r: usize| {
                    let (x, y) = (i % w, i / w);
                    (y.saturating_sub(r)..=(y + r).min(h - 1)).any(|v| {
                        (x.saturating_sub(r)..=(x + r).min(w - 1)).any(|u| {
                            cover[v * w + u] == Cover::Edge
                                || pixel(&old, v * w + u) != pixel(&old, i)
                        })
                    })
                };
                let near_edge = |i: usize| within(i, 1);
                let mut differences: Vec<u8> = new
                    .rgba
                    .chunks_exact(4)
                    .zip(old.rgba.chunks_exact(4))
                    .map(|(a, b)| (0..4).map(|i| a[i].abs_diff(b[i])).max().unwrap())
                    .collect();
                // A pie's apex, where every separator's ink meets, is a
                // feature narrower than a pixel that no old sample reached:
                // the smoothed edge may tint a pixel next to it.
                let beyond_one = (0..w * h)
                    .filter(|&i| !near_edge(i))
                    .map(|i| differences[i])
                    .max()
                    .unwrap_or(0);
                let far_max = (0..w * h)
                    .filter(|&i| !within(i, 2))
                    .map(|i| differences[i])
                    .max()
                    .unwrap_or(0);
                let changed = differences.iter().filter(|&&d| d > 0).count();
                let mean = differences.iter().map(|&d| d as f64).sum::<f64>() / (w * h) as f64;
                differences.sort_unstable();
                let p99 = differences[(w * h * 99) / 100];
                let p999 = differences[(w * h * 999) / 1000];
                let max = differences[w * h - 1];
                report.push_str(&format!(
                    "{name} @{scale}x: mean {mean:.3}, p99 {p99}, p99.9 {p999}, max {max}, \
                     changed {changed} px, beyond a pixel of an edge {beyond_one}, \
                     beyond two {far_max}\n"
                ));
                // Measured on lavapipe for the gallery sunburst, the donut and
                // the pie: a mean of at most 0.18, a p99 of at most 3 and a
                // p99.9 of at most 31.
                assert!(
                    changed > 0,
                    "{name} @{scale}x: the Flat edges changed\n{report}"
                );
                assert_eq!(
                    far_max, 0,
                    "{name} @{scale}x: beyond two pixels of an edge\n{report}"
                );
                assert!(
                    beyond_one <= 12,
                    "{name} @{scale}x: beyond a pixel of an edge {beyond_one}\n{report}"
                );
                // The fast path's own cases hold more edge per area (40
                // slices on a small donut), so more of the image moves.
                let (mean_max, p99_max, p999_max) = if index < 3 {
                    (0.3, 5, 45)
                } else {
                    (0.5, 16, 60)
                };
                assert!(mean <= mean_max, "{name} @{scale}x: mean {mean}\n{report}");
                assert!(p99 <= p99_max, "{name} @{scale}x: p99 {p99}\n{report}");
                assert!(p999 <= p999_max, "{name} @{scale}x: p99.9 {p999}\n{report}");
                if scale != 1.0 {
                    continue;
                }
                // The pick, at points a pixel or more from any edge, is the
                // same from frames of both draws and finds a sector exactly
                // where the old draw covered one.
                let frames = [&shipped, &reference].map(|pipeline| {
                    r.pipeline = pipeline.clone();
                    r.cached = None;
                    r.prepare(&chart, size, scale).unwrap()
                });
                let mut points = 0;
                for y in (2..h - 2).step_by(11) {
                    for x in (2..w - 2).step_by(11) {
                        let i = y * w + x;
                        if near_edge(i) {
                            continue;
                        }
                        points += 1;
                        let at = [x as f32 + 0.5, y as f32 + 0.5];
                        let picked = frames
                            .each_ref()
                            .map(|f| f.pick(at).unwrap().resolve().block_on().unwrap());
                        assert_eq!(picked[0], picked[1], "{name} at {at:?}");
                        assert_eq!(
                            picked[0].is_some(),
                            cover[i] == Cover::Interior,
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

    /// Across a slanted separator the analytic edge ramps smoothly: on
    /// every row the profile rises monotonically from one slice into the
    /// separator and falls monotonically into the next, the edge pixels take
    /// many intermediate values where the 2 x 2 grid took at most its three
    /// quarter steps, and each row's edge stays where the old draw put it,
    /// within a pixel. Both separators: an angular gap on the background,
    /// and the host default's white ink line between touching slices.
    #[test]
    fn a_slanted_flat_separator_has_a_smooth_monotonic_edge_profile() {
        // Two black halves of a pie on white, 2 degrees off vertical: a
        // slightly slanted separator.
        let half = |gap: f32, outline: bool| {
            let mut chart = RadialChart {
                kind: RadialKind::Pie,
                labels: RadialLabels::None,
                start_angle_degrees: -88.0,
                background: crate::Color::WHITE,
                slices: vec![
                    RadialSlice::new("a", 1.0, crate::Color::BLACK),
                    RadialSlice::new("b", 1.0, crate::Color::BLACK),
                ],
                ..Default::default()
            };
            chart.style.material = Material::Flat;
            chart.style.gloss = 0.0;
            chart.style.gap_degrees = gap;
            chart.style.tilt_degrees = 0.0;
            chart.style.depth = 0.0;
            chart.style.shadow = false;
            chart.style.inner_corner = 0.0;
            chart.style.outer_corner = 0.0;
            chart.style.outline.rim = outline;
            chart.style.outline.separators = outline;
            chart.style.outline.emphasis = false;
            chart.style.outline.width = 1.0;
            chart.style.outline.color = crate::Color::WHITE;
            chart.format_labels();
            chart
        };
        let mut report = String::new();
        for (name, chart) in [
            ("3 degree gap", half(3.0, false)),
            ("1 px white separator", half(0.0, true)),
        ] {
            slanted_separator_profile(name, &chart, &mut report);
        }
        println!("{report}");
    }
    fn slanted_separator_profile(name: &str, chart: &RadialChart, report: &mut String) {
        let mut r = renderer();
        let reference = reference_pipeline(&r, "fs_supersampled");
        let shipped = r.pipeline.clone();
        for scale in [1.0f32, 2.0] {
            let images = [&shipped, &reference]
                .map(|pipeline| export_with(&mut r, pipeline, chart, (400, 400), scale));
            let (w, h) = (images[0].width as usize, images[0].height as usize);
            let red = |image: &crate::RasterImage, x: usize, y: usize| image.rgba[(y * w + x) * 4];
            // The column the gap is brightest at on a row, near the middle.
            let brightest = |image: &crate::RasterImage, y: usize| {
                (w / 2 - w / 8..w / 2 + w / 8)
                    .max_by_key(|&x| (red(image, x, y), usize::MAX - x.abs_diff(w / 2)))
                    .unwrap()
            };
            let mut levels = [
                std::collections::BTreeSet::new(),
                std::collections::BTreeSet::new(),
            ];
            let mut rows = 0;
            // The upper separator, from a quarter of the radius to near the
            // rim; the radius is a little under half the height.
            for y in h * 3 / 20..h * 7 / 20 {
                rows += 1;
                let mut edges = [0.0f64; 2];
                for (k, image) in images.iter().enumerate() {
                    let top = brightest(image, y);
                    let span = w / 10;
                    // Rising from the left slice to the brightest column.
                    for x in top - span..top {
                        assert!(
                            red(image, x, y) <= red(image, x + 1, y),
                            "{name} @{scale}x row {y}: not monotonic left of the gap at {x} ({k})"
                        );
                    }
                    for x in top..top + span {
                        assert!(
                            red(image, x, y) >= red(image, x + 1, y),
                            "{name} @{scale}x row {y}: not monotonic right of the gap at {x} ({k})"
                        );
                    }
                    assert_eq!(
                        red(image, top - span, y),
                        0,
                        "{name} @{scale}x row {y}: left slice"
                    );
                    assert_eq!(
                        red(image, top + span, y),
                        0,
                        "{name} @{scale}x row {y}: right slice"
                    );
                    for x in top - span..=top {
                        let v = red(image, x, y);
                        if v > 0 && v < 255 {
                            levels[k].insert(v);
                        }
                    }
                    // The left edge: the slice ends where its coverage
                    // ends, the summed brightness from the slice's interior.
                    edges[k] = (top - span..=top)
                        .map(|x| 1.0 - red(image, x, y) as f64 / 255.0)
                        .sum::<f64>();
                }
                assert!(
                    (edges[0] - edges[1]).abs() <= 1.0,
                    "{name} @{scale}x row {y}: the edge moved from {} to {}",
                    edges[1],
                    edges[0]
                );
            }
            report.push_str(&format!(
                "{name} @{scale}x: {rows} rows, intermediate levels new {}, old {}\n",
                levels[0].len(),
                levels[1].len()
            ));
            assert!(
                levels[1].len() <= 3,
                "{name} @{scale}x: the 2 x 2 grid steps in quarters\n{report}"
            );
            assert!(
                levels[0].len() >= 12,
                "{name} @{scale}x: the analytic edge ramps smoothly\n{report}"
            );
        }
        r.pipeline = shipped;
    }

    /// The gallery's material scenes (`examples/radial_gallery.rs`): the
    /// traffic donut in each material of the gallery look, the raised Flat
    /// pie with its rim and separators, sharp and rounded level corners, a
    /// slice edited by hand, the pie of pie and a hover lift; then the
    /// Ceramic donut of #349, the eleven-material donut, and the shading
    /// test's charts that take analytic edges (no depth, or level with no
    /// bevel), which the gallery has only twice.
    fn material_charts() -> Vec<(String, RadialChart)> {
        use crate::theme::RadialTheme;
        let slice = |label: &str, value: f64| RadialSlice::new(label, value, crate::Color::BLACK);
        let base = |theme: &RadialTheme| {
            let mut chart = RadialChart::default();
            chart.kind = RadialKind::Donut { inner_radius: 0.5 };
            chart.labels = RadialLabels::Outside;
            chart.label_format = RadialLabelFormat::NamePercent;
            chart.slices = vec![
                slice("Search", 40.0),
                slice("Direct", 25.0),
                slice("Email", 15.0),
                slice("Union", 12.0),
                slice("Video", 8.0),
            ];
            let mut chart = chart.with_title("Regional share");
            theme.apply(&mut chart);
            chart
        };
        let ceramic = crate::theme::ThemeFamily::Gallery.radial();
        let mut matte = ceramic.clone();
        matte.material = Material::Matte;
        let mut v = Vec::new();
        for (name, material) in [
            ("matte", Material::Matte),
            ("ceramic", Material::Ceramic),
            ("satin-metal", Material::SatinMetal),
            ("toon", Material::Toon),
            ("enamel", Material::Enamel),
            ("brushed-metal", Material::BrushedMetal),
            ("paper", Material::Paper),
            ("plywood", Material::Wood),
            ("hatch", Material::Hatch),
            ("pearl", Material::Pearl),
        ] {
            let mut theme = ceramic.clone();
            theme.material = material;
            if material == Material::Toon {
                theme.outline.rim = true;
                theme.outline.separators = true;
            }
            if matches!(
                material,
                Material::Hatch | Material::Pearl | Material::Paper | Material::Wood
            ) {
                theme.texture_strength = 0.7;
            }
            if material == Material::Wood {
                theme.roughness = 0.9;
                theme.gloss = 0.1;
            }
            v.push((format!("gallery {name}"), base(&theme)));
        }
        let mut flat = matte.clone();
        flat.material = Material::Flat;
        flat.depth = 0.3;
        flat.bevel = 0.0;
        flat.gap_degrees = 0.0;
        flat.inner_corner = 0.0;
        flat.outer_corner = 0.0;
        flat.outline.rim = true;
        flat.outline.separators = true;
        let mut pie = base(&flat);
        pie.kind = RadialKind::Pie;
        pie.slices = vec![
            slice("Control", 2.0),
            slice("Low", 3.0),
            slice("Medium", 4.0),
            slice("High", 5.0),
        ];
        flat.apply(&mut pie);
        v.push(("gallery outlined raised pie".into(), pie));
        for (name, amount) in [("sharp", 0.0), ("rounded", 0.12)] {
            let mut theme = matte.clone();
            theme.inner_corner = amount;
            theme.outer_corner = amount;
            theme.depth = 0.0;
            theme.tilt_degrees = 0.0;
            theme.shadow = false;
            v.push((format!("gallery corners {name}"), base(&theme)));
        }
        let mut edited = base(&matte);
        edited.interaction.selected = Some(RadialTarget::main(0));
        let mut style = edited.style.clone();
        style.material = Material::Ceramic;
        style.outline = RadialOutline::default();
        style.outline.emphasis = true;
        style.outline.width = 1.5;
        let first = edited.slice_mut(RadialTarget::main(0)).unwrap();
        first.color = crate::Color::from_rgb8(195, 80, 210);
        first.style = Some(style);
        v.push(("gallery individual edit".into(), edited));
        let mut split = base(&matte);
        split.labels = RadialLabels::Inside;
        split.label_format = RadialLabelFormat::Percent;
        split.split = Some(RadialSplit {
            slice_index: 4,
            kind: RadialKind::Donut { inner_radius: 0.4 },
            children: vec![
                RadialSlice::new("D", 2.0, crate::Color::from_rgb8(72, 176, 205)),
                RadialSlice::new("E", 6.0, crate::Color::from_rgb8(106, 218, 234)),
            ],
        });
        v.push(("gallery pie of pie".into(), split));
        let mut hover = base(&ceramic);
        for _ in 0..10 {
            hover
                .interaction
                .animate_hover(Some(RadialTarget::main(0)), 0.02, 0.2);
        }
        v.push(("gallery hover lift".into(), hover));
        for (name, chart) in charts() {
            if matches!(
                name,
                "ceramic" | "materials" | "flat shadow" | "ceramic depth 0" | "ceramic sunburst"
            ) {
                v.push((name.into(), chart));
            }
        }
        for (_, chart) in &mut v {
            chart.format_labels();
        }
        v
    }

    /// `chart` drawn as `coverage_chart` draws it, keeping its outline in an
    /// opaque green: a magenta pixel has every sample on a sector's fill, a
    /// green one every sample on its ink, a grey one every sample on the
    /// background, and any other is an edge (or a label).
    fn ink_mask_chart(chart: &RadialChart) -> RadialChart {
        let green = crate::Color::from_rgb8(0, 255, 0);
        let mut c = coverage_chart(chart);
        let keep = |mask: &mut RadialStyle, original: &RadialStyle| {
            mask.outline.rim = original.outline.rim;
            mask.outline.separators = original.outline.separators;
            mask.outline.emphasis = original.outline.emphasis;
            mask.outline.color = green;
        };
        keep(&mut c.style, &chart.style);
        fn slices(
            mask: &mut [RadialSlice],
            original: &[RadialSlice],
            keep: &dyn Fn(&mut RadialStyle, &RadialStyle),
        ) {
            for (m, o) in mask.iter_mut().zip(original) {
                if let (Some(m), Some(o)) = (m.style.as_mut(), o.style.as_ref()) {
                    keep(m, o);
                }
                slices(&mut m.children, &o.children, keep);
            }
        }
        slices(&mut c.slices, &chart.slices, &keep);
        if let (Some(m), Some(o)) = (c.split.as_mut(), chart.split.as_ref()) {
            slices(&mut m.children, &o.children, &keep);
        }
        c
    }

    /// One ray per material pixel (#454) changes appearance only: against
    /// the 2 x 2 supersampled draw it replaces (`fs_material_supersampled`),
    /// every pixel more than two pixels from an edge (of coverage, of ink,
    /// or between sectors) is byte-identical, the difference at edges stays
    /// within a pinned mean and 99th percentile, and the pick is the same
    /// from frames of both draws. Over the gallery's material scenes, the
    /// Ceramic donut and the eleven materials, at 600 x 400 at 1x and 2x and
    /// at 333 x 257 at 2x.
    #[test]
    fn one_ray_material_pixels_stay_within_the_pinned_difference() {
        #[derive(Clone, Copy, PartialEq)]
        enum Mask {
            Fill,
            Ink,
            Background,
            Edge,
        }
        let mut r = renderer();
        let reference = reference_pipeline(&r, "fs_material_supersampled");
        let shipped = r.pipeline.clone();
        let mut report = String::new();
        let analytic = [
            "gallery corners sharp",
            "gallery corners rounded",
            "flat shadow",
            "ceramic depth 0",
            "ceramic sunburst",
        ];
        for (name, chart) in material_charts() {
            for (size, scale) in [((600, 400), 1.0f32), ((600, 400), 2.0), ((333, 257), 2.0)] {
                let new = export_with(&mut r, &shipped, &chart, size, scale);
                let old = export_with(&mut r, &reference, &chart, size, scale);
                let mask = export_with(&mut r, &reference, &ink_mask_chart(&chart), size, scale);
                let (w, h) = (new.width as usize, new.height as usize);
                let mask: Vec<Mask> = mask
                    .rgba
                    .chunks_exact(4)
                    .map(|p| match p {
                        [255, 0, 255, 255] => Mask::Fill,
                        [0, 255, 0, 255] => Mask::Ink,
                        [r, g, b, 255] if r == g && g == b => Mask::Background,
                        _ => Mask::Edge,
                    })
                    .collect();
                // Within `r` pixels of an edge: the (2r + 1)-square around
                // the pixel holds an edge of the mask or another class.
                let within = |i: usize, r: usize| {
                    let (x, y) = (i % w, i / w);
                    (y.saturating_sub(r)..=(y + r).min(h - 1)).any(|v| {
                        (x.saturating_sub(r)..=(x + r).min(w - 1))
                            .any(|u| mask[v * w + u] == Mask::Edge || mask[v * w + u] != mask[i])
                    })
                };
                let mut differences: Vec<u8> = new
                    .rgba
                    .chunks_exact(4)
                    .zip(old.rgba.chunks_exact(4))
                    .map(|(a, b)| (0..4).map(|i| a[i].abs_diff(b[i])).max().unwrap())
                    .collect();
                let far_max = (0..w * h)
                    .filter(|&i| !within(i, 2))
                    .map(|i| differences[i])
                    .max()
                    .unwrap_or(0);
                let changed = differences.iter().filter(|&&d| d > 0).count();
                let mean = differences.iter().map(|&d| d as f64).sum::<f64>() / (w * h) as f64;
                differences.sort_unstable();
                let p99 = differences[(w * h * 99) / 100];
                let max = differences[w * h - 1];
                report.push_str(&format!(
                    "{name} {}x{} @{scale}x: mean {mean:.3}, p99 {p99}, max {max}, \
                     changed {changed} px, beyond two pixels of an edge {far_max}\n",
                    size.0, size.1
                ));
                assert_eq!(
                    far_max, 0,
                    "{name} @{scale}x: beyond two pixels of an edge\n{report}"
                );
                // Measured on lavapipe: the charts whose slices are flat
                // tops (no depth, or level with no bevel) take analytic
                // edges, a mean of at most 0.17 and a p99 of at most 3, as
                // the Flat edges of #439; every other chart settles its
                // pixels from one ray only where that ray gives the samples'
                // own colour, and draws the same bytes.
                if analytic.contains(&name.as_str()) {
                    assert!(changed > 0, "{name} @{scale}x: analytic edges\n{report}");
                } else {
                    assert_eq!(max, 0, "{name} @{scale}x: the same bytes\n{report}");
                }
                assert!(mean <= 0.3, "{name} @{scale}x: mean {mean}\n{report}");
                assert!(p99 <= 5, "{name} @{scale}x: p99 {p99}\n{report}");
                if scale != 1.0 {
                    continue;
                }
                // The pick, at points more than a pixel from any edge, is
                // the same from frames of both draws and finds a sector
                // exactly where the old draw covered one.
                let frames = [&shipped, &reference].map(|pipeline| {
                    r.pipeline = pipeline.clone();
                    r.cached = None;
                    r.prepare(&chart, size, scale).unwrap()
                });
                let mut points = 0;
                for y in (2..h - 2).step_by(11) {
                    for x in (2..w - 2).step_by(11) {
                        let i = y * w + x;
                        if within(i, 1) {
                            continue;
                        }
                        points += 1;
                        let at = [x as f32 + 0.5, y as f32 + 0.5];
                        let picked = frames
                            .each_ref()
                            .map(|f| f.pick(at).unwrap().resolve().block_on().unwrap());
                        assert_eq!(picked[0], picked[1], "{name} at {at:?}");
                        assert_eq!(
                            picked[0].is_some(),
                            mask[i] != Mask::Background,
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
