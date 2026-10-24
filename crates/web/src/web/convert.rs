//! Error and value conversions shared by the JS API: `JsValue` errors,
//! safe-integer checks and the `ColorCycle` preset mirror.

use super::*;

pub(super) fn js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
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
    let set = |key: &str, value: JsValue| {
        let _ = js_sys::Reflect::set(object, &JsValue::from_str(key), &value);
    };
    set("code", JsValue::from_str(code));
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
            set(key, js_value);
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
