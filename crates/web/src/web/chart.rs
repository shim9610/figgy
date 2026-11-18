//! `FiggyChart`: construction, lifecycle, the document (config and
//! series), and the frame loop.

use super::*;

pub(super) const POOL_CAPACITY: u64 = 16 * 1024 * 1024;

/// Report one init stage to the host's `on_event` as `{ scope, stage,
/// phase }`. An exception the callback throws (or a failed property set on
/// the payload) is returned, so the call that reports progress rejects
/// with it.
pub(super) fn emit_init_progress(
    on_event: Option<&js_sys::Function>,
    event: InitEvent,
) -> Result<(), JsValue> {
    let Some(callback) = on_event else {
        return Ok(());
    };
    let payload = js_sys::Object::new();
    let phase = match event.phase {
        InitPhase::Started => "started",
        InitPhase::Finished => "finished",
    };
    for (key, value) in [
        ("scope", event.scope),
        ("stage", event.stage),
        ("phase", phase),
    ] {
        if !js_sys::Reflect::set(&payload, &key.into(), &JsValue::from_str(value))? {
            return Err(js_err(format!(
                "init progress: could not set `{key}` on the event object"
            )));
        }
    }
    callback.call1(&JsValue::UNDEFINED, &payload)?;
    Ok(())
}

/// The JS name of a pipeline failure's kind (#299).
fn failure_kind(kind: renderer::PipelineFailureKind) -> &'static str {
    match kind {
        renderer::PipelineFailureKind::Validation => "validation",
        renderer::PipelineFailureKind::Internal => "internal",
        renderer::PipelineFailureKind::Rejected => "rejected",
        renderer::PipelineFailureKind::DeviceLost => "device_lost",
        renderer::PipelineFailureKind::Cancelled => "cancelled",
    }
}

/// TypeScript names the `unchecked_return_type` of the wrappers' methods
/// use (#315), emitted into the generated `.d.ts`.
#[wasm_bindgen(typescript_custom_section)]
const FIGGY_RETURN_TYPES: &'static str = r#"
/**
 * JSON text returned by a figgy method: a `string`, not an object. Parse it
 * with `JSON.parse`; the method's documentation names its shape. Lengths
 * in it follow that method's units: `FiggyChart` geometry is in physical
 * canvas pixels (the canvas's `width`/`height` space) unless the method says
 * document logical pixels; `FiggyBoundedChart` geometry is in document
 * logical pixels.
 */
export type FiggyJson = string;

/** Background pipeline compiles of one scope (#299). */
export interface FiggyPipelineCounts {
    total: number;
    ready: number;
    /** Waiting or compiling. */
    compiling: number;
    failed: number;
}

/** `FiggyChart.pipeline_status()`: a plain object, not JSON. */
export interface FiggyPipelineStatus {
    /** What the last frame needed. */
    screen: FiggyPipelineCounts;
    /** Every compile scheduled so far. */
    all: FiggyPipelineCounts;
    /** Whether the last frame drew every series. */
    frame_complete: boolean;
}

/** `FiggyReadiness.status()`: a plain object, not JSON (#315). */
export interface FiggyReadinessStatus {
    screen: FiggyPipelineCounts;
    all: FiggyPipelineCounts;
    /** The chart was freed; the counts are zero then. */
    disposed: boolean;
}

/** One failed background compile, from `FiggyChart.pipeline_failures()`. */
export interface FiggyPipelineFailure {
    label: string;
    kind: "validation" | "internal" | "rejected" | "device_lost" | "cancelled";
    message: string;
}

/**
 * A theme as a plain object (not JSON): the model `Theme`'s fields, colours
 * as `{ r, g, b, a }`. Edit it and pass it to `apply_custom_theme`.
 */
export type FiggyThemeObject = Record<string, unknown>;
"#;

/// `{ total, ready, compiling, failed }` for one scope of background
/// compiles (#299).
fn counts_object(counts: renderer::PipelineCounts) -> Result<JsValue, JsValue> {
    let object = js_sys::Object::new();
    js_sys::Reflect::set(&object, &"total".into(), &(counts.total as f64).into())?;
    js_sys::Reflect::set(&object, &"ready".into(), &(counts.ready as f64).into())?;
    js_sys::Reflect::set(
        &object,
        &"compiling".into(),
        &(counts.compiling as f64).into(),
    )?;
    js_sys::Reflect::set(&object, &"failed".into(), &(counts.failed as f64).into())?;
    Ok(object.into())
}

/// The message a readiness wait rejects with once its chart was freed
/// (#315).
const CHART_DISPOSED: &str = concat!(
    "chart disposed: the chart was freed ",
    "(or stopped compiling in the background) before the wait ended"
);

/// A readiness wait's end as the promise's value: `undefined`, or an
/// `Error` (#315).
fn readiness_result(result: Result<(), renderer::ReadinessError>) -> Result<JsValue, JsValue> {
    match result {
        Ok(()) => Ok(JsValue::UNDEFINED),
        Err(renderer::ReadinessError::Failed(failure)) => Err(failure_error(&failure)),
        Err(renderer::ReadinessError::RendererDropped) => {
            Err(js_sys::Error::new(CHART_DISPOSED).into())
        }
    }
}

/// A handle on a chart's background pipeline compiles that does not borrow
/// the chart (#315), from [`FiggyChart::readiness`]. While one of its waits
/// is pending the host keeps full use of the chart: `pipeline_status()`,
/// `frame()`, `resize()` and `free()` all work. It holds the chart's compile
/// queue weakly and keeps nothing alive. Once the chart is freed a pending
/// or later wait rejects with an `Error` whose message starts with
/// `"chart disposed"`, and `status().disposed` is `true`. The handle draws
/// nothing: after `series_ready()` resolves, the host's next `frame()`
/// draws the series (or use `chart.series_ready()`, which draws, but
/// borrows the chart until it settles). Free the handle with `free()` when
/// done; freeing it ends nothing.
///
/// A chart that compiles no pipeline in the background (one made without
/// the browser's async compile queue) gives a handle with nothing to wait
/// for: its waits resolve at once and `disposed` stays `false`.
///
/// JS:
/// ```js
/// const readiness = chart.readiness();
/// const ready = readiness.series_ready(); // the chart stays usable
/// overlay.textContent = `${readiness.status().screen.ready} /
/// ${readiness.status().screen.total}`;
/// chart.resize(w, h);
/// await ready;
/// chart.frame();
/// ```
#[wasm_bindgen]
pub struct FiggyReadiness {
    readiness: renderer::PipelineReadiness,
}

#[wasm_bindgen]
impl FiggyReadiness {
    /// Resolves once every background compile the chart's last frame
    /// needed has ended, like `chart.series_ready()` but without drawing
    /// and without borrowing the chart. Rejects with an `Error` carrying
    /// `label`, `kind` and `message` when one of them failed, or with a
    /// `"chart disposed"` `Error` once the chart is freed. The promise owns
    /// a clone of the handle; it does not borrow this object.
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn series_ready(&self) -> js_sys::Promise {
        let readiness = self.readiness.clone();
        wasm_bindgen_futures::future_to_promise(async move {
            readiness_result(readiness.series_ready().await)
        })
    }

    /// Resolves once every background compile scheduled so far has ended,
    /// like `chart.pipelines_ready()` but without borrowing the chart.
    /// Rejects like [`Self::series_ready`].
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn pipelines_ready(&self) -> js_sys::Promise {
        let readiness = self.readiness.clone();
        wasm_bindgen_futures::future_to_promise(async move {
            readiness_result(readiness.pipelines_ready().await)
        })
    }

    /// The chart's background compiles now, as a plain object (not a JSON
    /// string): `{ screen: { total, ready, compiling, failed }, all: { … },
    /// disposed }`. `screen` and `all` are as in `chart.pipeline_status()`;
    /// there is no `frame_complete` (only a frame decides it, so read it on
    /// the chart). `disposed` is `true` once the chart is freed; the counts
    /// are zero then. Readable at any time, also while a wait is pending.
    #[wasm_bindgen(unchecked_return_type = "FiggyReadinessStatus")]
    pub fn status(&self) -> Result<JsValue, JsValue> {
        let status = self.readiness.status();
        let object = js_sys::Object::new();
        js_sys::Reflect::set(&object, &"screen".into(), &counts_object(status.screen)?)?;
        js_sys::Reflect::set(&object, &"all".into(), &counts_object(status.all)?)?;
        js_sys::Reflect::set(&object, &"disposed".into(), &status.renderer_dropped.into())?;
        Ok(object.into())
    }
}

/// `{ label, kind, message }` for one failed compile (#299).
fn failure_object(failure: &renderer::PipelineFailure) -> Result<JsValue, JsValue> {
    let object = js_sys::Object::new();
    js_sys::Reflect::set(&object, &"label".into(), &failure.label.as_str().into())?;
    js_sys::Reflect::set(&object, &"kind".into(), &failure_kind(failure.kind).into())?;
    js_sys::Reflect::set(&object, &"message".into(), &failure.message.as_str().into())?;
    Ok(object.into())
}

/// An `Error` for a failed compile, with its `label`, `kind` and `message`
/// as properties (#299).
fn failure_error(failure: &renderer::PipelineFailure) -> JsValue {
    let error = js_sys::Error::new(&format!(
        "the pipeline {} did not compile ({}): {}",
        failure.label,
        failure_kind(failure.kind),
        failure.message
    ));
    for (key, value) in [
        ("label", failure.label.as_str()),
        ("kind", failure_kind(failure.kind)),
    ] {
        if js_sys::Reflect::set(&error, &key.into(), &value.into()).is_err() {
            // Setting a property on a fresh `Error` does not throw; the
            // message carries both anyway.
        }
    }
    error.into()
}

/// [`emit_init_progress`] for a renderer observer, which cannot return an
/// error: the first failure is kept in `failed` and later events are not
/// reported. The caller rejects with `failed` once the observed call
/// returns.
fn observe_init_progress(
    on_event: Option<&js_sys::Function>,
    event: InitEvent,
    failed: &mut Option<JsValue>,
) {
    if failed.is_none()
        && let Err(error) = emit_init_progress(on_event, event)
    {
        *failed = Some(error);
    }
}

/// The shader set named `name` in the web API (#299): `"basic"` or
/// `"full"`. Any other value is refused, naming it.
pub(super) fn shader_set_from_name(name: &JsValue) -> Result<renderer::ShaderSet, JsValue> {
    match name.as_string().as_deref() {
        Some("basic") => Ok(renderer::ShaderSet::Basic),
        Some("full") => Ok(renderer::ShaderSet::Full),
        Some(other) => Err(JsValue::from(js_sys::Error::new(&format!(
            "shaders must be \"basic\" or \"full\", got \"{other}\""
        )))),
        None => Err(JsValue::from(js_sys::Error::new(
            "shaders must be the string \"basic\" or \"full\"",
        ))),
    }
}

/// The web API's name of `set`.
pub(super) fn shader_set_name(set: renderer::ShaderSet) -> &'static str {
    match set {
        renderer::ShaderSet::Basic => "basic",
        renderer::ShaderSet::Full => "full",
    }
}

/// `FiggyChart.create_with_options`' options, read and checked before
/// anything is created: `shaders` and `on_progress`, both optional. An
/// unknown key or a value of the wrong kind is refused.
pub(super) fn create_options(
    options: &JsValue,
) -> Result<(renderer::ShaderSet, Option<js_sys::Function>), JsValue> {
    if options.is_undefined() || options.is_null() {
        return Ok((renderer::ShaderSet::Basic, None));
    }
    if !options.is_object() || js_sys::Array::is_array(options) {
        return Err(JsValue::from(js_sys::Error::new(
            "create_with_options takes an options object",
        )));
    }
    let mut set = renderer::ShaderSet::Basic;
    let mut on_progress = None;
    for key in js_sys::Object::keys(options.unchecked_ref::<js_sys::Object>()).iter() {
        let value = js_sys::Reflect::get(options, &key)?;
        match key.as_string().as_deref() {
            Some("shaders") => {
                if !value.is_undefined() {
                    set = shader_set_from_name(&value)?;
                }
            }
            Some("on_progress") => {
                if !value.is_undefined() {
                    on_progress = Some(value.dyn_into::<js_sys::Function>().map_err(|_| {
                        JsValue::from(js_sys::Error::new("on_progress must be a function"))
                    })?);
                }
            }
            other => {
                return Err(JsValue::from(js_sys::Error::new(&format!(
                    "unknown create option \"{}\": the options are shaders and on_progress",
                    other.unwrap_or("")
                ))));
            }
        }
    }
    Ok((set, on_progress))
}

