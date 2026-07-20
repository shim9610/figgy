use renderer::data_config::{
    DataLineStyleConfig, DataRenderType, DataScatterPointStyleConfig, DataScatterStyleConfig,
    SeriesConfig,
};
use renderer::data_render::{
    ScatterStyleMapMeta, ScatterStyleOverrideGpu, ScatterStyleSlotGpu, shape_id,
};
use renderer::gpu_pick::{GpuPickScatter, GpuPickScatterStyle, GpuPickSeriesDescriptor};
use renderer::{Color, data_render::ColumnId};

const MASK_COLOR: u32 = 1;
const MASK_RADIUS: u32 = 2;
const MASK_SHAPE: u32 = 4;

/// Owned registration metadata for one exact GPU-pick series.
///
/// The style arrays contain only the small declarative style table and sparse
/// overrides from [`SeriesConfig`]. They never contain point-column values.
pub(crate) struct OwnedGpuPickSeriesDescriptor {
    source_id: Option<String>,
    series_id: String,
    x_column: ColumnId,
    y_column: ColumnId,
    scatter: Option<OwnedGpuPickScatter>,
    line_width_px: Option<f32>,
}

struct OwnedGpuPickScatter {
    base_radius_px: f32,
    base_shape_id: u32,
    style_index_column: Option<ColumnId>,
    style_slots: Vec<ScatterStyleSlotGpu>,
    style_overrides: Vec<ScatterStyleOverrideGpu>,
    style_meta: Option<ScatterStyleMapMeta>,
}

impl OwnedGpuPickSeriesDescriptor {
    /// Derive the exact legacy picker metadata at raw display scale `1.0`.
    ///
    /// `precise` must be `Config::draw_style.is_precise()`. Per-point style
    /// mapping is intentionally absent for every non-precise draw style.
    pub(crate) fn from_series(series: &SeriesConfig, precise: bool) -> Self {
        let scatter = scatter_style(&series.render_type)
            .map(|scatter| OwnedGpuPickScatter::from_config(scatter, precise));
        let line_width_px = line_style(&series.render_type).map(|line| line.line_width);

        Self {
            source_id: series.source_id.clone(),
            series_id: series.series_id.clone(),
            x_column: series.x_column.clone(),
            y_column: series.y_column.clone(),
            scatter,
            line_width_px,
        }
    }

    pub(crate) fn references_column(&self, id: &str) -> bool {
        self.x_column == id
            || self.y_column == id
            || self
                .scatter
                .as_ref()
                .and_then(|scatter| scatter.style_index_column.as_deref())
                == Some(id)
    }

    /// Build a renderer descriptor whose style slices borrow this owner.
    pub(crate) fn descriptor(&self) -> GpuPickSeriesDescriptor<'_> {
        GpuPickSeriesDescriptor {
            source_id: self.source_id.clone(),
            series_id: self.series_id.clone(),
            x_column: self.x_column.clone(),
            y_column: self.y_column.clone(),
            scatter: self.scatter.as_ref().map(OwnedGpuPickScatter::descriptor),
            line_width_px: self.line_width_px,
        }
    }

    /// Keep the descriptor borrow scoped to one registration/rebind call.
    pub(crate) fn with_descriptor<R>(
        &self,
        use_descriptor: impl FnOnce(GpuPickSeriesDescriptor<'_>) -> R,
    ) -> R {
        use_descriptor(self.descriptor())
    }
}

