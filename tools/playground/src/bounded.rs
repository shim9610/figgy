//! Bounded scenes: radial, categorical bars, box plots and radar charts.
//!
//! Each family has its own small renderer that prepares a frame of an exact
//! physical size. The host renders that frame (annotation underlay → chart →
//! annotation overlay) into an offscreen texture and egui shows it as an image.
//! One generic host covers all four; `Family` adapts the per-family names.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use renderer::HitId;
use renderer::annotation::BoundedAnnotationFrame;
use renderer::boxplot::*;
use renderer::categorical::*;
use renderer::decoration::DecorationMap;
use renderer::layout::NudgeResult;
use renderer::radar::*;
use renderer::radial::*;
use renderer::text::RichText;
use renderer::{
    Anchor, Annotation, AnnotationAxis, AnnotationKind, AnnotationLayer, Color, DecorationId,
    DecorationLayout, FrameBackground, LayerHit, LayerRef, Material, RendererDevice, ResizeHandle,
};

use crate::HostCtx;
use crate::scenes::SceneKind;
use crate::scenes::bounded::{box_chart, categorical_chart, radar_chart, radial_chart};
use crate::widgets::{choice, color, section, slider};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Object-safe face of a bounded scene, used by the host loop.
pub trait BoundedScene {
    fn controls(&mut self, ui: &mut egui::Ui, host: &mut HostCtx<'_>);
    fn viewport(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
        host: &mut HostCtx<'_>,
    );
    fn release(&mut self, egui_renderer: &mut egui_wgpu::Renderer);
}

/// Per-family adapter: the four renderers share a shape but not a trait.
pub trait Family: 'static {
    type Chart: Clone;
    type Renderer;
    type Frame;
    type Target: Clone + PartialEq + std::fmt::Debug;
    type Error: std::fmt::Display;

