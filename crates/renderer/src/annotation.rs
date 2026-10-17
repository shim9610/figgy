//! The dedicated annotation layer: reference lines, spans, text boxes and
//! arrows drawn by one shared CPU raster pass for every chart family.
//!
//! The layer is two panel-sized premultiplied RGBA8 textures. The *underlay*
//! (spans) is composited below the data and the *overlay* (lines, text,
//! arrows and the selection box) above every decoration, both with the
//! fullscreen textured pipeline the Cartesian renderer already uses for its
//! grid and decoration rasters. A chart family contributes exactly one
//! thing: its [`PixelSpace`] (the anchor ↔ pixel conversion of the frame).
//! Family shaders and decoration rasters are untouched, and an annotation
//! edit re-rasterizes only this layer.
//!
//! Hit testing uses the same measured geometry as the raster through an
//! [`AnnotationMap`], the annotation twin of
//! [`DecorationMap`](crate::decoration::DecorationMap): the renderer measures
//! text at the output scale when it registers an element, and the model's
//! `Selectable` / `Draggable` / `Resizable` implementations for
//! [`AnnotationElement`] turn pointer deltas back into anchors.

use std::sync::Arc;

use crate::config::{AxisOptions, AxisScale, Config};
use crate::decoration::scaled_rich_text;
use crate::gpu_memory::{GpuLedger, GpuResourceKind, TrackedTexture};
use crate::layout::RectF;
use crate::raster::{Canvas, Paint};
use crate::render_warning::{AnnotationSkip, RenderWarning};
use crate::text_render::{FontPolicy, draw_rich_text, measure_rich_text};
use crate::{CpuTextMeasure, HitId, HitMap, RendererDevice};
pub use model::annotation::{
    Anchor, AnchorKind, Annotation, AnnotationAxis, AnnotationElement, AnnotationGeometry,
    AnnotationHandle, AnnotationKind, AnnotationLineStyle, AnnotationSpace, ArrowStyle, AxisMap,
    AxisSpace, BoxStyle, PixelSpace,
};
use model::annotation::{arrow_label_rect, geometry, line_label_rect};
use model::{AnnotationLayer, DecorationId, layout::NudgeResult};

/// The anchor space of a Cartesian chart: the chart area is the panel, the
/// data area the plot, and `bottom_x` / `left_y` the two value axes (the
/// axes the data transform draws against). Logarithmic ranges are guarded
/// the same way the data transform guards them.
pub fn pixel_space_for_config(config: &Config) -> PixelSpace {
    let panel = RectF::from_rect(&config.chart_area.0);
    let plot = config
        .data_area()
        .map(|area| RectF::from_rect(&area.0))
        .unwrap_or(panel);
    PixelSpace {
        panel,
        plot,
        x: axis_space(&config.bottom_x),
        y: axis_space(&config.left_y),
    }
}

fn axis_space(axis: &AxisOptions) -> AxisSpace {
    let map = match axis.scale {
        AxisScale::Linear => AxisMap::Linear {
            min: axis.min,
            max: axis.max,
        },
        AxisScale::Logarithmic => {
            let (min, max) = crate::chart::guarded_log_range(axis.min, axis.max);
            AxisMap::Log { min, max }
        }
    };
    AxisSpace {
        map,
        inverted: axis.inverted,
    }
}

/// Renderer-measured annotation hit map for one frame. Coordinates and drag
/// deltas are logical pixels in the frame's own space: chart-local for
/// bounded charts, the chart area's frame for Cartesian charts.
pub struct AnnotationMap {
    state: AnnotationLayer,
    space: PixelSpace,
    map: HitMap<AnnotationLayer>,
    entries: Vec<(HitId, AnnotationElement)>,
    warnings: Vec<RenderWarning>,
}

