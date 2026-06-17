//! Public-facing wgpu renderer.
//!
//! `Renderer::try_new` builds every figgy GPU resource (pool, pipelines,
//! bind-group layouts, sampler, quad VB) in one call so users don't have to
//! wire `data_render::create_*_pipeline` / `ColumnPool::new` manually.
//!
//! Ownership: holds `Arc<wgpu::Device>` and `Arc<wgpu::Queue>` so the same
//! device can be shared with the host (egui_wgpu, iced_wgpu, etc.) without
//! lifetime gymnastics.
//!
//! `try_new` returns `Result<_, FiggyError>` so unsupported target formats and
//! device resource limits are reported before wgpu validation can abort.

use std::collections::HashMap;
use std::sync::Arc;

use crate::axis_render;
use crate::chart::Chart;
use crate::color::Color;
use crate::config::{Config, DrawStyle};
use crate::line::LineStylePreset;
use crate::data::ColumnSource;
use crate::data_config::{
    DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType, DataScatterStyleConfig,
    ErrorRef, SeriesConfig,
};
use crate::data_render::{
    self, AxisLayer, ColumnErrorBarDraw, ColumnHandle, ColumnId, ColumnLineLayer,
    ColumnPool, ColumnScatterLayer, DefragPolicy, PrimitiveStyle,
};
use crate::error::{FiggyError, Result};
use crate::layout::Rect;

/// A wgpu device/queue pair used by figgy.
///
/// wgpu does not expose a public parent-device id on `Queue`, so figgy cannot
/// prove at runtime that arbitrary handles came from the same logical device.
/// Passing them as one value keeps the pair together throughout the renderer
/// API and avoids accepting loose device/queue arguments at every call site.
#[derive(Clone)]
pub struct RendererDevice {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

impl RendererDevice {
    /// Bundle a wgpu device and queue that came from the same `request_device`
    /// result or host render state.
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        Self { device, queue }
    }

    pub fn device(&self) -> &Arc<wgpu::Device> { &self.device }
    pub fn queue(&self) -> &Arc<wgpu::Queue> { &self.queue }
}

#[derive(Clone, Copy, Debug)]
struct RendererDeviceCaps {
    features: wgpu::Features,
    max_texture_dimension_2d: u32,
    max_buffer_size: u64,
    /// Per-dimension dispatch ceiling for the arc-length compute scan.
    max_compute_workgroups_per_dimension: u32,
    /// The arc scan needs 256-wide workgroups; downlevel (GL-class)
    /// adapters report smaller/zero compute limits.
    max_compute_invocations_per_workgroup: u32,
}

impl RendererDeviceCaps {
    fn from_device(device: &wgpu::Device) -> Self {
        let limits = device.limits();
        Self {
            features: device.features(),
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            max_buffer_size: limits.max_buffer_size,
            max_compute_workgroups_per_dimension: limits.max_compute_workgroups_per_dimension,
            max_compute_invocations_per_workgroup: limits.max_compute_invocations_per_workgroup,
        }
    }
}

/// A dashed series' GPU arc-length prefix: the buffer (bound as the line
/// pipeline's vertex slots 4/5) and its used byte length.
type ArcPrefix = (Arc<wgpu::Buffer>, u64);

/// Facade bundling every figgy GPU resource.
pub struct Renderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    caps: RendererDeviceCaps,
    pool: ColumnPool,
    target_sample_count: u32,

    // Bind group layouts (exposed so callers can build per-panel bind groups).
    texture_bgl: wgpu::BindGroupLayout,
    transform_bgl: wgpu::BindGroupLayout,
    style_bgl: wgpu::BindGroupLayout,
    /// Constellation star-pass data layout (arc prefix + pool + offsets) —
    /// shared by the stars pipeline and every per-series star bind group.
    star_data_bgl: wgpu::BindGroupLayout,

    // Pipelines (precise + lazily-cached styled variants), all against
    // `surface_format`.
    pipelines: TargetPipelines,

    // Shared resources.
    sampler: wgpu::Sampler,
    quad_vb: wgpu::Buffer,

    /// Per-series GPU arc-scan state for dashed lines, keyed by series id.
    /// The prefix is re-dispatched on every draw that uses it (it depends on
    /// the data→pixel transform); buffers/bind groups are reused while the
    /// series layout (length, column offsets, pool generation) is stable.
    /// Mutated only in the `&mut self` prepare phase of `paint`/export — the
    /// renderer holds no locks; hosts that need shared access wrap the whole
    /// renderer (see the host-integration notes above `paint`).
    arc_cache: HashMap<String, data_render::line_arc::ArcScratch>,
    arc_pipelines: data_render::line_arc::ArcScanPipelines,
    /// Test-only narrowing of the arc-scan chunk size so the multi-chunk
    /// carry path is exercisable with small `n`. Always `None` in release.
    #[cfg(test)]
    arc_chunk_override: Option<u32>,

    /// Constellation deep-space backdrop cache. The backdrop depends only on
    /// the panel size and the (nebula, dust, seed) options — NOT on axis
    /// ranges — so pan/zoom/`set_config` refreshes reuse the baked bytes
    /// instead of re-running the fBm lattice over the whole panel (~40 ms at
    /// 2000×1600). Single slot: multi-panel hosts with differing sizes fall
    /// back to re-baking per refresh, same as before the cache. Mutated only
    /// in `&mut self` entry points, like `arc_cache`.
    space_bg: Option<SpaceBgCache>,

    surface_format: wgpu::TextureFormat,
}

/// Key + bytes of one baked constellation backdrop. `bake_gen` is a
/// monotonically increasing stamp; a `ChartView` whose grid texture already
/// holds this generation skips the (w·h·4)-byte re-upload too.
struct SpaceBgCache {
    key: SpaceBgKey,
    bake_gen: u64,
    rgba: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SpaceBgKey {
    w: u32,
    h: u32,
    nebula: u32,
    dust: u32,
    seed: u32,
}

fn validate_target_format(
    caps: RendererDeviceCaps,
    format: wgpu::TextureFormat,
) -> Result<()> {
    if !caps.features.contains(format.required_features()) {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "format requires device features that were not enabled".into(),
        });
    }
    let features = format.guaranteed_format_features(caps.features);
    if !features
        .allowed_usages
        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
    {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "format is not guaranteed to support RENDER_ATTACHMENT".into(),
        });
    }
    if !features
        .flags
        .contains(wgpu::TextureFormatFeatureFlags::BLENDABLE)
    {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "figgy pipelines require alpha blending into the target".into(),
        });
    }
    Ok(())
}

// Style descriptor table (docs/STYLE_REGISTRY.md §3). One descriptor fully
// describes a stylized render mode; the precise path is the absence of one
// (`style_variant` → `None`) and never routes through this table.

fn validate_target_sample_count(
    caps: RendererDeviceCaps,
    format: wgpu::TextureFormat,
    sample_count: u32,
) -> Result<()> {
    validate_target_format(caps, format)?;
    if sample_count == 1 {
        return Ok(());
    }
    let features = format.guaranteed_format_features(caps.features);
    if !features.flags.sample_count_supported(sample_count) {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: format!("format does not support {sample_count}x MSAA"),
        });
    }
    if !features
        .flags
        .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
    {
        return Err(FiggyError::UnsupportedSurfaceFormat {
            format,
            reason: "format does not support MSAA resolve targets".into(),
        });
    }
    Ok(())
}

fn preferred_msaa_sample_count(caps: RendererDeviceCaps, format: wgpu::TextureFormat) -> u32 {
    let features = format.guaranteed_format_features(caps.features);
    let can_resolve = features
        .flags
        .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE);
    if can_resolve && features.flags.sample_count_supported(4) {
        4
    } else if can_resolve && features.flags.sample_count_supported(2) {
        2
    } else {
        1
    }
}

/// Discriminant for cached per-style pipeline sets.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum StyleKey { Sketch, Milkyway, Constellation }

/// Everything the renderer needs to know about one stylized mode up front;
/// the per-format GPU objects live in the [`StyleSet`] the key maps to.
pub(crate) struct StyleVariant {
    pub(crate) key: StyleKey,
    pub(crate) needs_arc_prefix: bool,
    /// `Transform.style_params` packing — all three vec4 slots, layout per
    /// SHADER_COMMON.md §1.
    pub(crate) pack_params: fn(&DrawStyle) -> [f32; 12],
}

/// sketch: `[0] = (amplitude_px, wavelength_px, seed, 0)`, rest 0.
fn pack_sketch_params(style: &DrawStyle) -> [f32; 12] {
    match style.sketch() {
        Some(s) => {
            let mut p = [0.0; 12];
            p[0] = s.amplitude_px;
            p[1] = s.wavelength_px;
            p[2] = s.seed as f32;
            p
        }
        None => [0.0; 12],
    }
}

/// milkyway: `[0] = (star_density, ribbon_width_px, ribbon_intensity,
/// seed)`, `[1] = (star_scale, spread_px, faint_bias, planet_rim)`,
/// `[2] = (structure_scale, star_brightness, 0, 0)`.
/// `glow`/`nebula`/`dust` are CPU-raster parameters and never reach the GPU.
fn pack_milkyway_params(style: &DrawStyle) -> [f32; 12] {
    match style.milkyway() {
        Some(c) => [
            c.star_density, c.ribbon_width_px, c.ribbon_intensity, c.seed as f32,
            c.star_scale, c.spread_px, c.faint_bias, c.planet_rim,
            c.structure_scale, c.star_brightness, 0.0, 0.0,
        ],
        None => [0.0; 12],
    }
}

/// constellation: `[0] = (star_opacity, line_opacity, 0, 0)`,
/// `[1]`/`[2]` = 0.
/// Only ScatterLine series use this style.
fn pack_constellation_params(style: &DrawStyle) -> [f32; 12] {
    match style.constellation() {
        Some(c) => [
            c.star_opacity, c.line_opacity, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0,
        ],
        None => [0.0; 12],
    }
}

static SKETCH_VARIANT: StyleVariant = StyleVariant {
    key: StyleKey::Sketch,
    needs_arc_prefix: true,
    pack_params: pack_sketch_params,
};

static MILKYWAY_VARIANT: StyleVariant = StyleVariant {
    key: StyleKey::Milkyway,
    // Ribbon profile and every star attribute are arc-length parameterized.
    needs_arc_prefix: true,
    pack_params: pack_milkyway_params,
};

static CONSTELLATION_VARIANT: StyleVariant = StyleVariant {
    key: StyleKey::Constellation,
    needs_arc_prefix: false,
    pack_params: pack_constellation_params,
};

/// Style lookup. None for Precise.
pub(crate) fn style_variant(style: &DrawStyle) -> Option<&'static StyleVariant> {
    match style {
        DrawStyle::Precise => None,
        DrawStyle::Sketch(_) => Some(&SKETCH_VARIANT),
        DrawStyle::Milkyway(_) => Some(&MILKYWAY_VARIANT),
        DrawStyle::Constellation(_) => Some(&CONSTELLATION_VARIANT),
    }
}

/// One style's compiled GPU objects against one target format. Styles differ
/// structurally (sketch swaps entry points; milkyway draws additive star
/// passes and carries baked textures; constellation renders scatter stars
/// plus a connecting line), so the set is an enum and the draw side matches
/// once.
enum StyleSet {
    Sketch {
        line: wgpu::RenderPipeline,
        scatter: wgpu::RenderPipeline,
        errorbar: wgpu::RenderPipeline,
        line_verts: u32,
    },
    Milkyway(data_render::MilkywaySet),
    Constellation(data_render::PointConstellationSet),
}

/// Every pipeline compiled against one render-target format: the four
/// precise pipelines (created eagerly, as before) plus a lazy per-style
/// cache. A style's set is compiled the first time the prepare phase of
/// `paint`/export sees an item drawn in that style; charts that stay precise
/// never pay for any styled compile. Rebuilding for a new target format
/// starts from an empty cache — the next prepare phase recompiles on use.
struct TargetPipelines {
    axis: wgpu::RenderPipeline,
    line: wgpu::RenderPipeline,
    scatter: wgpu::RenderPipeline,
    errorbar: wgpu::RenderPipeline,
    sample_count: u32,
    styled: HashMap<StyleKey, StyleSet>,
}

fn create_target_pipelines(
    device: &wgpu::Device,
    texture_bgl: &wgpu::BindGroupLayout,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
) -> TargetPipelines {
    TargetPipelines {
        axis: data_render::create_fullscreen_textured_pipeline_with_sample_count(
            device, texture_bgl, surface_format, sample_count,
        ),
        line: data_render::create_line_columnar_pipeline_with_sample_count(
            device, transform_bgl, style_bgl, surface_format, sample_count,
        ),
        scatter: data_render::create_scatter_columnar_pipeline_with_sample_count(
            device, transform_bgl, style_bgl, surface_format, sample_count,
        ),
        errorbar: data_render::create_errorbar_columnar_pipeline_with_sample_count(
            device, transform_bgl, style_bgl, surface_format, sample_count,
        ),
        sample_count,
        styled: HashMap::new(),
    }
}

