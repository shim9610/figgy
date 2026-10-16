//! Raw WASM host for radial, categorical and box charts. The renderer owns all geometry.
use renderer::{
    DecorationLayout, RendererDevice, boxplot::*, categorical::*, layout::Rect, radial::*,
};
use serde_json::json;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
fn js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}
#[derive(Clone, PartialEq)]
enum DocumentChart {
    Radial(RadialChart),
    Categorical(CategoricalChart),
    BoxPlot(BoxPlotChart),
}
impl DocumentChart {
    pub fn family(&self) -> &'static str {
        match self {
            Self::Radial(_) => "radial",
            Self::Categorical(_) => "categorical",
            Self::BoxPlot(_) => "boxplot",
        }
    }
    pub fn decorations(&self) -> &DecorationLayout {
        match self {
            Self::Radial(c) => &c.decorations,
            Self::Categorical(c) => &c.decorations,
            Self::BoxPlot(c) => &c.decorations,
        }
    }
    pub fn decorations_mut(&mut self) -> &mut DecorationLayout {
        match self {
            Self::Radial(c) => &mut c.decorations,
            Self::Categorical(c) => &mut c.decorations,
            Self::BoxPlot(c) => &mut c.decorations,
        }
    }
    #[allow(dead_code)]
    pub fn radial(&self) -> Option<&RadialChart> {
        if let Self::Radial(c) = self {
            Some(c)
        } else {
            None
        }
    }
    pub fn validate(&self, rect: Rect) -> Result<(), JsValue> {
        if rect.width < 160 || rect.height < 160 {
            return Err(js_err(
                "bounded chart canvas must be at least 160x160 pixels",
            ));
        }
        match self {
            Self::Radial(c) => c.validate().map_err(js_err),
            Self::Categorical(c) => c.validate().map_err(js_err),
            Self::BoxPlot(c) => c.validate().map_err(js_err),
        }
    }
    pub fn selected_json(&self) -> serde_json::Value {
        match self {
            Self::Radial(c) => json!(c.interaction.selected),
            Self::Categorical(c) => json!(c.selected),
            Self::BoxPlot(c) => json!(c.selected),
        }
    }
    pub fn clear_selection(&mut self) {
        self.decorations_mut().selected = None;
        match self {
            Self::Radial(c) => c.interaction.selected = None,
            Self::Categorical(c) => c.selected = None,
            Self::BoxPlot(c) => c.selected = None,
        }
    }
    pub fn select_at(&mut self, frame: &DocumentFrame, point: [f32; 2], toggle: bool) {
        match (self, frame) {
            (Self::Radial(c), DocumentFrame::Radial(f)) => {
                let hit = f.hit_test(point);
                c.interaction.selected = if toggle && hit == c.interaction.selected {
                    None
                } else {
                    hit
                };
            }
            (Self::Categorical(c), DocumentFrame::Categorical(f)) => {
                let hit = f.hit_test(point);
                c.selected = if toggle && hit == c.selected {
                    None
                } else {
                    hit
                };
            }
            (Self::BoxPlot(c), DocumentFrame::BoxPlot(f)) => {
                let hit = f.hit_test(point).map(|p| p.target);
                c.selected = if toggle && hit == c.selected {
                    None
                } else {
                    hit
                };
            }
            _ => unreachable!(),
        }
    }
    pub fn hover(&mut self, frame: &DocumentFrame, point: Option<[f32; 2]>, dt: f32) -> bool {
        match (self, frame) {
            (Self::Radial(c), DocumentFrame::Radial(f)) => {
                let hit = point.and_then(|p| {
                    f.hit_test(p).or_else(|| {
                        f.hit_test_at_rest(p)
                            .filter(|t| Some(*t) == c.interaction.hovered)
                    })
                });
                c.interaction.animate_hover(hit, dt, 0.16)
            }
            (Self::Categorical(c), DocumentFrame::Categorical(f)) => {
                let hit = point.and_then(|p| f.hit_test(p));
                let changed = hit != c.hovered;
                c.hovered = hit;
                changed
            }
            (Self::BoxPlot(c), DocumentFrame::BoxPlot(f)) => {
                let hit = point.and_then(|p| f.hit_test(p)).map(|p| p.target);
                let changed = hit != c.hovered;
                c.hovered = hit;
                changed
            }
            _ => unreachable!(),
        }
    }
    pub fn step(&mut self, backwards: bool) {
        fn next<T: PartialEq + Clone>(items: Vec<T>, selected: &mut Option<T>, backwards: bool) {
            if items.is_empty() {
                return;
            }
            let i = selected
                .as_ref()
                .and_then(|s| items.iter().position(|x| x == s));
            let n = items.len();
            let i = match i {
                Some(i) if backwards => (i + n - 1) % n,
                Some(i) => (i + 1) % n,
                None if backwards => n - 1,
                None => 0,
            };
            *selected = Some(items[i].clone());
        }
        match self {
            Self::Radial(c) => {
                let mut items = c
                    .slices
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.value > 0.)
                    .map(|(i, _)| RadialTarget::main(i))
                    .collect::<Vec<_>>();
                if let Some(split) = &c.split {
                    items.extend(
                        split
                            .children
                            .iter()
                            .enumerate()
                            .filter(|(_, s)| s.value > 0.)
                            .map(|(i, _)| RadialTarget::detail(i)),
                    );
                }
                next(items, &mut c.interaction.selected, backwards);
            }
            Self::Categorical(c) => {
                let items = c
                    .categories
                    .iter()
                    .enumerate()
                    .flat_map(|(i, cat)| {
                        c.series.iter().filter_map(move |s| {
                            s.values[i].map(|_| CategoryBarTarget::new(&cat.id, &s.id))
                        })
                    })
                    .collect();
                next(items, &mut c.selected, backwards);
            }
            Self::BoxPlot(c) => {
                let items = c
                    .categories
                    .iter()
                    .enumerate()
                    .flat_map(|(i, cat)| {
                        c.series.iter().filter_map(move |s| {
                            s.values[i]
                                .as_ref()
                                .map(|_| BoxPlotTarget::new(&cat.id, &s.id))
                        })
                    })
                    .collect();
                next(items, &mut c.selected, backwards);
            }
        }
    }
}

