//! The GPU device behind a renderer: adapter and device creation, device
//! capabilities, the render target and its MSAA resolve, the per-target
//! pipelines, pipeline prewarming, GPU completion servicing and the browser
//! error scopes used on WebGPU.

use super::*;

/// A wgpu device/queue pair used by figgy.
///
/// wgpu does not expose a public parent-device id on `Queue`, so figgy cannot
/// prove at runtime that arbitrary handles came from the same logical device.
/// Passing them as one value keeps the pair together throughout the renderer
/// API and avoids accepting loose device/queue arguments at every call site.
#[derive(Clone)]
pub struct RendererDevice {
    pub(super) device: Arc<wgpu::Device>,
    pub(super) queue: Arc<wgpu::Queue>,
}

impl RendererDevice {
    /// Bundle a wgpu device and queue that came from the same `request_device`
    /// result or host render state.
    pub fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Self {
        Self { device, queue }
    }

    pub fn device(&self) -> &Arc<wgpu::Device> {
        &self.device
    }
    pub fn queue(&self) -> &Arc<wgpu::Queue> {
        &self.queue
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RendererDeviceCaps {
    pub(super) features: wgpu::Features,
    pub(super) max_texture_dimension_2d: u32,
    pub(super) max_buffer_size: u64,
    /// The arc-length scan and star pass bind the WHOLE pool as a single
    /// storage-buffer binding, so the pool can never exceed this. A request
    /// above it is clamped at construction (see [`effective_pool_capacity`])
    /// rather than deferred into a wgpu validation panic on the first
    /// dashed/sketch/milkyway line.
    pub(super) max_storage_buffer_binding_size: u64,
    /// Per-dimension dispatch ceiling for the arc-length compute scan.
    pub(super) max_compute_workgroups_per_dimension: u32,
    /// The arc scan needs 256-wide workgroups; downlevel (GL-class)
    /// adapters report smaller/zero compute limits.
    pub(super) max_compute_invocations_per_workgroup: u32,
}

impl RendererDeviceCaps {
    pub(super) fn from_device(device: &wgpu::Device) -> Self {
        let limits = device.limits();
        Self {
            features: device.features(),
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            max_buffer_size: limits.max_buffer_size,
            max_storage_buffer_binding_size: limits.max_storage_buffer_binding_size,
            max_compute_workgroups_per_dimension: limits.max_compute_workgroups_per_dimension,
            max_compute_invocations_per_workgroup: limits.max_compute_invocations_per_workgroup,
        }
    }
}

/// The largest pool the renderer can actually bind, given the device.
///
/// The arc-scan compute and the Milkyway star pass bind the entire pool
/// buffer as one storage binding, so the pool is capped by BOTH
/// `max_storage_buffer_binding_size` and `max_buffer_size`. `try_new` clamps
/// the caller's requested capacity to this instead of letting an oversized
/// pool become a runtime validation panic — a caller that needs a bigger
/// pool must request higher device limits when creating the device (wgpu's
/// defaults are only 128 MiB / 256 MiB). Callers can read the granted size
/// back via `renderer.pool().capacity()`.
///
/// This clamp is a convenience for the caller, not the enforcement point. The
/// pool applies the same combined ceiling at every buffer it creates, reading
/// the device's limits there rather than trusting a number captured once —
/// see `column_pool::buffer_ceiling`.
pub(super) fn effective_pool_capacity(requested: u64, caps: RendererDeviceCaps) -> u64 {
    requested
        .min(caps.max_storage_buffer_binding_size)
        .min(caps.max_buffer_size)
}

pub(super) fn issue_renderer_identity() -> Result<u64> {
    static NEXT_RENDERER_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT_RENDERER_ID
        .try_update(
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
            |current| current.checked_add(1),
        )
        .map(|previous| previous + 1)
        .map_err(|_| FiggyError::CounterExhausted {
            counter: "renderer identity",
        })
}

pub(super) fn validate_target_format(
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

pub(super) fn validate_target_sample_count(
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

pub(super) fn preferred_msaa_sample_count(
    caps: RendererDeviceCaps,
    format: wgpu::TextureFormat,
) -> u32 {
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

/// Whether a series draws a surface on a 3D chart: a heatmap kind.
pub(super) fn draws_surface(rt: &crate::data_config::DataRenderType) -> bool {
    matches!(
        rt,
        crate::data_config::DataRenderType::Heatmap { .. }
            | crate::data_config::DataRenderType::HeatmapContour { .. }
    )
}

/// Whether a series draws contours on a 3D chart's floor.
pub(super) fn draws_floor(rt: &crate::data_config::DataRenderType) -> bool {
    matches!(
        rt,
        crate::data_config::DataRenderType::Contour { .. }
            | crate::data_config::DataRenderType::HeatmapContour { .. }
    )
}

/// The ternary pipelines (`TargetPipelines::ternary`).
pub(super) const TERNARY_PIPELINES: usize = 5;

/// One of a ternary chart's pipelines (`TargetPipelines::ternary`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TernaryPipeline {
    Line = 0,
    Scatter = 1,
    ScatterMapped = 2,
    Ring = 3,
    RingMapped = 4,
}

impl TernaryPipeline {
    pub(super) const ALL: [Self; TERNARY_PIPELINES] = [
        Self::Line,
        Self::Scatter,
        Self::ScatterMapped,
        Self::Ring,
        Self::RingMapped,
    ];

    fn stage(self) -> &'static str {
        match self {
            Self::Line => "ternary line",
            Self::Scatter => "ternary scatter",
            Self::ScatterMapped => "mapped ternary scatter",
            Self::Ring => "ternary picked point ring",
            Self::RingMapped => "mapped ternary picked point ring",
        }
    }
}

/// The ternary pipeline `kind` over the layouts `[transform, style, style
/// map, value map]` (the plain ones read the first two).
fn create_ternary_pipeline(
    kind: TernaryPipeline,
    device: &wgpu::Device,
    shaders: &data_render::ShaderModules,
    [transform_bgl, style_bgl, style_map_bgl, value_map_bgl]: [&wgpu::BindGroupLayout; 4],
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    match kind {
        TernaryPipeline::Line => data_render::create_line_ternary_pipeline_with_sample_count(
            device,
            &shaders.line,
            transform_bgl,
            style_bgl,
            surface_format,
            sample_count,
        ),
        TernaryPipeline::Scatter | TernaryPipeline::Ring => {
            let (vs, fs, label) = if kind == TernaryPipeline::Scatter {
                ("vs_ternary", "fs_main", "figgy scatter ternary pipeline")
            } else {
                (
                    "vs_pick_ring_ternary",
                    "fs_pick_ring",
                    "figgy picked point ternary ring pipeline",
                )
            };
            data_render::create_scatter_ternary_pipeline(
                device,
                &shaders.scatter,
                transform_bgl,
                style_bgl,
                surface_format,
                sample_count,
                vs,
                fs,
                label,
            )
        }
        TernaryPipeline::ScatterMapped | TernaryPipeline::RingMapped => {
            let (vs, fs, label) = if kind == TernaryPipeline::ScatterMapped {
                (
                    "vs_ternary_mapped",
                    "fs_mapped",
                    "figgy scatter ternary mapped pipeline",
                )
            } else {
                (
                    "vs_pick_ring_ternary_mapped",
                    "fs_pick_ring",
                    "figgy picked point ternary mapped ring pipeline",
                )
            };
            data_render::create_scatter_ternary_mapped_pipeline(
                device,
                &shaders.scatter,
                transform_bgl,
                style_bgl,
                style_map_bgl,
                value_map_bgl,
                surface_format,
                sample_count,
                vs,
                fs,
                label,
            )
        }
    }
}

/// The 3D point pipelines (`TargetPipelines::view3d`, #132).
pub(super) const VIEW3D_PIPELINES: usize = 8;

/// One of a 3D chart's scatter and line pipelines (`TargetPipelines::view3d`,
/// #132): the markers (`vs_3d`, `vs_3d_mapped`) and the stroke (`vs_3d`)
/// in the panel composite with the depth test, their 1×1 ID picks
/// (`vs_3d_pick`, `vs_3d_mapped_pick`, the line's `vs_3d_pick`) into the
/// integer pick target, and the picked ring in the live slot
/// (`vs_pick_ring_3d`, `vs_pick_ring_3d_mapped`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum View3dPipeline {
    Scatter = 0,
    ScatterMapped = 1,
    ScatterPick = 2,
    ScatterMappedPick = 3,
    Ring = 4,
    RingMapped = 5,
    Line = 6,
    LinePick = 7,
}

impl View3dPipeline {
    pub(super) const ALL: [Self; VIEW3D_PIPELINES] = [
        Self::Scatter,
        Self::ScatterMapped,
        Self::ScatterPick,
        Self::ScatterMappedPick,
        Self::Ring,
        Self::RingMapped,
        Self::Line,
        Self::LinePick,
    ];

    fn stage(self) -> &'static str {
        match self {
            Self::Scatter => "3D scatter",
            Self::ScatterMapped => "mapped 3D scatter",
            Self::ScatterPick => "3D scatter pick",
            Self::ScatterMappedPick => "mapped 3D scatter pick",
            Self::Ring => "3D picked point ring",
            Self::RingMapped => "mapped 3D picked point ring",
            Self::Line => "3D line",
            Self::LinePick => "3D line pick",
        }
    }

    fn pass(self) -> data_render::View3dPass {
        match self {
            Self::Scatter | Self::ScatterMapped | Self::Line => data_render::View3dPass::Draw,
            Self::ScatterPick | Self::ScatterMappedPick | Self::LinePick => {
                data_render::View3dPass::Pick
            }
            Self::Ring | Self::RingMapped => data_render::View3dPass::Live,
        }
    }

    fn mapped(self) -> bool {
        matches!(
            self,
            Self::ScatterMapped | Self::ScatterMappedPick | Self::RingMapped
        )
    }
}

/// The 3D point pipeline `kind` over the layouts `[transform, style, style
/// map, camera, value map and camera]`: the precise entries bind the
/// camera group (the surface layout) as group 3, the mapped ones the
/// combined value map and camera group. The pick kinds render one sample
/// into the ID target; the others `sample_count` into `surface_format`.
fn create_view3d_pipeline(
    kind: View3dPipeline,
    device: &wgpu::Device,
    shaders: &data_render::ShaderModules,
    [
        transform_bgl,
        style_bgl,
        style_map_bgl,
        surface_bgl,
        view3d_value_map_bgl,
    ]: [&wgpu::BindGroupLayout; 5],
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let pass = kind.pass();
    let sample_count = if pass == data_render::View3dPass::Pick {
        1
    } else {
        sample_count
    };
    match kind {
        View3dPipeline::Line | View3dPipeline::LinePick => {
            data_render::create_line_view3d_pipeline_with_sample_count(
                device,
                &shaders.line,
                transform_bgl,
                style_bgl,
                surface_bgl,
                surface_format,
                sample_count,
                pass,
            )
        }
        _ if !kind.mapped() => {
            let (vs, fs, label) = match kind {
                View3dPipeline::ScatterPick => {
                    ("vs_3d_pick", "fs_3d_pick", "figgy scatter 3D pick pipeline")
                }
                View3dPipeline::Ring => (
                    "vs_pick_ring_3d",
                    "fs_pick_ring",
                    "figgy picked point 3D ring pipeline",
                ),
                _ => ("vs_3d", "fs_main", "figgy scatter 3D pipeline"),
            };
            data_render::create_scatter_view3d_pipeline(
                device,
                &shaders.scatter,
                transform_bgl,
                style_bgl,
                surface_bgl,
                surface_format,
                sample_count,
                pass,
                vs,
                fs,
                label,
            )
        }
        _ => {
            let (vs, fs, label) = match kind {
                View3dPipeline::ScatterMappedPick => (
                    "vs_3d_mapped_pick",
                    "fs_3d_mapped_pick",
                    "figgy scatter 3D mapped pick pipeline",
                ),
                View3dPipeline::RingMapped => (
                    "vs_pick_ring_3d_mapped",
                    "fs_pick_ring",
                    "figgy picked point 3D mapped ring pipeline",
                ),
                _ => (
                    "vs_3d_mapped",
                    "fs_mapped",
                    "figgy scatter 3D mapped pipeline",
                ),
            };
            data_render::create_scatter_view3d_mapped_pipeline(
                device,
                &shaders.scatter,
                transform_bgl,
                style_bgl,
                style_map_bgl,
                view3d_value_map_bgl,
                surface_format,
                sample_count,
                pass,
                vs,
                fs,
                label,
            )
        }
    }
}

/// Pipelines compiled against one render-target format. Axis is created with
/// the target; ordinary renderer use creates the other variants lazily, while
/// explicit full prewarm materializes every optional field before interaction.
pub(super) struct TargetPipelines {
    pub(super) shaders: data_render::ShaderModules,
    pub(super) axis: wgpu::RenderPipeline,
    pub(super) line: Option<wgpu::RenderPipeline>,
    /// The colour-mapped stroke (`vs_main_mapped` / `fs_main_mapped`) of a
    /// streamline series with a colour column: the line slots plus the value
    /// column, group 3 the point value map.
    pub(super) line_mapped: Option<wgpu::RenderPipeline>,
    /// The line shader's `vs_area` / `fs_area` fill, compiled when a line
    /// series declares a fill. Used in every draw style: the fill is a flat
    /// precise primitive under whatever the style draws for the line.
    pub(super) area: Option<wgpu::RenderPipeline>,
    pub(super) scatter: Option<wgpu::RenderPipeline>,
    pub(super) scatter_mapped: Option<wgpu::RenderPipeline>,
    pub(super) pick_ring: Option<wgpu::RenderPipeline>,
    pub(super) pick_ring_mapped: Option<wgpu::RenderPipeline>,
    /// A ternary chart's entries (`Config.ternary`), indexed by
    /// [`TernaryPipeline`]: the stroke (`vs_ternary`), the markers
    /// (`vs_ternary`, `vs_ternary_mapped`) and the picked ring
    /// (`vs_pick_ring_ternary`, `vs_pick_ring_ternary_mapped`), each the
    /// plain pipeline with the third component's vertex slot.
    pub(super) ternary: [Option<wgpu::RenderPipeline>; TERNARY_PIPELINES],
    /// A 3D chart's scatter and line entries (#132), indexed by
    /// [`View3dPipeline`]: the depth-tested draws, their ID picks and the
    /// picked rings, each the plain pipeline with the height's vertex slot
    /// and the camera group.
    pub(super) view3d: [Option<wgpu::RenderPipeline>; VIEW3D_PIPELINES],
    /// A 3D chart's surface module and pipelines, indexed by
    /// [`data_render::surface_mesh::SurfacePass`]: the depth-tested draw in
    /// the panel composite, the picked patch in the live slot and the ID
    /// pick. Compiled only when a 3D chart is drawn.
    pub(super) surface_shader: Option<wgpu::ShaderModule>,
    pub(super) surface: [Option<wgpu::RenderPipeline>; 5],
    /// A ternary series' region fill: the module and its stencil and cover
    /// pipelines, indexed by
    /// [`data_render::ternary_region::TernaryRegionPass`], drawn into the
    /// panel composite's stencil attachment. Compiled only when a ternary
    /// chart with a region fill is drawn.
    pub(super) ternary_region_shader: Option<wgpu::ShaderModule>,
    pub(super) ternary_region: [Option<wgpu::RenderPipeline>; 2],
    pub(super) errorbar: Option<wgpu::RenderPipeline>,
    pub(super) errorbar_mapped: Option<wgpu::RenderPipeline>,
    pub(super) bar: Option<wgpu::RenderPipeline>,
    pub(super) bar_envelope: Option<data_render::bar_envelope::Pipelines>,
    pub(super) bar_mapped: Option<wgpu::RenderPipeline>,
    pub(super) bar_selection: Option<wgpu::RenderPipeline>,
    /// The selected bin's outline over the `bar_mapped` layout: group 2 is
    /// the series' bar style map, so the outline resolves the bin's overrides
    /// as the mapped draw does (`vs_bar_selection_mapped`).
    pub(super) bar_selection_mapped: Option<wgpu::RenderPipeline>,
    /// The category bar draw (`vs_category_bars` / `fs_category_bars`).
    pub(super) bar_category: Option<wgpu::RenderPipeline>,
    /// The picked category bar's highlight over the same layout.
    pub(super) bar_category_selection: Option<wgpu::RenderPipeline>,
    /// The box plot draw (`vs_box_plot` / `fs_box_plot`) over the category
    /// bar layout.
    pub(super) bar_box_plot: Option<wgpu::RenderPipeline>,
    /// The picked box's highlight over the same layout.
    pub(super) bar_box_plot_selection: Option<wgpu::RenderPipeline>,
    /// The range bar draw (`vs_range_bars` / `fs_range_bars`) over the
    /// category bar layout.
    pub(super) bar_range: Option<wgpu::RenderPipeline>,
    /// The picked range bar's highlight over the same layout.
    pub(super) bar_range_selection: Option<wgpu::RenderPipeline>,
    /// A box plot's violin bodies (`vs_violin` / `fs_violin`) over the
    /// category bar layout.
    pub(super) bar_violin: Option<wgpu::RenderPipeline>,
    /// The picked violin's highlight over the same layout.
    pub(super) bar_violin_selection: Option<wgpu::RenderPipeline>,
    /// A box plot's raw points (`vs_box_points` / `fs_box_points`) over the
    /// category bar layout.
    pub(super) bar_box_points: Option<wgpu::RenderPipeline>,
    /// The picked raw point's ring over the same layout.
    pub(super) bar_box_point_selection: Option<wgpu::RenderPipeline>,
    /// The value labels of category bar, box plot and radar series
    /// (`vs_value_label` / `fs_value_label`), group 1 the series' value
    /// label group.
    pub(super) bar_value_label: Option<wgpu::RenderPipeline>,
    /// The selected value label's box over the same layout.
    pub(super) bar_value_label_selection: Option<wgpu::RenderPipeline>,
    pub(super) field: Option<wgpu::RenderPipeline>,
    pub(super) field_selection: Option<wgpu::RenderPipeline>,
    /// The selected contour level's highlight for a labelled contour, with
    /// group 3 bound to its label gaps (`fs_data_selection_labelled`).
    pub(super) field_selection_labelled: Option<wgpu::RenderPipeline>,
    /// The field quad through `fs_contour` — the isolines. Same module and same
    /// layouts as `field`, one entry point apart.
    pub(super) contour: Option<wgpu::RenderPipeline>,
    /// The same contour pass with group 3 bound to selected label anchors, so
    /// the stroke is omitted under the label quads.
    pub(super) contour_labelled: Option<wgpu::RenderPipeline>,
    /// The contour label draw. Compiled from `Renderer::contour_label_pipelines`,
    /// which owns the module and the group-1 layout.
    pub(super) contour_label: Option<wgpu::RenderPipeline>,
    /// The assembled vector pipelines, one per input form (`vector.rs`).
    pub(super) vector_points: Option<wgpu::RenderPipeline>,
    pub(super) vector_grid: Option<wgpu::RenderPipeline>,
    /// The streamline arrowheads (`VectorInput::Path`), drawn indirect over
    /// the arc slots.
    pub(super) vector_path: Option<wgpu::RenderPipeline>,
    pub(super) sample_count: u32,
    pub(super) styled: HashMap<StyleKey, StyleSet>,
}

impl TargetPipelines {
    /// Prepare-phase ensure (`&mut`): compile and cache the pipeline set of
    /// every style that `items` draw with and the cache doesn't hold yet.
    /// Run before pass recording so the draw phase (`&self`) only looks up.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ensure_styles_for_items(
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
            let Some(v) = style_variant(&item.chart_config.draw_style) else {
                continue;
            };
            self.styled.entry(v.key).or_insert_with(|| {
                create_style_set(
                    v.key,
                    device,
                    queue,
                    &self.shaders,
                    transform_bgl,
                    style_bgl,
                    star_data_bgl,
                    surface_format,
                    self.sample_count,
                )
            });
        }
    }

    /// Compile the surface module and its three pipelines when a 3D chart
    /// among `items` draws a heatmap as a surface (`Config.view_3d`). The
    /// draw and the picked patch target this set's format and sample count;
    /// the ID pick its own one-sample `Rgba32Uint`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ensure_surface_pipelines(
        &mut self,
        device: &wgpu::Device,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        surface_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        items: &[ChartDrawItem<'_>],
    ) {
        let needed = items.iter().any(|item| {
            item.chart_config.view_3d.is_some()
                && item.series.iter().any(|series| {
                    draws_surface(&series.config.render_type)
                        || draws_floor(&series.config.render_type)
                })
        });
        if !needed {
            return;
        }
        for pass in data_render::surface_mesh::SurfacePass::ALL {
            self.ensure_surface_pass(
                device,
                [transform_bgl, style_bgl, field_bgl, surface_bgl],
                surface_format,
                pass,
            );
        }
    }

    /// Compile the ternary region module and its two pipelines when a
    /// ternary chart among `items` draws a series with a region fill.
    pub(super) fn ensure_ternary_region_pipelines(
        &mut self,
        device: &wgpu::Device,
        transform_bgl: &wgpu::BindGroupLayout,
        region_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        items: &[ChartDrawItem<'_>],
    ) {
        let needed = items.iter().any(|item| {
            item.chart_config.ternary.is_some()
                && item
                    .series
                    .iter()
                    .any(|series| series.config.render_type.ternary_region().is_some())
        });
        if !needed {
            return;
        }
        for pass in data_render::ternary_region::TernaryRegionPass::ALL {
            self.ensure_ternary_region_pass(
                device,
                transform_bgl,
                region_bgl,
                surface_format,
                pass,
            );
        }
    }

    /// Compile the ternary region module and the one pipeline `pass` when
    /// they are missing.
    pub(super) fn ensure_ternary_region_pass(
        &mut self,
        device: &wgpu::Device,
        transform_bgl: &wgpu::BindGroupLayout,
        region_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        pass: data_render::ternary_region::TernaryRegionPass,
    ) {
        if self.ternary_region[pass as usize].is_some() {
            return;
        }
        let shader = self.ternary_region_shader.get_or_insert_with(|| {
            crate::gpu_compile::shader_module(
                device,
                wgpu::ShaderModuleDescriptor {
                    label: Some("figgy ternary region shader"),
                    source: wgpu::ShaderSource::Wgsl(
                        data_render::ternary_region::TERNARY_REGION_SHADER
                            .source
                            .into(),
                    ),
                },
            )
        });
        self.ternary_region[pass as usize] =
            Some(data_render::ternary_region::create_ternary_region_pipeline(
                device,
                shader,
                transform_bgl,
                region_bgl,
                surface_format,
                self.sample_count,
                pass,
            ));
    }

    /// Compile the one 3D point pipeline `kind` when it is missing (#132);
    /// `bgls` are the transform, style, style map, surface and combined
    /// value map layouts.
    pub(super) fn ensure_view3d_pass(
        &mut self,
        device: &wgpu::Device,
        bgls: [&wgpu::BindGroupLayout; 5],
        surface_format: wgpu::TextureFormat,
        kind: View3dPipeline,
    ) {
        if self.view3d[kind as usize].is_some() {
            return;
        }
        self.view3d[kind as usize] = Some(create_view3d_pipeline(
            kind,
            device,
            &self.shaders,
            bgls,
            surface_format,
            self.sample_count,
        ));
    }

    /// Compile the surface module and the one pipeline `pass` when they are
    /// missing; `bgls` are the transform, style, field and surface layouts.
    pub(super) fn ensure_surface_pass(
        &mut self,
        device: &wgpu::Device,
        bgls: [&wgpu::BindGroupLayout; 4],
        surface_format: wgpu::TextureFormat,
        pass: data_render::surface_mesh::SurfacePass,
    ) {
        let [transform_bgl, style_bgl, field_bgl, surface_bgl] = bgls;
        if self.surface[pass as usize].is_some() {
            return;
        }
        if pass.on_floor() {
            self.surface[pass as usize] = Some(data_render::surface_mesh::create_surface_pipeline(
                device,
                &self.shaders.field,
                transform_bgl,
                style_bgl,
                field_bgl,
                surface_bgl,
                surface_format,
                self.sample_count,
                pass,
            ));
            return;
        }
        let shader = self.surface_shader.get_or_insert_with(|| {
            crate::gpu_compile::shader_module(
                device,
                wgpu::ShaderModuleDescriptor {
                    label: Some("figgy surface shader"),
                    source: wgpu::ShaderSource::Wgsl(
                        data_render::surface_mesh::SURFACE_SHADER.source.into(),
                    ),
                },
            )
        });
        let slot = &mut self.surface[pass as usize];
        if slot.is_none() {
            *slot = Some(data_render::surface_mesh::create_surface_pipeline(
                device,
                shader,
                transform_bgl,
                style_bgl,
                field_bgl,
                surface_bgl,
                surface_format,
                self.sample_count,
                pass,
            ));
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn ensure_precise_variants_for_items(
        &mut self,
        device: &wgpu::Device,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        per_point_style_map_bgl: &wgpu::BindGroupLayout,
        point_value_map_bgl: &wgpu::BindGroupLayout,
        [surface_bgl, view3d_value_map_bgl]: [&wgpu::BindGroupLayout; 2],
        data_selection_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        vector_bgl: &wgpu::BindGroupLayout,
        star_data_bgl: &wgpu::BindGroupLayout,
        category_bar_data_bgl: &wgpu::BindGroupLayout,
        label_pipelines: Option<&crate::gpu_contour::ContourLabelPipelines>,
        surface_format: wgpu::TextureFormat,
        items: &[ChartDrawItem<'_>],
    ) {
        let mut need_vector_points = false;
        let mut need_vector_grid = false;
        let mut need_vector_path = false;
        let mut need_field = false;
        let mut need_contour = false;
        let mut need_contour_label = false;
        let mut need_line = false;
        let mut need_line_mapped = false;
        let mut need_area = false;
        let mut need_scatter = false;
        let mut need_errorbar = false;
        let mut need_bar = false;
        let mut need_bar_mapped = false;
        let mut need_bar_selection = false;
        let mut need_bar_selection_mapped = false;
        let mut need_category_bars = false;
        let mut need_category_bar_selection = false;
        let mut need_box_plot = false;
        let mut need_box_plot_selection = false;
        let mut need_range_bars = false;
        let mut need_range_bar_selection = false;
        let mut need_violin = false;
        let mut need_violin_selection = false;
        let mut need_box_points = false;
        let mut need_box_point_selection = false;
        let mut need_value_labels = false;
        let mut need_value_label_selection = false;
        let mut need_field_selection = false;
        let mut need_scatter_mapped = false;
        let mut need_errorbar_mapped = false;
        let mut need_pick_ring = false;
        let mut need_pick_ring_mapped = false;
        let mut need_ternary = [false; TERNARY_PIPELINES];
        let mut need_view3d = [false; VIEW3D_PIPELINES];
        for item in items {
            let typed_picks = item
                .chart_config
                .picked_data
                .as_ref()
                .filter(|config| config.visible && !config.refs.is_empty());
            let typed_point = typed_picks.is_some_and(|config| {
                config
                    .refs
                    .iter()
                    .any(|picked| matches!(picked, PickedDataRef::Point { .. }))
            });
            let typed_bin = typed_picks.is_some_and(|config| {
                config
                    .refs
                    .iter()
                    .any(|picked| matches!(picked, PickedDataRef::HistogramBin { .. }))
            });
            if let Some(config) = typed_picks {
                need_pick_ring |= typed_point;
                need_bar_selection |= typed_bin;
                need_field_selection |= config.refs.iter().any(|picked| {
                    matches!(
                        picked,
                        PickedDataRef::MatrixCell { .. } | PickedDataRef::ContourLevel { .. }
                    )
                });
            }
            // The area fill is drawn under every style's line with the one
            // precise area pipeline.
            for series in item.series {
                need_area |=
                    series_has_line_fill(&item.chart_config.draw_style, &series.config.render_type);
            }
            if style_variant(&item.chart_config.draw_style).is_some() {
                if item.chart_config.picked_points.is_some() {
                    need_pick_ring = true;
                }
                continue;
            }
            if item.chart_config.picked_points.is_some() {
                need_pick_ring = true;
            }
            for series in item.series {
                let primitives = effective_series_primitives(
                    &item.chart_config.draw_style,
                    &series.config.render_type,
                );
                need_field |= primitives.field;
                // A contour is the field quad through a different fragment entry
                // point, so it needs no line pipeline at all any more.
                need_contour |= primitives.contour;
                need_line |= primitives.line;
                // Streamline arrowheads draw through the path vector pipeline.
                need_vector_path |= primitives.line
                    && series
                        .config
                        .render_type
                        .streamlines()
                        .is_some_and(|streamlines| streamlines.arrows.is_some());
                need_line_mapped |= primitives.line
                    && series
                        .config
                        .render_type
                        .streamlines()
                        .is_some_and(|streamlines| streamlines.color_map.is_some());
                need_contour_label |=
                    primitives.contour && series_wants_contour_labels(&series.config.render_type);
                need_scatter |= primitives.scatter;
                need_bar |= primitives.bar;
                need_category_bars |= primitives.category_bars;
                need_category_bar_selection |= primitives.category_bars && typed_bin;
                need_box_plot |= primitives.box_plot;
                need_box_plot_selection |= primitives.box_plot && typed_bin;
                need_range_bars |= primitives.range_bars;
                need_range_bar_selection |= primitives.range_bars && typed_bin;
                need_violin |= primitives.violin;
                need_violin_selection |= primitives.violin && typed_bin;
                need_box_points |= primitives.box_points;
                need_box_point_selection |= primitives.box_points && typed_point;
                // Value labels compile with any label the series can show;
                // `Emphasized` radar labels with or without a selection.
                let labels =
                    data_render::value_label_kind(&series.config.render_type, true).is_some();
                need_value_labels |= labels;
                need_value_label_selection |= labels
                    && typed_picks.is_some_and(|config| {
                        config.refs.iter().any(|picked| {
                            matches!(picked, PickedDataRef::ValueLabel { series_id, .. }
                                if *series_id == series.config.series_id)
                        })
                    });
                if primitives.vector
                    && let Some((components, _)) = extract_vector(&series.config.render_type)
                {
                    match components.grid() {
                        Some(_) => need_vector_grid = true,
                        None => need_vector_points = true,
                    }
                    // A selected grid cell draws through the field selection
                    // pipeline; a selected point anchor through the ring.
                }
                need_field_selection |= primitives.vector
                    && typed_picks.is_some_and(|config| {
                        config
                            .refs
                            .iter()
                            .any(|picked| matches!(picked, PickedDataRef::MatrixCell { .. }))
                    });
                need_bar_mapped |= primitives.bar && series.style.bar_map.is_some();
                // The outline takes the mapped entry exactly when the draw does.
                need_bar_selection_mapped |=
                    primitives.bar && series.style.bar_map.is_some() && typed_bin;
                need_errorbar |= has_errorbar(&series.config.render_type);
                // `has_index` only means a style-index column is bound.
                // Sparse `*_style_overrides` still need the mapped shader.
                if primitives.scatter && series.style.scatter_map.is_some() {
                    need_scatter_mapped = true;
                    if item.chart_config.picked_points.is_some() || typed_point {
                        need_pick_ring_mapped = true;
                    }
                }
                // A ternary series draws through the ternary entries.
                if series.config.render_type.ternary_c_column().is_some() {
                    let mapped = series.style.scatter_map.is_some();
                    let ring = item.chart_config.picked_points.is_some() || typed_point;
                    need_ternary[TernaryPipeline::Line as usize] |= primitives.line;
                    need_ternary[TernaryPipeline::Scatter as usize] |=
                        primitives.scatter && !mapped;
                    need_ternary[TernaryPipeline::ScatterMapped as usize] |=
                        primitives.scatter && mapped;
                    need_ternary[TernaryPipeline::Ring as usize] |=
                        primitives.scatter && !mapped && ring;
                    need_ternary[TernaryPipeline::RingMapped as usize] |=
                        primitives.scatter && mapped && ring;
                }
                if has_errorbar(&series.config.render_type) && series.style.errorbar_map.is_some() {
                    need_errorbar_mapped = true;
                }
                // A scatter or line series on a 3D chart draws through the
                // 3D entries (#132); its picks compile when it is picked.
                if item.chart_config.view_3d.is_some() && series.config.z_column.is_some() {
                    let mapped = series.style.scatter_map.is_some();
                    let ring = item.chart_config.picked_points.is_some() || typed_point;
                    need_view3d[View3dPipeline::Line as usize] |= primitives.line;
                    need_view3d[View3dPipeline::Scatter as usize] |= primitives.scatter && !mapped;
                    need_view3d[View3dPipeline::ScatterMapped as usize] |=
                        primitives.scatter && mapped;
                    need_view3d[View3dPipeline::Ring as usize] |=
                        primitives.scatter && !mapped && ring;
                    need_view3d[View3dPipeline::RingMapped as usize] |=
                        primitives.scatter && mapped && ring;
                }
            }
        }
        if need_line && self.line.is_none() {
            self.line = Some(
                data_render::create_line_columnar_pipeline_with_sample_count(
                    device,
                    &self.shaders.line,
                    transform_bgl,
                    style_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        if need_line_mapped && self.line_mapped.is_none() {
            self.line_mapped = Some(data_render::create_line_mapped_pipeline_with_sample_count(
                device,
                &self.shaders.line,
                transform_bgl,
                style_bgl,
                point_value_map_bgl,
                surface_format,
                self.sample_count,
            ));
        }
        if need_area && self.area.is_none() {
            self.area = Some(data_render::create_line_area_pipeline_with_sample_count(
                device,
                &self.shaders.line,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            ));
        }
        if need_scatter && self.scatter.is_none() {
            self.scatter = Some(
                data_render::create_scatter_columnar_pipeline_with_sample_count(
                    device,
                    &self.shaders.scatter,
                    transform_bgl,
                    style_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        if need_field && self.field.is_none() {
            self.field = Some(
                data_render::create_field_columnar_pipeline_with_sample_count(
                    device,
                    &self.shaders.field,
                    transform_bgl,
                    style_bgl,
                    field_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        if need_contour && self.contour.is_none() {
            self.contour = Some(data_render::create_field_columnar_pipeline_with_entry(
                device,
                &self.shaders.field,
                transform_bgl,
                style_bgl,
                field_bgl,
                surface_format,
                self.sample_count,
                "fs_contour",
                "figgy field contour pipeline",
            ));
        }
        if need_contour_label
            && self.contour_labelled.is_none()
            && let Some(labels) = label_pipelines
        {
            self.contour_labelled = Some(data_render::create_labelled_contour_pipeline(
                device,
                &self.shaders.field,
                transform_bgl,
                style_bgl,
                field_bgl,
                labels.label_gap_bgl(),
                surface_format,
                self.sample_count,
            ));
        }
        if need_contour_label
            && self.contour_label.is_none()
            && let Some(labels) = label_pipelines
        {
            self.contour_label = Some(labels.render_pipeline(
                device,
                transform_bgl,
                surface_format,
                self.sample_count,
            ));
        }
        if need_bar && self.bar.is_none() {
            self.bar = Some(data_render::create_bar_columnar_pipeline_with_sample_count(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            ));
        }
        if (need_bar || need_bar_mapped) && self.bar_envelope.is_none() {
            self.bar_envelope = Some(data_render::bar_envelope::Pipelines::new(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                per_point_style_map_bgl,
                surface_format,
                self.sample_count,
            ));
        }
        if need_bar_mapped && self.bar_mapped.is_none() {
            self.bar_mapped = Some(
                data_render::create_bar_columnar_mapped_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        if need_bar_selection && self.bar_selection.is_none() {
            self.bar_selection = Some(
                data_render::create_bar_selection_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        if need_bar_selection_mapped && self.bar_selection_mapped.is_none() {
            self.bar_selection_mapped = Some(
                data_render::create_bar_selection_mapped_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        for (need, selection) in [
            (need_category_bars, false),
            (need_category_bar_selection, true),
        ] {
            let slot = if selection {
                &mut self.bar_category_selection
            } else {
                &mut self.bar_category
            };
            if need && slot.is_none() {
                *slot = Some(data_render::create_category_bar_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    category_bar_data_bgl,
                    surface_format,
                    self.sample_count,
                    selection,
                ));
            }
        }
        for (need, selection) in [(need_box_plot, false), (need_box_plot_selection, true)] {
            let slot = if selection {
                &mut self.bar_box_plot_selection
            } else {
                &mut self.bar_box_plot
            };
            if need && slot.is_none() {
                *slot = Some(data_render::create_box_plot_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    category_bar_data_bgl,
                    surface_format,
                    self.sample_count,
                    selection,
                ));
            }
        }
        for (need, selection) in [(need_range_bars, false), (need_range_bar_selection, true)] {
            let slot = if selection {
                &mut self.bar_range_selection
            } else {
                &mut self.bar_range
            };
            if need && slot.is_none() {
                *slot = Some(data_render::create_range_bar_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    category_bar_data_bgl,
                    surface_format,
                    self.sample_count,
                    selection,
                ));
            }
        }
        for (need, points, selection) in [
            (need_violin, false, false),
            (need_violin_selection, false, true),
            (need_box_points, true, false),
            (need_box_point_selection, true, true),
        ] {
            let slot = match (points, selection) {
                (false, false) => &mut self.bar_violin,
                (false, true) => &mut self.bar_violin_selection,
                (true, false) => &mut self.bar_box_points,
                (true, true) => &mut self.bar_box_point_selection,
            };
            if need && slot.is_none() {
                *slot = Some(data_render::create_violin_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    category_bar_data_bgl,
                    surface_format,
                    self.sample_count,
                    points,
                    selection,
                ));
            }
        }
        for (need, selection) in [
            (need_value_labels, false),
            (need_value_label_selection, true),
        ] {
            let slot = if selection {
                &mut self.bar_value_label_selection
            } else {
                &mut self.bar_value_label
            };
            if need && slot.is_none() {
                // An equal layout to the renderer's own: wgpu compares
                // layouts by their entries, so its bind groups fit.
                let value_label_bgl = data_render::create_value_label_bind_group_layout(device);
                *slot = Some(data_render::create_value_label_pipeline_with_sample_count(
                    device,
                    &self.shaders.bar,
                    transform_bgl,
                    &value_label_bgl,
                    per_point_style_map_bgl,
                    category_bar_data_bgl,
                    surface_format,
                    self.sample_count,
                    selection,
                ));
            }
        }
        for (need, input) in [
            (need_vector_points, data_render::vector::VectorInput::Points),
            (need_vector_grid, data_render::vector::VectorInput::Grid),
            (need_vector_path, data_render::vector::VectorInput::Path),
        ] {
            let slot = match input {
                data_render::vector::VectorInput::Points => &mut self.vector_points,
                data_render::vector::VectorInput::Grid => &mut self.vector_grid,
                data_render::vector::VectorInput::Path => &mut self.vector_path,
            };
            if need && slot.is_none() {
                *slot = Some(
                    data_render::vector::create_vector_pipeline_with_sample_count(
                        device,
                        self.shaders.vector(input),
                        transform_bgl,
                        style_bgl,
                        vector_bgl,
                        star_data_bgl,
                        surface_format,
                        self.sample_count,
                        input,
                    ),
                );
            }
        }
        if need_field_selection && self.field_selection.is_none() {
            self.field_selection = Some(
                data_render::create_field_selection_pipeline_with_sample_count(
                    device,
                    &self.shaders.field,
                    transform_bgl,
                    data_selection_bgl,
                    field_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        if need_field_selection
            && self.field_selection_labelled.is_none()
            && let Some(labels) = label_pipelines
        {
            self.field_selection_labelled = Some(
                data_render::create_field_selection_labelled_pipeline_with_sample_count(
                    device,
                    &self.shaders.field,
                    transform_bgl,
                    data_selection_bgl,
                    field_bgl,
                    labels.label_gap_bgl(),
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        if need_errorbar && self.errorbar.is_none() {
            self.errorbar = Some(
                data_render::create_errorbar_columnar_pipeline_with_sample_count(
                    device,
                    &self.shaders.errorbar,
                    transform_bgl,
                    style_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
        if need_scatter_mapped && self.scatter_mapped.is_none() {
            self.scatter_mapped = Some(
                data_render::create_scatter_columnar_mapped_pipeline_with_entries(
                    device,
                    &self.shaders.scatter,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    point_value_map_bgl,
                    surface_format,
                    self.sample_count,
                    "vs_mapped",
                    "fs_mapped",
                    "figgy scatter mapped pipeline",
                ),
            );
        }
        if need_pick_ring && self.pick_ring.is_none() {
            self.pick_ring = Some(data_render::create_scatter_columnar_pipeline_with_entries(
                device,
                &self.shaders.scatter,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
                "vs_pick_ring",
                "fs_pick_ring",
                "figgy picked point ring pipeline",
            ));
        }
        if need_pick_ring_mapped && self.pick_ring_mapped.is_none() {
            self.pick_ring_mapped = Some(
                data_render::create_scatter_columnar_mapped_pipeline_with_entries(
                    device,
                    &self.shaders.scatter,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    point_value_map_bgl,
                    surface_format,
                    self.sample_count,
                    "vs_pick_ring_mapped",
                    "fs_pick_ring",
                    "figgy picked point mapped ring pipeline",
                ),
            );
        }
        for kind in TernaryPipeline::ALL {
            if need_ternary[kind as usize] && self.ternary[kind as usize].is_none() {
                self.ternary[kind as usize] = Some(create_ternary_pipeline(
                    kind,
                    device,
                    &self.shaders,
                    [
                        transform_bgl,
                        style_bgl,
                        per_point_style_map_bgl,
                        point_value_map_bgl,
                    ],
                    surface_format,
                    self.sample_count,
                ));
            }
        }
        for kind in View3dPipeline::ALL {
            if need_view3d[kind as usize] {
                self.ensure_view3d_pass(
                    device,
                    [
                        transform_bgl,
                        style_bgl,
                        per_point_style_map_bgl,
                        surface_bgl,
                        view3d_value_map_bgl,
                    ],
                    surface_format,
                    kind,
                );
            }
        }
        if need_errorbar_mapped && self.errorbar_mapped.is_none() {
            self.errorbar_mapped = Some(
                data_render::create_errorbar_columnar_mapped_pipeline_from_shader(
                    device,
                    &self.shaders.errorbar,
                    transform_bgl,
                    style_bgl,
                    per_point_style_map_bgl,
                    surface_format,
                    self.sample_count,
                ),
            );
        }
    }

    /// Compile every target-format render pipeline, including variants that no
    /// current chart needs. Cold-start hosts use this before accepting their
    /// first chart so a later render-type/style switch cannot move shader
    /// compilation back onto the interaction that requested it.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn prewarm_all_observed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        per_point_style_map_bgl: &wgpu::BindGroupLayout,
        point_value_map_bgl: &wgpu::BindGroupLayout,
        data_selection_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        vector_bgl: &wgpu::BindGroupLayout,
        category_bar_data_bgl: &wgpu::BindGroupLayout,
        star_data_bgl: &wgpu::BindGroupLayout,
        label_pipelines: Option<&crate::gpu_contour::ContourLabelPipelines>,
        feature: Option<PreparationFeature>,
        surface_format: wgpu::TextureFormat,
        observer: &mut dyn FnMut(InitEvent),
    ) {
        macro_rules! ensure {
            ($field:ident, $stage:literal, $create:expr) => {
                if self.$field.is_none()
                    && feature.is_none_or(|f| f.includes_field(stringify!($field)))
                {
                    self.$field = Some(
                        observe_value_async(observer, "renderer.prewarm", $stage, || $create).await,
                    );
                }
            };
        }

        ensure!(
            line,
            "precise line",
            data_render::create_line_columnar_pipeline_with_sample_count(
                device,
                &self.shaders.line,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            line_mapped,
            "colour-mapped line",
            data_render::create_line_mapped_pipeline_with_sample_count(
                device,
                &self.shaders.line,
                transform_bgl,
                style_bgl,
                point_value_map_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            area,
            "line area fill",
            data_render::create_line_area_pipeline_with_sample_count(
                device,
                &self.shaders.line,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            scatter,
            "precise scatter",
            data_render::create_scatter_columnar_pipeline_with_sample_count(
                device,
                &self.shaders.scatter,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            errorbar,
            "precise errorbar",
            data_render::create_errorbar_columnar_pipeline_with_sample_count(
                device,
                &self.shaders.errorbar,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            scatter_mapped,
            "mapped scatter",
            data_render::create_scatter_columnar_mapped_pipeline_with_entries(
                device,
                &self.shaders.scatter,
                transform_bgl,
                style_bgl,
                per_point_style_map_bgl,
                point_value_map_bgl,
                surface_format,
                self.sample_count,
                "vs_mapped",
                "fs_mapped",
                "figgy scatter mapped pipeline",
            )
        );
        ensure!(
            pick_ring,
            "picked point ring",
            data_render::create_scatter_columnar_pipeline_with_entries(
                device,
                &self.shaders.scatter,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
                "vs_pick_ring",
                "fs_pick_ring",
                "figgy picked point ring pipeline",
            )
        );
        ensure!(
            pick_ring_mapped,
            "mapped picked point ring",
            data_render::create_scatter_columnar_mapped_pipeline_with_entries(
                device,
                &self.shaders.scatter,
                transform_bgl,
                style_bgl,
                per_point_style_map_bgl,
                point_value_map_bgl,
                surface_format,
                self.sample_count,
                "vs_pick_ring_mapped",
                "fs_pick_ring",
                "figgy picked point mapped ring pipeline",
            )
        );
        for kind in TernaryPipeline::ALL {
            if self.ternary[kind as usize].is_none()
                && feature.is_none_or(|f| f.includes_field("ternary"))
            {
                self.ternary[kind as usize] = Some(
                    observe_value_async(observer, "renderer.prewarm", kind.stage(), || {
                        create_ternary_pipeline(
                            kind,
                            device,
                            &self.shaders,
                            [
                                transform_bgl,
                                style_bgl,
                                per_point_style_map_bgl,
                                point_value_map_bgl,
                            ],
                            surface_format,
                            self.sample_count,
                        )
                    })
                    .await,
                );
            }
        }
        ensure!(
            errorbar_mapped,
            "mapped errorbar",
            data_render::create_errorbar_columnar_mapped_pipeline_from_shader(
                device,
                &self.shaders.errorbar,
                transform_bgl,
                style_bgl,
                per_point_style_map_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            bar,
            "histogram bars",
            data_render::create_bar_columnar_pipeline_with_sample_count(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            bar_mapped,
            "mapped histogram bars",
            data_render::create_bar_columnar_mapped_pipeline_with_sample_count(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                per_point_style_map_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            bar_envelope,
            "histogram envelope",
            data_render::bar_envelope::Pipelines::new(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                per_point_style_map_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            bar_selection,
            "selected histogram bin",
            data_render::create_bar_selection_pipeline_with_sample_count(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            bar_selection_mapped,
            "mapped selected histogram bin",
            data_render::create_bar_selection_mapped_pipeline_with_sample_count(
                device,
                &self.shaders.bar,
                transform_bgl,
                style_bgl,
                per_point_style_map_bgl,
                surface_format,
                self.sample_count,
            )
        );
        for selection in [false, true] {
            let slot = if selection {
                &mut self.bar_category_selection
            } else {
                &mut self.bar_category
            };
            if slot.is_none() && feature.is_none_or(|f| f.includes_field("bar_category")) {
                *slot = Some(
                    observe_value_async(
                        observer,
                        "renderer.prewarm",
                        if selection {
                            "selected category bar"
                        } else {
                            "category bars"
                        },
                        || {
                            data_render::create_category_bar_pipeline_with_sample_count(
                                device,
                                &self.shaders.bar,
                                transform_bgl,
                                style_bgl,
                                per_point_style_map_bgl,
                                category_bar_data_bgl,
                                surface_format,
                                self.sample_count,
                                selection,
                            )
                        },
                    )
                    .await,
                );
            }
        }
        for selection in [false, true] {
            let slot = if selection {
                &mut self.bar_box_plot_selection
            } else {
                &mut self.bar_box_plot
            };
            if slot.is_none() && feature.is_none_or(|f| f.includes_field("bar_box_plot")) {
                *slot = Some(
                    observe_value_async(
                        observer,
                        "renderer.prewarm",
                        if selection {
                            "selected box"
                        } else {
                            "box plots"
                        },
                        || {
                            data_render::create_box_plot_pipeline_with_sample_count(
                                device,
                                &self.shaders.bar,
                                transform_bgl,
                                style_bgl,
                                per_point_style_map_bgl,
                                category_bar_data_bgl,
                                surface_format,
                                self.sample_count,
                                selection,
                            )
                        },
                    )
                    .await,
                );
            }
        }
        for selection in [false, true] {
            let slot = if selection {
                &mut self.bar_range_selection
            } else {
                &mut self.bar_range
            };
            if slot.is_none() && feature.is_none_or(|f| f.includes_field("bar_range")) {
                *slot = Some(
                    observe_value_async(
                        observer,
                        "renderer.prewarm",
                        if selection {
                            "selected range bar"
                        } else {
                            "range bars"
                        },
                        || {
                            data_render::create_range_bar_pipeline_with_sample_count(
                                device,
                                &self.shaders.bar,
                                transform_bgl,
                                style_bgl,
                                per_point_style_map_bgl,
                                category_bar_data_bgl,
                                surface_format,
                                self.sample_count,
                                selection,
                            )
                        },
                    )
                    .await,
                );
            }
        }
        for (points, selection) in [(false, false), (false, true), (true, false), (true, true)] {
            let (slot, field, label) = match (points, selection) {
                (false, false) => (&mut self.bar_violin, "bar_violin", "violins"),
                (false, true) => (
                    &mut self.bar_violin_selection,
                    "bar_violin",
                    "selected violin",
                ),
                (true, false) => (&mut self.bar_box_points, "bar_box_points", "box points"),
                (true, true) => (
                    &mut self.bar_box_point_selection,
                    "bar_box_points",
                    "selected box point",
                ),
            };
            if slot.is_none() && feature.is_none_or(|f| f.includes_field(field)) {
                *slot = Some(
                    observe_value_async(observer, "renderer.prewarm", label, || {
                        data_render::create_violin_pipeline_with_sample_count(
                            device,
                            &self.shaders.bar,
                            transform_bgl,
                            style_bgl,
                            per_point_style_map_bgl,
                            category_bar_data_bgl,
                            surface_format,
                            self.sample_count,
                            points,
                            selection,
                        )
                    })
                    .await,
                );
            }
        }
        for selection in [false, true] {
            let slot = if selection {
                &mut self.bar_value_label_selection
            } else {
                &mut self.bar_value_label
            };
            if slot.is_none() && feature.is_none_or(|f| f.includes_field("bar_value_label")) {
                let value_label_bgl = data_render::create_value_label_bind_group_layout(device);
                *slot = Some(
                    observe_value_async(
                        observer,
                        "renderer.prewarm",
                        if selection {
                            "selected value label"
                        } else {
                            "value labels"
                        },
                        || {
                            data_render::create_value_label_pipeline_with_sample_count(
                                device,
                                &self.shaders.bar,
                                transform_bgl,
                                &value_label_bgl,
                                per_point_style_map_bgl,
                                category_bar_data_bgl,
                                surface_format,
                                self.sample_count,
                                selection,
                            )
                        },
                    )
                    .await,
                );
            }
        }
        ensure!(
            field,
            "heatmap field",
            data_render::create_field_columnar_pipeline_with_sample_count(
                device,
                &self.shaders.field,
                transform_bgl,
                style_bgl,
                field_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            vector_points,
            "vector points",
            data_render::vector::create_vector_pipeline_with_sample_count(
                device,
                &self.shaders.vector_points,
                transform_bgl,
                style_bgl,
                vector_bgl,
                star_data_bgl,
                surface_format,
                self.sample_count,
                data_render::vector::VectorInput::Points,
            )
        );
        ensure!(
            vector_path,
            "vector path",
            data_render::vector::create_vector_pipeline_with_sample_count(
                device,
                &self.shaders.vector_path,
                transform_bgl,
                style_bgl,
                vector_bgl,
                star_data_bgl,
                surface_format,
                self.sample_count,
                data_render::vector::VectorInput::Path,
            )
        );
        ensure!(
            vector_grid,
            "vector grid",
            data_render::vector::create_vector_pipeline_with_sample_count(
                device,
                &self.shaders.vector_grid,
                transform_bgl,
                style_bgl,
                vector_bgl,
                star_data_bgl,
                surface_format,
                self.sample_count,
                data_render::vector::VectorInput::Grid,
            )
        );
        ensure!(
            contour,
            "contour field",
            data_render::create_field_columnar_pipeline_with_entry(
                device,
                &self.shaders.field,
                transform_bgl,
                style_bgl,
                field_bgl,
                surface_format,
                self.sample_count,
                "fs_contour",
                "figgy field contour pipeline",
            )
        );
        ensure!(
            contour_labelled,
            "label-gapped contour field",
            data_render::create_labelled_contour_pipeline(
                device,
                &self.shaders.field,
                transform_bgl,
                style_bgl,
                field_bgl,
                label_pipelines
                    .expect("field preparation owns labels")
                    .label_gap_bgl(),
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            field_selection,
            "selected field data",
            data_render::create_field_selection_pipeline_with_sample_count(
                device,
                &self.shaders.field,
                transform_bgl,
                data_selection_bgl,
                field_bgl,
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            field_selection_labelled,
            "selected labelled field data",
            data_render::create_field_selection_labelled_pipeline_with_sample_count(
                device,
                &self.shaders.field,
                transform_bgl,
                data_selection_bgl,
                field_bgl,
                label_pipelines
                    .expect("field preparation owns labels")
                    .label_gap_bgl(),
                surface_format,
                self.sample_count,
            )
        );
        ensure!(
            contour_label,
            "contour labels",
            label_pipelines
                .expect("field preparation owns labels")
                .render_pipeline(device, transform_bgl, surface_format, self.sample_count,)
        );

        for (key, stage) in [
            (StyleKey::Sketch, "hand-drawn style"),
            (StyleKey::Milkyway, "milkyway style"),
            (StyleKey::Constellation, "constellation style"),
        ] {
            if feature.is_some_and(|f| f.style_key() != Some(key)) || self.styled.contains_key(&key)
            {
                continue;
            }
            let set = observe_value_async(observer, "renderer.prewarm", stage, || {
                create_style_set(
                    key,
                    device,
                    queue,
                    &self.shaders,
                    transform_bgl,
                    style_bgl,
                    star_data_bgl,
                    surface_format,
                    self.sample_count,
                )
            })
            .await;
            self.styled.insert(key, set);
        }
    }

    /// Draw-phase lookup: the cached set for the item's style, `None` for
    /// precise mode. A cache miss (impossible once the prepare phase ran)
    /// also yields `None` — callers then fall back to the precise pipelines
    /// instead of panicking.
    pub(super) fn style_set(&self, style: &DrawStyle) -> Option<&StyleSet> {
        style_variant(style).and_then(|v| self.styled.get(&v.key))
    }
}

pub(super) fn create_target_pipelines(
    device: &wgpu::Device,
    texture_bgl: &wgpu::BindGroupLayout,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    _per_point_style_map_bgl: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
) -> TargetPipelines {
    let mut observer = |_| {};
    create_target_pipelines_observed(
        device,
        texture_bgl,
        transform_bgl,
        style_bgl,
        _per_point_style_map_bgl,
        surface_format,
        sample_count,
        &mut observer,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_target_pipelines_observed(
    device: &wgpu::Device,
    texture_bgl: &wgpu::BindGroupLayout,
    _transform_bgl: &wgpu::BindGroupLayout,
    _style_bgl: &wgpu::BindGroupLayout,
    _per_point_style_map_bgl: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
    observer: &mut dyn FnMut(InitEvent),
) -> TargetPipelines {
    let shaders = observe_value(observer, "renderer", "shader modules", || {
        data_render::ShaderModules::new(device)
    });
    TargetPipelines {
        vector_points: None,
        vector_grid: None,
        vector_path: None,
        axis: observe_value(
            observer,
            "renderer",
            "figgy fullscreen textured pipeline",
            || {
                data_render::create_fullscreen_textured_pipeline_with_sample_count(
                    device,
                    &shaders.fullscreen,
                    texture_bgl,
                    surface_format,
                    sample_count,
                )
            },
        ),
        line: None,
        line_mapped: None,
        area: None,
        scatter: None,
        scatter_mapped: None,
        pick_ring: None,
        pick_ring_mapped: None,
        ternary: Default::default(),
        view3d: Default::default(),
        surface_shader: None,
        surface: Default::default(),
        ternary_region_shader: None,
        ternary_region: Default::default(),
        errorbar: None,
        errorbar_mapped: None,
        bar: None,
        bar_envelope: None,
        bar_mapped: None,
        bar_selection: None,
        bar_selection_mapped: None,
        bar_category: None,
        bar_category_selection: None,
        bar_box_plot: None,
        bar_box_plot_selection: None,
        bar_range: None,
        bar_range_selection: None,
        bar_violin: None,
        bar_violin_selection: None,
        bar_box_points: None,
        bar_box_point_selection: None,
        bar_value_label: None,
        bar_value_label_selection: None,
        field: None,
        field_selection: None,
        field_selection_labelled: None,
        contour: None,
        contour_labelled: None,
        contour_label: None,
        shaders,
        sample_count,
        styled: HashMap::new(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn create_target_pipelines_observed_async(
    device: &wgpu::Device,
    texture_bgl: &wgpu::BindGroupLayout,
    _transform_bgl: &wgpu::BindGroupLayout,
    _style_bgl: &wgpu::BindGroupLayout,
    _per_point_style_map_bgl: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
    _minimal: bool,
    observer: &mut dyn FnMut(InitEvent),
) -> Result<TargetPipelines> {
    let shaders = observe_value_async(observer, "renderer", "shader modules", || {
        data_render::ShaderModules::new(device)
    })
    .await;
    #[cfg(target_arch = "wasm32")]
    data_render::prewarm_browser_render_feature(
        device,
        surface_format,
        sample_count,
        _minimal.then_some(PreparationFeature::Basic),
        observer,
    )
    .await
    .map_err(|reason| FiggyError::GpuResourceAllocationFailed {
        resource: "browser render pipeline prewarm",
        reason,
    })?;
    let axis = observe_value_async(
        observer,
        "renderer",
        "figgy fullscreen textured pipeline",
        || {
            data_render::create_fullscreen_textured_pipeline_with_sample_count(
                device,
                &shaders.fullscreen,
                texture_bgl,
                surface_format,
                sample_count,
            )
        },
    )
    .await;
    Ok(TargetPipelines {
        shaders,
        axis,
        vector_points: None,
        vector_grid: None,
        vector_path: None,
        line: None,
        line_mapped: None,
        area: None,
        scatter: None,
        scatter_mapped: None,
        pick_ring: None,
        pick_ring_mapped: None,
        ternary: Default::default(),
        view3d: Default::default(),
        surface_shader: None,
        surface: Default::default(),
        ternary_region_shader: None,
        ternary_region: Default::default(),
        errorbar: None,
        errorbar_mapped: None,
        bar: None,
        bar_envelope: None,
        bar_mapped: None,
        bar_selection: None,
        bar_selection_mapped: None,
        bar_category: None,
        bar_category_selection: None,
        bar_box_plot: None,
        bar_box_plot_selection: None,
        bar_range: None,
        bar_range_selection: None,
        bar_violin: None,
        bar_violin_selection: None,
        bar_box_points: None,
        bar_box_point_selection: None,
        bar_value_label: None,
        bar_value_label_selection: None,
        field: None,
        field_selection: None,
        field_selection_labelled: None,
        contour: None,
        contour_labelled: None,
        contour_label: None,
        sample_count,
        styled: HashMap::new(),
    })
}

pub(super) fn validate_texture_extent(
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

pub(super) fn validate_buffer_size(
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

pub(super) fn create_texture_checked(
    device: &wgpu::Device,
    desc: &wgpu::TextureDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Texture> {
    // gpu-alloc: caller
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.create_texture(desc))).map_err(
        |_| FiggyError::GpuResourceAllocationFailed {
            resource,
            reason: "wgpu Device::create_texture panicked".into(),
        },
    )
}

pub(super) fn create_buffer_checked(
    device: &wgpu::Device,
    desc: &wgpu::BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Buffer> {
    // gpu-alloc: caller
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.create_buffer(desc))).map_err(
        |_| FiggyError::GpuResourceAllocationFailed {
            resource,
            reason: "wgpu Device::create_buffer panicked".into(),
        },
    )
}

/// Allocation context for one column-pool call.
///
/// A free function over the fields rather than a `&self` method for two
/// reasons. It keeps the caller's `&mut self.pool` borrow disjoint from the
/// device/queue/ledger borrows, and it makes the out-of-pool total something
/// that is *read at the call* — there is no place to cache it, so a budget
/// check can never run against a stale sum. The pool adds its own current
/// capacity and the capacity it is about to allocate, so `external_bytes`
/// deliberately excludes the pool row.
///
/// The sum happens inside `map`, so with no ceiling declared it does not happen
/// at all: the pool reads the out-of-pool total only to compare it against a
/// ceiling, and `GpuBudget` keeps the two together precisely so there is no
/// unused total to compute or to fabricate.
pub(super) fn pool_alloc_ctx<'a>(
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    memory_budget: Option<u64>,
    ledger: &GpuLedger,
) -> data_render::GpuAllocCtx<'a> {
    data_render::GpuAllocCtx {
        device,
        queue,
        budget: memory_budget.map(|ceiling_bytes| data_render::GpuBudget {
            ceiling_bytes,
            external_bytes: ledger.total_bytes(),
        }),
    }
}

pub(super) struct MsaaTarget {
    /// Charged to the ledger for as long as this target lives; the sample
    /// count multiplies the bytes, which is why it is worth its own row.
    pub(super) _texture: TrackedTexture,
    pub(super) view: wgpu::TextureView,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_msaa_target(
    device: &wgpu::Device,
    ledger: &Arc<GpuLedger>,
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
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    };
    let texture = TrackedTexture::new(
        ledger,
        GpuResourceKind::MsaaTarget,
        create_texture_checked(device, &desc, label)?,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(Some(MsaaTarget {
        _texture: texture,
        view,
    }))
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
pub(super) struct BrowserGpuError {
    pub(super) name: String,
    pub(super) constructor_name: String,
    pub(super) message: String,
}

#[cfg(target_arch = "wasm32")]
impl BrowserGpuError {
    pub(super) fn from_js(error: &wasm_bindgen::JsValue) -> Self {
        use wasm_bindgen::JsValue;

        let string_property = |object: &JsValue, property: &str| {
            js_sys::Reflect::get(object, &JsValue::from_str(property))
                .ok()
                .and_then(|value| value.as_string())
                .filter(|value| !value.is_empty())
        };
        let constructor_name = js_sys::Reflect::get(error, &JsValue::from_str("constructor"))
            .ok()
            .and_then(|constructor| string_property(&constructor, "name"))
            .unwrap_or_else(|| "unknown".into());
        Self {
            name: string_property(error, "name").unwrap_or_else(|| "GPUError".into()),
            constructor_name,
            message: string_property(error, "message").unwrap_or_else(|| format!("{error:?}")),
        }
    }

    pub(super) fn describe(&self) -> String {
        format!(
            "{} (constructor {}): {}",
            self.name, self.constructor_name, self.message
        )
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) fn browser_gpu_device_method(
    device: &wgpu::webgpu::GpuDevice,
    name: &str,
) -> std::result::Result<js_sys::Function, String> {
    use wasm_bindgen::{JsCast, JsValue};

    js_sys::Reflect::get(device.as_ref(), &JsValue::from_str(name))
        .map_err(|error| format!("GPUDevice.{name} lookup failed: {error:?}"))?
        .dyn_into()
        .map_err(|_| format!("GPUDevice.{name} is not a function"))
}

#[cfg(target_arch = "wasm32")]
pub(super) fn browser_scope_failure(resource: &'static str, reason: String) -> FiggyError {
    FiggyError::GpuResourceAllocationFailed { resource, reason }
}

#[cfg(target_arch = "wasm32")]
pub(super) struct BrowserErrorScopes {
    pub(super) device: wgpu::webgpu::GpuDevice,
    pub(super) pop: js_sys::Function,
    pub(super) active: u8,
    pub(super) resource: &'static str,
}

#[cfg(target_arch = "wasm32")]
impl BrowserErrorScopes {
    pub(super) fn push(device: &wgpu::Device, resource: &'static str) -> Result<Self> {
        use wasm_bindgen::JsValue;

        let device = device.as_webgpu().cloned().ok_or_else(|| {
            browser_scope_failure(
                resource,
                "wgpu device is not backed by a browser GPUDevice".into(),
            )
        })?;
        let push = browser_gpu_device_method(&device, "pushErrorScope")
            .map_err(|reason| browser_scope_failure(resource, reason))?;
        let pop = browser_gpu_device_method(&device, "popErrorScope")
            .map_err(|reason| browser_scope_failure(resource, reason))?;
        let mut scopes = Self {
            device,
            pop,
            active: 0,
            resource,
        };
        for filter in ["out-of-memory", "internal"] {
            push.call1(scopes.device.as_ref(), &JsValue::from_str(filter))
                .map_err(|error| {
                    browser_scope_failure(
                        resource,
                        format!("GPUDevice.pushErrorScope({filter}) failed: {error:?}"),
                    )
                })?;
            scopes.active += 1;
        }
        Ok(scopes)
    }

    pub(super) fn request_pop(&mut self) -> std::result::Result<js_sys::Promise, String> {
        use wasm_bindgen::JsCast;

        debug_assert!(self.active > 0);
        let value = self
            .pop
            .call0(self.device.as_ref())
            .map_err(|error| format!("GPUDevice.popErrorScope failed: {error:?}"))?;
        // A successful call starts the pop and removes that scope from the
        // device stack even though its Promise is still pending.
        self.active -= 1;
        value
            .dyn_into::<js_sys::Promise>()
            .map_err(|_| "GPUDevice.popErrorScope did not return a Promise".to_owned())
    }

    pub(super) fn request_all(mut self) -> BrowserPendingErrorScopes {
        // Request both pops before returning to JS. Once popErrorScope has
        // returned its Promise, that scope no longer occupies the device
        // stack while the Promise is awaited.
        let internal = self.request_pop();
        let out_of_memory = self.request_pop();
        BrowserPendingErrorScopes {
            internal,
            out_of_memory,
            resource: self.resource,
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for BrowserErrorScopes {
    fn drop(&mut self) {
        // Dropping an async export also drops its locals. Start every remaining
        // pop synchronously; JS Promises keep running after their Rust handles
        // are discarded.
        while self.active > 0 {
            self.active -= 1;
            let _ = self.pop.call0(self.device.as_ref());
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) struct BrowserPendingErrorScopes {
    pub(super) internal: std::result::Result<js_sys::Promise, String>,
    pub(super) out_of_memory: std::result::Result<js_sys::Promise, String>,
    pub(super) resource: &'static str,
}

#[cfg(target_arch = "wasm32")]
impl BrowserPendingErrorScopes {
    pub(super) async fn finish(self) -> Result<()> {
        let internal = browser_await_error_scope(self.internal).await;
        let out_of_memory = browser_await_error_scope(self.out_of_memory).await;
        finish_browser_error_scopes(self.resource, internal, out_of_memory)
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) async fn browser_await_error_scope(
    request: std::result::Result<js_sys::Promise, String>,
) -> std::result::Result<Option<BrowserGpuError>, String> {
    use wasm_bindgen_futures::JsFuture;

    let error = JsFuture::from(request?)
        .await
        .map_err(|error| format!("GPUDevice.popErrorScope rejected: {error:?}"))?;
    if error.is_null() || error.is_undefined() {
        Ok(None)
    } else {
        Ok(Some(BrowserGpuError::from_js(&error)))
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) fn finish_browser_error_scopes(
    resource: &'static str,
    internal: std::result::Result<Option<BrowserGpuError>, String>,
    out_of_memory: std::result::Result<Option<BrowserGpuError>, String>,
) -> Result<()> {
    let mut failures = Vec::new();
    match internal {
        Ok(Some(error)) => failures.push(format!(
            "browser WebGPU internal error: {}",
            error.describe()
        )),
        Err(error) => failures.push(format!("browser internal scope pop failed: {error}")),
        Ok(None) => {}
    }
    match out_of_memory {
        Ok(Some(error)) => failures.push(format!(
            "browser WebGPU out-of-memory error: {}",
            error.describe()
        )),
        Err(error) => failures.push(format!("browser OOM scope pop failed: {error}")),
        Ok(None) => {}
    }

    if failures.is_empty() {
        return Ok(());
    }
    Err(browser_scope_failure(resource, failures.join("; ")))
}

impl Renderer {
    /// Initialize base Cartesian resources against the given device/queue pair.
    ///
    /// Creates the column pool, shared layouts/sampler, shader modules, unit
    /// quad and axis pipeline. Optional data/style and compute pipelines are
    /// lazy. Use [`Self::prewarm_all`] or detached preparation to install them
    /// before first use; construction does not prepare chart-specific data.
    ///
    /// `pool_capacity_bytes` is the requested total size of the GPU column
    /// pool (sum of all chart data). It is clamped to what the device can
    /// bind — the arc-scan compute binds the whole pool as one storage
    /// binding, so a request above `max_storage_buffer_binding_size` (or
    /// `max_buffer_size`) is reduced instead of panicking later; read the
    /// granted size back via [`Self::pool`]`().capacity()`. To get a larger
    /// pool, raise those limits when requesting the wgpu device (the defaults
    /// are only 128 MiB / 256 MiB). This initial slab is **not** subject to
    /// [`Self::set_memory_budget`] — the budget governs growth, and this size is
    /// the host's own declaration. `surface_format` is the final render
    /// target color format; the eager axis pipeline and any later lazy
    /// graphics pipelines are compiled against it.
    pub fn try_new(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
    ) -> Result<Self> {
        Self::try_new_with_sample_count(gpu, surface_format, pool_capacity_bytes, 1)
    }

    /// Initialize the same base resources as [`Self::try_new`] and report
    /// timestamp-free startup stage boundaries to `observer`.
    pub fn try_new_observed(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Self> {
        Self::try_new_with_sample_count_observed(
            gpu,
            surface_format,
            pool_capacity_bytes,
            1,
            observer,
        )
    }

    pub(super) fn try_new_with_sample_count(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
        target_sample_count: u32,
    ) -> Result<Self> {
        let mut observer = |_| {};
        Self::try_new_with_sample_count_observed(
            gpu,
            surface_format,
            pool_capacity_bytes,
            target_sample_count,
            &mut observer,
        )
    }

    pub(super) fn try_new_with_sample_count_observed(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
        target_sample_count: u32,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Self> {
        let RendererDevice { device, queue } = gpu;
        started(observer, "renderer", "capabilities");
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

        // Clamp the pool to what the arc-scan / star-pass storage bindings
        // can address — graceful fallback instead of a first-dashed-line
        // panic. The caller can read the granted size back via `pool()`.
        let pool_capacity = effective_pool_capacity(pool_capacity_bytes, caps);
        if pool_capacity < pool_capacity_bytes {
            eprintln!(
                "[figgy] pool capacity {pool_capacity_bytes} B exceeds device limits \
                 (max_storage_buffer_binding_size {}, max_buffer_size {}); \
                 clamped to {pool_capacity} B",
                caps.max_storage_buffer_binding_size, caps.max_buffer_size
            );
        }
        finished(observer, "renderer", "capabilities");
        // host-alloc: W1-a
        let gpu_ledger = Arc::new(GpuLedger::new());
        let pool = observe_result(observer, "renderer", "figgy column pool", || {
            ColumnPool::new(
                pool_alloc_ctx(&device, &queue, None, &gpu_ledger),
                pool_capacity,
            )
        })?;

        let texture_bgl = data_render::create_texture_bind_group_layout(&device);
        let transform_bgl = data_render::create_scatter_transform_bind_group_layout(&device);
        let style_bgl = data_render::create_style_bind_group_layout(&device);
        let per_point_style_map_bgl =
            data_render::create_per_point_style_map_bind_group_layout(&device);
        let point_value_map_bgl = data_render::create_point_value_map_bind_group_layout(&device);
        let (neutral_point_value_map_buffers, neutral_point_value_map_charge) =
            data_render::create_point_value_map_buffers(
                &device,
                &gpu_ledger,
                &data_render::PointValueMapParamsGpu::NEUTRAL,
                &[],
            );
        let neutral_point_value_map_bg = data_render::bind_point_value_map(
            &device,
            &point_value_map_bgl,
            &neutral_point_value_map_buffers,
        );
        let view3d_value_map_bgl =
            data_render::surface_mesh::create_view3d_value_map_bind_group_layout(&device);
        let data_selection_bgl = data_render::create_data_selection_bind_group_layout(&device);
        let star_data_bgl = data_render::create_star_data_bind_group_layout(&device);
        let field_bgl = data_render::create_field_data_bind_group_layout(&device);
        let surface_bgl = data_render::surface_mesh::create_surface_data_bind_group_layout(&device);
        let region_bgl =
            data_render::ternary_region::create_ternary_region_bind_group_layout(&device);
        let vector_bgl = data_render::vector::create_vector_data_bind_group_layout(&device);
        let category_bar_data_bgl =
            data_render::create_category_bar_data_bind_group_layout(&device);
        let value_label_bgl = data_render::create_value_label_bind_group_layout(&device);
        let field_cell_tables = data_render::create_field_cell_tables(&device, &gpu_ledger)?;

        let sampler = data_render::create_linear_sampler(&device);
        let quad_vb = TrackedBuffer::new(
            &gpu_ledger,
            GpuResourceKind::Uniform,
            data_render::create_unit_centered_quad_vertex_buffer(&device),
        );

        // Axis compiles here. Line/scatter/errorbar, mapped, pick-ring,
        // styled, and arc-scan pipelines compile on first prepare that needs
        // them. The errorbar extent engine requires explicit async preparation.
        let pipelines = create_target_pipelines_observed(
            &device,
            &texture_bgl,
            &transform_bgl,
            &style_bgl,
            &per_point_style_map_bgl,
            surface_format,
            target_sample_count,
            observer,
        );
        let renderer_identity = observe_result(observer, "renderer", "identity", || {
            issue_renderer_identity()
        })?;

        Ok(Self {
            device,
            queue,
            caps,
            pool,
            chart_states: HashMap::new(),
            streaming_sources: HashMap::new(),
            stream_revision_lineage: HashMap::new(),
            next_stream_source_fit_epoch: 0,
            stream_runtime: None,
            chart_order: Vec::new(),
            next_chart_id: 0,
            visual_revision: RenderRevision::initial(renderer_identity),
            renderer_identity,
            observed_font_generation: crate::text_render::font_generation(),
            pending_defrag: false,
            picker: RendererPicker::disabled(),
            errorbar_extent_engine: None,
            target_sample_count,
            target_pipeline_generation: 1,
            texture_bgl,
            transform_bgl,
            style_bgl,
            per_point_style_map_bgl,
            point_value_map_bgl,
            neutral_point_value_map_bg,
            neutral_point_value_map_buffers,
            _neutral_point_value_map_charge: neutral_point_value_map_charge,
            view3d_value_map_bgl,
            data_selection_bgl,
            star_data_bgl,
            field_bgl,
            surface_bgl,
            region_bgl,
            vector_bgl,
            category_bar_data_bgl,
            value_label_bgl,
            field_cell_tables,
            pipelines,
            sampler,
            quad_vb,
            arc_cache: HashMap::new(),
            resident_displays: Vec::new(),
            resident_display_serial: 0,
            field_cache: HashMap::new(),
            value_map_cache: HashMap::new(),
            vector_cache: HashMap::new(),
            contour_label_pipelines: None,
            contour_cache: HashMap::new(),
            arc_pipelines: None,
            #[cfg(test)]
            arc_chunk_override: None,
            space_bg: None,
            surface_format,
            gpu_ledger,
            memory_budget: None,
            auto_resident_working_set_limit: None,
        })
    }

    pub(super) async fn try_new_with_sample_count_observed_async(
        gpu: RendererDevice,
        surface_format: wgpu::TextureFormat,
        pool_capacity_bytes: u64,
        target_sample_count: u32,
        gpu_ledger: Arc<GpuLedger>,
        minimal: bool,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Self> {
        let RendererDevice { device, queue } = gpu;
        started(observer, "renderer", "capabilities");
        let caps = RendererDeviceCaps::from_device(&device);
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

        let pool_capacity = effective_pool_capacity(pool_capacity_bytes, caps);
        if pool_capacity < pool_capacity_bytes {
            eprintln!(
                "[figgy] pool capacity {pool_capacity_bytes} B exceeds device limits \
                 (max_storage_buffer_binding_size {}, max_buffer_size {}); \
                 clamped to {pool_capacity} B",
                caps.max_storage_buffer_binding_size, caps.max_buffer_size
            );
        }
        finished(observer, "renderer", "capabilities");
        crate::init::yield_init_frame().await;
        let pool = observe_result_async(observer, "renderer", "figgy column pool", || {
            ColumnPool::new(
                pool_alloc_ctx(&device, &queue, None, &gpu_ledger),
                pool_capacity,
            )
        })
        .await?;

        let texture_bgl = data_render::create_texture_bind_group_layout(&device);
        let transform_bgl = data_render::create_scatter_transform_bind_group_layout(&device);
        let style_bgl = data_render::create_style_bind_group_layout(&device);
        let per_point_style_map_bgl =
            data_render::create_per_point_style_map_bind_group_layout(&device);
        let point_value_map_bgl = data_render::create_point_value_map_bind_group_layout(&device);
        let (neutral_point_value_map_buffers, neutral_point_value_map_charge) =
            data_render::create_point_value_map_buffers(
                &device,
                &gpu_ledger,
                &data_render::PointValueMapParamsGpu::NEUTRAL,
                &[],
            );
        let neutral_point_value_map_bg = data_render::bind_point_value_map(
            &device,
            &point_value_map_bgl,
            &neutral_point_value_map_buffers,
        );
        let view3d_value_map_bgl =
            data_render::surface_mesh::create_view3d_value_map_bind_group_layout(&device);
        let data_selection_bgl = data_render::create_data_selection_bind_group_layout(&device);
        let star_data_bgl = data_render::create_star_data_bind_group_layout(&device);
        let field_bgl = data_render::create_field_data_bind_group_layout(&device);
        let surface_bgl = data_render::surface_mesh::create_surface_data_bind_group_layout(&device);
        let region_bgl =
            data_render::ternary_region::create_ternary_region_bind_group_layout(&device);
        let vector_bgl = data_render::vector::create_vector_data_bind_group_layout(&device);
        let category_bar_data_bgl =
            data_render::create_category_bar_data_bind_group_layout(&device);
        let value_label_bgl = data_render::create_value_label_bind_group_layout(&device);
        let field_cell_tables = data_render::create_field_cell_tables(&device, &gpu_ledger)?;

        let sampler = data_render::create_linear_sampler(&device);
        let quad_vb = TrackedBuffer::new(
            &gpu_ledger,
            GpuResourceKind::Uniform,
            data_render::create_unit_centered_quad_vertex_buffer(&device),
        );

        let pipelines = create_target_pipelines_observed_async(
            &device,
            &texture_bgl,
            &transform_bgl,
            &style_bgl,
            &per_point_style_map_bgl,
            surface_format,
            target_sample_count,
            minimal,
            observer,
        )
        .await?;
        let renderer_identity = observe_result_async(observer, "renderer", "identity", || {
            issue_renderer_identity()
        })
        .await?;

        Ok(Self {
            device,
            queue,
            caps,
            pool,
            chart_states: HashMap::new(),
            streaming_sources: HashMap::new(),
            stream_revision_lineage: HashMap::new(),
            next_stream_source_fit_epoch: 0,
            stream_runtime: None,
            chart_order: Vec::new(),
            next_chart_id: 0,
            visual_revision: RenderRevision::initial(renderer_identity),
            renderer_identity,
            observed_font_generation: crate::text_render::font_generation(),
            pending_defrag: false,
            picker: RendererPicker::disabled(),
            errorbar_extent_engine: None,
            target_sample_count,
            target_pipeline_generation: 1,
            texture_bgl,
            transform_bgl,
            style_bgl,
            per_point_style_map_bgl,
            point_value_map_bgl,
            neutral_point_value_map_bg,
            neutral_point_value_map_buffers,
            _neutral_point_value_map_charge: neutral_point_value_map_charge,
            view3d_value_map_bgl,
            data_selection_bgl,
            star_data_bgl,
            field_bgl,
            surface_bgl,
            region_bgl,
            vector_bgl,
            category_bar_data_bgl,
            value_label_bgl,
            field_cell_tables,
            pipelines,
            sampler,
            quad_vb,
            arc_cache: HashMap::new(),
            resident_displays: Vec::new(),
            resident_display_serial: 0,
            field_cache: HashMap::new(),
            value_map_cache: HashMap::new(),
            vector_cache: HashMap::new(),
            contour_label_pipelines: None,
            contour_cache: HashMap::new(),
            arc_pipelines: None,
            #[cfg(test)]
            arc_chunk_override: None,
            space_bg: None,
            surface_format,
            gpu_ledger,
            memory_budget: None,
            auto_resident_working_set_limit: None,
        })
    }

    pub fn device(&self) -> &Arc<wgpu::Device> {
        &self.device
    }

    pub fn queue(&self) -> &Arc<wgpu::Queue> {
        &self.queue
    }

    pub fn pool(&self) -> &ColumnPool {
        &self.pool
    }

    /// Requested bytes of tracked GPU allocations, one row per [`GpuResourceKind`].
    ///
    /// The pool row comes from the pool itself (its live slab plus the
    /// ping-pong backup while one exists); every other row comes from the
    /// ledger. Final-owner drops move bytes to *retired*. A submission boundary
    /// captures retired resources; their bytes remain charged until GPU completion,
    /// when the handles are destroyed and their credits returned.
    ///
    /// This excludes lazy Milkyway/Constellation style textures, host-owned GPU
    /// resources, other renderers, CPU/output buffers and shader/pipeline/driver
    /// internals. It is not a measurement of physical VRAM residency. Streaming
    /// export admission separately checks known style-texture payload sizes.
    /// See [`Self::set_memory_budget`] for the allocation policy's scope.
    pub fn gpu_memory_usage(&self) -> GpuMemoryUsage {
        self.gpu_ledger
            .snapshot()
            .with_kind(
                GpuResourceKind::ColumnPool,
                self.pool.gpu_bytes(),
                self.pool.retired_bytes(),
            )
            .with_creations(GpuResourceKind::ColumnPool, self.pool.buffer_creations())
            .with_peak_at_least(self.pool.peak_bytes())
    }

    /// The host allocation budget, if any; see [`Self::set_memory_budget`] for scope.
    pub fn memory_budget(&self) -> Option<u64> {
        self.memory_budget
    }

    /// Set the host budget used by checked allocation and admission paths.
    ///
    /// **What it governs.** Checks at participating allocation paths, including
    /// pool growth. This is not a universal device allocator or a hard VRAM
    /// ceiling; see [`Self::gpu_memory_usage`] for accounting exclusions.
    /// Setting a budget never shrinks or evicts what is held, and it does *not*
    /// retroactively bound the pool's initial capacity: that slab is created
    /// during construction from the `pool_capacity_bytes` the host itself
    /// passed, before any budget exists. A pool growth that would cross the
    /// ceiling is refused as `AllocError::ResourceLimit` — the same failure the
    /// pool already reports for a device ceiling, so no host learns a new error
    /// shape.
    ///
    /// **Returns** the current tracked total when the new ceiling is below it,
    /// `None` when the total fits or the budget is disabled. The policy is applied either way: a
    /// host that lowers it deliberately, intending to release something next,
    /// gets what it asked for. But it learns at the call rather than at
    /// whatever growth fails later — the reason this returns anything at all.
    #[must_use = "a returned total means the new ceiling is already exceeded"]
    pub fn set_memory_budget(&mut self, budget: Option<u64>) -> Option<u64> {
        self.memory_budget = budget;
        let ceiling = budget?;
        let current = self.gpu_memory_usage().total_bytes();
        (current > ceiling).then_some(current)
    }

    /// Set the policy cap used only by renderer-selected residency changes.
    /// Native callers that never set this remain on the existing explicit
    /// promote/demote API. Returning the previous cap makes temporary host
    /// policy changes reversible without another getter.
    pub fn set_auto_resident_working_set_limit(&mut self, limit: Option<u64>) -> Option<u64> {
        let previous = std::mem::replace(&mut self.auto_resident_working_set_limit, limit);
        self.enforce_source_cache_policy();
        previous
    }

    /// Submission boundary: the host has submitted or discarded every command
    /// buffer recorded before retired owners were released. Snapshot those
    /// bytes now, but keep counting them until the queue reports completion.
    ///
    /// Split prepare/paint hosts call this once, after submitting all panels in
    /// the frame. Calling it from `prepare` would be too early because a host
    /// may still hold an unsubmitted command buffer. Idempotent.
    pub fn end_gpu_frame(&mut self) {
        // Cached bodies whose view and tokens are gone retire with this
        // frame's batch instead of waiting for the next prepare.
        self.resident_displays
            .retain(|display| display.view.strong_count() > 0);
        let batch = self.gpu_ledger.take_retirement();
        let (pool_retired, pool_bytes) = self.pool.take_retirement();
        if pool_bytes == 0 && batch.is_empty() {
            return;
        }
        let ledger = Arc::clone(&self.gpu_ledger);
        #[cfg(not(target_arch = "wasm32"))]
        self.queue.on_submitted_work_done(move || {
            ledger.complete_retirement(batch);
            pool_retired.complete(pool_bytes);
        });
        #[cfg(target_arch = "wasm32")]
        {
            // wgpu requires a Send completion callback even for WebGPU, but
            // browser buffer handles must stay on their owning JS thread.
            let (sender, receiver) = futures_channel::oneshot::channel();
            self.queue.on_submitted_work_done(move || {
                let _ = sender.send(());
            });
            wasm_bindgen_futures::spawn_local(async move {
                if receiver.await.is_ok() {
                    ledger.complete_retirement(batch);
                    pool_retired.complete(pool_bytes);
                }
            });
        }
    }

    /// Service submitted GPU work and retirement callbacks without waiting.
    ///
    /// This does not submit work or report a submission boundary: call
    /// [`Self::end_gpu_frame`] after submitting or discarding recorded commands
    /// first. Native hosts must keep servicing completions even while idle if
    /// they need retired memory to become available without another submission.
    /// On WebGPU the browser delivers completion callbacks asynchronously.
    /// Polling errors are returned to the host rather than treated as completion.
    pub fn service_gpu_completions(
        &self,
    ) -> std::result::Result<wgpu::PollStatus, wgpu::PollError> {
        self.device.poll(wgpu::PollType::Poll)
    }

    /// Handle onto the ledger, for resources this renderer creates but does
    /// not own (the windowed MSAA target).
    pub(crate) fn gpu_ledger(&self) -> &Arc<GpuLedger> {
        &self.gpu_ledger
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }

    /// Rebuild render-target pipelines if the host swap-chain format changed.
    ///
    /// Column data, chart views, textures, bind groups, styles, and buffers do
    /// not depend on the final color attachment format, so embedders can call
    /// this instead of tearing down the whole renderer state.
    pub fn ensure_target_format(&mut self, surface_format: wgpu::TextureFormat) -> Result<bool> {
        self.ensure_target(surface_format, self.target_sample_count)
    }

    pub(super) fn ensure_target(
        &mut self,
        surface_format: wgpu::TextureFormat,
        target_sample_count: u32,
    ) -> Result<bool> {
        if self.surface_format == surface_format && self.target_sample_count == target_sample_count
        {
            return Ok(false);
        }
        validate_target_sample_count(self.caps, surface_format, target_sample_count)?;
        let target_pipeline_generation =
            self.target_pipeline_generation
                .checked_add(1)
                .ok_or(FiggyError::CounterExhausted {
                    counter: "target pipeline generation",
                })?;
        // Fresh set with an empty styled cache — any styled / lazy precise
        // variants the next frame needs are recompiled by its prepare phase.
        // Recreate only the lazy variants that were already live so a format
        // change does not re-eager unused data/mapped/ring pipelines.
        let had_line = self.pipelines.line.is_some();
        let had_line_mapped = self.pipelines.line_mapped.is_some();
        let had_area = self.pipelines.area.is_some();
        let had_scatter = self.pipelines.scatter.is_some();
        let had_errorbar = self.pipelines.errorbar.is_some();
        let had_scatter_mapped = self.pipelines.scatter_mapped.is_some();
        let had_pick_ring = self.pipelines.pick_ring.is_some();
        let had_pick_ring_mapped = self.pipelines.pick_ring_mapped.is_some();
        let had_ternary = self.pipelines.ternary.each_ref().map(Option::is_some);
        let had_view3d = self.pipelines.view3d.each_ref().map(Option::is_some);
        let had_errorbar_mapped = self.pipelines.errorbar_mapped.is_some();
        let had_bar = self.pipelines.bar.is_some();
        let had_bar_mapped = self.pipelines.bar_mapped.is_some();
        self.pipelines = create_target_pipelines(
            &self.device,
            &self.texture_bgl,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            surface_format,
            target_sample_count,
        );
        if had_line {
            self.pipelines.line = Some(
                data_render::create_line_columnar_pipeline_with_sample_count(
                    &self.device,
                    &self.pipelines.shaders.line,
                    &self.transform_bgl,
                    &self.style_bgl,
                    surface_format,
                    target_sample_count,
                ),
            );
        }
        if had_line_mapped {
            self.pipelines.line_mapped =
                Some(data_render::create_line_mapped_pipeline_with_sample_count(
                    &self.device,
                    &self.pipelines.shaders.line,
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.point_value_map_bgl,
                    surface_format,
                    target_sample_count,
                ));
        }
        if had_area {
            self.pipelines.area = Some(data_render::create_line_area_pipeline_with_sample_count(
                &self.device,
                &self.pipelines.shaders.line,
                &self.transform_bgl,
                &self.style_bgl,
                surface_format,
                target_sample_count,
            ));
        }
        if had_scatter {
            self.pipelines.scatter = Some(
                data_render::create_scatter_columnar_pipeline_with_sample_count(
                    &self.device,
                    &self.pipelines.shaders.scatter,
                    &self.transform_bgl,
                    &self.style_bgl,
                    surface_format,
                    target_sample_count,
                ),
            );
        }
        if had_bar {
            self.pipelines.bar = Some(data_render::create_bar_columnar_pipeline_with_sample_count(
                &self.device,
                &self.pipelines.shaders.bar,
                &self.transform_bgl,
                &self.style_bgl,
                surface_format,
                target_sample_count,
            ));
        }
        if had_bar_mapped {
            self.pipelines.bar_mapped = Some(
                data_render::create_bar_columnar_mapped_pipeline_with_sample_count(
                    &self.device,
                    &self.pipelines.shaders.bar,
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.per_point_style_map_bgl,
                    surface_format,
                    target_sample_count,
                ),
            );
        }
        if had_errorbar {
            self.pipelines.errorbar = Some(
                data_render::create_errorbar_columnar_pipeline_with_sample_count(
                    &self.device,
                    &self.pipelines.shaders.errorbar,
                    &self.transform_bgl,
                    &self.style_bgl,
                    surface_format,
                    target_sample_count,
                ),
            );
        }
        if had_scatter_mapped {
            self.pipelines.scatter_mapped = Some(
                data_render::create_scatter_columnar_mapped_pipeline_with_entries(
                    &self.device,
                    &self.pipelines.shaders.scatter,
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.per_point_style_map_bgl,
                    &self.point_value_map_bgl,
                    surface_format,
                    target_sample_count,
                    "vs_mapped",
                    "fs_mapped",
                    "figgy scatter mapped pipeline",
                ),
            );
        }
        if had_pick_ring {
            self.pipelines.pick_ring =
                Some(data_render::create_scatter_columnar_pipeline_with_entries(
                    &self.device,
                    &self.pipelines.shaders.scatter,
                    &self.transform_bgl,
                    &self.style_bgl,
                    surface_format,
                    target_sample_count,
                    "vs_pick_ring",
                    "fs_pick_ring",
                    "figgy picked point ring pipeline",
                ));
        }
        for kind in TernaryPipeline::ALL {
            if had_ternary[kind as usize] {
                self.pipelines.ternary[kind as usize] = Some(create_ternary_pipeline(
                    kind,
                    &self.device,
                    &self.pipelines.shaders,
                    [
                        &self.transform_bgl,
                        &self.style_bgl,
                        &self.per_point_style_map_bgl,
                        &self.point_value_map_bgl,
                    ],
                    surface_format,
                    target_sample_count,
                ));
            }
        }
        for kind in View3dPipeline::ALL {
            if had_view3d[kind as usize] {
                self.pipelines.view3d[kind as usize] = Some(create_view3d_pipeline(
                    kind,
                    &self.device,
                    &self.pipelines.shaders,
                    [
                        &self.transform_bgl,
                        &self.style_bgl,
                        &self.per_point_style_map_bgl,
                        &self.surface_bgl,
                        &self.view3d_value_map_bgl,
                    ],
                    surface_format,
                    target_sample_count,
                ));
            }
        }
        if had_pick_ring_mapped {
            self.pipelines.pick_ring_mapped = Some(
                data_render::create_scatter_columnar_mapped_pipeline_with_entries(
                    &self.device,
                    &self.pipelines.shaders.scatter,
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.per_point_style_map_bgl,
                    &self.point_value_map_bgl,
                    surface_format,
                    target_sample_count,
                    "vs_pick_ring_mapped",
                    "fs_pick_ring",
                    "figgy picked point mapped ring pipeline",
                ),
            );
        }
        if had_errorbar_mapped {
            self.pipelines.errorbar_mapped = Some(
                data_render::create_errorbar_columnar_mapped_pipeline_from_shader(
                    &self.device,
                    &self.pipelines.shaders.errorbar,
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.per_point_style_map_bgl,
                    surface_format,
                    target_sample_count,
                ),
            );
        }
        self.surface_format = surface_format;
        self.target_sample_count = target_sample_count;
        self.target_pipeline_generation = target_pipeline_generation;
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

    pub fn texture_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.texture_bgl
    }

    pub fn transform_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.transform_bgl
    }

    pub fn style_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.style_bgl
    }

    pub fn axis_pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipelines.axis
    }

    pub fn line_pipeline(&self) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.line.as_ref()
    }

    pub fn scatter_pipeline(&self) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.scatter.as_ref()
    }

    pub fn errorbar_pipeline(&self) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.errorbar.as_ref()
    }

    pub fn bar_pipeline(&self) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.bar.as_ref()
    }

    pub fn bar_mapped_pipeline(&self) -> Option<&wgpu::RenderPipeline> {
        self.pipelines.bar_mapped.as_ref()
    }

    /// Compile and retain the resident Cartesian pipeline groups before first use.
    ///
    /// Covers data/style rendering, contour-label compute, arc scans, fit reduction
    /// and GPU picking. The separate radial renderer and nonresident matrix/tile
    /// streaming pipelines are outside this scope. Chart-specific data, views,
    /// styles, atlases and result buffers can still require allocation afterwards.
    ///
    /// The observer receives a started/finished pair around each independently
    /// scheduled stage. On wasm every finished stage yields to the browser so
    /// host UI and file parsing remain live while the renderer is preparing.
    /// Repeated calls reuse installed caches while their target generation remains
    /// valid. A target-format or sample-count change can require new pipelines.
    /// This does not guarantee zero driver compilation cost on the first draw.
    pub async fn prewarm_all_observed(
        &mut self,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<()> {
        if self.contour_label_pipelines.is_none() {
            #[cfg(target_arch = "wasm32")]
            crate::init::prewarm_compute_entries_js(
                &self.device,
                "contour.label.async",
                crate::gpu_contour::CONTOUR_ANCHOR_SHADER.source,
                &[
                    ("anchor_project", "anchor_seed"),
                    ("anchor_project", "anchor_step"),
                    ("anchor_project", "anchor_project"),
                    ("anchor_select", "anchor_select"),
                ],
                observer,
            )
            .await
            .map_err(|reason| FiggyError::GpuResourceAllocationFailed {
                resource: "contour label compute pipelines",
                reason,
            })?;
            self.contour_label_pipelines = Some(
                observe_value_async(observer, "renderer.prewarm", "contour label engine", || {
                    crate::gpu_contour::ContourLabelPipelines::new(&self.device, &self.field_bgl)
                })
                .await,
            );
        }

        self.pipelines
            .prewarm_all_observed(
                &self.device,
                &self.queue,
                &self.transform_bgl,
                &self.style_bgl,
                &self.per_point_style_map_bgl,
                &self.point_value_map_bgl,
                &self.data_selection_bgl,
                &self.field_bgl,
                &self.vector_bgl,
                &self.category_bar_data_bgl,
                &self.star_data_bgl,
                self.contour_label_pipelines.as_ref(),
                None,
                self.surface_format,
                observer,
            )
            .await;

        // The 3D surface and its floor contours (`Config.view_3d`).
        started(observer, "renderer.prewarm", "surface");
        for pass in data_render::surface_mesh::SurfacePass::ALL {
            self.pipelines.ensure_surface_pass(
                &self.device,
                [
                    &self.transform_bgl,
                    &self.style_bgl,
                    &self.field_bgl,
                    &self.surface_bgl,
                ],
                self.surface_format,
                pass,
            );
        }
        finished(observer, "renderer.prewarm", "surface");
        crate::init::yield_init_frame().await;

        // The 3D scatter and line entries (#132).
        for kind in View3dPipeline::ALL {
            if self.pipelines.view3d[kind as usize].is_none() {
                started(observer, "renderer.prewarm", kind.stage());
                self.pipelines.ensure_view3d_pass(
                    &self.device,
                    [
                        &self.transform_bgl,
                        &self.style_bgl,
                        &self.per_point_style_map_bgl,
                        &self.surface_bgl,
                        &self.view3d_value_map_bgl,
                    ],
                    self.surface_format,
                    kind,
                );
                finished(observer, "renderer.prewarm", kind.stage());
            }
        }
        crate::init::yield_init_frame().await;

        // A ternary series' region fill (`TernaryRegionStyle`).
        started(observer, "renderer.prewarm", "ternary region");
        for pass in data_render::ternary_region::TernaryRegionPass::ALL {
            self.pipelines.ensure_ternary_region_pass(
                &self.device,
                &self.transform_bgl,
                &self.region_bgl,
                self.surface_format,
                pass,
            );
        }
        finished(observer, "renderer.prewarm", "ternary region");
        crate::init::yield_init_frame().await;

        if self.arc_pipelines.is_none() {
            self.arc_pipelines = Some(
                data_render::line_arc::create_arc_scan_pipelines_observed_async(
                    &self.device,
                    observer,
                )
                .await
                .map_err(|reason| FiggyError::GpuResourceAllocationFailed {
                    resource: "line arc compute pipelines",
                    reason,
                })?,
            );
        }

        if self.errorbar_extent_engine.is_none() {
            started(observer, "renderer.prewarm", "fit extent compute");
            self.ensure_errorbar_extent_engine()
                .await
                .map_err(|error| FiggyError::GpuResourceAllocationFailed {
                    resource: "fit extent compute pipelines",
                    reason: error.to_string(),
                })?;
            finished(observer, "renderer.prewarm", "fit extent compute");
            crate::init::yield_init_frame().await;
        }

        started(observer, "renderer.prewarm", "point picking compute");
        self.enable_gpu_picking_observed_async(observer)
            .await
            .map_err(|error| FiggyError::GpuResourceAllocationFailed {
                resource: "point picking compute pipelines",
                reason: error.to_string(),
            })?;
        finished(observer, "renderer.prewarm", "point picking compute");
        crate::init::yield_init_frame().await;
        Ok(())
    }

    /// Prepare the resident Cartesian groups described by [`Self::prewarm_all_observed`]
    /// without progress callbacks.
    pub async fn prewarm_all(&mut self) -> Result<()> {
        let mut noop = |_| {};
        self.prewarm_all_observed(&mut noop).await
    }

    /// Wait until previously submitted GPU work has finished executing.
    ///
    /// This does not submit a frame. Hosts that have not drawn yet should call
    /// [`WindowedRenderer::first_frame_ready`] or draw first. On native the
    /// device is polled so the callback can resolve; on wasm the browser
    /// delivers `onSubmittedWorkDone`.
    pub async fn wait_submitted_work(&self) {
        let (sender, receiver) = futures_channel::oneshot::channel();
        self.queue.on_submitted_work_done(move || {
            let _ = sender.send(());
        });
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = self.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
        }
        let _ = receiver.await;
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    pub fn quad_vertex_buffer(&self) -> &wgpu::Buffer {
        &self.quad_vb
    }
}