impl TargetPipelines {
    /// Prepare-phase ensure (`&mut`): compile and cache the pipeline set of
    /// every style that `items` draw with and the cache doesn't hold yet.
    /// Run before pass recording so the draw phase (`&self`) only looks up.
    #[allow(clippy::too_many_arguments)]
    fn ensure_styles_for_items(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        star_data_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        items: &[ChartDrawItem<'_>],
    ) {
        for item in items {
            let Some(v) = style_variant(&item.chart_config.draw_style) else { continue };
            self.styled.entry(v.key).or_insert_with(|| match v.key {
                StyleKey::Sketch => StyleSet::Sketch {
                    line: data_render::create_line_columnar_pipeline_with_entries(
                        device, transform_bgl, style_bgl, surface_format,
                        self.sample_count,
                        "vs_sketch", "fs_main",
                        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
                        wgpu::PrimitiveTopology::TriangleStrip,
                        None,
                        "figgy line styled pipeline",
                    ),
                    scatter: data_render::create_scatter_columnar_pipeline_with_entries(
                        device, transform_bgl, style_bgl, surface_format,
                        self.sample_count,
                        "vs_sketch", "fs_sketch",
                        "figgy scatter styled pipeline",
                    ),
                    errorbar: data_render::create_errorbar_columnar_pipeline_with_entries(
                        device, transform_bgl, style_bgl, surface_format,
                        self.sample_count,
                        "vs_sketch", "figgy errorbar styled pipeline",
                    ),
                    line_verts: data_render::LINE_SKETCH_VERTICES_PER_INSTANCE,
                },
                // Bakes the milkyway PSF + blackbody textures (once, then cached with
                // the pipelines) — docs/CONSTELLATION_DESIGN.md §3c.
                StyleKey::Milkyway => StyleSet::Milkyway(
                    data_render::create_milkyway_set(
                        device, queue, transform_bgl, style_bgl, star_data_bgl,
                        surface_format, self.sample_count,
                    ),
                ),
                StyleKey::Constellation => StyleSet::Constellation(
                    data_render::create_point_constellation_set(
                        device, queue, transform_bgl, style_bgl, surface_format,
                        self.sample_count,
                    ),
                ),
            });
        }
    }

    /// Draw-phase lookup: the cached set for the item's style, `None` for
    /// precise mode. A cache miss (impossible once the prepare phase ran)
    /// also yields `None` — callers then fall back to the precise pipelines
    /// instead of panicking.
    fn style_set(&self, style: &DrawStyle) -> Option<&StyleSet> {
        style_variant(style).and_then(|v| self.styled.get(&v.key))
    }
}

fn validate_texture_extent(
    caps: RendererDeviceCaps,
    resource: &'static str,
    width: u32,
    height: u32,
) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(FiggyError::InvalidChartArea { width, height });
    }
    let max_dim = width.max(height);
    if max_dim > caps.max_texture_dimension_2d {
        return Err(FiggyError::GpuResourceLimit {
            resource,
            requested: max_dim as u64,
            limit: caps.max_texture_dimension_2d as u64,
        });
    }
    Ok(())
}

fn validate_buffer_size(
    caps: RendererDeviceCaps,
    resource: &'static str,
    size: u64,
) -> Result<()> {
    if size > caps.max_buffer_size {
        return Err(FiggyError::GpuResourceLimit {
            resource,
            requested: size,
            limit: caps.max_buffer_size,
        });
    }
    Ok(())
}

fn create_texture_checked(
    device: &wgpu::Device,
    desc: &wgpu::TextureDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Texture> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.create_texture(desc)))
        .map_err(|_| FiggyError::GpuResourceAllocationFailed {
            resource,
            reason: "wgpu Device::create_texture panicked".into(),
        })
}

fn create_buffer_checked(
    device: &wgpu::Device,
    desc: &wgpu::BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Buffer> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.create_buffer(desc)))
        .map_err(|_| FiggyError::GpuResourceAllocationFailed {
            resource,
            reason: "wgpu Device::create_buffer panicked".into(),
        })
}

struct MsaaTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

fn create_msaa_target(
    device: &wgpu::Device,
    caps: RendererDeviceCaps,
    label: &'static str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    sample_count: u32,
) -> Result<Option<MsaaTarget>> {
    if sample_count <= 1 {
        return Ok(None);
    }
    validate_target_sample_count(caps, format, sample_count)?;
    validate_texture_extent(caps, label, width, height)?;
    let desc = wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    };
    let texture = create_texture_checked(device, &desc, label)?;
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(Some(MsaaTarget { _texture: texture, view }))
}

impl Renderer {
    /// Initialize every figgy GPU resource against the given device/queue pair.
    ///
    /// `pool_capacity_bytes` is the total size of the GPU column pool
    /// (sum of all chart data). `surface_format` is the final render target
    /// color format; every graphics pipeline is compiled against it.
    pub fn try_new(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
    ) -> Result<Self> {
        Self::try_new_with_sample_count(gpu, surface_format, pool_capacity_bytes, 1)
    }

    fn try_new_with_sample_count(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
        target_sample_count: u32,
    ) -> Result<Self> {
        let RendererDevice { device, queue } = gpu;
        let caps = RendererDeviceCaps::from_device(&device);
        // Downlevel (GL-class) adapters report compute limits below what the
        // dashed-line arc scan needs; creating its pipelines there is a
        // validation panic. Refuse early with a real error instead.
        if caps.max_compute_invocations_per_workgroup < data_render::line_arc::WG
            || caps.max_compute_workgroups_per_dimension == 0
        {
            return Err(FiggyError::GpuResourceLimit {
                resource: "compute workgroup (figgy requires WebGPU-class compute; \
                           GL-downlevel adapters are not supported)",
                requested: u64::from(data_render::line_arc::WG),
                limit: u64::from(caps.max_compute_invocations_per_workgroup),
            });
        }
        validate_target_sample_count(caps, surface_format, target_sample_count)?;

        let pool = ColumnPool::new(&device, pool_capacity_bytes)?;

        let texture_bgl = data_render::create_texture_bind_group_layout(&device);
        let transform_bgl = data_render::create_scatter_transform_bind_group_layout(&device);
        let style_bgl = data_render::create_style_bind_group_layout(&device);
        let star_data_bgl = data_render::create_star_data_bind_group_layout(&device);

        let sampler = data_render::create_linear_sampler(&device);
        let quad_vb = data_render::create_unit_centered_quad_vertex_buffer(&device);

        // Precise pipelines compile eagerly; styled variants compile lazily
        // in the prepare phase of the first `paint`/export that uses them.
        let pipelines = create_target_pipelines(
            &device,
            &texture_bgl,
            &transform_bgl,
            &style_bgl,
            surface_format,
            target_sample_count,
        );
        let arc_pipelines = data_render::line_arc::create_arc_scan_pipelines(&device);

        Ok(Self {
            device,
            queue,
            caps,
            pool,
            target_sample_count,
            texture_bgl,
            transform_bgl,
            style_bgl,
            star_data_bgl,
            pipelines,
            sampler,
            quad_vb,
            arc_cache: HashMap::new(),
            arc_pipelines,
            #[cfg(test)]
            arc_chunk_override: None,
            space_bg: None,
            surface_format,
        })
    }

    // Handle / accessor methods.

    pub fn device(&self) -> &Arc<wgpu::Device> { &self.device }
    pub fn queue(&self) -> &Arc<wgpu::Queue> { &self.queue }
    pub fn pool(&self) -> &ColumnPool { &self.pool }
    pub fn surface_format(&self) -> wgpu::TextureFormat { self.surface_format }

    /// Rebuild render-target pipelines if the host swap-chain format changed.
    ///
    /// Column data, chart views, textures, bind groups, styles, and buffers do
    /// not depend on the final color attachment format, so embedders can call
    /// this instead of tearing down the whole renderer state.
    pub fn ensure_target_format(&mut self, surface_format: wgpu::TextureFormat) -> Result<bool> {
        self.ensure_target(surface_format, self.target_sample_count)
    }

    fn ensure_target(
        &mut self,
        surface_format: wgpu::TextureFormat,
        target_sample_count: u32,
    ) -> Result<bool> {
        if self.surface_format == surface_format
            && self.target_sample_count == target_sample_count
        {
            return Ok(false);
        }
        validate_target_sample_count(self.caps, surface_format, target_sample_count)?;
        // Fresh set with an empty styled cache — any styled pipelines the
        // next frame needs are recompiled by its prepare phase.
        self.pipelines = create_target_pipelines(
            &self.device,
            &self.texture_bgl,
            &self.transform_bgl,
            &self.style_bgl,
            surface_format,
            target_sample_count,
        );
        self.surface_format = surface_format;
        self.target_sample_count = target_sample_count;
        Ok(true)
    }

