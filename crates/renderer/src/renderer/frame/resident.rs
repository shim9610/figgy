//! The resident display cache: its key, probe and packet.

use super::*;

/// One panel's cached series bodies: every fill and every line, marker,
/// bar, field, contour and vector body of the panel's series, drawn once into
/// a transparent panel-sized texture. `prepare` draws it again only when its
/// [`ResidentDisplayKey`] changes; `paint_prepared` blits it between the grid
/// and the live picked highlights, so a repaint costs one textured quad
/// instead of every point again.
///
/// The cache holds the data layer only. Grid, decoration, annotation layers
/// and the picked and selected highlights keep their own textures or draw
/// live, and the key reads nothing they depend on, so editing or selecting
/// any of them never draws the bodies again. Each panel of a layered document
/// has its own entry, so editing one layer leaves every other layer's bodies.
///
/// Only Precise panels qualify: their pipelines blend premultiplied over,
/// which composites the same through a transparent texture (up to UNORM
/// rounding at partly covered pixels, as for the stream display). Styled blend
/// equations (additive, max) depend on what is under them, so those panels
/// keep drawing directly.
pub(in crate::renderer) struct ResidentDisplay {
    /// The panel view's content-revision cell. Its address names the panel
    /// (allocation identity): this `Weak` keeps the allocation, so the address
    /// cannot be reused by another panel while the entry exists. Once the view
    /// and every token holding it are gone, `strong_count()` is 0 and the
    /// entry retires. See "Identity and staleness checks" in `ARCHITECTURE.md`.
    pub(in crate::renderer) view: std::sync::Weak<std::sync::atomic::AtomicU64>,
    pub(in crate::renderer) key: ResidentDisplayKey,
    pub(in crate::renderer) serial: u64,
    pub(in crate::renderer) format: wgpu::TextureFormat,
    pub(in crate::renderer) sample_count: u32,
    /// Resolved, panel-sized composite the blit samples.
    pub(in crate::renderer) texture: TrackedTexture,
    /// Multisampled target resolved into `texture`; `None` at 1x.
    pub(in crate::renderer) msaa: Option<TrackedTexture>,
    /// The depth-stencil attachment of a 3D panel's composite, at the
    /// target's sample count (`surface_mesh::DEPTH_STENCIL_FORMAT`); `None`
    /// for every flat panel, whose pipelines have no depth state.
    pub(in crate::renderer) depth: Option<TrackedTexture>,
    pub(in crate::renderer) bind_group: wgpu::BindGroup,
}

/// Everything the series bodies are drawn from, and nothing else: the
/// data-to-pixel transform, the parts of the chart configuration the body
/// pipelines read, each series' configuration and style scale, the font
/// generation (contour labels), the allocation epochs of exactly the columns
/// the panel reads, and the pool layout and pipeline generations. A column
/// another panel reads never draws this one again.
/// Titles, legend, axes, grid, annotations and selections are not in it.
pub(in crate::renderer) struct ResidentDisplayKey {
    pub(in crate::renderer) size: (u32, u32),
    /// The data area relative to the panel.
    pub(in crate::renderer) data_area: Rect,
    pub(in crate::renderer) transform: data_render::ScatterTransform,
    pub(in crate::renderer) draw_style: crate::config::DrawStyle,
    pub(in crate::renderer) polar: Option<crate::polar::PolarOptions>,
    /// The parallel axes' ranges (`Config::parallel`): every row is placed
    /// by them (#141).
    pub(in crate::renderer) parallel: Option<crate::parallel::ParallelOptions>,
    pub(in crate::renderer) colorbar: Option<crate::config::ColorBarOptions>,
    /// How category bars share a slot (`Config::bar_groups`): group width,
    /// gap and stacking are drawn into the bodies.
    pub(in crate::renderer) bar_groups: Option<crate::data_config::BarGroups>,
    /// The panel's category axes (`bottom_x`, `left_y`) when it draws a
    /// category bar or box plot series, `None` otherwise: a bar or box
    /// override names its category by id, resolved to a row against these.
    pub(in crate::renderer) categories: Option<[Option<Vec<crate::config::Category>>; 2]>,
    /// The tick-label face: contour labels baked into the bodies take it
    /// (`ContourLabelAtlasInput::from_config`).
    pub(in crate::renderer) label_font: String,
    /// Each series' configuration and the display scale its style was built
    /// at: a style is a pure function of the two, so a host or the registry
    /// rebuilding every style after one series edit draws only the panels
    /// whose series changed.
    pub(in crate::renderer) series: Vec<(SeriesConfig, u32)>,
    pub(in crate::renderer) font_generation: u64,
    pub(in crate::renderer) columns: u64,
    pub(in crate::renderer) pool_layout_generation: u64,
    pub(in crate::renderer) target_pipeline_generation: u64,
    /// A 3D panel's camera uniform as the composite draws it: the camera,
    /// the fitted box (which the axes' label band sizes), the z axis range
    /// and the light. `None` for a flat panel.
    pub(in crate::renderer) camera: Option<data_render::surface_mesh::View3dGpu>,
    /// Whether the composite carries the stencil attachment: a ternary
    /// panel with a region fill. Its parameters are in `series`.
    pub(in crate::renderer) stencil: bool,
}

