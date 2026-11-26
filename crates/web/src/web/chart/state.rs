//! `FiggyChart`'s chart state: the config, series and selection
//! accessors the other files share, and the id-keyed series registry.

use super::*;

impl FiggyChart {
    /// What a radial chart created on this chart draws with (#313,
    /// `FiggyBoundedChart.create_on`): this chart's device bundle (its
    /// device, queue, device-loss record, GPU ledger and budget) and the
    /// instance and adapter its own surface came from. Clones of handles:
    /// the device lives while either chart holds them.
    pub(crate) fn shared_gpu(&self) -> (renderer::RendererDevice, wgpu::Instance, wgpu::Adapter) {
        (
            self.renderer.renderer_device(),
            self.renderer.instance().clone(),
            self.renderer.adapter().clone(),
        )
    }

    pub(in crate::web) fn chart_config(&self) -> Result<&renderer::Config, JsValue> {
        match self.monotonic {
            Some(id) => self.renderer.monotonic_config(id),
            None => self.renderer.chart_config(self.chart_id),
        }
        .map_err(js_err)
    }

    pub(in crate::web) fn chart_series(&self) -> Result<&[SeriesConfig], JsValue> {
        match self.monotonic {
            Some(id) => self.renderer.monotonic_series(id),
            None => self.renderer.chart_series(self.chart_id),
        }
        .map_err(js_err)
    }

    pub(in crate::web) fn chart_selection(&self) -> Result<Option<HitId>, JsValue> {
        match self.monotonic {
            Some(id) => self.renderer.monotonic_selection(id),
            None => self.renderer.chart_selection(self.chart_id),
        }
        .map_err(js_err)
    }

    /// Selects a chart element of the chart the kernel shows.
    pub(in crate::web) fn set_chart_selection(
        &mut self,
        selected: Option<HitId>,
    ) -> Result<(), JsValue> {
        match self.monotonic {
            Some(id) => self
                .renderer
                .monotonic_chart(id)
                .and_then(|mut chart| chart.set_selection(selected)),
            None => self.renderer.set_chart_selection(self.chart_id, selected),
        }
        .map_err(js_err)
    }

    /// The document as the shown chart stores it, through the general call
    /// or a monotonic chart's editor (#271), which refuses a document that
    /// would move its x-to-pixel mapping.
    pub(in crate::web) fn write_chart_config(
        &mut self,
        config: renderer::Config,
    ) -> Result<(), JsValue> {
        match self.monotonic {
            Some(id) => self
                .renderer
                .monotonic_chart(id)
                .and_then(|mut chart| chart.edit_config(|editor| editor.assign(config)))
                .map_err(js_err)?
                .map_err(|kind| {
                    js_err(renderer::FiggyError::InvalidConfig {
                        field: "config",
                        kind,
                    })
                }),
            None => self
                .renderer
                .set_chart_config(self.chart_id, config)
                .map_err(js_err),
        }
    }

    /// Whether `next` holds another annotation list than the shown
    /// document: another number of items or another id at some index. The
    /// selection's index names the same item only in the same list; an
    /// edit of an item's content keeps it.
    pub(in crate::web) fn annotations_replaced_by(
        &self,
        next: &renderer::Config,
    ) -> Result<bool, JsValue> {
        let current = &self.chart_config()?.annotations.items;
        let next = &next.annotations.items;
        Ok(current.len() != next.len() || current.iter().zip(next).any(|(a, b)| a.id != b.id))
    }

    pub(in crate::web) fn replace_chart_config(
        &mut self,
        config: renderer::Config,
    ) -> Result<(), JsValue> {
        self.write_chart_config(config)?;
        self.sync_hitmap()?;
        Ok(())
    }

    /// The series as the shown chart stores them, its document kept as
    /// stored: an edit of the series alone (a drag of a link exit or a
    /// value label, #442) neither copies nor re-validates the document.
    pub(in crate::web) fn replace_chart_series(
        &mut self,
        mut series: Vec<SeriesConfig>,
    ) -> Result<(), JsValue> {
        place_feynman_vertices(&mut series)?;
        match self.monotonic {
            Some(id) => self
                .renderer
                .monotonic_chart(id)
                .and_then(|mut chart| chart.set_series(series)),
            None => self.renderer.set_chart_series(self.chart_id, series),
        }
        .map_err(js_err)?;
        self.sync_hitmap()?;
        Ok(())
    }