    /// Block until submitted work on this device has completed.
    ///
    /// This is useful during shutdown in embedder examples, where dropping
    /// callback-owned GPU resources after the host surface/window has started
    /// tearing down can expose backend or driver lifetime bugs.
    ///
    /// No-op on wasm: the browser drives the device, blocking waits don't
    /// exist there, and the shutdown races this guards against are native
    /// driver concerns.
    pub fn wait_idle(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = self.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
        }
    }

    pub fn texture_bind_group_layout(&self) -> &wgpu::BindGroupLayout { &self.texture_bgl }
    pub fn transform_bind_group_layout(&self) -> &wgpu::BindGroupLayout { &self.transform_bgl }
    pub fn style_bind_group_layout(&self) -> &wgpu::BindGroupLayout { &self.style_bgl }

    pub fn axis_pipeline(&self) -> &wgpu::RenderPipeline { &self.pipelines.axis }
    pub fn line_pipeline(&self) -> &wgpu::RenderPipeline { &self.pipelines.line }
    pub fn scatter_pipeline(&self) -> &wgpu::RenderPipeline { &self.pipelines.scatter }
    pub fn errorbar_pipeline(&self) -> &wgpu::RenderPipeline { &self.pipelines.errorbar }

    pub fn sampler(&self) -> &wgpu::Sampler { &self.sampler }
    pub fn quad_vertex_buffer(&self) -> &wgpu::Buffer { &self.quad_vb }

    // Column management (returns Result).

    /// Add a column to the pool. `AllocError` is converted to `FiggyError::Pool`.
    pub fn add_column(
        &mut self,
        id: impl Into<ColumnId>,
        source: &dyn ColumnSource,
    ) -> Result<ColumnHandle> {
        Ok(self.pool.add_column(id.into(), source, &self.device, &self.queue)?)
    }

    pub fn remove_column(&mut self, id: &str) -> bool {
        self.pool.remove_column(id)
    }

    /// Compact every live column to the start of the pool. `true` iff
    /// anything actually moved.
    pub fn defragment(&mut self) -> Result<bool> {
        Ok(self.pool.defragment(&self.device, &self.queue)?)
    }

    pub fn set_defrag_policy(&mut self, policy: DefragPolicy) {
        self.pool.defrag_policy = policy;
    }

    pub fn handle_for(&self, id: &str) -> Result<ColumnHandle> {
        self.pool
            .handle_for(id)
            .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_string() })
    }

    pub fn is_valid_handle(&self, h: &ColumnHandle) -> bool {
        self.pool.is_valid_handle(h)
    }

    // Standalone path — figgy owns instance / adapter / device / queue / surface.
    //
    // For hosts where figgy renders the window directly (e.g. winit). Caller
    // writes zero lines of wgpu setup code.

    /// Build every GPU resource plus a surface and swap chain from a window
    /// handle. The returned [`WindowedRenderer`] manages the surface and
    /// expects `draw` to be called once per frame.
    ///
    /// `target` can be anything convertible to `wgpu::SurfaceTarget`
    /// (`Arc<winit::Window>`, a raw-handle wrapper, a web canvas, …). The
    /// figgy crate does not depend on winit types itself. On wasm, drive this
    /// from the host event loop (e.g. `wasm_bindgen_futures::spawn_local`).
    pub async fn for_window_async<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
    ) -> Result<WindowedRenderer<'w>> {
        let instance = data_render::create_instance();
        let surface = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            data_render::create_surface_for_window(&instance, target)
        }))
        .map_err(|_| FiggyError::SurfaceCreationFailed {
            reason: "wgpu surface creation panicked; check platform window/canvas/thread constraints"
                .into(),
        })?
        .map_err(|e| FiggyError::SurfaceCreationFailed { reason: format!("{e}") })?;
        let adapter = data_render::request_adapter_for_surface_async(&instance, &surface)
            .await
            .map_err(|_| FiggyError::AdapterUnavailable)?;
        let (device, queue) = data_render::request_device_async(&adapter)
            .await
            .map_err(|e| FiggyError::DeviceCreationFailed { reason: format!("{e}") })?;
        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let surface_config = data_render::try_configure_surface(
            &surface, &adapter, &device, size.0.max(1), size.1.max(1),
        )?;

        let caps = RendererDeviceCaps::from_device(&device);
        let preferred_sample_count = preferred_msaa_sample_count(caps, surface_config.format);
        let (target_sample_count, msaa_target) = match create_msaa_target(
            &device,
            caps,
            "figgy frame msaa target",
            surface_config.width,
            surface_config.height,
            surface_config.format,
            preferred_sample_count,
        ) {
            Ok(target) => (preferred_sample_count, target),
            Err(_) if preferred_sample_count > 1 => (1, None),
            Err(e) => return Err(e),
        };

        let inner = Renderer::try_new_with_sample_count(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            surface_config.format,
            pool_capacity_bytes,
            target_sample_count,
        )?;

        Ok(WindowedRenderer {
            inner,
            surface,
            surface_config,
            msaa_target,
            _instance: instance,
            _adapter: adapter,
        })
    }

    /// Blocking convenience wrapper around [`Self::for_window_async`]. Native
    /// only — on wasm, blocking the single thread would deadlock.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn for_window<'w>(
        target: impl Into<wgpu::SurfaceTarget<'w>>,
        size: (u32, u32),
        pool_capacity_bytes: u64,
    ) -> Result<WindowedRenderer<'w>> {
        pollster::block_on(Self::for_window_async(target, size, pool_capacity_bytes))
    }

    // Host-integration API (egui_wgpu / iced_wgpu / standalone).
    //
    // `Renderer` does NOT own the surface, swap chain, or event loop — the
    // host (egui, iced, a wgpu app) manages those. Per-frame flow:
    //   1. Host calls `surface.get_current_texture()` or enters its paint
    //      callback.
    //   2. If needed, the host invokes `update_transform` / `refresh_axis`
    //      after `chart.consume_data_dirty` / `consume_raster_dirty` — this
    //      is the "prepare" stage where `device`/`queue` are accessible.
    //   3. The host hands a `&mut RenderPass` to `renderer.paint(pass, items)`.
    //
    // Locking is the HOST's responsibility. `paint` takes `&mut self` (it
    // refreshes per-series GPU scan state); the renderer itself holds no
    // locks. Hosts that own the renderer exclusively (winit loop, wasm
    // wrapper) call it directly. Hosts whose paint callback only hands out
    // `&self` (egui's `CallbackTrait::paint`, iced's `shader::Primitive`)
    // wrap their state in a `Mutex` and lock around the call — see
    // `examples/egui_embed.rs` / `examples/iced_embed.rs`.
    //
    // egui pseudocode:
    // ```ignore
    // // resources own Mutex<FiggyState> (state.renderer + chart/view/items)
    // let cb = egui_wgpu::CallbackFn::new()
    //     .prepare(|device, queue, _enc, res| {
    //         let s = res.get_mut::<Mutex<FiggyState>>().unwrap().get_mut().unwrap();
    //         if s.chart.consume_data_dirty() { s.renderer.update_transform(&s.view, &s.chart); }
    //         if s.chart.consume_raster_dirty() { s.renderer.refresh_axis(&mut s.view, &s.chart, rect)?; }
    //         vec![]
    //     })
    //     .paint(|_info, pass, res| {
    //         let mut s = res.get::<Mutex<FiggyState>>().unwrap().lock().unwrap();
    //         let (renderer, items) = s.split_for_paint();
    //         renderer.paint(pass, target_size, &items).unwrap();
    //     });
    // ```

    /// Build a `ChartStyle` from `cfg.render_type`'s line/scatter/errorbar
    /// styles. Missing components default to BLACK / 1 px.
    pub fn create_style_for_series(&self, cfg: &SeriesConfig) -> ChartStyle {
        self.create_style_for_series_scaled(cfg, 1.0)
    }

    /// Scaled variant of `create_style_for_series` — every pixel size (line
    /// width, dash lengths, point radius, errorbar widths/cap) is multiplied
    /// by `scale` (used by the high-DPI export path).
    pub fn create_style_for_series_scaled(&self, cfg: &SeriesConfig, scale: f32) -> ChartStyle {
        // Decorrelates per-series hash patterns in the styled shader entries
        // (star placement, sketch wobble). Derived from the series id, so the
        // result stays deterministic for a given config; renaming a series
        // re-rolls its pattern.
        let series_salt = crate::sketch::fnv1a(&cfg.series_id);
        let line = match extract_line(&cfg.render_type) {
            Some(l) => {
                let mut s =
                    PrimitiveStyle::from_color_with_width(l.line_color, l.line_width * scale);
                let pattern = l.line_style.pattern();
                // GPU dash capacity is 8 scalars (2 × vec4); presets max out at 6.
                for (i, len) in pattern.iter().take(8).enumerate() {
                    s.dash[i / 4][i % 4] = len * scale;
                }
                s.dash_len = pattern.len().min(8) as u32;
                s
            }
            None => PrimitiveStyle::from_color_with_width(Color::BLACK, 1.0),
        };
        let scatter = match extract_scatter(&cfg.render_type) {
            Some(sc) => {
                let mut s = PrimitiveStyle::from_color(sc.point_color);
                s.point_radius_px = sc.point_size * scale;
                s.shape_id = data_render::shape_id(&sc.point_shape);
                s
            }
            None => PrimitiveStyle::from_color(Color::BLACK),
        };
        let errorbar = match extract_errorbar_style(&cfg.render_type) {
            Some(e) => {
                let mut s = PrimitiveStyle::from_color_with_width(
                    e.error_bar_color,
                    e.error_bar_width * scale,
                );
                s.cap_half_px = e.error_bar_cap_size * scale;
                s.cap_width_px = e.cap_width * scale;
                s
            }
            None => PrimitiveStyle::from_color(Color::BLACK),
        };
        let (mut line, mut scatter, mut errorbar) = (line, scatter, errorbar);
        line.series_salt = series_salt;
        scatter.series_salt = series_salt;
        errorbar.series_salt = series_salt;
        self.create_style_from_primitives(line, scatter, errorbar)
    }

    /// Shared tail of `create_style_for_series*`: allocate the three style
    /// uniform buffers and bind groups from fully-built values.
    fn create_style_from_primitives(
        &self,
        line: PrimitiveStyle,
        scatter: PrimitiveStyle,
        errorbar: PrimitiveStyle,
    ) -> ChartStyle {
        let dev = &self.device;
        let line_buf = data_render::create_style_uniform_buffer(dev, &line);
        let sc_buf = data_render::create_style_uniform_buffer(dev, &scatter);
        let eb_buf = data_render::create_style_uniform_buffer(dev, &errorbar);
        ChartStyle {
            line_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &line_buf),
            scatter_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &sc_buf),
            errorbar_bg: data_render::create_style_bind_group(dev, &self.style_bgl, &eb_buf),
        }
    }

    /// Build the per-panel GPU resources: grid + decoration textures and
    /// bind groups, plus the transform uniform buffer + bind group.
    /// `panel_rect` is the panel's pixel rect in surface coordinates.
    /// `chart.config_mut().chart_area` must match `panel_rect` before this
    /// call so the axis raster is drawn correctly.
    pub fn create_chart_view(
        &self,
        chart: &Chart,
        panel_rect: Rect,
    ) -> Result<ChartView> {
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);

        // Grid layer (drawn below data): raster + texture + bind group.
        let grid_rgba = axis_render::try_raster_chart_layer_to_rgba(
            chart.config(), axis_render::AxisLayerKind::Grid,
        )?;
        let grid_tex = data_render::upload_rgba_texture(
            &self.device,
            &self.queue,
            self.caps.max_texture_dimension_2d,
            w,
            h,
            &grid_rgba,
        )?;
        let grid_view_t = grid_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let grid_bg = data_render::create_texture_bind_group(
            &self.device, &self.texture_bgl, &grid_view_t, &self.sampler,
        );

        // Decoration layer (drawn above data).
        let dec_rgba = axis_render::try_raster_chart_layer_to_rgba(
            chart.config(), axis_render::AxisLayerKind::Decoration,
        )?;
        let dec_tex = data_render::upload_rgba_texture(
            &self.device,
            &self.queue,
            self.caps.max_texture_dimension_2d,
            w,
            h,
            &dec_rgba,
        )?;
        let dec_view_t = dec_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let dec_bg = data_render::create_texture_bind_group(
            &self.device, &self.texture_bgl, &dec_view_t, &self.sampler,
        );

        let t = data_render::scatter_transform_from_config(chart.config());
        let transform_buffer = data_render::create_scatter_transform_uniform_buffer(&self.device, &t);
        let transform_bg = data_render::create_scatter_transform_bind_group(
            &self.device, &self.transform_bgl, &transform_buffer,
        );

        Ok(ChartView {
            grid_texture: grid_tex,
            grid_bind_group: grid_bg,
            decoration_texture: dec_tex,
            decoration_bind_group: dec_bg,
            transform_buffer,
            transform_bg,
            panel_rect,
            grid_space_gen: None,
        })
    }

    /// When only the axis range (`AxisOptions { scale, min, max }`) changes,
    /// only the transform uniform buffer needs to be rewritten. No CPU
    /// raster, no texture rebuild — one GPU write.
    pub fn update_transform(&self, view: &ChartView, chart: &Chart) {
        let t = data_render::scatter_transform_from_config(chart.config());
        data_render::update_scatter_transform(&self.queue, &view.transform_buffer, &t);
    }

    /// Re-rasterize both grid and decoration textures. Updates in place via
    /// `write_texture` when the size matches; allocates new textures
    /// otherwise. Also refreshes the transform UB (chart_area feeds it).
    ///
    /// `&mut self`: the constellation backdrop cache (`space_bg`) lives on
    /// the renderer and may be (re)baked here — same prepare-phase mutability
    /// rule as `paint`.
    pub fn refresh_axis(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
    ) -> Result<()> {
        self.refresh_axis_with_selection(view, chart, panel_rect, &[])
    }

    /// [`Self::refresh_axis`] plus selection highlight boxes composited into
    /// the decoration layer (selection draws above the data, never below).
    /// `selection` comes from `Selectable::selection_box` / `HitMap` and is in
    /// absolute surface coordinates.
    pub fn refresh_axis_with_selection(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selection: &[crate::select::SelectionBox],
    ) -> Result<()> {
        let w = panel_rect.width.max(1);
        let h = panel_rect.height.max(1);

        let dec_rgba = axis_render::try_raster_chart_layer_to_rgba_with_selection(
            chart.config(), axis_render::AxisLayerKind::Decoration, selection,
        )?;

        // Grid layer. The constellation backdrop is axis-range independent,
        // so it is cached by (size, nebula, dust, seed): a pan/zoom/config
        // commit re-rasters only the decoration layer, and a view whose grid
        // texture already holds the cached generation skips the upload too.
        if let crate::config::DrawStyle::Milkyway(c) = &chart.config().draw_style {
            let key = SpaceBgKey {
                w,
                h,
                nebula: c.nebula.to_bits(),
                dust: c.dust.to_bits(),
                seed: c.seed,
            };
            if !self.space_bg.as_ref().is_some_and(|s| s.key == key) {
                let rgba = axis_render::try_raster_chart_layer_to_rgba(
                    chart.config(), axis_render::AxisLayerKind::Grid,
                )?;
                let bake_gen = self.space_bg.as_ref().map_or(1, |s| s.bake_gen + 1);
                self.space_bg = Some(SpaceBgCache { key, bake_gen, rgba });
            }
            let cache = self.space_bg.as_ref().expect("space_bg baked above");
            if view.grid_space_gen != Some(cache.bake_gen) {
                self.refresh_one_layer(
                    &mut view.grid_texture, &mut view.grid_bind_group,
                    &cache.rgba, w, h,
                )?;
                view.grid_space_gen = Some(cache.bake_gen);
            }
        } else {
            let grid_rgba = axis_render::try_raster_chart_layer_to_rgba(
                chart.config(), axis_render::AxisLayerKind::Grid,
            )?;
            self.refresh_one_layer(
                &mut view.grid_texture, &mut view.grid_bind_group,
                &grid_rgba, w, h,
            )?;
            view.grid_space_gen = None;
        }

        self.refresh_one_layer(
            &mut view.decoration_texture, &mut view.decoration_bind_group,
            &dec_rgba, w, h,
        )?;
        view.panel_rect = panel_rect;

        self.update_transform(view, chart);
        Ok(())
    }

    /// Update one texture in place or by reallocation. Used for both
    /// the grid and decoration layers.
    fn refresh_one_layer(
        &self,
        tex: &mut wgpu::Texture,
        bg: &mut wgpu::BindGroup,
        rgba: &[u8],
        w: u32, h: u32,
    ) -> Result<()> {
        let same_size = tex.width() == w && tex.height() == h;
        if same_size {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex, mip_level: 0,
                    origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h),
                },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        } else {
            let new_tex = data_render::upload_rgba_texture(
                &self.device,
                &self.queue,
                self.caps.max_texture_dimension_2d,
                w,
                h,
                rgba,
            )?;
            let new_view_t = new_tex.create_view(&wgpu::TextureViewDescriptor::default());
            let new_bg = data_render::create_texture_bind_group(
                &self.device, &self.texture_bgl, &new_view_t, &self.sampler,
            );
            *tex = new_tex;
            *bg = new_bg;
        }
        Ok(())
    }

    /// Draw multiple chart panels into one RenderPass.
    ///
    /// Takes a pass already opened by the host via `begin_render_pass`.
    /// `Renderer` owns the pool buffer, pipelines, and bind groups, so no
    /// external resources are needed.
    ///
    /// Returns `FiggyError::UnknownColumn` if a series references an unknown
    /// column id (or a stale handle).
    ///
    /// `target_size` is the color attachment pixel size of the current pass;
    /// panel_rect / data_area are clamped to it because wgpu validation would
    /// otherwise abort. Pass the surface pixel size reported by the host
    /// (winit, egui CallbackInfo, iced shader::Primitive, …).
    ///
    /// Takes `&mut self`: the call starts with a prepare phase that compiles
    /// any styled pipeline sets the items need but the cache lacks, and
    /// refreshes per-series GPU scan state (dashed-line arc prefixes) before
    /// recording into the pass. The renderer holds no internal locks — hosts
    /// whose paint callback only provides `&self` wrap their state in a
    /// `Mutex` (see the host-integration notes above).
    pub fn paint(
        &mut self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        items: &[ChartDrawItem<'_>],
    ) -> Result<()> {
        self.pipelines.ensure_styles_for_items(
            &self.device,
            &self.queue,
            &self.transform_bgl,
            &self.style_bgl,
            &self.star_data_bgl,
            self.surface_format,
            items,
        );
        let arcs = self.ensure_arc_prefixes(items);
        self.paint_with_pipelines(pass, target_size, items, &self.pipelines, &arcs)
    }

    /// Arc half of the prepare phase of `paint`/export (the styled-pipeline
    /// ensure is the other half). Ensures and dispatches the GPU arc-length
    /// prefix for every line series that needs one — dashed lines (dash
    /// phase) and every line of a style with `needs_arc_prefix` (sketch: the
    /// wobble is parameterized by arc length, so solid sketch lines need the
    /// prefix too). The immutable draw phase then only looks the buffers up,
    /// so it can coexist with the pipeline references the pass needs.
    fn ensure_arc_prefixes(&mut self, items: &[ChartDrawItem<'_>]) -> Vec<Vec<Option<ArcPrefix>>> {
        let mut arcs = Vec::with_capacity(items.len());
        for item in items {
            let variant = style_variant(&item.chart_config.draw_style);
            // Milkyway lines also get the arc-driven star pass: the
            // candidate-slot pitch the indirect kernel sizes the dispatch
            // with (the star VS derives the same value from the transform).
            let star_pitch = match &item.chart_config.draw_style {
                crate::config::DrawStyle::Milkyway(c) => Some(
                    (data_render::line_arc::STAR_SLOT_PITCH_FACTOR * c.structure_scale)
                        .max(1e-3),
                ),
                _ => None,
            };
            let mut per_series = Vec::with_capacity(item.series.len());
            for series in item.series {
                let cfg = series.config;
                let line = has_line(&cfg.render_type);
                let dashed = line
                    && extract_line(&cfg.render_type)
                        .is_some_and(|l| !matches!(l.line_style, LineStylePreset::Solid));
                per_series.push(
                    if dashed || (variant.is_some_and(|v| v.needs_arc_prefix) && line) {
                        self.ensure_arc_prefix(
                            &cfg.series_id,
                            &cfg.x_column,
                            &cfg.y_column,
                            item.chart_config,
                            if line { star_pitch } else { None },
                        )
                    } else {
                        None
                    },
                );
            }
            arcs.push(per_series);
        }
        arcs
    }

    fn paint_with_pipelines<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'_>,
        target_size: (u32, u32),
        items: &[ChartDrawItem<'a>],
        pipelines: &'a TargetPipelines,
        arcs: &[Vec<Option<ArcPrefix>>],
    ) -> Result<()> {
        for (item, item_arcs) in items.iter().zip(arcs) {
            let panel_rect = item.view.panel_rect;
            let data_area = item
                .chart_config
                .data_area()
                .map(|da| da.0)
                .unwrap_or(panel_rect);

            // The single style decision per panel: the chart's `DrawStyle`
            // resolves to a cached styled pipeline set (compiled by the
            // prepare phase) or `None` for precise — the precise path stays
            // untouched.
            let styled = pipelines.style_set(&item.chart_config.draw_style);

            // Bundle every series's primitives for the panel into one call.
            let series_list =
                self.build_series_layers(item.view, item.series, pipelines, item_arcs, styled)?;

            data_render::draw_chart_panel_columnar(
                pass,
                target_size,
                panel_rect,
                data_area,
                AxisLayer { pipeline: &pipelines.axis, bind_group: &item.view.grid_bind_group },
                &series_list,
                AxisLayer { pipeline: &pipelines.axis, bind_group: &item.view.decoration_bind_group },
            );
        }
        Ok(())
    }

    /// Convert one panel's `Series` list into `SeriesLayers` ready for
    /// drawing. The `config.render_type` enum variant decides which of
    /// line/scatter/errorbar each series needs; column ids are resolved to
    /// handles via the pool. `arcs` is this panel's slice of the prepare
    /// phase's output ([`Self::ensure_arc_prefixes`]), aligned with
    /// `series_specs` — dashed lines (and every line of an arc-needing
    /// style) pick up their arc-length prefix there. `styled` is the panel's
    /// resolved [`StyleSet`] — it selects the stylized pipeline variants
    /// (and the line strip's vertex count); `None` (precise mode, or the
    /// theoretically-impossible cache miss) selects the precise pipelines.
    /// Everything else is identical between the modes.
    fn build_series_layers<'a>(
        &'a self,
        view: &'a ChartView,
        series_specs: &[Series<'a>],
        pipelines: &'a TargetPipelines,
        arcs: &[Option<ArcPrefix>],
        styled: Option<&'a StyleSet>,
    ) -> Result<Vec<data_render::SeriesLayers<'a>>> {
        let pool = &self.pool;
        let lookup = |id: &ColumnId| -> Result<ColumnHandle> {
            pool.handle_for(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.clone() })
        };

        // Resolve the style set into per-primitive picks once. `stars` is the
        // constellation's arc-driven indirect star pass over the same polyline.
        struct LinePick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            verts: u32,
            texture_bg: Option<&'p wgpu::BindGroup>,
        }
        struct StarsPick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            texture_bg: &'p wgpu::BindGroup,
        }
        struct ScatterPick<'p> {
            pipeline: &'p wgpu::RenderPipeline,
            texture_bg: Option<&'p wgpu::BindGroup>,
        }
        let (line_pick, stars_pick, scatter_pick, errorbar_pipe) = match styled {
            None => (
                LinePick { pipeline: &pipelines.line, verts: 4, texture_bg: None },
                None,
                ScatterPick { pipeline: &pipelines.scatter, texture_bg: None },
                &pipelines.errorbar,
            ),
            Some(StyleSet::Sketch { line, scatter, errorbar, line_verts }) => (
                LinePick { pipeline: line, verts: *line_verts, texture_bg: None },
                None,
                ScatterPick { pipeline: scatter, texture_bg: None },
                errorbar,
            ),
            // Milkyway: line = ribbon + star pass, scatter = ringed
            // planets (premultiplied — they occlude the star field),
            // errorbar = bipolar jets with terminal shock knots.
            Some(StyleSet::Milkyway(c)) => (
                LinePick {
                    pipeline: &c.ribbon,
                    verts: data_render::MILKYWAY_RIBBON_VERTICES,
                    texture_bg: Some(&c.star_tex_bg),
                },
                Some(StarsPick { pipeline: &c.stars, texture_bg: &c.star_tex_bg }),
                ScatterPick { pipeline: &c.planets, texture_bg: Some(&c.star_tex_bg) },
                &c.jets,
            ),
            // Constellation: only ScatterLine is supported. The line is the
            // regular columnar stroke with style-level alpha; the scatter
            // layer renders PSF stars at the data points.
            Some(StyleSet::Constellation(c)) => (
                LinePick { pipeline: &c.line, verts: 4, texture_bg: None },
                None,
                ScatterPick { pipeline: &c.stars, texture_bg: Some(&c.star_tex_bg) },
                &pipelines.errorbar,
            ),
        };

        let mut out = Vec::with_capacity(series_specs.len());
        for (idx, series) in series_specs.iter().enumerate() {
            let cfg = series.config;
            let rt = &cfg.render_type;
            let constellation_only = matches!(styled, Some(StyleSet::Constellation(_)));
            let constellation_supported = matches!(rt, DataRenderType::ScatterLine { .. });
            let x_h = lookup(&cfg.x_column)?;
            let y_h = lookup(&cfg.y_column)?;

            let line = if has_line(rt) && (!constellation_only || constellation_supported) {
                let arc = arcs.get(idx).cloned().flatten();
                Some(ColumnLineLayer {
                    pipeline: line_pick.pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.line_bg,
                    pool_buffer: pool.buffer(),
                    x: x_h, y: y_h,
                    arc,
                    verts_per_instance: line_pick.verts,
                    texture_bg: line_pick.texture_bg,
                })
            } else { None };

            // Star pass: per-series GPU state lives in the arc scratch the
            // prepare phase built (indirect args + VS bind group). Missing
            // scratch/star (n < 2, or a theoretically-impossible prepare
            // miss) just skips the stars — the ribbon still draws.
            let line_extra = match (&line, &stars_pick) {
                (Some(_), Some(sp)) => self
                    .arc_cache
                    .get(cfg.series_id.as_str())
                    .and_then(|s| s.star.as_ref())
                    .map(|star| data_render::ColumnStarLayer {
                        pipeline: sp.pipeline,
                        transform_bg: &view.transform_bg,
                        style_bg: &series.style.line_bg,
                        texture_bg: sp.texture_bg,
                        star_bg: &star.vs_bg,
                        indirect: &star.indirect,
                    }),
                _ => None,
            };

            let scatter = if has_scatter(rt) && (!constellation_only || constellation_supported) {
                Some(ColumnScatterLayer {
                    pipeline: scatter_pick.pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: &series.style.scatter_bg,
                    quad_vb: &self.quad_vb,
                    pool_buffer: pool.buffer(),
                    x: x_h, y: y_h,
                    texture_bg: scatter_pick.texture_bg,
                })
            } else { None };

            let errorbar = if constellation_only {
                None
            } else { match (extract_err_y(rt), extract_err_x(rt)) {
                (None, None) => None,
                (ey_opt, ex_opt) => {
                    let (ey_lo, ey_hi) = match ey_opt {
                        Some(ErrorRef::Symmetric { column }) => {
                            let h = lookup(column)?; (h, h)
                        }
                        Some(ErrorRef::Asymmetric { lower, upper }) => {
                            (lookup(lower)?, lookup(upper)?)
                        }
                        None => {
                            let zero = self.zero_handle()?;
                            (zero, zero)
                        }
                    };
                    let (ex_lo, ex_hi) = match ex_opt {
                        Some(ErrorRef::Symmetric { column }) => {
                            let h = lookup(column)?; (h, h)
                        }
                        Some(ErrorRef::Asymmetric { lower, upper }) => {
                            (lookup(lower)?, lookup(upper)?)
                        }
                        None => {
                            let zero = self.zero_handle()?;
                            (zero, zero)
                        }
                    };
                    Some(ColumnErrorBarDraw {
                        pipeline: errorbar_pipe,
                        transform_bg: &view.transform_bg,
                        style_bg: &series.style.errorbar_bg,
                        pool_buffer: pool.buffer(),
                        x: x_h, y: y_h,
                        err_y_lo: ey_lo, err_y_hi: ey_hi,
                        err_x_lo: ex_lo, err_x_hi: ex_hi,
                    })
                }
            }};

            out.push(data_render::SeriesLayers { errorbar, line, line_extra, scatter });
        }
        Ok(out)
    }

    /// Ensure the GPU arc-length prefix for one dashed line series and return
    /// the buffer slice info. The whole computation runs on the GPU
    /// (`line_arc.wgsl` compute scan over the pool columns) — the data never
    /// returns to the CPU, keeping the pool's no-CPU-copy contract intact.
    /// Series longer than one chunk's dispatch capacity scan as sequential
    /// chunks linked by a carry buffer, so any pool-resident `n` is fine.
    ///
    /// The scan is re-dispatched on every draw that needs it (the prefix
    /// depends on the data→pixel transform): a handful of tiny compute
    /// dispatches submitted before the host's render pass, which queue order
    /// then sequences correctly. Buffers/bind groups are cached per series
    /// and rebuilt only when the series layout changes.
    /// `star_pitch`: `Some` adds the constellation star pass to the scratch
    /// (indirect args + VS bind group) and runs its kernel after the scan.
    fn ensure_arc_prefix(
        &mut self,
        series_id: &str,
        x_id: &str,
        y_id: &str,
        chart_config: &Config,
        star_pitch: Option<f32>,
    ) -> Option<ArcPrefix> {
        // Copy the layout scalars out so the pool borrow ends before the
        // cache mutation below.
        let (x_offset, x_len) = {
            let s = self.pool.slot(x_id)?;
            (s.offset, s.len_values)
        };
        let (y_offset, y_len) = {
            let s = self.pool.slot(y_id)?;
            (s.offset, s.len_values)
        };
        let n = x_len.min(y_len);
        if n < 2 {
            return None;
        }
        let n = u32::try_from(n).ok()?;
        // Pool offsets are 256-aligned bytes → exact f32 element indices.
        let x_base = u32::try_from(x_offset / 4).ok()?;
        let y_base = u32::try_from(y_offset / 4).ok()?;
        let generation = self.pool.generation();
        let t = data_render::scatter_transform_from_config(chart_config);

        // Runaway-churn backstop: ids of long-removed series would otherwise
        // pin GPU memory forever. Rebuilt on demand, so clearing is safe.
        if self.arc_cache.len() > 256 {
            self.arc_cache.clear();
        }
        let stale = self.arc_cache.get(series_id).is_none_or(|s| {
            !s.matches(n, x_base, y_base, generation)
                || s.star.is_some() != star_pitch.is_some()
        });
        if stale {
            #[cfg(test)]
            let chunk_override = self.arc_chunk_override;
            #[cfg(not(test))]
            let chunk_override = None;
            let scratch = data_render::line_arc::ArcScratch::build(
                &self.device,
                &self.arc_pipelines,
                self.pool.buffer(),
                n,
                x_base,
                y_base,
                generation,
                self.caps.max_compute_workgroups_per_dimension,
                chunk_override,
                star_pitch.is_some().then_some(&self.star_data_bgl),
            )?; // None only on a zero-dispatch-limit adapter; try_new rejects those.
            self.arc_cache.insert(series_id.to_string(), scratch);
        }
        let scratch = self.arc_cache.get(series_id)?;

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy line arc encoder"),
        });
        scratch.dispatch(&self.queue, &mut encoder, &self.arc_pipelines, &t, star_pitch);
        self.queue.submit(std::iter::once(encoder.finish()));

        Some((Arc::clone(&scratch.arc), u64::from(n) * 4))
    }

    /// Handle of the zero-filled column used to pad the unused dimension of
    /// asymmetric errorbars. Caller must pre-register `"__zero"` via
    /// `renderer.add_column("__zero", &zero_col)` (the web wrapper does this
    /// automatically). Lookup happens in the immutable draw phase, so the
    /// column cannot be created lazily here.
    fn zero_handle(&self) -> Result<ColumnHandle> {
        self.pool.handle_for("__zero").ok_or_else(|| FiggyError::UnknownColumn {
            id: "__zero (zero-fill column for the unused dim of an errorbar series; \
                 pre-register via `renderer.add_column(\"__zero\", &zero_col)`)".into(),
        })
    }

    // Headless PNG export.

    /// Render one chart panel offscreen at `scale × original` pixel
    /// dimensions and return an RGBA buffer. Fonts, line widths, and margins
    /// are all scaled proportionally so the result is visually consistent.
    ///
    /// - `scale` is clamped to [`MIN_EXPORT_SCALE`] .. [`MAX_EXPORT_SCALE`].
    /// - Background is fully transparent (alpha 0).
    /// - Output size is `chart_area × scale`.
    /// - Encoding / saving the PNG is the caller's responsibility — use
    ///   [`encode_png`] or [`Self::export_panel_png_bytes_async`].
    ///
    /// Async because the GPU→CPU readback must yield to the browser on wasm;
    /// on native the await resolves immediately (the device is polled to
    /// completion inline). Native callers can use the blocking
    /// [`Self::export_panel_rgba`] wrapper instead. `&mut self` for the same
    /// reason as [`Self::paint`]: the prepare phase (arc prefixes; the
    /// export's styled pipelines live in a per-call set instead).
    pub async fn export_panel_rgba_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        self.export_panel_rgba_with_clear_async(
            chart,
            series,
            scale,
            crate::color::Color::from_rgba(0.0, 0.0, 0.0, 0.0),
        )
        .await
    }

    /// Export a panel with an explicit clear color behind the chart.
    pub async fn export_panel_rgba_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<RasterImage> {
        let scale = clamp_export_scale(scale);
        let orig = chart.config().chart_area.0;
        let w = ((orig.width as f32) * scale).round().max(1.0) as u32;
        let h = ((orig.height as f32) * scale).round().max(1.0) as u32;
        validate_texture_extent(self.caps, "figgy export target dimension", w, h)?;
        let export_format = wgpu::TextureFormat::Rgba8Unorm;
        let mut export_sample_count = preferred_msaa_sample_count(self.caps, export_format);
        validate_target_sample_count(self.caps, export_format, export_sample_count)?;
        let export_features = export_format.guaranteed_format_features(self.caps.features);
        if !export_features
            .allowed_usages
            .contains(wgpu::TextureUsages::COPY_SRC)
        {
            return Err(FiggyError::UnsupportedSurfaceFormat {
                format: export_format,
                reason: "figgy export target is not guaranteed to support COPY_SRC".into(),
            });
        }

        // 1) Scaled config with chart_area overridden to (0,0,w,h) → temp chart.
        let mut scaled_config = chart.config().scaled(scale);
        scaled_config.chart_area = crate::layout::ChartArea(Rect {
            x: 0, y: 0, width: w, height: h,
        });
        let scaled_chart = Chart::new(scaled_config);

        // 2) Temp ChartView with scaled axis textures.
        let view = self.create_chart_view(&scaled_chart, Rect { x: 0, y: 0, width: w, height: h })?;

        // 3) Series styles — line_width also scales (extracted from render_type).
        let scaled_styles: Vec<ChartStyle> = series.iter()
            .map(|cfg| self.create_style_for_series_scaled(cfg, scale))
            .collect();
        let series_objs: Vec<Series<'_>> = series.iter().zip(scaled_styles.iter())
            .map(|(cfg, st)| Series { config: cfg, style: st })
            .collect();
        let items = [ChartDrawItem {
            view: &view,
            chart_config: scaled_chart.config(),
            series: &series_objs,
        }];

        // 3.5) Arc-prefix prepare phase — submitted before the render pass
        // below, so queue order sequences them.
        let arcs = self.ensure_arc_prefixes(&items);

        // 4) Offscreen target.
        let target_desc = wgpu::TextureDescriptor {
            label: Some("figgy export target"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: export_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let target_tex = create_texture_checked(&self.device, &target_desc, "figgy export target")?;
        let target_view = target_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let msaa_export_target = match create_msaa_target(
            &self.device,
            self.caps,
            "figgy export msaa target",
            w,
            h,
            export_format,
            export_sample_count,
        ) {
            Ok(target) => target,
            Err(_) if export_sample_count > 1 => {
                export_sample_count = 1;
                None
            }
            Err(e) => return Err(e),
        };
        let (render_view, resolve_target, store) = match msaa_export_target.as_ref() {
            Some(msaa) => (&msaa.view, Some(&target_view), wgpu::StoreOp::Discard),
            None => (&target_view, None, wgpu::StoreOp::Store),
        };
        // Per-call set against the export format. The same prepare-phase
        // ensure as `paint` fills the styled cache, so styled variants
        // compile only when this export actually draws with them.
        let mut export_target_pipelines = create_target_pipelines(
            &self.device,
            &self.texture_bgl,
            &self.transform_bgl,
            &self.style_bgl,
            export_format,
            export_sample_count,
        );
        export_target_pipelines.ensure_styles_for_items(
            &self.device,
            &self.queue,
            &self.transform_bgl,
            &self.style_bgl,
            &self.star_data_bgl,
            export_format,
            &items,
        );

        // 5) Readback buffer. Allocate at most the hardware limit and read rows
        // sequentially if the full image would exceed it.
        let unpadded_bpr_u64 = u64::from(w) * 4;
        let align = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let padded_bpr_u64 = ((unpadded_bpr_u64 + align - 1) / align) * align;
        if padded_bpr_u64 > u64::from(u32::MAX) {
            return Err(FiggyError::GpuResourceLimit {
                resource: "figgy export bytes_per_row",
                requested: padded_bpr_u64,
                limit: u64::from(u32::MAX),
            });
        }
        let unpadded_bpr = unpadded_bpr_u64 as u32;
        let padded_bpr = padded_bpr_u64 as u32;
        let max_rows = (self.caps.max_buffer_size / padded_bpr_u64).min(u64::from(h));
        if max_rows == 0 {
            return Err(FiggyError::GpuResourceLimit {
                resource: "figgy export readback row",
                requested: padded_bpr_u64,
                limit: self.caps.max_buffer_size,
            });
        }
        let mut rows_per_chunk = max_rows.min(u64::from(u32::MAX)) as u32;
        let readback = loop {
            let size = padded_bpr_u64 * u64::from(rows_per_chunk);
            validate_buffer_size(self.caps, "figgy export readback buffer", size)?;
            let desc = wgpu::BufferDescriptor {
                label: Some("figgy export readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            };
            match create_buffer_checked(&self.device, &desc, "figgy export readback buffer") {
                Ok(buffer) => break buffer,
                Err(e) if rows_per_chunk > 1 => {
                    rows_per_chunk = (rows_per_chunk / 2).max(1);
                    let _ = e;
                }
                Err(e) => return Err(e),
            }
        };

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy export encoder"),
        });

        // 6) Render pass — configured clear + a single paint.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy export pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: render_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear.r as f64,
                            g: clear.g as f64,
                            b: clear.b as f64,
                            a: clear.a as f64,
                        }),
                        store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.paint_with_pipelines(&mut pass, (w, h), &items, &export_target_pipelines, &arcs)?;
        }
        self.queue.submit(std::iter::once(encoder.finish()));

        // 7) Texture → readback buffer in row chunks.
        let bgra = false;
        let rgba_len = u64::from(w)
            .checked_mul(u64::from(h))
            .and_then(|px| px.checked_mul(4))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(FiggyError::GpuResourceLimit {
                resource: "figgy export rgba output",
                requested: u64::from(w).saturating_mul(u64::from(h)).saturating_mul(4),
                limit: usize::MAX as u64,
            })?;
        let mut rgba = vec![0u8; rgba_len];
        let mut y0 = 0;
        while y0 < h {
            let rows = rows_per_chunk.min(h - y0);
            let chunk_size = padded_bpr_u64 * u64::from(rows);
            let mut copy_encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy export copy encoder"),
            });
            copy_encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: y0, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bpr),
                        rows_per_image: Some(rows),
                    },
                },
                wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 },
            );
            self.queue.submit(std::iter::once(copy_encoder.finish()));

            let slice = readback.slice(..chunk_size);
            let (tx, rx) = futures_channel::oneshot::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
            // Native: drive the device to completion so the await below
            // resolves immediately. On wasm the browser polls the device and
            // the await yields to the JS event loop instead.
            #[cfg(not(target_arch = "wasm32"))]
            let _ = self.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            rx.await
                .map_err(|e| FiggyError::GpuResourceAllocationFailed {
                    resource: "figgy export readback mapping",
                    reason: format!("map_async sender dropped: {e}"),
                })?
                .map_err(|e| FiggyError::GpuResourceAllocationFailed {
                    resource: "figgy export readback mapping",
                    reason: format!("map_async: {e:?}"),
                })?;
            let mapped = slice.get_mapped_range();

            for local_y in 0..rows {
                let src_off = (local_y * padded_bpr) as usize;
                let dst_off = ((y0 + local_y) * unpadded_bpr) as usize;
                let row = &mapped[src_off..src_off + unpadded_bpr as usize];
                for i in 0..(w as usize) {
                    let p = i * 4;
                    let (b0, b1, b2, b3) = (row[p], row[p + 1], row[p + 2], row[p + 3]);
                    let (r, g, b, a) = if bgra { (b2, b1, b0, b3) } else { (b0, b1, b2, b3) };
                    let (or, og, ob) = if a == 0 || a == 255 {
                        (r, g, b)
                    } else {
                        let a_f = a as f32 / 255.0;
                        (
                            ((r as f32 / a_f).round().clamp(0.0, 255.0)) as u8,
                            ((g as f32 / a_f).round().clamp(0.0, 255.0)) as u8,
                            ((b as f32 / a_f).round().clamp(0.0, 255.0)) as u8,
                        )
                    };
                    rgba[dst_off + p] = or;
                    rgba[dst_off + p + 1] = og;
                    rgba[dst_off + p + 2] = ob;
                    rgba[dst_off + p + 3] = a;
                }
            }
            drop(mapped);
            readback.unmap();
            y0 += rows;
        }

        Ok(RasterImage { width: w, height: h, rgba })
    }

    /// Convenience wrapper: export panel RGBA, then encode PNG bytes in
    /// memory. Saving the bytes to disk is up to the caller.
    pub async fn export_panel_png_bytes_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        let img = self.export_panel_rgba_async(chart, series, scale).await?;
        encode_png(&img)
    }

    /// Convenience wrapper: export panel PNG with an explicit clear color.
    pub async fn export_panel_png_bytes_with_clear_async(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<Vec<u8>> {
        let img = self
            .export_panel_rgba_with_clear_async(chart, series, scale, clear)
            .await?;
        encode_png(&img)
    }

    /// Blocking convenience wrapper around [`Self::export_panel_rgba_async`].
    /// Native only — on wasm, await the async variant from the host's event
    /// loop instead.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_panel_rgba(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<RasterImage> {
        pollster::block_on(self.export_panel_rgba_async(chart, series, scale))
    }

    /// Blocking convenience wrapper around
    /// [`Self::export_panel_png_bytes_async`]. Native only.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_panel_png_bytes(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
    ) -> Result<Vec<u8>> {
        pollster::block_on(self.export_panel_png_bytes_async(chart, series, scale))
    }
}

