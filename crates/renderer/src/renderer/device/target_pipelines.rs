//! The pipelines of one render target and the style sets and passes they hold.

use super::*;

/// Pipelines compiled against one render-target format. Axis is created with
/// the target; ordinary renderer use creates the other variants lazily, while
/// explicit full prewarm materializes every optional field before interaction.
pub(in crate::renderer) struct TargetPipelines {
    pub(in crate::renderer) shaders: data_render::ShaderModules,
    pub(in crate::renderer) axis: wgpu::RenderPipeline,
    pub(in crate::renderer) line: Option<wgpu::RenderPipeline>,
    /// The colour-mapped stroke (`vs_main_mapped` / `fs_main_mapped`) of a
    /// streamline series with a colour column: the line slots plus the value
    /// column, group 3 the point value map.
    pub(in crate::renderer) line_mapped: Option<wgpu::RenderPipeline>,
    /// The rows of a parallel coordinates series (`vs_parallel` /
    /// `fs_parallel`, #141): no vertex slots, group 3 the series' parallel
    /// data.
    pub(in crate::renderer) line_parallel: Option<wgpu::RenderPipeline>,
    /// The picked row's redraw over the same layout
    /// (`vs_parallel_selected`).
    pub(in crate::renderer) line_parallel_selection: Option<wgpu::RenderPipeline>,
    /// The line shader's `vs_area` / `fs_area` fill, compiled when a line
    /// series declares a fill. Used in every draw style: the fill is a flat
    /// precise primitive under whatever the style draws for the line.
    pub(in crate::renderer) area: Option<wgpu::RenderPipeline>,
    pub(in crate::renderer) scatter: Option<wgpu::RenderPipeline>,
    pub(in crate::renderer) scatter_mapped: Option<wgpu::RenderPipeline>,
    pub(in crate::renderer) pick_ring: Option<wgpu::RenderPipeline>,
    pub(in crate::renderer) pick_ring_mapped: Option<wgpu::RenderPipeline>,
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
        Option<std::sync::Arc<data_render::ternary_region::RingScanPipelines>>,
    pub(in crate::renderer) errorbar: Option<wgpu::RenderPipeline>,
    pub(in crate::renderer) errorbar_mapped: Option<wgpu::RenderPipeline>,
    /// The precise stem draw (#206): `vs_stem` into `fs_stem`.
    pub(in crate::renderer) errorbar_stem: Option<wgpu::RenderPipeline>,
    pub(in crate::renderer) bar: Option<wgpu::RenderPipeline>,
    pub(in crate::renderer) bar_envelope: Option<data_render::bar_envelope::Pipelines>,
    pub(in crate::renderer) bar_mapped: Option<wgpu::RenderPipeline>,
    pub(in crate::renderer) bar_selection: Option<wgpu::RenderPipeline>,
    /// The selected bin's outline over the `bar_mapped` layout: group 2 is
    /// the series' bar style map, so the outline resolves the bin's overrides
    /// as the mapped draw does (`vs_bar_selection_mapped`).
    pub(in crate::renderer) bar_selection_mapped: Option<wgpu::RenderPipeline>,
    /// The category bar draw (`vs_category_bars` / `fs_category_bars`).
    pub(in crate::renderer) bar_category: Option<wgpu::RenderPipeline>,
    /// The picked category bar's highlight over the same layout.
    pub(in crate::renderer) bar_category_selection: Option<wgpu::RenderPipeline>,
    /// The box plot draw (`vs_box_plot` / `fs_box_plot`) over the category
    /// bar layout.
    pub(in crate::renderer) bar_box_plot: Option<wgpu::RenderPipeline>,
    /// The picked box's highlight over the same layout.
    pub(in crate::renderer) bar_box_plot_selection: Option<wgpu::RenderPipeline>,
    /// The range bar draw (`vs_range_bars` / `fs_range_bars`) over the
    /// category bar layout.
    pub(in crate::renderer) bar_range: Option<wgpu::RenderPipeline>,
    /// The picked range bar's highlight over the same layout.
    pub(in crate::renderer) bar_range_selection: Option<wgpu::RenderPipeline>,
    /// A box plot's violin bodies (`vs_violin` / `fs_violin`) over the
    /// category bar layout.
    pub(in crate::renderer) bar_violin: Option<wgpu::RenderPipeline>,
    /// The picked violin's highlight over the same layout.
    pub(in crate::renderer) bar_violin_selection: Option<wgpu::RenderPipeline>,
    /// A box plot's raw points (`vs_box_points` / `fs_box_points`) over the
    /// category bar layout.
    pub(in crate::renderer) bar_box_points: Option<wgpu::RenderPipeline>,
    /// The picked raw point's ring over the same layout.
    pub(in crate::renderer) bar_box_point_selection: Option<wgpu::RenderPipeline>,
    /// The value labels of category bar, box plot and radar series
    /// (`vs_value_label` / `fs_value_label`), group 1 the series' value
    /// label group.
    pub(in crate::renderer) bar_value_label: Option<wgpu::RenderPipeline>,
    /// The selected value label's box over the same layout.
    pub(in crate::renderer) bar_value_label_selection: Option<wgpu::RenderPipeline>,
    pub(in crate::renderer) field: Option<wgpu::RenderPipeline>,
    pub(in crate::renderer) field_selection: Option<wgpu::RenderPipeline>,
    /// The selected contour level's highlight for a labelled contour, with
    /// group 3 bound to its label gaps (`fs_data_selection_labelled`).
    pub(in crate::renderer) field_selection_labelled: Option<wgpu::RenderPipeline>,
    /// The field quad through `fs_contour` — the isolines. Same module and same
    /// layouts as `field`, one entry point apart.
    pub(in crate::renderer) contour: Option<wgpu::RenderPipeline>,
    /// The same contour pass with group 3 bound to selected label anchors, so
    /// the stroke is omitted under the label quads.
    pub(in crate::renderer) contour_labelled: Option<wgpu::RenderPipeline>,
    /// The contour label draw. Compiled from `Renderer::contour_label_pipelines`,
    /// which owns the module and the group-1 layout.
    pub(in crate::renderer) contour_label: Option<wgpu::RenderPipeline>,
    /// The assembled vector pipelines, one per input form (`vector.rs`).
    pub(in crate::renderer) vector_points: Option<wgpu::RenderPipeline>,
    pub(in crate::renderer) vector_grid: Option<wgpu::RenderPipeline>,
    /// The streamline arrowheads (`VectorInput::Path`), drawn indirect over
    /// the arc slots.
    pub(in crate::renderer) vector_path: Option<wgpu::RenderPipeline>,
    pub(in crate::renderer) sample_count: u32,
    pub(in crate::renderer) styled: HashMap<StyleKey, StyleSet>,
}