    pub(in crate::web) fn replace_chart_state(
        &mut self,
        config: renderer::Config,
        mut series: Vec<SeriesConfig>,
    ) -> Result<(), JsValue> {
        place_feynman_vertices(&mut series)?;
        match self.monotonic {
            // The series first; a document the editor refuses puts them back.
            Some(id) => {
                let previous = self.chart_series()?.to_vec();
                self.renderer
                    .monotonic_chart(id)
                    .and_then(|mut chart| chart.set_series(series))
                    .map_err(js_err)?;
                if let Err(error) = self.write_chart_config(config) {
                    // The previous series were valid on this document.
                    self.renderer
                        .monotonic_chart(id)
                        .and_then(|mut chart| chart.set_series(previous))
                        .map_err(js_err)?;
                    return Err(error);
                }
            }
            None => self
                .renderer
                .set_chart_state(self.chart_id, config, series)
                .map_err(js_err)?,
        }
        self.sync_hitmap()?;
        Ok(())
    }

    /// The hit map's registration set depends only on the active layer,
    /// the overlay count, the active subplot's overlay count, the subplot
    /// count (every subplot registers on the base layer) and the active
    /// overlay's links (an own X axis registers on the top side, a followed
    /// Y axis registers the chart's left one) and the number of polar
    /// category labels, each its own element (#309). Rebuild
    /// it when any of them changed; the registry has already dropped any
    /// `HitId` of the previous map. A layer change also clears the
    /// annotation selection: it is the element selection of the
    /// annotation layer, one level below the layer like the registry's
    /// `HitId`.
    pub(in crate::web) fn sync_hitmap(&mut self) -> Result<(), JsValue> {
        // Held aside while the config is borrowed; an early error leaves it
        // empty, which only rebuilds the map on the next call.
        let mut categories = std::mem::take(&mut self.hitmap_categories);
        let config = self.chart_config()?;
        let layer = config.active_layer();
        let subplot_overlays = match layer {
            renderer::LayerRef::Subplot(index) | renderer::LayerRef::SubplotOverlay(index, _) => {
                config
                    .subplots
                    .get(index)
                    .map(|subplot| subplot.config.overlays.len())
            }
            _ => None,
        };
        let key = (
            layer,
            config.overlays.len(),
            subplot_overlays,
            config.subplots.len(),
            config
                .overlay_of_layer(layer)
                .map(|overlay| (overlay.link_x, overlay.link_y)),
        );
        let categories_changed = !categories
            .iter()
            .copied()
            .eq(config.polar_category_label_counts());
        if categories_changed {
            categories.clear();
            categories.extend(config.polar_category_label_counts());
        }
        let rebuild = (key != self.hitmap_layer || categories_changed)
            .then(|| HitMap::for_active_layer(config));
        self.hitmap_categories = categories;
        if let Some(map) = rebuild {
            let layer_changed = key.0 != self.hitmap_layer.0;
            self.hitmap = map;
            self.hitmap_layer = key;
            self.dragging = false;
            self.resizing = None;
            self.annotation_pointer = false;
            self.value_label_pointer = false;
            self.feynman_pointer = None;
            if layer_changed {
                self.set_selected_annotation(None);
            }
        }
        Ok(())
    }

    /// The display scale and where the whole document lands on the
    /// surface: fitted to it, or where the held placement puts it while a
    /// subplot frame drags (#205), so pointer conversions and picks follow
    /// what is drawn.
    pub(in crate::web) fn display_scale_and_panel(&self) -> Result<(f32, Rect), JsValue> {
        if self.display_held {
            let (placed, _, scale) = self.display_config()?;
            return Ok((scale, placed.document_area()));
        }
        let logical = self.chart_config()?.document_area();
        Ok(renderer::fit_display_panel(
            (logical.width, logical.height),
            self.surface_size,
        ))
    }

