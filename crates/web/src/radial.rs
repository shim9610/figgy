//! Raw WASM host for the radial chart (pie, donut, detail ring), the
//! DataLess chart: the renderer owns all geometry.
//!
//! The JS class keeps the name `FiggyBoundedChart` and the `"radial"` family
//! argument hosts already pass. A categorical bar chart and a box plot are
//! Cartesian charts and a radar chart is a polar one: their documents open
//! through `FiggyChart::set_categorical_document`,
//! `FiggyChart::set_boxplot_document` and `FiggyChart::set_radar_document`,
//! and their family names are errors naming those methods.
use crate::web::convert::js_err;
use renderer::{
    AnnotationLayer, BoundedAnnotationFrame, DecorationId, LayerHit, LayerRef, RendererDevice,
    radial::*,
};
use serde_json::json;
use std::sync::Arc;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;
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
/// Whether data is drawn under `p`; a radial frame picks on the GPU.
async fn has_data(frame: &Arc<RadialFrame>, p: [f32; 2]) -> Result<bool, JsValue> {
    Ok(picked(frame.pick(p)).await?.is_some() || picked(frame.pick_at_rest(p)).await?.is_some())
}
/// The families [`FiggyRadialChart::create`] opens (#315).
const SUPPORTED_FAMILIES: &[&str] = &["radial"];
/// The radial document of `family`. The three former bounded families name
/// the `FiggyChart` method that opens their documents.
fn parse_chart(family: &str, value: &str) -> Result<RadialChart, JsValue> {
    match family {
        "radial" => {
            let mut chart: RadialChart = serde_json::from_str(value).map_err(js_err)?;
            // A slice without its drawn text gets the one its label format
            // writes, and a generated text (`text_generated`) is written
            // again from the document's values and format (#313); a text
            // the host wrote is drawn as it is.
            chart.regenerate_labels();
            Ok(chart)
        }
        "categorical" => Err(js_err(
            "categorical bar charts are Cartesian charts: open the document with FiggyChart.set_categorical_document",
        )),
        "boxplot" => Err(js_err(
            "box plots are Cartesian charts: open the document with FiggyChart.set_boxplot_document",
        )),
        "radar" => Err(js_err(
            "radar charts are polar charts: open the document with FiggyChart.set_radar_document",
        )),
        _ => Err(js_err(
            "unknown chart family: FiggyBoundedChart opens \"radial\" documents",
        )),
    }
}
/// Validate a radial document for a logical canvas of `size`.
fn validate(chart: &RadialChart, size: (u32, u32)) -> Result<(), JsValue> {
    if size.0 < 160 || size.1 < 160 {
        return Err(js_err(
            "radial chart canvas must be at least 160x160 pixels",
        ));
    }
    chart.validate().map_err(js_err)
}
/// What an export draws under the chart (#313): `"document"`,
/// `"transparent"` or a colour JSON `{"r":..,"g":..,"b":..,"a":..}` with
/// components `0..=1`. Anything else is an `Error` with `code`
/// `"export_background"`.
fn export_background(value: &str) -> Result<RadialExportBackground, JsValue> {
    match value {
        "document" => return Ok(RadialExportBackground::Document),
        "transparent" => return Ok(RadialExportBackground::Transparent),
        _ => {}
    }
    let refused = |reason: &str| {
        crate::web::convert::coded_error(
            &format!(
                "export background {value:?} is not \"document\", \"transparent\" or a \
                 colour JSON {{\"r\":..,\"g\":..,\"b\":..,\"a\":..}} with components 0 to 1: \
                 {reason}"
            ),
            "export_background",
            [],
        )
    };
    let color: renderer::Color =
        serde_json::from_str(value).map_err(|error| refused(&error.to_string()))?;
    if ![color.r, color.g, color.b, color.a]
        .iter()
        .all(|c| (0.0..=1.0).contains(c))
    {
        return Err(refused("a component is outside 0 to 1"));
    }
    Ok(RadialExportBackground::Color(color))
}
/// The browser host of one radial document, exported to JavaScript as
/// `FiggyBoundedChart`.
///
/// A chart made by [`Self::create`] requests its own adapter and device. A
/// chart made by [`Self::create_on`] draws on a `FiggyChart`'s device and
/// queue instead (#313): its own surface on its own canvas, the same
/// device-loss record, and the same GPU ledger and memory budget. Disposing
/// either chart (`free()`) drops only its own handles: the device lives while
/// the other holds it, and a dispose never destroys a device another chart
/// still draws on. Disposing the last chart on the device destroys it, since
/// figgy requested it, once the work submitted on it completes (#322).
#[wasm_bindgen(js_name = FiggyBoundedChart)]
pub struct FiggyRadialChart {
    surface: wgpu::Surface<'static>,
    _instance: wgpu::Instance,
    _adapter: wgpu::Adapter,
    gpu: RendererDevice,
    surface_config: wgpu::SurfaceConfiguration,
    chart: RadialChart,
    renderer: RadialRenderer,
    frame: Arc<RadialFrame>,
    annotations: Arc<BoundedAnnotationFrame>,
    size: (u32, u32),
}
#[wasm_bindgen(js_class = FiggyBoundedChart)]
impl FiggyRadialChart {
    /// The chart families `create` opens, as an array of strings (#315):
    /// `["radial"]`. The categorical, box plot and radar families moved to
    /// `FiggyChart` (`set_categorical_document`, `set_boxplot_document`,
    /// `set_radar_document`); passing one of them to `create` is an error
    /// naming that method.
    ///
    /// JS: `FiggyBoundedChart.supported_families() // ["radial"]`
    #[wasm_bindgen(unchecked_return_type = "string[]")]
    pub fn supported_families() -> js_sys::Array {
        crate::web::convert::js_names(SUPPORTED_FAMILIES)
    }
    /// One GPU surface; no kernel PNG transport is used for interactive frames.
    pub async fn create(
        canvas: HtmlCanvasElement,
        family: &str,
        value: &str,
        width: u32,
        height: u32,
    ) -> Result<FiggyRadialChart, JsValue> {
        console_error_panic_hook::set_once();
        let chart = parse_chart(family, value)?;
        validate(&chart, (width, height))?;
        let instance = renderer::gpu::create_instance();
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
            .map_err(js_err)?;
        let adapter = renderer::gpu::request_adapter_for_surface_async(&instance, &surface)
            .await
            .map_err(js_err)?;
        // figgy requests this device, so it is destroyed once the last
        // chart drawing on it is disposed (#322).
        let gpu = RendererDevice::request_async(&adapter)
            .await
            .map_err(js_err)?;
        Self::create_with(
            canvas,
            surface,
            instance,
            adapter,
            gpu,
            chart,
            (width, height),
        )
        .await
    }
    /// A radial chart on `parent`'s device and queue (#313), drawn to its
    /// own `canvas`: a promise of the `FiggyBoundedChart`, which validates
    /// the document as [`Self::create`] does but requests no adapter and no
    /// device.
    ///
    /// Lifetime: the chart holds clones of the parent's handles. Disposing
    /// it (`free()`) never destroys the parent's device, and disposing the
    /// parent leaves this chart drawing; the device lives while either
    /// holds it, and disposing the last of them destroys it (#322). Both share one device-loss record, so a lost device fails
    /// the next frame, export or pick of each with the loss, and one GPU
    /// ledger and memory budget: the parent's `gpu_memory_status` counts
    /// this chart's allocations, and the one budget bounds both charts.
    /// Both submit to the one queue in call order; a wait on
    /// either (`first_frame_ready`, an export) completes with the work
    /// submitted before it, the other chart's included. An async call
    /// (an export, a pick) borrows only this chart: while it is pending,
    /// `free()` on this chart throws as on every wasm-bindgen object
    /// (await it first), and disposing the parent does not cancel it.
    pub fn create_on(
        parent: &crate::web::FiggyChart,
        canvas: HtmlCanvasElement,
        family: String,
        value: String,
        width: u32,
        height: u32,
    ) -> js_sys::Promise {
        let created = Self::create_on_device_of(parent, canvas, family, value, width, height);
        wasm_bindgen_futures::future_to_promise(async move { created.await.map(JsValue::from) })
    }
    /// Configure `surface` for `gpu` and build the chart's renderer and first
    /// frame: the part [`Self::create`] and [`Self::create_on`] share.
    async fn create_with(
        canvas: HtmlCanvasElement,
        surface: wgpu::Surface<'static>,
        instance: wgpu::Instance,
        adapter: wgpu::Adapter,
        gpu: RendererDevice,
        chart: RadialChart,
        (width, height): (u32, u32),
    ) -> Result<FiggyRadialChart, JsValue> {
        let config = renderer::gpu::try_configure_surface(
            &surface,
            &adapter,
            gpu.device(),
            canvas.width(),
            canvas.height(),
        )
        .map_err(js_err)?;
        let mut renderer = RadialRenderer::try_new_async(gpu.clone(), config.format)
            .await
            .map_err(js_err)?;
        let scale = surface_scale(&config, (width, height));
        let (frame, annotations) = prepare(&mut renderer, &chart, (width, height), scale)?;
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
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn get_document(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.chart).map_err(js_err)
    }
    /// Validate and prepare before replacing the SSOT, retaining the old chart on failure.
    pub fn set_document(&mut self, value: &str, width: u32, height: u32) -> Result<(), JsValue> {
        let next = parse_chart("radial", value)?;
        validate(&next, (width, height))?;
        let scale = surface_scale(&self.surface_config, (width, height));
        let (frame, annotations) = prepare(&mut self.renderer, &next, (width, height), scale)?;
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
        let (frame, annotations) = prepare(&mut self.renderer, &self.chart, self.size, scale)?;
        self.frame = frame;
        self.annotations = annotations;
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
            // Fixed order: the frame paints its own background, then every
            // annotation and the annotation selection box above all
            // decorations.
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
        self.export_png_with_background(scale, "document").await
    }
    /// [`Self::export_png`] over another background (#313): `"document"`
    /// (the chart's own, as `export_png` draws it), `"transparent"` (no
    /// background: only the slices, labels and annotations) or a colour JSON
    /// `{"r":..,"g":..,"b":..,"a":..}` (components `0..=1`). The override is
    /// the export's alone: the document, its selection and the image on
    /// screen are unchanged. Any other value throws `code`
    /// `"export_background"` before anything is drawn.
    pub async fn export_png_with_background(
        &mut self,
        scale: f32,
        background: &str,
    ) -> Result<js_sys::Uint8Array, JsValue> {
        let background = export_background(background)?;
        let bytes = self
            .renderer
            .export_png_with_background(
                &self.chart,
                self.size,
                renderer::ExportOptions::document(scale),
                background,
            )
            .await
            .map_err(js_err)?;
        Ok(js_sys::Uint8Array::from(bytes.as_slice()))
    }
    /// The chart as an SVG document at `scale ×` resolution: the slices and
    /// background as one image, the title, labels, leader lines and
    /// annotations as vector content.
    pub async fn export_svg(&mut self, scale: f32) -> Result<String, JsValue> {
        self.renderer
            .export_svg(
                &self.chart,
                self.size,
                renderer::ExportOptions::document(scale),
            )
            .await
            .map_err(js_err)
    }

