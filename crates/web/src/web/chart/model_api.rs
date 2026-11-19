//! Model calls that need no chart: config defaults, validation and
//! normalization, legacy document conversion and draw style specs.

use super::*;

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
/// without a chart (`Config::from_categorical_with_report_in`): `{config,
/// series, uploads, columns, notes}`. `uploads` describes each column to
/// register (`{column, role, len}`, without the values); `columns` names
/// the document series of each (`{series_id, role, column}`); `notes` lists
/// every field the document set that the chart does not carry
/// (`{path, kind: "dropped" | "changed", reason}`).
///
/// `options` is an optional JSON object `{"chart_area"?: [x, y, width,
/// height]}` (#334): the chart area in whole logical pixels, as the chart's
/// `chart_area` (`get_config().chart_area`). The conversion places the
/// config on it and fits an explicit `decorations.data_area` to it, so the
/// result is the report `set_categorical_document` returns on a chart of
/// that chart area: the same config, series and notes. Without it, the
/// config keeps the placeholder chart area (1000 x 800) and the data area
/// is fitted to that. A data area the layout cannot hold is noted as
/// dropped with the reason. Options that are not such an object, or that
/// name another field, throw an `Error` with `code` `"invalid_options"`.
/// Throws the JSON error, or the document rule (an `Error` with a `code`),
/// as `set_categorical_document` does.
///
/// JS: `const { config, series, notes } = JSON.parse(convert_categorical_document(text, JSON.stringify({ chart_area: chart_area_of_the_chart })));`
#[wasm_bindgen]
pub fn convert_categorical_document(
    json: &str,
    options: Option<String>,
) -> Result<String, JsValue> {
    crate::convert_document_json::<renderer::CategoricalChart>(json, options.as_deref())
        .map_err(document_error)
}

/// A legacy box plot document (`BoxPlotChart` JSON) converted without a
/// chart (`Config::from_boxplot_with_report_in`), in the shape and with the
/// `options` `convert_categorical_document` takes. A record upload is
/// `{column, role: "box_record", rows, fields}` and an outlier upload
/// `{column, role: "box_outliers", len, groups}`.
///
/// JS: `const converted = JSON.parse(convert_boxplot_document(text));`
#[wasm_bindgen]
pub fn convert_boxplot_document(json: &str, options: Option<String>) -> Result<String, JsValue> {
    crate::convert_document_json::<renderer::BoxPlotChart>(json, options.as_deref())
        .map_err(document_error)
}

/// A legacy radar document (`RadarChart` JSON) converted without a chart
/// (`Config::from_radar_with_report_in`), in the shape and with the
/// `options` `convert_categorical_document` takes.
///
/// JS: `const converted = JSON.parse(convert_radar_document(text));`
#[wasm_bindgen]
pub fn convert_radar_document(json: &str, options: Option<String>) -> Result<String, JsValue> {
    crate::convert_document_json::<renderer::RadarChart>(json, options.as_deref())
        .map_err(document_error)
}

/// Every valid `draw_style` mode tag, as a JSON string array.
///
/// Returns JSON text (`FiggyJson`), not an object: parse it with `JSON.parse`.
#[wasm_bindgen(unchecked_return_type = "FiggyJson")]
pub fn draw_style_modes() -> Result<String, JsValue> {
    serde_json::to_string(renderer::config::DrawStyle::mode_tags()).map_err(js_err)
}