impl AnnotationMap {
    pub(crate) fn empty(space: PixelSpace) -> Self {
        Self {
            state: AnnotationLayer::default(),
            space,
            map: HitMap::for_state(),
            entries: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn hit_map(&self) -> &HitMap<AnnotationLayer> {
        &self.map
    }

    /// The layer state the map was built from.
    pub fn layer(&self) -> &AnnotationLayer {
        &self.state
    }

    /// The anchor space of the frame, for hosts that place new items at a
    /// pointer position or convert anchor kinds.
    pub fn space(&self) -> &PixelSpace {
        &self.space
    }

    /// Items the raster skipped, as `RenderWarning::AnnotationSkipped`.
    pub fn warnings(&self) -> &[RenderWarning] {
        &self.warnings
    }

    /// The common hit-map contract: a hit can be queried for identity,
    /// bounds, selection box, drag and handles. Nothing is selected
    /// implicitly. Later items win where items overlap.
    pub fn hit_test(&self, position: [f32; 2]) -> Option<HitId> {
        if position.iter().any(|v| !v.is_finite())
            || !self.space.panel.contains(position[0], position[1])
        {
            return None;
        }
        self.map.hit_test(
            &self.state,
            &CpuTextMeasure::default(),
            position[0],
            position[1],
        )
    }

    pub fn element(&self, id: HitId) -> Option<&AnnotationElement> {
        self.entries.iter().find(|(i, _)| *i == id).map(|(_, e)| e)
    }

    pub fn entries(&self) -> impl Iterator<Item = (HitId, &AnnotationElement)> {
        self.entries.iter().map(|(id, e)| (*id, e))
    }

    /// The hit id of `DecorationId::Annotation(index)`, when that item was
    /// drawn.
    pub fn find(&self, index: usize) -> Option<HitId> {
        self.entries
            .iter()
            .find(|(_, e)| e.index == index)
            .map(|(id, _)| *id)
    }

    pub fn bounds(&self, id: HitId) -> Option<RectF> {
        self.map
            .get(id)?
            .bounds(&self.state, &CpuTextMeasure::default())
    }

    pub fn selection_box(&self, id: HitId) -> Option<crate::SelectionBox> {
        self.map
            .selection_box(id, &self.state, &CpuTextMeasure::default())
    }

    /// Update the chart's SSOT through the common Draggable policy, then
    /// refresh the layer. Reuse this map only until the frame's layout,
    /// size or font set changes.
    pub fn drag_by(
        &self,
        layer: &mut AnnotationLayer,
        id: HitId,
        dx: f32,
        dy: f32,
    ) -> Option<NudgeResult> {
        Some(self.map.get(id)?.as_draggable()?.drag_by(layer, dx, dy))
    }

    pub fn hit_resize_handle(
        &self,
        id: HitId,
        position: [f32; 2],
    ) -> Option<crate::resize::ResizeHandle> {
        self.map.get(id)?.as_resizable()?.hit_resize_handle(
            &self.state,
            &CpuTextMeasure::default(),
            position[0],
            position[1],
        )
    }

    pub fn resize_by(
        &self,
        layer: &mut AnnotationLayer,
        id: HitId,
        handle: crate::resize::ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> Option<NudgeResult> {
        Some(
            self.map
                .get(id)?
                .as_resizable()?
                .resize_by(layer, handle, dx, dy),
        )
    }
}

/// One rasterization of an annotation layer.
pub(crate) struct AnnotationRaster {
    /// Premultiplied RGBA8, `None` when no span was drawn.
    pub underlay: Option<Vec<u8>>,
    /// Premultiplied RGBA8, `None` when nothing was drawn above the data.
    pub overlay: Option<Vec<u8>>,
    pub map: AnnotationMap,
}

/// What one rasterization needs.
pub(crate) struct RasterRequest<'a> {
    pub layer: &'a AnnotationLayer,
    /// The frame's anchor space, in the map's logical coordinates.
    pub space: PixelSpace,
    /// The logical origin of the raster's top-left pixel (the chart area's
    /// origin for Cartesian charts, zero for bounded charts).
    pub origin: [f32; 2],
    /// Raster size in physical pixels.
    pub size: (u32, u32),
    /// Physical pixels per logical pixel.
    pub scale: f32,
    /// The item whose selection box is drawn on the overlay.
    pub selected: Option<usize>,
    pub policy: FontPolicy,
}

/// Measured size of a text block in logical pixels, plus the first line's
/// ascent in physical pixels for baseline placement.
struct Measured {
    size: [f32; 2],
    ascent: f32,
    padding: f32,
}

fn measure(
    text: &model::text::RichText,
    padding: f32,
    scale: f32,
    policy: FontPolicy,
) -> Option<Measured> {
    if text.segments.is_empty() {
        return None;
    }
    let m = measure_rich_text(&scaled_rich_text(text, scale), policy);
    Some(Measured {
        size: [
            m.width / scale + padding * 2.0,
            m.height() / scale + padding * 2.0,
        ],
        ascent: m.ascent,
        padding,
    })
}

fn skip_reason(
    kind: &AnnotationKind,
    space: &PixelSpace,
    text_size: Option<[f32; 2]>,
) -> AnnotationSkip {
    let axes = space.value_axes();
    let finite_anchor = |a: &Anchor| match *a {
        Anchor::Data { x, y } => x.is_finite() && y.is_finite(),
        Anchor::Ratio { x, y } => x.is_finite() && y.is_finite(),
    };
    let axis_has_values = |axis: AnnotationAxis| match axis {
        AnnotationAxis::X => axes[0],
        AnnotationAxis::Y => axes[1],
    };
    let anchor_has_axes = |a: &Anchor| !matches!(a, Anchor::Data { .. }) || (axes[0] && axes[1]);
    match kind {
        AnnotationKind::ReferenceLine { axis, value, .. } => {
            if !value.is_finite() {
                AnnotationSkip::NonFinite
            } else if !axis_has_values(*axis) {
                AnnotationSkip::NoValueAxis
            } else {
                AnnotationSkip::OutOfRange
            }
        }
        AnnotationKind::Span { axis, from, to, .. } => {
            if !from.is_finite() || !to.is_finite() {
                AnnotationSkip::NonFinite
            } else if !axis_has_values(*axis) {
                AnnotationSkip::NoValueAxis
            } else {
                AnnotationSkip::OutOfRange
            }
        }
        AnnotationKind::Text { anchor, .. } => {
            if text_size.is_none() {
                AnnotationSkip::EmptyText
            } else if !finite_anchor(anchor) {
                AnnotationSkip::NonFinite
            } else if !anchor_has_axes(anchor) {
                AnnotationSkip::NoValueAxis
            } else {
                AnnotationSkip::OutOfRange
            }
        }
        AnnotationKind::Arrow { from, to, .. } => {
            if !finite_anchor(from) || !finite_anchor(to) {
                AnnotationSkip::NonFinite
            } else if !anchor_has_axes(from) || !anchor_has_axes(to) {
                AnnotationSkip::NoValueAxis
            } else {
                AnnotationSkip::OutOfRange
            }
        }
    }
}

fn dash(style: &model::line::LineStylePreset, scale: f32) -> Vec<f32> {
    style.pattern().iter().map(|v| v * scale).collect()
}

/// Rasterize `request.layer` into the underlay and overlay and register the
/// drawn items in an [`AnnotationMap`]. Items are drawn in index order; a
/// hidden or unplaceable item is skipped and, when unplaceable, reported.
pub(crate) fn rasterize(request: &RasterRequest<'_>) -> AnnotationRaster {
    let (w, h) = request.size;
    let scale = request.scale;
    let space = request.space;
    let origin = request.origin;
    let mut map = AnnotationMap::empty(space);
    map.state = request.layer.clone();
    if w == 0 || h == 0 || scale <= 0.0 || !scale.is_finite() {
        return AnnotationRaster {
            underlay: None,
            overlay: None,
            map,
        };
    }
    let mut underlay: Option<Canvas> = None;
    let mut overlay: Option<Canvas> = None;
    fn canvas(slot: &mut Option<Canvas>, w: u32, h: u32) -> Option<&mut Canvas> {
        if slot.is_none() {
            *slot = Canvas::new(w, h);
        }
        slot.as_mut()
    }
    // Logical map coordinate -> physical raster pixel.
    let px = |p: [f32; 2]| ((p[0] - origin[0]) * scale, (p[1] - origin[1]) * scale);
    let mut registered = Vec::new();
    for (index, item) in request.layer.items.iter().enumerate() {
        if !item.visible {
            continue;
        }
        let measured = match &item.kind {
            AnnotationKind::ReferenceLine { label, .. } => label
                .as_ref()
                .and_then(|t| measure(t, 0.0, scale, request.policy)),
            AnnotationKind::Span { .. } => None,
            AnnotationKind::Text {
                text, box_style, ..
            } => measure(
                text,
                box_style.as_ref().map_or(0.0, |b| b.padding),
                scale,
                request.policy,
            ),
            AnnotationKind::Arrow { text, .. } => text
                .as_ref()
                .and_then(|t| measure(t, 0.0, scale, request.policy)),
        };
        let text_size = measured.as_ref().map(|m| m.size);
        let Some(geometry) = geometry(&item.kind, &space, text_size) else {
            map.warnings.push(RenderWarning::AnnotationSkipped {
                index,
                reason: skip_reason(&item.kind, &space, text_size),
            });
            continue;
        };
        let element = AnnotationElement {
            index,
            space,
            text_size,
            handle: None,
        };
        match (&item.kind, geometry) {
            (AnnotationKind::Span { fill, .. }, AnnotationGeometry::Span(rect)) => {
                if let Some(c) = canvas(&mut underlay, w, h) {
                    let (x, y) = px([rect.x, rect.y]);
                    c.draw_rect(
                        x,
                        y,
                        rect.width * scale,
                        rect.height * scale,
                        &Paint::fill(fill),
                    );
                }
            }
            (
                AnnotationKind::ReferenceLine { style, label, .. },
                AnnotationGeometry::Line { a, b },
            ) => {
                if let Some(c) = canvas(&mut overlay, w, h) {
                    if style.width > 0.0 && style.color.a > 0.0 {
                        c.draw_line(
                            px(a),
                            px(b),
                            &Paint::stroke(&style.color, style.width * scale)
                                .with_dash(&dash(&style.style, scale)),
                        );
                    }
                    if let (Some(label), Some(m)) = (label, &measured) {
                        let rect = line_label_rect(a, b, m.size);
                        let (x, y) = px([rect.x, rect.y]);
                        draw_rich_text(
                            c,
                            &scaled_rich_text(label, scale),
                            (x, y + m.ascent),
                            request.policy,
                        );
                    }
                }
            }
            (
                AnnotationKind::Text {
                    text, box_style, ..
                },
                AnnotationGeometry::Text(rect),
            ) => {
                if let (Some(c), Some(m)) = (canvas(&mut overlay, w, h), &measured) {
                    let (x, y) = px([rect.x, rect.y]);
                    if let Some(b) = box_style {
                        let (pw, ph) = (rect.width * scale, rect.height * scale);
                        if b.fill.a > 0.0 {
                            c.draw_rect(x, y, pw, ph, &Paint::fill(&b.fill));
                        }
                        if b.border_width > 0.0 && b.border_color.a > 0.0 {
                            let bw = b.border_width * scale;
                            c.draw_rect(
                                x + bw * 0.5,
                                y + bw * 0.5,
                                (pw - bw).max(0.0),
                                (ph - bw).max(0.0),
                                &Paint::stroke(&b.border_color, bw),
                            );
                        }
                    }
                    let pad = m.padding * scale;
                    draw_rich_text(
                        c,
                        &scaled_rich_text(text, scale),
                        (x + pad, y + pad + m.ascent),
                        request.policy,
                    );
                }
            }
            (AnnotationKind::Arrow { style, text, .. }, AnnotationGeometry::Arrow { from, to }) => {
                if let Some(c) = canvas(&mut overlay, w, h) {
                    let (p0, p1) = (px(from), px(to));
                    let d = (p1.0 - p0.0, p1.1 - p0.1);
                    let len = (d.0 * d.0 + d.1 * d.1).sqrt();
                    let head_len = (style.head_length * scale).min(len);
                    let dir = if len > 0.0 {
                        (d.0 / len, d.1 / len)
                    } else {
                        (1.0, 0.0)
                    };
                    let base = (p1.0 - dir.0 * head_len, p1.1 - dir.1 * head_len);
                    if style.line.width > 0.0 && style.line.color.a > 0.0 && len > head_len {
                        c.draw_line(
                            p0,
                            base,
                            &Paint::stroke(&style.line.color, style.line.width * scale)
                                .with_dash(&dash(&style.line.style, scale)),
                        );
                    }
                    if head_len > 0.0 && style.head_width > 0.0 && style.line.color.a > 0.0 {
                        let half = style.head_width * scale * 0.5;
                        let n = (-dir.1 * half, dir.0 * half);
                        c.fill_polygon(
                            &[
                                p1,
                                (base.0 + n.0, base.1 + n.1),
                                (base.0 - n.0, base.1 - n.1),
                            ],
                            &Paint::fill(&style.line.color),
                        );
                    }
                    if let (Some(text), Some(m)) = (text, &measured) {
                        let rect = arrow_label_rect(from, to, m.size);
                        let (x, y) = px([rect.x, rect.y]);
                        draw_rich_text(
                            c,
                            &scaled_rich_text(text, scale),
                            (x, y + m.ascent),
                            request.policy,
                        );
                    }
                }
            }
            _ => unreachable!("geometry kind follows the item kind"),
        }
        registered.push(element);
    }
    for element in registered {
        let id = map.map.register(element.clone());
        map.entries.push((id, element));
    }
    if let Some(index) = request.selected
        && let Some(id) = map.find(index)
        && let Some(mut selection) = map.selection_box(id)
        && let Some(c) = canvas(&mut overlay, w, h)
    {
        let shift = |r: RectF| RectF {
            x: (r.x - origin[0]) * scale,
            y: (r.y - origin[1]) * scale,
            width: r.width * scale,
            height: r.height * scale,
        };
        selection.rect = shift(selection.rect);
        selection.stroke_width *= scale;
        for handle in &mut selection.handles {
            *handle = shift(*handle);
        }
        crate::axis_render::draw_selection_boxes(c, &[selection]);
    }
    AnnotationRaster {
        underlay: underlay.map(Canvas::into_rgba),
        overlay: overlay.map(Canvas::into_rgba),
        map,
    }
}

/// The selected annotation index named by a bounded chart's decoration
/// selection.
pub(crate) fn selected_index(selected: Option<&DecorationId>) -> Option<usize> {
    AnnotationLayer::selected_index(selected)
}

// GPU side shared by the bounded renderers.

/// One uploaded annotation raster and its bind group.
pub(crate) struct LayerTexture {
    pub texture: TrackedTexture,
    pub bind_group: wgpu::BindGroup,
}

/// How a bounded frame treats its background.
///
/// `Opaque` (default) draws the chart's `background` colour inside the
/// frame's single draw, as every bounded chart always has. `HostClear`
/// leaves the background to the host's clear colour, which lets an
/// annotation underlay drawn before the frame show through; the host must
/// then clear with the chart's background colour. Pixels of a chart without
/// an underlay are the same under both modes up to premultiplied rounding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FrameBackground {
    #[default]
    Opaque,
    HostClear,
}

/// The fullscreen textured pipeline of one bounded renderer, created on the
/// first frame that has an annotation to draw.
pub(crate) struct AnnotationPipeline {
    pub layout: wgpu::BindGroupLayout,
    pub sampler: wgpu::Sampler,
    pub pipeline: wgpu::RenderPipeline,
}

impl AnnotationPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let layout = crate::data_render::create_texture_bind_group_layout(device);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("figgy annotation sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline =
            crate::data_render::create_fullscreen_textured_pipeline(device, &layout, format);
        Self {
            layout,
            sampler,
            pipeline,
        }
    }