    /// [`Self::export_svg`] as one-page PDF bytes.
    pub async fn export_pdf(&mut self, scale: f32) -> Result<js_sys::Uint8Array, JsValue> {
        let bytes = self
            .renderer
            .export_pdf(
                &self.chart,
                self.size,
                renderer::ExportOptions::document(scale),
            )
            .await
            .map_err(js_err)?;
        Ok(js_sys::Uint8Array::from(bytes.as_slice()))
    }
    /// Renderer-measured logical pixel bounds and resize handles.
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    /// Rects are `[x, y, width, height]` in document logical pixels, not canvas
    /// pixels.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn decorations(&self) -> Result<String, JsValue> {
        let entry = |id: &DecorationId, b: renderer::SelectionBox| {
            let r = b.rect();
            json!({"id":id.element_id(),"rect":[r.x,r.y,r.width,r.height],"handles":b.handles().iter().map(|r|vec![r.x,r.y,r.width,r.height]).collect::<Vec<_>>()})
        };
        let (f, c) = (&self.frame, &self.chart);
        let values = f
            .decoration_ids(c)
            .filter_map(|(hit, id)| Some(entry(&id, f.decoration_selection_box(c, hit)?)))
            .collect::<Vec<_>>();
        serde_json::to_string(&values).map_err(js_err)
    }
    /// The topmost decoration under a point, as its stable id. While the
    /// annotation layer is active (and shown) its drawn items sit on top of
    /// every decoration and answer as `"annotation_<n>"`; on the base layer
    /// they are not registered and clicks pass through them.
    ///
    /// Asynchronous: over the plot area it asks whether data is drawn
    /// there, which the radial chart answers with its GPU pick.
    ///
    /// `x`, `y` are document logical pixels. Resolves to the plain id string,
    /// not JSON.
    pub async fn hit_decoration(
        &self,
        x: f32,
        y: f32,
        edit_area: bool,
    ) -> Result<Option<String>, JsValue> {
        let (f, c) = (&self.frame, &self.chart);
        let hit = self
            .annotations
            .route_active_layer(c.active_layer(), [x, y], || f.decoration_at(c, [x, y]));
        let id = match hit {
            Some(LayerHit::Annotation { index, .. }) => {
                return Ok(Some(DecorationId::Annotation(index).element_id()));
            }
            Some(LayerHit::Element(id)) => id,
            None => return Ok(None),
        };
        let Some(key) = f.decoration_id(c, id) else {
            return Ok(None);
        };
        if key == DecorationId::DataArea
            && !edit_area
            && c.decorations.selected != Some(key.clone())
            && has_data(f, [x, y]).await?
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
                    .or_else(|| {
                        self.frame
                            .decoration_ids(&self.chart)
                            .find(|(_, id)| id.element_id() == name)
                            .map(|(_, id)| id)
                    })
                    .ok_or_else(|| js_err("unknown decoration"))?,
            ),
            None => None,
        };
        self.clear_selection();
        self.chart.decorations.selected = selected;
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
    /// Put the decoration `id` (an id [`Self::decorations`] lists, such as
    /// `"chart_title"` or `"radial_label_0_1"`) back where the layout places
    /// it (#313): its stored offset is removed, and for `"data_area"` the
    /// explicit plot region too (`DecorationLayout::reset`). Not a drag by
    /// the negative offset: nothing is left stored. Returns whether the
    /// document changed. An unknown id, and an annotation (which has a
    /// position of its own, not an offset), throw.
    pub fn reset_decoration_position(&mut self, id: &str) -> Result<bool, JsValue> {
        if self.drawn_annotation(id).is_some() {
            return Err(js_err(
                "an annotation has no decoration offset: move it with drag_annotation",
            ));
        }
        let key = self
            .frame
            .decoration_ids(&self.chart)
            .find(|(_, key)| key.element_id() == id)
            .map(|(_, key)| key)
            .ok_or_else(|| js_err("unknown decoration"))?;
        let mut next = self.chart.clone();
        next.decorations.reset(&key);
        if next == self.chart {
            return Ok(false);
        }
        self.adopt(next)?;
        Ok(true)
    }
    /// Rewrite the automatic slice labels from the document's values and
    /// label format (#313): each generated text (`text_generated`) and each
    /// missing one, as `RadialChart::regenerate_labels` writes them; a text
    /// the host wrote stays. With `rewrite_user_text`, every text is
    /// rewritten, the host's included (`RadialChart::format_labels`).
    /// `set_document` already regenerates the generated texts of the
    /// document it is given; this is the call for an explicit rewrite.
    pub fn regenerate_labels(&mut self, rewrite_user_text: bool) -> Result<(), JsValue> {
        let mut next = self.chart.clone();
        if rewrite_user_text {
            next.format_labels();
        } else {
            next.regenerate_labels();
        }
        self.adopt(next)
    }
    /// `x`, `y` are document logical pixels.
    pub fn resize_handle(&self, target: &str, x: f32, y: f32) -> Option<String> {
        if let Some(index) = self.drawn_annotation(target) {
            return self.annotation_handle(index as u32, x, y);
        }
        let id = self.frame.decorations().find(target)?;
        self.frame
            .decoration_resize_handle(&self.chart, id, [x, y])
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
            .decorations()
            .find(target)
            .ok_or_else(|| js_err("unknown decoration"))?;
        let handle = handle
            .map(|name| {
                [N, S, W, E, NW, NE, SW, SE]
                    .into_iter()
                    .find(|h| format!("{h:?}") == name)
                    .ok_or_else(|| js_err("unknown resize handle"))
            })
            .transpose()?;
        // The radial decorations edit the chart they are placed from.
        let mut next = self.chart.clone();
        let result = match handle {
            Some(h) => self.frame.resize_decoration(&mut next, id, h, dx, dy),
            None => self.frame.drag_decoration(&mut next, id, dx, dy),
        };
        if result != Some(renderer::layout::NudgeResult::Moved) {
            return Ok(false);
        }
        let scale = self.surface_scale();
        let (frame, annotations) = prepare(&mut self.renderer, &next, self.size, scale)?;
        self.chart = next;
        self.frame = frame;
        self.annotations = annotations;
        Ok(true)
    }
    /// Apply the radial member of the theme family `name` (#291, #295:
    /// `gallery`, `cool-slate`, `midnight` or `warm-paper`, as
    /// [`Self::theme_names`] lists them): the background, label look, material, view,
    /// slice shape and outline, the palette on the top-level slices and the
    /// theme's tint on the slices below them. Values, labels, the slice
    /// structure and the kind stay. A name that is not a family changes
    /// nothing and throws `code` `"theme_unknown_name"` with the family
    /// `names`.
    pub fn apply_theme(&mut self, name: &str) -> Result<(), JsValue> {
        let mut next = self.chart.clone();
        next.apply_theme(name)
            .map_err(crate::web::convert::theme_error)?;
        self.adopt(next)
    }
    /// [`Self::apply_theme`] with the palette preset `cycle` on the
    /// top-level slices instead of the family's colours (#357). Refused,
    /// changing nothing, as [`Self::apply_theme`] refuses the name.
    pub fn apply_theme_with_palette(
        &mut self,
        name: &str,
        cycle: crate::web::convert::ColorCycle,
    ) -> Result<(), JsValue> {
        let family =
            renderer::theme::ThemeFamily::parse(name).map_err(crate::web::convert::theme_error)?;
        let mut next = self.chart.clone();
        family
            .radial()
            .with_palette(renderer::ColorCycle::from(cycle))
            .apply(&mut next);
        self.adopt(next)
    }
    /// The theme names (#291, #295), as an array of strings: the four
    /// families.
    ///
    /// Returns an array of strings, not a JSON string.
    #[wasm_bindgen(unchecked_return_type = "string[]")]
    pub fn theme_names(&self) -> js_sys::Array {
        crate::web::convert::js_names(renderer::theme::ThemeFamily::NAMES)
    }
    /// Validate and prepare an edited document, then adopt it; the previous
    /// chart and frames stay on failure.
    fn adopt(&mut self, next: RadialChart) -> Result<(), JsValue> {
        validate(&next, self.size)?;
        let scale = self.surface_scale();
        let (frame, annotations) = prepare(&mut self.renderer, &next, self.size, scale)?;
        self.chart = next;
        self.frame = frame;
        self.annotations = annotations;
        Ok(())
    }
    /// Set the layer that receives pointer interaction: `"Base"` or
    /// `"Annotations"` as JSON (the radial chart has no overlays, so an
    /// `{ "Overlay": n }` is stored as `"Base"`). A change clears the
    /// decoration selection (`decorations.selected`, an annotation
    /// included); the data selection is kept. Returns the stored layer as
    /// JSON. Drawn pixels never depend on it.
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
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
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
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
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    /// Rects are in document logical pixels, not canvas pixels.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn annotations(&self) -> Result<String, JsValue> {
        let map = self.annotations.annotations();
        let items = &self.chart.annotations.items;
        let selected = AnnotationLayer::selected_index(self.chart.decorations.selected.as_ref());
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
    ///
    /// `x`, `y` are document logical pixels.
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
            Some(i) if (i as usize) < self.chart.annotations.items.len() => {
                Some(DecorationId::Annotation(i as usize))
            }
            Some(_) => return Err(js_err("unknown annotation")),
            None => None,
        };
        let mut next = self.chart.clone();
        next.decorations.selected = selected;
        next.interaction.selected = None;
        self.adopt(next)
    }
    /// The index of the selected annotation, or `undefined`. An index is
    /// valid until the annotation list changes (see `annotation_index`).
    pub fn selected_annotation(&self) -> Option<u32> {
        AnnotationLayer::selected_index(self.chart.decorations.selected.as_ref()).map(|i| i as u32)
    }
    /// The `id` of every annotation, in index order, as an array of strings
    /// (not a JSON string): `""` for an item without one (#315). Like
    /// `FiggyChart.annotation_ids`.
    #[wasm_bindgen(unchecked_return_type = "string[]")]
    pub fn annotation_ids(&self) -> js_sys::Array {
        let array = js_sys::Array::new();
        for item in &self.chart.annotations.items {
            array.push(&JsValue::from_str(&item.id));
        }
        array
    }
    /// The current index of the annotation whose `id` is `id`, or
    /// `undefined` when none has it (#315). An empty id, or one several
    /// items share, is an error. Like `FiggyChart.annotation_index`.
    pub fn annotation_index(&self, id: &str) -> Result<Option<u32>, JsValue> {
        crate::annotation_index_of(&self.chart.annotations.items, id)
    }
    /// Select the annotation whose `id` is `id`, like `select_annotation`
    /// with its index (#315); an error, with the selection unchanged, when
    /// no annotation or more than one has it, or `id` is empty.
    pub fn select_annotation_by_id(&mut self, id: &str) -> Result<(), JsValue> {
        let index = crate::annotation_index_of(&self.chart.annotations.items, id)?
            .ok_or_else(|| js_err(format!("no annotation has the id {id:?}")))?;
        self.select_annotation(Some(index))
    }
    /// The compass name of the annotation's live handle under a point.
    ///
    /// `x`, `y` are document logical pixels.
    pub fn annotation_handle(&self, index: u32, x: f32, y: f32) -> Option<String> {
        // An annotation not drawn in the current frame has no handle.
        let id = self.annotations.annotations().find(index as usize)?;
        self.annotations
            .annotations()
            .hit_resize_handle(id, [x, y])
            .map(|h| format!("{h:?}"))
    }
    /// The vertex handle (`"v<i>"`) of a polygon annotation under a point.
    ///
    /// `x`, `y` are document logical pixels.
    pub fn annotation_vertex(&self, index: u32, x: f32, y: f32) -> Option<String> {
        // An annotation not drawn in the current frame has no handle.
        let id = self.annotations.annotations().find(index as usize)?;
        self.annotations
            .annotations()
            .hit_vertex(id, [x, y])
            .map(|i| format!("v{i}"))
    }
    /// Move an annotation, or one of its handles (a compass name or a
    /// polygon vertex `"v<i>"`), by a pointer delta. Returns `false` for a
    /// rejected move; the document is unchanged then.
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
        let result =
            if let Some(vertex) = handle.as_deref().and_then(crate::web::vertex_handle_index) {
                map.drag_vertex(&mut next.annotations, id, vertex, dx, dy)
            } else if let Some(name) = handle {
                let h = [N, S, W, E, NW, NE, SW, SE]
                    .into_iter()
                    .find(|h| format!("{h:?}") == name)
                    .ok_or_else(|| js_err("unknown resize handle"))?;
                map.resize_by(&mut next.annotations, id, h, dx, dy)
            } else {
                map.drag_by(&mut next.annotations, id, dx, dy)
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
        next.annotations.items.push(item);
        let index = next.annotations.items.len() - 1;
        self.adopt(next)?;
        Ok(index as u32)
    }
    /// Append a text note at the plot's `corner` (a `NoteAlign` name:
    /// `"TopLeft"`, `"Top"`, ..., `"BottomRight"`), its block inside the
    /// plot, and return its index (#258). `text` is a `RichText` JSON value;
    /// the note owns it. It is an ordinary text annotation.
    pub fn add_corner_note(&mut self, text: &str, corner: &str) -> Result<u32, JsValue> {
        let text: renderer::text::RichText = serde_json::from_str(text).map_err(js_err)?;
        let corner: renderer::NoteAlign =
            serde_json::from_value(serde_json::Value::String(corner.into()))
                .map_err(|_| js_err("unknown note corner"))?;
        let mut next = self.chart.clone();
        let index = next.annotations.add_corner_note(text, corner);
        self.adopt(next)?;
        Ok(index as u32)
    }
    /// Replace the annotation at `index`.
    pub fn set_annotation(&mut self, index: u32, json: &str) -> Result<(), JsValue> {
        let item: renderer::Annotation = serde_json::from_str(json).map_err(js_err)?;
        let mut next = self.chart.clone();
        let slot = next
            .annotations
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
        if index >= next.annotations.items.len() {
            return Err(js_err("unknown annotation"));
        }
        next.annotations.items.remove(index);
        next.decorations.selected = match next.decorations.selected.take() {
            Some(DecorationId::Annotation(i)) if i == index => None,
            Some(DecorationId::Annotation(i)) if i > index => Some(DecorationId::Annotation(i - 1)),
            other => other,
        };
        self.adopt(next)
    }
    /// Show or hide the whole annotation layer without re-rasterizing it.
    pub fn set_annotations_visible(&mut self, visible: bool) -> Result<(), JsValue> {
        let mut next = self.chart.clone();
        next.annotations.visible = visible;
        self.adopt(next)
    }
    /// The `Anchor` JSON at a point for `"Data"` or `"Ratio"`; `null` when
    /// the point has no such position (a radial chart has no data anchors).
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    /// `x`, `y` are document logical pixels.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
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
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn annotation_warnings(&self) -> Result<String, JsValue> {
        crate::annotation_warnings_json(self.annotations.render_warnings())
    }
    /// Where an annotation that is not part of the document yet would be
    /// drawn (#334), with the contract of `FiggyChart.annotation_preview`:
    /// placed by the annotation layer's own placement code in the current
    /// frame's anchor space, its text measured at the scale the canvas
    /// shows the chart at. The document, the selection and the raster stay
    /// as they are. `json` is an `Annotation`, the shape `add_annotation`
    /// takes, checked like `FiggyChart.annotation_preview` checks it
    /// (codes `annotation_json`, `annotation_invalid`).
    ///
    /// Returns JSON `{ kind, visible, drawn, canvas, document }`. `canvas`
    /// is the shape in physical canvas pixels and `document` in document
    /// logical pixels (the units of `annotations` and `hit_annotation`),
    /// each `{ shape, points | rect, bounds }`: `shape` is `"text"` (`rect`
    /// `[x, y, w, h]`), `"arrow"` (`points`, its two ends) or `"polygon"`
    /// (a shape's outline), and `bounds` is everything the raster paints for
    /// it, labels included. The radial chart has no value axes, so a
    /// reference line, a span and a data anchor are never drawn. An item the
    /// raster would skip has `drawn: false` and the `reason`
    /// `annotation_warnings` would give (`NonFinite`, `NoValueAxis`,
    /// `OutOfRange`, `EmptyText`). `visible: false` (a hidden item) is
    /// placed as it is drawn once shown.
    /// JS: `const p = JSON.parse(chart.annotation_preview(itemJson));`
    pub fn annotation_preview(&self, json: &str) -> Result<String, JsValue> {
        let item = crate::web::FiggyChart::parse_annotation(json)?;
        let placed = crate::radial_annotation_preview(
            &item,
            *self.annotations.annotations().space(),
            self.surface_scale(),
        );
        serde_json::to_string(&crate::annotation_preview_value(&item, placed)).map_err(js_err)
    }
    /// Select the data under a point (or toggle it off) and resolve to the
    /// selection as JSON. Asynchronous: the radial chart picks on the GPU,
    /// like `FiggyChart.pick_point`.
    ///
    /// Resolves to JSON text (`FiggyJson`), not an object. `x`, `y` are
    /// document logical pixels.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub async fn pick(&mut self, x: f32, y: f32, toggle: bool) -> Result<String, JsValue> {
        self.chart.decorations.selected = None;
        let hit = radial_at(&self.frame, [x, y], self.chart.interaction.hovered).await?;
        let interaction = &mut self.chart.interaction;
        interaction.selected = if toggle && hit == interaction.selected {
            None
        } else {
            hit
        };
        serde_json::to_string(&json!(self.chart.interaction.selected)).map_err(js_err)
    }
    /// Hover the data under a point (`None` leaves the chart) and advance
    /// the hover animation by `dt` seconds; resolves to whether the chart
    /// changed. Asynchronous: the radial chart picks on the GPU.
    pub async fn hover(
        &mut self,
        x: Option<f32>,
        y: Option<f32>,
        dt: f32,
    ) -> Result<bool, JsValue> {
        let hit = match x.zip(y) {
            Some((x, y)) => radial_at(&self.frame, [x, y], self.chart.interaction.hovered).await?,
            None => None,
        };
        Ok(self.chart.interaction.animate_hover(hit, dt.min(0.1), 0.16))
    }
    pub fn clear_selection(&mut self) {
        self.chart.decorations.selected = None;
        self.chart.interaction.selected = None;
    }
    pub fn step_selection(&mut self, backwards: bool) {
        self.chart.step_selection(backwards);
    }
}