    /// The config, panel rect and scale the canvas draws with: the
    /// renderer's placed config for the display `create` and `resize` set,
    /// borrowed while it is current.
    pub(in crate::web) fn display_config(
        &self,
    ) -> Result<(std::borrow::Cow<'_, renderer::config::Config>, Rect, f32), JsValue> {
        match self.monotonic {
            Some(id) => self.renderer.monotonic_display_config(id),
            None => self.renderer.chart_display_config(self.chart_id),
        }
        .map_err(js_err)
    }

    pub(super) fn render_stamp(&self) -> Result<renderer::ChartRenderStamp, JsValue> {
        match self.monotonic {
            Some(id) => self.renderer.monotonic_render_stamp(id),
            None => self.renderer.chart_render_stamp(self.chart_id),
        }
        .map_err(js_err)
    }

    pub(in crate::web) fn display_delta_to_document(
        &self,
        dx: f32,
        dy: f32,
    ) -> Result<(f32, f32), JsValue> {
        let (scale, _) = self.display_scale_and_panel()?;
        Ok(if scale > 0.0 {
            (dx / scale, dy / scale)
        } else {
            (dx, dy)
        })
    }

    /// A display point in document coordinates: the inverse of
    /// `display_config_for_surface`'s scale and placement.
    pub(in crate::web) fn display_point_to_document(
        &self,
        x: f32,
        y: f32,
    ) -> Result<[f32; 2], JsValue> {
        let (scale, panel) = self.display_scale_and_panel()?;
        let logical = self.chart_config()?.document_area();
        Ok(if scale > 0.0 {
            [
                logical.x as f32 + (x - panel.x as f32) / scale,
                logical.y as f32 + (y - panel.y as f32) / scale,
            ]
        } else {
            [x, y]
        })
    }

    pub(in crate::web) fn label_text(&self, label: &str) -> Result<RichText, JsValue> {
        let content = &self.chart_config()?.legend.content;
        Ok(RichText {
            segments: rich_segments_from_label(label),
            color: content.color,
            font_size: content.font_size,
            font: content.font.clone(),
            align: content.align,
        })
    }

    pub(in crate::web) fn request_host_redraw(&mut self) {
        self.redraw_pending = true;
    }

    /// Whether `id` is a registered column or triangle index column: a
    /// triangle mesh names its triangles among its columns (#210).
    pub(in crate::web) fn column_registered(&self, id: &str) -> bool {
        self.renderer.logical_column(id).is_some() || self.renderer.pool().triangles(id).is_some()
    }

    /// Keep an auto-managed legend on `new_cfg` as `set_config` does: a
    /// heatmap's legend mark shows the colour map, so an auto-managed legend
    /// follows a new map while a host-written one stays as written.
    /// Returns whether `new_cfg` writes its own legend content, which ends
    /// the auto-management once it is installed.
    pub(in crate::web) fn follow_config_legend(
        &self,
        new_cfg: &mut renderer::Config,
        series: &[SeriesConfig],
    ) -> Result<bool, JsValue> {
        let old_config = self.chart_config()?;
        let legend_content_changed = old_config.legend.content != new_cfg.legend.content;
        let colormap_changed = old_config.colorbar.as_ref().map(|bar| &bar.colormap)
            != new_cfg.colorbar.as_ref().map(|bar| &bar.colormap);
        if colormap_changed && !legend_content_changed && self.legend_auto_managed {
            renderer::config::update_legend_symbols_preserving_text(
                &mut new_cfg.legend.content,
                series,
                new_cfg.colorbar.as_ref().map(|bar| &bar.colormap),
            );
        }
        Ok(legend_content_changed)
    }

    pub(in crate::web) fn ensure_columns_exist(&self, cfg: &SeriesConfig) -> Result<(), JsValue> {
        // A monotonic line series' columns are its envelope, which its
        // group's reduction writes (#271): the renderer skips them as well
        // (`validate.rs`), and refuses the series on a general chart with
        // `MonotonicGeneralChart`.
        if cfg.monotonic().is_some() {
            return Ok(());
        }
        for id in referenced_columns(cfg) {
            if !self.column_registered(id) {
                return Err(js_err(format!(
                    "series '{}' references unregistered column '{id}'",
                    cfg.series_id
                )));
            }
        }
        Ok(())
    }
}

#[wasm_bindgen]
impl FiggyChart {
    // ---- series registry (id-keyed upsert / unregister) ----