pub(super) async fn create_chart_kernel(
    canvas: HtmlCanvasElement,
    on_event: Option<js_sys::Function>,
    shader_set: renderer::ShaderSet,
) -> Result<FiggyChart, JsValue> {
    console_error_panic_hook::set_once();

    let (mut renderer, w, h) = {
        let on_event_ref = on_event.as_ref();
        let mut failed = None;
        let mut observer = |event| observe_init_progress(on_event_ref, event, &mut failed);
        let (w, h) = (canvas.width().max(1), canvas.height().max(1));
        // No pipeline compile is awaited (#299): the renderer schedules its
        // shader set's render pipelines on its compile queue, which the
        // browser works through in the background, and the first frame
        // draws with the one pipeline an empty chart needs.
        let renderer = Renderer::for_window_with_shader_set_async_observed(
            wgpu::SurfaceTarget::Canvas(canvas),
            (w, h),
            POOL_CAPACITY,
            shader_set,
            &mut observer,
        )
        .await
        .map_err(js_err);
        if let Some(error) = failed {
            return Err(error);
        }
        let mut renderer = renderer?;
        renderer.set_defrag_policy(DefragPolicy::OnAllocFailure);
        (renderer, w, h)
    };

    emit_init_progress(
        on_event.as_ref(),
        InitEvent::new("web.create", "chart resources", InitPhase::Started),
    )?;

    let mut config = renderer::default::default_config();
    config.chart_area = ChartArea(Rect {
        x: 0,
        y: 0,
        width: w,
        height: h,
    });
    let chart_id = renderer
        .register_chart(config.clone(), Vec::new())
        .map_err(js_err)?;
    // The canvas shows the document scaled to its surface (#160).
    renderer
        .set_chart_display(chart_id, Some((w, h)))
        .map_err(js_err)?;
    let chart = Chart::new(config);
    let view = renderer
        .create_chart_view(
            &chart,
            Rect {
                x: 0,
                y: 0,
                width: w,
                height: h,
            },
        )
        .map_err(js_err)?;

    emit_init_progress(
        on_event.as_ref(),
        InitEvent::new("web.create", "chart resources", InitPhase::Finished),
    )?;
    let mut chart = FiggyChart {
        renderer,
        chart_id,
        view,
        surface_size: (w, h),
        legend_auto_managed: true,
        color_seq: 0,
        hitmap: HitMap::standard_chart(),
        hitmap_layer: (renderer::LayerRef::Base, 0, None, 0, None),
        hitmap_categories: Vec::new(),
        dragging: false,
        drag_pointer: [0.0; 2],
        resizing: None,
        cycle: renderer::ColorCycle::Classic,
        clear_color: Color::WHITE,
        view_dirty: false,
        redraw_pending: true,
        last_presented_stamp: None,
        selected_annotation: None,
        annotation_selection_dirty: false,
        annotation_pointer: false,
        value_label_pointer: false,
        display_held: false,
        feynman_pointer: None,
        plot_resize: None,
        plot_resize_request: None,
        monotonic: None,
    };

    emit_init_progress(
        on_event.as_ref(),
        InitEvent::new("web.create", "first frame", InitPhase::Started),
    )?;
    chart.first_frame_ready().await?;
    emit_init_progress(
        on_event.as_ref(),
        InitEvent::new("web.create", "first frame", InitPhase::Finished),
    )?;
    Ok(chart)
}

/// Parameter metadata for one `draw_style` mode — a JSON array of
/// `{key, min, max, default, integer}`. Ranges are the RECOMMENDED
/// slider spans (the SSoT accepts values beyond them; the renderer
/// applies only safety guards), and they come from the model crate, so
/// hosts never hardcode them.
///
/// JS: `const specs = JSON.parse(draw_style_param_specs("constellation"));`
/// Valid modes: `draw_style_modes()`.
///
/// Returns JSON text (`FiggyJson`), not an object: parse it with `JSON.parse`.
#[wasm_bindgen(unchecked_return_type = "FiggyJson")]
pub fn draw_style_param_specs(mode: &str) -> Result<String, JsValue> {
    let specs = renderer::config::DrawStyle::param_specs_for_mode(mode).ok_or_else(|| {
        js_err(format!(
            "unknown draw_style mode {mode:?} (valid: {})",
            renderer::config::DrawStyle::mode_tags().join(", ")
        ))
    })?;
    serde_json::to_string(specs).map_err(js_err)
}

/// A rich text from TeX (`RichText::from_tex`, #45): text with `$…$` or
/// `\(…\)` math spans, as the JSON of a `RichText` for any title, axis
/// title, legend, label or annotation text of `set_config` / `set_series`.
/// Errors name the TeX byte range.
///
/// JS: `cfg.chart_title.text = JSON.parse(rich_text_from_tex("$E = mc^2$", 16, "", 0, 0, 0, 1));`
///
/// Returns JSON text (`FiggyJson`), not an object: parse it with `JSON.parse`.
#[wasm_bindgen(unchecked_return_type = "FiggyJson")]
pub fn rich_text_from_tex(
    tex: &str,
    font_size: f32,
    font: &str,
    r: f32,
    g: f32,
    b: f32,
    a: f32,
) -> Result<String, JsValue> {
    crate::rich_text_json_from_tex(tex, font_size, font, [r, g, b, a]).map_err(js_err)
}

/// `layout::auto_layout` (#44) on a config JSON: the config with axis
/// margins, axis title offsets and tick-label formats chosen so its tick
/// labels, titles, legend and colourbar do not overlap, as the renderer
/// draws them. Pure: it reads only the JSON and the text measure and
/// changes no chart; apply the result with `set_config`. Call it again after
/// an auto-fit or a zoom, which reset a linear axis to decimal labels.
/// Errors are JSON errors.
///
/// JS: `chart.set_config(autoLayout(chart.get_config()));`
///
/// Returns JSON text (`FiggyJson`), not an object: parse it with `JSON.parse`.
/// Lengths in the config are document logical pixels.
#[wasm_bindgen(js_name = autoLayout, unchecked_return_type = "FiggyJson")]
pub fn auto_layout(config_json: &str) -> Result<String, JsValue> {
    crate::auto_layout_json(config_json).map_err(js_err)
}

// ------------------------------------------------------------------
// The model API for editors (#314): pure functions of JSON that need no
// chart, no canvas and no GPU.
// ------------------------------------------------------------------

/// The default config (`default::default_config`) as JSON: the document a
/// new chart starts from, for an editor to fill in. Its `chart_area` is a
/// placeholder the host sets.
///
/// JS: `const config = JSON.parse(default_config());`
#[wasm_bindgen]
pub fn default_config() -> Result<String, JsValue> {
    crate::default_config_json().map_err(js_err)
}

/// Check a config JSON and, when given, a series list JSON against the
/// chart rules (`Config::validate_chart`, then `Config::validate_series`)
/// without a chart. Returns `{"valid": bool, "errors": [..]}`; it does not
/// throw for a refused document. An error is `{stage, path, field,
/// series_id, series_index, code, message}`: `stage` is `"parse"`,
/// `"chart"` or `"series"`; `path` names the field (`"bottom_x"`,
/// `"colorbar.axis"`, `"series[2]"`, or `"config"` / `"series"` for JSON
/// that does not parse, which adds `line` and `column`); `code` is the
/// rule's code, as on the `Error`s `set_config` throws (`"json"` for a
/// parse error). The rules stop at the first refusal, so `errors` holds at
/// most one. What needs the column pool (registered columns, lengths, GPU
/// limits) is checked by `set_config` / `set_series` only.
///
/// JS: `const { valid, errors } = JSON.parse(validate_config(JSON.stringify(cfg), JSON.stringify(series)));`
#[wasm_bindgen]
pub fn validate_config(config_json: &str, series_json: Option<String>) -> Result<String, JsValue> {
    crate::validate_config_json(config_json, series_json.as_deref()).map_err(js_err)
}

/// A config JSON in its normalized form: read and written back, so an
/// omitted optional field takes its documented default and a field at its
/// default is written as the renderer writes it. Numbers keep every bit.
/// Throws the JSON error when the text is not a config; a config that
/// parses but breaks a rule is returned as it is (`validate_config` checks
/// it).
///
/// JS: `const canonical = normalize_config(text);`
#[wasm_bindgen]
pub fn normalize_config(config_json: &str) -> Result<String, JsValue> {
    crate::normalize_config_json(config_json).map_err(js_err)
}

/// A legacy categorical bar document (`CategoricalChart` JSON) converted
/// without a chart (`Config::from_categorical_with_report`): `{config,
/// series, uploads, columns, notes}`. `uploads` describes each column to
/// register (`{column, role, len}`, without the values); `columns` names
/// the document series of each (`{series_id, role, column}`); `notes` lists
/// every field the document set that the chart does not carry
/// (`{path, kind: "dropped" | "changed", reason}`). An explicit
/// `decorations.data_area` is the converted config's data area, fitted to
/// the placeholder chart area (1000 x 800; #334): a host that sets another
/// chart area fits it again, as the `set_*_document` calls do, and one the
/// layout cannot hold is noted as dropped with the reason. Throws the JSON error,
/// or the document rule (an `Error` with a `code`), as
/// `set_categorical_document` does.
///
/// JS: `const { config, series, notes } = JSON.parse(convert_categorical_document(text));`
#[wasm_bindgen]
pub fn convert_categorical_document(json: &str) -> Result<String, JsValue> {
    crate::convert_categorical_document_json(json).map_err(js_err)
}

/// A legacy box plot document (`BoxPlotChart` JSON) converted without a
/// chart (`Config::from_boxplot_with_report`), in the shape
/// `convert_categorical_document` returns. A record upload is
/// `{column, role: "box_record", rows, fields}` and an outlier upload
/// `{column, role: "box_outliers", len, groups}`.
///
/// JS: `const converted = JSON.parse(convert_boxplot_document(text));`
#[wasm_bindgen]
pub fn convert_boxplot_document(json: &str) -> Result<String, JsValue> {
    crate::convert_boxplot_document_json(json).map_err(js_err)
}

/// A legacy radar document (`RadarChart` JSON) converted without a chart
/// (`Config::from_radar_with_report`), in the shape
/// `convert_categorical_document` returns.
///
/// JS: `const converted = JSON.parse(convert_radar_document(text));`
#[wasm_bindgen]
pub fn convert_radar_document(json: &str) -> Result<String, JsValue> {
    crate::convert_radar_document_json(json).map_err(js_err)
}

/// Every valid `draw_style` mode tag, as a JSON string array.
///
/// Returns JSON text (`FiggyJson`), not an object: parse it with `JSON.parse`.
#[wasm_bindgen(unchecked_return_type = "FiggyJson")]
pub fn draw_style_modes() -> Result<String, JsValue> {
    serde_json::to_string(renderer::config::DrawStyle::mode_tags()).map_err(js_err)
}

// ------------------------------------------------------------------
// FiggyChart — low-level wasm kernel bound to one canvas.
// ------------------------------------------------------------------