enum DocumentRenderer {
    Radial(RadialRenderer),
    Categorical(CategoricalRenderer),
    BoxPlot(BoxPlotRenderer),
}
enum DocumentFrame {
    Radial(Arc<RadialFrame>),
    Categorical(Arc<CategoricalFrame>),
    BoxPlot(Arc<BoxPlotFrame>),
}
impl DocumentRenderer {
    pub fn new(
        gpu: RendererDevice,
        chart: &DocumentChart,
        format: wgpu::TextureFormat,
    ) -> Result<Self, JsValue> {
        Ok(match chart {
            DocumentChart::Radial(_) => {
                Self::Radial(RadialRenderer::new(gpu, format).map_err(js_err)?)
            }
            DocumentChart::Categorical(_) => {
                Self::Categorical(CategoricalRenderer::new(gpu, format).map_err(js_err)?)
            }
            DocumentChart::BoxPlot(_) => {
                Self::BoxPlot(BoxPlotRenderer::new(gpu, format).map_err(js_err)?)
            }
        })
    }
    pub fn prepare(
        &mut self,
        chart: &DocumentChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<DocumentFrame, JsValue> {
        match (self, chart) {
            (Self::Radial(r), DocumentChart::Radial(c)) => Ok(DocumentFrame::Radial(
                r.prepare(c, size, scale).map_err(js_err)?,
            )),
            (Self::Categorical(r), DocumentChart::Categorical(c)) => Ok(
                DocumentFrame::Categorical(r.prepare(c, size, scale).map_err(js_err)?),
            ),
            (Self::BoxPlot(r), DocumentChart::BoxPlot(c)) => Ok(DocumentFrame::BoxPlot(
                r.prepare(c, size, scale).map_err(js_err)?,
            )),
            _ => Err(js_err("cannot change chart family")),
        }
    }
    pub async fn export(
        &mut self,
        chart: &DocumentChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<renderer::RasterImage, JsValue> {
        match (self, chart) {
            (Self::Radial(r), DocumentChart::Radial(c)) => {
                r.export_rgba_async(c, size, scale).await.map_err(js_err)
            }
            (Self::Categorical(r), DocumentChart::Categorical(c)) => {
                r.export_rgba_async(c, size, scale).await.map_err(js_err)
            }
            (Self::BoxPlot(r), DocumentChart::BoxPlot(c)) => {
                r.export_rgba_async(c, size, scale).await.map_err(js_err)
            }
            _ => Err(js_err("cannot change chart family")),
        }
    }
    #[allow(dead_code)]
    pub fn gpu_bytes(&self) -> u64 {
        match self {
            Self::Radial(r) => r.gpu_memory_usage().total_bytes(),
            Self::Categorical(r) => r.gpu_memory_usage().total_bytes(),
            Self::BoxPlot(r) => r.gpu_memory_usage().total_bytes(),
        }
    }
    pub fn end_frame(&self) {
        match self {
            Self::Radial(r) => r.end_frame(),
            Self::Categorical(r) => r.end_frame(),
            Self::BoxPlot(r) => r.end_frame(),
        }
    }
}
impl DocumentFrame {
    pub fn decorations(&self) -> &renderer::decoration::DecorationMap {
        match self {
            Self::Radial(f) => f.decorations(),
            Self::Categorical(f) => f.decorations(),
            Self::BoxPlot(f) => f.decorations(),
        }
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        match self {
            Self::Radial(f) => f.draw(pass),
            Self::Categorical(f) => f.draw(pass),
            Self::BoxPlot(f) => f.draw(pass),
        }
    }
    pub fn has_data(&self, p: [f32; 2]) -> bool {
        match self {
            Self::Radial(f) => f.hit_test(p).is_some(),
            Self::Categorical(f) => f.hit_test(p).is_some(),
            Self::BoxPlot(f) => f.hit_test(p).is_some(),
        }
    }
}

fn parse_chart(family: &str, value: &str) -> Result<DocumentChart, JsValue> {
    Ok(match family {
        "radial" => DocumentChart::Radial(serde_json::from_str(value).map_err(js_err)?),
        "categorical" => DocumentChart::Categorical(serde_json::from_str(value).map_err(js_err)?),
        "boxplot" => DocumentChart::BoxPlot(serde_json::from_str(value).map_err(js_err)?),
        _ => return Err(js_err("unknown bounded chart family")),
    })
}
#[wasm_bindgen]
pub struct FiggyBoundedChart {
    surface: wgpu::Surface<'static>,
    _instance: wgpu::Instance,
    _adapter: wgpu::Adapter,
    gpu: RendererDevice,
    surface_config: wgpu::SurfaceConfiguration,
    chart: DocumentChart,
    renderer: DocumentRenderer,
    frame: DocumentFrame,
    size: (u32, u32),
}
#[wasm_bindgen]
impl FiggyBoundedChart {
    /// One GPU surface; no kernel PNG transport is used for interactive frames.
    pub async fn create(
        canvas: HtmlCanvasElement,
        family: &str,
        value: &str,
        width: u32,
        height: u32,
    ) -> Result<FiggyBoundedChart, JsValue> {
        console_error_panic_hook::set_once();
        let chart = parse_chart(family, value)?;
        chart.validate(Rect {
            x: 0,
            y: 0,
            width,
            height,
        })?;
        let instance = renderer::data_render::create_instance();
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(js_err)?;
        let adapter = renderer::data_render::request_adapter_for_surface_async(&instance, &surface)
            .await
            .map_err(js_err)?;
        let (device, queue) = renderer::data_render::request_device_async(&adapter)
            .await
            .map_err(js_err)?;
        let config = renderer::data_render::try_configure_surface(
            &surface,
            &adapter,
            &device,
            canvas.width(),
            canvas.height(),
        )
        .map_err(js_err)?;
        let gpu = RendererDevice::new(Arc::new(device), Arc::new(queue));
        let mut renderer = DocumentRenderer::new(gpu.clone(), &chart, config.format)?;
        let frame = renderer.prepare(&chart, (width, height), 1.)?;
        Ok(Self {
            surface,
            _instance: instance,
            _adapter: adapter,
            gpu,
            surface_config: config,
            chart,
            renderer,
            frame,
            size: (width, height),
        })
    }
    pub fn get_document(&self) -> Result<String, JsValue> {
        match &self.chart {
            DocumentChart::Radial(c) => serde_json::to_string(c).map_err(js_err),
            DocumentChart::Categorical(c) => serde_json::to_string(c).map_err(js_err),
            DocumentChart::BoxPlot(c) => serde_json::to_string(c).map_err(js_err),
        }
    }
    /// Validate and prepare before replacing the SSOT, retaining the old chart on failure.
    pub fn set_document(&mut self, value: &str, width: u32, height: u32) -> Result<(), JsValue> {
        let next = parse_chart(self.chart.family(), value)?;
        next.validate(Rect {
            x: 0,
            y: 0,
            width,
            height,
        })?;
        let frame = self.renderer.prepare(&next, (width, height), 1.)?;
        self.chart = next;
        self.frame = frame;
        self.size = (width, height);
        Ok(())
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        self.surface_config.width = width.max(1);
        self.surface_config.height = height.max(1);
        self.surface
            .configure(self.gpu.device(), &self.surface_config);
    }
    pub fn frame(&mut self) -> Result<(), JsValue> {
        let scale = (self.surface_config.width as f32 / self.size.0 as f32)
            .min(self.surface_config.height as f32 / self.size.1 as f32)
            .clamp(0.5, 4.);
        self.frame = self.renderer.prepare(&self.chart, self.size, scale)?;
        let surface = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface
                    .configure(self.gpu.device(), &self.surface_config);
                return Ok(());
            }
            e => return Err(js_err(format!("surface acquisition: {e:?}"))),
        };
        let view = surface.texture.create_view(&Default::default());
        let mut encoder = self
            .gpu
            .device()
            .create_command_encoder(&Default::default());
        {
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
            self.frame.draw(&mut pass);
        }
        self.gpu.queue().submit([encoder.finish()]);
        self.gpu.queue().present(surface);
        self.renderer.end_frame();
        Ok(())
    }
    pub async fn export_png(&mut self, scale: f32) -> Result<js_sys::Uint8Array, JsValue> {
        let image = self.renderer.export(&self.chart, self.size, scale).await?;
        let bytes = renderer::encode_png(&image).map_err(js_err)?;
        Ok(js_sys::Uint8Array::from(bytes.as_slice()))
    }
    /// Renderer-measured logical pixel bounds and resize handles.
    pub fn decorations(&self) -> Result<String, JsValue> {
        let map = self.frame.decorations();
        let values=map.entries().filter_map(|(id,e)|map.selection_box(id).map(|b|json!({"id":e.id.element_id(),"rect":[b.rect.x,b.rect.y,b.rect.width,b.rect.height],"handles":b.handles.iter().map(|r|vec![r.x,r.y,r.width,r.height]).collect::<Vec<_>>()}))).collect::<Vec<_>>();
        serde_json::to_string(&values).map_err(js_err)
    }
    pub fn hit_decoration(&self, x: f32, y: f32, edit_area: bool) -> Option<String> {
        let map = self.frame.decorations();
        let id = map.hit_test([x, y])?;
        let e = map.element(id)?;
        if e.id == renderer::DecorationId::DataArea
            && !edit_area
            && self.chart.decorations().selected != Some(e.id.clone())
            && self.frame.has_data([x, y])
        {
            return None;
        }
        Some(e.id.element_id())
    }
    pub fn select_decoration(&mut self, target: Option<String>) -> Result<(), JsValue> {
        let selected = match target {
            Some(name) => Some(
                self.frame
                    .decorations()
                    .entries()
                    .find(|(_, e)| e.id.element_id() == name)
                    .ok_or_else(|| js_err("unknown decoration"))?
                    .1
                    .id
                    .clone(),
            ),
            None => None,
        };
        self.chart.clear_selection();
        self.chart.decorations_mut().selected = selected;
        Ok(())
    }
    pub fn resize_handle(&self, target: &str, x: f32, y: f32) -> Option<String> {
        let map = self.frame.decorations();
        let (id, _) = map.entries().find(|(_, e)| e.id.element_id() == target)?;
        map.hit_resize_handle(id, [x, y]).map(|h| format!("{h:?}"))
    }
    pub fn drag(
        &mut self,
        target: &str,
        dx: f32,
        dy: f32,
        handle: Option<String>,
    ) -> Result<bool, JsValue> {
        if !dx.is_finite() || !dy.is_finite() {
            return Err(js_err("drag must be finite"));
        }
        use renderer::resize::ResizeHandle::*;
        let map = self.frame.decorations();
        let (id, _) = map
            .entries()
            .find(|(_, e)| e.id.element_id() == target)
            .ok_or_else(|| js_err("unknown decoration"))?;
        let mut next = self.chart.clone();
        let result = if let Some(name) = handle {
            let h = [N, S, W, E, NW, NE, SW, SE]
                .into_iter()
                .find(|h| format!("{h:?}") == name)
                .ok_or_else(|| js_err("unknown resize handle"))?;
            map.resize_by(next.decorations_mut(), id, h, dx, dy)
        } else {
            map.drag_by(next.decorations_mut(), id, dx, dy)
        };
        if result != Some(renderer::layout::NudgeResult::Moved) {
            return Ok(false);
        }
        let frame = self.renderer.prepare(&next, self.size, 1.)?;
        self.chart = next;
        self.frame = frame;
        Ok(true)
    }
    pub fn pick(&mut self, x: f32, y: f32, toggle: bool) -> Result<String, JsValue> {
        self.chart.decorations_mut().selected = None;
        self.chart.select_at(&self.frame, [x, y], toggle);
        serde_json::to_string(&self.chart.selected_json()).map_err(js_err)
    }
    pub fn hover(&mut self, x: Option<f32>, y: Option<f32>, dt: f32) -> bool {
        self.chart
            .hover(&self.frame, x.zip(y).map(|(x, y)| [x, y]), dt.min(0.1))
    }
    pub fn clear_selection(&mut self) {
        self.chart.clear_selection();
    }
    pub fn step_selection(&mut self, backwards: bool) {
        self.chart.step(backwards);
    }
}