    fn upload(
        &self,
        gpu: &RendererDevice,
        ledger: &Arc<GpuLedger>,
        label: &str,
        size: (u32, u32),
        rgba: &[u8],
    ) -> LayerTexture {
        let device = gpu.device();
        // gpu-alloc: PanelTexture
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let texture = TrackedTexture::new(ledger, GpuResourceKind::PanelTexture, texture);
        gpu.queue().write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.0 * 4),
                rows_per_image: Some(size.1),
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        let bind_group = crate::data_render::create_texture_bind_group(
            device,
            &self.layout,
            &texture.create_view(&Default::default()),
            &self.sampler,
        );
        LayerTexture {
            texture,
            bind_group,
        }
    }
}

/// The GPU textures of one bounded annotation raster, shared between frames
/// that differ only in the layer's `visible` flag.
pub(crate) struct AnnotationRasters {
    underlay: Option<LayerTexture>,
    overlay: Option<LayerTexture>,
    map: AnnotationMap,
}

/// The annotation layer of one bounded chart frame: the two textures and
/// their hit map. Draw [`Self::draw_underlay`] before the chart frame and
/// [`Self::draw_overlay`] after it, into the same attachment, with the same
/// size and format as the frame. Both are no-ops for a hidden layer and for
/// an empty texture.
pub struct BoundedAnnotationFrame {
    rasters: Arc<AnnotationRasters>,
    /// `None` until the renderer has drawn an annotation; a layer with no
    /// textures never needs the pipeline.
    pipeline: Option<wgpu::RenderPipeline>,
    visible: bool,
    size: (u32, u32),
}

