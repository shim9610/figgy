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

/// The theme `name` of the chart's kind (#291), or an error naming the
/// kind's themes. A chart that draws no series yet has no kind.
pub(crate) fn theme_named(
    config: &renderer::Config,
    series: &[renderer::SeriesConfig],
    name: &str,
) -> Result<renderer::theme::Theme, JsValue> {
    use renderer::theme::{ChartKind, Theme};
    let kind = ChartKind::of(config, series, renderer::LayerRef::Base).ok_or_else(|| {
        js_err("the chart draws no series yet, so it has no kind to take a theme of")
    })?;
    Theme::named(kind, name).ok_or_else(|| {
        js_err(format!(
            "no {kind} theme {name:?}: use {}",
            Theme::names(kind).join(", ")
        ))
    })
}

/// The radial theme `name` (#291), or an error naming the radial themes.
pub(crate) fn radial_theme_named(name: &str) -> Result<renderer::theme::RadialTheme, JsValue> {
    use renderer::theme::RadialTheme;
    RadialTheme::named(name).ok_or_else(|| {
        js_err(format!(
            "no radial theme {name:?}: use {}",
            RadialTheme::names().join(", ")
        ))
    })
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
