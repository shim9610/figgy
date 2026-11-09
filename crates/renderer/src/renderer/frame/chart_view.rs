//! Chart views, their layer stamps and panel raster keys.

use super::*;

/// Per-panel GPU resources: separate grid + decoration textures (for layered
/// compositing) plus the transform uniform.
///
/// Grid (below data) and decoration (axis lines, labels — above data) are
/// rasterized to two textures so the GPU can composite in order:
/// `grid → data → decoration`.
pub struct ChartView {
    pub(in crate::renderer) grid_texture: TrackedTexture,
    pub(in crate::renderer) grid_bind_group: wgpu::BindGroup,
    pub(in crate::renderer) decoration_texture: TrackedTexture,
    pub(in crate::renderer) decoration_bind_group: wgpu::BindGroup,
    /// The annotation layer: spans below the data, everything else above
    /// every decoration. `None` where the layer drew nothing on that side.
    /// Refreshed by `refresh_axis*` (the anchor space may have moved) and by
    /// `refresh_annotations` (the items changed); never by anything else.
    pub(in crate::renderer) annotation_underlay: Option<crate::annotation::LayerTexture>,
    pub(in crate::renderer) annotation_overlay: Option<crate::annotation::LayerTexture>,
    pub(in crate::renderer) annotation_map: crate::annotation::AnnotationMap,
    /// The annotation whose selection box the overlay carries.
    pub(in crate::renderer) annotation_selected: Option<usize>,
    pub(in crate::renderer) layer_stamps: ChartViewLayerStamps,
    pub(in crate::renderer) transform_buffer: TrackedBuffer,
    pub(in crate::renderer) transform_bg: wgpu::BindGroup,
    /// Moves on every content change, so a prepared token can tell it is
    /// stale. The cell's allocation is also the panel's identity (the
    /// resident display cache keys on it), so it is created once and never
    /// replaced. See "Identity and staleness checks" in `ARCHITECTURE.md`.
    pub(in crate::renderer) content_revision: Arc<std::sync::atomic::AtomicU64>,
    /// Changes only when the streamed data prefix's coordinate space changes.
    /// Decoration texture rewrites deliberately leave this revision untouched.
    pub(in crate::renderer) stream_revision: Arc<std::sync::atomic::AtomicU64>,
    /// Panel pixel rect in surface coordinates.
    pub(in crate::renderer) panel_rect: Rect,
    /// The display or export scale the view's rasters were drawn at
    /// ([`Renderer::create_chart_view_at`],
    /// [`Renderer::refresh_axis_with_selection_at`]; 1 otherwise), the one a
    /// broken axis' piece frames snapped on (#205).
    pub(in crate::renderer) display_scale: f32,
    /// Generation of the renderer's cached constellation backdrop currently
    /// uploaded into `grid_texture` (`None` = non-cached content). Lets
    /// `refresh_axis` skip the full-panel texture write on cache hits.
    pub(in crate::renderer) grid_space_gen: Option<u64>,
    /// Tick-guard substitutions in the current axis raster, derived from the
    /// chart that `create_chart_view` or the last successful `refresh_axis*`
    /// rasterized. On a layered view this is the merged list of every panel,
    /// overlay warnings tagged `WarningAxis::OverlayY(index)`.
    pub(in crate::renderer) tick_warnings: Vec<crate::RenderWarning>,
    /// `tick_warnings` followed by the annotation layer's skipped items.
    pub(in crate::renderer) render_warnings: Vec<crate::RenderWarning>,
    /// One nested single-panel view per overlay layer, in `Config::overlays`
    /// order. Each holds its own grid and decoration textures and transform
    /// uniform; none nests further (one level of layering).
    pub(in crate::renderer) overlay_views: Vec<ChartView>,
    /// What this panel's textures hold: the internal panel config, the
    /// selection boxes composited into its decoration texture, the font
    /// generation and the panel rect of the last raster. A refresh that
    /// arrives with the same key uploads nothing, which is what keeps the
    /// other layers' textures untouched when one layer is edited.
    pub(in crate::renderer) raster_key: Option<PanelRasterKey>,
    /// Counts the rasters uploaded into this panel's textures.
    pub(in crate::renderer) raster_generation: u64,
}

/// Cache key of one panel's grid and decoration rasters.
#[derive(Clone, PartialEq)]
pub(in crate::renderer) struct PanelRasterKey {
    pub(in crate::renderer) config: Config,
    pub(in crate::renderer) selection: Vec<crate::select::SelectionBox>,
    pub(in crate::renderer) font_generation: u64,
    pub(in crate::renderer) panel_rect: Rect,
    /// The display or export scale the raster snapped its piece frames on
    /// (#205).
    pub(in crate::renderer) scale: f32,
}

impl PanelRasterKey {
    /// The annotation layer has its own rasters and revision, so it is not
    /// part of the axis key: an annotation edit keeps every panel's cache.
    pub(in crate::renderer) fn new(
        config: &Config,
        selection: &[crate::select::SelectionBox],
        font_generation: u64,
        panel_rect: Rect,
        scale: f32,
    ) -> Self {
        let mut config = config.clone();
        config.annotations = crate::AnnotationLayer::empty();
        Self {
            config,
            selection: selection.to_vec(),
            font_generation,
            panel_rect,
            scale,
        }
    }
}

