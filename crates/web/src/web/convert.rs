//! Error and value conversions shared by the JS API: `JsValue` errors,
//! safe-integer checks and the `ColorCycle` preset mirror.

use super::*;

/// An error the JS API throws. A rule refusal (#215) names its rule, so
/// the exception is an `Error` whose `message` is the rule's sentence and
/// whose `code` is the rule's variant name (`RuleKind::code`), which a host
/// branches on without parsing the text; every other error throws its
/// message as a string, as before.
pub(crate) trait JsErrorSource: std::fmt::Display {
    /// The rule this error reports, when it is a rule refusal.
    fn rule(&self) -> Option<&renderer::RuleKind> {
        None
    }
}

impl JsErrorSource for renderer::FiggyError {
    fn rule(&self) -> Option<&renderer::RuleKind> {
        match self {
            renderer::FiggyError::InvalidConfig { kind, .. }
            | renderer::FiggyError::InvalidSeriesConfig { kind, .. } => Some(kind),
            _ => None,
        }
    }
}

impl JsErrorSource for renderer::ConfigError {
    fn rule(&self) -> Option<&renderer::RuleKind> {
        Some(&self.kind)
    }
}

impl JsErrorSource for renderer::RuleKind {
    fn rule(&self) -> Option<&renderer::RuleKind> {
        Some(self)
    }
}

impl JsErrorSource for renderer::radial::RadialError {
    fn rule(&self) -> Option<&renderer::RuleKind> {
        match self {
            renderer::radial::RadialError::Invalid(kind) => Some(kind),
            _ => None,
        }
    }
}

impl JsErrorSource for String {}
impl JsErrorSource for &str {}
impl JsErrorSource for serde_json::Error {}
impl JsErrorSource for std::collections::TryReserveError {}
impl JsErrorSource for std::num::TryFromIntError {}
impl JsErrorSource for renderer::GpuPickError {}
impl JsErrorSource for renderer::gpu_extent::GpuExtentError {}
impl JsErrorSource for wgpu::BufferAsyncError {}
impl JsErrorSource for wgpu::MapRangeError {}
impl JsErrorSource for futures_channel::oneshot::Canceled {}
impl JsErrorSource for wgpu::RequestDeviceError {}
impl JsErrorSource for wgpu::RequestAdapterError {}
impl JsErrorSource for wgpu::CreateSurfaceError {}

impl JsErrorSource for renderer::theme::ThemeError {}

/// A refusal as a JS `Error` with a `code` (#295), built as
/// [`column_registration_error`] builds a registration error: `message` is
/// the sentence, `code` names the refusal, and `fields` ride along. A field
/// that cannot be attached still reports the refusal, as a message that
/// names the missing field.
pub(crate) fn coded_error<'a>(
    message: &str,
    code: &str,
    fields: impl IntoIterator<Item = (&'a str, JsValue)>,
) -> JsValue {
    let js_error = js_sys::Error::new(message);
    let object: &JsValue = js_error.as_ref();
    let attached = std::iter::once(("code", JsValue::from_str(code)))
        .chain(fields)
        .try_for_each(|(key, value)| {
            match js_sys::Reflect::set(object, &JsValue::from_str(key), &value) {
                Ok(true) => Ok(()),
                Ok(false) | Err(_) => Err(js_err(format!(
                    "{message} (the `{key}` field could not be attached to the error object)"
                ))),
            }
        });
    match attached {
        Ok(()) => js_error.into(),
        Err(plain) => plain,
    }
}

/// `names` as a JS array of strings.
pub(crate) fn js_names(names: &[&str]) -> js_sys::Array {
    names
        .iter()
        .map(|name| JsValue::from_str(name))
        .collect::<js_sys::Array>()
}

/// A theme or subplot refusal (#295) as a coded JS `Error`
/// ([`coded_error`]):
///
/// | `code` | Refusal | Fields |
/// |---|---|---|
/// | `theme_unknown_name` | `ThemeError::UnknownName` | `names` |
/// | `theme_wrong_kind` | `ThemeError::WrongKind` | |
/// | `theme_draw_style` | `ThemeError::UnsupportedDrawStyle` | `series_id` |
/// | `subplot_index` | `ThemeError::SubplotIndex` | `index`, `count` |
pub(crate) fn theme_error(error: renderer::theme::ThemeError) -> JsValue {
    use renderer::theme::ThemeError;
    match &error {
        ThemeError::UnknownName { names, .. } => coded_error(
            &error.to_string(),
            "theme_unknown_name",
            [("names", js_names(names).into())],
        ),
        ThemeError::WrongKind { .. } => coded_error(&error.to_string(), "theme_wrong_kind", []),
        ThemeError::UnsupportedDrawStyle { series_id } => coded_error(
            &error.to_string(),
            "theme_draw_style",
            [("series_id", JsValue::from_str(series_id))],
        ),
        ThemeError::SubplotIndex(index) => subplot_index_error(*index),
    }
}

/// An index past the chart's subplots: `code` `"subplot_index"`, with the
/// `index` asked for and the subplot `count`.
pub(crate) fn subplot_index_error(error: renderer::theme::SubplotIndexError) -> JsValue {
    coded_error(
        &error.to_string(),
        "subplot_index",
        [
            ("index", JsValue::from(error.index as f64)),
            ("count", JsValue::from(error.count as f64)),
        ],
    )
}

/// `index` checked against `count` subplots, as a coded error.
pub(crate) fn check_subplot(index: usize, count: usize) -> Result<(), JsValue> {
    renderer::theme::SubplotIndexError::check(index, count).map_err(subplot_index_error)
}