impl OwnedGpuPickScatter {
    fn from_config(scatter: &DataScatterStyleConfig, precise: bool) -> Self {
        let base_radius_px = scatter.point_size;
        let base_shape_id = shape_id(&scatter.point_shape);

        if !precise {
            return Self {
                base_radius_px,
                base_shape_id,
                style_index_column: None,
                style_slots: Vec::new(),
                style_overrides: Vec::new(),
                style_meta: None,
            };
        }

        let style_slots: Vec<_> = scatter
            .point_style_table
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(scatter_style_slot_gpu)
            .collect();
        let style_overrides: Vec<_> = scatter
            .point_style_overrides
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter_map(|override_| {
                let point_index = u32::try_from(override_.index).ok()?;
                let slot = scatter_style_slot_gpu(&override_.style);
                Some(ScatterStyleOverrideGpu {
                    point_index,
                    _pad: [0; 3],
                    color_premul: slot.color_premul,
                    meta: slot.meta,
                })
            })
            .collect();
        let style_index_column = scatter.point_style_index_column.clone();
        let has_index = style_index_column.is_some();
        let style_meta = (has_index || !style_slots.is_empty() || !style_overrides.is_empty())
            .then_some(ScatterStyleMapMeta {
                style_count: style_slots.len().min(u32::MAX as usize) as u32,
                override_count: style_overrides.len().min(u32::MAX as usize) as u32,
                has_index: u32::from(has_index),
                _pad: 0,
            });

        Self {
            base_radius_px,
            base_shape_id,
            style_index_column,
            style_slots,
            style_overrides,
            style_meta,
        }
    }

    fn descriptor(&self) -> GpuPickScatter<'_> {
        GpuPickScatter {
            base_radius_px: self.base_radius_px,
            base_shape_id: self.base_shape_id,
            style_map: self.style_meta.map(|style_meta| GpuPickScatterStyle {
                style_index_column: self.style_index_column.clone(),
                style_slots: &self.style_slots,
                style_overrides: &self.style_overrides,
                style_meta,
            }),
        }
    }
}

fn scatter_style(render_type: &DataRenderType) -> Option<&DataScatterStyleConfig> {
    match render_type {
        DataRenderType::Scatter { scatter }
        | DataRenderType::ScatterLine { scatter, .. }
        | DataRenderType::ScatterErrorbarX { scatter, .. }
        | DataRenderType::ScatterErrorbarY { scatter, .. }
        | DataRenderType::ScatterErrorbarXY { scatter, .. }
        | DataRenderType::LineScatterErrorbarX { scatter, .. }
        | DataRenderType::LineScatterErrorbarY { scatter, .. }
        | DataRenderType::LineScatterErrorbarXY { scatter, .. } => Some(scatter),
        DataRenderType::Line { .. } => None,
    }
}

fn line_style(render_type: &DataRenderType) -> Option<&DataLineStyleConfig> {
    match render_type {
        DataRenderType::Line { line }
        | DataRenderType::ScatterLine { line, .. }
        | DataRenderType::LineScatterErrorbarX { line, .. }
        | DataRenderType::LineScatterErrorbarY { line, .. }
        | DataRenderType::LineScatterErrorbarXY { line, .. } => Some(line),
        DataRenderType::Scatter { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. } => None,
    }
}

fn scatter_style_slot_gpu(slot: &DataScatterPointStyleConfig) -> ScatterStyleSlotGpu {
    let mut mask = 0u32;
    let mut color_premul = [0.0; 4];
    let mut radius_px = 0.0;
    let mut mapped_shape_id = 0.0;

    if let Some(color) = slot.point_color {
        mask |= MASK_COLOR;
        color_premul = premul_rgba(color);
    }
    if let Some(radius) = slot.point_size {
        mask |= MASK_RADIUS;
        radius_px = radius;
    }
    if let Some(shape) = slot.point_shape.as_ref() {
        mask |= MASK_SHAPE;
        mapped_shape_id = shape_id(shape) as f32;
    }

    ScatterStyleSlotGpu {
        color_premul,
        meta: [radius_px, mapped_shape_id, mask as f32, 0.0],
    }
}

fn premul_rgba(color: Color) -> [f32; 4] {
    let alpha = color.a.clamp(0.0, 1.0);
    [color.r * alpha, color.g * alpha, color.b * alpha, alpha]
}

#[cfg(test)]
mod tests {
    use super::{MASK_COLOR, MASK_RADIUS, MASK_SHAPE, OwnedGpuPickSeriesDescriptor};
    use renderer::Color;
    use renderer::data_config::{
        DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType, DataScatterPointStyleConfig,
        DataScatterPointStyleOverride, DataScatterStyleConfig, ErrorRef, ScatterShape,
        SeriesConfig,
    };
    use renderer::line::LineStylePreset;

