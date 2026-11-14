//! Creating and refreshing chart views, their layers and annotations.

use super::*;

impl Renderer {
    /// Build the per-panel GPU resources: grid + decoration textures and
    /// bind groups, plus the transform uniform buffer + bind group.
    /// `panel_rect` is the panel's pixel rect in surface coordinates.
    /// `chart.config_mut().chart_area` must match `panel_rect` before this
    /// call so the axis raster is drawn correctly. The chart is taken as
    /// stored (scale 1): [`Self::create_chart_view_at`] takes a placed or
    /// exported one.
    pub fn create_chart_view(&self, chart: &Chart, panel_rect: Rect) -> Result<ChartView> {
        self.create_chart_view_at(chart, panel_rect, 1.0)
    }

    /// [`Self::create_chart_view`] for a `chart` whose pixel values carry
    /// the display or export `scale` (#205): the view's rasters snap a
    /// broken axis' piece frames to whole logical pixels, then scale, so a
    /// 2x raster is the 1x raster doubled. The view keeps the scale for
    /// [`Self::refresh_axis`] and [`Self::refresh_axis_with_selection`].
    pub fn create_chart_view_at(
        &self,
        chart: &Chart,
        panel_rect: Rect,
        scale: f32,
    ) -> Result<ChartView> {
        self.health.check()?;
        let panels = expand_for_render(chart.config())?;
        let places = panel_places(chart.config(), &panels, panel_rect);
        let font_generation = crate::text_render::font_generation();
        let mut view =
            self.create_panel_view(&panels[0], panel_rect, false, font_generation, scale)?;
        for (panel, place) in panels[1..].iter().zip(&places[1..]) {
            view.overlay_views.push(self.create_panel_view(
                panel,
                place.rect,
                !place.draws_backdrop,
                font_generation,
                scale,
            )?);
        }
        // Annotation layer: two optional textures from one shared raster
        // pass. One layer per document, on the base view, anchored in the
        // parent's own axis space (the group data area).
        let (w, h) = checked_panel_raster_size(self.caps, chart.config(), panel_rect)?;
        let annotations = annotation_raster_for_chart(chart, None)?;
        view.annotation_underlay = annotations
            .underlay
            .as_deref()
            .map(|rgba| self.create_layer_texture(rgba, w, h))
            .transpose()?;
        view.annotation_overlay = annotations
            .overlay
            .as_deref()
            .map(|rgba| self.create_layer_texture(rgba, w, h))
            .transpose()?;
        view.annotation_map = annotations.map;
        view.annotation_key = Some(AnnotationRasterKey {
            config: chart.config().clone(),
            selected: None,
            font_generation,
            panel_rect,
        });
        view.tick_warnings = merged_render_warnings(chart.config(), &panels);
        view.rebuild_render_warnings();
        // Drawn from the chart as it stands, without a selection: what
        // `prepare` would draw for it, so the first prepare keeps it.
        if chart.selected().is_none() && panel_rect == chart.config().chart_area.0 {
            view.raster_source = chart.raster_stamp().map(|stamp| RasterSource {
                chart: RasterChart::Held { stamp },
                font_generation,
                panel_rect,
                scale,
            });
        }
        Ok(view)
    }

