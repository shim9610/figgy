//! Native SSOT editor. Click a slice, change its color/style/label, or edit the chart.
//! cargo run -p figgy-renderer --example radial_editor --features egui_demo
use eframe::egui_wgpu::RenderState;
use renderer::{Color, Material, RendererDevice, radial::*};
use std::sync::Arc;
const MATERIALS: &[Material] = &[
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
const FORMATS: &[RadialLabelFormat] = &[
    RadialLabelFormat::Name,
    RadialLabelFormat::Value,
    RadialLabelFormat::Percent,
    RadialLabelFormat::NamePercent,
    RadialLabelFormat::NameValue,
    RadialLabelFormat::ValuePercent,
    RadialLabelFormat::NameValuePercent,
];
fn choice<T: Copy + PartialEq + std::fmt::Debug>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut T,
    options: &[T],
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{value:?}"))
        .show_ui(ui, |ui| {
            for option in options {
                ui.selectable_value(value, *option, format!("{option:?}"));
            }
        });
}
fn color(ui: &mut egui::Ui, label: &str, c: &mut Color) {
    ui.label(label);
    let mut rgb = [
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
    ];
    if ui.color_edit_button_srgb(&mut rgb).changed() {
        *c = Color::from_rgb8(rgb[0], rgb[1], rgb[2]);
    }
}
fn style(ui: &mut egui::Ui, s: &mut RadialStyle) {
    choice(ui, "material", &mut s.material, MATERIALS);
    for (value, range, name) in [
        (&mut s.tilt_degrees, 0.0..=65.0, "Tilt"),
        (&mut s.depth, 0.0..=0.4, "Depth"),
        (&mut s.inner_corner, 0.0..=0.25, "Inner corners"),
        (&mut s.outer_corner, 0.0..=0.25, "Outer corners"),
        (&mut s.bevel, 0.0..=0.1, "3D bevel"),
        (&mut s.gap_degrees, 0.0..=8.0, "Gap"),
        (&mut s.gloss, 0.0..=1.0, "Gloss"),
        (&mut s.roughness, 0.05..=1.0, "Roughness"),
        (&mut s.texture_strength, 0.0..=1.0, "Texture strength"),
        (&mut s.texture_scale, 0.1..=8.0, "Texture scale"),
        (
            &mut s.texture_angle_degrees,
            0.0..=360.0,
            "Texture direction",
        ),
        (&mut s.hover_lift, 0.0..=0.25, "Hover lift"),
    ] {
        ui.add(egui::Slider::new(value, range).text(name));
    }
    ui.checkbox(&mut s.shadow, "Shadow");
    ui.checkbox(&mut s.outline.rim, "Outer / inner outline");
    ui.checkbox(&mut s.outline.separators, "Slice borders");
    ui.checkbox(&mut s.outline.emphasis, "Selection outline");
    ui.add(egui::Slider::new(&mut s.outline.width, 0.0..=8.0).text("Outline width"));
    color(ui, "Outline color", &mut s.outline.color);
}
struct Editor {
    state: RenderState,
    renderer: RadialRenderer,
    chart: RadialChart,
    frame: Option<Arc<RadialFrame>>,
    target: Option<(wgpu::Texture, egui::TextureId)>,
    error: String,
    resize_handle: Option<renderer::resize::ResizeHandle>,
}
impl Editor {
    fn new(
        cc: &eframe::CreationContext<'_>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        let state = cc.wgpu_render_state.clone().ok_or("wgpu required")?;
        let renderer = RadialRenderer::new(
            RendererDevice::new(
                Arc::new(state.device.clone()),
                Arc::new(state.queue.clone()),
            ),
            wgpu::TextureFormat::Rgba8Unorm,
        )?;
        let mut chart = RadialChart {
            title: "Select a slice to edit".into(),
            kind: RadialKind::Donut { inner_radius: 0.5 },
            slices: vec![
                RadialSlice::new("Search", 40.0, Color::from_rgb8(51, 132, 245)),
                RadialSlice::new("Direct", 25.0, Color::from_rgb8(165, 216, 72)),
                RadialSlice::new("Email", 15.0, Color::from_rgb8(80, 96, 121)),
                RadialSlice::new("Union", 12.0, Color::from_rgb8(255, 151, 62)),
                RadialSlice::new("Video", 8.0, Color::from_rgb8(48, 192, 230)),
            ],
            ..Default::default()
        };
        chart.style = RadialStyle::three_dimensional();
        Ok(Self {
            state,
            renderer,
            chart,
            frame: None,
            target: None,
            error: String::new(),
            resize_handle: None,
        })
    }
    fn draw(&mut self, size: (u32, u32), scale: f32) -> Result<(), RadialError> {
        let frame = self.renderer.prepare(&self.chart, size, scale)?;
        if self
            .frame
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, &frame))
        {
            return Ok(());
        }
        let (w, h) = frame.size();
        if self
            .target
            .as_ref()
            .is_none_or(|(t, _)| t.width() != w || t.height() != h)
        {
            if let Some((_, id)) = self.target.take() {
                self.state.renderer.write().free_texture(&id);
            }
            let target = self.state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("radial editor panel"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let id = self.state.renderer.write().register_native_texture(
                &self.state.device,
                &target.create_view(&Default::default()),
                wgpu::FilterMode::Nearest,
            );
            self.target = Some((target, id));
        }
        let mut encoder = self
            .state
            .device
            .create_command_encoder(&Default::default());
        {
            let view = self
                .target
                .as_ref()
                .unwrap()
                .0
                .create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            frame.draw(&mut pass);
        }
        self.state.queue.submit([encoder.finish()]);
        self.frame = Some(frame);
        self.renderer.end_frame();
        Ok(())
    }
}
impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Panel::left("options")
            .resizable(false)
            .exact_size(280.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading("Radial chart editor");
                    ui.label("Click a slice, or choose one below.");
                    if ui
                        .selectable_label(self.chart.interaction.selected.is_none(), "Whole chart")
                        .clicked()
                    {
                        self.chart.interaction.selected = None;
                    }
                    for (i, slice) in self.chart.slices.iter().enumerate() {
                        let target = RadialTarget::main(i);
                        if ui
                            .selectable_label(
                                self.chart.interaction.selected == Some(target),
                                &slice.label,
                            )
                            .clicked()
                        {
                            self.chart.interaction.selected = Some(target);
                        }
                    }
                    ui.separator();
                    let selected = self.chart.interaction.selected;
                    if let Some(target) = selected {
                        let default_style = self.chart.style.clone();
                        let default_labels = self.chart.labels;
                        let default_format = self.chart.label_format;
                        let slice = self.chart.slice_mut(target).unwrap();
                        ui.text_edit_singleline(&mut slice.label);
                        color(ui, "Slice color", &mut slice.color);
                        let mut custom = slice.style.is_some();
                        if ui.checkbox(&mut custom, "Individual style").changed() {
                            slice.style = custom.then_some(default_style);
                        }
                        let mut labels = slice.labels.unwrap_or(default_labels);
                        choice(
                            ui,
                            "placement",
                            &mut labels,
                            &[
                                RadialLabels::None,
                                RadialLabels::Inside,
                                RadialLabels::Outside,
                            ],
                        );
                        if labels != slice.labels.unwrap_or(default_labels) {
                            slice.labels = Some(labels);
                        }
                        let mut format = slice.label_format.unwrap_or(default_format);
                        choice(ui, "label content", &mut format, FORMATS);
                        if format != slice.label_format.unwrap_or(default_format) {
                            slice.label_format = Some(format);
                        }
                        if ui.button("Inherit chart labels").clicked() {
                            slice.labels = None;
                            slice.label_format = None;
                            slice.label_color = None;
                        }
                        if let Some(s) = &mut slice.style {
                            style(ui, s);
                        }
                    } else {
                        ui.text_edit_singleline(&mut self.chart.title);
                        choice(
                            ui,
                            "placement",
                            &mut self.chart.labels,
                            &[
                                RadialLabels::None,
                                RadialLabels::Inside,
                                RadialLabels::Outside,
                            ],
                        );
                        choice(ui, "label content", &mut self.chart.label_format, FORMATS);
                        ui.add(
                            egui::Slider::new(&mut self.chart.label_decimals, 0..=6)
                                .text("Decimals"),
                        );
                        ui.text_edit_singleline(&mut self.chart.value_suffix);
                        if let RadialKind::Donut { inner_radius } = &mut self.chart.kind {
                            ui.add(egui::Slider::new(inner_radius, 0.1..=0.85).text("Hole"));
                        }
                        style(ui, &mut self.chart.style);
                    }
                    let editing_area =
                        self.chart.decorations.selected == Some(renderer::DecorationId::DataArea);
                    if ui
                        .selectable_label(editing_area, "Edit plot area")
                        .clicked()
                    {
                        self.chart.decorations.selected = if editing_area {
                            None
                        } else {
                            Some(renderer::DecorationId::DataArea)
                        };
                        self.chart.interaction.selected = None;
                    }
                    if let Some(selected) = self.chart.decorations.selected.clone() {
                        ui.label(format!("Decoration: {}", selected.element_id()));
                        if ui.button("Reset position").clicked() {
                            self.chart.decorations.reset(&selected);
                        }
                    }
                    if !self.error.is_empty() {
                        ui.colored_label(egui::Color32::RED, &self.error);
                    }
                });
            });
        egui::CentralPanel::default().show(ui, |ui| {
            let available = ui.available_size();
            if available.x < 160.0 || available.y < 160.0 {
                ui.label("Enlarge the window to draw the chart.");
                return;
            }
            let logical = (available.x.floor() as u32, available.y.floor() as u32);
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(logical.0 as f32, logical.1 as f32),
                egui::Sense::click_and_drag(),
            );
            // The inspector may have edited names, values or formats: write the
            // drawn label strings, then prepare before picking so resize and
            // inspector edits never use old bounds.
            self.chart.format_labels();
            if let Err(e) = self.draw(logical, ctx.pixels_per_point()) {
                self.error = e.to_string();
            }
            let decoration = response.interact_pointer_pos().and_then(|p| {
                let frame = self.frame.as_ref()?;
                let hit =
                    frame.decoration_at(&self.chart, [p.x - rect.left(), p.y - rect.top()])?;
                let key = frame.decoration_id(&self.chart, hit)?;
                // Foreground labels win; otherwise data wins over the area
                // until the user explicitly selects the area for editing.
                if key == renderer::DecorationId::DataArea
                    && self.chart.decorations.selected != Some(key.clone())
                    && frame
                        .hit_test([p.x - rect.left(), p.y - rect.top()])
                        .is_some()
                {
                    return None;
                }
                Some(key)
            });
            if response.drag_started() {
                self.resize_handle = ctx.input(|i| i.pointer.press_origin()).and_then(|p| {
                    let frame = self.frame.as_ref()?;
                    let id = frame.decoration(self.chart.decorations.selected.as_ref()?)?;
                    frame.decoration_resize_handle(
                        &self.chart,
                        id,
                        [p.x - rect.left(), p.y - rect.top()],
                    )
                });
            }
            if (response.clicked() || response.drag_started()) && self.resize_handle.is_none() {
                self.chart.decorations.selected = decoration.clone();
            }
            if response.dragged() {
                if let (Some(frame), Some(selected)) =
                    (&self.frame, &self.chart.decorations.selected.clone())
                {
                    if let Some(id) = frame.decoration(selected) {
                        let delta = ctx.input(|i| i.pointer.delta());
                        let previous = self.chart.decorations.clone();
                        if let Some(handle) = self.resize_handle {
                            frame.resize_decoration(&mut self.chart, id, handle, delta.x, delta.y);
                        } else {
                            frame.drag_decoration(&mut self.chart, id, delta.x, delta.y);
                        }
                        // Keep the last feasible model when text no longer fits.
                        if self
                            .renderer
                            .prepare(&self.chart, logical, ctx.pixels_per_point())
                            .is_err()
                        {
                            self.chart.decorations = previous;
                        }
                    }
                }
            }
            if response.drag_stopped() {
                self.resize_handle = None;
            }
            let hovered = response
                .hover_pos()
                .filter(|_| decoration.is_none())
                .and_then(|p| {
                    let frame = self.frame.as_ref()?;
                    let point = [p.x - rect.left(), p.y - rect.top()];
                    frame.hit_test(point).or_else(|| {
                        frame
                            .hit_test_at_rest(point)
                            .filter(|t| Some(*t) == self.chart.interaction.hovered)
                    })
                });
            if response.clicked() {
                self.chart.interaction.selected = hovered;
            }
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            if self.chart.interaction.animate_hover(hovered, dt, 0.16) {
                ctx.request_repaint();
            }
            match self.draw(logical, ctx.pixels_per_point()) {
                Ok(()) => self.error.clear(),
                Err(e) => self.error = e.to_string(),
            }
            if let Some((_, id)) = &self.target
                && self.error.is_empty()
            {
                ui.painter().image(
                    *id,
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        });
    }
}
impl Drop for Editor {
    fn drop(&mut self) {
        if let Some((_, id)) = self.target.take() {
            self.state.renderer.write().free_texture(&id);
        }
        self.frame = None;
        self.renderer.clear_cache();
        self.renderer.end_frame();
        let _ = self.state.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }
}
fn main() -> eframe::Result<()> {
    eframe::run_native(
        "figgy radial editor",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 780.0]),
            renderer: eframe::Renderer::Wgpu,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(Editor::new(cc)?))),
    )
}
