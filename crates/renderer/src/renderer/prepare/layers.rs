//! Series layer building: build_series_layers and its per-kind arms.

mod bars;
mod extras;
mod field;
mod lines;
mod markers;
mod surface;
mod vector;

use super::*;

/// The region params a ternary series' draw and its pick scan over: every
/// column's f32 lane and the rows all three columns hold.
fn ternary_region_params(
    series_id: &str,
    color_premul: [f32; 4],
    x: ColumnHandle,
    y: ColumnHandle,
    c: ColumnHandle,
) -> Result<data_render::ternary_region::TernaryRegionParamsGpu> {
    let invalid = |reason: crate::RuleKind| FiggyError::InvalidSeriesConfig {
        series_id: series_id.to_owned(),
        kind: reason,
    };
    let lane = |handle: ColumnHandle| -> Result<u32> {
        if !handle.offset.is_multiple_of(4) {
            return Err(invalid(crate::RuleKind::RegionColumnAlignment));
        }
        u32::try_from(handle.offset / 4)
            .map_err(|_| invalid(crate::RuleKind::RegionColumnLaneOverflow))
    };
    let count = x.len_values.min(y.len_values).min(c.len_values);
    Ok(data_render::ternary_region::TernaryRegionParamsGpu {
        color_premul,
        x_base: lane(x)?,
        y_base: lane(y)?,
        c_base: lane(c)?,
        count: u32::try_from(count).map_err(|_| invalid(crate::RuleKind::RegionRowsOverflow))?,
    })
}

fn ring_scan_too_large(series_id: &str) -> FiggyError {
    FiggyError::InvalidSeriesConfig {
        series_id: series_id.to_owned(),
        kind: crate::RuleKind::RegionDispatchGrid,
    }
}

/// The style set resolved into per-primitive picks once. `stars` is the
/// constellation's arc-driven indirect star pass over the same polyline.
#[derive(Clone, Copy)]
struct LinePick<'p> {
    pipeline: &'p wgpu::RenderPipeline,
    verts: u32,
    texture_bg: Option<&'p wgpu::BindGroup>,
}
#[derive(Clone, Copy)]
struct StarsPick<'p> {
    pipeline: &'p wgpu::RenderPipeline,
    texture_bg: &'p wgpu::BindGroup,
}
#[derive(Clone, Copy)]
struct ScatterPick<'p> {
    pipeline: &'p wgpu::RenderPipeline,
    texture_bg: Option<&'p wgpu::BindGroup>,
}

/// The picks every series of one panel draws with: the line, star, scatter,
/// error-bar and stem pipelines of the panel's draw style.
#[derive(Clone, Copy)]
struct StylePicks<'p> {
    line: Option<LinePick<'p>>,
    stars: Option<StarsPick<'p>>,
    scatter: Option<ScatterPick<'p>>,
    errorbar: Option<&'p wgpu::RenderPipeline>,
    stem: Option<&'p wgpu::RenderPipeline>,
}

/// One series' inputs to the family builders: the panel's view, config,
/// pipelines and products, and the series' own columns and primitives.
#[derive(Clone, Copy)]
struct SeriesInputs<'a, 's> {
    view: &'a ChartView,
    chart_config: &'a Config,
    series_specs: &'s [Series<'a>],
    pipelines: &'a TargetPipelines,
    products: PreparedSeriesProducts<'a>,
    styled: Option<&'a StyleSet>,
    column_buffer: &'a wgpu::Buffer,
    phase: Option<streaming_runtime::StreamDrawPhase>,
    picks: StylePicks<'a>,
    camera_3d: Option<&'s data_render::surface_mesh::View3dGpu>,
    idx: usize,
    series: Series<'a>,
    cfg: &'a SeriesConfig,
    rt: &'a DataRenderType,
    constellation_only: bool,
    primitives: EffectiveSeriesPrimitives,
    x_h: ColumnHandle,
    y_h: ColumnHandle,
    z_3d: Option<ColumnHandle>,
}