#[wasm_bindgen]
pub struct FiggyChart {
    pub(super) renderer: WindowedRenderer<'static>,
    pub(super) chart_id: ChartId,
    pub(super) view: ChartView,
    /// Current WebGPU surface size in physical canvas pixels. This is a
    /// viewport property, not the exported document size.
    pub(super) surface_size: (u32, u32),
    /// True while the legend follows the wrapper's auto row layout.
    /// Direct `set_config` legend edits turn this off so later series
    /// changes preserve user-authored text instead of deleting rows.
    pub(super) legend_auto_managed: bool,
    /// Monotonic color assignment for newly registered series.
    pub(super) color_seq: usize,
    pub(super) hitmap: HitMap,
    /// The (active layer, overlay count, active subplot's overlay count,
    /// subplot count, active overlay's `link_x` / `link_y`) the hit map was
    /// registered for. Any change rebuilds it with
    /// `HitMap::for_active_layer`.
    pub(super) hitmap_layer: (
        renderer::LayerRef,
        usize,
        Option<usize>,
        usize,
        Option<(bool, bool)>,
    ),
    /// The polar category label counts, chart then subplots, the hit map
    /// was registered for (`Config::polar_category_label_counts`, #309):
    /// one entry per panel, compared in place and replaced only when they
    /// change.
    pub(super) hitmap_categories: Vec<usize>,
    pub(super) dragging: bool,
    /// The pointer in document coordinates while a chart element drags:
    /// the press point advanced by every move. Polar axes and tick
    /// labels move relative to it.
    pub(super) drag_pointer: [f32; 2],
    pub(super) resizing: Option<ModelResizeHandle>,
    pub(super) cycle: renderer::ColorCycle,
    pub(super) clear_color: Color,
    pub(super) view_dirty: bool,
    /// Host-owned surface state not represented by renderer chart
    /// revisions (currently the clear color). Starts true so the initial
    /// surface frame is never skipped.
    pub(super) redraw_pending: bool,
    /// Renderer-issued identity of the state last presented onscreen.
    pub(super) last_presented_stamp: Option<ChartRenderStamp>,
    /// The annotation whose selection box the overlay draws. Host state
    /// like the decoration selection; not part of the config JSON.
    pub(super) selected_annotation: Option<usize>,
    /// The annotation selection changed since the overlay was drawn.
    pub(super) annotation_selection_dirty: bool,
    /// The pointer press that started the current drag or handle drag
    /// landed on the selected annotation (the annotation layer's route),
    /// so `on_move` edits the annotation instead of a chart element.
    pub(super) annotation_pointer: bool,
    /// The current drag started on a value label (`press_value_label`), so
    /// `on_move` moves the selected label instead of a chart element.
    pub(super) value_label_pointer: bool,
    /// A subplot frame drag holds the display placement
    /// (`Renderer::hold_chart_display`, #205) until the pointer's release.
    pub(super) display_held: bool,
    /// The current drag started on a part of a Feynman series
    /// (`press_feynman`, #234): the series and what `on_move` moves.
    pub(super) feynman_pointer: Option<(String, renderer::FeynmanHit)>,
    /// The document a plot-area resize gesture proposes on a chart with
    /// monotonic series (#271), built up move by move and never applied:
    /// the resize takes the groups' sources, which the host passes.
    pub(super) plot_resize: Option<renderer::Config>,
    /// The plot rect the last such gesture proposed on release, until the
    /// element reads it (`take_plot_resize_request`).
    pub(super) plot_resize_request: Option<renderer::layout::Rect>,
    /// The monotonic chart the kernel shows instead of its general one
    /// (`set_monotonic_document`, #271), reached only through its handle.
    pub(super) monotonic: Option<renderer::MonotonicChartId>,
}

impl FiggyChart {
    /// What a radial chart created on this chart draws with (#313,
    /// `FiggyBoundedChart.create_on`): this chart's device bundle (its
    /// device, queue, device-loss record, GPU ledger and budget) and the
    /// instance and adapter its own surface came from. Clones of handles:
    /// the device lives while either chart holds them.
    pub(crate) fn shared_gpu(&self) -> (renderer::RendererDevice, wgpu::Instance, wgpu::Adapter) {
        (
            self.renderer.renderer_device(),
            self.renderer.instance().clone(),
            self.renderer.adapter().clone(),
        )
    }

    pub(super) fn chart_config(&self) -> Result<&renderer::Config, JsValue> {
        match self.monotonic {
            Some(id) => self.renderer.monotonic_config(id),
            None => self.renderer.chart_config(self.chart_id),
        }
        .map_err(js_err)
    }

    pub(super) fn chart_series(&self) -> Result<&[SeriesConfig], JsValue> {
        match self.monotonic {
            Some(id) => self.renderer.monotonic_series(id),
            None => self.renderer.chart_series(self.chart_id),
        }
        .map_err(js_err)
    }

    pub(super) fn chart_selection(&self) -> Result<Option<HitId>, JsValue> {
        match self.monotonic {
            Some(id) => self.renderer.monotonic_selection(id),
            None => self.renderer.chart_selection(self.chart_id),
        }
        .map_err(js_err)
    }

    /// Selects a chart element of the chart the kernel shows.
    pub(super) fn set_chart_selection(&mut self, selected: Option<HitId>) -> Result<(), JsValue> {
        match self.monotonic {
            Some(id) => self
                .renderer
                .monotonic_chart(id)
                .and_then(|mut chart| chart.set_selection(selected)),
            None => self.renderer.set_chart_selection(self.chart_id, selected),
        }
        .map_err(js_err)
    }

    /// The document as the shown chart stores it, through the general call
    /// or a monotonic chart's editor (#271), which refuses a document that
    /// would move its x-to-pixel mapping.
    fn write_chart_config(&mut self, config: renderer::Config) -> Result<(), JsValue> {
        match self.monotonic {
            Some(id) => self
                .renderer
                .monotonic_chart(id)
                .and_then(|mut chart| chart.edit_config(|editor| editor.assign(config)))
                .map_err(js_err)?
                .map_err(|kind| {
                    js_err(renderer::FiggyError::InvalidConfig {
                        field: "config",
                        kind,
                    })
                }),
            None => self
                .renderer
                .set_chart_config(self.chart_id, config)
                .map_err(js_err),
        }
    }

    pub(super) fn replace_chart_config(&mut self, config: renderer::Config) -> Result<(), JsValue> {
        self.write_chart_config(config)?;
        self.sync_hitmap()?;
        Ok(())
    }

    pub(super) fn replace_chart_state(
        &mut self,
        config: renderer::Config,
        mut series: Vec<SeriesConfig>,
    ) -> Result<(), JsValue> {
        // A Feynman diagram is fixed where it is first drawn (#234): the
        // wrapper commits the layout of every unplaced vertex as the series
        // is registered, so `get_series` returns placed coordinates.
        for item in &mut series {
            if let Some(diagram) = item.render_type.feynman_mut() {
                diagram.place_vertices().map_err(js_err)?;
            }
        }
        match self.monotonic {
            // The series first; a document the editor refuses puts them back.
            Some(id) => {
                let previous = self.chart_series()?.to_vec();
                self.renderer
                    .monotonic_chart(id)
                    .and_then(|mut chart| chart.set_series(series))
                    .map_err(js_err)?;
                if let Err(error) = self.write_chart_config(config) {
                    // The previous series were valid on this document.
                    self.renderer
                        .monotonic_chart(id)
                        .and_then(|mut chart| chart.set_series(previous))
                        .map_err(js_err)?;
                    return Err(error);
                }
            }
            None => self
                .renderer
                .set_chart_state(self.chart_id, config, series)
                .map_err(js_err)?,
        }
        self.sync_hitmap()?;
        Ok(())
    }

    /// The hit map's registration set depends only on the active layer,
    /// the overlay count, the active subplot's overlay count, the subplot
    /// count (every subplot registers on the base layer) and the active
    /// overlay's links (an own X axis registers on the top side, a followed
    /// Y axis registers the chart's left one) and the number of polar
    /// category labels, each its own element (#309). Rebuild
    /// it when any of them changed; the registry has already dropped any
    /// `HitId` of the previous map. A layer change also clears the
    /// annotation selection: it is the element selection of the
    /// annotation layer, one level below the layer like the registry's
    /// `HitId`.
    pub(super) fn sync_hitmap(&mut self) -> Result<(), JsValue> {
        // Held aside while the config is borrowed; an early error leaves it
        // empty, which only rebuilds the map on the next call.
        let mut categories = std::mem::take(&mut self.hitmap_categories);
        let config = self.chart_config()?;
        let layer = config.active_layer();
        let subplot_overlays = match layer {
            renderer::LayerRef::Subplot(index) | renderer::LayerRef::SubplotOverlay(index, _) => {
                config
                    .subplots
                    .get(index)
                    .map(|subplot| subplot.config.overlays.len())
            }
            _ => None,
        };
        let key = (
            layer,
            config.overlays.len(),
            subplot_overlays,
            config.subplots.len(),
            config
                .overlay_of_layer(layer)
                .map(|overlay| (overlay.link_x, overlay.link_y)),
        );
        let categories_changed = !categories
            .iter()
            .copied()
            .eq(config.polar_category_label_counts());
        if categories_changed {
            categories.clear();
            categories.extend(config.polar_category_label_counts());
        }
        let rebuild = (key != self.hitmap_layer || categories_changed)
            .then(|| HitMap::for_active_layer(config));
        self.hitmap_categories = categories;
        if let Some(map) = rebuild {
            let layer_changed = key.0 != self.hitmap_layer.0;
            self.hitmap = map;
            self.hitmap_layer = key;
            self.dragging = false;
            self.resizing = None;
            self.annotation_pointer = false;
            self.value_label_pointer = false;
            self.feynman_pointer = None;
            if layer_changed {
                self.set_selected_annotation(None);
            }
        }
        Ok(())
    }

    /// The display scale and where the whole document lands on the
    /// surface: fitted to it, or where the held placement puts it while a
    /// subplot frame drags (#205), so pointer conversions and picks follow
    /// what is drawn.
    pub(super) fn display_scale_and_panel(&self) -> Result<(f32, Rect), JsValue> {
        if self.display_held {
            let (placed, _, scale) = self.display_config()?;
            return Ok((scale, placed.document_area()));
        }
        let logical = self.chart_config()?.document_area();
        Ok(renderer::fit_display_panel(
            (logical.width, logical.height),
            self.surface_size,
        ))
    }

    /// The config, panel rect and scale the canvas draws with: the
    /// renderer's placed config for the display `create` and `resize` set,
    /// borrowed while it is current.
    pub(super) fn display_config(
        &self,
    ) -> Result<(std::borrow::Cow<'_, renderer::config::Config>, Rect, f32), JsValue> {
        match self.monotonic {
            Some(id) => self.renderer.monotonic_display_config(id),
            None => self.renderer.chart_display_config(self.chart_id),
        }
        .map_err(js_err)
    }

    fn render_stamp(&self) -> Result<renderer::ChartRenderStamp, JsValue> {
        match self.monotonic {
            Some(id) => self.renderer.monotonic_render_stamp(id),
            None => self.renderer.chart_render_stamp(self.chart_id),
        }
        .map_err(js_err)
    }

    pub(super) fn display_delta_to_document(
        &self,
        dx: f32,
        dy: f32,
    ) -> Result<(f32, f32), JsValue> {
        let (scale, _) = self.display_scale_and_panel()?;
        Ok(if scale > 0.0 {
            (dx / scale, dy / scale)
        } else {
            (dx, dy)
        })
    }

    /// A display point in document coordinates: the inverse of
    /// `display_config_for_surface`'s scale and placement.
    pub(super) fn display_point_to_document(&self, x: f32, y: f32) -> Result<[f32; 2], JsValue> {
        let (scale, panel) = self.display_scale_and_panel()?;
        let logical = self.chart_config()?.document_area();
        Ok(if scale > 0.0 {
            [
                logical.x as f32 + (x - panel.x as f32) / scale,
                logical.y as f32 + (y - panel.y as f32) / scale,
            ]
        } else {
            [x, y]
        })
    }

    pub(super) fn label_text(&self, label: &str) -> Result<RichText, JsValue> {
        let content = &self.chart_config()?.legend.content;
        Ok(RichText {
            segments: rich_segments_from_label(label),
            color: content.color,
            font_size: content.font_size,
            font: content.font.clone(),
            align: content.align,
        })
    }

