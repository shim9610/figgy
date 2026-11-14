//! Bar, box and range bar selection highlights.

use super::*;

impl PrepareContext<'_> {
    /// A histogram bin's outline.
    pub(super) fn histogram_bin_selection<'a>(
        &'a self,
        s: &SelectionInputs<'a, '_>,
        selection_cfg: &crate::config::DataSelectionsConfig,
        bin_index: &usize,
        selected_bars: &mut Vec<ColumnBarSelectionLayer<'a>>,
    ) -> Result<()> {
        let SelectionInputs {
            view,
            series,
            pipelines,
            columns,
            rt,
            column_buffer,
            ..
        } = *s;
        let (edges, values) = columns
            .bar
            .ok_or(FiggyError::internal("bar primitive has selection columns"))?;
        let count = data_render::bar_instance_count(edges.len_values, values.len_values);
        let Some(instance) = columns.rows.instance(*bin_index, count as usize) else {
            return Ok(());
        };
        // The bin's global index keys its overrides, whatever
        // instance the outline draws.
        let Ok(global_bin) = u32::try_from(*bin_index) else {
            return Ok(());
        };
        // The draw's own unresolved bar `Style` with the
        // selection in lanes a bar never reads as fill or
        // border (`bar_columnar.wgsl`): the outline resolves
        // the bin's gap and width on the GPU exactly when the
        // draw binds the style map.
        let bar_cfg =
            extract_bar(rt).ok_or(FiggyError::internal("histogram selection has a bar config"))?;
        let mut outline_style = PrimitiveStyle::from_bar(bar_cfg, series.style.display_scale);
        outline_style.color_premul =
            PrimitiveStyle::from_color(selection_cfg.highlight_color).color_premul;
        outline_style.line_width_px = selection_cfg.outline_width_px;
        outline_style.dash_len = global_bin;
        let outline_buffer = data_render::create_style_uniform_buffer(&self.device, &outline_style);
        let style_bg =
            data_render::create_style_bind_group(&self.device, &self.style_bgl, &outline_buffer);
        let style_map = series.style.bar_map.as_ref();
        selected_bars.push(ColumnBarSelectionLayer {
            pipeline: match style_map {
                Some(_) => pipelines
                    .bar_selection_mapped
                    .as_ref()
                    .ok_or(FiggyError::internal(
                        "prepare ensured mapped histogram selection pipeline",
                    ))?,
                None => pipelines
                    .bar_selection
                    .as_ref()
                    .ok_or(FiggyError::internal(
                        "prepare ensured histogram selection pipeline",
                    ))?,
            },
            transform_bg: &view.transform_bg,
            style_bg,
            style_map_bg: style_map.map(|map| &map.bind_group),
            pool_buffer: column_buffer,
            edges,
            values,
            instance,
            instances: 1,
            category: None,
        });
        Ok(())
    }

    /// A box plot's box or violin body.
    pub(super) fn box_selection<'a>(
        &'a self,
        s: &SelectionInputs<'a, '_>,
        selection_cfg: &crate::config::DataSelectionsConfig,
        bin_index: &usize,
        half: &Option<ViolinSide>,
        selected_bars: &mut Vec<ColumnBarSelectionLayer<'a>>,
    ) -> Result<()> {
        let SelectionInputs {
            view,
            series,
            pipelines,
            columns,
            rt,
            column_buffer,
            ..
        } = *s;
        let Some(category) = columns.category.clone() else {
            return Ok(());
        };
        let Ok(row) = u32::try_from(*bin_index) else {
            return Ok(());
        };
        let boxes = rt.box_plot().ok_or(FiggyError::internal(
            "a box plot selection has a box plot config",
        ))?;
        let mut highlight =
            PrimitiveStyle::from_box_plot(boxes.color, boxes.style, series.style.display_scale);
        highlight.color_premul =
            PrimitiveStyle::from_color(selection_cfg.highlight_color).color_premul;
        highlight.line_width_px = selection_cfg.outline_width_px;
        highlight.dash_len = row;
        let buffer = data_render::create_style_uniform_buffer(&self.device, &highlight);
        let style_bg = data_render::create_style_bind_group(&self.device, &self.style_bgl, &buffer);
        // The violin's outline first, under the box's ring:
        // every segment, the vertex stage keeping the row's
        // (and the selected half's: a selected half rings
        // that half alone, not the box).
        if let Some((violin, segments)) = columns.violin.clone() {
            highlight.primitive_flags = match half {
                None | Some(ViolinSide::Both) => 0,
                Some(ViolinSide::Low) => data_render::VIOLIN_SIDE_LOW,
                Some(ViolinSide::High) => data_render::VIOLIN_SIDE_HIGH,
            };
            let buffer = data_render::create_style_uniform_buffer(&self.device, &highlight);
            selected_bars.push(ColumnBarSelectionLayer {
                pipeline: pipelines
                    .bar_violin_selection
                    .as_ref()
                    .ok_or(FiggyError::internal(
                        "prepare ensured violin selection pipeline",
                    ))?,
                transform_bg: &view.transform_bg,
                style_bg: data_render::create_style_bind_group(
                    &self.device,
                    &self.style_bgl,
                    &buffer,
                ),
                style_map_bg: None,
                pool_buffer: column_buffer,
                edges: columns.x,
                values: columns.y,
                instance: 0,
                instances: segments,
                category: Some(violin),
            });
        }
        if half.is_some_and(|half| half != ViolinSide::Both) {
            return Ok(());
        }
        selected_bars.push(ColumnBarSelectionLayer {
            pipeline: pipelines
                .bar_box_plot_selection
                .as_ref()
                .ok_or(FiggyError::internal(
                    "prepare ensured box plot selection pipeline",
                ))?,
            transform_bg: &view.transform_bg,
            style_bg,
            style_map_bg: None,
            pool_buffer: column_buffer,
            edges: columns.x,
            values: columns.y,
            instance: row,
            instances: 1,
            category: Some(category),
        });
        Ok(())
    }

    /// A box plot's raw point.
    pub(super) fn box_point_selection<'a>(
        &'a self,
        s: &SelectionInputs<'a, '_>,
        selection_cfg: &crate::config::DataSelectionsConfig,
        point_index: &usize,
        selected_bars: &mut Vec<ColumnBarSelectionLayer<'a>>,
    ) -> Result<()> {
        let SelectionInputs {
            view,
            series,
            pipelines,
            columns,
            rt,
            column_buffer,
            ..
        } = *s;
        let Some(points) = columns.box_points.clone() else {
            return Ok(());
        };
        let Ok(index) = u32::try_from(*point_index) else {
            return Ok(());
        };
        let boxes = rt.box_plot().ok_or(FiggyError::internal(
            "a box point selection has a box plot config",
        ))?;
        let mut highlight =
            PrimitiveStyle::from_box_plot(boxes.color, boxes.style, series.style.display_scale);
        highlight.color_premul =
            PrimitiveStyle::from_color(selection_cfg.highlight_color).color_premul;
        highlight.line_width_px = selection_cfg.outline_width_px;
        let buffer = data_render::create_style_uniform_buffer(&self.device, &highlight);
        selected_bars.push(ColumnBarSelectionLayer {
            pipeline: pipelines
                .bar_box_point_selection
                .as_ref()
                .ok_or(FiggyError::internal(
                    "prepare ensured box point selection pipeline",
                ))?,
            transform_bg: &view.transform_bg,
            style_bg: data_render::create_style_bind_group(&self.device, &self.style_bgl, &buffer),
            style_map_bg: None,
            pool_buffer: column_buffer,
            edges: columns.x,
            values: columns.y,
            instance: index,
            instances: 1,
            category: Some(points),
        });
        Ok(())
    }

    /// A range bar.
    pub(super) fn range_bar_selection<'a>(
        &'a self,
        s: &SelectionInputs<'a, '_>,
        selection_cfg: &crate::config::DataSelectionsConfig,
        bin_index: &usize,
        selected_bars: &mut Vec<ColumnBarSelectionLayer<'a>>,
    ) -> Result<()> {
        let SelectionInputs {
            view,
            series,
            pipelines,
            columns,
            rt,
            column_buffer,
            ..
        } = *s;
        let Some(category) = columns.category.clone() else {
            return Ok(());
        };
        let Ok(row) = u32::try_from(*bin_index) else {
            return Ok(());
        };
        let ranges = rt.range_bars().ok_or(FiggyError::internal(
            "a range bar selection has a range bar config",
        ))?;
        let mut highlight = PrimitiveStyle::from_category_bars(
            ranges.color,
            ranges.style,
            series.style.display_scale,
        );
        highlight.color_premul =
            PrimitiveStyle::from_color(selection_cfg.highlight_color).color_premul;
        highlight.line_width_px = selection_cfg.outline_width_px;
        highlight.dash_len = row;
        let buffer = data_render::create_style_uniform_buffer(&self.device, &highlight);
        let style_bg = data_render::create_style_bind_group(&self.device, &self.style_bgl, &buffer);
        selected_bars.push(ColumnBarSelectionLayer {
            pipeline: pipelines
                .bar_range_selection
                .as_ref()
                .ok_or(FiggyError::internal(
                    "prepare ensured range bar selection pipeline",
                ))?,
            transform_bg: &view.transform_bg,
            style_bg,
            style_map_bg: None,
            pool_buffer: column_buffer,
            edges: columns.x,
            values: columns.y,
            instance: row,
            instances: 1,
            category: Some(category),
        });
        Ok(())
    }

    /// A category bar.
    pub(super) fn category_bar_selection<'a>(
        &'a self,
        s: &SelectionInputs<'a, '_>,
        selection_cfg: &crate::config::DataSelectionsConfig,
        bin_index: &usize,
        selected_bars: &mut Vec<ColumnBarSelectionLayer<'a>>,
    ) -> Result<()> {
        let SelectionInputs {
            view,
            series,
            pipelines,
            columns,
            rt,
            column_buffer,
            ..
        } = *s;
        let Some(category) = columns.category.clone() else {
            return Ok(());
        };
        let Ok(row) = u32::try_from(*bin_index) else {
            return Ok(());
        };
        let bars = rt.category_bars().ok_or(FiggyError::internal(
            "a category bar selection has a category bar config",
        ))?;
        let mut highlight =
            PrimitiveStyle::from_category_bars(bars.color, bars.style, series.style.display_scale);
        highlight.color_premul =
            PrimitiveStyle::from_color(selection_cfg.highlight_color).color_premul;
        highlight.line_width_px = selection_cfg.outline_width_px;
        highlight.dash_len = row;
        let buffer = data_render::create_style_uniform_buffer(&self.device, &highlight);
        let style_bg = data_render::create_style_bind_group(&self.device, &self.style_bgl, &buffer);
        selected_bars.push(ColumnBarSelectionLayer {
            pipeline: pipelines
                .bar_category_selection
                .as_ref()
                .ok_or(FiggyError::internal(
                    "prepare ensured category bar selection pipeline",
                ))?,
            transform_bg: &view.transform_bg,
            style_bg,
            style_map_bg: None,
            pool_buffer: column_buffer,
            edges: columns.x,
            values: columns.y,
            instance: row,
            instances: 1,
            category: Some(category),
        });
        Ok(())
    }
}