    fn line(width: f32) -> DataLineStyleConfig {
        DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::BLACK,
            line_width: width,
        }
    }

    fn scatter() -> DataScatterStyleConfig {
        DataScatterStyleConfig {
            point_color: Color::WHITE,
            point_shape: ScatterShape::StarFilled,
            point_size: 6.25,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
        }
    }

    fn error_style() -> DataErrorBarStyleConfig {
        DataErrorBarStyleConfig {
            error_bar_color: Color::BLACK,
            error_bar_width: 1.0,
            error_bar_cap_size: 2.0,
            cap_width: 1.0,
            error_bar_style_table: None,
            error_bar_style_index_column: None,
            error_bar_style_overrides: None,
        }
    }

    fn error_ref(name: &str) -> ErrorRef {
        ErrorRef::Symmetric {
            column: name.into(),
        }
    }

    fn series(render_type: DataRenderType) -> SeriesConfig {
        SeriesConfig {
            series_id: "series".into(),
            source_id: Some("source".into()),
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type,
        }
    }

    #[test]
    fn every_render_variant_preserves_scatter_and_line_presence() {
        let cases = [
            (DataRenderType::Scatter { scatter: scatter() }, true, None),
            (DataRenderType::Line { line: line(1.0) }, false, Some(1.0)),
            (
                DataRenderType::ScatterLine {
                    scatter: scatter(),
                    line: line(2.0),
                },
                true,
                Some(2.0),
            ),
            (
                DataRenderType::ScatterErrorbarX {
                    scatter: scatter(),
                    err_x: error_ref("ex"),
                    err_style: error_style(),
                },
                true,
                None,
            ),
            (
                DataRenderType::ScatterErrorbarY {
                    scatter: scatter(),
                    err_y: error_ref("ey"),
                    err_style: error_style(),
                },
                true,
                None,
            ),
            (
                DataRenderType::ScatterErrorbarXY {
                    scatter: scatter(),
                    err_x: error_ref("ex"),
                    err_y: error_ref("ey"),
                    err_style: error_style(),
                },
                true,
                None,
            ),
            (
                DataRenderType::LineScatterErrorbarX {
                    scatter: scatter(),
                    line: line(3.0),
                    err_x: error_ref("ex"),
                    err_style: error_style(),
                },
                true,
                Some(3.0),
            ),
            (
                DataRenderType::LineScatterErrorbarY {
                    scatter: scatter(),
                    line: line(4.0),
                    err_y: error_ref("ey"),
                    err_style: error_style(),
                },
                true,
                Some(4.0),
            ),
            (
                DataRenderType::LineScatterErrorbarXY {
                    scatter: scatter(),
                    line: line(5.0),
                    err_x: error_ref("ex"),
                    err_y: error_ref("ey"),
                    err_style: error_style(),
                },
                true,
                Some(5.0),
            ),
        ];

        for (render_type, has_scatter, line_width) in cases {
            let owned = OwnedGpuPickSeriesDescriptor::from_series(&series(render_type), true);
            owned.with_descriptor(|descriptor| {
                assert_eq!(descriptor.scatter.is_some(), has_scatter);
                assert_eq!(descriptor.line_width_px, line_width);
            });
        }
    }

    #[test]
    fn non_precise_mode_uses_raw_base_style_and_disables_mapping() {
        let mut scatter = scatter();
        scatter.point_size = -3.5;
        scatter.point_style_index_column = Some("style-index".into());
        scatter.point_style_table = Some(vec![DataScatterPointStyleConfig {
            point_size: Some(99.0),
            ..Default::default()
        }]);
        scatter.point_style_overrides = Some(vec![DataScatterPointStyleOverride {
            index: 0,
            style: DataScatterPointStyleConfig {
                point_shape: Some(ScatterShape::Triangle),
                ..Default::default()
            },
        }]);
        let cfg = series(DataRenderType::ScatterLine {
            scatter,
            line: line(-7.25),
        });

        let owned = OwnedGpuPickSeriesDescriptor::from_series(&cfg, false);
        assert!(owned.references_column("x"));
        assert!(owned.references_column("y"));
        assert!(!owned.references_column("style-index"));
        let descriptor = owned.descriptor();
        let scatter = descriptor.scatter.expect("scatter descriptor");

        assert_eq!(scatter.base_radius_px, -3.5);
        assert_eq!(scatter.base_shape_id, 25);
        assert!(scatter.style_map.is_none());
        assert_eq!(descriptor.line_width_px, Some(-7.25));
    }

    #[test]
    fn precise_mapping_preserves_masks_values_and_override_order() {
        let mut scatter = scatter();
        scatter.point_style_index_column = Some("style-index".into());
        scatter.point_style_table = Some(vec![
            DataScatterPointStyleConfig {
                point_color: Some(Color::new(0.8, 0.4, 0.2, 0.5)),
                point_shape: Some(ScatterShape::DiamondFilled),
                point_size: Some(9.5),
            },
            DataScatterPointStyleConfig {
                point_size: Some(-2.25),
                ..Default::default()
            },
        ]);
        let mut overrides = vec![
            DataScatterPointStyleOverride {
                index: 9,
                style: DataScatterPointStyleConfig {
                    point_size: Some(11.0),
                    ..Default::default()
                },
            },
            DataScatterPointStyleOverride {
                index: 2,
                style: DataScatterPointStyleConfig {
                    point_shape: Some(ScatterShape::Square),
                    ..Default::default()
                },
            },
            DataScatterPointStyleOverride {
                index: 9,
                style: DataScatterPointStyleConfig {
                    point_shape: Some(ScatterShape::TriangleDownFilled),
                    ..Default::default()
                },
            },
        ];
        if let Ok(too_large) = usize::try_from(u64::from(u32::MAX) + 1) {
            overrides.insert(
                1,
                DataScatterPointStyleOverride {
                    index: too_large,
                    style: DataScatterPointStyleConfig {
                        point_size: Some(999.0),
                        ..Default::default()
                    },
                },
            );
        }
        scatter.point_style_overrides = Some(overrides);
        let cfg = series(DataRenderType::Scatter { scatter });

        let owned = OwnedGpuPickSeriesDescriptor::from_series(&cfg, true);
        assert!(owned.references_column("style-index"));
        let descriptor = owned.descriptor();
        assert_eq!(descriptor.source_id.as_deref(), Some("source"));
        assert_eq!(descriptor.series_id, "series");
        assert_eq!(descriptor.x_column, "x");
        assert_eq!(descriptor.y_column, "y");
        let scatter = descriptor.scatter.expect("scatter descriptor");
        assert_eq!(scatter.base_radius_px, 6.25);
        assert_eq!(scatter.base_shape_id, 25);
        let map = scatter.style_map.expect("precise style map");

        assert_eq!(map.style_index_column.as_deref(), Some("style-index"));
        assert_eq!(map.style_meta.style_count, 2);
        assert_eq!(map.style_meta.override_count, 3);
        assert_eq!(map.style_meta.has_index, 1);
        assert_eq!(map.style_meta._pad, 0);
        assert_eq!(map.style_slots[0].color_premul, [0.4, 0.2, 0.1, 0.5]);
        assert_eq!(
            map.style_slots[0].meta,
            [
                9.5,
                8.0,
                (MASK_COLOR | MASK_RADIUS | MASK_SHAPE) as f32,
                0.0
            ]
        );
        assert_eq!(
            map.style_slots[1].meta,
            [-2.25, 0.0, MASK_RADIUS as f32, 0.0]
        );
        assert_eq!(
            map.style_overrides
                .iter()
                .map(|override_| override_.point_index)
                .collect::<Vec<_>>(),
            vec![9, 2, 9]
        );
        assert_eq!(
            map.style_overrides[0].meta,
            [11.0, 0.0, MASK_RADIUS as f32, 0.0]
        );
        assert_eq!(
            map.style_overrides[1].meta,
            [0.0, 1.0, MASK_SHAPE as f32, 0.0]
        );
        assert_eq!(
            map.style_overrides[2].meta,
            [0.0, 17.0, MASK_SHAPE as f32, 0.0]
        );
    }

    #[test]
    fn precise_empty_mapping_stays_absent() {
        let cfg = series(DataRenderType::Scatter { scatter: scatter() });
        let owned = OwnedGpuPickSeriesDescriptor::from_series(&cfg, true);

        assert!(
            owned
                .descriptor()
                .scatter
                .expect("scatter descriptor")
                .style_map
                .is_none()
        );
    }
}
