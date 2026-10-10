//! Native SSOT editor. Click a box, change its color/style/label, or edit the chart.
//! cargo run -p figgy-renderer --example boxplot_editor --features egui_demo
use eframe::egui_wgpu::RenderState;
use renderer::{Category, Color, RendererDevice, boxplot::*};
use std::sync::Arc;
const MATERIALS: &[BoxPlotMaterial] = &[
    BoxPlotMaterial::Flat,
    BoxPlotMaterial::Matte,
    BoxPlotMaterial::SatinMetal,
];
const LABELS: &[BoxPlotLabels] = &[
    BoxPlotLabels::None,
    BoxPlotLabels::Median,
    BoxPlotLabels::MedianAndCount,
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
fn style(ui: &mut egui::Ui, s: &mut BoxPlotStyle) {
    choice(ui, "material", &mut s.material, MATERIALS);
    for (value, range, name) in [
        (&mut s.corner_radius, 0.0..=3.0, "Corners"),
        (&mut s.gloss, 0.0..=1.0, "Gloss"),
        (&mut s.texture_strength, 0.0..=1.0, "Texture strength"),
        (&mut s.texture_scale, 0.1..=8.0, "Texture scale"),
        (&mut s.emphasis_brightness, 0.0..=0.5, "Emphasis"),
    ] {
        ui.add(egui::Slider::new(value, range).text(name));
    }
    ui.checkbox(&mut s.outline, "Outline");
    ui.add(egui::Slider::new(&mut s.outline_width, 0.0..=8.0).text("Outline width"));
    color(ui, "Outline color", &mut s.outline_color);
    ui.checkbox(&mut s.notched, "Notched (supplied median CI)");
    ui.add(egui::Slider::new(&mut s.notch_depth, 0.0..=0.45).text("Notch depth"));
    ui.checkbox(&mut s.caps, "Whisker caps");
    ui.add(egui::Slider::new(&mut s.cap_ratio, 0.0..=1.0).text("Cap width"));
    ui.add(egui::Slider::new(&mut s.whisker_width, 0.0..=8.0).text("Whisker width"));
    color(ui, "Whisker color", &mut s.whisker_color);
    ui.add(egui::Slider::new(&mut s.median_width, 0.0..=8.0).text("Median width"));
    color(ui, "Median color", &mut s.median_color);
    ui.checkbox(&mut s.show_mean, "Mean marker");
    ui.add(egui::Slider::new(&mut s.mean_size, 1.0..=24.0).text("Mean size"));
    color(ui, "Mean color", &mut s.mean_color);
    ui.checkbox(&mut s.show_outliers, "Outliers");
    ui.checkbox(&mut s.outlier_filled, "Filled outliers");
    choice(
        ui,
        "outlier shape",
        &mut s.outlier_shape,
        &[BoxOutlierShape::Circle, BoxOutlierShape::Square],
    );
    ui.add(egui::Slider::new(&mut s.outlier_size, 1.0..=24.0).text("Outlier size"));
    color(ui, "Outlier color", &mut s.outlier_color);
}
struct Editor {
    state: RenderState,
    renderer: BoxPlotRenderer,
    chart: BoxPlotChart,
    frame: Option<Arc<BoxPlotFrame>>,
    target: Option<(wgpu::Texture, egui::TextureId)>,
    error: String,
}
impl Editor {
    fn new(
        cc: &eframe::CreationContext<'_>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        let state = cc.wgpu_render_state.clone().ok_or("wgpu required")?;
        let renderer = BoxPlotRenderer::new(
            RendererDevice::new(
                Arc::new(state.device.clone()),
                Arc::new(state.queue.clone()),
            ),
            wgpu::TextureFormat::Rgba8Unorm,
        )?;
        let mut chart = BoxPlotChart {
            title: "Select a box to edit".into(),
            categories: ["A", "B", "C", "D"]
                .into_iter()
                .map(|s| Category::new(s, s))
                .collect(),
            series: vec![
                BoxPlotSeries::new(
                    "before",
                    "Before",
                    [42., 55., 36., 48.]
                        .map(|median| {
                            let mut v = BoxSummary::new(
                                median - 25.,
                                median - 12.,
                                median,
                                median + 15.,
                                median + 30.,
                            );
                            v.median_ci = Some([median - 5., median + 5.]);
                            v.mean = Some(median + 2.);
                            v.sample_count = Some(80);
                            v.outliers = vec![median + 39.];
                            Some(v)
                        })
                        .into(),
                    Color::from_rgb8(85, 143, 211),
                ),
                BoxPlotSeries::new(
                    "after",
                    "After",
                    [50., 44., 40., 58.]
                        .map(|median| {
                            let mut v = BoxSummary::new(
                                median - 25.,
                                median - 12.,
                                median,
                                median + 15.,
                                median + 30.,
                            );
                            v.median_ci = Some([median - 5., median + 5.]);
                            v.mean = Some(median + 2.);
                            v.sample_count = Some(80);
                            v.outliers = vec![median + 39.];
                            Some(v)
                        })
                        .into(),
                    Color::from_rgb8(255, 166, 105),
                ),
            ],
            value_title: "Response".into(),
            value_range: Some([0., 130.]),
            ..Default::default()
        };
        chart.style.material = BoxPlotMaterial::Matte;
        Ok(Self {
            state,
            renderer,
            chart,
            frame: None,
            target: None,
            error: String::new(),
        })
    }
    fn draw(&mut self, size: (u32, u32), scale: f32) -> Result<(), BoxPlotError> {
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
                label: Some("boxplot editor panel"),
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
                    ui.heading("Box plot editor");
                    choice(
                        ui,
                        "direction",
                        &mut self.chart.direction,
                        &[BoxPlotDirection::Vertical, BoxPlotDirection::Horizontal],
                    );
                    ui.checkbox(&mut self.chart.grid, "Grid");
                    ui.checkbox(&mut self.chart.legend, "Legend");
                    if ui
                        .selectable_label(self.chart.selected.is_none(), "Whole chart")
                        .clicked()
                    {
                        self.chart.selected = None;
                    }
                    for category in &self.chart.categories {
                        for series in &self.chart.series {
                            let target = BoxPlotTarget::new(&category.id, &series.id);
                            if ui
                                .selectable_label(
                                    self.chart.selected.as_ref() == Some(&target),
                                    format!("{} / {}", category.label, series.label),
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
                        let series = self
                            .chart
                            .series
                            .iter()
                            .find(|s| s.id == target.series_id)
                            .unwrap();
                        let base_color = series.color;
                        let base_style = series
                            .style
                            .clone()
                            .unwrap_or_else(|| self.chart.style.clone());
                        let base_labels = self.chart.labels;
                        if !self.chart.overrides.iter().any(|o| o.target == target) {
                            self.chart
                                .overrides
                                .push(BoxPlotOverride::new(target.clone()));
                        }
                        let edit = self
                            .chart
                            .overrides
                            .iter_mut()
                            .find(|o| o.target == target)
                            .unwrap();
                        let mut current = edit.color.unwrap_or(base_color);
                        color(ui, "Box color", &mut current);
                        if current != edit.color.unwrap_or(base_color) {
                            edit.color = Some(current);
                        }
                        let mut custom = edit.style.is_some();
                        if ui.checkbox(&mut custom, "Individual style").changed() {
                            edit.style = custom.then_some(base_style);
                        }
                        if let Some(s) = &mut edit.style {
                            style(ui, s);
                        }
                        let mut labels = edit.labels.unwrap_or(base_labels);
                        choice(ui, "placement", &mut labels, LABELS);
                        if labels != edit.labels.unwrap_or(base_labels) {
                            edit.labels = Some(labels);
                        }
                        if ui.button("Reset individual settings").clicked() {
                            *edit = BoxPlotOverride::new(target);
                        }
                    } else {
                        ui.text_edit_singleline(&mut self.chart.title);
                        choice(ui, "placement", &mut self.chart.labels, LABELS);
                        ui.add(
                            egui::Slider::new(&mut self.chart.label_decimals, 0..=6)
                                .text("Decimals"),
                        );
                        ui.text_edit_singleline(&mut self.chart.value_suffix);
                        ui.add(
                            egui::Slider::new(&mut self.chart.group_width, 0.1..=0.95)
                                .text("Group width"),
                        );
                        ui.add(
                            egui::Slider::new(&mut self.chart.box_gap, 0.0..=24.0).text("Box gap"),
                        );
                        style(ui, &mut self.chart.style);
                    }
                    if !self.error.is_empty() {
                        ui.colored_label(egui::Color32::RED, &self.error);
                    }
                });
            });
        egui::CentralPanel::default().show(ui, |ui| {
            let available = ui.available_size();
            if available.x < 240.0 || available.y < 200.0 {
                ui.label("Enlarge the window to draw the chart.");
                return;
            }
            let logical = (available.x.floor() as u32, available.y.floor() as u32);
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(logical.0 as f32, logical.1 as f32),
                egui::Sense::click(),
            );
            // Refresh geometry before hit testing after a resize/direction edit.
            if let Err(e) = self.draw(logical, ctx.pixels_per_point()) {
                self.error = e.to_string();
            }
            let hovered = response.hover_pos().and_then(|p| {
                let frame = self.frame.as_ref()?;
                let point = [p.x - rect.left(), p.y - rect.top()];
                frame.hit_test(point).map(|hit| hit.target)
            });
            if response.clicked() {
                self.chart.selected = hovered.clone();
            }
            if let Some(t) = &hovered {
                if let Some(v) = self.chart.summary(t) {
                    response.clone().on_hover_text(format!(
                        "{} / {}\nQ1: {}  Median: {}  Q3: {}\nWhiskers: {} .. {}\nMean: {}  n: {}",
                        t.category_id,
                        t.series_id,
                        v.q1,
                        v.median,
                        v.q3,
                        v.whisker_low,
                        v.whisker_high,
                        v.mean.map_or_else(|| "—".into(), |m| m.to_string()),
                        v.sample_count.map_or_else(|| "—".into(), |n| n.to_string())
                    ));
                }
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
        "figgy boxplot editor",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 780.0]),
            renderer: eframe::Renderer::Wgpu,
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(Editor::new(cc)?))),
    )
}