    /// Explicit reset: rebuild every auto legend row from the series'
    /// `SeriesConfig::label`; a series without one gets no row.
    pub(super) fn rebuild_legend_from_series_labels(&mut self) -> Result<(), JsValue> {
        let series = self.chart_series()?;
        let mut entries: Vec<(SeriesConfig, RichText)> = Vec::new();
        entries.try_reserve(series.len()).map_err(js_err)?;
        for cfg in series {
            if let Some(label) = &cfg.label {
                entries.push((cfg.clone(), label.clone()));
            }
        }

        let mut config = self.chart_config()?.clone();
        config.legend.content.segments.clear();
        for (cfg, label) in entries {
            renderer::config::append_legend_entry_rich(
                &mut config.legend.content,
                renderer::config::series_symbol_segments(
                    &cfg,
                    config.colorbar.as_ref().map(|bar| &bar.colormap),
                ),
                label.segments,
            );
        }
        config.legend.visible = !config.legend.content.segments.is_empty();
        self.replace_chart_config(config)?;
        self.legend_auto_managed = true;
        Ok(())
    }

    pub(super) fn request_host_redraw(&mut self) {
        self.redraw_pending = true;
    }
    /// Whether `id` is a registered column or triangle index column: a
    /// triangle mesh names its triangles among its columns (#210).
    pub(super) fn column_registered(&self, id: &str) -> bool {
        self.renderer.logical_column(id).is_some() || self.renderer.pool().triangles(id).is_some()
    }

    pub(super) fn ensure_columns_exist(&self, cfg: &SeriesConfig) -> Result<(), JsValue> {
        for id in referenced_columns(cfg) {
            if !self.column_registered(id) {
                return Err(js_err(format!(
                    "series '{}' references unregistered column '{id}'",
                    cfg.series_id
                )));
            }
        }
        Ok(())
    }
}

#[wasm_bindgen]
impl FiggyChart {
    /// Bind the low-level chart kernel to `canvas` (uses the canvas's
    /// current pixel size). Ordinary web hosts should prefer
    /// `figgy-chart.js` and its `<figgy-chart>` Custom Element.
    /// JS: `const chart = await FiggyChart.create(canvas);`
    pub async fn create(canvas: HtmlCanvasElement) -> Result<FiggyChart, JsValue> {
        create_chart_kernel(canvas, None, renderer::ShaderSet::Basic).await
    }

    /// Same as [`Self::create`], reporting each init stage to `on_event`
    /// as `{ scope, stage, phase }`. Do not call other kernel methods
    /// from the callback. An exception the callback throws rejects the
    /// returned promise with it; later stages are not reported.
    pub async fn create_with_progress(
        canvas: HtmlCanvasElement,
        on_event: js_sys::Function,
    ) -> Result<FiggyChart, JsValue> {
        create_chart_kernel(canvas, Some(on_event), renderer::ShaderSet::Basic).await
    }

    /// [`Self::create`] with options (#299), all optional:
    ///
    /// - `shaders`: `"basic"` (the default) or `"full"`. A chart starts on
    ///   the basic shader set, which draws linear and logarithmic
    ///   rectangular charts, and moves to the full set by itself when its
    ///   config first has an axis break, a probability axis or polar
    ///   coordinates. A host that knows it will show such a chart passes
    ///   `"full"`, so the basic set is never compiled and discarded.
    /// - `on_progress`: the callback of [`Self::create_with_progress`].
    ///
    /// An unknown option, or a `shaders` value other than those two,
    /// rejects the promise with an `Error` that names it; nothing is
    /// created.
    /// JS: `const chart = await FiggyChart.create_with_options(canvas, { shaders: "full" });`
    pub async fn create_with_options(
        canvas: HtmlCanvasElement,
        options: JsValue,
    ) -> Result<FiggyChart, JsValue> {
        let (set, on_progress) = create_options(&options)?;
        create_chart_kernel(canvas, on_progress, set).await
    }

    /// The shader set this chart compiles (#299): `"basic"` or `"full"`.
    ///
    /// Returns a plain string, not JSON.
    #[wasm_bindgen(unchecked_return_type = "\"basic\" | \"full\"")]
    pub fn shader_set(&self) -> String {
        shader_set_name(self.renderer.shader_set()).into()
    }

    /// Move this chart to at least the shader set `shaders` (`"basic"` or
    /// `"full"`, #299), as its first chart with an axis break, a
    /// probability axis or polar coordinates would. A chart never goes
    /// back, so `"basic"` and a repeated `"full"` do nothing. Any other
    /// value throws an `Error` that names it.
    pub fn require_shader_set(&mut self, shaders: JsValue) -> Result<(), JsValue> {
        let set = shader_set_from_name(&shaders)?;
        self.renderer.require_shader_set(set);
        Ok(())
    }

    /// Register a font (TTF/OTF/TTC bytes) for SSoT `font` family names.
    /// Returns the family names the file declares — use them verbatim in
    /// `content.font` / label styles. Registered fonts win over native
    /// system fonts. A character the chosen face cannot draw falls back
    /// per character to registered fonts (in registration order), then
    /// the bundled Liberation Sans, then, on desktop only, installed
    /// system fonts. Resolution through registered and bundled faces is
    /// therefore identical on web and desktop; installed system fonts
    /// are a desktop-only fallback. Already-drawn text re-rasterizes on
    /// the next `frame()`.
    ///
    /// JS: `chart.register_font(new Uint8Array(await (await fetch(url)).arrayBuffer()))`
    pub fn register_font(&mut self, bytes: &[u8]) -> Result<Vec<String>, JsValue> {
        let families =
            renderer::text_render::register_font_bytes(bytes.to_vec()).map_err(js_err)?;
        // Text may already be on screen in the fallback font — force a
        // decoration re-raster so the registration is visible.
        Ok(families)
    }

    /// Pool growth on an otherwise out-of-space upload, on by default.
    /// Growth remains bounded by the device storage-binding limit and
    /// the configured renderer memory budget; `false` keeps the pool's
    /// current capacity.
    pub fn set_pool_auto_growth(&mut self, enabled: bool) {
        self.renderer.set_pool_growth_policy(if enabled {
            GrowthPolicy::OnAllocFailure
        } else {
            GrowthPolicy::Fixed
        });
    }