impl BoundedAnnotationFrame {
    /// Renderer-measured annotation hit map, in chart-local logical pixels.
    pub fn annotations(&self) -> &AnnotationMap {
        &self.rasters.map
    }

    /// Items the raster skipped, as `RenderWarning::AnnotationSkipped`.
    pub fn render_warnings(&self) -> &[RenderWarning] {
        self.rasters.map.warnings()
    }

    /// Physical size of the textures, matching the chart frame.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// `true` when [`Self::draw_underlay`] draws something, so the host must
    /// render the chart frame with [`FrameBackground::HostClear`] and clear
    /// with the chart's background for the underlay to show.
    pub fn draws_underlay(&self) -> bool {
        self.visible && self.rasters.underlay.is_some()
    }

    /// `true` when [`Self::draw_overlay`] draws something.
    pub fn draws_overlay(&self) -> bool {
        self.visible && self.rasters.overlay.is_some()
    }

    /// True when the GPU rasters are shared (a `visible` toggle only).
    pub fn shares_rasters_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.rasters, &other.rasters)
    }

    pub fn draw_underlay(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.visible
            && let Some(layer) = &self.rasters.underlay
        {
            self.draw(pass, layer);
        }
    }

    pub fn draw_overlay(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.visible
            && let Some(layer) = &self.rasters.overlay
        {
            self.draw(pass, layer);
        }
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, layer: &LayerTexture) {
        let Some(pipeline) = &self.pipeline else {
            return;
        };
        pass.set_viewport(0.0, 0.0, self.size.0 as f32, self.size.1 as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, self.size.0, self.size.1);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &layer.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Everything the raster depends on besides the layer's `visible` flag.
#[derive(Clone, PartialEq)]
struct RasterKey {
    items: Vec<Annotation>,
    space: PixelSpace,
    size: (u32, u32),
    scale: f32,
    fonts: u64,
    selected: Option<usize>,
}

/// Per-renderer cache of the annotation layer for bounded charts: one GPU
/// raster per distinct input, one frame per raster and visibility.
pub(crate) struct BoundedAnnotationCache {
    format: wgpu::TextureFormat,
    pipeline: Option<AnnotationPipeline>,
    rasters: Option<(RasterKey, Arc<AnnotationRasters>)>,
    frame: Option<(RasterKey, bool, Arc<BoundedAnnotationFrame>)>,
}

impl BoundedAnnotationCache {
    pub fn new(format: wgpu::TextureFormat) -> Self {
        Self {
            format,
            pipeline: None,
            rasters: None,
            frame: None,
        }
    }

    pub fn clear(&mut self) {
        self.rasters = None;
        self.frame = None;
    }

    /// Bytes a fresh raster of `layer` at `size` would allocate, for the
    /// renderer's admission check before any GPU work.
    pub fn bytes_needed(&self, layer: &AnnotationLayer, size: (u32, u32)) -> u64 {
        let (underlay, overlay) =
            layer
                .items
                .iter()
                .filter(|i| i.visible)
                .fold((false, false), |(u, o), i| {
                    if i.is_underlay() {
                        (true, o)
                    } else {
                        (u, true)
                    }
                });
        let texture = u64::from(size.0) * u64::from(size.1) * 4;
        texture * (u64::from(underlay) + u64::from(overlay))
    }

    fn key(
        layer: &AnnotationLayer,
        space: PixelSpace,
        size: (u32, u32),
        scale: f32,
        selected: Option<usize>,
    ) -> RasterKey {
        RasterKey {
            items: layer.items.clone(),
            space,
            size,
            scale,
            fonts: crate::text_render::font_generation(),
            selected,
        }
    }

    /// Whether [`Self::prepare`] with these arguments would rasterize and
    /// upload new textures rather than reuse the cached ones.
    pub fn needs_raster(
        &self,
        layer: &AnnotationLayer,
        space: PixelSpace,
        size: (u32, u32),
        scale: f32,
        selected: Option<usize>,
    ) -> bool {
        let key = Self::key(layer, space, size, scale, selected);
        !self.rasters.as_ref().is_some_and(|(k, _)| *k == key)
    }

    /// The annotation frame for `layer` in `space`. `size` is physical; the
    /// raster is drawn at `scale` physical pixels per logical pixel.
    pub fn prepare(
        &mut self,
        gpu: &RendererDevice,
        ledger: &Arc<GpuLedger>,
        layer: &AnnotationLayer,
        space: PixelSpace,
        size: (u32, u32),
        scale: f32,
        selected: Option<usize>,
    ) -> Arc<BoundedAnnotationFrame> {
        let key = Self::key(layer, space, size, scale, selected);
        if let Some((k, visible, frame)) = &self.frame
            && *k == key
            && *visible == layer.visible
        {
            return Arc::clone(frame);
        }
        let rasters = match &self.rasters {
            Some((k, rasters)) if *k == key => Arc::clone(rasters),
            _ => {
                let raster = rasterize(&RasterRequest {
                    layer,
                    space,
                    origin: [0.0, 0.0],
                    size,
                    scale,
                    selected,
                    policy: FontPolicy::Standard,
                });
                // The fullscreen pipeline is compiled on the first frame that
                // has an annotation to draw, not by the renderer constructor.
                let mut upload = |rgba: Option<Vec<u8>>, label: &str| {
                    rgba.map(|rgba| {
                        self.pipeline
                            .get_or_insert_with(|| {
                                AnnotationPipeline::new(&gpu.device(), self.format)
                            })
                            .upload(gpu, ledger, label, size, &rgba)
                    })
                };
                let underlay = upload(raster.underlay, "annotation underlay");
                let overlay = upload(raster.overlay, "annotation overlay");
                let rasters = Arc::new(AnnotationRasters {
                    underlay,
                    overlay,
                    map: raster.map,
                });
                self.rasters = Some((key.clone(), Arc::clone(&rasters)));
                rasters
            }
        };
        let frame = Arc::new(BoundedAnnotationFrame {
            rasters,
            pipeline: self.pipeline.as_ref().map(|p| p.pipeline.clone()),
            visible: layer.visible,
            size,
        });
        self.frame = Some((key, layer.visible, Arc::clone(&frame)));
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::RichText;
    use model::Color;

    fn space() -> PixelSpace {
        PixelSpace {
            panel: RectF {
                x: 0.,
                y: 0.,
                width: 200.,
                height: 120.,
            },
            plot: RectF {
                x: 40.,
                y: 10.,
                width: 140.,
                height: 90.,
            },
            x: AxisSpace {
                map: AxisMap::Linear { min: 0., max: 10. },
                inverted: false,
            },
            y: AxisSpace {
                map: AxisMap::Linear { min: 0., max: 1. },
                inverted: false,
            },
        }
    }

    fn pixel(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    #[test]
    fn raster_splits_underlay_and_overlay_and_reports_skips() {
        let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
            .lock()
            .unwrap();
        let layer = AnnotationLayer {
            visible: true,
            items: vec![
                Annotation::new(AnnotationKind::Span {
                    axis: AnnotationAxis::X,
                    from: 2.,
                    to: 4.,
                    fill: Color::from_rgba(1., 0., 0., 0.5),
                }),
                Annotation::new(AnnotationKind::ReferenceLine {
                    axis: AnnotationAxis::Y,
                    value: 0.5,
                    style: AnnotationLineStyle {
                        color: Color::BLACK,
                        width: 2.,
                        style: Default::default(),
                    },
                    label: Some(RichText::plain("half", Color::BLACK, 10., "")),
                }),
                Annotation::new(AnnotationKind::ReferenceLine {
                    axis: AnnotationAxis::X,
                    value: f64::NAN,
                    style: Default::default(),
                    label: None,
                }),
                Annotation::new(AnnotationKind::Text {
                    anchor: Anchor::Ratio { x: 0.1, y: 0.1 },
                    text: RichText::plain("", Color::BLACK, 10., ""),
                    box_style: None,
                }),
                Annotation::new(AnnotationKind::Arrow {
                    from: Anchor::Data { x: 1., y: 0.1 },
                    to: Anchor::Data { x: 9., y: 0.9 },
                    style: Default::default(),
                    text: None,
                }),
                Annotation {
                    visible: false,
                    ..Annotation::new(AnnotationKind::Text {
                        anchor: Anchor::Ratio { x: 0.1, y: 0.1 },
                        text: RichText::plain("hidden", Color::BLACK, 10., ""),
                        box_style: None,
                    })
                },
            ],
        };
        for scale in [1.0f32, 2.0] {
            let (w, h) = ((200. * scale) as u32, (120. * scale) as u32);
            let raster = rasterize(&RasterRequest {
                layer: &layer,
                space: space(),
                origin: [0., 0.],
                size: (w, h),
                scale,
                selected: Some(1),
                policy: FontPolicy::Standard,
            });
            let underlay = raster.underlay.expect("span draws an underlay");
            let overlay = raster.overlay.expect("line and arrow draw an overlay");
            // The span covers x in 2..4 -> 68..96 logical px, every plot row.
            let inside = pixel(&underlay, w, (80. * scale) as u32, (50. * scale) as u32);
            assert_eq!(inside, [128, 0, 0, 128], "{scale}");
            let outside = pixel(&underlay, w, (120. * scale) as u32, (50. * scale) as u32);
            assert_eq!(outside, [0, 0, 0, 0]);
            // The span never reaches the overlay; the line sits at y = 55.
            assert_eq!(
                pixel(&overlay, w, (80. * scale) as u32, (30. * scale) as u32),
                [0; 4]
            );
            let on_line = pixel(&overlay, w, (80. * scale) as u32, (55. * scale) as u32);
            assert!(on_line[3] > 200, "{scale}: {on_line:?}");
            assert_eq!(
                raster.map.warnings(),
                &[
                    RenderWarning::AnnotationSkipped {
                        index: 2,
                        reason: AnnotationSkip::NonFinite
                    },
                    RenderWarning::AnnotationSkipped {
                        index: 3,
                        reason: AnnotationSkip::EmptyText
                    }
                ]
            );
            let drawn: Vec<usize> = raster.map.entries().map(|(_, e)| e.index).collect();
            assert_eq!(drawn, [0, 1, 4]);
            // Later items win where they overlap: the arrow's box covers the
            // middle of the plot, so probe the line and the span beside it.
            let hit = raster.map.hit_test([175., 55.]).expect("line is hit");
            assert_eq!(raster.map.element(hit).unwrap().index, 1);
            assert_eq!(
                raster
                    .map
                    .hit_test([70., 98.])
                    .map(|id| raster.map.element(id).unwrap().index),
                Some(0)
            );
            assert_eq!(
                raster
                    .map
                    .hit_test([80., 55.])
                    .map(|id| raster.map.element(id).unwrap().index),
                Some(4)
            );
            assert_eq!(raster.map.hit_test([5., 5.]), None);
            let selection = raster.map.selection_box(hit).unwrap();
            assert_eq!(selection.handles.len(), 2);
        }
    }

    #[test]
    fn radial_space_rejects_data_anchors_with_a_warning() {
        let _fonts = crate::text_render::FONT_REGISTRATION_TEST_LOCK
            .lock()
            .unwrap();
        let mut s = space();
        s.x = AxisSpace::NONE;
        s.y = AxisSpace::NONE;
        let layer = AnnotationLayer {
            visible: true,
            items: vec![
                Annotation::new(AnnotationKind::Text {
                    anchor: Anchor::Data { x: 1., y: 1. },
                    text: RichText::plain("data", Color::BLACK, 10., ""),
                    box_style: None,
                }),
                Annotation::new(AnnotationKind::Text {
                    anchor: Anchor::Ratio { x: 0.2, y: 0.2 },
                    text: RichText::plain("ratio", Color::BLACK, 10., ""),
                    box_style: Some(BoxStyle::default()),
                }),
                Annotation::new(AnnotationKind::Span {
                    axis: AnnotationAxis::X,
                    from: 0.,
                    to: 1.,
                    fill: Color::BLACK,
                }),
            ],
        };
        let raster = rasterize(&RasterRequest {
            layer: &layer,
            space: s,
            origin: [0., 0.],
            size: (200, 120),
            scale: 1.,
            selected: None,
            policy: FontPolicy::Standard,
        });
        assert!(raster.underlay.is_none());
        assert!(raster.overlay.is_some());
        assert_eq!(
            raster.map.warnings(),
            &[
                RenderWarning::AnnotationSkipped {
                    index: 0,
                    reason: AnnotationSkip::NoValueAxis
                },
                RenderWarning::AnnotationSkipped {
                    index: 2,
                    reason: AnnotationSkip::NoValueAxis
                }
            ]
        );
        assert_eq!(raster.map.entries().count(), 1);
    }

    #[test]
    fn cartesian_space_follows_the_config_axes() {
        let mut config = crate::default::default_config();
        config.bottom_x.min = -1.;
        config.bottom_x.max = 3.;
        config.left_y.scale = AxisScale::Logarithmic;
        config.left_y.min = 1.;
        config.left_y.max = 100.;
        config.left_y.inverted = true;
        let s = pixel_space_for_config(&config);
        let da = RectF::from_rect(&config.data_area().unwrap().0);
        assert_eq!(s.plot, da);
        assert_eq!(s.panel, RectF::from_rect(&config.chart_area.0));
        assert_eq!(
            s.value_to_px(AnnotationAxis::X, 1.),
            Some(da.x + da.width * 0.5)
        );
        // Inverted log axis: 10 is halfway, 0 has no position.
        let mid = s.value_to_px(AnnotationAxis::Y, 10.).unwrap();
        assert!((mid - (da.y + da.height * 0.5)).abs() < 1e-3);
        assert_eq!(s.value_to_px(AnnotationAxis::Y, 0.), None);
        assert_eq!(s.value_axes(), [true, true]);
    }
}