// Export result type, scale clamping, PNG encoding.

/// Lower bound for export scale. 1.0 is screen-equivalent (96 DPI baseline,
/// `scale = dpi / 96`); 0.25 corresponds to ~24 DPI (thumbnails).
pub const MIN_EXPORT_SCALE: f32 = 0.25;
/// Upper bound. 8.0 corresponds to ~768 DPI; we clamp instead of erroring
/// because anything bigger explodes memory and time
/// (an 8× export of a 1920×1080 panel is 15360×8640 RGBA ≈ 530 MB).
pub const MAX_EXPORT_SCALE: f32 = 8.0;

/// Convert DPI to a `scale` value relative to the 96 DPI baseline.
pub fn dpi_to_scale(dpi: f32) -> f32 {
    clamp_export_scale(dpi / 96.0)
}

/// Clamp `scale` to [`MIN_EXPORT_SCALE`] .. [`MAX_EXPORT_SCALE`].
pub fn clamp_export_scale(scale: f32) -> f32 {
    if scale.is_nan() || scale <= 0.0 {
        return MIN_EXPORT_SCALE;
    }
    scale.clamp(MIN_EXPORT_SCALE, MAX_EXPORT_SCALE)
}

/// In-memory RGBA8 image (straight alpha).
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, channel order R, G, B, A.
    pub rgba: Vec<u8>,
}

