//! `FiggyChart` creation options, init progress, and the readiness
//! types of its background pipeline compiles.

use super::*;

pub(in crate::web) const POOL_CAPACITY: u64 = 16 * 1024 * 1024;

/// Report one init stage to the host's `on_event` as `{ scope, stage,
/// phase }`. An exception the callback throws (or a failed property set on
/// the payload) is returned, so the call that reports progress rejects
/// with it.
pub(in crate::web) fn emit_init_progress(
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
pub(super) fn counts_object(counts: renderer::PipelineCounts) -> Result<JsValue, JsValue> {
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
    pub(super) readiness: renderer::PipelineReadiness,
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
pub(super) fn failure_object(failure: &renderer::PipelineFailure) -> Result<JsValue, JsValue> {
    let object = js_sys::Object::new();
    js_sys::Reflect::set(&object, &"label".into(), &failure.label.as_str().into())?;
    js_sys::Reflect::set(&object, &"kind".into(), &failure_kind(failure.kind).into())?;
    js_sys::Reflect::set(&object, &"message".into(), &failure.message.as_str().into())?;
    Ok(object.into())
}

/// An `Error` for a failed compile, with its `label`, `kind` and `message`
/// as properties (#299).
pub(super) fn failure_error(failure: &renderer::PipelineFailure) -> JsValue {
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
pub(super) fn observe_init_progress(
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
pub(in crate::web) fn shader_set_from_name(name: &JsValue) -> Result<renderer::ShaderSet, JsValue> {
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
pub(in crate::web) fn shader_set_name(set: renderer::ShaderSet) -> &'static str {
    match set {
        renderer::ShaderSet::Basic => "basic",
        renderer::ShaderSet::Full => "full",
    }
}

/// `FiggyChart.create_with_options`' options, read and checked before
/// anything is created: `shaders` and `on_progress`, both optional. An
/// unknown key or a value of the wrong kind is refused.
pub(in crate::web) fn create_options(
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
