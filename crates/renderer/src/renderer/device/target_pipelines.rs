//! The pipelines of one render target and the style sets and passes they hold.

use super::*;

/// Pipelines compiled against one render-target format. Axis is created with
/// the target; ordinary renderer use creates the other variants lazily, while
/// explicit full prewarm materializes every optional field before interaction.
pub(in crate::renderer) struct TargetPipelines {
    pub(in crate::renderer) shaders: data_render::ShaderModules,
    pub(in crate::renderer) axis: wgpu::RenderPipeline,
    /// The precise (unstyled) pipelines, indexed by [`PrecisePipeline`]
    /// (#341): each created by the first frame that needs it
    /// ([`create_precise_pipeline`]), or by the prewarm.
    pub(in crate::renderer) precise: [Option<wgpu::RenderPipeline>; PRECISE_PIPELINES],
    /// A ternary chart's entries (`Config.ternary`), indexed by
    /// [`TernaryPipeline`]: the stroke (`vs_ternary`), the markers
    /// (`vs_ternary`, `vs_ternary_mapped`) and the picked ring
    /// (`vs_pick_ring_ternary`, `vs_pick_ring_ternary_mapped`), each the
    /// plain pipeline with the third component's vertex slot.
    pub(in crate::renderer) ternary: [Option<wgpu::RenderPipeline>; TERNARY_PIPELINES],
    /// A 3D chart's scatter and line entries (#132), indexed by
    /// [`View3dPipeline`]: the depth-tested draws, their ID picks and the
    /// picked rings, each the plain pipeline with the height's vertex slot
    /// and the camera group.
    pub(in crate::renderer) view3d: [Option<wgpu::RenderPipeline>; VIEW3D_PIPELINES],
    /// A 3D chart's surface module and pipelines, indexed by
    /// [`data_render::surface_mesh::SurfacePass`]: the depth-tested draw in
    /// the panel composite, the picked patch in the live slot and the ID
    /// pick. Compiled only when a 3D chart is drawn.
    pub(in crate::renderer) surface_shader: Option<wgpu::ShaderModule>,
    pub(in crate::renderer) surface:
        [Option<wgpu::RenderPipeline>; data_render::surface_mesh::SurfacePass::ALL.len()],
    /// A triangle mesh series' module and pipelines (#210), indexed by
    /// [`data_render::tri_mesh::TriMeshPass`]: the fill and the contour
    /// strokes over the field's group 2. Compiled only when a mesh is drawn.
    pub(in crate::renderer) tri_mesh_shader: Option<wgpu::ShaderModule>,
    pub(in crate::renderer) tri_mesh:
        [Option<wgpu::RenderPipeline>; data_render::tri_mesh::TriMeshPass::ALL.len()],
    /// A triangle mesh's surface pipelines on a 3D chart (#210), indexed by
    /// [`data_render::surface_mesh::SurfacePass`] as `surface` is: the
    /// surface passes from the mesh surface module
    /// (`TRI_SURFACE_SHADER`), the floor passes from the mesh module.
    /// Compiled only when a 3D chart draws a mesh.
    pub(in crate::renderer) tri_surface_shader: Option<wgpu::ShaderModule>,
    pub(in crate::renderer) tri_surface:
        [Option<wgpu::RenderPipeline>; data_render::surface_mesh::SurfacePass::ALL.len()],
    /// A ternary series' region fill: the module and its stencil and cover
    /// pipelines, indexed by
    /// [`data_render::ternary_region::TernaryRegionPass`], drawn into the
    /// panel composite's stencil attachment. Compiled only when a ternary
    /// chart with a region fill is drawn.
    pub(in crate::renderer) ternary_region_shader: Option<wgpu::ShaderModule>,
    pub(in crate::renderer) ternary_region:
        [Option<wgpu::RenderPipeline>; data_render::ternary_region::TernaryRegionPass::ALL.len()],
    /// The ring-start scan the region draw and pick read (#49), from the
    /// same module.
    pub(in crate::renderer) ternary_region_scan:
        Option<data_render::ternary_region::RingScanPipelines>,
    /// The histogram envelope's fill and scan, created with the first
    /// precise or mapped histogram.
    pub(in crate::renderer) bar_envelope: Option<data_render::bar_envelope::Pipelines>,
    /// The stacked category-bar fit's reduction (#466), from the bar
    /// module: created by the first fit of a stacked group or by the full
    /// prewarm, never by a draw.
    pub(in crate::renderer) category_stack_fit: Option<data_render::CategoryStackFit>,
    pub(in crate::renderer) sample_count: u32,
    pub(in crate::renderer) styled: HashMap<StyleKey, StyleSet>,
}

