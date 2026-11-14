//! Creating a target's pipelines, blocking, observed or async.

use super::*;

pub(in crate::renderer) fn create_target_pipelines(
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
pub(in crate::renderer) fn create_target_pipelines_observed(
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
        line_parallel: None,
        line_parallel_selection: None,
        area: None,
        scatter: None,
        scatter_mapped: None,
        pick_ring: None,
        pick_ring_mapped: None,
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
        errorbar: None,
        errorbar_mapped: None,
        errorbar_stem: None,
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
pub(in crate::renderer) async fn create_target_pipelines_observed_async(
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
        line_parallel: None,
        line_parallel_selection: None,
        area: None,
        scatter: None,
        scatter_mapped: None,
        pick_ring: None,
        pick_ring_mapped: None,
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
        errorbar: None,
        errorbar_mapped: None,
        errorbar_stem: None,
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
