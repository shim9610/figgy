//! Raw WASM host for radial, categorical, box and radar charts. The renderer owns all geometry.
use renderer::{
    AnnotationLayer, BoundedAnnotationFrame, DecorationId, DecorationLayout, FrameBackground,
    LayerHit, LayerRef, RendererDevice, boxplot::*, categorical::*, layout::Rect, radar::*,
    radial::*,
};
use serde_json::json;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
fn js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}
/// A radial pick request read back: the slice the GPU picked, if any.
async fn picked(
    ticket: Result<RadialPickTicket, RadialError>,
) -> Result<Option<RadialTarget>, JsValue> {
    ticket.map_err(js_err)?.resolve().await.map_err(js_err)
}
/// The slice under `point` of a radial frame, keeping the hovered slice
/// while the pointer is in its unlifted footprint (hover hysteresis).
async fn radial_at(
    frame: &Arc<RadialFrame>,
    point: [f32; 2],
    hovered: Option<RadialTarget>,
) -> Result<Option<RadialTarget>, JsValue> {
    if let Some(hit) = picked(frame.pick(point)).await? {
        return Ok(Some(hit));
    }
    Ok(picked(frame.pick_at_rest(point))
        .await?
        .filter(|t| Some(*t) == hovered))
}
#[derive(Clone, PartialEq)]
enum DocumentChart {
    Radial(RadialChart),
    Categorical(CategoricalChart),
    BoxPlot(BoxPlotChart),
    Radar(RadarChart),
}
impl DocumentChart {
    pub fn family(&self) -> &'static str {
        match self {
            Self::Radial(_) => "radial",
            Self::Categorical(_) => "categorical",
            Self::BoxPlot(_) => "boxplot",
            Self::Radar(_) => "radar",
        }
    }
    pub fn decorations(&self) -> &DecorationLayout {
        match self {
            Self::Radial(c) => &c.decorations,
            Self::Categorical(c) => &c.decorations,
            Self::BoxPlot(c) => &c.decorations,
            Self::Radar(c) => &c.decorations,
        }
    }
    pub fn decorations_mut(&mut self) -> &mut DecorationLayout {
        match self {
            Self::Radial(c) => &mut c.decorations,
            Self::Categorical(c) => &mut c.decorations,
            Self::BoxPlot(c) => &mut c.decorations,
            Self::Radar(c) => &mut c.decorations,
        }
    }
    pub fn active_layer(&self) -> LayerRef {
        match self {
            Self::Radial(c) => c.active_layer(),
            Self::Categorical(c) => c.active_layer(),
            Self::BoxPlot(c) => c.active_layer(),
            Self::Radar(c) => c.active_layer(),
        }
    }
    pub fn set_active_layer(&mut self, layer: LayerRef) -> LayerRef {
        match self {
            Self::Radial(c) => c.set_active_layer(layer),
            Self::Categorical(c) => c.set_active_layer(layer),
            Self::BoxPlot(c) => c.set_active_layer(layer),
            Self::Radar(c) => c.set_active_layer(layer),
        }
    }
    pub fn annotations(&self) -> &AnnotationLayer {
        match self {
            Self::Radial(c) => &c.annotations,
            Self::Categorical(c) => &c.annotations,
            Self::BoxPlot(c) => &c.annotations,
            Self::Radar(c) => &c.annotations,
        }
    }
    pub fn annotations_mut(&mut self) -> &mut AnnotationLayer {
        match self {
            Self::Radial(c) => &mut c.annotations,
            Self::Categorical(c) => &mut c.annotations,
            Self::BoxPlot(c) => &mut c.annotations,
            Self::Radar(c) => &mut c.annotations,
        }
    }
    pub fn background(&self) -> renderer::Color {
        match self {
            Self::Radial(c) => c.background,
            Self::Categorical(c) => c.background,
            Self::BoxPlot(c) => c.background,
            Self::Radar(c) => c.background,
        }
    }
    /// A visible span needs the frame to leave the background to the host
    /// clear, so the underlay drawn before it shows through. Charts without
    /// one keep today's opaque frame and white clear exactly.
    pub fn frame_background(&self) -> FrameBackground {
        let layer = self.annotations();
        if layer.visible && layer.items.iter().any(|i| i.visible && i.is_underlay()) {
            FrameBackground::HostClear
        } else {
            FrameBackground::Opaque
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
            Self::Radar(c) => c.validate().map_err(js_err),
        }
    }
    pub fn selected_json(&self) -> serde_json::Value {
        match self {
            Self::Radial(c) => json!(c.interaction.selected),
            Self::Categorical(c) => json!(c.selected),
            Self::BoxPlot(c) => json!(c.selected),
            Self::Radar(c) => json!(c.selected),
        }
    }
    pub fn clear_selection(&mut self) {
        self.decorations_mut().selected = None;
        match self {
            Self::Radial(c) => c.interaction.selected = None,
            Self::Categorical(c) => c.selected = None,
            Self::BoxPlot(c) => c.selected = None,
            Self::Radar(c) => c.selected = None,
        }
    }
    /// Select the data under `point`. A radial chart picks on the GPU, so
    /// this resolves asynchronously.
    pub async fn select_at(
        &mut self,
        frame: &DocumentFrame,
        point: [f32; 2],
        toggle: bool,
    ) -> Result<(), JsValue> {
        match (self, frame) {
            (Self::Radial(c), DocumentFrame::Radial(f)) => {
                let hit = radial_at(f, point, c.interaction.hovered).await?;
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
            (Self::Radar(c), DocumentFrame::Radar(f)) => {
                let hit = f.hit_test(point).map(|p| p.target);
                c.selected = if toggle && hit == c.selected {
                    None
                } else {
                    hit
                };
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    /// Hover the data under `point`, or none. A radial chart picks on the
    /// GPU, so this resolves asynchronously.
    pub async fn hover(
        &mut self,
        frame: &DocumentFrame,
        point: Option<[f32; 2]>,
        dt: f32,
    ) -> Result<bool, JsValue> {
        Ok(match (self, frame) {
            (Self::Radial(c), DocumentFrame::Radial(f)) => {
                let hit = match point {
                    Some(p) => radial_at(f, p, c.interaction.hovered).await?,
                    None => None,
                };
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
            (Self::Radar(c), DocumentFrame::Radar(f)) => {
                let hit = point.and_then(|p| f.hit_test(p)).map(|p| p.target);
                let changed = hit != c.hovered;
                c.hovered = hit;
                changed
            }
            _ => unreachable!(),
        })
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
            Self::Radar(c) => {
                // A selected vertex steps along its series' finite vertices;
                // otherwise the series polygons are stepped in order.
                let vertex = c
                    .selected
                    .as_ref()
                    .and_then(|t| t.category_id.as_ref().and_then(|_| c.series(&t.series_id)));
                let items = match vertex {
                    Some(s) => c
                        .categories
                        .iter()
                        .zip(&s.values)
                        .filter(|(_, v)| v.is_finite())
                        .map(|(cat, _)| RadarTarget::vertex(&s.id, &cat.id))
                        .collect(),
                    None => c
                        .series
                        .iter()
                        .map(|s| RadarTarget::series(&s.id))
                        .collect(),
                };
                next(items, &mut c.selected, backwards);
            }
        }
    }
}

enum DocumentRenderer {
    Radial(RadialRenderer),
    Categorical(CategoricalRenderer),
    BoxPlot(BoxPlotRenderer),
    Radar(RadarRenderer),
}
enum DocumentFrame {
    Radial(Arc<RadialFrame>),
    Categorical(Arc<CategoricalFrame>),
    BoxPlot(Arc<BoxPlotFrame>),
    Radar(Arc<RadarFrame>),
}
impl DocumentRenderer {
    pub async fn new_async(
        gpu: RendererDevice,
        chart: &DocumentChart,
        format: wgpu::TextureFormat,
    ) -> Result<Self, JsValue> {
        Ok(match chart {
            DocumentChart::Radial(_) => Self::Radial(
                RadialRenderer::new_async(gpu, format)
                    .await
                    .map_err(js_err)?,
            ),
            DocumentChart::Categorical(_) => Self::Categorical(
                CategoricalRenderer::new_async(gpu, format)
                    .await
                    .map_err(js_err)?,
            ),
            DocumentChart::BoxPlot(_) => Self::BoxPlot(
                BoxPlotRenderer::new_async(gpu, format)
                    .await
                    .map_err(js_err)?,
            ),
            DocumentChart::Radar(_) => Self::Radar(
                RadarRenderer::new_async(gpu, format)
                    .await
                    .map_err(js_err)?,
            ),
        })
    }
    /// The chart frame and its annotation layer for one document state.
    pub fn prepare(
        &mut self,
        chart: &DocumentChart,
        size: (u32, u32),
        scale: f32,
    ) -> Result<(DocumentFrame, Arc<BoundedAnnotationFrame>), JsValue> {
        let background = chart.frame_background();
        match (self, chart) {
            (Self::Radial(r), DocumentChart::Radial(c)) => {
                r.set_background(background);
                let frame = r.prepare(c, size, scale).map_err(js_err)?;
                let annotations = r.prepare_annotations(c, size, scale).map_err(js_err)?;
                Ok((DocumentFrame::Radial(frame), annotations))
            }
            (Self::Categorical(r), DocumentChart::Categorical(c)) => {
                r.set_background(background);
                let frame = r.prepare(c, size, scale).map_err(js_err)?;
                let annotations = r.prepare_annotations(c, size, scale).map_err(js_err)?;
                Ok((DocumentFrame::Categorical(frame), annotations))
            }
            (Self::BoxPlot(r), DocumentChart::BoxPlot(c)) => {
                r.set_background(background);
                let frame = r.prepare(c, size, scale).map_err(js_err)?;
                let annotations = r.prepare_annotations(c, size, scale).map_err(js_err)?;
                Ok((DocumentFrame::BoxPlot(frame), annotations))
            }
            (Self::Radar(r), DocumentChart::Radar(c)) => {
                r.set_background(background);
                let frame = r.prepare(c, size, scale).map_err(js_err)?;
                let annotations = r.prepare_annotations(c, size, scale).map_err(js_err)?;
                Ok((DocumentFrame::Radar(frame), annotations))
            }
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
            (Self::Radar(r), DocumentChart::Radar(c)) => {
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
            Self::Radar(r) => r.gpu_memory_usage().total_bytes(),
        }
    }
    pub fn end_frame(&self) {
        match self {
            Self::Radial(r) => r.end_frame(),
            Self::Categorical(r) => r.end_frame(),
            Self::BoxPlot(r) => r.end_frame(),
            Self::Radar(r) => r.end_frame(),
        }
    }
}
impl DocumentFrame {
    /// The decoration map of a categorical, box plot or radar frame; the
    /// radial frame answers through its `HitMap<RadialChart>` with the
    /// chart instead.
    pub fn decoration_map(&self) -> Option<&renderer::decoration::DecorationMap> {
        match self {
            Self::Radial(_) => None,
            Self::Categorical(f) => Some(f.decorations()),
            Self::BoxPlot(f) => Some(f.decorations()),
            Self::Radar(f) => Some(f.decorations()),
        }
    }
    /// The hit-map entry of the decoration named `name` (its stable
    /// element id).
    fn decoration_named(&self, name: &str) -> Option<renderer::HitId> {
        match self {
            Self::Radial(f) => f.decorations().find(name),
            _ => self.decoration_map()?.hit_map().find(name),
        }
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        match self {
            Self::Radial(f) => f.draw(pass),
            Self::Categorical(f) => f.draw(pass),
            Self::BoxPlot(f) => f.draw(pass),
            Self::Radar(f) => f.draw(pass),
        }
    }
    /// Whether data is drawn under `p`; a radial frame picks on the GPU.
    pub async fn has_data(&self, p: [f32; 2]) -> Result<bool, JsValue> {
        Ok(match self {
            Self::Radial(f) => {
                picked(f.pick(p)).await?.is_some() || picked(f.pick_at_rest(p)).await?.is_some()
            }
            Self::Categorical(f) => f.hit_test(p).is_some(),
            Self::BoxPlot(f) => f.hit_test(p).is_some(),
            Self::Radar(f) => f.hit_test(p).is_some(),
        })
    }
}

fn parse_chart(family: &str, value: &str) -> Result<DocumentChart, JsValue> {
    Ok(match family {
        "radial" => {
            let mut chart: RadialChart = serde_json::from_str(value).map_err(js_err)?;
            // A slice without its drawn text gets the one its label format
            // writes; a text the document carries is drawn as it is.
            chart.normalize_labels();
            DocumentChart::Radial(chart)
        }
        "categorical" => DocumentChart::Categorical(serde_json::from_str(value).map_err(js_err)?),
        "boxplot" => DocumentChart::BoxPlot(serde_json::from_str(value).map_err(js_err)?),
        "radar" => DocumentChart::Radar(serde_json::from_str(value).map_err(js_err)?),
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
    annotations: Arc<BoundedAnnotationFrame>,
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
        let mut renderer = DocumentRenderer::new_async(gpu.clone(), &chart, config.format).await?;
        let scale = surface_scale(&config, (width, height));
        let (frame, annotations) = renderer.prepare(&chart, (width, height), scale)?;
        Ok(Self {
            surface,
            _instance: instance,
            _adapter: adapter,
            gpu,
            surface_config: config,
            chart,
            renderer,
            frame,
            annotations,
            size: (width, height),
        })
    }
    pub fn get_document(&self) -> Result<String, JsValue> {
        match &self.chart {
            DocumentChart::Radial(c) => serde_json::to_string(c).map_err(js_err),
            DocumentChart::Categorical(c) => serde_json::to_string(c).map_err(js_err),
            DocumentChart::BoxPlot(c) => serde_json::to_string(c).map_err(js_err),
            DocumentChart::Radar(c) => serde_json::to_string(c).map_err(js_err),
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
        let scale = surface_scale(&self.surface_config, (width, height));
        let (frame, annotations) = self.renderer.prepare(&next, (width, height), scale)?;
        self.chart = next;
        self.frame = frame;
        self.annotations = annotations;
        self.size = (width, height);
        Ok(())
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        self.surface_config.width = width.max(1);
        self.surface_config.height = height.max(1);
        self.surface
            .configure(self.gpu.device(), &self.surface_config);
    }
    /// The scale the surface shows the chart at; every frame, and the hit
    /// maps an edit answers from until the next frame, are prepared at it.
    fn surface_scale(&self) -> f32 {
        surface_scale(&self.surface_config, self.size)
    }
    pub fn frame(&mut self) -> Result<(), JsValue> {
        let scale = self.surface_scale();
        let (frame, annotations) = self.renderer.prepare(&self.chart, self.size, scale)?;
        self.frame = frame;
        self.annotations = annotations;
        // A frame that leaves its background to the clear needs the chart
        // background there; every other frame keeps the white clear it had.
        let clear = match self.chart.frame_background() {
            FrameBackground::HostClear => {
                let c = self.chart.background();
                wgpu::Color {
                    r: f64::from(c.r),
                    g: f64::from(c.g),
                    b: f64::from(c.b),
                    a: f64::from(c.a),
                }
            }
            FrameBackground::Opaque => wgpu::Color::WHITE,
        };
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
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            // Fixed order: spans below the chart, every other annotation and
            // the annotation selection box above all decorations.
            self.annotations.draw_underlay(&mut pass);
            self.frame.draw(&mut pass);
            self.annotations.draw_overlay(&mut pass);
        }
        self.gpu.queue().submit([encoder.finish()]);
        self.gpu.queue().present(surface);
        self.renderer.end_frame();
        Ok(())
    }
    /// Submit one frame and wait for actual GPU completion. Hosts must await
    /// this before publishing Ready; rejected completion (including device
    /// loss) is returned to the caller instead of reporting an unusable chart.
    pub async fn first_frame_ready(&mut self) -> Result<(), JsValue> {
        self.frame()?;
        renderer::init::wait_browser_submitted_work(self.gpu.queue())
            .await
            .map_err(js_err)
    }

    pub async fn export_png(&mut self, scale: f32) -> Result<js_sys::Uint8Array, JsValue> {
        let image = self.renderer.export(&self.chart, self.size, scale).await?;
        let bytes = renderer::encode_png(&image).map_err(js_err)?;
        Ok(js_sys::Uint8Array::from(bytes.as_slice()))
    }
    /// Renderer-measured logical pixel bounds and resize handles.
    pub fn decorations(&self) -> Result<String, JsValue> {
        let entry = |id: &DecorationId, b: renderer::SelectionBox| {
            let r = b.rect();
            json!({"id":id.element_id(),"rect":[r.x,r.y,r.width,r.height],"handles":b.handles().iter().map(|r|vec![r.x,r.y,r.width,r.height]).collect::<Vec<_>>()})
        };
        let values = match (&self.frame, &self.chart) {
            (DocumentFrame::Radial(f), DocumentChart::Radial(c)) => f
                .decoration_ids(c)
                .filter_map(|(hit, id)| Some(entry(&id, f.decoration_selection_box(c, hit)?)))
                .collect::<Vec<_>>(),
            (frame, _) => {
                let map = frame
                    .decoration_map()
                    .ok_or_else(|| js_err("chart and frame families differ"))?;
                map.entries()
                    .filter_map(|(id, e)| Some(entry(&e.id, map.selection_box(id)?)))
                    .collect::<Vec<_>>()
            }
        };
        serde_json::to_string(&values).map_err(js_err)
    }
    /// The topmost decoration under a point, as its stable id. While the
    /// annotation layer is active (and shown) its drawn items sit on top of
    /// every decoration and answer as `"annotation_<n>"`; on the base layer
    /// they are not registered and clicks pass through them.
    ///
    /// Asynchronous: over the plot area it asks whether data is drawn
    /// there, which a radial chart answers with its GPU pick.
    pub async fn hit_decoration(
        &self,
        x: f32,
        y: f32,
        edit_area: bool,
    ) -> Result<Option<String>, JsValue> {
        let layer = self.chart.active_layer();
        let hit = match (&self.frame, &self.chart) {
            (DocumentFrame::Radial(f), DocumentChart::Radial(c)) => self
                .annotations
                .route_active_layer(layer, [x, y], || f.decoration_at(c, [x, y])),
            (frame, _) => match frame.decoration_map() {
                Some(map) => self.annotations.hit_test_active_layer(layer, map, [x, y]),
                None => None,
            },
        };
        let id = match hit {
            Some(LayerHit::Annotation { index, .. }) => {
                return Ok(Some(DecorationId::Annotation(index).element_id()));
            }
            Some(LayerHit::Element(id)) => id,
            None => return Ok(None),
        };
        let key = match (&self.frame, &self.chart) {
            (DocumentFrame::Radial(f), DocumentChart::Radial(c)) => f.decoration_id(c, id),
            (frame, _) => frame
                .decoration_map()
                .and_then(|map| map.element(id))
                .map(|e| e.id.clone()),
        };
        let Some(key) = key else {
            return Ok(None);
        };
        if key == renderer::DecorationId::DataArea
            && !edit_area
            && self.chart.decorations().selected != Some(key.clone())
            && self.frame.has_data([x, y]).await?
        {
            return Ok(None);
        }
        Ok(Some(key.element_id()))
    }
    /// Select a decoration by the id `hit_decoration` reported, or none.
    /// An `"annotation_<n>"` id selects that drawn annotation
    /// (`decorations.selected = Annotation(n)`).
    pub fn select_decoration(&mut self, target: Option<String>) -> Result<(), JsValue> {
        let selected = match target {
            Some(name) => Some(
                self.drawn_annotation(&name)
                    .map(DecorationId::Annotation)
                    .or_else(|| match (&self.frame, &self.chart) {
                        (DocumentFrame::Radial(f), DocumentChart::Radial(c)) => f
                            .decoration_ids(c)
                            .find(|(_, id)| id.element_id() == name)
                            .map(|(_, id)| id),
                        (frame, _) => frame
                            .decoration_map()?
                            .entries()
                            .find(|(_, e)| e.id.element_id() == name)
                            .map(|(_, e)| e.id.clone()),
                    })
                    .ok_or_else(|| js_err("unknown decoration"))?,
            ),
            None => None,
        };
        self.chart.clear_selection();
        self.chart.decorations_mut().selected = selected;
        Ok(())
    }
    /// The drawn annotation an `"annotation_<n>"` id names, as its index.
    fn drawn_annotation(&self, target: &str) -> Option<usize> {
        self.annotations
            .annotations()
            .entries()
            .find(|(_, e)| e.id().element_id() == target)
            .map(|(_, e)| e.index)
    }
    pub fn resize_handle(&self, target: &str, x: f32, y: f32) -> Option<String> {
        if let Some(index) = self.drawn_annotation(target) {
            return self.annotation_handle(index as u32, x, y);
        }
        let id = self.frame.decoration_named(target)?;
        match (&self.frame, &self.chart) {
            (DocumentFrame::Radial(f), DocumentChart::Radial(c)) => {
                f.decoration_resize_handle(c, id, [x, y])
            }
            (frame, _) => frame.decoration_map()?.hit_resize_handle(id, [x, y]),
        }
        .map(|h| format!("{h:?}"))
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
        if let Some(index) = self.drawn_annotation(target) {
            return self.drag_annotation(index as u32, dx, dy, handle);
        }
        use renderer::resize::ResizeHandle::*;
        let id = self
            .frame
            .decoration_named(target)
            .ok_or_else(|| js_err("unknown decoration"))?;
        let handle = handle
            .map(|name| {
                [N, S, W, E, NW, NE, SW, SE]
                    .into_iter()
                    .find(|h| format!("{h:?}") == name)
                    .ok_or_else(|| js_err("unknown resize handle"))
            })
            .transpose()?;
        let mut next = self.chart.clone();
        let result = match (&self.frame, &mut next) {
            // The radial decorations edit the chart they are placed from.
            (DocumentFrame::Radial(f), DocumentChart::Radial(c)) => match handle {
                Some(h) => f.resize_decoration(c, id, h, dx, dy),
                None => f.drag_decoration(c, id, dx, dy),
            },
            (frame, next) => {
                let map = frame
                    .decoration_map()
                    .ok_or_else(|| js_err("chart and frame families differ"))?;
                match handle {
                    Some(h) => map.resize_by(next.decorations_mut(), id, h, dx, dy),
                    None => map.drag_by(next.decorations_mut(), id, dx, dy),
                }
            }
        };
        if result != Some(renderer::layout::NudgeResult::Moved) {
            return Ok(false);
        }
        let (frame, annotations) = self
            .renderer
            .prepare(&next, self.size, self.surface_scale())?;
        self.chart = next;
        self.frame = frame;
        self.annotations = annotations;
        Ok(true)
    }
    /// Validate and prepare an edited document, then adopt it; the previous
    /// chart and frames stay on failure.
    fn adopt(&mut self, next: DocumentChart) -> Result<(), JsValue> {
        next.validate(Rect {
            x: 0,
            y: 0,
            width: self.size.0,
            height: self.size.1,
        })?;
        let (frame, annotations) = self
            .renderer
            .prepare(&next, self.size, self.surface_scale())?;
        self.chart = next;
        self.frame = frame;
        self.annotations = annotations;
        Ok(())
    }
    /// Set the layer that receives pointer interaction: `"Base"` or
    /// `"Annotations"` as JSON (bounded charts have no overlays, so an
    /// `{ "Overlay": n }` is stored as `"Base"`). A change clears the
    /// decoration selection (`decorations.selected`, an annotation
    /// included); the data selection is kept. Returns the stored layer as
    /// JSON. Drawn pixels never depend on it.
    pub fn set_active_layer(&mut self, json: &str) -> Result<String, JsValue> {
        let layer: LayerRef = serde_json::from_str(json).map_err(js_err)?;
        let mut next = self.chart.clone();
        let stored = next.set_active_layer(layer);
        if next != self.chart {
            self.adopt(next)?;
        }
        serde_json::to_string(&stored).map_err(js_err)
    }
    /// The layer that receives pointer interaction, as JSON.
    pub fn active_layer(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.chart.active_layer()).map_err(js_err)
    }
    fn annotation_hit_id(&self, index: usize) -> Result<renderer::HitId, JsValue> {
        self.annotations
            .annotations()
            .find(index)
            .ok_or_else(|| js_err("annotation is not drawn in the current frame"))
    }

    // Annotation layer. Coordinates are document logical pixels like the
    // decoration methods; the layer's items live in the document JSON.

    /// The drawn annotations as JSON: `index`, `id`, `kind`, the selection
    /// box `rect` and `handles`, and `selected`. Skipped items are listed by
    /// `annotation_warnings` instead.
    pub fn annotations(&self) -> Result<String, JsValue> {
        let map = self.annotations.annotations();
        let items = &self.chart.annotations().items;
        let selected = AnnotationLayer::selected_index(self.chart.decorations().selected.as_ref());
        let values = map
            .entries()
            .filter_map(|(id, e)| {
                let item = items.get(e.index)?;
                let b = map.selection_box(id)?;
                Some(json!({
                    "index": e.index,
                    "id": item.id,
                    "element": e.id().element_id(),
                    "kind": crate::annotation_kind_name(&item.kind),
                    "rect": [b.rect().x, b.rect().y, b.rect().width, b.rect().height],
                    "handles": b.handles().iter().map(|r| vec![r.x, r.y, r.width, r.height]).collect::<Vec<_>>(),
                    "selected": selected == Some(e.index),
                }))
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&values).map_err(js_err)
    }
    /// The topmost drawn annotation under a point, as its index.
    pub fn hit_annotation(&self, x: f32, y: f32) -> Option<u32> {
        let map = self.annotations.annotations();
        let id = map.hit_test([x, y])?;
        map.element(id).map(|e| e.index as u32)
    }
    /// Select one annotation (`decorations.selected = Annotation(index)`) or
    /// none. Clears the data and decoration selections like
    /// `select_decoration`.
    pub fn select_annotation(&mut self, index: Option<u32>) -> Result<(), JsValue> {
        let selected = match index {
            Some(i) if (i as usize) < self.chart.annotations().items.len() => {
                Some(DecorationId::Annotation(i as usize))
            }
            Some(_) => return Err(js_err("unknown annotation")),
            None => None,
        };
        let mut next = self.chart.clone();
        next.clear_selection();
        next.decorations_mut().selected = selected;
        self.adopt(next)
    }
    pub fn selected_annotation(&self) -> Option<u32> {
        AnnotationLayer::selected_index(self.chart.decorations().selected.as_ref())
            .map(|i| i as u32)
    }
    /// The compass name of the annotation's live handle under a point.
    pub fn annotation_handle(&self, index: u32, x: f32, y: f32) -> Option<String> {
        let id = self.annotation_hit_id(index as usize).ok()?;
        self.annotations
            .annotations()
            .hit_resize_handle(id, [x, y])
            .map(|h| format!("{h:?}"))
    }
    /// Move an annotation, or one of its handles, by a pointer delta.
    /// Returns `false` for a rejected move; the document is unchanged then.
    pub fn drag_annotation(
        &mut self,
        index: u32,
        dx: f32,
        dy: f32,
        handle: Option<String>,
    ) -> Result<bool, JsValue> {
        if !dx.is_finite() || !dy.is_finite() {
            return Err(js_err("drag must be finite"));
        }
        use renderer::resize::ResizeHandle::*;
        let id = self.annotation_hit_id(index as usize)?;
        let map = self.annotations.annotations();
        let mut next = self.chart.clone();
        let result = if let Some(name) = handle {
            let h = [N, S, W, E, NW, NE, SW, SE]
                .into_iter()
                .find(|h| format!("{h:?}") == name)
                .ok_or_else(|| js_err("unknown resize handle"))?;
            map.resize_by(next.annotations_mut(), id, h, dx, dy)
        } else {
            map.drag_by(next.annotations_mut(), id, dx, dy)
        };
        if result != Some(renderer::layout::NudgeResult::Moved) {
            return Ok(false);
        }
        self.adopt(next)?;
        Ok(true)
    }
    /// Append an `Annotation` JSON value and return its index.
    pub fn add_annotation(&mut self, json: &str) -> Result<u32, JsValue> {
        let item: renderer::Annotation = serde_json::from_str(json).map_err(js_err)?;
        let mut next = self.chart.clone();
        next.annotations_mut().items.push(item);
        let index = next.annotations().items.len() - 1;
        self.adopt(next)?;
        Ok(index as u32)
    }
    /// Replace the annotation at `index`.
    pub fn set_annotation(&mut self, index: u32, json: &str) -> Result<(), JsValue> {
        let item: renderer::Annotation = serde_json::from_str(json).map_err(js_err)?;
        let mut next = self.chart.clone();
        let slot = next
            .annotations_mut()
            .items
            .get_mut(index as usize)
            .ok_or_else(|| js_err("unknown annotation"))?;
        *slot = item;
        self.adopt(next)
    }
    /// Remove the annotation at `index`; later indices move down one and the
    /// selection follows or clears.
    pub fn remove_annotation(&mut self, index: u32) -> Result<(), JsValue> {
        let index = index as usize;
        let mut next = self.chart.clone();
        if index >= next.annotations().items.len() {
            return Err(js_err("unknown annotation"));
        }
        next.annotations_mut().items.remove(index);
        let selected = next.decorations().selected.clone();
        next.decorations_mut().selected = match selected {
            Some(DecorationId::Annotation(i)) if i == index => None,
            Some(DecorationId::Annotation(i)) if i > index => Some(DecorationId::Annotation(i - 1)),
            other => other,
        };
        self.adopt(next)
    }
    /// Show or hide the whole annotation layer without re-rasterizing it.
    pub fn set_annotations_visible(&mut self, visible: bool) -> Result<(), JsValue> {
        let mut next = self.chart.clone();
        next.annotations_mut().visible = visible;
        self.adopt(next)
    }
    /// The `Anchor` JSON at a point for `"Data"` or `"Ratio"`; `null` when
    /// the point has no such position (a radial chart has no data anchors).
    pub fn pointer_to_anchor(&self, x: f32, y: f32, kind: &str) -> Result<String, JsValue> {
        let kind = match kind {
            "Data" => renderer::AnchorKind::Data,
            "Ratio" => renderer::AnchorKind::Ratio,
            _ => return Err(js_err("anchor kind must be \"Data\" or \"Ratio\"")),
        };
        use renderer::AnnotationSpace;
        let anchor = self
            .annotations
            .annotations()
            .space()
            .px_to_anchor([x, y], kind);
        serde_json::to_string(&anchor).map_err(js_err)
    }
    /// Annotations the current frame skipped, as `[{index, reason}]`.
    pub fn annotation_warnings(&self) -> Result<String, JsValue> {
        crate::annotation_warnings_json(self.annotations.render_warnings())
    }
    /// Select the data under a point (or toggle it off) and resolve to the
    /// selection as JSON. Asynchronous: a radial chart picks on the GPU,
    /// like `FiggyChart.pick_point`.
    pub async fn pick(&mut self, x: f32, y: f32, toggle: bool) -> Result<String, JsValue> {
        self.chart.decorations_mut().selected = None;
        self.chart.select_at(&self.frame, [x, y], toggle).await?;
        serde_json::to_string(&self.chart.selected_json()).map_err(js_err)
    }
    /// Hover the data under a point (`None` leaves the chart) and advance
    /// the hover animation by `dt` seconds; resolves to whether the chart
    /// changed. Asynchronous: a radial chart picks on the GPU.
    pub async fn hover(
        &mut self,
        x: Option<f32>,
        y: Option<f32>,
        dt: f32,
    ) -> Result<bool, JsValue> {
        self.chart
            .hover(&self.frame, x.zip(y).map(|(x, y)| [x, y]), dt.min(0.1))
            .await
    }
    pub fn clear_selection(&mut self) {
        self.chart.clear_selection();
    }
    pub fn step_selection(&mut self, backwards: bool) {
        self.chart.step(backwards);
    }
}

/// The scale a surface of `config`'s size shows a chart of logical `size` at.
fn surface_scale(config: &wgpu::SurfaceConfiguration, size: (u32, u32)) -> f32 {
    (config.width as f32 / size.0.max(1) as f32)
        .min(config.height as f32 / size.1.max(1) as f32)
        .clamp(0.5, 4.)
}
