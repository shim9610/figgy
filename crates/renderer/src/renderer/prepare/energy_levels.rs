//! Energy-level preparation (#203): each energy-level series is drawn on the
//! CPU through the model's emit ([`crate::energy_levels::LevelPlacement`])
//! into one panel-sized raster, cached by everything the raster reads, and
//! composited in the data slot after every GPU body and picked highlight of
//! its panel (`renderer::composite`). The approved exception to "per-point
//! appearance is computed in the shader" (#203): the series is DataLess, its
//! appearance is computed at raster time from the SSOT and never stored.

use super::*;
use crate::energy_levels::{EnergyLevelSeries, LevelPlacement, PickedRow};

/// One energy-level series' raster in the scratch cache
/// ([`ScratchEntry::EnergyLevels`]), keyed by what the raster reads: the
/// panel config (axes, categories, areas, colour bar, selection, draw
/// style), the series' own SSOT and the records it draws (its own, or its
/// source's), and the font generation. Every key part is SSOT
/// configuration, which rule 2 lets the cache copy.
pub(in crate::renderer) struct EnergyLevelScratch {
    config: Config,
    series: SeriesConfig,
    records: Option<EnergyLevelSeries>,
    font_generation: u64,
    scale: f32,
    raster: Arc<EnergyLevelRaster>,
}

/// The uploaded raster of one energy-level series in one panel.
pub(crate) struct EnergyLevelRaster {
    pub(crate) layer: crate::annotation::LayerTexture,
}

/// The row of `cfg` the panel's typed selection picks, as the emit widens
/// it: a histogram bin is a level row, a point a transition row.
fn picked_row(config: &Config, cfg: &SeriesConfig) -> Option<PickedRow> {
    use crate::config::PickedDataRef;
    let selection = config.picked_data.as_ref().filter(|s| s.visible)?;
    match selection.refs.first()? {
        PickedDataRef::HistogramBin {
            series_id,
            bin_index,
            ..
        } if *series_id == cfg.series_id => Some(PickedRow::Level(*bin_index)),
        PickedDataRef::Point {
            series_id,
            point_index,
            ..
        } if *series_id == cfg.series_id => Some(PickedRow::Transition(*point_index)),
        _ => None,
    }
}

/// The records `levels` draws: its own, or those of its source among the
/// document's series (every panel's), `None` when the source names no
/// energy-level series that holds records.
pub(in crate::renderer) fn source_records<'s>(
    levels: &'s EnergyLevelSeries,
    items: &'s [ChartDrawItem<'s>],
) -> Option<&'s EnergyLevelSeries> {
    let Some(source) = &levels.source else {
        return Some(levels);
    };
    items
        .iter()
        .flat_map(|item| item.series.iter())
        .map(|series| series.config)
        .find(|cfg| cfg.series_id == *source)
        .and_then(SeriesConfig::energy_levels)
        .filter(|found| found.source.is_none())
}

/// Rasterize `records` in the look and view of `levels` over the panel
/// `config`: one RGBA8 canvas the size of the chart area, the panel's
/// origin at its top-left pixel. `config` is placed at `scale` render
/// pixels per logical pixel (a display or export config is scaled
/// already) while the series' look is in logical pixels, so the parts are
/// placed in logical pixels (the anchor space divided by `scale`) and the
/// canvas sink scales them, text included.
pub(crate) fn rasterize(
    config: &Config,
    levels: &EnergyLevelSeries,
    records: &EnergyLevelSeries,
    picked: Option<PickedRow>,
    scale: f32,
) -> Result<Option<(Vec<u8>, (u32, u32))>> {
    let area = config.chart_area.0;
    let (w, h) = (area.width.max(1), area.height.max(1));
    let Some(mut canvas) = crate::raster::Canvas::new(w, h) else {
        return Err(FiggyError::RasterWrapFailed {
            reason: format!("energy-level raster of {w}x{h} pixels"),
        });
    };
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let logical = |r: crate::layout::RectF| crate::layout::RectF {
        x: r.x / scale,
        y: r.y / scale,
        width: r.width / scale,
        height: r.height / scale,
    };
    let mut space = crate::annotation::pixel_space_for_config(config);
    space.panel = logical(space.panel);
    space.plot = logical(space.plot);
    let placement = LevelPlacement {
        space: &space,
        categories: config.bottom_x.categories.as_deref().unwrap_or(&[]),
        view: levels.show,
        colorbar: config.colorbar.as_ref(),
        picked,
    };
    let policy = crate::text_render::FontPolicy::for_style(&config.draw_style);
    let measure = crate::text_render::CpuTextMeasure::with_policy(policy);
    let mut sink = crate::decoration_render::CanvasSink::placed(
        None,
        Some(&mut canvas),
        crate::sketch::DecoStroker::Precise,
        policy,
        [area.x as f32 / scale, area.y as f32 / scale],
        scale,
    );
    placement.emit(records, &levels.style, &measure, &mut sink);
    sink.finish()?;
    Ok(Some((canvas.into_rgba(), (w, h))))
}

impl PrepareContext<'_> {
    /// The raster energy-level series `cfg` draws in the panel `config`,
    /// rastered or reused; `None` for every other series, and for a view
    /// whose source holds no records.
    pub(in crate::renderer) fn ensure_energy_level_scratch(
        &mut self,
        config: &Config,
        cfg: &SeriesConfig,
        items: &[ChartDrawItem<'_>],
        scale: f32,
    ) -> Result<Option<Arc<EnergyLevelRaster>>> {
        let Some(levels) = cfg.energy_levels() else {
            return Ok(None);
        };
        let Some(records) = source_records(levels, items) else {
            return Ok(None);
        };
        let source_copy = levels.source.is_some().then_some(records);
        let font_generation = crate::text_render::font_generation();
        if let Some(ScratchEntry::EnergyLevels(scratch)) =
            self.contour_cache.get(cfg.series_id.as_str())
            && scratch.font_generation == font_generation
            && scratch.scale == scale
            && scratch.config == *config
            && scratch.series == *cfg
            && scratch.records.as_ref() == source_copy
        {
            return Ok(Some(Arc::clone(&scratch.raster)));
        }
        let Some((rgba, (w, h))) =
            rasterize(config, levels, records, picked_row(config, cfg), scale)?
        else {
            return Ok(None);
        };
        let texture = crate::gpu_memory::TrackedTexture::new(
            self.gpu_ledger,
            crate::gpu_memory::GpuResourceKind::PanelTexture,
            data_render::upload_rgba_texture(
                self.device,
                self.queue,
                self.caps.max_texture_dimension_2d,
                w,
                h,
                &rgba,
            )?,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = data_render::create_texture_bind_group(
            self.device,
            self.texture_bgl,
            &view,
            self.sampler,
        );
        let raster = Arc::new(EnergyLevelRaster {
            layer: crate::annotation::LayerTexture {
                texture,
                bind_group,
            },
        });
        // The contour scratch cache: one map, one eviction rule, one limit.
        evict_if_full(
            &mut self.contour_cache,
            cfg.series_id.as_str(),
            CONTOUR_SCRATCH_CACHE_LIMIT,
        );
        self.contour_cache
            .try_reserve(1)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "contour scratch cache",
                reason: error.to_string(),
            })?;
        self.contour_cache.insert(
            cfg.series_id.clone(),
            ScratchEntry::EnergyLevels(EnergyLevelScratch {
                config: config.clone(),
                series: cfg.clone(),
                records: source_copy.cloned(),
                font_generation,
                scale,
                raster: Arc::clone(&raster),
            }),
        );
        Ok(Some(raster))
    }
}