impl TargetPipelines {
    /// Prepare-phase ensure (`&mut`): compile and cache the pipeline set of
    /// every style that `items` draw with and the cache doesn't hold yet.
    /// Run before pass recording so the draw phase (`&self`) only looks up.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::renderer) fn ensure_styles_for_items(
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
    pub(in crate::renderer) fn ensure_surface_pipelines(
        &mut self,
        device: &wgpu::Device,
        transform_bgl: &wgpu::BindGroupLayout,
        style_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        surface_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        items: &[ChartDrawItem<'_>],
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
        transform_bgl: &wgpu::BindGroupLayout,
        region_bgl: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        items: &[ChartDrawItem<'_>],
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
                transform_bgl,
                region_bgl,
                surface_format,
                pass,
            );
        }
        self.ensure_ternary_region_scan(device);
    }

    /// The region module's shader, compiled once.
    pub(super) fn ternary_region_module(&mut self, device: &wgpu::Device) -> &wgpu::ShaderModule {
        self.ternary_region_shader.get_or_insert_with(|| {
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
        })
    }

    /// Compile the ring-start scan when it is missing, and return it.
    pub(in crate::renderer) fn ensure_ternary_region_scan(
        &mut self,
        device: &wgpu::Device,
    ) -> std::sync::Arc<data_render::ternary_region::RingScanPipelines> {
        if let Some(scan) = &self.ternary_region_scan {
            return std::sync::Arc::clone(scan);
        }
        let scan = std::sync::Arc::new(data_render::ternary_region::create_ring_scan_pipelines(
            device,
            self.ternary_region_module(device),
        ));
        self.ternary_region_scan = Some(std::sync::Arc::clone(&scan));
        scan
    }

    /// Compile the ternary region module and the one pipeline `pass` when
    /// they are missing.
    pub(in crate::renderer) fn ensure_ternary_region_pass(
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
        let sample_count = self.sample_count;
        let shader = self.ternary_region_module(device);
        let pipeline = Some(data_render::ternary_region::create_ternary_region_pipeline(
            device,
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
                data_render::surface_mesh::SurfaceInput::Grid,
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
        bgls: [&wgpu::BindGroupLayout; 4],
        surface_format: wgpu::TextureFormat,
        pass: data_render::surface_mesh::SurfacePass,
    ) {
        let [transform_bgl, style_bgl, field_bgl, surface_bgl] = bgls;
        if self.tri_surface[pass as usize].is_some() {
            return;
        }
        let shader: &wgpu::ShaderModule = if pass.on_floor() {
            self.tri_mesh_shader
                .get_or_insert_with(|| data_render::tri_mesh::shader_module(device))
        } else {
            self.tri_surface_shader.get_or_insert_with(|| {
                crate::gpu_compile::shader_module(
                    device,
                    wgpu::ShaderModuleDescriptor {
                        label: Some("figgy triangle surface shader"),
                        source: wgpu::ShaderSource::Wgsl(
                            data_render::surface_mesh::TRI_SURFACE_SHADER.source.into(),
                        ),
                    },
                )
            })
        };
        self.tri_surface[pass as usize] = Some(data_render::surface_mesh::create_surface_pipeline(
            device,
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
        [transform_bgl, style_bgl, field_bgl]: [&wgpu::BindGroupLayout; 3],
        surface_format: wgpu::TextureFormat,
        pass: data_render::tri_mesh::TriMeshPass,
    ) {
        if self.tri_mesh[pass as usize].is_some() {
            return;
        }
        let shader = self
            .tri_mesh_shader
            .get_or_insert_with(|| data_render::tri_mesh::shader_module(device));
        self.tri_mesh[pass as usize] = Some(data_render::tri_mesh::create_tri_mesh_pipeline(
            device,
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