    /// One panel's grid, decoration and transform resources from one
    /// internal config. `overlay` panels never draw the Milkyway backdrop:
    /// the grid slot belongs to the first panel of their area. No panel carries annotation
    /// textures here; `create_chart_view` gives them to the base view.
    pub(in crate::renderer) fn create_panel_view(
        &self,
        config: &Config,
        panel_rect: Rect,
        overlay: bool,
        font_generation: u64,
        scale: f32,
    ) -> Result<ChartView> {
        let (w, h) = checked_panel_raster_size(self.caps, config, panel_rect)?;

        // Both raster layers from one placement pass: the grid (drawn below
        // data) and the decoration (drawn above). An overlay panel in the
        // Milkyway style gets a transparent grid: the backdrop is the base
        // panel's.
        let (grid_rgba, dec_rgba) = raster_panel_layers(config, overlay, &[], true, scale)?;
        let grid = self.create_layer_texture(&grid_rgba, w, h)?;
        let decoration = self.create_layer_texture(&dec_rgba, w, h)?;

        let t = data_render::scatter_transform_from_config(config);
        let transform_buffer = TrackedBuffer::new(
            &self.gpu_ledger,
            GpuResourceKind::Uniform,
            data_render::create_scatter_transform_uniform_buffer(&self.device, &t),
        );
        let transform_bg = data_render::create_scatter_transform_bind_group(
            &self.device,
            &self.transform_bgl,
            &transform_buffer,
        );

        Ok(ChartView {
            grid_texture: grid.texture,
            grid_bind_group: grid.bind_group,
            decoration_texture: decoration.texture,
            decoration_bind_group: decoration.bind_group,
            annotation_underlay: None,
            annotation_overlay: None,
            annotation_map: crate::annotation::AnnotationMap::empty(
                crate::annotation::pixel_space_for_config(config),
            ),
            annotation_selected: None,
            layer_stamps: ChartViewLayerStamps {
                grid: 1,
                decoration: 1,
                annotations: 1,
            },
            transform_buffer,
            transform_bg,
            content_revision: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            stream_revision: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            panel_rect,
            display_scale: scale,
            grid_space_gen: None,
            tick_warnings: axis_render::tick_warnings(config),
            render_warnings: Vec::new(),
            overlay_views: Vec::new(),
            raster_key: Some(PanelRasterKey::new(
                config,
                &[],
                font_generation,
                panel_rect,
                scale,
            )),
            raster_generation: 1,
            raster_source: None,
            annotation_key: None,
        })
    }