    /// Read-only accounting. These are requested GPU allocations, not a
    /// driver VRAM measurement; retired bytes remain charged until the
    /// queue completion callback fires.
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    /// Sizes are in bytes.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn gpu_memory_status(&self) -> String {
        let usage = self.renderer.gpu_memory_usage();
        let pool = self.renderer.pool();
        let mut sources: Vec<_> = self
            .renderer
            .logical_column_ids()
            .filter_map(|id| {
                let column = self.renderer.logical_column(id)?;
                let (residency, encoded_bytes) = match column {
                    renderer::LogicalColumn::Resident(slot) => ("resident", Some(slot.byte_size)),
                    renderer::LogicalColumn::Streamed(_) => ("streamed", None),
                };
                Some(serde_json::json!({
                    "id": id,
                    "length": column.len(),
                    "revision": self.renderer.column_revision(id).map(|r| r.to_string()),
                    "residency": residency,
                    "encoded_bytes": encoded_bytes,
                }))
            })
            .collect();
        sources.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        let resources: Vec<_> = GpuResourceKind::ALL
            .into_iter()
            .map(|kind| {
                serde_json::json!({
                    "kind": kind.label(),
                    "live_bytes": usage.live_bytes_of(kind),
                    "retired_bytes": usage.retired_bytes_of(kind),
                })
            })
            .collect();
        serde_json::json!({
            "total_bytes": usage.total_bytes(),
            "live_bytes": usage.live_bytes(),
            "retired_bytes": usage.retired_bytes(),
            "memory_budget_bytes": self.renderer.memory_budget(),
            "pool": {
                "capacity_bytes": pool.capacity(),
                "used_bytes": pool.used_bytes(),
                "free_bytes": pool.free_bytes(),
                "largest_free_bytes": pool.largest_free_region(),
                "backup_bytes": pool.backup_bytes(),
                "retired_bytes": usage.retired_bytes_of(renderer::GpuResourceKind::ColumnPool),
                "auto_growth": pool.growth_policy == GrowthPolicy::OnAllocFailure,
            },
            "resources": resources,
            "sources": sources,
        })
        .to_string()
    }

    /// Compact live resident columns, drop the unused pool slab, and wait
    /// for the device completion fence before reporting reclaimed bytes.
    /// The 16 MiB startup slab remains as the minimum reusable pool.
    ///
    /// Resolves to JSON text (`FiggyJson`) in the shape of `gpu_memory_status`,
    /// not an object.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub async fn release_unused_gpu_memory(&mut self) -> Result<String, JsValue> {
        let status = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
        if matches!(
            status.status,
            renderer::StreamingState::Active
                | renderer::StreamingState::AllSubmitted
                | renderer::StreamingState::Cancelling
        ) {
            return Err(js_err(
                "finish or cancel the active stream before GPU pool cleanup",
            ));
        }
        self.renderer
            .release_unused_pool_memory(POOL_CAPACITY)
            .await
            .map_err(column_registration_error)?;
        Ok(self.gpu_memory_status())
    }

    // ---- series registry (id-keyed upsert / unregister) ----

    /// Register or update a line series over two registered columns.
    ///
    /// Upsert by `series_id`: a new id takes the next color of the active
    /// cycle; an existing id is replaced in place and keeps its color.
    /// Non-empty `label` adds/updates the legend row; its `$…$` spans are
    /// TeX and `\$` is a dollar sign.
    pub fn add_line_series(
        &mut self,
        series_id: &str,
        x_column: &str,
        y_column: &str,
        line_width: f32,
        label: &str,
    ) -> Result<(), JsValue> {
        let current = self.chart_series()?.to_vec();
        let existing = current.iter().position(|c| c.series_id == series_id);
        let color = match existing {
            Some(i) => match &current[i].render_type {
                DataRenderType::Xy(renderer::data_config::XySeries {
                    scatter: None,
                    line: Some(line),
                    errors: None,
                    ..
                }) => line.line_color,
                _ => self.cycle.color(i),
            },
            None => self.cycle.color(self.color_seq),
        };
        let next_color_seq = if existing.is_none() {
            self.color_seq
                .checked_add(1)
                .ok_or_else(|| js_err("series color sequence exhausted"))?
        } else {
            self.color_seq
        };
        let label_changed = !label.is_empty();
        let rich_label = if label_changed {
            Some(self.label_text(label)?)
        } else {
            existing.and_then(|i| current[i].label.clone())
        };

        let cfg = SeriesConfig {
            series_id: series_id.into(),
            source_id: None,
            label: rich_label.clone(),
            x_column: x_column.into(),
            y_column: y_column.into(),
            z_column: None,
            render_type: DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: None,
                line: Some(DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: color,
                    line_width: line_width.max(0.5),
                    fill: None,
                    fill_color: None,
                    interpolation: renderer::data_config::LineInterpolation::Linear,
                }),
                errors: None,
                stem: None,
            }),
        };
        self.ensure_columns_exist(&cfg)?;
        let legend_entry = renderer::config::legend_entry_of_series(
            &self.chart_config()?.legend.content,
            &current,
            series_id,
        );
        let mut proposed = current;
        if let Some(index) = existing {
            proposed[index] = cfg.clone();
        } else {
            proposed.try_reserve(1).map_err(js_err)?;
            proposed.push(cfg.clone());
        }
        self.renderer
            .preflight_chart_styles(&proposed)
            .map_err(js_err)?;
        let mut next_config = self.chart_config()?.clone();
        if label_changed && let Some(label) = rich_label {
            match legend_entry {
                Some(index) => renderer::config::set_legend_entry_label(
                    &mut next_config.legend.content,
                    index,
                    renderer::config::series_symbol_segments(
                        &cfg,
                        next_config.colorbar.as_ref().map(|bar| &bar.colormap),
                    ),
                    label.segments,
                ),
                None => renderer::config::append_legend_entry_rich(
                    &mut next_config.legend.content,
                    renderer::config::series_symbol_segments(
                        &cfg,
                        next_config.colorbar.as_ref().map(|bar| &bar.colormap),
                    ),
                    label.segments,
                ),
            }
            next_config.legend.visible = true;
        } else {
            renderer::config::update_legend_symbols_preserving_text(
                &mut next_config.legend.content,
                &proposed,
                next_config.colorbar.as_ref().map(|bar| &bar.colormap),
            );
        }

        self.replace_chart_state(next_config, proposed)?;
        self.color_seq = next_color_seq;
        Ok(())
    }

    /// Set / change / remove a series' legend label. `'\n'` breaks lines;
    /// unicode sub/superscripts (`₀`, `⁻`, …) map to styled segments;
    /// `$…$` spans are TeX and `\$` is a dollar sign.
    /// Empty string removes the legend row. Returns `true` when the
    /// series exists.
    pub fn set_series_label(&mut self, series_id: &str, label: &str) -> Result<bool, JsValue> {
        let mut series = self.chart_series()?.to_vec();
        let Some(i) = series.iter().position(|c| c.series_id == series_id) else {
            return Ok(false);
        };
        let mut config = self.chart_config()?.clone();
        let entry =
            renderer::config::legend_entry_of_series(&config.legend.content, &series, series_id);
        if label.is_empty() {
            series[i].label = None;
            if let Some(entry) = entry
                && renderer::config::remove_legend_entry(&mut config.legend.content, entry)
                && config.legend.content.segments.is_empty()
            {
                config.legend.visible = false;
            }
        } else {
            let label = self.label_text(label)?;
            series[i].label = Some(label.clone());
            let symbol = renderer::config::series_symbol_segments(
                &series[i],
                config.colorbar.as_ref().map(|bar| &bar.colormap),
            );
            match entry {
                Some(entry) => renderer::config::set_legend_entry_label(
                    &mut config.legend.content,
                    entry,
                    symbol,
                    label.segments,
                ),
                None => renderer::config::append_legend_entry_rich(
                    &mut config.legend.content,
                    symbol,
                    label.segments,
                ),
            }
            config.legend.visible = true;
        }
        self.replace_chart_state(config, series)?;
        Ok(true)
    }

    /// Unregister a series. Columns stay registered. Auto-managed legends
    /// remove its row; freely edited legends preserve their text and only
    /// synchronize recognized series symbols.
    /// Returns `true` when the series existed.
    pub fn remove_series(&mut self, series_id: &str) -> Result<bool, JsValue> {
        let mut series = self.chart_series()?.to_vec();
        let Some(i) = series.iter().position(|c| c.series_id == series_id) else {
            return Ok(false);
        };
        let mut config = self.chart_config()?.clone();
        let entry =
            renderer::config::legend_entry_of_series(&config.legend.content, &series, series_id);
        series.remove(i);
        if self.legend_auto_managed {
            if let Some(entry) = entry
                && renderer::config::remove_legend_entry(&mut config.legend.content, entry)
                && config.legend.content.segments.is_empty()
            {
                config.legend.visible = false;
            }
        } else {
            renderer::config::update_legend_symbols_preserving_text(
                &mut config.legend.content,
                &series,
                config.colorbar.as_ref().map(|bar| &bar.colormap),
            );
        }
        self.replace_chart_state(config, series)?;
        Ok(true)
    }

    /// Replace one contour series' levels with axis-native nice levels.
    /// The resulting explicit values remain the series SSoT.
    pub fn set_contour_nice_levels(
        &mut self,
        series_id: &str,
        target_count: usize,
        use_colormap_colors: bool,
    ) -> Result<u32, JsValue> {
        let colorbar = self
            .chart_config()?
            .colorbar
            .clone()
            .ok_or_else(|| js_err("contour levels need Config.colorbar"))?;
        let mut series = self.chart_series()?.to_vec();
        let cfg = series
            .iter_mut()
            .find(|cfg| cfg.series_id == series_id)
            .ok_or_else(|| js_err("no series with this id in the chart"))?;
        let contour = match &mut cfg.render_type {
            DataRenderType::Field(renderer::data_config::FieldSeries {
                contour: Some(contour),
                ..
            }) => contour,
            _ => return Err(js_err("series does not have contour levels")),
        };
        contour
            .set_nice_levels(&colorbar.axis, target_count)
            .map_err(|error| js_err(format!("{error:?}")))?;
        if use_colormap_colors {
            contour.set_colormap_colors(&colorbar);
        }
        let count = u32::try_from(contour.levels.len()).map_err(js_err)?;
        let json = serde_json::to_string(&series).map_err(js_err)?;
        self.set_series(&json)?;
        self.request_host_redraw();
        Ok(count)
    }

    /// Report the effective primitive/grid extent currently drawn.
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    /// `{ drawn_count, cols, rows, truncated }`: counts, not pixels.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn series_draw_info(&self, series_id: &str) -> Result<String, JsValue> {
        let info = match self.monotonic {
            Some(id) => self.renderer.monotonic_series_draw_info(id, series_id),
            None => self.renderer.series_draw_info(self.chart_id, series_id),
        }
        .map_err(js_err)?;
        serde_json::to_string(&serde_json::json!({
            "drawn_count": info.drawn_count,
            "cols": info.cols,
            "rows": info.rows,
            "truncated": info.truncated,
        }))
        .map_err(js_err)
    }

    // ---- SSoT I/O ----
    //
    // The whole option tree (`Config`) is plain data; these round-trip it
    // as JSON so a host can read it, edit anything — axis scale, tick
    // shape/length, colors, fonts, label text — and hand it back. The
    // standard flow: auto-fit first, then refine via the SSoT.
    // Full schema reference: crates/web/SCHEMA.md.

    /// Serialize the full chart option SSoT to a JSON string.
    /// JS: `const cfg = JSON.parse(chart.get_config());`
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    /// Positions and sizes in it (`chart_area`, margins, decoration offsets)
    /// are document logical pixels, not canvas pixels.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn get_config(&self) -> Result<String, JsValue> {
        serde_json::to_string(self.chart_config()?).map_err(js_err)
    }

    /// Replace the whole option SSoT from JSON. Marks everything dirty —
    /// the next `frame()` re-rasters the chrome and refreshes the
    /// transform, exactly like any other config edit.
    /// Use `get_config`, edit only the desired fields, then submit the
    /// complete config here for selective control. For example, writing
    /// only axis `min` / `max` preserves tick and label settings, whereas
    /// the auto-fit helpers derive those settings along with the range.
    /// This call does not auto-fit ranges or fit the title band.
    /// JS: `chart.set_config(JSON.stringify(cfg));`
    pub fn set_config(&mut self, json: &str) -> Result<(), JsValue> {
        let mut new_cfg: renderer::Config = serde_json::from_str(json).map_err(js_err)?;
        let old_config = self.chart_config()?;
        let legend_content_changed = old_config.legend.content != new_cfg.legend.content;
        let colormap_changed = old_config.colorbar.as_ref().map(|bar| &bar.colormap)
            != new_cfg.colorbar.as_ref().map(|bar| &bar.colormap);
        let series = self.chart_series()?.to_vec();
        // A heatmap's legend mark shows the colour map: an auto-managed
        // legend follows a new map, a host-written one stays as written.
        if colormap_changed && !legend_content_changed && self.legend_auto_managed {
            renderer::config::update_legend_symbols_preserving_text(
                &mut new_cfg.legend.content,
                &series,
                new_cfg.colorbar.as_ref().map(|bar| &bar.colormap),
            );
        }
        self.renderer
            .preflight_chart_styles(&series)
            .map_err(js_err)?;
        self.replace_chart_config(new_cfg)?;
        if legend_content_changed {
            self.legend_auto_managed = false;
        }
        Ok(())
    }

    /// Serialize the series declarations (columns, render type, styles).
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn get_series(&self) -> Result<String, JsValue> {
        serde_json::to_string(self.chart_series()?).map_err(js_err)
    }

    /// Replace the series declarations from JSON. Column references must
    /// already be registered; GPU styles are rebuilt, legend labels are
    /// kept for series ids that survive.
    ///
    /// `Contour` and `HeatmapContour` accept `contour.levels` lists with
    /// 0 through 1024 entries. A list with 1025 or more entries rejects the
    /// JSON call with a JavaScript exception; the previously committed
    /// chart config, series declarations, and GPU styles remain active.
    /// The WASM facade neither adds a smaller limit nor truncates levels.
    /// Contour labels use the same 1024 capacity for automatic and explicit
    /// placement. `spacing_px` must be finite and positive even for hidden
    /// labels or explicit anchors; rejection preserves the prior state.
    pub fn set_series(&mut self, json: &str) -> Result<(), JsValue> {
        let new_series: Vec<SeriesConfig> = serde_json::from_str(json).map_err(js_err)?;
        for cfg in &new_series {
            self.ensure_columns_exist(cfg)?;
        }
        self.renderer
            .preflight_chart_styles(&new_series)
            .map_err(js_err)?;
        let mut config = self.chart_config()?.clone();
        if self.legend_auto_managed {
            let colormap = config.colorbar.as_ref().map(|bar| &bar.colormap);
            let content = &mut config.legend.content;
            // Entries pair with series by the series their symbol names
            // (a symbol that names none pairs by position): drop the
            // entries whose series is gone, restyle the rest, and add one
            // for each labeled series without an entry.
            for index in (0..renderer::config::legend_entry_count(content)).rev() {
                let kept = match renderer::config::legend_entry_series_id(content, index) {
                    Some(series_id) => new_series.iter().any(|cfg| cfg.series_id == series_id),
                    None => index < new_series.len(),
                };
                if !kept {
                    renderer::config::remove_legend_entry(content, index);
                }
            }
            renderer::config::update_legend_symbols_preserving_text(content, &new_series, colormap);
            for cfg in &new_series {
                if let Some(label) = &cfg.label
                    && renderer::config::legend_entry_of_series(
                        content,
                        &new_series,
                        &cfg.series_id,
                    )
                    .is_none()
                {
                    renderer::config::append_legend_entry_rich(
                        content,
                        renderer::config::series_symbol_segments(cfg, colormap),
                        label.segments.clone(),
                    );
                }
            }
            config.legend.visible = !config.legend.content.segments.is_empty();
        }
        let new_len = new_series.len();
        self.replace_chart_state(config, new_series)?;
        self.color_seq = new_len.max(self.color_seq);
        Ok(())
    }

    /// Open a legacy categorical bar chart document (`CategoricalChart` JSON,
    /// what `FiggyBoundedChart` took as the `"categorical"` family) as this
    /// Cartesian chart.
    ///
    /// `Config::from_categorical` turns the document into a config with a
    /// categorical axis and `bar_groups`, one `CategoryBars` series per
    /// document series and the columns those read: the category positions
    /// (`categorical.positions`) and one value column per series
    /// (`categorical.values.<series id>`), registered as one batch. The
    /// value axis is fitted to the bars from the columns' statistics and
    /// rounded to nice ticks. The chart keeps its chart area, and an explicit
    /// `decorations.data_area` (logical pixels, edges rounded to whole
    /// pixels) is the data area in it (`Config::fit_legacy_data_area`, #334)
    /// unless the layout cannot hold it, which keeps the automatic layout; the series,
    /// columns and config of a categorical document opened before
    /// are replaced, every other column stays. From here on the chart is
    /// edited through the Cartesian API (`get_config` / `set_config`,
    /// `get_series` / `set_series`, the column registry).
    ///
    /// The replace is atomic (#311): see "Document replace" on
    /// [`Self::set_radar_document`]. The ids are the documented ones for
    /// every document opened.
    ///
    /// JS: `chart.set_categorical_document(JSON.stringify(doc));`
    // The legacy document is read through its deprecated reader type.
    #[allow(deprecated)]
    pub fn set_categorical_document(&mut self, json: &str) -> Result<(), JsValue> {
        const FAMILY: &str = "categorical";
        let validate = |error: JsValue| document_stage_error(FAMILY, "validate", error);
        let mut document: renderer::CategoricalChart =
            serde_json::from_str(json).map_err(|e| validate(js_err(e)))?;
        document.validate().map_err(|e| validate(js_err(e)))?;
        // The explicit data area is fitted to this chart's area, on the
        // automatic layout converted without it (#334).
        let data_area = document.decorations.data_area.take();
        let (mut config, series, uploads) = renderer::config::Config::from_categorical(&document);
        config.chart_area = self.chart_config().map_err(validate)?.chart_area.clone();
        if let Some(area) = &data_area {
            match config.fit_legacy_data_area(area) {
                Ok(()) => {}
                // A refusal changes nothing: the field is dropped and the
                // automatic layout stays, as `convert_*_document` reports
                // it.
                Err(
                    renderer::layout::LayoutError::TargetOutOfChartArea
                    | renderer::layout::LayoutError::EmptyDataArea
                    | renderer::layout::LayoutError::Infeasible
                    | renderer::layout::LayoutError::OverflowHorizontal { .. }
                    | renderer::layout::LayoutError::OverflowVertical { .. },
                ) => {}
            }
        }
        let horizontal = config
            .bar_groups
            .as_ref()
            .is_some_and(|g| g.direction == renderer::CategoryBarDirection::Horizontal);
        self.replace_document(FAMILY, config, series, &uploads, |pool, config, series| {
            // Fit the value axis to the bars and round it to nice ticks. A
            // range fit resets the label format, so the document's is put
            // back.
            let format = if horizontal {
                config.bottom_x.label_style.format.clone()
            } else {
                config.left_y.label_style.format.clone()
            };
            let mut fitted = renderer::Chart::new(config);
            fitted
                .auto_fit_layer_union(pool, renderer::LayerRef::Base, series, 0.0)
                .map_err(js_err)?;
            let mut config = fitted.config().clone();
            let value = if horizontal {
                &mut config.bottom_x
            } else {
                &mut config.left_y
            };
            value.label_style.format = format;
            let (min, max) = (value.min, value.max);
            // A fitted range that ticks cannot round (`TickError`: empty,
            // non-finite or non-positive on a log axis) keeps the fit as it is.
            if value.auto_ticks(min, max, 5).is_ok() {
                let fitted = value.clone();
                let twin = if horizontal {
                    &mut config.top_x
                } else {
                    &mut config.right_y
                };
                twin.min = fitted.min;
                twin.max = fitted.max;
                twin.major_spacing = fitted.major_spacing;
                twin.minor_count = fitted.minor_count;
            }
            Ok(config)
        })
    }

    /// Open a legacy box plot document (`BoxPlotChart` JSON, what
    /// `FiggyBoundedChart` took as the `"boxplot"` family) as this Cartesian
    /// chart.
    ///
    /// `Config::from_boxplot` turns the document into a config with a
    /// categorical axis and a grouped `bar_groups`, one `BoxPlot` series per
    /// document series and what those read: the category positions
    /// (`boxplot.positions`, a registry column), and per series its record
    /// of box statistics (`boxplot.record.<series id>`, as `register_record`
    /// registers one) and, when it has outliers, its ragged outlier column
    /// (`boxplot.outliers.<series id>`, as `register_ragged`). The summaries
    /// upload through borrowed adapters; nothing is copied. Without a
    /// document `value_range` the value axis is fitted to the records' and
    /// outliers' statistics with a tenth of the span as padding and rounded
    /// to nice ticks. The chart keeps its chart area, and an explicit
    /// `decorations.data_area` (logical pixels, edges rounded to whole
    /// pixels) is the data area in it (`Config::fit_legacy_data_area`, #334)
    /// unless the layout cannot hold it, which keeps the automatic layout; the series, columns,
    /// records and config of a box plot document opened before are
    /// replaced, every other column stays. From here on the chart is edited
    /// through the Cartesian API.
    ///
    /// The replace is atomic (#311): see "Document replace" on
    /// [`Self::set_radar_document`]. The ids are the documented ones for
    /// every document opened.
    ///
    /// JS: `chart.set_boxplot_document(JSON.stringify(doc));`
    // The legacy document is read through its deprecated reader type.
    #[allow(deprecated)]
    pub fn set_boxplot_document(&mut self, json: &str) -> Result<(), JsValue> {
        const FAMILY: &str = "boxplot";
        let validate = |error: JsValue| document_stage_error(FAMILY, "validate", error);
        let mut document: renderer::BoxPlotChart =
            serde_json::from_str(json).map_err(|e| validate(js_err(e)))?;
        document.validate().map_err(|e| validate(js_err(e)))?;
        // The explicit data area is fitted to this chart's area, on the
        // automatic layout converted without it (#334).
        let data_area = document.decorations.data_area.take();
        let (mut config, series, uploads) = renderer::config::Config::from_boxplot(&document);
        config.chart_area = self.chart_config().map_err(validate)?.chart_area.clone();
        if let Some(area) = &data_area {
            match config.fit_legacy_data_area(area) {
                Ok(()) => {}
                // A refusal changes nothing: the field is dropped and the
                // automatic layout stays, as `convert_*_document` reports
                // it.
                Err(
                    renderer::layout::LayoutError::TargetOutOfChartArea
                    | renderer::layout::LayoutError::EmptyDataArea
                    | renderer::layout::LayoutError::Infeasible
                    | renderer::layout::LayoutError::OverflowHorizontal { .. }
                    | renderer::layout::LayoutError::OverflowVertical { .. },
                ) => {}
            }
        }
        let horizontal = document.direction == renderer::BoxPlotDirection::Horizontal;
        let fit_range = document.value_range.is_none();
        self.replace_document(FAMILY, config, series, &uploads, |pool, config, series| {
            if !fit_range {
                return Ok(config);
            }
            let format = if horizontal {
                config.bottom_x.label_style.format.clone()
            } else {
                config.left_y.label_style.format.clone()
            };
            let mut fitted = renderer::Chart::new(config);
            fitted
                .auto_fit_layer_union(pool, renderer::LayerRef::Base, series, 0.1)
                .map_err(js_err)?;
            let mut config = fitted.config().clone();
            let value = if horizontal {
                &mut config.bottom_x
            } else {
                &mut config.left_y
            };
            value.label_style.format = format;
            let (min, max) = (value.min, value.max);
            // A fitted range that ticks cannot round (`TickError`: empty,
            // non-finite or non-positive on a log axis) keeps the fit as it is.
            if value.auto_ticks(min, max, 5).is_ok() {
                let fitted = value.clone();
                let twin = if horizontal {
                    &mut config.top_x
                } else {
                    &mut config.right_y
                };
                twin.min = fitted.min;
                twin.max = fitted.max;
                twin.major_spacing = fitted.major_spacing;
                twin.minor_count = fitted.minor_count;
            }
            Ok(config)
        })
    }

    /// Open a legacy radar document (`RadarChart` JSON, what
    /// `FiggyBoundedChart` took as the `"radar"` family) as this polar chart.
    ///
    /// `Config::from_radar` turns the document into a polar config whose
    /// angle axis holds the document's categories, one `Radar` series per
    /// document series and the columns those read: the category indices
    /// (`radar.positions`) and one value column per series
    /// (`radar.values.<series id>`). The values upload through borrowed
    /// adapters; nothing is copied. Without a document `value_range` the
    /// radius axis is fitted to the value columns' statistics, from zero
    /// when no value is negative (as the former chart fitted it), and
    /// rounded to nice ticks; a document `ring_spacing` is the ring spacing
    /// either way. The chart keeps its chart area, and an explicit
    /// `decorations.data_area` (logical pixels, edges rounded to whole
    /// pixels) is the data area in it (`Config::fit_legacy_data_area`, #334)
    /// unless the layout cannot hold it, which keeps the automatic layout; the series, columns and
    /// config of a radar document opened before are replaced, every other
    /// column stays. From here on the chart is edited through the Cartesian
    /// API (`get_config` / `set_config`, `get_series` / `set_series`, the
    /// column registry), and its vertices pick, select and step as points.
    ///
    /// # Document replace
    ///
    /// The three document calls replace the chart atomically (#311), in
    /// one renderer transaction. They prepare the candidate config, series
    /// and uploads, check them (styles; each documented id is free or holds
    /// what the document uploads under it: a column, a record or a ragged
    /// column, and is not streamed), then upload every column, record and
    /// ragged column under its documented id (`radar.positions`,
    /// `boxplot.record.<series id>`, ...), fit the axes from the candidate
    /// columns, and commit the columns, the removal of the previous
    /// document's columns and the chart's config and series in one step.
    /// The previous document's columns are each id under `categorical.`,
    /// `boxplot.` or `radar.` that the replaced series read, of whichever
    /// family, that the new document does not upload again; an id it does
    /// upload again is replaced in place. The ids are therefore the
    /// documented ones for every document opened.
    /// On a chart that shows a monotonic document
    /// (`set_monotonic_document`), the document replaces the general chart
    /// behind it and the monotonic chart ends with the commit, as
    /// `clear_monotonic_document` ends it; a failure leaves the monotonic
    /// document shown.
    ///
    /// A failure rejects with an `Error` whose `stage` is `"validate"`
    /// (parse, validation, styles, ids), `"upload"` (a column that the GPU or the memory budget cannot hold, with the `code` and byte fields of
    /// `register_column_f32`), `"fit"` or `"commit"` (the series checks
    /// against the candidate columns, the chart's revisions, the picker)
    /// and whose message names the stage and the cause. The chart's
    /// config, series, columns, element and data selections, picks and
    /// image are then exactly as they were: nothing reaches the chart
    /// before the commit, which cannot fail. Only a broken internal
    /// invariant after the commit (the monotonic chart or the chart itself
    /// gone) rejects then, with an error that names it as internal and no
    /// `stage`; the new document is shown in that case.
    ///
    /// Memory: the transaction builds a complete candidate column pool, so
    /// at the peak the GPU holds the live pool slab, the new document's
    /// staging (half of it for single-lane columns, plus their transient
    /// expansion buffer) and a candidate slab of the pool's capacity — or a
    /// grown one, twice the capacity or what the surviving columns and the
    /// new document need, when they do not fit. The previous document's
    /// columns take no room in the candidate. After the commit the former
    /// slab stays as the pool's backup when the capacity did not change.
    /// A memory budget that cannot hold the peak refuses the call at the
    /// upload stage with `code` `"budget_exceeded"`.
    ///
    /// JS: `chart.set_radar_document(JSON.stringify(doc));`
    // The legacy document is read through its deprecated reader type.
    #[allow(deprecated)]
    pub fn set_radar_document(&mut self, json: &str) -> Result<(), JsValue> {
        const FAMILY: &str = "radar";
        let validate = |error: JsValue| document_stage_error(FAMILY, "validate", error);
        let mut document: renderer::RadarChart =
            serde_json::from_str(json).map_err(|e| validate(js_err(e)))?;
        document.validate().map_err(|e| validate(js_err(e)))?;
        // The explicit data area is fitted to this chart's area, on the
        // automatic layout converted without it (#334).
        let data_area = document.decorations.data_area.take();
        let (mut config, series, uploads) = renderer::config::Config::from_radar(&document);
        config.chart_area = self.chart_config().map_err(validate)?.chart_area.clone();
        if let Some(area) = &data_area {
            match config.fit_legacy_data_area(area) {
                Ok(()) => {}
                // A refusal changes nothing: the field is dropped and the
                // automatic layout stays, as `convert_*_document` reports
                // it.
                Err(
                    renderer::layout::LayoutError::TargetOutOfChartArea
                    | renderer::layout::LayoutError::EmptyDataArea
                    | renderer::layout::LayoutError::Infeasible
                    | renderer::layout::LayoutError::OverflowHorizontal { .. }
                    | renderer::layout::LayoutError::OverflowVertical { .. },
                ) => {}
            }
        }
        let fit_range = document.value_range.is_none();
        let ring_spacing = document.ring_spacing;
        self.replace_document(FAMILY, config, series, &uploads, |pool, config, series| {
            if !fit_range {
                return Ok(config);
            }
            // Fit the radius axis to the values, from zero when none is
            // negative, and round it to nice ticks. A range fit resets the
            // label format, so the document's is put back.
            let format = config.left_y.label_style.format.clone();
            let mut fitted = renderer::Chart::new(config);
            fitted
                .auto_fit_layer_union(pool, renderer::LayerRef::Base, series, 0.0)
                .map_err(js_err)?;
            let mut config = fitted.config().clone();
            let radius = &mut config.left_y;
            radius.label_style.format = format;
            let min = if radius.min >= 0.0 { 0.0 } else { radius.min };
            let max = radius.max;
            // A fitted range that ticks cannot round (`TickError`: empty,
            // non-finite or non-positive on a log axis) keeps the fit as it is.
            if radius.auto_ticks(min, max, 5).is_ok()
                && let Some(spacing) = ring_spacing
            {
                radius.major_spacing = spacing;
            }
            Ok(config)
        })
    }

    // ---- frame / resize / export ----

    /// Wait until previously submitted GPU work has finished. Does not
    /// draw. Used to separate an earlier compute submit from the next
    /// render submit when measuring startup.
    pub async fn wait_submitted_work(&self) -> Result<(), JsValue> {
        self.renderer.wait_submitted_work().await.map_err(js_err)
    }

    /// Compile the drawable-series fit-bound engine without drawing. On wasm
    /// this uses `createComputePipelineAsync`. Series registration does
    /// not do this; `auto_fit_all` does.
    pub async fn ensure_extent_engine(&mut self) -> Result<(), JsValue> {
        self.renderer
            .ensure_errorbar_extent_engine()
            .await
            .map_err(js_err)
    }

    /// Submit the current chart if it is dirty, then wait until that GPU
    /// work has finished. Hosts should keep a loading overlay up until
    /// this resolves. The wait does not remove a main-thread Dawn/driver
    /// freeze; it only reports first-submit completion. While a series'
    /// pipelines still compile in the background its frame leaves it out
    /// (#299); this draws again as they resolve and settles with the frame
    /// that holds every series.
    ///
    /// JS: `await chart.first_frame_ready();`
    pub async fn first_frame_ready(&mut self) -> Result<(), JsValue> {
        self.draw_until_complete().await?;
        renderer::init::wait_browser_submitted_work(self.renderer.queue())
            .await
            .map_err(js_err)?;
        Ok(())
    }

    /// Draw until a frame draws every series (#299). A frame that left a
    /// series out while its pipelines compile waits for a compile to change
    /// state, then draws again. A frame leaves a series out only while one
    /// of its compiles is waiting or compiling, and every compile ends
    /// (ready, rejected, device lost, superseded or cancelled), so this ends.
    async fn draw_until_complete(&mut self) -> Result<(), JsValue> {
        self.frame()?;
        while !self.renderer.frame_complete() {
            if self.renderer.compiles_pending() {
                self.renderer.compile_progress().await;
            }
            self.redraw_pending = true;
            self.frame()?;
            if !self.renderer.frame_complete() && !self.renderer.compiles_pending() {
                // Nothing is compiling, yet a series was left out: no
                // compile will ever wake this wait.
                return Err(JsValue::from_str(
                    "a series waits for a pipeline that no compile is pending for",
                ));
            }
        }
        Ok(())
    }

    /// Resolves once a frame draws every series of the chart (#299),
    /// drawing again as the pipelines it waits for compile. It rejects with
    /// an `Error` carrying `label`, `kind` and `message` when one of those
    /// compiles failed (see [`Self::pipeline_failures`]); the chart still
    /// draws that series, and the draw reports the error as usual.
    ///
    /// JS: `await chart.series_ready();`
    pub async fn series_ready(&mut self) -> Result<(), JsValue> {
        self.draw_until_complete().await?;
        self.renderer
            .series_ready()
            .await
            .map_err(|failure| failure_error(&failure))
    }

    /// Resolves once every background compile scheduled so far has ended
    /// (#299). It rejects like [`Self::series_ready`] with the first that
    /// failed. It draws nothing.
    ///
    /// JS: `await chart.pipelines_ready();`
    pub async fn pipelines_ready(&mut self) -> Result<(), JsValue> {
        self.renderer
            .pipelines_ready()
            .await
            .map_err(|failure| failure_error(&failure))
    }

    /// Whether the last frame drew every series of the chart (#299): false
    /// while a series waits for its pipelines' background compile, which
    /// the frames that follow draw as they resolve.
    pub fn frame_complete(&self) -> bool {
        self.renderer.frame_complete()
    }

    /// The chart's background pipeline compiles (#299) as an object:
    /// `{ screen: { total, ready, compiling, failed }, all: { … },
    /// frame_complete }`. `screen` counts what the last frame needed, so a
    /// progress bar over it never goes back; `all` grows when a chart first
    /// needs a module compiled on demand. `compiling` counts the waiting
    /// compiles too.
    ///
    /// JS: `const { screen, all, frame_complete } = chart.pipeline_status();`
    #[wasm_bindgen(unchecked_return_type = "FiggyPipelineStatus")]
    pub fn pipeline_status(&self) -> Result<JsValue, JsValue> {
        let status = self.renderer.pipeline_status();
        let object = js_sys::Object::new();
        js_sys::Reflect::set(&object, &"screen".into(), &counts_object(status.screen)?)?;
        js_sys::Reflect::set(&object, &"all".into(), &counts_object(status.all)?)?;
        js_sys::Reflect::set(
            &object,
            &"frame_complete".into(),
            &status.frame_complete.into(),
        )?;
        Ok(object.into())
    }

    /// Every background compile that failed (#299), in order, as
    /// `[{ label, kind, message }]`. `kind` is `"validation"` or
    /// `"internal"` (the browser's `GPUPipelineError` reason), `"rejected"`
    /// (any other rejection), `"device_lost"` or `"cancelled"`.
    ///
    /// JS: `for (const { label, kind } of chart.pipeline_failures()) …`
    ///
    /// Returns an array of plain objects, not a JSON string.
    #[wasm_bindgen(unchecked_return_type = "FiggyPipelineFailure[]")]
    pub fn pipeline_failures(&self) -> Result<js_sys::Array, JsValue> {
        let array = js_sys::Array::new();
        for failure in self.renderer.pipeline_failures() {
            array.push(&failure_object(&failure)?);
        }
        Ok(array)
    }

    /// A handle on the chart's background pipeline compiles that does not
    /// borrow the chart (#315): see [`FiggyReadiness`]. Its waits leave the
    /// chart free for `pipeline_status()`, `frame()`, `resize()` and
    /// `free()`; once the chart is freed they reject with a
    /// `"chart disposed"` `Error`. `series_ready()`, `pipelines_ready()` and
    /// `first_frame_ready()` on the chart keep working as before.
    ///
    /// JS: `const readiness = chart.readiness(); await readiness.pipelines_ready();`
    pub fn readiness(&self) -> FiggyReadiness {
        FiggyReadiness {
            readiness: self.renderer.pipeline_readiness(),
        }
    }

    /// Alias for [`Self::first_frame_ready`].
    pub async fn warm_up(&mut self) -> Result<(), JsValue> {
        self.first_frame_ready().await
    }

    /// Prepare the resident Cartesian pipeline groups while reporting the
    /// current stage as `{ scope, stage, phase }` objects. Separate bounded
    /// renderers, nonresident matrix/tile pipelines and future chart-specific
    /// allocations are outside this scope. The chart is mutably
    /// borrowed until the Promise settles; hosts must keep their normal
    /// async busy gate active and queue chart work rather than calling the
    /// kernel from the callback. An exception the callback throws rejects
    /// the returned promise with it once the prewarm returns; later stages
    /// are not reported.
    pub async fn prewarm_all_with_progress(
        &mut self,
        on_event: js_sys::Function,
    ) -> Result<(), JsValue> {
        let mut failed = None;
        let mut observer = |event| observe_init_progress(Some(&on_event), event, &mut failed);
        let prewarmed = self
            .renderer
            .prewarm_all_observed(&mut observer)
            .await
            .map_err(js_err);
        if let Some(error) = failed {
            return Err(error);
        }
        prewarmed?;
        self.request_host_redraw();
        Ok(())
    }

    /// Prepare the same resident Cartesian groups as prewarm_all_with_progress,
    /// without a progress callback. The host must prevent other calls on this
    /// kernel until the Promise settles; wasm-bindgen does not queue them.
    pub async fn prewarm_all(&mut self) -> Result<(), JsValue> {
        self.renderer.prewarm_all().await.map_err(js_err)?;
        self.request_host_redraw();
        Ok(())
    }

    /// Process pending pool maintenance and draw only when visual state is
    /// dirty. A clean rAF tick returns before touching the pool or surface.
    pub fn frame(&mut self) -> Result<(), JsValue> {
        self.renderer
            .sync_external_invalidations()
            .map_err(js_err)?;
        let current_stamp = self.render_stamp()?;
        let renderer_dirty = current_stamp.needs_draw_since(self.last_presented_stamp.as_ref());
        let raster_dirty = current_stamp.needs_raster_since(self.last_presented_stamp.as_ref());
        // The annotation layer has its own raster revision: an item edit
        // or a changed annotation selection redraws the two annotation
        // textures only; `refresh_axis` redoes them with the axis raster.
        let annotation_dirty = current_stamp
            .needs_annotation_raster_since(self.last_presented_stamp.as_ref())
            || self.annotation_selection_dirty;
        // A pipeline a left-out series waited for has compiled: draw again,
        // so the chart fills in without a host call (#299).
        let decision = frame_decision(
            renderer_dirty,
            raster_dirty,
            self.view_dirty,
            self.redraw_pending || self.renderer.compile_redraw_due(),
            self.renderer.has_pending_maintenance(),
        );
        let refresh_raster = match decision {
            FrameDecision::Clean => return Ok(()),
            FrameDecision::MaintenanceOnly => {
                let _ = self
                    .renderer
                    .process_pending_maintenance()
                    .map_err(js_err)?;
                return Ok(());
            }
            FrameDecision::Draw { refresh_raster } => refresh_raster,
        };

        let _ = self
            .renderer
            .process_pending_maintenance()
            .map_err(js_err)?;
        let current_stamp = self.render_stamp()?;

        // Browser resize is preview zoom, not a document mutation. The
        // stored chart remains the export SSoT; the live canvas renders a
        // scaled/letterboxed display chart derived from it.
        let (display_config, panel_rect, _) = self.display_config()?;
        let display_chart = Chart::new(display_config.into_owned());
        // `draw_registered` redraws the text and axis raster itself when the
        // chart, its selection or its display placement changed (#265).
        if (annotation_dirty && !refresh_raster)
            || self.view.annotation_selected() != self.selected_annotation
        {
            self.renderer
                .refresh_annotations(
                    &mut self.view,
                    &display_chart,
                    panel_rect,
                    self.selected_annotation,
                )
                .map_err(js_err)?;
        }

        // The renderer draws the chart at the display placement set by
        // `set_chart_display`, skipping a series whose source is not
        // resident (#160).
        match self.monotonic {
            Some(chart) => self.renderer.draw_monotonic(
                self.clear_color,
                &mut [renderer::MonotonicChartDrawItem {
                    chart,
                    view: &mut self.view,
                }],
            ),
            None => self.renderer.draw_registered(
                self.clear_color,
                &mut [RegisteredChartDrawItem {
                    chart_id: self.chart_id,
                    view: &mut self.view,
                }],
            ),
        }
        .map_err(js_err)?;

        // Only a successfully submitted/presented draw advances the
        // renderer-issued onscreen stamp. Any earlier failure leaves the
        // frame dirty so the next persistent rAF retries it.
        consume_successful_frame(&mut self.view_dirty, &mut self.redraw_pending);
        self.annotation_selection_dirty = false;
        self.last_presented_stamp = Some(current_stamp);
        Ok(())
    }

    /// Set the WebGPU surface clear color used behind the chart panel.
    /// Components are linear 0..1 RGBA floats. This is host/app state:
    /// it does not change the chart Config JSON.
    pub fn set_clear_color(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.clear_color = Color::from_rgba(
            r.clamp(0.0, 1.0),
            g.clamp(0.0, 1.0),
            b.clamp(0.0, 1.0),
            a.clamp(0.0, 1.0),
        );
        self.request_host_redraw();
    }

    /// Resize the swap chain viewport. The chart Config keeps its
    /// document/export `chart_area`; live rendering scales that logical
    /// document into this surface.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), JsValue> {
        let (w, h) = (width.max(1), height.max(1));
        // A monotonic chart resizes through its resize call (#271): with no
        // sources it is refused when a group's mapping would move
        // (`resize_monotonic` takes them).
        if self.monotonic.is_some() {
            return self.resize_monotonic(w, h, js_sys::Array::new(), JsValue::UNDEFINED);
        }
        self.renderer
            .set_chart_display(self.chart_id, Some((w, h)))
            .map_err(js_err)?;
        self.renderer.resize(w, h).map_err(js_err)?;
        self.surface_size = (w, h);
        self.view_dirty = true;
        Ok(())
    }
}