/// Encode `img` as PNG bytes in memory. Saving to disk is the caller's job.
pub fn encode_png(img: &RasterImage) -> Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc
            .write_header()
            .map_err(|e| FiggyError::RasterWrapFailed { reason: format!("png header: {e}") })?;
        writer
            .write_image_data(&img.rgba)
            .map_err(|e| FiggyError::RasterWrapFailed { reason: format!("png write: {e}") })?;
    }
    Ok(buf)
}

// Public auxiliary types.

/// Per-panel GPU resources: separate grid + decoration textures (for layered
/// compositing) plus the transform uniform.
///
/// Grid (below data) and decoration (axis lines, labels — above data) are
/// rasterized to two textures so the GPU can composite in order:
/// `grid → data → decoration`.
pub struct ChartView {
    grid_texture: wgpu::Texture,
    grid_bind_group: wgpu::BindGroup,
    decoration_texture: wgpu::Texture,
    decoration_bind_group: wgpu::BindGroup,
    transform_buffer: wgpu::Buffer,
    transform_bg: wgpu::BindGroup,
    /// Panel pixel rect in surface coordinates.
    panel_rect: Rect,
    /// Generation of the renderer's cached constellation backdrop currently
    /// uploaded into `grid_texture` (`None` = non-cached content). Lets
    /// `refresh_axis` skip the full-panel texture write on cache hits.
    grid_space_gen: Option<u64>,
}

impl ChartView {
    pub fn panel_rect(&self) -> Rect { self.panel_rect.clone() }
}

/// Bundle of style bind groups for line, scatter, and errorbar primitives.
/// Built via `Renderer::create_style_for_series*`. Multiple series in one
/// chart can share a single `ChartStyle` (`Series.style: &ChartStyle`).
///
/// The style uniform buffers live inside the bind groups — wgpu keeps bound
/// resources alive, so no named buffer fields are needed.
pub struct ChartStyle {
    line_bg: wgpu::BindGroup,
    scatter_bg: wgpu::BindGroup,
    errorbar_bg: wgpu::BindGroup,
}