/// How many times each raster layer of a [`ChartView`] has been written,
/// starting at 1 for the view's creation. Lets a host or a test prove that a
/// refresh touched only the layer it was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChartViewLayerStamps {
    pub grid: u64,
    pub decoration: u64,
    pub annotations: u64,
}

impl ChartView {
    pub fn panel_rect(&self) -> Rect {
        self.panel_rect
    }

    /// The number of overlay layers this view holds textures for, which is
    /// the overlay count of the config it was created or last refreshed with.
    pub fn overlay_count(&self) -> usize {
        self.overlay_views.len()
    }

    /// How many rasters have been uploaded into each panel's textures, base
    /// panel first and then one entry per overlay. An edit of one layer
    /// advances only that layer's entry (and the top panel's, when the
    /// selection boxes it carries change); a frame that reuses a panel's
    /// cached raster leaves its entry alone.
    pub fn layer_raster_generations(&self) -> Vec<u64> {
        std::iter::once(self.raster_generation)
            .chain(self.overlay_views.iter().map(|view| view.raster_generation))
            .collect()
    }

    /// Renderer-measured annotation hit map of the current annotation
    /// raster, in the chart's own coordinates (the chart area's frame).
    /// Rebuilt by `refresh_axis*` and `refresh_annotations`; a `HitId` from
    /// it is valid until the next of those calls.
    pub fn annotation_map(&self) -> &crate::annotation::AnnotationMap {
        &self.annotation_map
    }

    /// The pointer route of the active layer at `position`: while
    /// `config.active_layer()` is `Annotations` (and the layer is shown) the
    /// annotation items of this view sit on top of `hit_map`, otherwise only
    /// `hit_map` answers and clicks pass through the items. `config` is the
    /// config the view was last refreshed with (the display config of a
    /// scaled view) and `hit_map` its [`crate::HitMap::for_active_layer`] map, so
    /// both maps measure in the same pixels.
    pub fn hit_test_active_layer(
        &self,
        config: &Config,
        hit_map: &crate::HitMap,
        position: [f32; 2],
    ) -> Option<crate::annotation::LayerHit> {
        self.annotation_map
            .route(config.active_layer(), &config.annotations, position, || {
                hit_map.hit_test(
                    config,
                    &crate::CpuTextMeasure::for_style(&config.draw_style),
                    position[0],
                    position[1],
                )
            })
    }

    /// The annotation whose selection box the overlay currently carries, as
    /// last passed to [`Renderer::refresh_annotations`].
    pub fn annotation_selected(&self) -> Option<usize> {
        self.annotation_selected
    }

    pub fn layer_stamps(&self) -> ChartViewLayerStamps {
        self.layer_stamps
    }

    pub(in crate::renderer) fn rebuild_render_warnings(&mut self) {
        self.render_warnings = self
            .tick_warnings
            .iter()
            .chain(self.annotation_map.warnings())
            .cloned()
            .collect();
    }

    /// Bind groups of the annotation textures for the stream display
    /// composition, which draws underlay and overlay itself.
    pub(crate) fn annotation_bind_groups(
        &self,
        visible: bool,
    ) -> (Option<&wgpu::BindGroup>, Option<&wgpu::BindGroup>) {
        if !visible {
            return (None, None);
        }
        (
            self.annotation_underlay.as_ref().map(|l| &l.bind_group),
            self.annotation_overlay.as_ref().map(|l| &l.bind_group),
        )
    }

    /// The tick settings this view's current axis raster drew differently
    /// from how its chart's config wrote them: an impossible `major_spacing`
    /// drawn at automatic spacing, a clamped `minor_count`, an adjusted log
    /// decade step. Empty when every setting is drawn as written.
    ///
    /// [`Renderer::create_chart_view`] sets it, and every successful
    /// [`Renderer::refresh_axis`] / [`Renderer::refresh_axis_with_selection`]
    /// re-derives it from the chart it rasterized; nothing else changes it.
    /// A host that refreshes only when its dirty flag or
    /// [`ChartRenderStamp::needs_raster_since`] says the raster changed reads
    /// it right after that refresh. Compare with the previous list to report
    /// changes only; a NaN `requested` never compares equal.
    ///
    /// The tick warnings are followed by the annotation layer's
    /// `AnnotationSkipped` warnings, re-derived by `refresh_annotations` and
    /// by every `refresh_axis*` (the latter redoes the annotation raster).
    pub fn render_warnings(&self) -> &[crate::RenderWarning] {
        &self.render_warnings
    }

    pub(in crate::renderer) fn advance_content_revision(&self) -> Result<u64> {
        self.content_revision
            .try_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |current| current.checked_add(1),
            )
            .map(|previous| previous + 1)
            .map_err(|_| FiggyError::CounterExhausted {
                counter: "chart view content revision",
            })
    }

    pub(in crate::renderer) fn advance_stream_revision(&self) -> Result<u64> {
        self.stream_revision
            .try_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |current| current.checked_add(1),
            )
            .map(|previous| previous + 1)
            .map_err(|_| FiggyError::CounterExhausted {
                counter: "chart view stream revision",
            })
    }
}