impl TargetPipelines {
    /// The precise pipeline `kind`, when it has been created.
    pub(in crate::renderer) fn precise(
        &self,
        kind: PrecisePipeline,
    ) -> Option<&wgpu::RenderPipeline> {
        self.precise[kind as usize].as_ref()
    }

    /// Create the precise pipeline `kind` when it is missing and `layouts`
    /// can build it (#341).
    pub(in crate::renderer) fn ensure_precise(
        &mut self,
        kind: PrecisePipeline,
        layouts: &PreciseLayouts<'_>,
    ) {
        if self.precise[kind as usize].is_none() {
            self.precise[kind as usize] =
                create_precise_pipeline(kind, layouts, &self.shaders, self.sample_count);
        }
    }

    /// Create the histogram envelope's pipelines when they are missing.
    pub(in crate::renderer) fn ensure_bar_envelope(&mut self, layouts: &PreciseLayouts<'_>) {
        if self.bar_envelope.is_none() {
            self.bar_envelope = Some(data_render::bar_envelope::Pipelines::new(
                layouts.device,
                layouts.cache,
                &self.shaders.bar,
                layouts.transform_bgl,
                layouts.style_bgl,
                layouts.per_point_style_map_bgl,
                layouts.surface_format,
                self.sample_count,
            ));
        }
    }

    /// Create the stacked category-bar fit's reduction when it is missing
    /// (#466): the bar module's `reduce_category_stacks` entry.
    pub(in crate::renderer) fn ensure_category_stack_fit(
        &mut self,
        device: &wgpu::Device,
        cache: Option<&wgpu::PipelineCache>,
    ) {
        if self.category_stack_fit.is_none() {
            self.category_stack_fit = Some(data_render::CategoryStackFit::new(
                device,
                cache,
                &self.shaders.bar,
            ));
        }
    }

    /// Create the ternary entry `kind` when it is missing.
    pub(in crate::renderer) fn ensure_ternary(
        &mut self,
        kind: TernaryPipeline,
        layouts: &PreciseLayouts<'_>,
    ) {
        if self.ternary[kind as usize].is_none() {
            self.ternary[kind as usize] = Some(create_ternary_pipeline(
                kind,
                layouts.device,
                layouts.cache,
                &self.shaders,
                layouts.ternary(),
                layouts.surface_format,
                self.sample_count,
            ));
        }
    }