/// One drawable series — `data_config::SeriesConfig` (declarative) plus a
/// `ChartStyle` (GPU uniform buffers). The unit of input to `Renderer::paint`.
pub struct Series<'a> {
    /// Column ids + render type + label. Pure declaration, no GPU state.
    pub config: &'a SeriesConfig,
    /// GPU style created by `Renderer::create_style_for_series`.
    pub style: &'a ChartStyle,
}

/// One chart panel to draw: view + chart config + series slice.
pub struct ChartDrawItem<'a> {
    pub view: &'a ChartView,
    pub chart_config: &'a crate::config::Config,
    pub series: &'a [Series<'a>],
}

// DataRenderType branching helpers — which layers are needed and which
// sub-style to extract.

fn has_line(rt: &DataRenderType) -> bool {
    matches!(
        rt,
        DataRenderType::Line { .. }
            | DataRenderType::ScatterLine { .. }
            | DataRenderType::LineScatterErrorbarX { .. }
            | DataRenderType::LineScatterErrorbarY { .. }
            | DataRenderType::LineScatterErrorbarXY { .. }
    )
}

fn has_scatter(rt: &DataRenderType) -> bool {
    !matches!(rt, DataRenderType::Line { .. })
}

fn extract_line(rt: &DataRenderType) -> Option<&DataLineStyleConfig> {
    match rt {
        DataRenderType::Line { line }
        | DataRenderType::ScatterLine { line, .. }
        | DataRenderType::LineScatterErrorbarX { line, .. }
        | DataRenderType::LineScatterErrorbarY { line, .. }
        | DataRenderType::LineScatterErrorbarXY { line, .. } => Some(line),
        _ => None,
    }
}

fn extract_scatter(rt: &DataRenderType) -> Option<&DataScatterStyleConfig> {
    match rt {
        DataRenderType::Scatter { scatter }
        | DataRenderType::ScatterLine { scatter, .. }
        | DataRenderType::ScatterErrorbarX { scatter, .. }
        | DataRenderType::ScatterErrorbarY { scatter, .. }
        | DataRenderType::ScatterErrorbarXY { scatter, .. }
        | DataRenderType::LineScatterErrorbarX { scatter, .. }
        | DataRenderType::LineScatterErrorbarY { scatter, .. }
        | DataRenderType::LineScatterErrorbarXY { scatter, .. } => Some(scatter),
        DataRenderType::Line { .. } => None,
    }
}

fn extract_errorbar_style(rt: &DataRenderType) -> Option<&DataErrorBarStyleConfig> {
    match rt {
        DataRenderType::ScatterErrorbarX { err_style, .. }
        | DataRenderType::ScatterErrorbarY { err_style, .. }
        | DataRenderType::ScatterErrorbarXY { err_style, .. }
        | DataRenderType::LineScatterErrorbarX { err_style, .. }
        | DataRenderType::LineScatterErrorbarY { err_style, .. }
        | DataRenderType::LineScatterErrorbarXY { err_style, .. } => Some(err_style),
        _ => None,
    }
}

fn extract_err_y(rt: &DataRenderType) -> Option<&ErrorRef> {
    match rt {
        DataRenderType::ScatterErrorbarY { err_y, .. }
        | DataRenderType::ScatterErrorbarXY { err_y, .. }
        | DataRenderType::LineScatterErrorbarY { err_y, .. }
        | DataRenderType::LineScatterErrorbarXY { err_y, .. } => Some(err_y),
        _ => None,
    }
}

fn extract_err_x(rt: &DataRenderType) -> Option<&ErrorRef> {
    match rt {
        DataRenderType::ScatterErrorbarX { err_x, .. }
        | DataRenderType::ScatterErrorbarXY { err_x, .. }
        | DataRenderType::LineScatterErrorbarX { err_x, .. }
        | DataRenderType::LineScatterErrorbarXY { err_x, .. } => Some(err_x),
        _ => None,
    }
}

// WindowedRenderer — figgy owns the surface and swap chain.

/// Returned by `Renderer::for_window`. The caller never touches the surface,
/// surface config, encoder, or render pass — just calls `draw` per frame.
///
/// `Deref<Target = Renderer>` is implemented, so `add_column`,
/// `create_chart_view`, `create_style_for_series`, … are callable directly.
pub struct WindowedRenderer<'w> {
    inner: Renderer,
    surface: wgpu::Surface<'w>,
    surface_config: wgpu::SurfaceConfiguration,
    msaa_target: Option<MsaaTarget>,
    /// Instance and adapter must outlive the surface; keep them pinned here.
    _instance: wgpu::Instance,
    _adapter: wgpu::Adapter,
}

impl<'w> std::ops::Deref for WindowedRenderer<'w> {
    type Target = Renderer;
    fn deref(&self) -> &Renderer { &self.inner }
}

impl<'w> std::ops::DerefMut for WindowedRenderer<'w> {
    fn deref_mut(&mut self) -> &mut Renderer { &mut self.inner }
}

impl Drop for WindowedRenderer<'_> {
    fn drop(&mut self) {
        self.inner.wait_idle();
    }
}

