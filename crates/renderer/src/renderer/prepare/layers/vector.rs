//! Vector layers: arrows and streamline arrowheads.

use super::*;

impl PrepareContext<'_> {
    /// A series' arrows, or a streamline's arrowheads on its arc slots.
    pub(super) fn vector_layer<'a>(
        &'a self,
        s: &SeriesInputs<'a, '_>,
    ) -> Result<Option<data_render::vector::ColumnVectorLayer<'a>>> {
        let SeriesInputs {
            view,
            pipelines,
            products,
            phase,
            idx,
            cfg,
            rt,
            primitives,
            ..
        } = *s;
        let camera_binding = |mapped: bool| self.camera_binding(s, mapped);
        // Arrows: the group-2 state was resolved in `ensure_vector_scratches`
        // and is reused whole while nothing it depends on changes.
        let vector = if primitives.vector && phase.is_none() {
            let scratch =
                self.vector_cache
                    .get(cfg.series_id.as_str())
                    .ok_or(FiggyError::internal(
                        "prepare ensured this vector's scratch",
                    ))?;
            let pipeline = match scratch.input {
                data_render::vector::VectorInput::Points => pipelines.vector_points.as_ref(),
                data_render::vector::VectorInput::Grid => pipelines.vector_grid.as_ref(),
                data_render::vector::VectorInput::Path => pipelines.vector_path.as_ref(),
                data_render::vector::VectorInput::Points3d => {
                    pipelines.view3d[View3dPipeline::Vector as usize].as_ref()
                }
            }
            .ok_or(FiggyError::internal("prepare ensured vector pipeline"))?;
            // A 3D chart's arrows (#211) bind the panel's camera.
            let view3d = (scratch.input == data_render::vector::VectorInput::Points3d)
                .then(|| camera_binding(false))
                .transpose()?;
            Some(data_render::vector::ColumnVectorLayer {
                pipeline,
                transform_bg: &view.transform_bg,
                style_bg: scratch.style_bg.clone(),
                vector_bg: scratch.vector_bg.clone(),
                charge: Arc::clone(&scratch.charge),
                count: scratch.count,
                indirect: None,
                view3d,
            })
        } else if primitives.line
            && phase.is_none()
            && rt
                .streamlines()
                .is_some_and(|streamlines| streamlines.arrows.is_some())
        {
            // Streamline arrowheads: the vector glyph on the arc slots of
            // the line's arc scan, drawn indirect in the vector slot,
            // under the stroke (#51, D5). A series too short to scan has
            // no slots and draws no heads.
            let scratch =
                self.vector_cache
                    .get(cfg.series_id.as_str())
                    .ok_or(FiggyError::internal(
                        "prepare ensured this streamline's arrowhead scratch",
                    ))?;
            let pipeline = pipelines.vector_path.as_ref().ok_or(FiggyError::internal(
                "prepare ensured the path vector pipeline",
            ))?;
            products
                .arcs
                .get(idx)
                .and_then(|series| series.arc.as_ref())
                .and_then(|arc| arc.star.as_ref())
                .map(|slots| data_render::vector::ColumnVectorLayer {
                    pipeline,
                    transform_bg: &view.transform_bg,
                    style_bg: scratch.style_bg.clone(),
                    vector_bg: scratch.vector_bg.clone(),
                    charge: Arc::clone(&scratch.charge),
                    count: scratch.count,
                    indirect: Some((&slots.indirect, &slots.vs_bg)),
                    view3d: None,
                })
        } else {
            None
        };
        Ok(vector)
    }
}