/// The current frame's inputs to compare against a [`ResidentDisplayKey`]
/// without copying them.
pub(in crate::renderer) struct ResidentDisplayProbe<'a> {
    pub(in crate::renderer) size: (u32, u32),
    pub(in crate::renderer) data_area: Rect,
    pub(in crate::renderer) transform: data_render::ScatterTransform,
    pub(in crate::renderer) chart_config: &'a Config,
    pub(in crate::renderer) series: &'a [Series<'a>],
    pub(in crate::renderer) font_generation: u64,
    pub(in crate::renderer) columns: u64,
    pub(in crate::renderer) pool_layout_generation: u64,
    pub(in crate::renderer) target_pipeline_generation: u64,
    pub(in crate::renderer) camera: Option<data_render::surface_mesh::View3dGpu>,
    pub(in crate::renderer) stencil: bool,
}

impl ResidentDisplayProbe<'_> {
    /// Whether the panel's composite needs the depth-stencil attachment.
    pub(in crate::renderer) fn needs_depth_stencil(&self) -> bool {
        self.camera.is_some() || self.stencil
    }
}

impl ResidentDisplayKey {
    pub(in crate::renderer) fn matches(&self, probe: &ResidentDisplayProbe<'_>) -> bool {
        self.size == probe.size
            && self.data_area == probe.data_area
            && bytemuck::bytes_of(&self.transform) == bytemuck::bytes_of(&probe.transform)
            && self.font_generation == probe.font_generation
            && self.columns == probe.columns
            && self.pool_layout_generation == probe.pool_layout_generation
            && self.target_pipeline_generation == probe.target_pipeline_generation
            && match (&self.camera, &probe.camera) {
                (Some(a), Some(b)) => bytemuck::bytes_of(a) == bytemuck::bytes_of(b),
                (None, None) => true,
                _ => false,
            }
            && self.stencil == probe.stencil
            && self.draw_style == probe.chart_config.draw_style
            && self.polar.as_ref() == probe.chart_config.polar()
            && self.parallel.as_ref() == probe.chart_config.parallel()
            && self.colorbar == probe.chart_config.colorbar
            && self.bar_groups == probe.chart_config.bar_groups
            && match &self.categories {
                Some([bottom, left]) => {
                    has_category_bars(probe.series)
                        && *bottom == probe.chart_config.bottom_x.categories
                        && *left == probe.chart_config.left_y.categories
                }
                None => !has_category_bars(probe.series),
            }
            && self.label_font == probe.chart_config.bottom_x.label_style.label_font
            && self.series.len() == probe.series.len()
            && self
                .series
                .iter()
                .zip(probe.series)
                .all(|((config, scale), now)| {
                    config == now.config && *scale == now.style.display_scale.to_bits()
                })
    }

    pub(in crate::renderer) fn from_probe(probe: &ResidentDisplayProbe<'_>) -> Self {
        Self {
            size: probe.size,
            data_area: probe.data_area,
            transform: probe.transform,
            draw_style: probe.chart_config.draw_style,
            polar: probe.chart_config.polar().cloned(),
            parallel: probe.chart_config.parallel().cloned(),
            colorbar: probe.chart_config.colorbar.clone(),
            bar_groups: probe.chart_config.bar_groups.clone(),
            categories: has_category_bars(probe.series).then(|| {
                [
                    probe.chart_config.bottom_x.categories.clone(),
                    probe.chart_config.left_y.categories.clone(),
                ]
            }),
            label_font: probe.chart_config.bottom_x.label_style.label_font.clone(),
            series: probe
                .series
                .iter()
                .map(|series| (series.config.clone(), series.style.display_scale.to_bits()))
                .collect(),
            font_generation: probe.font_generation,
            columns: probe.columns,
            pool_layout_generation: probe.pool_layout_generation,
            target_pipeline_generation: probe.target_pipeline_generation,
            camera: probe.camera,
            stencil: probe.stencil,
        }
    }
}

/// Whether any of the panel's series is a category bar or a box plot
/// series: their overrides name their category by id.
pub(super) fn has_category_bars(series: &[Series<'_>]) -> bool {
    series.iter().any(|series| {
        let rt = &series.config.render_type;
        rt.category_bars().is_some() || rt.box_plot().is_some()
    })
}

/// A token's hold on one [`ResidentDisplay`]: the bind group to blit and the
/// serial it was drawn at, so a later re-draw makes an older token stale.
pub(in crate::renderer) struct ResidentDisplayPacket {
    /// Address of the view's content-revision cell, the entry's identity.
    pub(in crate::renderer) view: usize,
    pub(in crate::renderer) serial: u64,
    pub(in crate::renderer) bind_group: wgpu::BindGroup,
    pub(in crate::renderer) _charge: crate::gpu_memory::SharedCharge,
}