    fn renderer(gpu: RendererDevice) -> Result<Self::Renderer, Self::Error>;
    fn set_background(r: &mut Self::Renderer, b: FrameBackground);
    fn prepare(
        r: &mut Self::Renderer,
        c: &Self::Chart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<Self::Frame>, Self::Error>;
    fn annotations(
        r: &mut Self::Renderer,
        c: &Self::Chart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<BoundedAnnotationFrame>, Self::Error>;
    fn end_frame(r: &Self::Renderer);
    fn export(
        r: &mut Self::Renderer,
        c: &Self::Chart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<renderer::RasterImage, Self::Error>;

    fn frame_size(f: &Self::Frame) -> (u32, u32);
    fn draw(f: &Self::Frame, pass: &mut wgpu::RenderPass<'_>);
    /// The data under `p`: answered at once, or by a GPU pick the host
    /// polls on later frames (the radial chart).
    fn pick(f: &Arc<Self::Frame>, p: [f32; 2]) -> Pick<Self::Target>;
    /// The frame's `DecorationMap`; `None` for a chart whose decorations are
    /// a `HitMap` of the chart (the radial chart), which overrides the
    /// decoration methods below.
    fn decoration_map(f: &Self::Frame) -> Option<&DecorationMap>;
    fn decoration_at(f: &Self::Frame, _c: &Self::Chart, p: [f32; 2]) -> Option<HitId> {
        Self::decoration_map(f)?.hit_test(p)
    }
    fn decoration_find(f: &Self::Frame, _c: &Self::Chart, id: &DecorationId) -> Option<HitId> {
        let map = Self::decoration_map(f)?;
        map.entries().find(|(_, e)| &e.id == id).map(|(hit, _)| hit)
    }
    fn decoration_key(f: &Self::Frame, _c: &Self::Chart, hit: HitId) -> Option<DecorationId> {
        Self::decoration_map(f)?.element(hit).map(|e| e.id.clone())
    }
    fn decoration_handle(
        f: &Self::Frame,
        _c: &Self::Chart,
        hit: HitId,
        p: [f32; 2],
    ) -> Option<ResizeHandle> {
        Self::decoration_map(f)?.hit_resize_handle(hit, p)
    }
    fn move_decoration(
        f: &Self::Frame,
        c: &mut Self::Chart,
        hit: HitId,
        handle: Option<ResizeHandle>,
        dx: f32,
        dy: f32,
    ) -> Option<NudgeResult> {
        let map = Self::decoration_map(f)?;
        let layout = Self::layout(c);
        match handle {
            Some(h) => map.resize_by(layout, hit, h, dx, dy),
            None => map.drag_by(layout, hit, dx, dy),
        }
    }

    fn background(c: &Self::Chart) -> Color;
    fn layout(c: &mut Self::Chart) -> &mut DecorationLayout;
    fn annotation_layer(c: &mut Self::Chart) -> &mut AnnotationLayer;
    fn active_layer(c: &Self::Chart) -> LayerRef;
    fn set_active_layer(c: &mut Self::Chart, l: LayerRef);
    fn selected(c: &Self::Chart) -> Option<Self::Target>;
    fn set_selected(c: &mut Self::Chart, t: Option<Self::Target>);
    /// Returns `true` while a hover animation still runs.
    fn hover(c: &mut Self::Chart, t: Option<Self::Target>, dt: f32) -> bool;
    fn inspector(ui: &mut egui::Ui, c: &mut Self::Chart);
}

/// A data pick: ready, or a GPU readback polled once per frame.
pub enum Pick<T> {
    Ready(Option<T>),
    Pending(Pin<Box<dyn Future<Output = Option<T>>>>),
}
impl<T> Pick<T> {
    /// The picked target once the answer is in; `None` while it is pending.
    fn poll(&mut self) -> Option<Option<T>> {
        match self {
            Self::Ready(t) => Some(t.take()),
            Self::Pending(f) => match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
                Poll::Ready(t) => Some(t),
                Poll::Pending => None,
            },
        }
    }
}

/// What a press waits for its data pick to decide.
enum Press {
    /// Select the data under the press, or nothing.
    Data,
    /// The plot area was hit: data under it wins until the area is selected.
    AreaOrData,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    None,
    Decoration(Option<ResizeHandle>),
    Annotation(Option<ResizeHandle>),
}

pub struct Host<F: Family> {
    name: &'static str,
    renderer: F::Renderer,
    chart: F::Chart,
    initial: F::Chart,
    frame: Option<Arc<F::Frame>>,
    layer: Option<Arc<BoundedAnnotationFrame>>,
    target: Option<(wgpu::Texture, egui::TextureId)>,
    drag: Drag,
    /// The press waiting for its data pick.
    press: Option<(Press, Pick<F::Target>)>,
    /// The hover pick in flight, and the last answered hover.
    hover_pick: Option<Pick<F::Target>>,
    hovered: Option<F::Target>,
    size: (u32, u32),
    error: String,
}

pub fn create(kind: SceneKind, gpu: RendererDevice) -> Result<Box<dyn BoundedScene>, String> {
    fn make<F: Family>(
        name: &'static str,
        gpu: RendererDevice,
        chart: F::Chart,
    ) -> Result<Box<dyn BoundedScene>, String> {
        let mut renderer = F::renderer(gpu).map_err(|e| e.to_string())?;
        // Leave the background to our clear so annotation spans show under the data.
        F::set_background(&mut renderer, FrameBackground::HostClear);
        Ok(Box::new(Host::<F> {
            name,
            renderer,
            initial: chart.clone(),
            chart,
            frame: None,
            layer: None,
            target: None,
            drag: Drag::None,
            press: None,
            hover_pick: None,
            hovered: None,
            size: (1, 1),
            error: String::new(),
        }))
    }
    match kind {
        SceneKind::Radial => make::<RadialFamily>("radial", gpu, radial_chart()),
        SceneKind::Categorical => {
            make::<CategoricalFamily>("categorical", gpu, categorical_chart())
        }
        SceneKind::BoxPlot => make::<BoxFamily>("boxplot", gpu, box_chart()),
        SceneKind::Radar => make::<RadarFamily>("radar", gpu, radar_chart()),
        _ => Err("not a bounded scene".into()),
    }
}

impl<F: Family> Host<F> {
    /// Prepare the frame + annotation layer and render them into the texture.
    fn render(&mut self, host: &mut HostCtx<'_>, scale: f32) -> Result<(), String> {
        let frame = F::prepare(&mut self.renderer, &self.chart, self.size, scale)
            .map_err(|e| e.to_string())?;
        let layer = F::annotations(&mut self.renderer, &self.chart, self.size, scale)
            .map_err(|e| e.to_string())?;
        let (w, h) = F::frame_size(&frame);
        if self
            .target
            .as_ref()
            .is_none_or(|(t, _)| t.width() != w || t.height() != h)
        {
            if let Some((_, id)) = self.target.take() {
                host.egui_renderer.free_texture(&id);
            }
            let texture = host.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("bounded scene"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let id = host.egui_renderer.register_native_texture(
                host.device,
                &texture.create_view(&Default::default()),
                wgpu::FilterMode::Nearest,
            );
            self.target = Some((texture, id));
        }
        let view = self
            .target
            .as_ref()
            .unwrap()
            .0
            .create_view(&Default::default());
        let bg = F::background(&self.chart);
        let mut encoder = host.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: bg.r as f64,
                            g: bg.g as f64,
                            b: bg.b as f64,
                            a: bg.a as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            layer.draw_underlay(&mut pass);
            F::draw(&frame, &mut pass);
            layer.draw_overlay(&mut pass);
        }
        host.queue.submit([encoder.finish()]);
        self.frame = Some(frame);
        self.layer = Some(layer);
        F::end_frame(&self.renderer);
        Ok(())
    }

    fn on_press(&mut self, p: [f32; 2]) {
        let (Some(frame), Some(layer)) = (self.frame.clone(), self.layer.clone()) else {
            return;
        };
        let active = F::active_layer(&self.chart);
        // The selected annotation's own handles win first.
        if let Some(DecorationId::Annotation(index)) = F::layout(&mut self.chart).selected.clone()
            && let Some(id) = layer.annotations().find(index)
            && let Some(h) = layer.annotations().hit_resize_handle(id, p)
        {
            self.drag = Drag::Annotation(Some(h));
            return;
        }
        // Then the selected decoration's resize handles.
        if let Some(selected) = F::layout(&mut self.chart).selected.clone()
            && let Some(id) = F::decoration_find(&frame, &self.chart, &selected)
            && let Some(h) = F::decoration_handle(&frame, &self.chart, id, p)
        {
            self.drag = Drag::Decoration(Some(h));
            return;
        }
        match layer.route_active_layer(active, p, || F::decoration_at(&frame, &self.chart, p)) {
            Some(LayerHit::Annotation { index, .. }) => {
                F::layout(&mut self.chart).selected = Some(DecorationId::Annotation(index));
                self.drag = Drag::Annotation(None);
                return;
            }
            Some(LayerHit::Element(id)) => {
                let key = F::decoration_key(&frame, &self.chart, id);
                // Data wins over the plot area until the area itself is selected.
                let area_passthrough = key == Some(DecorationId::DataArea)
                    && F::layout(&mut self.chart).selected != key;
                if !area_passthrough {
                    F::layout(&mut self.chart).selected = key;
                    F::set_selected(&mut self.chart, None);
                    self.drag = Drag::Decoration(None);
                    return;
                }
                self.press = Some((Press::AreaOrData, F::pick(&frame, p)));
                self.drag = Drag::None;
                return;
            }
            None => {}
        }
        self.press = Some((Press::Data, F::pick(&frame, p)));
        self.drag = Drag::None;
    }

    /// Apply the press once its data pick has answered.
    fn poll_press(&mut self) {
        let Some((press, pick)) = &mut self.press else {
            return;
        };
        let Some(hit) = pick.poll() else { return };
        let area = matches!(press, Press::AreaOrData) && hit.is_none();
        self.press = None;
        if area {
            F::layout(&mut self.chart).selected = Some(DecorationId::DataArea);
            F::set_selected(&mut self.chart, None);
            self.drag = Drag::Decoration(None);
        } else {
            F::layout(&mut self.chart).selected = None;
            F::set_selected(&mut self.chart, hit);
        }
    }

    fn on_move(&mut self, dx: f32, dy: f32, scale: f32) {
        let (Some(frame), Some(layer)) = (self.frame.clone(), self.layer.clone()) else {
            return;
        };
        let Some(selected) = F::layout(&mut self.chart).selected.clone() else {
            return;
        };
        let previous = self.chart.clone();
        let moved = match self.drag {
            Drag::None => return,
            Drag::Annotation(handle) => {
                let DecorationId::Annotation(index) = selected else {
                    return;
                };
                let map = layer.annotations();
                let Some(id) = map.find(index) else { return };
                let items = F::annotation_layer(&mut self.chart);
                match handle {
                    Some(h) => map.resize_by(items, id, h, dx, dy),
                    None => map.drag_by(items, id, dx, dy),
                }
            }
            Drag::Decoration(handle) => {
                let Some(id) = F::decoration_find(&frame, &self.chart, &selected) else {
                    return;
                };
                F::move_decoration(&frame, &mut self.chart, id, handle, dx, dy)
            }
        };
        // Keep the last feasible model when the new layout no longer fits.
        if moved == Some(NudgeResult::Moved)
            && F::prepare(&mut self.renderer, &self.chart, self.size, scale).is_err()
        {
            self.chart = previous;
        }
    }
}

impl<F: Family> BoundedScene for Host<F> {
    fn controls(&mut self, ui: &mut egui::Ui, host: &mut HostCtx<'_>) {
        section(ui, "Chart", true, |ui| {
            F::inspector(ui, &mut self.chart);
        });
        section(ui, "Selection & layers", true, |ui| {
            ui.label(format!("Data: {:?}", F::selected(&self.chart)));
            ui.label(format!(
                "Decoration: {:?}",
                F::layout(&mut self.chart).selected
            ));
            let mut layer = F::active_layer(&self.chart);
            if choice(
                ui,
                "Active layer",
                &mut layer,
                &[LayerRef::Base, LayerRef::Annotations],
            ) {
                F::set_active_layer(&mut self.chart, layer);
            }
            ui.horizontal(|ui| {
                let editing = F::layout(&mut self.chart).selected == Some(DecorationId::DataArea);
                if ui.selectable_label(editing, "Edit plot area").clicked() {
                    F::layout(&mut self.chart).selected =
                        (!editing).then_some(DecorationId::DataArea);
                    F::set_selected(&mut self.chart, None);
                }
                if let Some(sel) = F::layout(&mut self.chart).selected.clone()
                    && ui.button("Reset position").clicked()
                {
                    F::layout(&mut self.chart).reset(&sel);
                }
                if ui.button("Reset scene").clicked() {
                    self.chart = self.initial.clone();
                }
            });
        });
        section(ui, "Annotations", false, |ui| {
            let accent = Color::from_rgb8(200, 40, 120);
            let add = |chart: &mut F::Chart, kind| {
                F::annotation_layer(chart).items.push(Annotation::new(kind));
                F::set_active_layer(chart, LayerRef::Annotations);
            };
            ui.horizontal_wrapped(|ui| {
                if ui.button("+ Text").clicked() {
                    add(
                        &mut self.chart,
                        AnnotationKind::Text {
                            anchor: Anchor::Ratio { x: 0.5, y: 0.5 },
                            text: RichText::plain("note", Color::BLACK, 15.0, ""),
                            box_style: Some(Default::default()),
                        },
                    );
                }
                if ui.button("+ Arrow").clicked() {
                    add(
                        &mut self.chart,
                        AnnotationKind::Arrow {
                            from: Anchor::Ratio { x: 0.2, y: 0.2 },
                            to: Anchor::Ratio { x: 0.45, y: 0.45 },
                            style: Default::default(),
                            text: None,
                        },
                    );
                }
                if ui.button("+ Span").clicked() {
                    add(
                        &mut self.chart,
                        AnnotationKind::Span {
                            axis: AnnotationAxis::X,
                            from: -0.5,
                            to: 0.5,
                            fill: Color::from_rgba(1.0, 0.85, 0.2, 0.35),
                        },
                    );
                }
                if ui.button("+ Ref line").clicked() {
                    add(
                        &mut self.chart,
                        AnnotationKind::ReferenceLine {
                            axis: AnnotationAxis::Y,
                            value: 20.0,
                            style: renderer::AnnotationLineStyle {
                                color: accent,
                                width: 1.5,
                                style: renderer::line::LineStylePreset::Dash,
                            },
                            label: Some(RichText::plain("target", accent, 13.0, "")),
                        },
                    );
                }
            });
            let layer = F::annotation_layer(&mut self.chart);
            ui.checkbox(&mut layer.visible, "Visible");
            ui.label(format!("{} items", layer.items.len()));
            if let Some(l) = &self.layer {
                for w in l.render_warnings() {
                    ui.colored_label(egui::Color32::from_rgb(170, 110, 0), w.to_string());
                }
            }
        });
        section(ui, "Export", false, |ui| {
            if ui.button("Export PNG (2x)").clicked() {
                let result = F::export(&mut self.renderer, &self.chart, self.size, 2.0)
                    .map_err(|e| e.to_string())
                    .and_then(|img| renderer::encode_png(&img).map_err(|e| e.to_string()))
                    .and_then(|bytes| {
                        let dir = std::path::Path::new("target/playground");
                        let path = dir.join(format!("{}.png", self.name));
                        std::fs::create_dir_all(dir)
                            .and_then(|_| std::fs::write(&path, &bytes))
                            .map(|_| path.display().to_string())
                            .map_err(|e| e.to_string())
                    });
                *host.status = match result {
                    Ok(path) => format!("Saved {path}"),
                    Err(e) => format!("Export failed: {e}"),
                };
            }
        });
        if !self.error.is_empty() {
            ui.colored_label(egui::Color32::RED, &self.error);
        }
    }

    fn viewport(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
        host: &mut HostCtx<'_>,
    ) {
        if rect.width() < 160.0 || rect.height() < 160.0 {
            return;
        }
        let scale = host.pixels_per_point;
        self.size = (rect.width().floor() as u32, rect.height().floor() as u32);
        // Prepare first so picking uses this size's geometry.
        if let Err(e) = self.render(host, scale) {
            self.error = e;
        }
        let (pressed, down, released, delta, pos, origin, dt) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.delta(),
                i.pointer.hover_pos(),
                i.pointer.press_origin(),
                i.stable_dt.min(0.1),
            )
        });
        let local = |p: egui::Pos2| [p.x - rect.left(), p.y - rect.top()];
        // Picks requested on earlier frames answer first.
        self.poll_press();
        if let Some(hit) = self.hover_pick.as_mut().and_then(Pick::poll) {
            self.hover_pick = None;
            self.hovered = hit;
        }
        if pressed && let Some(o) = origin.filter(|o| rect.contains(*o)) {
            self.on_press(local(o));
        }
        if down && (delta.x != 0.0 || delta.y != 0.0) {
            self.on_move(delta.x, delta.y, scale);
        }
        if released {
            self.drag = Drag::None;
        }
        match (
            pos.filter(|p| rect.contains(*p) && self.drag == Drag::None),
            &self.frame,
        ) {
            (Some(p), Some(frame)) => {
                if self.hover_pick.is_none() {
                    self.hover_pick = Some(F::pick(frame, local(p)));
                }
            }
            _ => {
                self.hover_pick = None;
                self.hovered = None;
            }
        }
        if self.press.is_some() || self.hover_pick.is_some() {
            ui.ctx().request_repaint();
        }
        let hovered = self.hovered.clone();
        if F::hover(&mut self.chart, hovered.clone(), dt) {
            ui.ctx().request_repaint();
        }
        if let Some(t) = hovered {
            response.clone().on_hover_text_at_pointer(format!("{t:?}"));
        }
        match self.render(host, scale) {
            Ok(()) => self.error.clear(),
            Err(e) => self.error = e,
        }
        if let Some((_, id)) = &self.target {
            ui.painter().image(
                *id,
                egui::Rect::from_min_size(
                    rect.min,
                    egui::vec2(self.size.0 as f32, self.size.1 as f32),
                ),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }

    fn release(&mut self, egui_renderer: &mut egui_wgpu::Renderer) {
        if let Some((_, id)) = self.target.take() {
            egui_renderer.free_texture(&id);
        }
        self.frame = None;
        self.layer = None;
        self.press = None;
        self.hover_pick = None;
        F::end_frame(&self.renderer);
    }
}

// ---------------------------------------------------------------------------
// Radial
// ---------------------------------------------------------------------------

pub struct RadialFamily;

const RADIAL_MATERIALS: &[Material] = &[
    Material::Flat,
    Material::Matte,
    Material::Ceramic,
    Material::SatinMetal,
    Material::Toon,
    Material::Enamel,
    Material::BrushedMetal,
    Material::Paper,
    Material::Wood,
    Material::Hatch,
    Material::Pearl,
];
// Every material but Toon and Pearl, which bars and boxes reject.
const FLAT_BODY_MATERIALS: &[Material] = &[
    Material::Flat,
    Material::Matte,
    Material::Ceramic,
    Material::SatinMetal,
    Material::Enamel,
    Material::BrushedMetal,
    Material::Paper,
    Material::Wood,
    Material::Hatch,
];
const RADIAL_FORMATS: &[RadialLabelFormat] = &[
    RadialLabelFormat::Name,
    RadialLabelFormat::Value,
    RadialLabelFormat::Percent,
    RadialLabelFormat::NamePercent,
    RadialLabelFormat::NameValue,
    RadialLabelFormat::NameValuePercent,
];

impl Family for RadialFamily {
    type Chart = RadialChart;
    type Renderer = RadialRenderer;
    type Frame = RadialFrame;
    type Target = RadialTarget;
    type Error = RadialError;

    fn renderer(gpu: RendererDevice) -> Result<RadialRenderer, RadialError> {
        RadialRenderer::new(gpu, FORMAT)
    }
    fn set_background(r: &mut RadialRenderer, b: FrameBackground) {
        r.set_background(b);
    }
    fn prepare(
        r: &mut RadialRenderer,
        c: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<RadialFrame>, RadialError> {
        r.prepare(c, size, scale)
    }
    fn annotations(
        r: &mut RadialRenderer,
        c: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<BoundedAnnotationFrame>, RadialError> {
        r.prepare_annotations(c, size, scale)
    }
    fn end_frame(r: &RadialRenderer) {
        r.end_frame();
    }
    fn export(
        r: &mut RadialRenderer,
        c: &RadialChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<renderer::RasterImage, RadialError> {
        r.export_rgba(c, size, scale)
    }
    fn frame_size(f: &RadialFrame) -> (u32, u32) {
        f.size()
    }
    fn draw(f: &RadialFrame, pass: &mut wgpu::RenderPass<'_>) {
        f.draw(pass);
    }
    fn pick(f: &Arc<RadialFrame>, p: [f32; 2]) -> Pick<RadialTarget> {
        match f.pick(p) {
            Ok(ticket) => Pick::Pending(Box::pin(async move { ticket.resolve().await.ok()? })),
            Err(_) => Pick::Ready(None),
        }
    }
    fn decoration_map(_: &RadialFrame) -> Option<&DecorationMap> {
        None
    }
    fn decoration_at(f: &RadialFrame, c: &RadialChart, p: [f32; 2]) -> Option<HitId> {
        f.decoration_at(c, p)
    }
    fn decoration_find(f: &RadialFrame, _: &RadialChart, id: &DecorationId) -> Option<HitId> {
        f.decoration(id)
    }
    fn decoration_key(f: &RadialFrame, c: &RadialChart, hit: HitId) -> Option<DecorationId> {
        f.decoration_id(c, hit)
    }
    fn decoration_handle(
        f: &RadialFrame,
        c: &RadialChart,
        hit: HitId,
        p: [f32; 2],
    ) -> Option<ResizeHandle> {
        f.decoration_resize_handle(c, hit, p)
    }
    fn move_decoration(
        f: &RadialFrame,
        c: &mut RadialChart,
        hit: HitId,
        handle: Option<ResizeHandle>,
        dx: f32,
        dy: f32,
    ) -> Option<NudgeResult> {
        match handle {
            Some(h) => f.resize_decoration(c, hit, h, dx, dy),
            None => f.drag_decoration(c, hit, dx, dy),
        }
    }
    fn background(c: &RadialChart) -> Color {
        c.background
    }
    fn layout(c: &mut RadialChart) -> &mut DecorationLayout {
        &mut c.decorations
    }
    fn annotation_layer(c: &mut RadialChart) -> &mut AnnotationLayer {
        &mut c.annotations
    }
    fn active_layer(c: &RadialChart) -> LayerRef {
        c.active_layer()
    }
    fn set_active_layer(c: &mut RadialChart, l: LayerRef) {
        c.set_active_layer(l);
    }
    fn selected(c: &RadialChart) -> Option<RadialTarget> {
        c.interaction.selected
    }
    fn set_selected(c: &mut RadialChart, t: Option<RadialTarget>) {
        c.interaction.selected = t;
    }
    fn hover(c: &mut RadialChart, t: Option<RadialTarget>, dt: f32) -> bool {
        c.interaction.animate_hover(t, dt, 0.16)
    }
    fn inspector(ui: &mut egui::Ui, c: &mut RadialChart) {
        ui.horizontal(|ui| {
            ui.label("Title");
            ui.text_edit_singleline(&mut c.title);
        });
        let mut donut = matches!(c.kind, RadialKind::Donut { .. });
        if ui.checkbox(&mut donut, "Donut").changed() {
            c.kind = if donut {
                RadialKind::Donut { inner_radius: 0.5 }
            } else {
                RadialKind::Pie
            };
        }
        if let RadialKind::Donut { inner_radius } = &mut c.kind {
            slider(ui, "hole", inner_radius, 0.1..=0.85);
        }
        choice(
            ui,
            "labels",
            &mut c.labels,
            &[
                RadialLabels::None,
                RadialLabels::Inside,
                RadialLabels::Outside,
            ],
        );
        choice(ui, "label content", &mut c.label_format, RADIAL_FORMATS);
        let s = &mut c.style;
        choice(ui, "material", &mut s.material, RADIAL_MATERIALS);
        slider(ui, "tilt°", &mut s.tilt_degrees, 0.0..=65.0);
        slider(ui, "depth", &mut s.depth, 0.0..=0.4);
        slider(ui, "gap°", &mut s.gap_degrees, 0.0..=8.0);
        slider(ui, "corners", &mut s.outer_corner, 0.0..=0.25);
        slider(ui, "gloss", &mut s.gloss, 0.0..=1.0);
        slider(ui, "hover lift", &mut s.hover_lift, 0.0..=0.25);
        ui.checkbox(&mut s.shadow, "shadow");
        ui.separator();
        let selected = c.interaction.selected;
        for (i, slice) in c.slices.iter_mut().enumerate() {
            ui.push_id(i, |ui| {
                let strong = selected == Some(RadialTarget::main(i));
                ui.horizontal(|ui| {
                    let mut label = egui::RichText::new(&slice.label);
                    if strong {
                        label = label.strong();
                    }
                    ui.label(label);
                    ui.add(
                        egui::DragValue::new(&mut slice.value)
                            .range(0.0..=1000.0)
                            .speed(0.5),
                    );
                });
                color(ui, "colour", &mut slice.color);
            });
        }
        if ui.button("+ slice").clicked() {
            let n = c.slices.len();
            c.slices.push(RadialSlice::new(
                format!("Slice {n}"),
                10.0,
                renderer::ColorCycle::Vivid.color(n),
            ));
        }
        // The drawn label strings are written at edit time.
        c.format_labels();
    }
}

// ---------------------------------------------------------------------------
// Categorical bars
// ---------------------------------------------------------------------------

pub struct CategoricalFamily;

impl Family for CategoricalFamily {
    type Chart = CategoricalChart;
    type Renderer = CategoricalRenderer;
    type Frame = CategoricalFrame;
    type Target = CategoryBarTarget;
    type Error = CategoricalError;

    fn renderer(gpu: RendererDevice) -> Result<CategoricalRenderer, CategoricalError> {
        CategoricalRenderer::new(gpu, FORMAT)
    }
    fn set_background(r: &mut CategoricalRenderer, b: FrameBackground) {
        r.set_background(b);
    }
    fn prepare(
        r: &mut CategoricalRenderer,
        c: &CategoricalChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<CategoricalFrame>, CategoricalError> {
        r.prepare(c, size, scale)
    }
    fn annotations(
        r: &mut CategoricalRenderer,
        c: &CategoricalChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<BoundedAnnotationFrame>, CategoricalError> {
        r.prepare_annotations(c, size, scale)
    }
    fn end_frame(r: &CategoricalRenderer) {
        r.end_frame();
    }
    fn export(
        r: &mut CategoricalRenderer,
        c: &CategoricalChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<renderer::RasterImage, CategoricalError> {
        r.export_rgba(c, size, scale)
    }
    fn frame_size(f: &CategoricalFrame) -> (u32, u32) {
        f.size()
    }
    fn draw(f: &CategoricalFrame, pass: &mut wgpu::RenderPass<'_>) {
        f.draw(pass);
    }
    fn pick(f: &Arc<CategoricalFrame>, p: [f32; 2]) -> Pick<CategoryBarTarget> {
        Pick::Ready(f.hit_test(p))
    }
    fn decoration_map(f: &CategoricalFrame) -> Option<&DecorationMap> {
        Some(f.decorations())
    }
    fn background(c: &CategoricalChart) -> Color {
        c.background
    }
    fn layout(c: &mut CategoricalChart) -> &mut DecorationLayout {
        &mut c.decorations
    }
    fn annotation_layer(c: &mut CategoricalChart) -> &mut AnnotationLayer {
        &mut c.annotations
    }
    fn active_layer(c: &CategoricalChart) -> LayerRef {
        c.active_layer()
    }
    fn set_active_layer(c: &mut CategoricalChart, l: LayerRef) {
        c.set_active_layer(l);
    }
    fn selected(c: &CategoricalChart) -> Option<CategoryBarTarget> {
        c.selected.clone()
    }
    fn set_selected(c: &mut CategoricalChart, t: Option<CategoryBarTarget>) {
        c.selected = t;
    }
    fn hover(c: &mut CategoricalChart, t: Option<CategoryBarTarget>, _dt: f32) -> bool {
        c.hovered = t;
        false
    }
    fn inspector(ui: &mut egui::Ui, c: &mut CategoricalChart) {
        ui.horizontal(|ui| {
            ui.label("Title");
            ui.text_edit_singleline(&mut c.title);
        });
        choice(
            ui,
            "mode",
            &mut c.mode,
            &[
                CategoryBarMode::Grouped,
                CategoryBarMode::Stacked,
                CategoryBarMode::PercentStacked,
            ],
        );
        choice(
            ui,
            "direction",
            &mut c.direction,
            &[
                CategoryBarDirection::Vertical,
                CategoryBarDirection::Horizontal,
            ],
        );
        choice(
            ui,
            "labels",
            &mut c.labels,
            &[
                CategoryBarLabels::Auto,
                CategoryBarLabels::None,
                CategoryBarLabels::Inside,
                CategoryBarLabels::Outside,
            ],
        );
        choice(
            ui,
            "label format",
            &mut c.label_format,
            &[
                CategoryBarLabelFormat::Value,
                CategoryBarLabelFormat::Percent,
                CategoryBarLabelFormat::ValuePercent,
                CategoryBarLabelFormat::CategoryValue,
            ],
        );
        ui.horizontal(|ui| {
            ui.checkbox(&mut c.grid, "grid");
            ui.checkbox(&mut c.legend, "legend");
        });
        slider(ui, "group width", &mut c.group_width, 0.1..=0.95);
        slider(ui, "bar gap", &mut c.bar_gap, 0.0..=24.0);
        choice(ui, "material", &mut c.style.material, FLAT_BODY_MATERIALS);
        slider(ui, "corners", &mut c.style.corner_radius, 0.0..=32.0);
        slider(ui, "gloss", &mut c.style.gloss, 0.0..=1.0);
        ui.separator();
        for (i, s) in c.series.iter_mut().enumerate() {
            ui.push_id(i, |ui| color(ui, &s.label.clone(), &mut s.color));
        }
    }
}

// ---------------------------------------------------------------------------
// Box plot
// ---------------------------------------------------------------------------

pub struct BoxFamily;

impl Family for BoxFamily {
    type Chart = BoxPlotChart;
    type Renderer = BoxPlotRenderer;
    type Frame = BoxPlotFrame;
    type Target = BoxPlotTarget;
    type Error = BoxPlotError;

    fn renderer(gpu: RendererDevice) -> Result<BoxPlotRenderer, BoxPlotError> {
        BoxPlotRenderer::new(gpu, FORMAT)
    }
    fn set_background(r: &mut BoxPlotRenderer, b: FrameBackground) {
        r.set_background(b);
    }
    fn prepare(
        r: &mut BoxPlotRenderer,
        c: &BoxPlotChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<BoxPlotFrame>, BoxPlotError> {
        r.prepare(c, size, scale)
    }
    fn annotations(
        r: &mut BoxPlotRenderer,
        c: &BoxPlotChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<BoundedAnnotationFrame>, BoxPlotError> {
        r.prepare_annotations(c, size, scale)
    }
    fn end_frame(r: &BoxPlotRenderer) {
        r.end_frame();
    }
    fn export(
        r: &mut BoxPlotRenderer,
        c: &BoxPlotChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<renderer::RasterImage, BoxPlotError> {
        r.export_rgba(c, size, scale)
    }
    fn frame_size(f: &BoxPlotFrame) -> (u32, u32) {
        f.size()
    }
    fn draw(f: &BoxPlotFrame, pass: &mut wgpu::RenderPass<'_>) {
        f.draw(pass);
    }
    fn pick(f: &Arc<BoxPlotFrame>, p: [f32; 2]) -> Pick<BoxPlotTarget> {
        Pick::Ready(f.hit_test(p).map(|pick| pick.target))
    }
    fn decoration_map(f: &BoxPlotFrame) -> Option<&DecorationMap> {
        Some(f.decorations())
    }
    fn background(c: &BoxPlotChart) -> Color {
        c.background
    }
    fn layout(c: &mut BoxPlotChart) -> &mut DecorationLayout {
        &mut c.decorations
    }
    fn annotation_layer(c: &mut BoxPlotChart) -> &mut AnnotationLayer {
        &mut c.annotations
    }
    fn active_layer(c: &BoxPlotChart) -> LayerRef {
        c.active_layer()
    }
    fn set_active_layer(c: &mut BoxPlotChart, l: LayerRef) {
        c.set_active_layer(l);
    }
    fn selected(c: &BoxPlotChart) -> Option<BoxPlotTarget> {
        c.selected.clone()
    }
    fn set_selected(c: &mut BoxPlotChart, t: Option<BoxPlotTarget>) {
        c.selected = t;
    }
    fn hover(c: &mut BoxPlotChart, t: Option<BoxPlotTarget>, _dt: f32) -> bool {
        c.hovered = t;
        false
    }
    fn inspector(ui: &mut egui::Ui, c: &mut BoxPlotChart) {
        ui.horizontal(|ui| {
            ui.label("Title");
            ui.text_edit_singleline(&mut c.title);
        });
        choice(
            ui,
            "direction",
            &mut c.direction,
            &[BoxPlotDirection::Vertical, BoxPlotDirection::Horizontal],
        );
        choice(
            ui,
            "labels",
            &mut c.labels,
            &[
                BoxPlotLabels::None,
                BoxPlotLabels::Median,
                BoxPlotLabels::MedianAndCount,
            ],
        );
        ui.horizontal(|ui| {
            ui.checkbox(&mut c.grid, "grid");
            ui.checkbox(&mut c.legend, "legend");
        });
        let s = &mut c.style;
        choice(ui, "material", &mut s.material, FLAT_BODY_MATERIALS);
        ui.horizontal(|ui| {
            ui.checkbox(&mut s.notched, "notched");
            ui.checkbox(&mut s.show_mean, "mean");
            ui.checkbox(&mut s.show_outliers, "outliers");
            ui.checkbox(&mut s.caps, "caps");
        });
        slider(ui, "corners", &mut s.corner_radius, 0.0..=20.0);
        slider(ui, "median width", &mut s.median_width, 0.5..=6.0);
        slider(ui, "whisker width", &mut s.whisker_width, 0.5..=6.0);
        slider(ui, "outlier size", &mut s.outlier_size, 1.0..=16.0);
        color(ui, "median colour", &mut s.median_color);
        ui.separator();
        for (i, series) in c.series.iter_mut().enumerate() {
            ui.push_id(i, |ui| color(ui, &series.label.clone(), &mut series.color));
        }
    }
}

// ---------------------------------------------------------------------------
// Radar
// ---------------------------------------------------------------------------

pub struct RadarFamily;

impl Family for RadarFamily {
    type Chart = RadarChart;
    type Renderer = RadarRenderer;
    type Frame = RadarFrame;
    type Target = RadarTarget;
    type Error = RadarError;

    fn renderer(gpu: RendererDevice) -> Result<RadarRenderer, RadarError> {
        RadarRenderer::new(gpu, FORMAT)
    }
    fn set_background(r: &mut RadarRenderer, b: FrameBackground) {
        r.set_background(b);
    }
    fn prepare(
        r: &mut RadarRenderer,
        c: &RadarChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<RadarFrame>, RadarError> {
        r.prepare(c, size, scale)
    }
    fn annotations(
        r: &mut RadarRenderer,
        c: &RadarChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<Arc<BoundedAnnotationFrame>, RadarError> {
        r.prepare_annotations(c, size, scale)
    }
    fn end_frame(r: &RadarRenderer) {
        r.end_frame();
    }
    fn export(
        r: &mut RadarRenderer,
        c: &RadarChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<renderer::RasterImage, RadarError> {
        r.export_rgba(c, size, scale)
    }
    fn frame_size(f: &RadarFrame) -> (u32, u32) {
        f.size()
    }
    fn draw(f: &RadarFrame, pass: &mut wgpu::RenderPass<'_>) {
        f.draw(pass);
    }
    fn pick(f: &Arc<RadarFrame>, p: [f32; 2]) -> Pick<RadarTarget> {
        Pick::Ready(f.hit_test(p).map(|pick| pick.target))
    }
    fn decoration_map(f: &RadarFrame) -> Option<&DecorationMap> {
        Some(f.decorations())
    }
    fn background(c: &RadarChart) -> Color {
        c.background
    }
    fn layout(c: &mut RadarChart) -> &mut DecorationLayout {
        &mut c.decorations
    }
    fn annotation_layer(c: &mut RadarChart) -> &mut AnnotationLayer {
        &mut c.annotations
    }
    fn active_layer(c: &RadarChart) -> LayerRef {
        c.active_layer()
    }
    fn set_active_layer(c: &mut RadarChart, l: LayerRef) {
        c.set_active_layer(l);
    }
    fn selected(c: &RadarChart) -> Option<RadarTarget> {
        c.selected.clone()
    }
    fn set_selected(c: &mut RadarChart, t: Option<RadarTarget>) {
        c.selected = t;
    }
    fn hover(c: &mut RadarChart, t: Option<RadarTarget>, _dt: f32) -> bool {
        c.hovered = t;
        false
    }
    fn inspector(ui: &mut egui::Ui, c: &mut RadarChart) {
        ui.horizontal(|ui| {
            ui.label("Title");
            ui.text_edit_singleline(&mut c.title);
        });
        choice(
            ui,
            "labels",
            &mut c.labels,
            &[
                RadarLabels::None,
                RadarLabels::Values,
                RadarLabels::Emphasized,
            ],
        );
        ui.horizontal(|ui| {
            ui.checkbox(&mut c.grid, "grid");
            ui.checkbox(&mut c.legend, "legend");
        });
        let s = &mut c.style;
        choice(
            ui,
            "grid shape",
            &mut s.grid_shape,
            &[RadarGridShape::Polygon, RadarGridShape::Circle],
        );
        choice(
            ui,
            "marker",
            &mut s.marker,
            &[RadarMarker::None, RadarMarker::Circle, RadarMarker::Square],
        );
        ui.checkbox(&mut s.clockwise, "clockwise");
        slider(
            ui,
            "start angle°",
            &mut s.start_angle_degrees,
            -180.0..=180.0,
        );
        slider(ui, "line width", &mut s.line_width_px, 0.0..=12.0);
        slider(ui, "fill opacity", &mut s.fill_opacity, 0.0..=1.0);
        slider(ui, "marker size", &mut s.marker_size_px, 1.0..=24.0);
        slider(ui, "emphasis", &mut s.emphasis_brightness, 0.0..=0.5);
        color(ui, "spoke colour", &mut s.spoke_color);
        color(ui, "ring colour", &mut s.ring_color);
        ui.separator();
        for (i, series) in c.series.iter_mut().enumerate() {
            ui.push_id(i, |ui| color(ui, &series.label.clone(), &mut series.color));
        }
    }
}