    /// Register or update a line series over two registered columns.
    ///
    /// Upsert by `series_id`: a new id takes the next color of the active
    /// cycle; an existing id is replaced in place and keeps its color.
    /// Non-empty `label` adds/updates the legend row; its `$…$` spans are
    /// TeX and `\$` is a dollar sign.
    pub fn add_line_series(
        &mut self,
        series_id: &str,
        x_column: &str,
        y_column: &str,
        line_width: f32,
        label: &str,
    ) -> Result<(), JsValue> {
        let current = self.chart_series()?.to_vec();
        let existing = current.iter().position(|c| c.series_id == series_id);
        let color = match existing {
            // The series' own colour: its line's, or, with markers only,
            // its markers' (a scatter, a stem or a series with errors).
            Some(i) => match &current[i].render_type {
                DataRenderType::Xy(renderer::data_config::XySeries {
                    line: Some(line), ..
                }) => line.line_color,
                DataRenderType::Xy(renderer::data_config::XySeries {
                    scatter: Some(scatter),
                    ..
                }) => scatter.point_color,
                _ => self.cycle.color(i),
            },
            None => self.cycle.color(self.color_seq),
        };
        let next_color_seq = if existing.is_none() {
            self.color_seq
                .checked_add(1)
                .ok_or_else(|| js_err("series color sequence exhausted"))?
        } else {
            self.color_seq
        };
        let label_changed = !label.is_empty();
        let rich_label = if label_changed {
            Some(self.label_text(label)?)
        } else {
            existing.and_then(|i| current[i].label.clone())
        };

        let cfg = SeriesConfig {
            series_id: series_id.into(),
            source_id: None,
            label: rich_label.clone(),
            x_column: x_column.into(),
            y_column: y_column.into(),
            z_column: None,
            render_type: DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: None,
                line: Some(DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: color,
                    line_width: line_width.max(0.5),
                    fill: None,
                    fill_color: None,
                    interpolation: renderer::data_config::LineInterpolation::Linear,
                }),
                errors: None,
                stem: None,
            }),
        };
        self.ensure_columns_exist(&cfg)?;
        let legend_entry = renderer::config::legend_entry_of_series(
            &self.chart_config()?.legend.content,
            &current,
            series_id,
        );
        let mut proposed = current;
        if let Some(index) = existing {
            proposed[index] = cfg.clone();
        } else {
            proposed.try_reserve(1).map_err(js_err)?;
            proposed.push(cfg.clone());
        }
        self.renderer
            .preflight_chart_styles(&proposed)
            .map_err(js_err)?;
        let mut next_config = self.chart_config()?.clone();
        if label_changed && let Some(label) = rich_label {
            match legend_entry {
                Some(index) => renderer::config::set_legend_entry_label(
                    &mut next_config.legend.content,
                    index,
                    renderer::config::series_symbol_segments(
                        &cfg,
                        next_config.colorbar.as_ref().map(|bar| &bar.colormap),
                    ),
                    label.segments,
                ),
                None => renderer::config::append_legend_entry_rich(
                    &mut next_config.legend.content,
                    renderer::config::series_symbol_segments(
                        &cfg,
                        next_config.colorbar.as_ref().map(|bar| &bar.colormap),
                    ),
                    label.segments,
                ),
            }
            next_config.legend.visible = true;
        } else {
            renderer::config::update_legend_symbols_preserving_text(
                &mut next_config.legend.content,
                &proposed,
                next_config.colorbar.as_ref().map(|bar| &bar.colormap),
            );
        }

        self.replace_chart_state(next_config, proposed)?;
        self.color_seq = next_color_seq;
        Ok(())
    }

    /// Set / change / remove a series' legend label. `'\n'` breaks lines;
    /// unicode sub/superscripts (`₀`, `⁻`, …) map to styled segments;
    /// `$…$` spans are TeX and `\$` is a dollar sign.
    /// Empty string removes the legend row. Returns `true` when the
    /// series exists.
    pub fn set_series_label(&mut self, series_id: &str, label: &str) -> Result<bool, JsValue> {
        let mut series = self.chart_series()?.to_vec();
        let Some(i) = series.iter().position(|c| c.series_id == series_id) else {
            return Ok(false);
        };
        let mut config = self.chart_config()?.clone();
        let entry =
            renderer::config::legend_entry_of_series(&config.legend.content, &series, series_id);
        if label.is_empty() {
            series[i].label = None;
            if let Some(entry) = entry
                && renderer::config::remove_legend_entry(&mut config.legend.content, entry)
                && config.legend.content.segments.is_empty()
            {
                config.legend.visible = false;
            }
        } else {
            let label = self.label_text(label)?;
            series[i].label = Some(label.clone());
            let symbol = renderer::config::series_symbol_segments(
                &series[i],
                config.colorbar.as_ref().map(|bar| &bar.colormap),
            );
            match entry {
                Some(entry) => renderer::config::set_legend_entry_label(
                    &mut config.legend.content,
                    entry,
                    symbol,
                    label.segments,
                ),
                None => renderer::config::append_legend_entry_rich(
                    &mut config.legend.content,
                    symbol,
                    label.segments,
                ),
            }
            config.legend.visible = true;
        }
        self.replace_chart_state(config, series)?;
        Ok(true)
    }

    /// Unregister a series. Columns stay registered. Auto-managed legends
    /// remove its row; freely edited legends preserve their text and only
    /// synchronize recognized series symbols.
    /// Returns `true` when the series existed.
    pub fn remove_series(&mut self, series_id: &str) -> Result<bool, JsValue> {
        let mut series = self.chart_series()?.to_vec();
        let Some(i) = series.iter().position(|c| c.series_id == series_id) else {
            return Ok(false);
        };
        let mut config = self.chart_config()?.clone();
        let entry =
            renderer::config::legend_entry_of_series(&config.legend.content, &series, series_id);
        series.remove(i);
        if self.legend_auto_managed {
            if let Some(entry) = entry
                && renderer::config::remove_legend_entry(&mut config.legend.content, entry)
                && config.legend.content.segments.is_empty()
            {
                config.legend.visible = false;
            }
        } else {
            renderer::config::update_legend_symbols_preserving_text(
                &mut config.legend.content,
                &series,
                config.colorbar.as_ref().map(|bar| &bar.colormap),
            );
        }
        self.replace_chart_state(config, series)?;
        Ok(true)
    }

    /// Replace one contour series' levels with axis-native nice levels.
    /// The resulting explicit values remain the series SSoT.
    pub fn set_contour_nice_levels(
        &mut self,
        series_id: &str,
        target_count: usize,
        use_colormap_colors: bool,
    ) -> Result<u32, JsValue> {
        let colorbar = self
            .chart_config()?
            .colorbar
            .clone()
            .ok_or_else(|| js_err("contour levels need Config.colorbar"))?;
        let mut series = self.chart_series()?.to_vec();
        let cfg = series
            .iter_mut()
            .find(|cfg| cfg.series_id == series_id)
            .ok_or_else(|| js_err("no series with this id in the chart"))?;
        let contour = match &mut cfg.render_type {
            DataRenderType::Field(renderer::data_config::FieldSeries {
                contour: Some(contour),
                ..
            }) => contour,
            _ => return Err(js_err("series does not have contour levels")),
        };
        contour
            .set_nice_levels(&colorbar.axis, target_count)
            .map_err(|error| js_err(format!("{error:?}")))?;
        if use_colormap_colors {
            contour.set_colormap_colors(&colorbar);
        }
        let count = u32::try_from(contour.levels.len()).map_err(js_err)?;
        let json = serde_json::to_string(&series).map_err(js_err)?;
        self.set_series(&json)?;
        self.request_host_redraw();
        Ok(count)
    }

    /// Report the effective primitive/grid extent currently drawn.
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    /// `{ drawn_count, cols, rows, truncated }`: counts, not pixels.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn series_draw_info(&self, series_id: &str) -> Result<String, JsValue> {
        let info = match self.monotonic {
            Some(id) => self.renderer.monotonic_series_draw_info(id, series_id),
            None => self.renderer.series_draw_info(self.chart_id, series_id),
        }
        .map_err(js_err)?;
        serde_json::to_string(&serde_json::json!({
            "drawn_count": info.drawn_count,
            "cols": info.cols,
            "rows": info.rows,
            "truncated": info.truncated,
        }))
        .map_err(js_err)
    }
}
