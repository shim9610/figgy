//! Native SSOT editor. Click a series polygon or a vertex, change its color,
//! style or value, or edit the chart.
//! cargo run -p figgy-renderer --example radar_editor --features egui_demo
use eframe::egui_wgpu::RenderState;
use renderer::{Category, Color, RendererDevice, radar::*};
use std::sync::Arc;
const GRID_SHAPES: &[RadarGridShape] = &[RadarGridShape::Polygon, RadarGridShape::Circle];
const MARKERS: &[RadarMarker] = &[RadarMarker::None, RadarMarker::Circle, RadarMarker::Square];
const LABELS: &[RadarLabels] = &[
    RadarLabels::None,
    RadarLabels::Values,
    RadarLabels::Emphasized,
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
fn style(ui: &mut egui::Ui, s: &mut RadarStyle) {
    choice(ui, "grid shape", &mut s.grid_shape, GRID_SHAPES);
    ui.checkbox(&mut s.clockwise, "Clockwise");
    for (value, range, name) in [
        (&mut s.start_angle_degrees, -180.0..=180.0, "Start angle"),
        (&mut s.line_width_px, 0.0..=12.0, "Line width"),
        (&mut s.fill_opacity, 0.0..=1.0, "Fill opacity"),
        (&mut s.emphasis_brightness, 0.0..=0.5, "Emphasis"),
    ] {
        ui.add(egui::Slider::new(value, range).text(name));
    }
    choice(ui, "marker", &mut s.marker, MARKERS);
    ui.add(egui::Slider::new(&mut s.marker_size_px, 1.0..=24.0).text("Marker size"));
    color(ui, "Spoke color", &mut s.spoke_color);
    color(ui, "Ring color", &mut s.ring_color);
}
/// The per-series overrides, shown as the resolved values so a toggle starts
/// from what the chart already draws.
fn series_style(
    ui: &mut egui::Ui,
    s: &mut RadarSeriesStyle,
    resolved: (f32, Color, RadarMarker, f32),
) {
    let (line_width, fill, marker, marker_size) = resolved;
    let mut width = s.line_width_px.unwrap_or(line_width);
    if ui
        .add(egui::Slider::new(&mut width, 0.0..=12.0).text("Line width"))
        .changed()
    {
        s.line_width_px = Some(width);
    }
    let mut own_fill = s.fill.is_some();
    if ui.checkbox(&mut own_fill, "Own fill color").changed() {
        s.fill = own_fill.then_some(fill);
    }
    if let Some(fill) = &mut s.fill {
        let mut rgba = [
            (fill.r * 255.0).round() as u8,
            (fill.g * 255.0).round() as u8,
            (fill.b * 255.0).round() as u8,
            (fill.a * 255.0).round() as u8,
        ];
        ui.label("Fill");
        if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
            *fill = Color::from_rgba(
                rgba[0] as f32 / 255.0,
                rgba[1] as f32 / 255.0,
                rgba[2] as f32 / 255.0,
                rgba[3] as f32 / 255.0,
            );
        }
    }
    let mut own_marker = s.marker.unwrap_or(marker);
    choice(ui, "series marker", &mut own_marker, MARKERS);
    if own_marker != s.marker.unwrap_or(marker) {
        s.marker = Some(own_marker);
    }
    let mut size = s.marker_size_px.unwrap_or(marker_size);
    if ui
        .add(egui::Slider::new(&mut size, 1.0..=24.0).text("Marker size"))
        .changed()
    {
        s.marker_size_px = Some(size);
    }
}
struct Editor {
    state: RenderState,
    renderer: RadarRenderer,
    chart: RadarChart,
    frame: Option<Arc<RadarFrame>>,
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
        let renderer = RadarRenderer::new(
            RendererDevice::new(
                Arc::new(state.device.clone()),
                Arc::new(state.queue.clone()),
            ),
            wgpu::TextureFormat::Rgba8Unorm,
        )?;
        let chart = RadarChart {
            title: "Select a series or a vertex to edit".into(),
            categories: ["Speed", "Safety", "Size", "Docs", "Tests", "Ergonomics"]
                .into_iter()
                .map(|s| Category::new(s.to_lowercase(), s))
                .collect(),
            series: vec![
                RadarSeries::new(
                    "before",
                    "Before",
                    vec![6., 8., 4., 5., 7., 6.5],
                    Color::from_rgb8(85, 143, 211),
                ),
                RadarSeries::new(
                    "after",
                    "After",
                    vec![8., 7., 6., 8., 9., 7.],
                    Color::from_rgb8(255, 166, 105),
                ),
            ],
            value_range: Some([0., 10.]),
            ring_spacing: Some(2.5),
            ..Default::default()
        };
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
    fn draw(&mut self, size: (u32, u32), scale: f32) -> Result<(), RadarError> {
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
                label: Some("radar editor panel"),
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
                    ui.heading("Radar editor");
                    ui.checkbox(&mut self.chart.grid, "Grid");
                    ui.checkbox(&mut self.chart.legend, "Legend");
                    if ui
                        .selectable_label(self.chart.selected.is_none(), "Whole chart")
                        .clicked()
                    {
                        self.chart.selected = None;
                    }
                    for series in &self.chart.series {
                        let target = RadarTarget::series(&series.id);
                        if ui
                            .selectable_label(
                                self.chart.selected.as_ref() == Some(&target),
                                &series.label,
                            )
                            .clicked()
                        {
                            self.chart.selected = Some(target);
                        }
                        for category in &self.chart.categories {
                            let target = RadarTarget::vertex(&series.id, &category.id);
                            if ui
                                .selectable_label(
                                    self.chart.selected.as_ref() == Some(&target),
                                    format!("  {} / {}", series.label, category.label),
                                )
                                .clicked()
                            {
                                self.chart.selected = Some(target);
                            }
                        }
                    }
                    if ui.button("Reverse category order").clicked() {
                        let ids: Vec<_> = self
                            .chart
                            .categories
                            .iter()
                            .rev()
                            .map(|c| c.id.clone())
                            .collect();
                        let order: Vec<_> = ids.iter().map(String::as_str).collect();
                        let _ = self.chart.reorder_categories(&order);
                    }
                    ui.separator();
                    if let Some(target) = self.chart.selected.clone() {
                        let index = self
                            .chart
                            .series
                            .iter()
                            .position(|s| s.id == target.series_id)
                            .unwrap();
                        let resolved = self.chart.resolved_style(&self.chart.series[index]);
                        let series = &mut self.chart.series[index];
                        if let Some(category) = target
                            .category_id
                            .as_deref()
                            .and_then(|id| self.chart.categories.iter().position(|c| c.id == id))
                        {
                            ui.horizontal(|ui| {
                                ui.label("Value");
                                ui.add(
                                    egui::DragValue::new(&mut series.values[category])
                                        .range(0.0..=10.0)
                                        .speed(0.1),
                                );
                            });
                        }
                        color(ui, "Series color", &mut series.color);
                        let mut custom = series.style.is_some();
                        if ui.checkbox(&mut custom, "Individual style").changed() {
                            series.style = custom.then_some(RadarSeriesStyle::default());
                        }
                        if let Some(s) = &mut series.style {
                            series_style(ui, s, resolved);
                        }
                        if ui.button("Reset individual settings").clicked() {
                            series.style = None;
                        }
                    } else {
                        ui.text_edit_singleline(&mut self.chart.title);
                        choice(ui, "placement", &mut self.chart.labels, LABELS);
                        ui.add(
                            egui::Slider::new(&mut self.chart.label_decimals, 0..=6)
                                .text("Decimals"),
                        );
                        ui.text_edit_singleline(&mut self.chart.value_suffix);
                        let mut fixed = self.chart.value_range.is_some();
                        if ui.checkbox(&mut fixed, "Fixed 0 to 10 axis").changed() {
                            self.chart.value_range = fixed.then_some([0., 10.]);
                            self.chart.ring_spacing = fixed.then_some(2.5);
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
                        self.chart.selected = None;
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
            if available.x < 240.0 || available.y < 240.0 {
                ui.label("Enlarge the window to draw the chart.");
                return;
            }
            let logical = (available.x.floor() as u32, available.y.floor() as u32);
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(logical.0 as f32, logical.1 as f32),
                egui::Sense::click_and_drag(),
            );
            // Refresh geometry before hit testing after a resize/style edit.
            if let Err(e) = self.draw(logical, ctx.pixels_per_point()) {
                self.error = e.to_string();
            }
            // Prepare before picking: resize and inspector edits must not use old bounds.
            if let Err(e) = self.draw(logical, ctx.pixels_per_point()) {
                self.error = e.to_string();
            }
            let decoration = response.interact_pointer_pos().and_then(|p| {
                let frame = self.frame.as_ref()?;
                let map = frame.decorations();
                let hit = map.hit_test([p.x - rect.left(), p.y - rect.top()])?;
                let key = map.element(hit)?.id.clone();
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
                    let map = self.frame.as_ref()?.decorations();
                    let selected = self.chart.decorations.selected.as_ref()?;
                    let (id, _) = map.entries().find(|(_, e)| &e.id == selected)?;
                    map.hit_resize_handle(id, [p.x - rect.left(), p.y - rect.top()])
                });
            }
            if (response.clicked() || response.drag_started()) && self.resize_handle.is_none() {
                self.chart.decorations.selected = decoration.clone();
            }
            if response.dragged() {
                if let (Some(frame), Some(selected)) =
                    (&self.frame, &self.chart.decorations.selected.clone())
                {
                    let map = frame.decorations();
                    if let Some((id, _)) = map.entries().find(|(_, e)| &e.id == selected) {
                        let delta = ctx.input(|i| i.pointer.delta());
                        let previous = self.chart.decorations.clone();
                        if let Some(handle) = self.resize_handle {
                            map.resize_by(
                                &mut self.chart.decorations,
                                id,
                                handle,
                                delta.x,
                                delta.y,
                            );
                        } else {
                            map.drag_by(&mut self.chart.decorations, id, delta.x, delta.y);
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
                    frame.hit_test(point).map(|hit| hit.target)
                });
            if response.clicked() {
                self.chart.selected = hovered.clone();
            }
            if let Some(t) = &hovered
                && let Some(series) = self.chart.series(&t.series_id)
            {
                let category = t
                    .category_id
                    .as_deref()
                    .and_then(|id| self.chart.categories.iter().find(|c| c.id == id));
                let text = match (category, self.chart.value(t)) {
                    (Some(category), Some(value)) => format!(
                        "{} / {}\nValue: {}",
                        series.label,
                        category.label,
                        self.chart.label_text(value)
                    ),
                    _ => series.label.clone(),
                };
                response.clone().on_hover_text(text);
            }
            self.chart.hovered = hovered;
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
        "figgy radar editor",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 780.0]),
            renderer: eframe::Renderer::Wgpu,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(Editor::new(cc)?))),
    )
}
