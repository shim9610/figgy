//! Creating a target's pipelines, blocking, observed or async.

use super::*;

pub(in crate::renderer) fn create_target_pipelines(
    device: &wgpu::Device,
    cache: Option<&wgpu::PipelineCache>,
    texture_bgl: &wgpu::BindGroupLayout,
    transform_bgl: &wgpu::BindGroupLayout,
    style_bgl: &wgpu::BindGroupLayout,
    _per_point_style_map_bgl: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
    set: ShaderSet,
) -> TargetPipelines {
    let mut observer = |_| {};
    create_target_pipelines_observed(
        device,
        cache,
        texture_bgl,
        transform_bgl,
        style_bgl,
        _per_point_style_map_bgl,
        surface_format,
        sample_count,
        set,
        &mut observer,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::renderer) fn create_target_pipelines_observed(
    device: &wgpu::Device,
    cache: Option<&wgpu::PipelineCache>,
    texture_bgl: &wgpu::BindGroupLayout,
    _transform_bgl: &wgpu::BindGroupLayout,
    _style_bgl: &wgpu::BindGroupLayout,
    _per_point_style_map_bgl: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
    set: ShaderSet,
    observer: &mut dyn FnMut(InitEvent),
) -> TargetPipelines {
    let shaders = observe_value(observer, "renderer", "shader modules", || {
        data_render::ShaderModules::new(device, set)
    });
    let axis = observe_value(
        observer,
        "renderer",
        "figgy fullscreen textured pipeline",
        || {
            data_render::create_fullscreen_textured_pipeline_with_sample_count(
                device,
                cache,
                &shaders.fullscreen,
                texture_bgl,
                surface_format,
                sample_count,
            )
        },
    );
    TargetPipelines::empty(shaders, axis, sample_count)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::renderer) async fn create_target_pipelines_observed_async(
    device: &wgpu::Device,
    cache: Option<&wgpu::PipelineCache>,
    texture_bgl: &wgpu::BindGroupLayout,
    _transform_bgl: &wgpu::BindGroupLayout,
    _style_bgl: &wgpu::BindGroupLayout,
    _per_point_style_map_bgl: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
    sample_count: u32,
    _minimal: bool,
    set: ShaderSet,
    observer: &mut dyn FnMut(InitEvent),
) -> Result<TargetPipelines> {
    let shaders = observe_value_async(observer, "renderer", "shader modules", || {
        data_render::ShaderModules::new(device, set)
    })
    .await;
    // A minimal renderer awaits no compile (#299): the one pipeline built
    // below is created directly, and the others are warmed in the
    // background by the caller. An awaited compile would queue behind every
    // compile the page already has in flight, another chart's included.
    #[cfg(target_arch = "wasm32")]
    let warmed = if _minimal {
        Ok(())
    } else {
        data_render::prewarm_browser_render_feature(
            device,
            surface_format,
            sample_count,
            None,
            set,
            None,
            observer,
        )
        .await
    };
    #[cfg(target_arch = "wasm32")]
    warmed.map_err(|reason| FiggyError::GpuResourceAllocationFailed {
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
                cache,
                &shaders.fullscreen,
                texture_bgl,
                surface_format,
                sample_count,
            )
        },
    )
    .await;
    Ok(TargetPipelines::empty(shaders, axis, sample_count))
}

impl TargetPipelines {
    /// A set with its modules and axis pipeline and no other pipeline yet.
    pub(in crate::renderer) fn empty(
        shaders: data_render::ShaderModules,
        axis: wgpu::RenderPipeline,
        sample_count: u32,
    ) -> Self {
        Self {
            shaders,
            axis,
            precise: std::array::from_fn(|_| None),
            ternary: Default::default(),
            view3d: Default::default(),
            surface_shader: None,
            surface: Default::default(),
            tri_mesh_shader: None,
            tri_mesh: Default::default(),
            tri_surface_shader: None,
            tri_surface: Default::default(),
            ternary_region_shader: None,
            ternary_region: Default::default(),
            ternary_region_scan: None,
            bar_envelope: None,
            category_stack_fit: None,
            sample_count,
            styled: HashMap::new(),
        }
    }
}