impl FiggyRadialChart {
    /// [`Self::create_on`] for Rust callers: the chart itself rather than a
    /// promise. The parent is read before the returned future starts, so
    /// the future borrows nothing.
    pub fn create_on_device_of(
        parent: &crate::web::FiggyChart,
        canvas: HtmlCanvasElement,
        family: String,
        value: String,
        width: u32,
        height: u32,
    ) -> impl std::future::Future<Output = Result<FiggyRadialChart, JsValue>> + 'static {
        console_error_panic_hook::set_once();
        let (gpu, instance, adapter) = parent.shared_gpu();
        async move {
            let chart = parse_chart(&family, &value)?;
            validate(&chart, (width, height))?;
            let surface = instance
                .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
                .map_err(js_err)?;
            Self::create_with(
                canvas,
                surface,
                instance,
                adapter,
                gpu,
                chart,
                (width, height),
            )
            .await
        }
    }
}

/// The radial frame and its annotation layer for one document state.
fn prepare(
    renderer: &mut RadialRenderer,
    chart: &RadialChart,
    size: (u32, u32),
    scale: f32,
) -> Result<(Arc<RadialFrame>, Arc<BoundedAnnotationFrame>), JsValue> {
    let frame = renderer.prepare(chart, size, scale).map_err(js_err)?;
    let annotations = renderer
        .prepare_annotations(chart, size, scale)
        .map_err(js_err)?;
    Ok((frame, annotations))
}

/// The scale a surface of `config`'s size shows a chart of logical `size` at.
fn surface_scale(config: &wgpu::SurfaceConfiguration, size: (u32, u32)) -> f32 {
    (config.width as f32 / size.0.max(1) as f32)
        .min(config.height as f32 / size.1.max(1) as f32)
        .clamp(0.5, 4.)
}