/// The id prefixes of the columns the legacy document calls register: a
/// column the replaced series read under one of them belongs to the
/// replaced document (#311).
const DOCUMENT_ID_PREFIXES: [&str; 3] = ["categorical.", "boxplot.", "radar."];

/// The borrowed adapters one document upload reads through (#311): none for
/// a plain column, which is its own source, the nine record fields of a box
/// plot series, or its outlier values and offsets.
enum DocumentAdapters<'a> {
    Column,
    Record([renderer::BoxColumn<'a>; 9]),
    Outliers([renderer::BoxColumn<'a>; 2]),
}

/// `error` with the document replace stage it failed at (#311): the message
/// names the family and the stage, and the `Error` carries `stage`; the
/// `code` and byte fields of a registration error stay on it.
pub(super) fn document_stage_error(family: &str, stage: &str, error: JsValue) -> JsValue {
    use wasm_bindgen::JsCast;
    let cause = match error.dyn_ref::<js_sys::Error>() {
        Some(error) => String::from(error.message()),
        None => error
            .as_string()
            .unwrap_or_else(|| "an error that is not a string".to_owned()),
    };
    let message = format!("{family} document {stage} failed: {cause}");
    let object = match error.dyn_into::<js_sys::Error>() {
        Ok(object) => {
            object.set_message(&message);
            object
        }
        Err(_) => js_sys::Error::new(&message),
    };
    match js_sys::Reflect::set(
        object.as_ref(),
        &JsValue::from_str("stage"),
        &JsValue::from_str(stage),
    ) {
        Ok(true) => object.into(),
        // The message names the stage either way.
        Ok(false) | Err(_) => JsValue::from_str(&message),
    }
}

impl FiggyChart {
    /// Whether `id` names anything the renderer holds: a column (resident or
    /// streamed), a record, a ragged column or a triangle index column.
    fn document_id_in_use(&self, id: &str) -> bool {
        let pool = self.renderer.pool();
        self.renderer.logical_column(id).is_some()
            || pool.record(id).is_some()
            || pool.ragged(id).is_some()
            || pool.triangles(id).is_some()
    }

    /// Why a document cannot upload `values` under `id`, or `None` (#311):
    /// the id is free, or holds what the upload replaces — a resident
    /// column for a column, a record for a record, a ragged column for a
    /// ragged column.
    fn document_id_conflict(
        &self,
        id: &str,
        values: &renderer::UploadValues<'_>,
    ) -> Option<String> {
        use renderer::{LogicalColumn, UploadValues};
        let pool = self.renderer.pool();
        let held = if let Some(LogicalColumn::Streamed(_)) = self.renderer.logical_column(id) {
            "a streamed column"
        } else if pool.record(id).is_some() {
            "a record"
        } else if pool.ragged(id).is_some() {
            "a ragged column"
        } else if pool.triangles(id).is_some() {
            "a triangle index column"
        } else if pool.slot(id).is_some() {
            "a column"
        } else {
            return None;
        };
        let wanted = match values {
            UploadValues::BoxRecord(_) => "a record",
            UploadValues::BoxOutliers(_) => "a ragged column",
            _ => "a column",
        };
        (held != wanted).then(|| {
            format!(
                "the id {id} holds {held}, and the document uploads {wanted} under it; \
                 remove it before opening the document"
            )
        })
    }

    /// Replace the chart with a converted legacy document atomically (#311),
    /// in one renderer transaction
    /// (`Renderer::begin_replace_chart_sources`): validate the candidate
    /// (styles and ids), then upload its columns, records and ragged columns
    /// under the documented ids, fit (`fit` receives the candidate pool, the
    /// candidate config and series), remove the previous document's columns
    /// that the new one does not reuse, and swap the config and series, all
    /// in one commit. Every failure leaves the chart, its columns and its
    /// selection as they were; the error names the stage.
    fn replace_document<'a>(
        &mut self,
        family: &'static str,
        config: renderer::Config,
        series: Vec<SeriesConfig>,
        uploads: &[renderer::SeriesUpload<'a>],
        fit: impl FnOnce(
            &renderer::ColumnPool,
            renderer::Config,
            &[SeriesConfig],
        ) -> Result<renderer::Config, JsValue>,
    ) -> Result<(), JsValue> {
        use renderer::{BatchSource, ColumnSource, UploadValues};
        use wasm_bindgen::JsCast;
        let validate = |error: JsValue| document_stage_error(family, "validate", error);

        // ---- validate: the chart, the styles, then the ids ----
        // A document opens on the general chart; a monotonic document shown
        // instead ends with the commit (#271), so it must still be there.
        if let Some(id) = self.monotonic {
            self.renderer
                .monotonic_config(id)
                .map_err(|e| validate(js_err(e)))?;
        }
        self.renderer
            .preflight_chart_styles(&series)
            .map_err(|e| validate(js_err(e)))?;
        for upload in uploads {
            if let Some(conflict) = self.document_id_conflict(&upload.column, &upload.values) {
                return Err(validate(js_err(conflict)));
            }
        }
        // The previous document's columns the new one does not reuse: every
        // id under a document prefix that the replaced series of the general
        // chart read.
        let mut uploaded = std::collections::HashSet::new();
        uploaded
            .try_reserve(uploads.len())
            .map_err(|e| validate(js_err(e)))?;
        uploaded.extend(uploads.iter().map(|upload| upload.column.as_str()));
        let mut stale: Vec<String> = Vec::new();
        for item in self
            .renderer
            .chart_series(self.chart_id)
            .map_err(|e| validate(js_err(e)))?
        {
            let outliers = item.render_type.box_plot().and_then(|b| b.outliers);
            for id in [
                Some(item.x_column.as_str()),
                Some(item.y_column.as_str()),
                outliers,
            ]
            .into_iter()
            .flatten()
            {
                let document_column = DOCUMENT_ID_PREFIXES
                    .iter()
                    .any(|prefix| id.starts_with(prefix));
                if document_column && !uploaded.contains(id) && self.document_id_in_use(id) {
                    stale.try_reserve(1).map_err(|e| validate(js_err(e)))?;
                    stale.push(id.to_owned());
                }
            }
        }
        drop(uploaded);
        stale.sort_unstable();
        stale.dedup();
        let mut removals: Vec<&str> = Vec::new();
        removals
            .try_reserve_exact(stale.len())
            .map_err(|e| validate(js_err(e)))?;
        removals.extend(stale.iter().map(String::as_str));

        // The uploads, borrowed from the document through its adapters.
        let mut adapters = Vec::new();
        adapters
            .try_reserve_exact(uploads.len())
            .map_err(|e| validate(js_err(e)))?;
        for upload in uploads {
            adapters.push(match &upload.values {
                UploadValues::BoxRecord(_) => DocumentAdapters::Record(
                    upload
                        .values
                        .box_fields()
                        .ok_or_else(|| validate(js_err("a box plot record upload has fields")))?,
                ),
                UploadValues::BoxOutliers(_) => {
                    let (values, offsets) = upload
                        .values
                        .box_outliers()
                        .ok_or_else(|| validate(js_err("a box plot outlier upload has values")))?;
                    DocumentAdapters::Outliers([values, offsets])
                }
                _ => DocumentAdapters::Column,
            });
        }
        // Each field names itself (#160): the record is paired by name, never
        // by the position of the schema's entries.
        let mut fields: Vec<[(&str, &dyn ColumnSource); 9]> = Vec::new();
        fields
            .try_reserve_exact(
                adapters
                    .iter()
                    .filter(|adapter| matches!(adapter, DocumentAdapters::Record(_)))
                    .count(),
            )
            .map_err(|e| validate(js_err(e)))?;
        for adapter in &adapters {
            if let DocumentAdapters::Record(columns) = adapter {
                fields.push(std::array::from_fn(|k| {
                    (
                        renderer::BoxField::ALL[k].name(),
                        &columns[k] as &dyn ColumnSource,
                    )
                }));
            }
        }
        let mut batch: Vec<(&str, BatchSource<'_>)> = Vec::new();
        batch
            .try_reserve_exact(uploads.len())
            .map_err(|e| validate(js_err(e)))?;
        let mut records = fields.iter();
        for (upload, adapter) in uploads.iter().zip(&adapters) {
            let source = match adapter {
                DocumentAdapters::Column => BatchSource::Column(&upload.values),
                DocumentAdapters::Record(_) => BatchSource::Record(renderer::RecordSource {
                    schema: renderer::BOX_PLOT_RECORD,
                    fields: records
                        .next()
                        .ok_or_else(|| validate(js_err("a box plot record has its fields")))?,
                }),
                DocumentAdapters::Outliers([values, offsets]) => {
                    BatchSource::Ragged(renderer::RaggedSource { values, offsets })
                }
            };
            batch.push((upload.column.as_str(), source));
        }

        // As `set_chart_state` does: the shader set the document needs
        // (polar coordinates for a radar) is admitted before it is installed,
        // so its pipelines compile from the start (#299). The fit changes
        // only ranges and ticks, never what decides the set.
        self.renderer.require_shaders_for(&config);

        // ---- upload, fit and the plan, in one transaction ----
        // The fit's own error, and whether it ran: an error after it is the
        // commit stage's (the series checks, the chart plan, the picker).
        let mut fit_error = None;
        let mut fitted = false;
        let count = series.len();
        let guard =
            self.renderer
                .begin_replace_chart_sources(self.chart_id, &batch, &removals, |pool| {
                    match fit(pool, config, &series) {
                        Ok(config) => {
                            fitted = true;
                            Ok((config, series))
                        }
                        Err(error) => {
                            fit_error = Some(error);
                            // Never reported: the stage error below carries the
                            // fit's own.
                            Err(renderer::FiggyError::Internal {
                                invariant: "the document fit failed",
                            })
                        }
                    }
                });
        let guard = match guard {
            Ok(guard) => guard,
            Err(error) => {
                return Err(match fit_error {
                    Some(error) => document_stage_error(family, "fit", error),
                    None if fitted => document_stage_error(family, "commit", js_err(error)),
                    None => {
                        document_stage_error(family, "upload", column_registration_error(error))
                    }
                });
            }
        };

        // ---- commit: columns, removals, config and series at once ----
        guard.commit();
        // The document wrote its own legend.
        self.legend_auto_managed = false;
        self.color_seq = count.max(self.color_seq);
        // Nothing below fails but for a broken invariant: the monotonic
        // chart was checked before the transaction and only this call
        // removes it, and the hit map reads the chart the commit just
        // wrote. Such a failure is reported as an internal error, not as a
        // stage: the new document is shown by then.
        let internal = |what: &str, error: JsValue| {
            js_err(format!(
                "internal error after the {family} document was committed: {what}: {}",
                error
                    .dyn_ref::<js_sys::Error>()
                    .map(|error| String::from(error.message()))
                    .or_else(|| error.as_string())
                    .unwrap_or_default()
            ))
        };
        // A monotonic document shown instead of the general chart ends: the
        // document replaces what the chart shows (#271).
        if let Some(id) = self.monotonic.take() {
            self.renderer
                .monotonic_chart(id)
                .and_then(|chart| chart.remove())
                .map_err(|error| internal("ending the monotonic chart", js_err(error)))?;
            self.set_selected_annotation(None);
            self.view_dirty = true;
        }
        self.sync_hitmap()
            .map_err(|error| internal("rebuilding the hit map", error))
    }
}