impl<'w> WindowedRenderer<'w> {
    pub fn surface_size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
    }

    /// Reconfigure the swap chain after a window resize. The caller is then
    /// responsible for updating each chart's `chart_area` — figgy doesn't
    /// know your panel layout policy.
    pub fn resize(&mut self, w: u32, h: u32) -> Result<()> {
        data_render::reconfigure_surface(
            &self.surface, &self._adapter, self.inner.device(),
            &mut self.surface_config, w.max(1), h.max(1),
        )?;
        let preferred_sample_count =
            preferred_msaa_sample_count(self.inner.caps, self.surface_config.format);
        let (target_sample_count, msaa_target) = match create_msaa_target(
            self.inner.device(),
            self.inner.caps,
            "figgy frame msaa target",
            self.surface_config.width,
            self.surface_config.height,
            self.surface_config.format,
            preferred_sample_count,
        ) {
            Ok(target) => (preferred_sample_count, target),
            Err(_) if preferred_sample_count > 1 => (1, None),
            Err(e) => return Err(e),
        };
        self.inner
            .ensure_target(self.surface_config.format, target_sample_count)?;
        self.msaa_target = msaa_target;
        Ok(())
    }

    /// Draw one frame.
    ///
    /// Acquires the current swap-chain texture, builds an encoder + render
    /// pass, calls `paint(items)`, then submits and presents. The caller is
    /// only responsible for processing per-panel dirty flags
    /// (`chart.consume_*_dirty()` → `refresh_axis` / `update_transform`).
    pub fn draw(&mut self, clear: crate::color::Color, items: &[ChartDrawItem<'_>]) -> Result<()> {
        let frame = match self.surface.get_current_texture() {
            Ok(t) => t,
            Err(error @ (wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated)) => {
                // Let the caller decide whether to resize, retry, or exit.
                return Err(FiggyError::SurfaceAcquireFailed { error });
            }
            Err(error) => return Err(FiggyError::SurfaceAcquireFailed { error }),
        };
        let target = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let (color_view, resolve_target, store) = match self.msaa_target.as_ref() {
            Some(msaa) => (&msaa.view, Some(&target), wgpu::StoreOp::Discard),
            None => (&target, None, wgpu::StoreOp::Store),
        };

        let mut encoder = self
            .inner
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy frame encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy frame pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear.r as f64,
                            g: clear.g as f64,
                            b: clear.b as f64,
                            a: clear.a as f64,
                        }),
                        store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.inner.paint(
                &mut pass,
                (self.surface_config.width, self.surface_config.height),
                items,
            )?;
        }
        self.inner
            .queue()
            .submit(std::iter::once(encoder.finish()));
        frame.present();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Column;
    use crate::data_render::{create_instance, request_adapter, request_device};

    fn col_f64(data: Vec<f64>) -> Column<f64> {
        let min = data.iter().copied().fold(f64::INFINITY, f64::min);
        let max = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Column { data, min, max }
    }

    fn test_scatter_style() -> DataScatterStyleConfig {
        DataScatterStyleConfig {
            point_color: Color::new(0.0, 0.0, 0.0, 1.0),
            point_shape: crate::data_config::ScatterShape::CircleFilled,
            point_size: 5.0,
        }
    }

    fn test_errorbar_style() -> DataErrorBarStyleConfig {
        DataErrorBarStyleConfig {
            error_bar_color: Color::new(1.0, 0.0, 0.0, 1.0),
            error_bar_width: 1.0,
            error_bar_cap_size: 4.0,
            cap_width: 1.0,
        }
    }

    fn basic_errorbar_chart() -> Chart {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 320, height: 240 });
        config.legend.visible = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 4.0);
        chart.set_y_range(0.0, 5.0);
        chart
    }

    #[test]
    fn point_constellation_scatterline_exports() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::new(device), Arc::new(queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        ).unwrap();
        r.add_column("pc_x", &col_f64(vec![0.0, 0.25, 0.5, 0.75, 1.0])).unwrap();
        r.add_column("pc_y", &col_f64(vec![0.2, 0.75, 0.35, 0.9, 0.5])).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 320, height: 240 });
        config.legend.visible = false;
        config.draw_style = crate::config::DrawStyle::Constellation(
            crate::config::ConstellationOptions::default(),
        );
        let mut chart = Chart::new(config);
        chart.set_x_range(-0.05, 1.05);
        chart.set_y_range(0.0, 1.0);

        let series = [SeriesConfig {
            series_id: "pc".into(),
            label: None,
            x_column: "pc_x".into(),
            y_column: "pc_y".into(),
            render_type: DataRenderType::ScatterLine {
                scatter: DataScatterStyleConfig {
                    point_color: Color::new(1.0, 1.0, 1.0, 1.0),
                    point_shape: crate::data_config::ScatterShape::CircleFilled,
                    point_size: 5.0,
                },
                line: DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(0.4, 0.7, 1.0, 1.0),
                    line_width: 2.0,
                },
            },
        }];
        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let lit = img.rgba.chunks_exact(4).filter(|p| p[3] > 0).count();
        assert!(lit > 100, "point constellation export produced too little ink: {lit}");
    }

    #[test]
    fn all_scatter_shapes_export_visible_markers() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::new(device), Arc::new(queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        ).unwrap();

        let shapes = [
            crate::data_config::ScatterShape::Circle,
            crate::data_config::ScatterShape::Square,
            crate::data_config::ScatterShape::Triangle,
            crate::data_config::ScatterShape::Diamond,
            crate::data_config::ScatterShape::Cross,
            crate::data_config::ScatterShape::CircleFilled,
            crate::data_config::ScatterShape::SquareFilled,
            crate::data_config::ScatterShape::TriangleFilled,
            crate::data_config::ScatterShape::DiamondFilled,
            crate::data_config::ScatterShape::TriangleDown,
            crate::data_config::ScatterShape::TriangleLeft,
            crate::data_config::ScatterShape::TriangleRight,
            crate::data_config::ScatterShape::Plus,
            crate::data_config::ScatterShape::Pentagon,
            crate::data_config::ScatterShape::Hexagon,
            crate::data_config::ScatterShape::Octagon,
            crate::data_config::ScatterShape::Star,
            crate::data_config::ScatterShape::TriangleDownFilled,
            crate::data_config::ScatterShape::TriangleLeftFilled,
            crate::data_config::ScatterShape::TriangleRightFilled,
            crate::data_config::ScatterShape::PlusFilled,
            crate::data_config::ScatterShape::CrossFilled,
            crate::data_config::ScatterShape::PentagonFilled,
            crate::data_config::ScatterShape::HexagonFilled,
            crate::data_config::ScatterShape::OctagonFilled,
            crate::data_config::ScatterShape::StarFilled,
        ];

        let mut series = Vec::new();
        for (i, shape) in shapes.iter().enumerate() {
            let x = (i % 7) as f64;
            let y = (i / 7) as f64;
            let x_id = format!("shape_x_{i}");
            let y_id = format!("shape_y_{i}");
            r.add_column(&x_id, &col_f64(vec![x])).unwrap();
            r.add_column(&y_id, &col_f64(vec![y])).unwrap();
            series.push(SeriesConfig {
                series_id: format!("shape_{i}"),
                label: None,
                x_column: x_id,
                y_column: y_id,
                render_type: DataRenderType::Scatter {
                    scatter: DataScatterStyleConfig {
                        point_color: Color::new(1.0, 0.0, 0.0, 1.0),
                        point_shape: shape.clone(),
                        point_size: 8.0,
                    },
                },
            });
        }

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 700, height: 400 });
        config.legend.visible = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(-0.5, 6.5);
        chart.set_y_range(-0.5, 3.8);

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let red_ink = img
            .rgba
            .chunks_exact(4)
            .filter(|p| p[0] > 150 && p[1] < 80 && p[2] < 80 && p[3] > 100)
            .count();
        assert!(red_ink > 600, "scatter shape export produced too little red ink: {red_ink}");
    }

    #[test]
    fn xy_errorbar_does_not_require_zero_column() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        ).unwrap();
        r.add_column("x", &col_f64(vec![1.0, 2.0, 3.0])).unwrap();
        r.add_column("y", &col_f64(vec![1.0, 3.0, 2.0])).unwrap();
        r.add_column("ex", &col_f64(vec![0.1, 0.2, 0.1])).unwrap();
        r.add_column("ey", &col_f64(vec![0.3, 0.1, 0.2])).unwrap();

        let chart = basic_errorbar_chart();
        let series = [SeriesConfig {
            series_id: "xy".into(),
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::ScatterErrorbarXY {
                scatter: test_scatter_style(),
                err_x: ErrorRef::Symmetric { column: "ex".into() },
                err_y: ErrorRef::Symmetric { column: "ey".into() },
                err_style: test_errorbar_style(),
            },
        }];

        r.export_panel_rgba(&chart, &series, 1.0).unwrap();
    }

    #[test]
    fn single_direction_errorbar_still_requires_zero_column() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        ).unwrap();
        r.add_column("x", &col_f64(vec![1.0, 2.0, 3.0])).unwrap();
        r.add_column("y", &col_f64(vec![1.0, 3.0, 2.0])).unwrap();
        r.add_column("ey", &col_f64(vec![0.3, 0.1, 0.2])).unwrap();

        let chart = basic_errorbar_chart();
        let series = [SeriesConfig {
            series_id: "y".into(),
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::ScatterErrorbarY {
                scatter: test_scatter_style(),
                err_y: ErrorRef::Symmetric { column: "ey".into() },
                err_style: test_errorbar_style(),
            },
        }];

        let err = match r.export_panel_rgba(&chart, &series, 1.0) {
            Ok(_) => panic!("Y-only errorbar must still need __zero for the missing X side"),
            Err(err) => err,
        };
        match err {
            FiggyError::UnknownColumn { id } => assert!(id.contains("__zero"), "{id}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    /// Two line series on distinct column pairs must BOTH render — this
    /// mirrors the web demo (sine on x/y + rc on t/v) and guards the
    /// second-series draw path end to end via headless export.
    #[test]
    fn two_line_series_both_render() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        ).unwrap();

        let n = 512;
        let xs: Vec<f64> = (0..n).map(|i| i as f64 * 6.28 / n as f64).collect();
        let ys: Vec<f64> = xs.iter().map(|x| x.sin()).collect();
        let ts: Vec<f64> = (0..n).map(|i| i as f64 * 5.0 / n as f64).collect();
        let vs: Vec<f64> = ts.iter().map(|t| 1.0 - (-t).exp()).collect();
        r.add_column("x", &col_f64(xs)).unwrap();
        r.add_column("y", &col_f64(ys)).unwrap();
        r.add_column("t", &col_f64(ts)).unwrap();
        r.add_column("v", &col_f64(vs)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 640, height: 480 });
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 6.5);
        chart.set_y_range(-1.2, 1.2);

        let line_cfg = |id: &str, x: &str, y: &str, color: Color| SeriesConfig {
            series_id: id.into(),
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: color,
                    line_width: 2.0,
                },
            },
        };
        let series = [
            line_cfg("sine", "x", "y", Color::BLACK),
            line_cfg("rc", "t", "v", Color::new(1.0, 0.0, 0.0, 1.0)),
        ];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (mut red, mut black) = (0usize, 0usize);
        for px in img.rgba.chunks_exact(4) {
            if px[3] > 16 {
                if px[0] > 120 && px[1] < 90 && px[2] < 90 { red += 1; }
                if px[0] < 90 && px[1] < 90 && px[2] < 90 { black += 1; }
            }
        }
        assert!(black > 300, "sine (black) curve missing: {black} px");
        assert!(red > 300, "rc (red) curve missing: {red} px");
    }

    /// Milkyway arc-star density must be a property of the polyline's ARC, not of how
    /// densely the data samples it — the field-reported failure mode of the
    /// old per-segment quad budget, where a sparse polyline saturated at 24
    /// stars per segment and then ignored the density knob entirely. The
    /// same curve sampled at 8 vs 480 points must produce comparable star
    /// ink (the indirect pass budgets by total arc; the 8-point chord arc is
    /// only a few % shorter).
    #[test]
    fn milkyway_arc_star_density_is_sampling_invariant() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::new(device), Arc::new(queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        ).unwrap();

        let curve = |n: usize| -> (Vec<f64>, Vec<f64>) {
            let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
            let ys: Vec<f64> = xs.iter().map(|x| 50.0 + 30.0 * (x * 4.2).sin()).collect();
            (xs, ys)
        };
        let (sx, sy) = curve(8);
        let (dx, dy) = curve(480);
        r.add_column("inv_sx", &col_f64(sx)).unwrap();
        r.add_column("inv_sy", &col_f64(sy)).unwrap();
        r.add_column("inv_dx", &col_f64(dx)).unwrap();
        r.add_column("inv_dy", &col_f64(dy)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area =
            crate::layout::ChartArea(Rect { x: 0, y: 0, width: 640, height: 400 });
        // Stars only: ribbon/backdrop/glow off so the ink count measures the
        // star field alone. Density 60 needs ~540 stars over this arc — the
        // old per-segment budget would cap the 8-point series at 7·24 = 168.
        config.draw_style = crate::config::DrawStyle::Milkyway(
            crate::config::MilkywayOptions {
                star_density: 60.0,
                ribbon_intensity: 0.0,
                nebula: 0.0,
                dust: 0.0,
                glow: 0.0,
                ..Default::default()
            },
        );
        let mut chart = Chart::new(config);
        chart.set_x_range(-0.05, 1.05);
        chart.set_y_range(0.0, 100.0);

        let line_cfg = |id: &str, x: &str, y: &str| SeriesConfig {
            series_id: id.into(),
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(1.0, 1.0, 1.0, 1.0),
                    line_width: 2.0,
                },
            },
        };
        let mut star_ink = |id: &str, x: &str, y: &str| -> usize {
            let series = [line_cfg(id, x, y)];
            let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
            img.rgba
                .chunks_exact(4)
                .filter(|p| p[3] > 0 && (p[0] > 40 || p[1] > 40 || p[2] > 40))
                .count()
        };
        // Same series id on purpose: identical salt isolates sampling as the
        // only variable.
        let sparse = star_ink("inv", "inv_sx", "inv_sy");
        let dense = star_ink("inv", "inv_dx", "inv_dy");

        assert!(sparse > 1_000, "sparse star field missing ({sparse} px)");
        assert!(dense > 1_000, "dense star field missing ({dense} px)");
        let ratio = sparse as f64 / dense as f64;
        assert!(
            (0.65..=1.55).contains(&ratio),
            "star ink must not depend on sampling density: sparse {sparse} px \
             vs dense {dense} px (ratio {ratio:.2})"
        );
    }

    /// The constellation backdrop cache must rebake only when its key
    /// (panel size, nebula, dust, seed) changes: range-only refreshes hit
    /// the cache, and a view already holding the baked generation skips the
    /// texture re-upload (`grid_space_gen` stamp).
    #[test]
    fn space_background_rebakes_only_on_key_change() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::new(device), Arc::new(queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        ).unwrap();

        let rect = Rect { x: 0, y: 0, width: 320, height: 200 };
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(rect.clone());
        config.draw_style = crate::config::DrawStyle::Milkyway(
            crate::config::MilkywayOptions::default(),
        );
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(0.0, 1.0);

        let mut view = r.create_chart_view(&chart, rect.clone()).unwrap();
        assert_eq!(view.grid_space_gen, None, "fresh view starts unstamped");

        r.refresh_axis(&mut view, &chart, rect.clone()).unwrap();
        assert_eq!(r.space_bg.as_ref().map(|s| s.bake_gen), Some(1));
        assert_eq!(view.grid_space_gen, Some(1));

        // Range-only change (pan/zoom): cache hit — no rebake, no restamp.
        chart.set_x_range(0.0, 2.0);
        r.refresh_axis(&mut view, &chart, rect.clone()).unwrap();
        assert_eq!(
            r.space_bg.as_ref().map(|s| s.bake_gen),
            Some(1),
            "range-only refresh must not rebake the backdrop"
        );
        assert_eq!(view.grid_space_gen, Some(1));

        // Backdrop parameter change: rebake + re-upload.
        if let crate::config::DrawStyle::Milkyway(c) = &mut chart.config_mut().draw_style {
            c.seed = 7;
        }
        r.refresh_axis(&mut view, &chart, rect.clone()).unwrap();
        assert_eq!(
            r.space_bg.as_ref().map(|s| s.bake_gen),
            Some(2),
            "seed change must rebake the backdrop"
        );
        assert_eq!(view.grid_space_gen, Some(2));

        // Leaving the style un-stamps the view; the slot itself is retained.
        chart.config_mut().draw_style = crate::config::DrawStyle::Precise;
        r.refresh_axis(&mut view, &chart, rect).unwrap();
        assert_eq!(view.grid_space_gen, None);
        assert_eq!(r.space_bg.as_ref().map(|s| s.bake_gen), Some(2));
    }

    /// The GPU arc-length scan must equal a sequential CPU reference for
    /// every dispatch shape: single block (n ≤ 256), one sums level, two
    /// sums levels (n > 65 536), and the sequential multi-chunk path (forced
    /// via a narrowed chunk size) that makes n unlimited — both an
    /// exact-multiple boundary and a ragged tail. This is the correctness
    /// proof that lets the pool keep NO CPU copy of column data.
    #[test]
    fn gpu_arc_prefix_matches_cpu_reference() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        ).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 900, height: 600 });
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(-1.2, 1.2);

        for (case, n, chunk) in [
            ("tiny", 3usize, None),
            ("one-level", 1000, None),
            ("two-level", 70_000, None),
            // Forced multi-chunk shapes: an exact-multiple boundary and a
            // ragged tail both exercise the sequential carry chain.
            ("chunked-even", 2_000, Some(1_000)),
            ("chunked-ragged", 2_500, Some(1_000)),
        ] {
            r.arc_chunk_override = chunk;
            let xs: Vec<f64> = (0..n).map(|i| i as f64 / n as f64).collect();
            let ys: Vec<f64> = (0..n).map(|i| (i as f64 * 0.37).sin()).collect();
            let xid = format!("ax_{case}");
            let yid = format!("ay_{case}");
            r.add_column(&xid, &col_f64(xs.clone())).unwrap();
            r.add_column(&yid, &col_f64(ys.clone())).unwrap();

            let (arc_buf, len_bytes) = r
                .ensure_arc_prefix(&format!("s_{case}"), &xid, &yid, chart.config(), None)
                .expect("arc prefix");

            // Read the GPU result back.
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: len_bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_buffer_to_buffer(&arc_buf, 0, &readback, 0, len_bytes);
            queue.submit(std::iter::once(enc.finish()));
            let slice = readback.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            let gpu: Vec<f32> = bytemuck::cast_slice(&slice.get_mapped_range()).to_vec();

            // Sequential CPU reference mirroring the shader math.
            let t = data_render::scatter_transform_from_config(chart.config());
            let px = |x: f64, y: f64| -> (f32, f32) {
                let nx = ((x as f32) - t.data_min[0]) / (t.data_max[0] - t.data_min[0]) * 2.0 - 1.0;
                let ny = ((y as f32) - t.data_min[1]) / (t.data_max[1] - t.data_min[1]) * 2.0 - 1.0;
                (nx / t.pixel_to_ndc[0], ny / t.pixel_to_ndc[1])
            };
            let mut acc = 0.0f32;
            let mut reference = vec![0.0f32];
            for i in 1..n {
                let a = px(xs[i - 1], ys[i - 1]);
                let b = px(xs[i], ys[i]);
                acc += ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
                reference.push(acc);
            }

            assert_eq!(gpu.len(), n, "{case}: length");
            let total = reference.last().copied().unwrap_or(0.0).max(1.0);
            for (i, (g, c)) in gpu.iter().zip(reference.iter()).enumerate() {
                assert!(
                    (g - c).abs() <= total * 2e-4 + 0.05,
                    "{case}: arc[{i}] gpu={g} cpu={c} (total {total})"
                );
            }
        }
    }

    /// Log-axis auto-fit with zeros in the data must clamp the lower bound
    /// to the smallest POSITIVE value (via the pool's CPU shadow) instead of
    /// feeding log10 a non-positive min.
    #[test]
    fn log_auto_fit_clamps_to_smallest_positive() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        ).unwrap();
        // Data spans 0..1000 with smallest positive 0.04.
        let vals = vec![0.0, 0.04, 1.0, 50.0, 1000.0];
        r.add_column("v", &col_f64(vals)).unwrap();

        let mut config = crate::default::default_config();
        config.left_y.scale = crate::config::AxisScale::Logarithmic;
        let mut chart = Chart::new(config);
        chart.auto_fit_y_union(r.pool(), &["v"], 0.0).unwrap();

        let min = chart.config().left_y.min;
        assert!(
            (min - 0.04).abs() < 1e-9,
            "log fit lower bound must be the smallest positive value, got {min}"
        );
        assert!(chart.config().left_y.max >= 1000.0);
    }

    /// `auto_fit_*` must leave a UNIFORM margin fraction on all four sides —
    /// measured in pixels, from the user's point of view. Data is a triangle
    /// wave touching its exact min/max so the ink bbox equals the data bbox.
    #[test]
    fn auto_fit_margins_are_uniform_on_all_sides() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        ).unwrap();

        let n = 400;
        let xs: Vec<f64> = (0..n).map(|i| i as f64).collect();
        // Triangle wave between exactly 10 and 90.
        let ys: Vec<f64> = (0..n)
            .map(|i| {
                let t = (i % 80) as f64 / 80.0;
                let tri = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
                10.0 + tri * 80.0
            })
            .collect();
        r.add_column("x", &col_f64(xs)).unwrap();
        r.add_column("y", &col_f64(ys)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 800, height: 600 });
        config.legend.visible = false;
        // Bare data area: no grid/tick ink inside to pollute the bbox.
        config.grid.show_major_x = false;
        config.grid.show_major_y = false;
        config.grid.show_minor_x = false;
        config.grid.show_minor_y = false;
        let mut chart = Chart::new(config);
        let pad = 0.05;
        chart.auto_fit_x_union(r.pool(), &["x"], pad).unwrap();
        chart.auto_fit_y_union(r.pool(), &["y"], pad).unwrap();

        let series = [SeriesConfig {
            series_id: "s".into(),
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 1.0,
                },
            },
        }];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (w, h) = (img.width as usize, img.height as usize);
        let da = chart.config().data_area().unwrap().0;

        let (mut min_x, mut max_x, mut min_y, mut max_y) = (usize::MAX, 0usize, usize::MAX, 0usize);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let p = &img.rgba[i..i + 4];
                if p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90 {
                    min_x = min_x.min(x); max_x = max_x.max(x);
                    min_y = min_y.min(y); max_y = max_y.max(y);
                }
            }
        }
        assert!(min_x < max_x, "no data ink found");

        // Expected margin fraction of the data_area extent on every side:
        // pad / (1 + 2·pad) of the full padded span.
        let expect = pad / (1.0 + 2.0 * pad);
        let close = |frac: f64, side: &str| {
            assert!(
                (frac - expect).abs() < 0.015,
                "{side} margin {frac:.3}, expected {expect:.3}"
            );
        };
        let daw = da.width as f64;
        let dah = da.height as f64;
        close((min_x as f64 - da.x as f64) / daw, "left");
        close((da.x as f64 + daw - 1.0 - max_x as f64) / daw, "right");
        close((min_y as f64 - da.y as f64) / dah, "top");
        close((da.y as f64 + dah - 1.0 - max_y as f64) / dah, "bottom");
    }

    /// A solid line stays CONTINUOUS no matter how dense the data is. With
    /// sub-pixel segments (here ~0.04 px each) naive per-segment quads
    /// degenerate into disconnected slivers — every x column the curve
    /// crosses must contain ink.
    #[test]
    fn dense_line_has_no_gaps() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        ).unwrap();

        let n = 50_000;
        let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
        // Decay curve like the report, plus measurement-style jitter: the
        // sub-pixel zig-zag flips the segment direction at every point,
        // which is what real noisy data does to the quad extrusion.
        let ys: Vec<f64> = xs
            .iter()
            .enumerate()
            .map(|(i, x)| (-6.0 * x).exp() + if i % 2 == 0 { 0.0008 } else { -0.0008 })
            .collect();
        r.add_column("dx", &col_f64(xs)).unwrap();
        r.add_column("dy", &col_f64(ys)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 800, height: 400 });
        config.legend.visible = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(-0.05, 1.05);

        let series = [SeriesConfig {
            series_id: "dense".into(),
            label: None,
            x_column: "dx".into(),
            y_column: "dy".into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 1.5,
                },
            },
        }];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (w, h) = (img.width as usize, img.height as usize);
        let col_has_red = |x: usize| {
            (0..h).any(|y| {
                let i = (y * w + x) * 4;
                let p = &img.rgba[i..i + 4];
                p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
            })
        };
        let first = (0..w).find(|&x| col_has_red(x)).expect("curve drew nothing");
        let last = (0..w).rev().find(|&x| col_has_red(x)).unwrap();
        let gaps: Vec<usize> = (first..=last).filter(|&x| !col_has_red(x)).collect();
        assert!(
            gaps.is_empty(),
            "dense solid line has {} gap columns between x={first} and x={last}: {:?}…",
            gaps.len(),
            &gaps[..gaps.len().min(20)]
        );

        // Continuity alone is not enough — a stippled line can still touch
        // every column. A 1.5 px stroke must average ≥ 1 ink pixel per
        // column along a near-horizontal curve.
        let red_per_col = |x: usize| {
            (0..h)
                .filter(|&y| {
                    let i = (y * w + x) * 4;
                    let p = &img.rgba[i..i + 4];
                    p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
                })
                .count()
        };
        let total: usize = (first..=last).map(red_per_col).sum();
        let density = total as f64 / (last - first + 1) as f64;
        assert!(
            density >= 1.0,
            "dense solid line is stippled: {density:.2} ink px per column (expected ≥ 1.0)"
        );
    }

    /// The square-cap fix must NOT change line semantics: NaN values still
    /// break the line (the caps may narrow a gap by ~line_width, never close
    /// a real one), and nothing assumes monotonic x — a path that doubles
    /// back stays continuous.
    #[test]
    fn caps_preserve_nan_breaks_and_nonmonotonic_paths() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        ).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 800, height: 400 });
        config.legend.visible = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(-0.1, 1.1);

        let line_cfg = |id: &str, x: &str, y: &str| SeriesConfig {
            series_id: id.into(),
            label: None,
            x_column: x.into(),
            y_column: y.into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Solid,
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 1.5,
                },
            },
        };
        let red_cols = |img: &crate::RasterImage| -> Vec<usize> {
            let (w, h) = (img.width as usize, img.height as usize);
            (0..w)
                .filter(|&x| {
                    (0..h).any(|y| {
                        let i = (y * w + x) * 4;
                        let p = &img.rgba[i..i + 4];
                        p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
                    })
                })
                .collect()
        };

        // 1) Dense noisy line with a NaN band: x ∈ [0.40, 0.45] → ~36 px of
        //    data area. The break must survive (gap may shrink by ~1.5 px of
        //    caps, so probe the gap's middle).
        let n = 20_000;
        let xs: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
        let ys: Vec<f64> = xs
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                if (0.40..0.45).contains(&x) {
                    f64::NAN
                } else {
                    0.5 + 0.3 * (x * 9.0).sin() + if i % 2 == 0 { 0.0008 } else { -0.0008 }
                }
            })
            .collect();
        r.add_column("gx", &col_f64(xs)).unwrap();
        r.add_column("gy", &col_f64(ys)).unwrap();

        let series = [line_cfg("gap", "gx", "gy")];
        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let cols = red_cols(&img);
        let da = chart.config().data_area().unwrap().0;
        let to_px = |v: f64| da.x as f64 + v * da.width as f64;
        let (gap_a, gap_b) = (to_px(0.40), to_px(0.45));
        let mid_a = (gap_a + 4.0) as usize;
        let mid_b = (gap_b - 4.0) as usize;
        let leaked: Vec<&usize> =
            cols.iter().filter(|&&x| x >= mid_a && x <= mid_b).collect();
        assert!(
            leaked.is_empty(),
            "NaN break was bridged: ink at columns {leaked:?} inside the gap ({mid_a}..{mid_b})"
        );
        // Both sides of the gap still drew.
        assert!(cols.iter().any(|&x| (x as f64) < gap_a - 2.0), "left of gap missing");
        assert!(cols.iter().any(|&x| (x as f64) > gap_b + 2.0), "right of gap missing");

        // 2) Non-monotonic path: x sweeps 0→1 then back 1→0 at a higher y.
        //    Every column the path covers must have ink in BOTH passes.
        let m = 30_000;
        let xs2: Vec<f64> = (0..m)
            .map(|i| {
                let t = i as f64 / (m - 1) as f64;
                if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 }
            })
            .collect();
        let ys2: Vec<f64> = (0..m)
            .map(|i| {
                let t = i as f64 / (m - 1) as f64;
                let base = if t < 0.5 { 0.25 } else { 0.75 };
                base + if i % 2 == 0 { 0.0008 } else { -0.0008 }
            })
            .collect();
        r.add_column("mx", &col_f64(xs2)).unwrap();
        r.add_column("my", &col_f64(ys2)).unwrap();

        let series2 = [line_cfg("loop", "mx", "my")];
        let img2 = r.export_panel_rgba(&chart, &series2, 1.0).unwrap();
        let (w2, h2) = (img2.width as usize, img2.height as usize);
        let half = h2 / 2; // y=0.25 band renders below the midline, 0.75 above
        let band_has = |x: usize, top: bool| {
            let range = if top { 0..half } else { half..h2 };
            range.into_iter().any(|y| {
                let i = (y * w2 + x) * 4;
                let p = &img2.rgba[i..i + 4];
                p[3] > 16 && p[0] > 120 && p[1] < 90 && p[2] < 90
            })
        };
        let cols2 = red_cols(&img2);
        let (first2, last2) = (*cols2.first().unwrap(), *cols2.last().unwrap());
        let gaps2: Vec<usize> = (first2..=last2)
            .filter(|&x| !(band_has(x, true) && band_has(x, false)))
            .collect();
        assert!(
            gaps2.is_empty(),
            "non-monotonic path broke: {} columns missing one of the two passes: {:?}…",
            gaps2.len(),
            &gaps2[..gaps2.len().min(20)]
        );
    }

    /// Dash regularity on a CURVED line: the dash phase must advance with
    /// arc length, so on/off run lengths along the curve stay close to the
    /// pattern (Dash = [8, 4]). A phase that jumps at segment joints
    /// (per-segment restart, or screen-position projection) shatters the
    /// pattern into irregular fragments and fails the run statistics.
    #[test]
    fn dashed_curve_runs_match_pattern() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            4 * 1024 * 1024,
        ).unwrap();

        // Worst case for joint phase errors: a SPARSE sine (large direction
        // change per segment) on a WIDE panel (fragments far from the origin
        // amplify any screen-position-derived phase).
        let n = 64;
        let ts: Vec<f64> = (0..n).map(|i| i as f64 * 6.28 / (n - 1) as f64).collect();
        let vs: Vec<f64> = ts.iter().map(|t| t.sin()).collect();
        r.add_column("t", &col_f64(ts)).unwrap();
        r.add_column("v", &col_f64(vs)).unwrap();

        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(Rect { x: 0, y: 0, width: 1000, height: 600 });
        // Hide chrome that could add red-ish AA pixels; keep it minimal.
        config.legend.visible = false;
        let mut chart = Chart::new(config);
        chart.set_x_range(0.0, 6.28);
        chart.set_y_range(-1.2, 1.2);

        let series = [SeriesConfig {
            series_id: "rc".into(),
            label: None,
            x_column: "t".into(),
            y_column: "v".into(),
            render_type: DataRenderType::Line {
                line: crate::data_config::DataLineStyleConfig {
                    line_style: crate::line::LineStylePreset::Dash, // [8, 4]
                    line_color: Color::new(1.0, 0.0, 0.0, 1.0),
                    line_width: 2.0,
                },
            },
        }];

        let img = r.export_panel_rgba(&chart, &series, 1.0).unwrap();
        let (w, h) = (img.width as usize, img.height as usize);
        let is_red = |x: usize, y: usize| {
            let i = (y * w + x) * 4;
            let p = &img.rgba[i..i + 4];
            p[3] > 128 && p[0] > 120 && p[1] < 90 && p[2] < 90
        };

        // Trace the x-monotonic curve column by column: ink centroid y per
        // column (None = gap), arc step ds = √(1 + dy²) per 1-px x step.
        // On/off run lengths in ARC pixels must then match the [8, 4]
        // pattern regardless of local slope.
        let col_y: Vec<Option<f64>> = (0..w)
            .map(|x| {
                let ys: Vec<usize> = (0..h).filter(|&y| is_red(x, y)).collect();
                if ys.is_empty() { None } else {
                    Some(ys.iter().sum::<usize>() as f64 / ys.len() as f64)
                }
            })
            .collect();
        let x0 = col_y.iter().position(|c| c.is_some()).expect("dashed curve drew nothing");
        let x1 = col_y.iter().rposition(|c| c.is_some()).unwrap();

        let mut runs_on: Vec<f64> = Vec::new();
        let mut runs_off: Vec<f64> = Vec::new();
        let mut cur_on = true;
        let mut run = 0.0f64;
        let mut last_y = col_y[x0].unwrap();
        for x in x0..=x1 {
            let (on, ds) = match col_y[x] {
                Some(y) => {
                    let ds = (1.0 + (y - last_y).powi(2)).sqrt();
                    last_y = y;
                    (true, ds)
                }
                // In a gap the curve continues invisibly — approximate the
                // arc step with the local slope we last saw.
                None => (false, 1.0),
            };
            if on == cur_on {
                run += ds;
            } else {
                if cur_on { runs_on.push(run) } else { runs_off.push(run) }
                cur_on = on;
                run = ds;
            }
        }
        if cur_on { runs_on.push(run) } else { runs_off.push(run) }

        let median = |v: &mut Vec<f64>| -> f64 {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            if v.is_empty() { 0.0 } else { v[v.len() / 2] }
        };
        let needles = runs_on.iter().filter(|&&l| l <= 3.0).count();
        let n_on = runs_on.len();
        let med_on = median(&mut runs_on);
        let med_off = median(&mut runs_off);

        // Dash pattern is [8 on, 4 off] in arc px (line caps blur ±~2 px).
        assert!(
            (5.0..=12.0).contains(&med_on),
            "median on-run {med_on:.1} arc-px, expected ≈8 (runs: {runs_on:?})"
        );
        assert!(
            (2.0..=7.0).contains(&med_off),
            "median off-run {med_off:.1} arc-px, expected ≈4 (runs: {runs_off:?})"
        );
        assert!(
            needles * 5 <= n_on.max(1),
            "{needles}/{n_on} on-runs are needle fragments (≤3 arc-px): {runs_on:?}"
        );
    }

    #[test]
    fn renderer_init_and_add_column() {
        let inst = create_instance();
        let Ok(adapter) = request_adapter(&inst) else { return; };
        let Ok((device, queue)) = request_device(&adapter) else { return; };
        let device = Arc::new(device);
        let queue = Arc::new(queue);

        let mut r = Renderer::try_new(
            RendererDevice::new(Arc::clone(&device), Arc::clone(&queue)),
            wgpu::TextureFormat::Bgra8Unorm,
            1024 * 1024,
        ).unwrap();

        let c = col_f64((0..100).map(|i| i as f64).collect());
        let h = r.add_column("x", &c).unwrap();
        assert_eq!(h.len_values, 100);

        let h2 = r.handle_for("x").unwrap();
        assert!(r.is_valid_handle(&h2));

        // Unknown id → UnknownColumn.
        let res = r.handle_for("nope");
        assert!(matches!(res, Err(FiggyError::UnknownColumn { .. })));
    }
}