    /// Prepare-phase ensure (`&mut`): compile and cache the pipeline set of
    /// every style that `items` draw with and the cache doesn't hold yet.
    /// Run before pass recording so the draw phase (`&self`) only looks up.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::renderer) fn ensure_styles_for_items(
        &mut self,
        device: &wgpu::Device,
        cache: Option<&wgpu::PipelineCache>,
        queue: &wgpu::Queue,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        star_data_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        items: &[PanelDraw<'_>],
    ) {
        for item in items {
            let Some(v) = style_variant(&item.chart_config.draw_style) else {
                continue;
            };
            self.styled.entry(v.key).or_insert_with(|| {
                create_style_set(
                    v.key,
                    device,
                    cache,
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
    pub(in crate::renderer) fn ensure_surface_pipelines(
        &mut self,
        device: &wgpu::Device,
        cache: Option<&wgpu::PipelineCache>,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        surface_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        items: &[PanelDraw<'_>],
    ) {
        let on_3d = |draws: fn(&crate::data_config::DataRenderType) -> bool| {
            items.iter().any(|item| {
                item.chart_config.view_3d().is_some()
                    && item
                        .series
                        .iter()
                        .any(|series| draws(&series.config.render_type))
            })
        };
        if on_3d(|rt| draws_surface(rt) || draws_floor(rt)) {
            for pass in data_render::surface_mesh::SurfacePass::ALL {
                self.ensure_surface_pass(
                    device,
                    cache,
                    [transform_bgl, style_bgl, field_bgl, surface_bgl],
                    surface_format,
                    pass,
                );
            }
        }
        // A triangle mesh on a 3D chart (#210): its surface and its floor
        // contours, from the mesh surface module and the mesh module.
        if on_3d(|rt| draws_mesh_surface(rt) || draws_mesh_floor(rt)) {
            for pass in data_render::surface_mesh::SurfacePass::ALL {
                self.ensure_tri_surface_pass(
                    device,
                    cache,
                    [transform_bgl, style_bgl, field_bgl, surface_bgl],
                    surface_format,
                    pass,
                );
            }
        }
    }

    /// Compile the ternary region module and its two pipelines when a
    /// ternary chart among `items` draws a series with a region fill.
    pub(in crate::renderer) fn ensure_ternary_region_pipelines(
        &mut self,
        device: &wgpu::Device,
        cache: Option<&wgpu::PipelineCache>,
        transform_bgl: &wgpu::BindGroupLayout,
        region_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        items: &[PanelDraw<'_>],
    ) {
        let needed = items.iter().any(|item| {
            item.chart_config.ternary().is_some()
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
                cache,
                transform_bgl,
                region_bgl,
                surface_format,
                pass,
            );
        }
        self.ensure_ternary_region_scan(device, cache);
    }

    /// The region module's shader, compiled once.
    pub(super) fn ternary_region_module(&mut self, device: &wgpu::Device) -> &wgpu::ShaderModule {
        let variant = self.shaders.set;
        self.ternary_region_shader.get_or_insert_with(|| {
            crate::gpu_compile::shader_module(
                device,
                wgpu::ShaderModuleDescriptor {
                    label: Some("figgy ternary region shader"),
                    source: wgpu::ShaderSource::Wgsl(
                        data_render::ternary_region::TERNARY_REGION_SHADER_BY_SET
                            .of(variant)
                            .source
                            .into(),
                    ),
                },
            )
        })
    }

    /// Compile the ring-start scan when it is missing.
    pub(in crate::renderer) fn ensure_ternary_region_scan(
        &mut self,
        device: &wgpu::Device,
        cache: Option<&wgpu::PipelineCache>,
    ) {
        if self.ternary_region_scan.is_some() {
            return;
        }
        let scan = data_render::ternary_region::create_ring_scan_pipelines(
            device,
            cache,
            self.ternary_region_module(device),
        );
        self.ternary_region_scan = Some(scan);
    }

    /// Compile the ternary region module and the one pipeline `pass` when
    /// they are missing.
    pub(in crate::renderer) fn ensure_ternary_region_pass(
        &mut self,
        device: &wgpu::Device,
        cache: Option<&wgpu::PipelineCache>,
        transform_bgl: &wgpu::BindGroupLayout,
        region_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        pass: data_render::ternary_region::TernaryRegionPass,
    ) {
        if self.ternary_region[pass as usize].is_some() {
            return;
        }
        let sample_count = self.sample_count;
        let shader = self.ternary_region_module(device);
        let pipeline = Some(data_render::ternary_region::create_ternary_region_pipeline(
            device,
            cache,
            shader,
            transform_bgl,
            region_bgl,
            surface_format,
            sample_count,
            pass,
        ));
        self.ternary_region[pass as usize] = pipeline;
    }

    /// Compile the one 3D point pipeline `kind` when it is missing (#132);
    /// `bgls` are the transform, style, style map, surface, combined value
    /// map and vector data layouts.
    pub(in crate::renderer) fn ensure_view3d_pass(
        &mut self,
        device: &wgpu::Device,
        cache: Option<&wgpu::PipelineCache>,
        bgls: [&wgpu::BindGroupLayout; 6],
        surface_format: wgpu::TextureFormat,
        kind: View3dPipeline,
    ) {
        if self.view3d[kind as usize].is_some() {
            return;
        }
        self.view3d[kind as usize] = Some(create_view3d_pipeline(
            kind,
            device,
            cache,
            &self.shaders,
            bgls,
            surface_format,
            self.sample_count,
        ));
    }

    /// Compile the surface module and the one pipeline `pass` when they are
    /// missing; `bgls` are the transform, style, field and surface layouts.
    pub(in crate::renderer) fn ensure_surface_pass(
        &mut self,
        device: &wgpu::Device,
        cache: Option<&wgpu::PipelineCache>,
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
                cache,
                &self.shaders.field,
                transform_bgl,
                style_bgl,
                field_bgl,
                surface_bgl,
                surface_format,
                self.sample_count,
                pass,
                data_render::surface_mesh::SurfaceInput::Grid,
            ));
            return;
        }
        let variant = self.shaders.set;
        let shader = self.surface_shader.get_or_insert_with(|| {
            crate::gpu_compile::shader_module(
                device,
                wgpu::ShaderModuleDescriptor {
                    label: Some("figgy surface shader"),
                    source: wgpu::ShaderSource::Wgsl(
                        data_render::surface_mesh::SURFACE_SHADER_BY_SET
                            .of(variant)
                            .source
                            .into(),
                    ),
                },
            )
        });
        let slot = &mut self.surface[pass as usize];
        if slot.is_none() {
            *slot = Some(data_render::surface_mesh::create_surface_pipeline(
                device,
                cache,
                shader,
                transform_bgl,
                style_bgl,
                field_bgl,
                surface_bgl,
                surface_format,
                self.sample_count,
                pass,
                data_render::surface_mesh::SurfaceInput::Grid,
            ));
        }
    }

    /// One of a triangle mesh's surface pipelines (#210): the surface passes
    /// from the mesh surface module, the floor passes from the mesh module
    /// (`vs_tri_floor`), as [`Self::ensure_surface_pass`] builds a grid's.
    pub(in crate::renderer) fn ensure_tri_surface_pass(
        &mut self,
        device: &wgpu::Device,
        cache: Option<&wgpu::PipelineCache>,
        bgls: [&wgpu::BindGroupLayout; 4],
        surface_format: wgpu::TextureFormat,
        pass: data_render::surface_mesh::SurfacePass,
    ) {
        let [transform_bgl, style_bgl, field_bgl, surface_bgl] = bgls;
        if self.tri_surface[pass as usize].is_some() {
            return;
        }
        let variant = self.shaders.set;
        let shader: &wgpu::ShaderModule = if pass.on_floor() {
            self.tri_mesh_shader
                .get_or_insert_with(|| data_render::tri_mesh::shader_module(device, variant))
        } else {
            self.tri_surface_shader.get_or_insert_with(|| {
                crate::gpu_compile::shader_module(
                    device,
                    wgpu::ShaderModuleDescriptor {
                        label: Some("figgy triangle surface shader"),
                        source: wgpu::ShaderSource::Wgsl(
                            data_render::surface_mesh::TRI_SURFACE_SHADER_BY_SET
                                .of(variant)
                                .source
                                .into(),
                        ),
                    },
                )
            })
        };
        self.tri_surface[pass as usize] = Some(data_render::surface_mesh::create_surface_pipeline(
            device,
            cache,
            shader,
            transform_bgl,
            style_bgl,
            field_bgl,
            surface_bgl,
            surface_format,
            self.sample_count,
            pass,
            data_render::surface_mesh::SurfaceInput::Mesh,
        ));
    }

    /// Compile the triangle mesh pipeline of `pass` (#210), and the mesh
    /// module first when no mesh has been drawn yet.
    pub(in crate::renderer) fn ensure_tri_mesh_pass(
        &mut self,
        device: &wgpu::Device,
        cache: Option<&wgpu::PipelineCache>,
        [transform_bgl, style_bgl, field_bgl]: [&wgpu::BindGroupLayout; 3],
        surface_format: wgpu::TextureFormat,
        pass: data_render::tri_mesh::TriMeshPass,
    ) {
        if self.tri_mesh[pass as usize].is_some() {
            return;
        }
        let variant = self.shaders.set;
        let shader = self
            .tri_mesh_shader
            .get_or_insert_with(|| data_render::tri_mesh::shader_module(device, variant));
        self.tri_mesh[pass as usize] = Some(data_render::tri_mesh::create_tri_mesh_pipeline(
            device,
            cache,
            shader,
            transform_bgl,
            style_bgl,
            field_bgl,
            surface_format,
            self.sample_count,
            pass,
        ));
    }

    /// Draw-phase lookup: the cached set for the item's style, `None` for
    /// precise mode. A cache miss (impossible once the prepare phase ran)
    /// also yields `None` — callers then fall back to the precise pipelines
    /// instead of panicking.
    pub(in crate::renderer) fn style_set(&self, style: &DrawStyle) -> Option<&StyleSet> {
        style_variant(style).and_then(|v| self.styled.get(&v.key))
    }
}
