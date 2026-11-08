//! Frame assembly: chart views and their layer textures, the prepared frame
//! and its per-series products, resident display caches, panel placement for
//! layered documents, and the prepare, refresh and paint entry points.

use super::*;

mod chart_view;
mod paint;
mod panels;
mod prepared;
mod prepared_layers;
mod resident;
mod submit;
mod views;

pub use chart_view::*;
pub use panels::*;
pub use prepared::*;
pub(super) use prepared_layers::*;
pub(super) use resident::*;

/// Key + bytes of one baked constellation backdrop. `bake_gen` is a
/// monotonically increasing stamp; a `ChartView` whose grid texture already
/// holds this generation skips the (w·h·4)-byte re-upload too.
pub(super) struct SpaceBgCache {
    pub(super) key: SpaceBgKey,
    pub(super) bake_gen: u64,
    pub(super) rgba: Vec<u8>,
}

/// What the Milkyway backdrop is baked from. [`Self::bake`] reads the key
/// and nothing else, so a backdrop parameter that is not compared here does
/// not reach the raster.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct SpaceBgKey {
    pub(super) w: u32,
    pub(super) h: u32,
    pub(super) nebula: u32,
    pub(super) dust: u32,
    pub(super) seed: u32,
}

impl SpaceBgKey {
    fn bake(&self) -> Result<Vec<u8>> {
        let SpaceBgKey {
            w,
            h,
            nebula,
            dust,
            seed,
        } = *self;
        let Some(mut canvas) = crate::raster::Canvas::new(w, h) else {
            return Err(FiggyError::RasterWrapFailed {
                reason: format!("raster target allocation failed for {w}x{h}"),
            });
        };
        crate::axis_render::draw_space_background(
            &mut canvas,
            seed,
            f32::from_bits(nebula),
            f32::from_bits(dust),
        );
        Ok(canvas.into_rgba())
    }
}

/// One chart panel to draw: view + chart config + series slice.
pub struct ChartDrawItem<'a> {
    pub view: &'a ChartView,
    pub chart_config: &'a crate::config::Config,
    pub series: &'a [Series<'a>],
}

/// Prepare-only resources for an explicitly identified renderer-owned chart.
/// Config and ordered series are borrowed from the Renderer registry.
pub struct RegisteredChartDrawItem<'a> {
    pub chart_id: ChartId,
    pub view: &'a ChartView,
}