    /// Upload one panel-sized raster as a texture with its bind group.
    pub(in crate::renderer) fn create_layer_texture(
        &self,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> Result<crate::annotation::LayerTexture> {
        let texture = TrackedTexture::new(
            &self.gpu_ledger,
            GpuResourceKind::PanelTexture,
            data_render::upload_rgba_texture(
                &self.device,
                &self.queue,
                self.caps.max_texture_dimension_2d,
                w,
                h,
                rgba,
            )?,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = data_render::create_texture_bind_group(
            &self.device,
            &self.texture_bgl,
            &view,
            &self.sampler,
        );
        Ok(crate::annotation::LayerTexture {
            texture,
            bind_group,
        })
    }

    /// Re-rasterize only the annotation layer of `view` from `chart`:
    /// the underlay and overlay textures, the annotation hit map and the
    /// annotation warnings. The grid and decoration textures are not
    /// touched, so an annotation edit never rewrites axis labels. `selected`
    /// is the annotation whose selection box the overlay draws; the view
    /// remembers it for the next `refresh_axis*`.
    ///
    /// Call it when [`ChartRenderStamp::needs_annotation_raster_since`] says
    /// the items changed or when the annotation selection changed. A panel
    /// rect that differs from the view's moves the anchor space as well, so
    /// that case refreshes every layer like [`Self::refresh_axis`].
    pub fn refresh_annotations(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selected: Option<usize>,
    ) -> Result<()> {
        let result = self.refresh_annotations_unreported(view, chart, panel_rect, selected);
        self.reported(result)
    }

    pub(super) fn refresh_annotations_unreported(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selected: Option<usize>,
    ) -> Result<()> {
        self.health.check()?;
        view.annotation_selected = selected;
        if view.panel_rect != panel_rect {
            return self.refresh_axis(view, chart, panel_rect);
        }
        view.advance_content_revision()?;
        self.refresh_annotation_layers(view, chart, panel_rect)
    }

    pub(in crate::renderer) fn refresh_annotation_layers(
        &self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
    ) -> Result<()> {
        let (w, h) = checked_panel_raster_size(self.caps, chart.config(), panel_rect)?;
        let key = AnnotationRasterKey {
            config: chart.config().clone(),
            selected: view.annotation_selected,
            font_generation: crate::text_render::font_generation(),
            panel_rect,
        };
        if view.annotation_key.as_ref() == Some(&key) {
            return Ok(());
        }
        // Unknown until the raster is uploaded: a failure leaves it stale.
        view.annotation_key = None;
        let raster = annotation_raster_for_chart(chart, view.annotation_selected)?;
        self.refresh_optional_layer(
            &mut view.annotation_underlay,
            raster.underlay.as_deref(),
            w,
            h,
        )?;
        self.refresh_optional_layer(
            &mut view.annotation_overlay,
            raster.overlay.as_deref(),
            w,
            h,
        )?;
        view.annotation_map = raster.map;
        view.layer_stamps.annotations += 1;
        view.annotation_key = Some(key);
        view.rebuild_render_warnings();
        Ok(())
    }

    pub(in crate::renderer) fn refresh_optional_layer(
        &self,
        slot: &mut Option<crate::annotation::LayerTexture>,
        rgba: Option<&[u8]>,
        w: u32,
        h: u32,
    ) -> Result<()> {
        match (slot.as_mut(), rgba) {
            (Some(layer), Some(rgba)) => {
                self.refresh_one_layer(&mut layer.texture, &mut layer.bind_group, rgba, w, h)
            }
            (None, Some(rgba)) => {
                *slot = Some(self.create_layer_texture(rgba, w, h)?);
                Ok(())
            }
            (_, None) => {
                *slot = None;
                Ok(())
            }
        }
    }

    /// Write only the data transform uniform; leave axis/grid/decorations unchanged.
    ///
    /// An axis-range edit normally needs [`Self::refresh_axis`] as well to update
    /// tick labels and the grid. In the prepare/paint flow, refresh those raster
    /// layers and let [`Self::prepare`] write the transform from the same config;
    /// no separate call to this method is needed.
    pub fn update_transform(&mut self, view: &ChartView, chart: &Chart) -> Result<()> {
        let result = self.update_transform_unreported(view, chart);
        self.reported(result)
    }

    pub(super) fn update_transform_unreported(
        &mut self,
        view: &ChartView,
        chart: &Chart,
    ) -> Result<()> {
        self.health.check()?;
        let panels = expand_for_render(chart.config())?;
        let views = layer_views(view, panels.len())?;
        view.advance_stream_revision()?;
        view.advance_content_revision()?;
        for (panel, layer_view) in panels.iter().zip(views) {
            let t = data_render::scatter_transform_from_config(panel);
            data_render::update_scatter_transform(&self.queue, &layer_view.transform_buffer, &t);
        }
        Ok(())
    }

    /// Re-rasterize both grid and decoration textures. Updates in place via
    /// `write_texture` when the size matches; allocates new textures
    /// otherwise. The data transform uniform needs no separate call in the
    /// prepare/paint flow — [`Self::prepare`] (and the `paint`/`draw`
    /// facades) writes it every frame; [`Self::update_transform`] remains
    /// for imperative one-off updates outside that flow.
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
        let result = self.refresh_axis_unreported(view, chart, panel_rect);
        self.reported(result)
    }

    pub(super) fn refresh_axis_unreported(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
    ) -> Result<()> {
        self.refresh_axis_with_selection(view, chart, panel_rect, None)
    }

    /// [`Self::refresh_axis`] plus the selection highlight of the element
    /// `selected` names composited into the decoration layer (selection draws
    /// above the data, never below). `selected` is the element id the hit
    /// map reports (`Selectable::element_id`, `"axis_bottom"`,
    /// `"overlay_0:legend"`, ...); the box is built from the element's drawn
    /// parts on the document's active layer, the same box
    /// [`Self::selection_box`] returns for host UI. An id no element of the
    /// active layer has draws nothing.
    pub fn refresh_axis_with_selection(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selected: Option<&str>,
    ) -> Result<()> {
        let scale = view.display_scale;
        let result =
            self.refresh_axis_with_selection_unreported(view, chart, panel_rect, selected, scale);
        self.reported(result)
    }

    /// [`Self::refresh_axis_with_selection`] for a `chart` whose pixel
    /// values carry the display or export `scale` (#205), which the view
    /// then keeps: a host that shows a document scaled to its surface passes
    /// the placement's scale with the placed chart, so a broken axis' piece
    /// frames snap to whole logical pixels and 2x stays 1x doubled.
    pub fn refresh_axis_with_selection_at(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selected: Option<&str>,
        scale: f32,
    ) -> Result<()> {
        let result =
            self.refresh_axis_with_selection_unreported(view, chart, panel_rect, selected, scale);
        self.reported(result)
    }

    pub(super) fn refresh_axis_with_selection_unreported(
        &mut self,
        view: &mut ChartView,
        chart: &Chart,
        panel_rect: Rect,
        selected: Option<&str>,
        scale: f32,
    ) -> Result<()> {
        self.health.check()?;
        // Unknown until the raster is drawn: a failure leaves the view stale.
        view.raster_source = None;
        view.display_scale = scale;
        let selection: Vec<crate::select::SelectionBox> = selected
            .and_then(|id| self.selection_box(chart, id))
            .into_iter()
            .collect();
        let selection = &selection[..];
        let panels = expand_for_render(chart.config())?;
        let places = panel_places(chart.config(), &panels, panel_rect);
        let font_generation = crate::text_render::font_generation();
        // The view follows the document's layer count: a layer added since
        // the last refresh gets its textures here, a removed one drops them.
        view.overlay_views.truncate(panels.len() - 1);
        while view.overlay_views.len() < panels.len() - 1 {
            let index = view.overlay_views.len() + 1;
            let place = &places[index];
            let layer = self.create_panel_view(
                &panels[index],
                place.rect,
                !place.draws_backdrop,
                font_generation,
                scale,
            )?;
            view.overlay_views.push(layer);
        }
        // Each selection box is composited into the topmost panel of the
        // area it sits in, so it is above every layer's decoration there, as
        // it was above the single one.
        let routed = route_selection_boxes(&places, selection);
        for (index, panel) in panels.iter().enumerate() {
            let layer_view = if index == 0 {
                &mut *view
            } else {
                &mut view.overlay_views[index - 1]
            };
            self.refresh_panel_view(
                layer_view,
                panel,
                places[index].rect,
                &routed[index],
                !places[index].draws_backdrop,
                font_generation,
                scale,
            )?;
        }
        view.panel_rect = panel_rect;
        view.tick_warnings = merged_render_warnings(chart.config(), &panels);
        // The anchor space follows the axes and the data area, which this
        // refresh may have moved, so the annotation layer is redone here as
        // well (its key skips an unchanged one); its own revision covers item
        // edits between axis refreshes.
        self.refresh_annotation_layers(view, chart, panel_rect)?;
        // Drawn from the chart as `prepare` would draw it (its own selection
        // and area): the next prepare keeps it.
        if selected == chart.selected() && panel_rect == chart.config().chart_area.0 {
            view.raster_source = chart.raster_stamp().map(|stamp| RasterSource {
                chart: RasterChart::Held { stamp },
                font_generation,
                panel_rect,
                scale,
            });
        }
        Ok(())
    }

    /// The selection box of the element `element_id` names on the active
    /// layer of `chart`, in absolute surface coordinates: what
    /// [`Self::refresh_axis_with_selection`] draws, for host UI that outlines
    /// or hit-tests the selection itself. `None` for an id the active layer
    /// has no element for, or an element that draws nothing.
    pub fn selection_box(
        &self,
        chart: &Chart,
        element_id: &str,
    ) -> Option<crate::select::SelectionBox> {
        let config = chart.config();
        let map = crate::select::HitMap::for_active_layer(config);
        let id = map.find(element_id)?;
        map.selection_box(
            id,
            config,
            &crate::text_render::CpuTextMeasure::for_style(&config.draw_style),
        )
    }

    /// Refresh one panel's textures from its internal config, unless the
    /// textures already hold exactly this raster.
    pub(in crate::renderer) fn refresh_panel_view(
        &mut self,
        view: &mut ChartView,
        config: &Config,
        panel_rect: Rect,
        selection: &[crate::select::SelectionBox],
        overlay: bool,
        font_generation: u64,
        scale: f32,
    ) -> Result<()> {
        let (w, h) = checked_panel_raster_size(self.caps, config, panel_rect)?;
        let key = PanelRasterKey::new(config, selection, font_generation, panel_rect, scale);
        if view.raster_key.as_ref() == Some(&key) {
            // Same raster: the revisions still advance so a token prepared
            // before this call is retired, exactly as for a real rewrite.
            // The Milkyway backdrop keeps its own cache and stamp: a view
            // created with a direct grid raster adopts the shared bake here.
            view.advance_content_revision()?;
            if let crate::config::DrawStyle::Milkyway(c) = &config.draw_style
                && !overlay
            {
                self.refresh_space_backdrop(view, c, w, h)?;
            }
            return Ok(());
        }

        // One placement pass lays out both layers; a base panel in the
        // Milkyway style takes its grid texture from the cached backdrop
        // instead of a grid raster.
        let backdrop = matches!(config.draw_style, DrawStyle::Milkyway(_)) && !overlay;
        let (grid_rgba, dec_rgba) =
            raster_panel_layers(config, overlay, selection, !backdrop, scale)?;
        if view.panel_rect != panel_rect {
            view.advance_stream_revision()?;
        }
        view.advance_content_revision()?;

        // Grid layer. The constellation backdrop is axis-range independent,
        // so it is cached by (size, nebula, dust, seed): a pan/zoom/config
        // commit re-rasters only the decoration layer, and a view whose grid
        // texture already holds the cached generation skips the upload too.
        // Overlay panels never draw the backdrop; the base panel owns it.
        if let crate::config::DrawStyle::Milkyway(c) = &config.draw_style
            && !overlay
        {
            self.refresh_space_backdrop(view, c, w, h)?;
        } else {
            self.refresh_one_layer(
                &mut view.grid_texture,
                &mut view.grid_bind_group,
                &grid_rgba,
                w,
                h,
            )?;
            view.grid_space_gen = None;
            view.layer_stamps.grid += 1;
        }

        self.refresh_one_layer(
            &mut view.decoration_texture,
            &mut view.decoration_bind_group,
            &dec_rgba,
            w,
            h,
        )?;
        view.layer_stamps.decoration += 1;
        view.panel_rect = panel_rect;
        view.tick_warnings = axis_render::tick_warnings(config);
        view.raster_key = Some(key);
        view.raster_generation = view.raster_generation.wrapping_add(1);

        Ok(())
    }

    /// Bake the Milkyway backdrop for `(w, h, nebula, dust, seed)` unless the
    /// renderer already holds it, and upload it into the view's grid texture
    /// unless the view already holds that generation.
    pub(in crate::renderer) fn refresh_space_backdrop(
        &mut self,
        view: &mut ChartView,
        c: &crate::config::MilkywayOptions,
        w: u32,
        h: u32,
    ) -> Result<()> {
        let key = SpaceBgKey {
            w,
            h,
            nebula: c.nebula.to_bits(),
            dust: c.dust.to_bits(),
            seed: c.seed,
        };
        if !self.space_bg.as_ref().is_some_and(|s| s.key == key) {
            let rgba = key.bake()?;
            let bake_gen = self.space_bg.as_ref().map_or(1, |s| s.bake_gen + 1);
            self.space_bg = Some(SpaceBgCache {
                key,
                bake_gen,
                rgba,
            });
        }
        let cache = self
            .space_bg
            .as_ref()
            .ok_or(FiggyError::internal("space_bg baked above"))?;
        if view.grid_space_gen != Some(cache.bake_gen) {
            self.refresh_one_layer(
                &mut view.grid_texture,
                &mut view.grid_bind_group,
                &cache.rgba,
                w,
                h,
            )?;
            view.grid_space_gen = Some(cache.bake_gen);
            view.layer_stamps.grid += 1;
        }
        Ok(())
    }

    /// Update one texture in place or by reallocation. Used for both
    /// the grid and decoration layers.
    pub(in crate::renderer) fn refresh_one_layer(
        &self,
        tex: &mut TrackedTexture,
        bg: &mut wgpu::BindGroup,
        rgba: &[u8],
        w: u32,
        h: u32,
    ) -> Result<()> {
        if rgba.len() as u64 != u64::from(w) * u64::from(h) * 4 {
            return Err(FiggyError::internal(
                "a panel raster has its texture's size (checked before rasterising)",
            ));
        }
        let same_size = tex.width() == w && tex.height() == h;
        if same_size {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
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
        } else {
            // The replacement is charged before the assignment below drops
            // the old one, so a resize reads as `old + new` for one frame —
            // which is what the device is holding.
            let new_tex = TrackedTexture::new(
                &self.gpu_ledger,
                GpuResourceKind::PanelTexture,
                data_render::upload_rgba_texture(
                    &self.device,
                    &self.queue,
                    self.caps.max_texture_dimension_2d,
                    w,
                    h,
                    rgba,
                )?,
            );
            let new_view_t = new_tex.create_view(&wgpu::TextureViewDescriptor::default());
            let new_bg = data_render::create_texture_bind_group(
                &self.device,
                &self.texture_bgl,
                &new_view_t,
                &self.sampler,
            );
            *tex = new_tex;
            *bg = new_bg;
        }
        Ok(())
    }
}
