//! The radial scene: the DataLess radial chart (pie, donut and detail ring).
//!
//! The radial chart has its own small renderer that prepares a frame of an
//! exact physical size. The host renders that frame (the chart, then the
//! annotation layer) into an offscreen texture and egui shows it as an image.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use renderer::annotation::BoundedAnnotationFrame;
use renderer::layout::NudgeResult;
use renderer::radial::*;
use renderer::text::RichText;
use renderer::{
    Anchor, Annotation, AnnotationAxis, AnnotationKind, Color, DecorationId, LayerHit, LayerRef,
    Material, RendererDevice, ResizeHandle,
};

use crate::HostCtx;
use crate::scenes::radial::radial_chart;
use crate::widgets::{choice, color, section, slider};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A slice pick: a GPU readback polled once per frame.
type Pick = Pin<Box<dyn Future<Output = Option<RadialTarget>>>>;

/// The slice under `p`, picked on the GPU; `None` once answered without a
/// slice, and at once when the request fails.
fn pick(f: &Arc<RadialFrame>, p: [f32; 2]) -> Pick {
    match f.pick(p) {
        Ok(ticket) => Box::pin(async move { ticket.resolve().await.ok()? }),
        Err(_) => Box::pin(async { None }),
    }
}

/// The picked slice once the answer is in; `None` while it is pending.
fn poll(pick: &mut Pick) -> Option<Option<RadialTarget>> {
    match pick.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(t) => Some(t),
        Poll::Pending => None,
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

pub struct RadialHost {
    renderer: RadialRenderer,
    chart: RadialChart,
    initial: RadialChart,
    frame: Option<Arc<RadialFrame>>,
    layer: Option<Arc<BoundedAnnotationFrame>>,
    target: Option<(wgpu::Texture, egui::TextureId)>,
    drag: Drag,
    /// The press waiting for its data pick.
    press: Option<(Press, Pick)>,
    /// The hover pick in flight, and the last answered hover.
    hover_pick: Option<Pick>,
    hovered: Option<RadialTarget>,
    size: (u32, u32),
    error: String,
}

impl RadialHost {
    pub fn create(gpu: RendererDevice) -> Result<Self, String> {
        let chart = radial_chart();
        Ok(Self {
            renderer: RadialRenderer::new(gpu, FORMAT).map_err(|e| e.to_string())?,
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
        })
    }

    /// Prepare the frame + annotation layer and render them into the texture.
    fn render(&mut self, host: &mut HostCtx<'_>, scale: f32) -> Result<(), String> {
        let frame = self
            .renderer
            .prepare(&self.chart, self.size, scale)
            .map_err(|e| e.to_string())?;
        let layer = self
            .renderer
            .prepare_annotations(&self.chart, self.size, scale)
            .map_err(|e| e.to_string())?;
        let (w, h) = frame.size();
        if self
            .target
            .as_ref()
            .is_none_or(|(t, _)| t.width() != w || t.height() != h)
        {
            if let Some((_, id)) = self.target.take() {
                host.egui_renderer.free_texture(&id);
            }
            let texture = host.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("radial scene"),
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
        let bg = self.chart.background;
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
            frame.draw(&mut pass);
            layer.draw_overlay(&mut pass);
        }
        host.queue.submit([encoder.finish()]);
        self.frame = Some(frame);
        self.layer = Some(layer);
        self.renderer.end_frame();
        Ok(())
    }

    fn on_press(&mut self, p: [f32; 2]) {
        let (Some(frame), Some(layer)) = (self.frame.clone(), self.layer.clone()) else {
            return;
        };
        let active = self.chart.active_layer();
        // The selected annotation's own handles win first.
        if let Some(DecorationId::Annotation(index)) = self.chart.decorations.selected.clone()
            && let Some(id) = layer.annotations().find(index)
            && let Some(h) = layer.annotations().hit_resize_handle(id, p)
        {
            self.drag = Drag::Annotation(Some(h));
            return;
        }
        // Then the selected decoration's resize handles.
        if let Some(selected) = self.chart.decorations.selected.clone()
            && let Some(id) = frame.decoration(&selected)
            && let Some(h) = frame.decoration_resize_handle(&self.chart, id, p)
        {
            self.drag = Drag::Decoration(Some(h));
            return;
        }
        match layer.route_active_layer(active, p, || frame.decoration_at(&self.chart, p)) {
            Some(LayerHit::Annotation { index, .. }) => {
                self.chart.decorations.selected = Some(DecorationId::Annotation(index));
                self.drag = Drag::Annotation(None);
                return;
            }
            Some(LayerHit::Element(id)) => {
                let key = frame.decoration_id(&self.chart, id);
                // Data wins over the plot area until the area itself is selected.
                let area_passthrough =
                    key == Some(DecorationId::DataArea) && self.chart.decorations.selected != key;
                if !area_passthrough {
                    self.chart.decorations.selected = key;
                    self.chart.interaction.selected = None;
                    self.drag = Drag::Decoration(None);
                    return;
                }
                self.press = Some((Press::AreaOrData, pick(&frame, p)));
                self.drag = Drag::None;
                return;
            }
            None => {}
        }
        self.press = Some((Press::Data, pick(&frame, p)));
        self.drag = Drag::None;
    }

    /// Apply the press once its data pick has answered.
    fn poll_press(&mut self) {
        let Some((press, pick)) = &mut self.press else {
            return;
        };
        let Some(hit) = poll(pick) else { return };
        let area = matches!(press, Press::AreaOrData) && hit.is_none();
        self.press = None;
        if area {
            self.chart.decorations.selected = Some(DecorationId::DataArea);
            self.chart.interaction.selected = None;
            self.drag = Drag::Decoration(None);
        } else {
            self.chart.decorations.selected = None;
            self.chart.interaction.selected = hit;
        }
    }

    fn on_move(&mut self, dx: f32, dy: f32, scale: f32) {
        let (Some(frame), Some(layer)) = (self.frame.clone(), self.layer.clone()) else {
            return;
        };
        let Some(selected) = self.chart.decorations.selected.clone() else {
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
                let items = &mut self.chart.annotations;
                match handle {
                    Some(h) => map.resize_by(items, id, h, dx, dy),
                    None => map.drag_by(items, id, dx, dy),
                }
            }
            Drag::Decoration(handle) => {
                let Some(id) = frame.decoration(&selected) else {
                    return;
                };
                match handle {
                    Some(h) => frame.resize_decoration(&mut self.chart, id, h, dx, dy),
                    None => frame.drag_decoration(&mut self.chart, id, dx, dy),
                }
            }
        };
        // Keep the last feasible model when the new layout no longer fits.
        if moved == Some(NudgeResult::Moved)
            && self
                .renderer
                .prepare(&self.chart, self.size, scale)
                .is_err()
        {
            self.chart = previous;
        }
    }

    pub fn controls(&mut self, ui: &mut egui::Ui, host: &mut HostCtx<'_>) {
        section(ui, "Chart", true, |ui| {
            inspector(ui, &mut self.chart);
        });
        section(ui, "Selection & layers", true, |ui| {
            ui.label(format!("Data: {:?}", self.chart.interaction.selected));
            ui.label(format!("Decoration: {:?}", self.chart.decorations.selected));
            let mut layer = self.chart.active_layer();
            if choice(
                ui,
                "Active layer",
                &mut layer,
                &[LayerRef::Base, LayerRef::Annotations],
            ) {
                self.chart.set_active_layer(layer);
            }
            ui.horizontal(|ui| {
                let editing = self.chart.decorations.selected == Some(DecorationId::DataArea);
                if ui.selectable_label(editing, "Edit plot area").clicked() {
                    self.chart.decorations.selected = (!editing).then_some(DecorationId::DataArea);
                    self.chart.interaction.selected = None;
                }
                if let Some(sel) = self.chart.decorations.selected.clone()
                    && ui.button("Reset position").clicked()
                {
                    self.chart.decorations.reset(&sel);
                }
                if ui.button("Reset scene").clicked() {
                    self.chart = self.initial.clone();
                }
            });
        });
        section(ui, "Annotations", false, |ui| {
            let accent = Color::from_rgb8(200, 40, 120);
            let add = |chart: &mut RadialChart, kind| {
                chart.annotations.items.push(Annotation::new(kind));
                chart.set_active_layer(LayerRef::Annotations);
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
            let layer = &mut self.chart.annotations;
            ui.checkbox(&mut layer.visible, "Visible");
            ui.label(format!("{} items", layer.items.len()));
            if let Some(l) = &self.layer {
                for w in l.render_warnings() {
                    ui.colored_label(egui::Color32::from_rgb(170, 110, 0), w.to_string());
                }
            }
        });
        section(ui, "Export", false, |ui| {
            let exported = if ui.button("Export PNG (2x)").clicked() {
                Some((
                    "png",
                    self.renderer
                        .export_rgba(&self.chart, self.size, 2.0)
                        .map_err(|e| e.to_string())
                        .and_then(|img| renderer::encode_png(&img).map_err(|e| e.to_string())),
                ))
            } else if ui.button("Export SVG (2x)").clicked() {
                Some((
                    "svg",
                    self.renderer
                        .export_svg(&self.chart, self.size, 2.0)
                        .map(String::into_bytes)
                        .map_err(|e| e.to_string()),
                ))
            } else if ui.button("Export PDF (2x)").clicked() {
                Some((
                    "pdf",
                    self.renderer
                        .export_pdf(&self.chart, self.size, 2.0)
                        .map_err(|e| e.to_string()),
                ))
            } else {
                None
            };
            if let Some((extension, bytes)) = exported {
                let result = bytes.and_then(|bytes| {
                    let dir = std::path::Path::new("target/playground");
                    let path = dir.join(format!("radial.{extension}"));
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

    pub fn viewport(
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
        if let Some(hit) = self.hover_pick.as_mut().and_then(poll) {
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
                    self.hover_pick = Some(pick(frame, local(p)));
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
        let hovered = self.hovered;
        if self.chart.interaction.animate_hover(hovered, dt, 0.16) {
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

    pub fn release(&mut self, egui_renderer: &mut egui_wgpu::Renderer) {
        if let Some((_, id)) = self.target.take() {
            egui_renderer.free_texture(&id);
        }
        self.frame = None;
        self.layer = None;
        self.press = None;
        self.hover_pick = None;
        self.renderer.end_frame();
    }
}

// ---------------------------------------------------------------------------
// Inspector
// ---------------------------------------------------------------------------

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
const RADIAL_FORMATS: &[RadialLabelFormat] = &[
    RadialLabelFormat::Name,
    RadialLabelFormat::Value,
    RadialLabelFormat::Percent,
    RadialLabelFormat::NamePercent,
    RadialLabelFormat::NameValue,
    RadialLabelFormat::NameValuePercent,
];

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