/// `theme` as a plain JS object: its fields, colours as `{r, g, b, a}`.
pub(crate) fn theme_to_js(theme: &renderer::theme::Theme) -> Result<JsValue, JsValue> {
    let json = serde_json::to_string(theme).map_err(js_err)?;
    js_sys::JSON::parse(&json)
}

/// A theme object a host edited (`FiggyChart::theme`), read back; an object
/// that is not a theme throws an `Error` with `code` `"theme_invalid"`.
pub(crate) fn theme_from_js(value: &JsValue) -> Result<renderer::theme::Theme, JsValue> {
    let invalid = |reason: String| {
        coded_error(
            &format!("not a theme object: {reason}; pass an object `theme(name)` handed out"),
            "theme_invalid",
            [],
        )
    };
    let json = js_sys::JSON::stringify(value)
        .map_err(|_| invalid("it does not serialize to JSON".into()))?;
    let json = json
        .as_string()
        .ok_or_else(|| invalid("it does not serialize to a JSON string".into()))?;
    serde_json::from_str(&json).map_err(|error| invalid(error.to_string()))
}

pub(crate) fn js_err(e: impl JsErrorSource) -> JsValue {
    let message = e.to_string();
    let Some(kind) = e.rule() else {
        return JsValue::from_str(&message);
    };
    let error = js_sys::Error::new(&message);
    // An `Error` that cannot take the code still throws the sentence.
    match js_sys::Reflect::set(
        error.as_ref(),
        &JsValue::from_str("code"),
        &JsValue::from_str(kind.code()),
    ) {
        Ok(true) => error.into(),
        Ok(false) | Err(_) => JsValue::from_str(&message),
    }
}

pub(super) fn column_registration_error(error: renderer::FiggyError) -> JsValue {
    let (code, requested, limit, largest_free, total_free) = match &error {
        renderer::FiggyError::Pool(AllocError::OutOfSpace {
            requested,
            largest_free,
            total_free,
        }) => (
            "pool_space",
            Some(*requested),
            None,
            Some(*largest_free),
            Some(*total_free),
        ),
        renderer::FiggyError::Pool(AllocError::BudgetExceeded {
            requested, limit, ..
        }) => (
            "budget_exceeded",
            Some(*requested),
            Some(*limit),
            None,
            None,
        ),
        renderer::FiggyError::Pool(AllocError::ResourceLimit {
            requested, limit, ..
        })
        | renderer::FiggyError::GpuResourceLimit {
            requested, limit, ..
        } => ("device_limit", Some(*requested), Some(*limit), None, None),
        renderer::FiggyError::Pool(AllocError::AllocationFailed { .. })
        | renderer::FiggyError::GpuResourceAllocationFailed { .. } => {
            ("allocation_failed", None, None, None, None)
        }
        _ => ("registration_failed", None, None, None, None),
    };
    let js_error = js_sys::Error::new(&error.to_string());
    let object: &JsValue = js_error.as_ref();
    // A field that cannot be attached still reports the registration
    // error, as a message that names the missing field.
    let set = |key: &str, value: JsValue| match js_sys::Reflect::set(
        object,
        &JsValue::from_str(key),
        &value,
    ) {
        Ok(true) => Ok(()),
        Ok(false) | Err(_) => Err(js_err(format!(
            "{error} (the `{key}` field could not be attached to the error object)"
        ))),
    };
    if let Err(plain) = set("code", JsValue::from_str(code)) {
        return plain;
    }
    for (key, value) in [
        ("requestedBytes", requested),
        ("limitBytes", limit),
        ("largestFreeBytes", largest_free),
        ("totalFreeBytes", total_free),
    ] {
        if let Some(value) = value {
            let js_value = if value <= 9_007_199_254_740_991 {
                JsValue::from_f64(value as f64)
            } else {
                JsValue::from_str(&value.to_string())
            };
            if let Err(plain) = set(key, js_value) {
                return plain;
            }
        }
    }
    js_error.into()
}

pub(super) fn js_safe_u64(value: f64, name: &str) -> Result<u64, JsValue> {
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > 9_007_199_254_740_991.0
    {
        return Err(js_err(format!(
            "{name} must be a non-negative safe integer"
        )));
    }
    Ok(value as u64)
}

/// Series color rotations (mirror of `model::ColorCycle`).
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum ColorCycle {
    Classic,
    Vivid,
    Balanced,
    ColorblindSafe,
    Monochrome,
}

impl From<ColorCycle> for renderer::ColorCycle {
    fn from(c: ColorCycle) -> Self {
        match c {
            ColorCycle::Classic => Self::Classic,
            ColorCycle::Vivid => Self::Vivid,
            ColorCycle::Balanced => Self::Balanced,
            ColorCycle::ColorblindSafe => Self::ColorblindSafe,
            ColorCycle::Monochrome => Self::Monochrome,
        }
    }
}

/// CSS color strings (`"rgb(r g b / a)"`) for a cycle — lets the host UI
/// render swatches / legends with exactly the chart's palette.
#[wasm_bindgen]
pub fn color_cycle_css(cycle: ColorCycle) -> Vec<String> {
    renderer::ColorCycle::from(cycle)
        .colors()
        .iter()
        .map(|c| {
            format!(
                "rgb({} {} {} / {})",
                (c.r * 255.0).round() as u8,
                (c.g * 255.0).round() as u8,
                (c.b * 255.0).round() as u8,
                c.a,
            )
        })
        .collect()
}
