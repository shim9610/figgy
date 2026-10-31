//! Selected-cell neighbours preserve the resident shader's lattice arithmetic.
//! Only axis addressing is rebased; Config keeps the original selected indices.
use super::super::super::selection_prepare::SelectionLayers;
use super::*;

// Six padded field bindings plus the typed-selection uniform.
pub(super) const OVERHEAD: u64 = 8 + 4 + 16 + 16 + 8 + 64 + 48;

#[derive(Clone, Copy)]
pub(super) struct Axis {
    pub offset: u64,
    pub len: u64,
    local: u32,
}

#[derive(Clone, Copy)]
pub(super) struct Cell {
    pub axes: [Axis; 2],
    centers: bool,
    interpolated: bool,
}

impl Cell {
    pub fn resolve(
        series: &SeriesConfig,
        index: [usize; 2],
        mut len: impl FnMut(&str) -> Option<u64>,
    ) -> Option<Self> {
        let (extent, x_len, y_len) = field_runtime::extent(series, &mut len).ok()?;
        let matrix = extract_matrix(&series.render_type)?;
        let fill = extract_field_fill(&series.render_type)?;
        let centers = matches!(matrix.grid_layout, crate::data_config::GridLayout::Centers);
        let interpolated = matches!(fill.shading, crate::data_config::Shading::Interpolated);
        let counts = if matches!(
            matrix.orientation,
            crate::data_config::MatrixOrientation::ColumnsAreY
        ) {
            [extent.rows, extent.cols]
        } else {
            [extent.cols, extent.rows]
        };
        let lengths = [u64::from(x_len), u64::from(y_len)];
        let mut axes = [Axis {
            offset: 0,
            len: 0,
            local: 0,
        }; 2];
        for axis in 0..2 {
            if index[axis] >= counts[axis].saturating_sub(usize::from(interpolated)) {
                return None;
            }
            let start = if centers && !interpolated {
                index[axis].saturating_sub(1)
            } else {
                index[axis]
            } as u64;
            let end = (index[axis] as u64)
                .checked_add(if !centers && interpolated { 3 } else { 2 })?
                .min(lengths[axis]);
            axes[axis] = Axis {
                offset: start,
                len: end.checked_sub(start)?,
                local: u32::try_from(index[axis] as u64 - start).ok()?,
            };
        }
        Some(Self {
            axes,
            centers,
            interpolated,
        })
    }
}

pub(super) fn packet(
    context: &PrepareContext<'_>,
    view: &ChartView,
    config: &Config,
    pipelines: &TargetPipelines,
    chunk: &RecordedChunk,
    ranges: &[ColumnRange],
    cell: Cell,
) -> StreamResult<StreamSelectionPacket> {
    let x = chunk.column_handle(ranges[0])?;
    let y = chunk.column_handle(*ranges.get(1).unwrap_or(&ranges[0]))?;
    let params = data_render::FieldParamsGpu {
        x_base: u32::try_from(x.offset / 4).map_err(|_| StreamError::TooLarge)?,
        y_base: u32::try_from(y.offset / 4).map_err(|_| StreamError::TooLarge)?,
        x_len: cell.axes[0].len as u32,
        y_len: cell.axes[1].len as u32,
        cols: cell.axes[0].len as u32 - u32::from(!cell.centers),
        rows: cell.axes[1].len as u32 - u32::from(!cell.centers),
        flags: if cell.centers {
            data_render::FIELD_FLAG_CENTERS
        } else {
            0
        } | if cell.interpolated {
            data_render::FIELD_FLAG_INTERPOLATED
        } else {
            0
        },
        level_count: 0,
        stop_count: 0,
        opacity: 1.0,
        line_width_px: 0.0,
        level_color_count: 0,
        z_min: [0.0; 2],
        z_max: [0.0; 2],
    };
    let (field_bg, charge) = data_render::create_field_data_bind_group(
        context.device,
        context.gpu_ledger,
        context.field_bgl,
        &chunk.work,
        data_render::FieldTables {
            grid: &[data_render::GridColumnGpu { base: 0, len: 0 }],
            levels: &[0.0],
            stops: &[[0.0; 4]],
            level_colors: &[[0.0; 4]],
            lookup_metadata: &[data_render::ContourLookupMetadataGpu {
                finite_count: 0,
                negative_infinity_count: 0,
            }],
            params: &params,
        },
    );
    let selected = config.picked_data.as_ref().ok_or(StreamError::WrongState)?;
    let mut selection = data_render::DataSelectionGpu::from_color(selected.highlight_color);
    selection.metrics[0] = selected.outline_width_px;
    selection.indices = [
        data_render::DATA_SELECTION_KIND_MATRIX_CELL,
        0,
        cell.axes[0].local,
        cell.axes[1].local,
    ];
    let (selection_bg, selection_charge) = data_render::create_data_selection_bind_group(
        context.gpu_ledger,
        context.device,
        context.data_selection_bgl,
        &selection,
    );
    let layers = SelectionLayers {
        picked: Vec::new(),
        selected_bars: Vec::new(),
        selected_fields: vec![ColumnFieldSelectionLayer {
            pipeline: pipelines
                .field_selection
                .as_ref()
                .ok_or(StreamError::WrongState)?,
            transform_bg: &view.transform_bg,
            selection_bg,
            selection_charge,
            field_bg,
            charge,
            drawable: true,
        }],
    };
    let mut packet = PreparedSeries::from_layers(layers.into_series_layers(), None, None);
    packet._column_charge = Some(chunk.work.shared_charge());
    packet._stream_transform_charge = Some(view.transform_buffer.shared_charge());
    Ok(StreamSelectionPacket {
        packet,
        _map_charge: None,
    })
}
