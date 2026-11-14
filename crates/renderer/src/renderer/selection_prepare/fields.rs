//! Field, vector cell, contour and mesh selection highlights.

use super::*;

impl PrepareContext<'_> {
    /// A field's cell outline.
    pub(super) fn field_cell_selection<'a>(
        &'a self,
        s: &SelectionInputs<'a, '_>,
        selection_cfg: &crate::config::DataSelectionsConfig,
        x_index: &usize,
        y_index: &usize,
        selected_fields: &mut Vec<ColumnFieldSelectionLayer<'a>>,
    ) -> Result<()> {
        let SelectionInputs {
            view,
            pipelines,
            cfg,
            ..
        } = *s;
        let (Some(x_index), Some(y_index)) =
            (u32::try_from(*x_index).ok(), u32::try_from(*y_index).ok())
        else {
            return Ok(());
        };
        let scratch = self
            .field_cache
            .get(cfg.series_id.as_str())
            .ok_or(FiggyError::internal(
                "prepare ensured selected field scratch",
            ))?;
        let mut selection =
            data_render::DataSelectionGpu::from_color(selection_cfg.highlight_color);
        selection.metrics[0] = selection_cfg.outline_width_px;
        selection.indices = [
            data_render::DATA_SELECTION_KIND_MATRIX_CELL,
            0,
            x_index,
            y_index,
        ];
        let (selection_bg, selection_charge) = data_render::create_data_selection_bind_group(
            &self.gpu_ledger,
            &self.device,
            &self.data_selection_bgl,
            &selection,
        )?;
        selected_fields.push(ColumnFieldSelectionLayer {
            pipeline: pipelines
                .field_selection
                .as_ref()
                .ok_or(FiggyError::internal(
                    "prepare ensured field selection pipeline",
                ))?,
            transform_bg: &view.transform_bg,
            selection_bg,
            selection_charge,
            field_bg: scratch.field_bg.clone(),
            charge: Arc::clone(&scratch.charge),
            drawable: scratch.drawable,
            vertices: data_render::FIELD_VERTICES,
            instances: 0..1,
        });
        Ok(())
    }

    /// A grid vector's cell outline.
    pub(super) fn vector_cell_selection<'a>(
        &'a self,
        s: &SelectionInputs<'a, '_>,
        selection_cfg: &crate::config::DataSelectionsConfig,
        x_index: &usize,
        y_index: &usize,
        selected_fields: &mut Vec<ColumnFieldSelectionLayer<'a>>,
    ) -> Result<()> {
        let SelectionInputs {
            view,
            pipelines,
            cfg,
            ..
        } = *s;
        let (Some(x_index), Some(y_index)) =
            (u32::try_from(*x_index).ok(), u32::try_from(*y_index).ok())
        else {
            return Ok(());
        };
        let scratch = self
            .vector_cache
            .get(cfg.series_id.as_str())
            .ok_or(FiggyError::internal(
                "prepare ensured selected vector scratch",
            ))?;
        let Some((field_bg, charge)) = scratch.field.as_ref() else {
            return Ok(());
        };
        let mut selection =
            data_render::DataSelectionGpu::from_color(selection_cfg.highlight_color);
        selection.metrics[0] = selection_cfg.outline_width_px;
        selection.indices = [
            data_render::DATA_SELECTION_KIND_MATRIX_CELL,
            0,
            x_index,
            y_index,
        ];
        let (selection_bg, selection_charge) = data_render::create_data_selection_bind_group(
            &self.gpu_ledger,
            &self.device,
            &self.data_selection_bgl,
            &selection,
        )?;
        selected_fields.push(ColumnFieldSelectionLayer {
            pipeline: pipelines
                .field_selection
                .as_ref()
                .ok_or(FiggyError::internal(
                    "prepare ensured field selection pipeline",
                ))?,
            transform_bg: &view.transform_bg,
            selection_bg,
            selection_charge,
            field_bg: field_bg.clone(),
            charge: Arc::clone(charge),
            drawable: scratch.count != 0,
            vertices: data_render::FIELD_VERTICES,
            instances: 0..1,
        });
        Ok(())
    }

    /// A contour level's isoline highlight.
    pub(super) fn contour_level_selection<'a>(
        &'a self,
        s: &SelectionInputs<'a, '_>,
        selection_cfg: &crate::config::DataSelectionsConfig,
        level_index: &usize,
        x_index: &usize,
        y_index: &usize,
        selected_fields: &mut Vec<ColumnFieldSelectionLayer<'a>>,
    ) -> Result<()> {
        let SelectionInputs {
            view,
            pipelines,
            contour_scratch,
            rt,
            ..
        } = *s;
        let (Some(level_index), Some(x_index), Some(y_index)) = (
            u32::try_from(*level_index).ok(),
            u32::try_from(*x_index).ok(),
            u32::try_from(*y_index).ok(),
        ) else {
            return Ok(());
        };
        if usize::try_from(level_index).ok().is_none_or(|index| {
            extract_contour(rt).is_none_or(|contour| index >= contour.levels.len())
        }) {
            return Ok(());
        }
        let scratch = contour_scratch.ok_or(FiggyError::internal(
            "prepare captured selected contour scratch",
        ))?;
        let mut selection =
            data_render::DataSelectionGpu::from_color(selection_cfg.highlight_color);
        selection.metrics[1] = selection_cfg.contour_width_extra_px;
        selection.indices = [
            data_render::DATA_SELECTION_KIND_CONTOUR_LEVEL,
            level_index,
            x_index,
            y_index,
        ];
        let (selection_bg, selection_charge) = data_render::create_data_selection_bind_group(
            &self.gpu_ledger,
            &self.device,
            &self.data_selection_bgl,
            &selection,
        )?;
        // A labelled contour's highlight leaves its label
        // gaps; the draw binds the label group with it.
        let pipeline = if scratch.label.is_some() {
            pipelines
                .field_selection_labelled
                .as_ref()
                .ok_or(FiggyError::internal(
                    "prepare ensured labelled contour selection pipeline",
                ))?
        } else {
            pipelines
                .field_selection
                .as_ref()
                .ok_or(FiggyError::internal(
                    "prepare ensured contour selection pipeline",
                ))?
        };
        selected_fields.push(ColumnFieldSelectionLayer {
            pipeline,
            transform_bg: &view.transform_bg,
            selection_bg,
            selection_charge,
            field_bg: scratch.field_bg.clone(),
            charge: Arc::clone(&scratch.charge),
            drawable: scratch.drawable,
            vertices: data_render::FIELD_VERTICES,
            instances: 0..1,
        });
        Ok(())
    }

    /// A triangle mesh's triangle outline.
    pub(super) fn mesh_triangle_selection<'a>(
        &'a self,
        s: &SelectionInputs<'a, '_>,
        selection_cfg: &crate::config::DataSelectionsConfig,
        triangle_index: &usize,
        selected_fields: &mut Vec<ColumnFieldSelectionLayer<'a>>,
    ) -> Result<()> {
        let SelectionInputs {
            view,
            pipelines,
            cfg,
            ..
        } = *s;
        let Ok(triangle) = u32::try_from(*triangle_index) else {
            return Ok(());
        };
        let scratch = self
            .field_cache
            .get(cfg.series_id.as_str())
            .ok_or(FiggyError::internal(
                "prepare ensured selected mesh scratch",
            ))?;
        if triangle >= scratch.signature.params.rows {
            return Ok(());
        }
        let mut selection =
            data_render::DataSelectionGpu::from_color(selection_cfg.highlight_color);
        selection.metrics[0] = selection_cfg.outline_width_px;
        selection.indices = [
            data_render::DATA_SELECTION_KIND_MESH_TRIANGLE,
            triangle,
            0,
            0,
        ];
        let (selection_bg, selection_charge) = data_render::create_data_selection_bind_group(
            &self.gpu_ledger,
            &self.device,
            &self.data_selection_bgl,
            &selection,
        )?;
        selected_fields.push(ColumnFieldSelectionLayer {
            pipeline: pipelines.tri_mesh[data_render::tri_mesh::TriMeshPass::Selection as usize]
                .as_ref()
                .ok_or(FiggyError::internal(
                    "prepare ensured mesh selection pipeline",
                ))?,
            transform_bg: &view.transform_bg,
            selection_bg,
            selection_charge,
            field_bg: scratch.field_bg.clone(),
            charge: Arc::clone(&scratch.charge),
            drawable: scratch.drawable,
            vertices: data_render::tri_mesh::TRI_MESH_VERTICES,
            instances: triangle..triangle + 1,
        });
        Ok(())
    }

    /// A triangle mesh's contour level.
    pub(super) fn mesh_contour_selection<'a>(
        &'a self,
        s: &SelectionInputs<'a, '_>,
        selection_cfg: &crate::config::DataSelectionsConfig,
        level_index: &usize,
        selected_fields: &mut Vec<ColumnFieldSelectionLayer<'a>>,
    ) -> Result<()> {
        let SelectionInputs {
            view,
            pipelines,
            cfg,
            ..
        } = *s;
        let Ok(level_index) = u32::try_from(*level_index) else {
            return Ok(());
        };
        if cfg
            .tri_mesh()
            .and_then(|mesh| mesh.contour.as_ref())
            .is_none_or(|contour| level_index as usize >= contour.levels.len())
        {
            return Ok(());
        }
        let scratch = self
            .field_cache
            .get(cfg.series_id.as_str())
            .ok_or(FiggyError::internal(
                "prepare ensured selected mesh scratch",
            ))?;
        let mut selection =
            data_render::DataSelectionGpu::from_color(selection_cfg.highlight_color);
        selection.metrics[1] = selection_cfg.contour_width_extra_px;
        selection.indices = [
            data_render::DATA_SELECTION_KIND_CONTOUR_LEVEL,
            level_index,
            0,
            0,
        ];
        let (selection_bg, selection_charge) = data_render::create_data_selection_bind_group(
            &self.gpu_ledger,
            &self.device,
            &self.data_selection_bgl,
            &selection,
        )?;
        selected_fields.push(ColumnFieldSelectionLayer {
            pipeline: pipelines.tri_mesh[data_render::tri_mesh::TriMeshPass::Selection as usize]
                .as_ref()
                .ok_or(FiggyError::internal(
                    "prepare ensured mesh selection pipeline",
                ))?,
            transform_bg: &view.transform_bg,
            selection_bg,
            selection_charge,
            field_bg: scratch.field_bg.clone(),
            charge: Arc::clone(&scratch.charge),
            drawable: scratch.drawable,
            vertices: data_render::tri_mesh::TRI_MESH_VERTICES,
            instances: 0..scratch.signature.params.rows,
        });
        Ok(())
    }
}