/// The field family's layers of one series.
struct FieldLayers<'a> {
    field: Option<data_render::ColumnFieldLayer<'a>>,
    mesh: Option<data_render::tri_mesh::ColumnTriMeshLayer<'a>>,
    mesh_contour: Option<data_render::tri_mesh::ColumnTriMeshLayer<'a>>,
    surface: Option<data_render::surface_mesh::ColumnSurfaceLayer<'a>>,
    selected_surface: Option<data_render::surface_mesh::ColumnSurfaceLayer<'a>>,
    floor: Option<data_render::surface_mesh::ColumnSurfaceLayer<'a>>,
    contour: Option<data_render::ColumnContourLayer<'a>>,
}

/// A box plot series' violins and raw points.
type BoxExtraLayers<'a> = (
    Option<data_render::ColumnCategoryBarLayer<'a>>,
    Option<data_render::ColumnCategoryBarLayer<'a>>,
);

impl PrepareContext<'_> {
    /// A region series' ring starts for the pick (#49): the scan its draw
    /// runs, over the same rows. `None` for any other series, or when its
    /// columns are not resident; a scan the draw would fail fails the pick
    /// with the same error.
    pub(in crate::renderer) fn region_ring_starts(
        &mut self,
        config: &Config,
        series: &SeriesConfig,
    ) -> Result<Option<wgpu::Buffer>> {
        let (Some(_), Some(_), Some(c)) = (
            series.render_type.ternary_region(),
            config.ternary(),
            series.render_type.ternary_c_column(),
        ) else {
            return Ok(None);
        };
        let (Some(x), Some(y), Some(c)) = (
            self.pool.handle_for(&series.x_column),
            self.pool.handle_for(&series.y_column),
            self.pool.handle_for(c),
        ) else {
            return Ok(None);
        };
        let params = ternary_region_params(&series.series_id, [0.0; 4], x, y, c)?;
        let scan = self.pipelines.ensure_ternary_region_scan(self.device);
        let (buffer, _charge) = data_render::ternary_region::scan_ring_starts(
            self.device,
            self.queue,
            self.gpu_ledger,
            &scan,
            self.pool.buffer(),
            &params,
            self.caps.max_compute_workgroups_per_dimension,
        )?
        .ok_or_else(|| ring_scan_too_large(&series.series_id))?;
        Ok(Some(buffer))
    }

    pub(in crate::renderer) fn build_series_layers<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &'a Config,
        series_specs: &[Series<'a>],
        pipelines: &'a TargetPipelines,
        products: PreparedSeriesProducts<'a>,
        styled: Option<&'a StyleSet>,
    ) -> Result<(Vec<data_render::SeriesLayers<'a>>, HashMap<ColumnId, u64>)> {
        let pool = &self.pool;
        let mut column_sources = HashMap::new();
        let mut lookup = |id: &str| -> Result<ColumnHandle> {
            let handle = pool
                .handle_for(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })?;
            let allocation_epoch = pool
                .allocation_epoch(id)
                .ok_or_else(|| FiggyError::UnknownColumn { id: id.into() })?;
            column_sources.entry(id.into()).or_insert(allocation_epoch);
            Ok(handle)
        };

        let layers = self.build_series_layers_with_columns(
            view,
            chart_config,
            series_specs,
            pipelines,
            products,
            styled,
            pool.buffer(),
            &mut lookup,
            None,
        )?;
        Ok((layers, column_sources))
    }

    /// Column locations and their lifetime authority are resolved by the caller;
    /// geometry and style interpretation remain shared with resident rendering.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::renderer) fn build_series_layers_with_columns<'a>(
        &'a self,
        view: &'a ChartView,
        chart_config: &'a Config,
        series_specs: &[Series<'a>],
        pipelines: &'a TargetPipelines,
        products: PreparedSeriesProducts<'a>,
        styled: Option<&'a StyleSet>,
        column_buffer: &'a wgpu::Buffer,
        mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
        phase: Option<streaming_runtime::StreamDrawPhase>,
    ) -> Result<Vec<data_render::SeriesLayers<'a>>> {
        // The stem pipeline (#206) per draw style: the precise and sketch
        // stem entries, a Milkyway jet (its `vs_jet` stem branch), none
        // under Constellation, which drops a stem series.
        let stem_pipe = match styled {
            None => pipelines.errorbar_stem.as_ref(),
            Some(StyleSet::Sketch { stem, .. }) => Some(stem),
            Some(StyleSet::Milkyway(c)) => Some(&c.jets),
            Some(StyleSet::Constellation(_)) => None,
        };
        let (line_pick, stars_pick, scatter_pick, errorbar_pipe) = match styled {
            None => (
                pipelines.line.as_ref().map(|pipeline| LinePick {
                    pipeline,
                    verts: 4,
                    texture_bg: None,
                }),
                None,
                pipelines.scatter.as_ref().map(|pipeline| ScatterPick {
                    pipeline,
                    texture_bg: None,
                }),
                pipelines.errorbar.as_ref(),
            ),
            Some(StyleSet::Sketch {
                line,
                scatter,
                errorbar,
                line_verts,
                ..
            }) => (
                Some(LinePick {
                    pipeline: line,
                    verts: *line_verts,
                    texture_bg: None,
                }),
                None,
                Some(ScatterPick {
                    pipeline: scatter,
                    texture_bg: None,
                }),
                Some(errorbar),
            ),
            // Milkyway: line = ribbon + star pass, scatter = ringed
            // planets (premultiplied — they occlude the star field),
            // errorbar = bipolar jets with terminal shock knots.
            Some(StyleSet::Milkyway(c)) => (
                Some(LinePick {
                    pipeline: &c.ribbon,
                    verts: data_render::MILKYWAY_RIBBON_VERTICES,
                    texture_bg: Some(&c.star_tex_bg),
                }),
                Some(StarsPick {
                    pipeline: &c.stars,
                    texture_bg: &c.star_tex_bg,
                }),
                Some(ScatterPick {
                    pipeline: &c.planets,
                    texture_bg: Some(&c.star_tex_bg),
                }),
                Some(&c.jets),
            ),
            // Constellation: only ScatterLine is supported. The line is the
            // regular columnar stroke with style-level alpha; the scatter
            // layer renders PSF stars at the data points.
            Some(StyleSet::Constellation(c)) => (
                Some(LinePick {
                    pipeline: &c.line,
                    verts: 4,
                    texture_bg: None,
                }),
                None,
                Some(ScatterPick {
                    pipeline: &c.stars,
                    texture_bg: Some(&c.star_tex_bg),
                }),
                None,
            ),
        };
        let picks = StylePicks {
            line: line_pick,
            stars: stars_pick,
            scatter: scatter_pick,
            errorbar: errorbar_pipe,
            stem: stem_pipe,
        };

        // A 3D chart's camera for this panel (#132): the scatter and line
        // series with a height column draw through the 3D entries with it.
        let camera_3d = chart_config.view_3d().and_then(|_| {
            let panel = view.panel_rect;
            let target = (
                (f64::from(panel.x), f64::from(panel.y)),
                (f64::from(panel.width), f64::from(panel.height)),
            );
            data_render::surface_mesh::view3d_gpu(chart_config, target, target)
        });
        let mut out = Vec::with_capacity(series_specs.len());
        for (idx, series) in series_specs.iter().enumerate() {
            let cfg = series.config;
            let rt = &cfg.render_type;
            let constellation_only = matches!(styled, Some(StyleSet::Constellation(_)));
            let primitives = effective_series_primitives(&chart_config.draw_style, rt);
            // A DataLess series (energy levels, a Feynman diagram) reads no
            // column: its only layer is the CPU raster this prepare resolved
            // (#203, #234). A streamed phase never draws one (rule 8).
            if cfg.is_dataless() {
                let energy_levels = products
                    .contours
                    .get(idx)
                    .and_then(|series| series.energy_levels.as_ref())
                    .filter(|_| phase.is_none())
                    .map(|raster| data_render::AxisLayer {
                        pipeline: &pipelines.axis,
                        bind_group: &raster.layer.bind_group,
                    });
                out.push(data_render::SeriesLayers {
                    energy_levels,
                    ..data_render::SeriesLayers::empty()
                });
                continue;
            }
            let x_h = lookup(&cfg.x_column)?;
            // The height column of a scatter or line series on a 3D chart
            // (#132), and its camera group: the surface layout for the
            // precise entries, the combined value map and camera layout for
            // the mapped markers and their ring.
            // A point vector names its height too (#211): its arrows and its
            // picked anchor's ring draw through the camera.
            let z_3d = match (camera_3d.as_ref(), cfg.z_column.as_deref()) {
                (Some(_), Some(z))
                    if (primitives.line || primitives.scatter || primitives.vector)
                        && phase.is_none() =>
                {
                    Some(lookup(z)?)
                }
                _ => None,
            };
            // A box plot's `y_column` is its record, not a column: its
            // member columns are looked up with its params below.
            let y_h = if rt.box_plot().is_some() {
                x_h
            } else {
                lookup(&cfg.y_column)?
            };
            // A radar polygon has one vertex per category: its columns are
            // data, so their lengths are checked here against the pool.
            if rt.radar().is_some() {
                radar_columns_fit(chart_config, cfg, x_h.len_values, y_h.len_values)?;
            }

            let s = SeriesInputs {
                view,
                chart_config,
                series_specs,
                pipelines,
                products,
                styled,
                column_buffer,
                phase,
                picks,
                camera_3d: camera_3d.as_ref(),
                idx,
                series: *series,
                cfg,
                rt,
                constellation_only,
                primitives,
                x_h,
                y_h,
                z_3d,
            };
            let FieldLayers {
                field,
                mesh,
                mesh_contour,
                surface,
                selected_surface,
                floor,
                contour,
            } = self.field_layers(&s)?;
            let vector = self.vector_layer(&s)?;
            let category_bars = self.category_bar_layer(&s, &mut lookup)?;
            let (violin, box_points) = self.box_extra_layers(&s, &mut lookup)?;
            let bar = self.bar_layer(&s)?;
            let line = self.line_layer(&s, &mut lookup)?;
            let area = self.area_layer(&s, line.is_some(), &mut lookup)?;
            let region = self.region_layer(&s, &mut lookup)?;
            let line_extra = self.line_extra_layer(&s, line.is_some())?;
            let scatter = self.scatter_layer(&s, &mut lookup)?;
            let errorbar = self.errorbar_layer(&s, &mut lookup)?;
            let stem = self.stem_layer(&s)?;
            let selection_prepare::SelectionLayers {
                picked,
                selected_bars,
                selected_fields,
            } = self.selection_layers(
                &s,
                bar.as_ref(),
                category_bars.as_ref(),
                violin.as_ref(),
                box_points.as_ref(),
                &mut lookup,
            )?;
            let value_labels = self.value_label_layer(&s, category_bars.as_ref(), &mut lookup)?;
            let parallel = self.parallel_layer(&s, &mut lookup)?;
            // A 3D series' line and markers draw with the surfaces, not
            // among the flat bodies.
            let (line, line_3d) = match z_3d {
                Some(_) => (None, line),
                None => (line, None),
            };
            let (scatter, scatter_3d) = match z_3d {
                Some(_) => (None, scatter),
                None => (scatter, None),
            };
            let (vector, vector_3d) = match &vector {
                Some(layer) if layer.view3d.is_some() => (None, vector),
                _ => (vector, None),
            };
            out.push(data_render::SeriesLayers {
                area,
                field,
                mesh,
                mesh_contour,
                bar,
                violin,
                category_bars,
                box_points,
                contour,
                vector,
                errorbar,
                stem,
                line,
                parallel,
                line_extra,
                scatter,
                line_3d,
                scatter_3d,
                vector_3d,
                selected_bars,
                selected_fields,
                picked,
                value_labels,
                energy_levels: None,
                surface,
                selected_surface,
                floor,
                region,
            });
        }
        Ok(out)
    }

    /// The panel camera's group 3 for a 3D series: the surface layout for
    /// the precise entries, the combined value map and camera layout for the
    /// mapped markers and their ring.
    fn camera_binding(
        &self,
        s: &SeriesInputs<'_, '_>,
        mapped: bool,
    ) -> Result<data_render::surface_mesh::View3dBinding> {
        data_render::surface_mesh::create_view3d_binding(
            &self.gpu_ledger,
            &self.device,
            if mapped {
                self.view3d_value_map_bgl
            } else {
                self.surface_bgl
            },
            s.camera_3d
                .ok_or(FiggyError::internal("a 3D series has its panel's camera"))?,
            s.series.style.display_scale,
            0,
            self.scatter_value_map_buffers(s.cfg, mapped),
        )
    }
}
