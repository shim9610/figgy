//! figgy for the web.
//!
//! The public browser surface is the `figgy-chart.js` Custom Element
//! (`<figgy-chart>`). It owns the canvas, async initialization, rAF loop,
//! resize/DPR handling, pointer wiring, busy gate, and CustomEvent output.
//! This Rust module exposes the raw `FiggyChart` wasm class that the Custom
//! Element uses as its low-level kernel; advanced hosts may call it directly
//! when they intentionally want to own those browser responsibilities.
//!
//! Lifecycle model: **one instance, register/unregister**. Columns and series
//! are managed by id. Column registration and explicit registration updates
//! are separate operations; `set_series` only selects registered columns for
//! drawing. `remove_*` unregisters (and drops dependents), and pool
//! defragmentation runs automatically after removals.
//!
//! The whole implementation is gated to `wasm32`: on native targets this
//! crate compiles to an empty library so `cargo test --workspace` stays
//! green. Build the artifact with:
//!
//! ```bash
//! npx wasm-pack build crates/web --release --target web
//! ```
//!
//! See `crates/renderer/WASM.md` for the I/O architecture this implements.

#[cfg(any(target_arch = "wasm32", test))]
mod borrowed_column;

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrameDecision {
    Clean,
    MaintenanceOnly,
    Draw { refresh_raster: bool },
}

/// The JSON of [`renderer::text::RichText::from_tex`]: the rich text the
/// `rich_text_from_tex` binding returns, ready for any `RichText` field of
/// `set_config` or `set_series`. The error names the TeX byte range.
#[cfg(any(target_arch = "wasm32", test))]
fn rich_text_json_from_tex(
    tex: &str,
    font_size: f32,
    font: &str,
    color: [f32; 4],
) -> Result<String, String> {
    let [r, g, b, a] = color;
    let text =
        renderer::text::RichText::from_tex(tex, renderer::Color::new(r, g, b, a), font_size, font)
            .map_err(|e| e.to_string())?;
    serde_json::to_string(&text).map_err(|e| e.to_string())
}

/// The JSON of [`renderer::layout::auto_layout`] (#44) on the config JSON
/// `json`, measured with the renderer's text measure for the config's draw
/// style: what the `autoLayout` binding returns, for `set_config`. Reads
/// only the JSON and the measure. The error is the JSON parse or write
/// error.
#[cfg(any(target_arch = "wasm32", test))]
fn auto_layout_json(json: &str) -> Result<String, String> {
    let config: renderer::Config = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
    serde_json::to_string(&renderer::layout::auto_layout(&config, &measure))
        .map_err(|e| e.to_string())
}

#[cfg(any(target_arch = "wasm32", test))]
fn consume_successful_frame(view_dirty: &mut bool, redraw_pending: &mut bool) {
    *view_dirty = false;
    *redraw_pending = false;
}

#[cfg(any(target_arch = "wasm32", test))]
fn frame_decision(
    renderer_dirty: bool,
    raster_dirty: bool,
    view_dirty: bool,
    redraw_pending: bool,
    maintenance_pending: bool,
) -> FrameDecision {
    let visual_pending = renderer_dirty || raster_dirty || view_dirty || redraw_pending;
    match (visual_pending, maintenance_pending) {
        (false, false) => FrameDecision::Clean,
        (false, true) => FrameDecision::MaintenanceOnly,
        (true, _) => FrameDecision::Draw {
            refresh_raster: raster_dirty || view_dirty,
        },
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn picked_point_json_string(picked: &renderer::PickedPoint) -> serde_json::Result<String> {
    serde_json::to_string(&serde_json::json!({
        "source_id": picked.source_id.as_ref(),
        "series_id": &picked.series_id,
        "point_index": picked.point_index,
        "distance_px": picked.distance_px,
    }))
}

/// A value label hit as `pick_point` reports it: the point fields with the
/// label's category as `point_index`, and the label's id, which
/// `press_value_label` takes.
#[cfg(any(target_arch = "wasm32", test))]
fn picked_label_json_string(
    source_id: Option<&str>,
    series_id: &str,
    category_index: usize,
    distance_px: f32,
) -> serde_json::Result<String> {
    serde_json::to_string(&serde_json::json!({
        "source_id": source_id,
        "series_id": series_id,
        "point_index": category_index,
        "distance_px": distance_px,
        "label_id": renderer::data_config::value_label_id(series_id, category_index),
    }))
}

#[cfg(any(target_arch = "wasm32", test))]
fn picked_data_json_string(picked: &renderer::PickedData) -> serde_json::Result<String> {
    let value = match picked {
        renderer::PickedData::Point {
            source_id,
            series_id,
            point_index,
            distance_px,
        } => serde_json::json!({
            "kind": "point",
            "source_id": source_id.as_ref(),
            "series_id": series_id,
            "point_index": point_index,
            "distance_px": distance_px,
        }),
        renderer::PickedData::HistogramBin {
            source_id,
            series_id,
            bin_index,
            half,
            distance_px,
        } => {
            let mut json = serde_json::json!({
                "kind": "histogram_bin",
                "source_id": source_id.as_ref(),
                "series_id": series_id,
                "bin_index": bin_index,
                "distance_px": distance_px,
            });
            // A split violin's half, as `PickedDataRef` serializes it.
            if let Some(half) = half {
                json["half"] = serde_json::json!(half);
            }
            json
        }
        renderer::PickedData::MatrixCell {
            source_id,
            series_id,
            x_index,
            y_index,
            distance_px,
        } => serde_json::json!({
            "kind": "matrix_cell",
            "source_id": source_id.as_ref(),
            "series_id": series_id,
            "x_index": x_index,
            "y_index": y_index,
            "distance_px": distance_px,
        }),
        renderer::PickedData::ContourLevel {
            source_id,
            series_id,
            level_index,
            x_index,
            y_index,
            distance_px,
        } => serde_json::json!({
            "kind": "contour_level",
            "source_id": source_id.as_ref(),
            "series_id": series_id,
            "level_index": level_index,
            "x_index": x_index,
            "y_index": y_index,
            "distance_px": distance_px,
        }),
        renderer::PickedData::ValueLabel {
            source_id,
            series_id,
            category_index,
            part,
            distance_px,
        } => {
            let mut value = serde_json::json!({
                "kind": "value_label",
                "source_id": source_id.as_ref(),
                "series_id": series_id,
                "category_index": category_index,
                "label_id": renderer::data_config::value_label_id(series_id, *category_index),
                "distance_px": distance_px,
            });
            // An energy-level label names its list (#203).
            if let Some(part) = part {
                value["part"] = serde_json::to_value(part)?;
            }
            value
        }
        renderer::PickedData::MeshTriangle {
            source_id,
            series_id,
            triangle_index,
            vertex_index,
            distance_px,
        } => serde_json::json!({
            "kind": "mesh_triangle",
            "source_id": source_id.as_ref(),
            "series_id": series_id,
            "triangle_index": triangle_index,
            "vertex_index": vertex_index,
            "distance_px": distance_px,
        }),
        // A Feynman part names its row (#234), for `press_feynman`.
        renderer::PickedData::FeynmanVertex {
            source_id,
            series_id,
            vertex_index,
            distance_px,
        } => serde_json::json!({
            "kind": "feynman_vertex",
            "source_id": source_id.as_ref(),
            "series_id": series_id,
            "index": vertex_index,
            "distance_px": distance_px,
        }),
        renderer::PickedData::FeynmanPropagator {
            source_id,
            series_id,
            propagator_index,
            distance_px,
        } => serde_json::json!({
            "kind": "feynman_propagator",
            "source_id": source_id.as_ref(),
            "series_id": series_id,
            "index": propagator_index,
            "distance_px": distance_px,
        }),
        renderer::PickedData::FeynmanLabel {
            source_id,
            series_id,
            part,
            index,
            distance_px,
        } => serde_json::json!({
            "kind": "feynman_label",
            "source_id": source_id.as_ref(),
            "series_id": series_id,
            "part": serde_json::to_value(part)?,
            "index": index,
            "distance_px": distance_px,
        }),
    };
    serde_json::to_string(&value)
}

#[cfg(target_arch = "wasm32")]
const DEMO_COLUMN_IDS: [&str; 4] = ["demo_x", "demo_sin", "demo_t", "demo_rc"];

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ColumnRegistryAction {
    Register,
    Update,
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_column_registry_action(
    is_registered: bool,
    action: ColumnRegistryAction,
) -> Result<(), &'static str> {
    match (action, is_registered) {
        (ColumnRegistryAction::Register, false) | (ColumnRegistryAction::Update, true) => Ok(()),
        (ColumnRegistryAction::Register, true) => Err("is already registered"),
        (ColumnRegistryAction::Update, false) => Err("is not registered"),
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_column_data_len(len: usize) -> Result<(), &'static str> {
    if len == 0 {
        Err("data must not be empty")
    } else {
        Ok(())
    }
}

/// Validate a batch registration's shape before anything is uploaded.
///
/// The flat-buffer size check is an **error**, not a truncation: the
/// smallest-common-extent rule is about columns disagreeing with
/// each other, whereas a buffer whose size does not match the declared
/// `ids.len() x values_per_column` shape cannot be cut into columns at all.
#[cfg(any(target_arch = "wasm32", test))]
fn validate_column_batch(
    is_registered: impl Fn(&str) -> bool,
    ids: &[String],
    values_per_column: usize,
    data_len: usize,
) -> Result<(), String> {
    if ids.is_empty() {
        return Err("a column batch needs at least one id".to_string());
    }
    validate_column_data_len(values_per_column)
        .map_err(|reason| format!("values_per_column {reason}"))?;
    let expected = ids
        .len()
        .checked_mul(values_per_column)
        .ok_or_else(|| "column batch size overflows".to_string())?;
    if data_len != expected {
        return Err(format!(
            "column batch data has {data_len} values, but {} ids x {values_per_column} \
             values_per_column is {expected}",
            ids.len()
        ));
    }
    // One set rather than a quadratic scan: a matrix batch is thousands of ids,
    // and the allocation is proportional to the id count like everything else on
    // this path.
    let mut seen = std::collections::HashSet::new();
    seen.try_reserve(ids.len())
        .map_err(|_| "column batch id set allocation failed".to_string())?;
    for id in ids {
        validate_column_registry_action(is_registered(id), ColumnRegistryAction::Register)
            .map_err(|reason| format!("column '{id}' {reason}"))?;
        if !seen.insert(id.as_str()) {
            return Err(format!("column '{id}' appears twice in one batch"));
        }
    }
    Ok(())
}

/// Replace the chart title with the label `text` (`$…$` spans are TeX,
/// `renderer::text::rich_segments_from_label`) and grow its band to hold it,
/// as `Chart::with_title` does natively, so a multi-line or math title set
/// from the browser is not clipped at the top of the chart.
#[cfg(any(target_arch = "wasm32", test))]
fn set_chart_title_label(config: &mut renderer::Config, text: &str) {
    config.chart_title.text.segments = renderer::text::rich_segments_from_label(text);
    let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
    renderer::layout::fit_chart_title_margin(config, &measure);
}

#[cfg(any(target_arch = "wasm32", test))]
struct PreparedDemoDeclarations {
    config: renderer::Config,
    series: Vec<renderer::SeriesConfig>,
    color_seq: usize,
}

#[cfg(any(target_arch = "wasm32", test))]
fn prepare_demo_declarations(
    current_config: &renderer::Config,
    current_series: &[renderer::SeriesConfig],
    cycle: renderer::ColorCycle,
    color_seq: usize,
) -> Result<PreparedDemoDeclarations, String> {
    use renderer::line::LineStylePreset;
    use renderer::{DataLineStyleConfig, DataRenderType, SeriesConfig};

    let mut config = current_config.clone();
    let mut series = Vec::new();
    series
        .try_reserve(current_series.len().saturating_add(2))
        .map_err(|error| error.to_string())?;
    series.extend(current_series.iter().cloned());
    let mut next_color_seq = color_seq;

    for (series_id, x_column, y_column, label) in [
        ("sine", "demo_x", "demo_sin", "sin(x)"),
        ("rc", "demo_t", "demo_rc", "RC charge"),
    ] {
        let existing = series.iter().position(|item| item.series_id == series_id);
        let color = match existing {
            // Exhaustive so a new render type has to declare where its colour
            // comes from. A wildcard here would hand a bar or field series the
            // cycle colour while its own configured colour sat unread.
            Some(index) => match &series[index].render_type {
                DataRenderType::Xy(renderer::data_config::XySeries {
                    scatter: None,
                    line: Some(line),
                    ..
                }) => line.line_color,
                DataRenderType::Xy(_) => cycle.color(index),
                DataRenderType::Histogram { bar } => bar.fill_color,
                DataRenderType::CategoryBars { color, .. } => *color,
                DataRenderType::BoxPlot { color, .. } => *color,
                DataRenderType::RangeBars { color, .. } => *color,
                DataRenderType::Radar { color, .. } => *color,
                DataRenderType::Field(renderer::data_config::FieldSeries {
                    contour: Some(contour),
                    ..
                }) => contour.line.line_color,
                // A heatmap's colour is the colourbar's colormap, not a swatch
                // this demo could reuse; an image's colours are its data.
                DataRenderType::TriMesh(renderer::tri_mesh::TriMeshSeries {
                    contour: Some(contour),
                    ..
                }) => contour.line.line_color,
                DataRenderType::Field(renderer::data_config::FieldSeries {
                    contour: None, ..
                })
                | DataRenderType::TriMesh(renderer::tri_mesh::TriMeshSeries {
                    contour: None,
                    ..
                })
                | DataRenderType::RgbImage { .. } => cycle.color(index),
                DataRenderType::Vector { style, .. } => style.color,
                DataRenderType::EnergyLevels(levels) => levels.style.level.color,
                DataRenderType::Feynman(diagram) => diagram.style.color,
                DataRenderType::Streamlines { line, .. }
                | DataRenderType::Parallel { line, .. } => line.line_color,
                DataRenderType::MonotonicLine(series) => series.line.line_color,
                DataRenderType::Ternary { scatter, line, .. } => scatter
                    .as_ref()
                    .map(|s| s.point_color)
                    .or(line.as_ref().map(|l| l.line_color))
                    .unwrap_or_else(|| cycle.color(index)),
            },
            None => cycle.color(next_color_seq),
        };
        if existing.is_none() {
            next_color_seq = next_color_seq
                .checked_add(1)
                .ok_or_else(|| "series color sequence exhausted".to_string())?;
        }
        let rich_label = renderer::text::RichText {
            segments: renderer::text::rich_segments_from_text(label),
            color: config.legend.content.color,
            font_size: config.legend.content.font_size,
            font: config.legend.content.font.clone(),
            align: config.legend.content.align,
        };
        let item = SeriesConfig {
            series_id: series_id.to_string(),
            source_id: None,
            label: Some(rich_label.clone()),
            x_column: x_column.to_string(),
            y_column: y_column.to_string(),
            z_column: None,
            render_type: DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: None,
                line: Some(DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: color,
                    line_width: 2.0,
                    fill: None,
                    fill_color: None,
                    interpolation: renderer::data_config::LineInterpolation::Linear,
                }),
                errors: None,
                stem: None,
            }),
        };
        // The legend entry is the one that stands for this series, not
        // the entry at the series' position.
        let entry =
            renderer::config::legend_entry_of_series(&config.legend.content, &series, series_id);
        let symbol = renderer::config::series_symbol_segments(
            &item,
            config.colorbar.as_ref().map(|bar| &bar.colormap),
        );
        match existing {
            Some(index) => series[index] = item.clone(),
            None => series.push(item.clone()),
        }
        match entry {
            Some(index) => renderer::config::set_legend_entry_label(
                &mut config.legend.content,
                index,
                symbol,
                rich_label.segments,
            ),
            None => renderer::config::append_legend_entry_rich(
                &mut config.legend.content,
                symbol,
                rich_label.segments,
            ),
        }
        config.legend.visible = true;
    }
    config.chart_title.text.segments = renderer::text::rich_segments_from_text("figgy");
    config.bottom_x.title_option.text.segments = renderer::text::rich_segments_from_text("x");
    config.left_y.title_option.text.segments = renderer::text::rich_segments_from_text("y");

    Ok(PreparedDemoDeclarations {
        config,
        series,
        color_seq: next_color_seq,
    })
}

#[cfg(test)]
mod tests {
    use renderer::data_config::{
        ContourConfig, ErrorRef, FieldFillConfig, FillMode, GridLayout, MatrixOrientation,
        MatrixRef, Shading,
    };
    use renderer::{
        Color, DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType,
        DataScatterStyleConfig, GpuFieldExtentMode, GpuSeriesExtentMode, GpuSeriesFitMode,
        ScatterShape, SeriesConfig,
    };

    use super::{
        ColumnRegistryAction, FrameDecision, consume_successful_frame, frame_decision,
        picked_data_json_string, picked_label_json_string, picked_point_json_string,
        prepare_demo_declarations, set_chart_title_label, validate_column_batch,
        validate_column_data_len, validate_column_registry_action,
    };

    fn scatter_style() -> DataScatterStyleConfig {
        DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::Circle,
            point_size: 4.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        }
    }

    fn errorbar_style() -> DataErrorBarStyleConfig {
        DataErrorBarStyleConfig {
            error_bar_color: Color::BLACK,
            error_bar_width: 1.0,
            error_bar_cap_size: 2.0,
            cap_width: 1.0,
            error_bar_style_table: None,
            error_bar_style_index_column: None,
            error_bar_style_overrides: None,
        }
    }

    fn line_style() -> DataLineStyleConfig {
        DataLineStyleConfig {
            line_style: renderer::line::LineStylePreset::Solid,
            line_color: Color::BLACK,
            line_width: 1.0,
            fill: None,
            fill_color: None,
            interpolation: renderer::data_config::LineInterpolation::Linear,
        }
    }

    fn series_config(series_id: &str, render_type: DataRenderType) -> SeriesConfig {
        SeriesConfig {
            series_id: series_id.to_string(),
            source_id: None,
            label: None,
            x_column: "x".to_string(),
            y_column: "y".to_string(),
            z_column: None,
            render_type,
        }
    }

    fn line_series(series_id: &str, x: &str, y: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: series_id.to_string(),
            source_id: None,
            label: None,
            x_column: x.to_string(),
            y_column: y.to_string(),
            z_column: None,
            render_type: DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: None,
                line: Some(line_style()),
                errors: None,
                stem: None,
            }),
        }
    }

    #[test]
    fn frame_decision_covers_every_dirty_source_without_surface_work_when_clean() {
        for bits in 0u8..32 {
            let renderer_dirty = bits & 1 != 0;
            let raster_dirty = bits & 2 != 0;
            let view_dirty = bits & 4 != 0;
            let redraw_pending = bits & 8 != 0;
            let maintenance_pending = bits & 16 != 0;
            let visual_pending = renderer_dirty || raster_dirty || view_dirty || redraw_pending;
            let expected = if !visual_pending {
                if maintenance_pending {
                    FrameDecision::MaintenanceOnly
                } else {
                    FrameDecision::Clean
                }
            } else {
                FrameDecision::Draw {
                    refresh_raster: raster_dirty || view_dirty,
                }
            };

            assert_eq!(
                frame_decision(
                    renderer_dirty,
                    raster_dirty,
                    view_dirty,
                    redraw_pending,
                    maintenance_pending,
                ),
                expected,
                "dirty-state mask {bits:05b}"
            );
        }
    }

    // `set_title` grows the title band like `Chart::with_title`: a single
    // default-size line keeps the default band, and a three-line title gets a
    // band at least as tall as its block, so its first line is not clipped.
    #[test]
    fn set_chart_title_label_grows_the_band_to_hold_a_multi_line_title() {
        use renderer::text_render::{FontPolicy, measure_rich_text};

        let default_band = renderer::default::default_config().chart_title.top_margin;
        let mut config = renderer::default::default_config();
        set_chart_title_label(&mut config, "figgy");
        assert_eq!(config.chart_title.top_margin, default_band);

        set_chart_title_label(&mut config, "first\nsecond\nthird");
        let block = measure_rich_text(&config.chart_title.text, FontPolicy::Standard);
        assert!(block.height() > default_band);
        assert!(config.chart_title.top_margin >= block.height());
        assert!(config.data_area().is_ok());
    }

    // The text setters read their text as a label (#272): `$…$` spans are
    // TeX, the same segments `RichText::from_tex` gives; a `\$` is a dollar
    // sign; text whose TeX does not parse is shown as written. A math title
    // taller than the band grows it as a multi-line one does.
    #[test]
    fn set_chart_title_label_reads_dollar_spans_as_tex() {
        use renderer::text::rich_segments_from_text;
        use renderer::text_render::{FontPolicy, measure_rich_text};

        let mut config = renderer::default::default_config();
        let tex = r"$\displaystyle \sum_{i=1}^{n} \frac{x_i^2}{\sigma^2}$";
        set_chart_title_label(&mut config, tex);
        assert_eq!(
            config.chart_title.text.segments,
            renderer::text::tex::rich_segments_from_tex(tex).unwrap()
        );
        assert!(
            config
                .chart_title
                .text
                .segments
                .iter()
                .any(|s| s.math_node.is_some())
        );
        let block = measure_rich_text(&config.chart_title.text, FontPolicy::Standard);
        assert!(config.chart_title.top_margin >= block.height());

        set_chart_title_label(&mut config, r"Price (\$)");
        assert_eq!(
            config.chart_title.text.segments,
            rich_segments_from_text("Price ($)")
        );
        set_chart_title_label(&mut config, r"$\frac{$");
        assert_eq!(
            config.chart_title.text.segments,
            rich_segments_from_text(r"$\frac{$")
        );
    }

    // `get_config` / `set_config` and `get_series` / `set_series` are
    // `serde_json` over `renderer::Config` and `SeriesConfig`: a segment's
    // `char_set` reaches JavaScript as `greek` plus `math` (written only for
    // `Math`), comes back from it unchanged, and a segment naming both sets
    // is rejected before `set_config` touches any state.
    #[test]
    fn config_and_series_json_carry_the_segment_char_set_both_ways() {
        use renderer::text::{CharSet, RichSegment};

        let mut config = renderer::default::default_config();
        config.chart_title.text.segments = vec![
            RichSegment {
                char_set: CharSet::Greek,
                ..RichSegment::plain('s')
            },
            RichSegment {
                char_set: CharSet::Math,
                ..RichSegment::plain('≤')
            },
            RichSegment::plain('1'),
        ];
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains(r#""text":"s","bold":false,"italic":false,"underline":false,"superscript":false,"subscript":false,"greek":true}"#), "{json}");
        assert!(json.contains(r#""text":"≤","bold":false,"italic":false,"underline":false,"superscript":false,"subscript":false,"greek":false,"math":true}"#), "{json}");
        assert!(json.contains(r#""text":"1","bold":false,"italic":false,"underline":false,"superscript":false,"subscript":false,"greek":false}"#), "{json}");
        let back: renderer::Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back, config);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);

        let both = json.replacen(r#""greek":true}"#, r#""greek":true,"math":true}"#, 1);
        assert_ne!(both, json);
        let err = serde_json::from_str::<renderer::Config>(&both)
            .expect_err("a segment cannot be greek and math at once")
            .to_string();
        assert!(err.contains("both"), "{err}");

        let mut series = vec![series_config(
            "s",
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: Some(scatter_style()),
                line: None,
                errors: None,
                stem: None,
            }),
        )];
        series[0].label = Some(config.chart_title.text.clone());
        let json = serde_json::to_string(&series).unwrap();
        assert!(json.contains(r#""greek":false,"math":true}"#), "{json}");
        let back: Vec<SeriesConfig> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, series);
    }

    // An axis' tick label clearance (#272) is set from JavaScript as its
    // label format is, through `get_config` / `set_config`:
    // `label_style.clearance` is `"Keep"` when written, absent for the
    // default `Auto`, and a document without it reads as `Auto`.
    #[test]
    fn config_json_carries_the_label_clearance_both_ways() {
        use renderer::config::LabelClearanceMode;

        let config = renderer::default::default_config();
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("clearance"), "{json}");
        let back: renderer::Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.left_y.label_style.clearance, LabelClearanceMode::Auto);

        let mut value = serde_json::to_value(&config).unwrap();
        value["left_y"]["label_style"]["clearance"] = "Keep".into();
        let edited: renderer::Config = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            edited.left_y.label_style.clearance,
            LabelClearanceMode::Keep
        );
        assert_eq!(
            edited.bottom_x.label_style.clearance,
            LabelClearanceMode::Auto
        );
        assert_eq!(serde_json::to_value(&edited).unwrap(), value);
        value["left_y"]["label_style"]["clearance"] = "Never".into();
        assert!(serde_json::from_value::<renderer::Config>(value).is_err());
    }

    // An axis `unit` (#273) is absent by default, comes back from
    // `set_config` as `{ "scale", "offset" }` unchanged, and a unit the axis
    // refuses fails validation with its code.
    #[test]
    fn config_json_carries_the_axis_unit_both_ways() {
        use renderer::config::AxisUnit;

        let config = renderer::default::default_config();
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("\"unit\""), "{json}");
        let back: renderer::Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.left_y.unit, None);

        let mut value = serde_json::to_value(&config).unwrap();
        value["left_y"]["unit"] = serde_json::json!({ "scale": 0.5, "offset": -273.15 });
        let edited: renderer::Config = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(edited.left_y.unit, Some(AxisUnit::new(0.5, -273.15)));
        assert_eq!(edited.bottom_x.unit, None);
        assert_eq!(edited.validate_chart(), Ok(()));
        assert_eq!(serde_json::to_value(&edited).unwrap(), value);

        value["left_y"]["unit"] = serde_json::json!({ "scale": 0.0, "offset": 0.0 });
        let refused: renderer::Config = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            refused.validate_chart().unwrap_err().kind.code(),
            "AxisUnitScale"
        );
        value["left_y"]["unit"] = serde_json::json!({ "scale": 2.0 });
        assert!(serde_json::from_value::<renderer::Config>(value).is_err());
    }

    // A stem (#206) reaches JavaScript through `get_series` and comes back
    // through `set_series` unchanged: the `stem` key on the marker variant,
    // its defaults omitted, and no key at all without stems.
    #[test]
    fn series_json_carries_a_stem_both_ways() {
        let stem = renderer::data_config::Stem {
            baseline: -1.5,
            direction: renderer::data_config::CategoryBarDirection::Horizontal,
            color: renderer::Color::new(0.4, 0.4, 0.4, 1.0),
            width: 1.5,
            line_style: renderer::line::LineStylePreset::Solid,
        };
        let series = vec![series_config(
            "s",
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: Some(scatter_style()),
                line: None,
                errors: None,
                stem: Some(stem),
            }),
        )];
        let json = serde_json::to_value(&series).unwrap();
        let written = &json[0]["render_type"]["Scatter"]["stem"];
        assert_eq!(written["baseline"], -1.5);
        assert_eq!(written["direction"], "Horizontal");
        assert!(written.get("line_style").is_none(), "{written}");
        let back: Vec<SeriesConfig> = serde_json::from_value(json).unwrap();
        assert_eq!(back, series);
        let plain = vec![series_config(
            "s",
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: Some(scatter_style()),
                line: None,
                errors: None,
                stem: None,
            }),
        )];
        let json = serde_json::to_value(&plain).unwrap();
        assert!(json[0]["render_type"]["Scatter"].get("stem").is_none());
    }

    // The math fields reach JavaScript and come back unchanged through
    // `get_config`/`set_config` and `get_series`/`set_series`; a malformed
    // tree fails at parse, so `set_config` rejects it before it touches
    // any state; `rich_text_from_tex` writes the JSON a host puts there.
    // `autoLayout` is `layout::auto_layout` through JSON: the same document
    // as the Rust call, laid out again it is unchanged, and JSON that is not
    // a config is rejected without a result.
    #[test]
    fn auto_layout_json_is_the_rust_auto_layout() {
        let mut config = renderer::default::default_config();
        config.left_y.min = 0.0;
        config.left_y.max = 0.003;
        config.left_y.major_spacing = 0.0005;
        config.left_y.title_option.text =
            renderer::text::RichText::plain("Optical depth", renderer::Color::BLACK, 22.0, "");
        let json = serde_json::to_string(&config).unwrap();
        let out = super::auto_layout_json(&json).unwrap();
        let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
        let expected = renderer::layout::auto_layout(&config, &measure);
        assert_eq!(out, serde_json::to_string(&expected).unwrap());
        assert_ne!(expected, config, "the 0.00300 labels need a layout");
        assert_eq!(super::auto_layout_json(&out).unwrap(), out);
        assert!(super::auto_layout_json("{\"chart_area\": 1}").is_err());
        assert!(super::auto_layout_json("not json").is_err());
    }

    #[test]
    fn config_and_series_json_carry_math_nodes_both_ways() {
        let title = super::rich_text_json_from_tex(
            r"$\frac{\partial u}{\partial t} = \alpha \nabla^2 u$",
            18.0,
            "",
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        let title: renderer::text::RichText = serde_json::from_str(&title).unwrap();
        assert_eq!(title.validate_math(), Ok(()));
        let mut config = renderer::default::default_config();
        config.chart_title.text = title.clone();
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            json.contains(
                r#""math_node":{"id":1,"kind":{"Frac":"Auto"}},"math_parent":{"id":0,"slot":0}"#
            ),
            "{json}"
        );
        let back: renderer::Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back, config);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);

        let broken = json.replacen(
            r#""math_parent":{"id":0,"slot":0}"#,
            r#""math_parent":{"id":7,"slot":0}"#,
            1,
        );
        assert_ne!(broken, json);
        let err = serde_json::from_str::<renderer::Config>(&broken)
            .expect_err("a malformed tree is refused at parse")
            .to_string();
        assert!(err.contains("parent"), "{err}");

        let mut series = vec![series_config(
            "s",
            DataRenderType::Xy(renderer::data_config::XySeries {
                scatter: Some(scatter_style()),
                line: None,
                errors: None,
                stem: None,
            }),
        )];
        series[0].label = Some(title);
        let json = serde_json::to_string(&series).unwrap();
        let back: Vec<SeriesConfig> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, series);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);

        let err = super::rich_text_json_from_tex(r"$\frac{a}$", 12.0, "", [0.0; 4]).unwrap_err();
        assert!(
            err.contains("missing argument") && err.contains("1..6"),
            "{err}"
        );
    }

    #[test]
    fn initial_state_requires_the_first_draw() {
        assert_eq!(
            frame_decision(true, true, false, true, false),
            FrameDecision::Draw {
                refresh_raster: true
            }
        );
    }

    #[test]
    fn successful_frame_consumes_visual_state_but_failed_frame_preserves_it() {
        let mut successful_view_dirty = true;
        let mut successful_redraw_pending = true;
        consume_successful_frame(&mut successful_view_dirty, &mut successful_redraw_pending);
        assert!(!successful_view_dirty);
        assert!(!successful_redraw_pending);

        let failed_view_dirty = true;
        let failed_redraw_pending = true;
        assert!(failed_view_dirty);
        assert!(failed_redraw_pending);
    }

    #[test]
    fn display_panel_uniformly_scales_document() {
        let (scale, panel) = renderer::fit_display_panel((1000, 800), (2000, 1600));
        assert!((scale - 2.0).abs() < 1e-6);
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (0, 0, 2000, 1600)
        );
    }

    #[test]
    fn display_panel_letterboxes_aspect_ratio_changes() {
        let (scale, panel) = renderer::fit_display_panel((1000, 800), (1600, 800));
        assert!((scale - 1.0).abs() < 1e-6);
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (300, 0, 1000, 800)
        );
    }

    #[test]
    fn display_config_scales_without_mutating_logical_document() {
        let mut config = renderer::default::default_config();
        config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
            x: 0,
            y: 0,
            width: 1000,
            height: 800,
        });
        let original = config.clone();

        let (display, panel, scale) = renderer::display_config_for_surface(&config, (500, 400));

        assert!((scale - 0.5).abs() < 1e-6);
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (0, 0, 500, 400)
        );
        assert_eq!(config.chart_area, original.chart_area);
        assert_eq!(
            config.bottom_x.label_style.font_size,
            original.bottom_x.label_style.font_size
        );
        assert_eq!(display.chart_area.0.width, 500);
        assert!((display.bottom_x.label_style.font_size - 9.0).abs() < 1e-6);
    }

    #[test]
    fn picked_point_json_matches_public_contract() {
        let picked = renderer::PickedPoint {
            source_id: None,
            series_id: "series-a".into(),
            point_index: 7,
            distance_px: 4.0,
        };

        let json: serde_json::Value =
            serde_json::from_str(&picked_point_json_string(&picked).unwrap()).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "source_id": null,
                "series_id": "series-a",
                "point_index": 7,
                "distance_px": 4.0,
            })
        );
    }

    #[test]
    fn a_feynman_hit_names_its_kind_row_and_part() {
        let picked = renderer::PickedData::FeynmanLabel {
            source_id: None,
            series_id: "diagram".into(),
            part: renderer::FeynmanLabelPart::Momentum,
            index: 3,
            distance_px: 0.0,
        };
        let json: serde_json::Value =
            serde_json::from_str(&picked_data_json_string(&picked).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "feynman_label",
                "source_id": null,
                "series_id": "diagram",
                "part": "momentum",
                "index": 3,
                "distance_px": 0.0,
            })
        );
        let vertex = renderer::PickedData::FeynmanVertex {
            source_id: None,
            series_id: "diagram".into(),
            vertex_index: 1,
            distance_px: 2.0,
        };
        let json: serde_json::Value =
            serde_json::from_str(&picked_data_json_string(&vertex).unwrap()).unwrap();
        assert_eq!(json["kind"], "feynman_vertex");
        assert_eq!(json["index"], 1);
    }

    #[test]
    fn a_value_label_hit_names_its_category_and_its_label_id() {
        let picked = renderer::PickedData::ValueLabel {
            source_id: None,
            series_id: "bars:a".into(),
            category_index: 2,
            part: None,
            distance_px: 0.0,
        };
        let json: serde_json::Value =
            serde_json::from_str(&picked_data_json_string(&picked).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "value_label",
                "source_id": null,
                "series_id": "bars:a",
                "category_index": 2,
                "label_id": "series:bars:a:label:2",
                "distance_px": 0.0,
            })
        );
        let point: serde_json::Value =
            serde_json::from_str(&picked_label_json_string(None, "bars:a", 2, 0.0).unwrap())
                .unwrap();
        assert_eq!(point["point_index"], 2);
        assert_eq!(point["label_id"], "series:bars:a:label:2");
        assert_eq!(
            renderer::data_config::parse_value_label_id(point["label_id"].as_str().unwrap()),
            Some(("bars:a", 2))
        );
    }

    #[test]
    fn picked_data_json_is_tagged_and_keeps_canonical_indices() {
        let picked = renderer::PickedData::ContourLevel {
            source_id: Some("source-grid".into()),
            series_id: "contour-a".into(),
            level_index: 4,
            x_index: 2,
            y_index: 3,
            distance_px: 0.75,
        };

        let json: serde_json::Value =
            serde_json::from_str(&picked_data_json_string(&picked).unwrap()).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "kind": "contour_level",
                "source_id": "source-grid",
                "series_id": "contour-a",
                "level_index": 4,
                "x_index": 2,
                "y_index": 3,
                "distance_px": 0.75,
            })
        );
    }

    #[test]
    fn all_render_types_normalize_to_their_drawable_extent_domain() {
        let x_error = ErrorRef::Symmetric {
            column: "x_err".to_string(),
        };
        let y_error = ErrorRef::Symmetric {
            column: "y_err".to_string(),
        };
        let cases = [
            (
                DataRenderType::Xy(renderer::data_config::XySeries {
                    scatter: None,
                    line: Some(line_style()),
                    errors: None,
                    stem: None,
                }),
                GpuSeriesExtentMode::Line,
            ),
            (
                DataRenderType::Xy(renderer::data_config::XySeries {
                    scatter: Some(scatter_style()),
                    line: None,
                    errors: None,
                    stem: None,
                }),
                GpuSeriesExtentMode::Points,
            ),
            (
                DataRenderType::Xy(renderer::data_config::XySeries {
                    scatter: Some(scatter_style()),
                    line: Some(line_style()),
                    errors: None,
                    stem: None,
                }),
                GpuSeriesExtentMode::Points,
            ),
            (
                DataRenderType::Xy(renderer::data_config::XySeries {
                    scatter: Some(scatter_style()),
                    line: None,
                    errors: Some(renderer::data_config::ErrorBars {
                        x: Some(x_error.clone()),
                        y: None,
                        style: errorbar_style(),
                    }),
                    stem: None,
                }),
                GpuSeriesExtentMode::PointsX,
            ),
            (
                DataRenderType::Xy(renderer::data_config::XySeries {
                    scatter: Some(scatter_style()),
                    line: None,
                    errors: Some(renderer::data_config::ErrorBars {
                        x: None,
                        y: Some(y_error.clone()),
                        style: errorbar_style(),
                    }),
                    stem: None,
                }),
                GpuSeriesExtentMode::PointsY,
            ),
            (
                DataRenderType::Xy(renderer::data_config::XySeries {
                    scatter: Some(scatter_style()),
                    line: None,
                    errors: Some(renderer::data_config::ErrorBars {
                        x: Some(x_error.clone()),
                        y: Some(y_error.clone()),
                        style: errorbar_style(),
                    }),
                    stem: None,
                }),
                GpuSeriesExtentMode::PointsXY,
            ),
            (
                DataRenderType::Xy(renderer::data_config::XySeries {
                    scatter: Some(scatter_style()),
                    line: Some(line_style()),
                    errors: Some(renderer::data_config::ErrorBars {
                        x: Some(x_error.clone()),
                        y: None,
                        style: errorbar_style(),
                    }),
                    stem: None,
                }),
                GpuSeriesExtentMode::PointsX,
            ),
            (
                DataRenderType::Xy(renderer::data_config::XySeries {
                    scatter: Some(scatter_style()),
                    line: Some(line_style()),
                    errors: Some(renderer::data_config::ErrorBars {
                        x: None,
                        y: Some(y_error.clone()),
                        style: errorbar_style(),
                    }),
                    stem: None,
                }),
                GpuSeriesExtentMode::PointsY,
            ),
            (
                DataRenderType::Xy(renderer::data_config::XySeries {
                    scatter: Some(scatter_style()),
                    line: Some(line_style()),
                    errors: Some(renderer::data_config::ErrorBars {
                        x: Some(x_error),
                        y: Some(y_error),
                        style: errorbar_style(),
                    }),
                    stem: None,
                }),
                GpuSeriesExtentMode::PointsXY,
            ),
        ];

        for (render_type, expected) in cases {
            assert_eq!(
                GpuSeriesFitMode::from_render_type(&render_type),
                Some(GpuSeriesFitMode::Paired(expected))
            );
        }

        let matrix = |grid_layout| MatrixRef {
            columns: vec!["z0".into(), "z1".into()],
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout,
        };
        let fill = |shading| FieldFillConfig {
            mode: FillMode::Continuous,
            shading,
            opacity: 1.0,
        };
        let contour = || ContourConfig {
            levels: vec![0.5],
            line: line_style(),
            per_level_color: None,
            labels: None,
        };
        let field_cases = [
            (
                DataRenderType::Field(renderer::data_config::FieldSeries {
                    matrix: matrix(GridLayout::Edges),
                    fill: Some(fill(Shading::Flat)),
                    contour: None,
                    surface: None,
                }),
                GpuFieldExtentMode::EdgesCells,
            ),
            (
                DataRenderType::Field(renderer::data_config::FieldSeries {
                    matrix: matrix(GridLayout::Edges),
                    fill: Some(fill(Shading::Interpolated)),
                    contour: None,
                    surface: None,
                }),
                GpuFieldExtentMode::EdgesSamples,
            ),
            (
                DataRenderType::Field(renderer::data_config::FieldSeries {
                    matrix: matrix(GridLayout::Centers),
                    fill: None,
                    contour: Some(contour()),
                    surface: None,
                }),
                GpuFieldExtentMode::CentersSamples,
            ),
            (
                DataRenderType::Field(renderer::data_config::FieldSeries {
                    matrix: matrix(GridLayout::Centers),
                    fill: Some(fill(Shading::Flat)),
                    contour: Some(contour()),
                    surface: None,
                }),
                GpuFieldExtentMode::CentersCells,
            ),
            (
                DataRenderType::Field(renderer::data_config::FieldSeries {
                    matrix: matrix(GridLayout::Centers),
                    fill: Some(fill(Shading::Interpolated)),
                    contour: Some(contour()),
                    surface: None,
                }),
                GpuFieldExtentMode::CentersSamples,
            ),
        ];
        for (render_type, expected) in field_cases {
            assert_eq!(
                GpuSeriesFitMode::from_render_type(&render_type),
                Some(GpuSeriesFitMode::Field(expected))
            );
        }
    }

    #[test]
    fn a_column_batch_is_validated_before_anything_is_uploaded() {
        let columns = |id: &str| id == "taken";
        let ids = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();

        // The shape that works: 3 ids x 2 values = 6.
        assert!(validate_column_batch(columns, &ids(&["a", "b", "c"]), 2, 6).is_ok());

        assert_eq!(
            validate_column_batch(columns, &[], 2, 0).unwrap_err(),
            "a column batch needs at least one id"
        );
        assert_eq!(
            validate_column_batch(columns, &ids(&["a"]), 0, 0).unwrap_err(),
            "values_per_column data must not be empty"
        );
        // A flat buffer that does not divide into the declared shape is not a
        // length mismatch to draw through — it cannot be cut into columns at all.
        assert!(
            validate_column_batch(columns, &ids(&["a", "b"]), 3, 5)
                .unwrap_err()
                .contains("5 values")
        );
        assert_eq!(
            validate_column_batch(columns, &ids(&["a", "taken"]), 2, 4).unwrap_err(),
            "column 'taken' is already registered"
        );
        assert_eq!(
            validate_column_batch(columns, &ids(&["a", "a"]), 2, 4).unwrap_err(),
            "column 'a' appears twice in one batch"
        );
        assert!(validate_column_batch(columns, &ids(&["__zero"]), 2, 2).is_ok());
    }

    #[test]
    fn demo_declarations_are_complete_and_stable_on_repeat() {
        let initial_config = renderer::default::default_config();
        let first =
            prepare_demo_declarations(&initial_config, &[], renderer::ColorCycle::Classic, 0)
                .unwrap();
        assert_eq!(
            first
                .series
                .iter()
                .map(|item| item.series_id.as_str())
                .collect::<Vec<_>>(),
            vec!["sine", "rc"]
        );
        assert_eq!(
            first
                .series
                .iter()
                .map(|item| item.label.as_ref().map(|label| label.segments.clone()))
                .collect::<Vec<_>>(),
            vec![
                Some(renderer::text::rich_segments_from_text("sin(x)")),
                Some(renderer::text::rich_segments_from_text("RC charge"))
            ]
        );
        assert_eq!(first.color_seq, 2);
        assert_eq!(
            first.config.chart_title.text.segments,
            renderer::text::rich_segments_from_text("figgy")
        );
        assert_eq!(
            first.config.bottom_x.title_option.text.segments,
            renderer::text::rich_segments_from_text("x")
        );
        assert_eq!(
            first.config.left_y.title_option.text.segments,
            renderer::text::rich_segments_from_text("y")
        );

        let repeated = prepare_demo_declarations(
            &first.config,
            &first.series,
            renderer::ColorCycle::Classic,
            first.color_seq,
        )
        .unwrap();
        assert_eq!(repeated.config, first.config);
        assert_eq!(repeated.series, first.series);
        assert_eq!(repeated.color_seq, first.color_seq);
    }

    /// The demo's legend entries are found by the series they stand for: a
    /// host series without a label sits before them, so their entries are
    /// not at their series' positions, and loading the demo again restyles
    /// them in place instead of relabeling a neighbour or adding a row.
    #[test]
    fn demo_legend_entries_follow_their_series_past_an_unlabeled_one() {
        let config = renderer::default::default_config();
        let host = vec![line_series("host", "host-x", "host-y")];
        assert!(host[0].label.is_none());
        let first =
            prepare_demo_declarations(&config, &host, renderer::ColorCycle::Classic, 1).unwrap();
        let repeated = prepare_demo_declarations(
            &first.config,
            &first.series,
            renderer::ColorCycle::Classic,
            first.color_seq,
        )
        .unwrap();
        let content = &repeated.config.legend.content;
        assert_eq!(renderer::config::legend_entry_count(content), 2);
        assert_eq!(
            renderer::config::legend_entry_series_id(content, 0),
            Some("sine")
        );
        assert_eq!(
            renderer::config::legend_entry_series_id(content, 1),
            Some("rc")
        );
        assert_eq!(repeated.config, first.config);
    }

    #[test]
    fn demo_precommit_abort_preserves_every_host_owned_authority() {
        let config = renderer::default::default_config();
        let series = vec![line_series("stable", "stable-x", "stable-y")];
        let styles = vec![7u32];
        let color_seq = 1usize;

        let prepared_declarations =
            prepare_demo_declarations(&config, &series, renderer::ColorCycle::Classic, color_seq)
                .unwrap();
        drop(prepared_declarations);

        assert_eq!(config, renderer::default::default_config());
        assert_eq!(series, vec![line_series("stable", "stable-x", "stable-y")]);
        assert_eq!(styles, vec![7]);
        assert_eq!(color_seq, 1);
    }

    #[test]
    fn column_registry_actions_fail_closed() {
        assert_eq!(
            validate_column_registry_action(false, ColumnRegistryAction::Register),
            Ok(())
        );
        assert_eq!(
            validate_column_registry_action(true, ColumnRegistryAction::Register),
            Err("is already registered")
        );
        assert_eq!(
            validate_column_registry_action(true, ColumnRegistryAction::Update),
            Ok(())
        );
        assert_eq!(
            validate_column_registry_action(false, ColumnRegistryAction::Update),
            Err("is not registered")
        );
    }

    #[test]
    fn column_data_length_is_validated() {
        assert_eq!(validate_column_data_len(0), Err("data must not be empty"));
        assert_eq!(validate_column_data_len(1), Ok(()));
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::Cell;

    use wasm_bindgen::prelude::*;
    use web_sys::HtmlCanvasElement;

    use renderer::layout::{ChartArea, NudgeResult, Rect};
    use renderer::line::LineStylePreset;
    use renderer::text::{RichText, rich_segments_from_label};
    use renderer::{
        Chart, ChartId, ChartRenderStamp, ChartView, Color, ColumnSource, CpuTextMeasure,
        DataLineStyleConfig, DataRenderType, DefragPolicy, FitExtent, HiLoColumnSource, HitId,
        HitMap, LayerHit, RegisteredChartDrawItem, Renderer, ResizeHandle as ModelResizeHandle,
        SeriesConfig, StreamColumn, StreamEncoding, StreamRangeSourceBinding, StreamReplay,
        StreamSourceBinding, StreamStatistics, StreamingChartOptions, StreamingLimits,
        WindowedRenderer,
    };

    use renderer::data_render::{AllocError, GrowthPolicy};
    use renderer::gpu_memory::GpuResourceKind;
    use renderer::{InitEvent, InitPhase};

    use crate::borrowed_column::{
        BorrowedCastF32Column, BorrowedF32Column, BorrowedF64Column, BorrowedU32Column,
        JsStreamColumn,
    };
    use crate::{
        ColumnRegistryAction, DEMO_COLUMN_IDS, FrameDecision, PreparedDemoDeclarations,
        consume_successful_frame, frame_decision, prepare_demo_declarations, set_chart_title_label,
        validate_column_batch, validate_column_data_len, validate_column_registry_action,
    };

    mod chart;
    pub use chart::{
        FiggyChart, auto_layout, draw_style_modes, draw_style_param_specs, rich_text_from_tex,
    };
    mod columns;
    use columns::{VectorColumns, referenced_columns, vector_columns};
    pub(crate) mod convert;
    pub(crate) use convert::radial_theme_named;
    pub use convert::{ColorCycle, color_cycle_css};
    use convert::{column_registration_error, js_err, js_safe_u64};
    mod decorations;
    pub use decorations::AxisPreset;
    pub(crate) use decorations::vertex_handle_index;
    pub(crate) use decorations::{annotation_kind_name, annotation_warnings_json};
    mod export;
    mod monotonic;
    mod picking;
    mod streaming;
    pub use streaming::{AutoStreamingRangeRequestResult, AutoStreamingRequestResult};
    pub use streaming::{StreamingOperationHandle, StreamingStepResult, StreamingUsageResult};

    #[cfg(test)]
    thread_local! {
        static LOAD_DEMO_ABORT_BEFORE_RENDERER_COMMIT: Cell<bool> = const { Cell::new(false) };
    }

    #[cfg(test)]
    fn set_load_demo_abort_before_renderer_commit(enabled: bool) {
        LOAD_DEMO_ABORT_BEFORE_RENDERER_COMMIT.with(|state| state.set(enabled));
    }

    #[cfg(test)]
    mod tests {
        use std::hash::{Hash, Hasher};

        use wasm_bindgen::JsCast;
        use wasm_bindgen_test::*;

        use super::*;

        wasm_bindgen_test_configure!(run_in_browser);

        #[derive(Debug, PartialEq)]
        struct PoolAuthority {
            buffer: u64,
            capacity: u64,
            generation: u32,
            used_bytes: u64,
            free_bytes: u64,
            slots: Vec<(String, u64, u64, usize, u32, u64, u64, Option<u64>)>,
        }

        #[derive(Debug, PartialEq)]
        struct FiggyAuthority {
            config: renderer::Config,
            series: Vec<SeriesConfig>,
            column_revisions: Vec<(String, Option<u64>)>,
            legend_auto_managed: bool,
            color_seq: usize,
            chart_stamp: ChartRenderStamp,
            visual_revision: renderer::RenderRevision,
            picker_visible_selection: Option<HitId>,
            pending_maintenance: bool,
            pool: PoolAuthority,
        }

        fn canvas() -> HtmlCanvasElement {
            let document = web_sys::window()
                .expect("window")
                .document()
                .expect("document");
            let canvas = document
                .create_element("canvas")
                .expect("canvas element")
                .dyn_into::<HtmlCanvasElement>()
                .expect("HtmlCanvasElement");
            canvas.set_width(800);
            canvas.set_height(500);
            canvas
        }

        fn buffer_identity(buffer: &wgpu::Buffer) -> u64 {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            buffer.hash(&mut hasher);
            hasher.finish()
        }

        fn snapshot(chart: &FiggyChart) -> FiggyAuthority {
            let pool = chart.renderer.pool();
            let mut ids = chart
                .renderer
                .logical_column_ids()
                .map(str::to_string)
                .collect::<Vec<_>>();
            ids.sort();
            ids.dedup();
            let column_revisions = ids
                .iter()
                .map(|id| (id.clone(), chart.renderer.column_revision(id)))
                .collect();
            let slots = ids
                .into_iter()
                .map(|id| {
                    let slot = pool
                        .slot(&id)
                        .expect("host registry and renderer pool stay synchronized");
                    (
                        id,
                        slot.offset,
                        slot.byte_size,
                        slot.len_values,
                        slot.generation,
                        slot.min.to_bits(),
                        slot.max.to_bits(),
                        slot.min_positive.map(f64::to_bits),
                    )
                })
                .collect();

            FiggyAuthority {
                config: chart.chart_config().unwrap().clone(),
                series: chart.chart_series().unwrap().to_vec(),
                column_revisions,
                legend_auto_managed: chart.legend_auto_managed,
                color_seq: chart.color_seq,
                chart_stamp: chart.renderer.chart_render_stamp(chart.chart_id).unwrap(),
                visual_revision: chart.renderer.visual_revision(),
                picker_visible_selection: chart.renderer.chart_selection(chart.chart_id).unwrap(),
                pending_maintenance: chart.renderer.has_pending_maintenance(),
                pool: PoolAuthority {
                    buffer: buffer_identity(pool.buffer()),
                    capacity: pool.capacity(),
                    generation: pool.generation(),
                    used_bytes: pool.used_bytes(),
                    free_bytes: pool.free_bytes(),
                    slots,
                },
            }
        }

        async fn chart_with_stable_extent() -> FiggyChart {
            let mut chart = FiggyChart::create(canvas()).await.expect("create chart");
            chart
                .register_column_f64("stable-x", &[0.0, 1.0, 2.0])
                .unwrap();
            chart
                .register_column_f64("stable-y", &[2.0, 3.0, 4.0])
                .unwrap();
            chart
                .add_line_series("stable", "stable-x", "stable-y", 2.0, "stable")
                .unwrap();
            chart.ensure_extent_engine().await.unwrap();
            chart.auto_fit_all(0.0).await.unwrap();
            chart
        }

        #[wasm_bindgen_test(async)]
        async fn real_load_demo_late_abort_preserves_every_authority_then_retries() {
            set_load_demo_abort_before_renderer_commit(false);
            let mut chart = chart_with_stable_extent().await;
            chart.renderer.enable_gpu_picking_async().await.unwrap();
            chart
                .renderer
                .prepare_gpu_picking_for_chart(chart.chart_id)
                .unwrap();
            let before = snapshot(&chart);

            set_load_demo_abort_before_renderer_commit(true);
            let error = chart
                .load_demo()
                .expect_err("test hook must abort load_demo");
            set_load_demo_abort_before_renderer_commit(false);
            assert_eq!(
                error.as_string().as_deref(),
                Some("injected load_demo abort before renderer commit")
            );
            assert_eq!(snapshot(&chart), before);

            chart.load_demo().expect("retry must commit");
            for id in crate::DEMO_COLUMN_IDS {
                assert_eq!(
                    chart.renderer.logical_column(id).map(|c| c.len()),
                    Some(512)
                );
                assert!(chart.renderer.pool().slot(id).is_some());
            }
            assert!(
                chart
                    .chart_series()
                    .unwrap()
                    .iter()
                    .any(|series| series.series_id == "sine")
            );
            assert!(
                chart
                    .chart_series()
                    .unwrap()
                    .iter()
                    .any(|series| series.series_id == "rc")
            );
        }

        /// A matrix-shaped batch registers every column in one upload, and the
        /// f64 form keeps the `(hi, lo)` precision the single-column path does.
        ///
        /// `ColumnPool` creations answers the "one upload" claim: the batch takes a
        /// single staging buffer, so a 16-column grid must not create 16.
        #[wasm_bindgen_test(async)]
        async fn a_column_batch_registers_every_column_in_one_upload() {
            const N: usize = 16;
            let mut chart = FiggyChart::create(canvas()).await.expect("create chart");
            let ids: Vec<String> = (0..N).map(|c| format!("g{c}")).collect();
            // A grid whose values need the low half: the integer part alone would
            // round away the +0.125 / +0.875 offsets in f32.
            let data: Vec<f64> = (0..N)
                .flat_map(|c| (0..N).map(move |r| 1_700_000_000_000.0 + c as f64 + r as f64 / 8.0))
                .collect();

            let before = chart
                .renderer
                .gpu_memory_usage()
                .creations_of(renderer::GpuResourceKind::ColumnPool);
            chart
                .register_columns_f64(ids.clone(), &data, N)
                .expect("batch register");
            let creations = chart
                .renderer
                .gpu_memory_usage()
                .creations_of(renderer::GpuResourceKind::ColumnPool)
                - before;
            assert!(
                creations <= 1,
                "{N} columns must upload through one staging buffer, not {creations}"
            );

            for id in &ids {
                assert_eq!(
                    chart.renderer.logical_column(id).map(|c| c.len()),
                    Some(N as u64)
                );
                assert!(chart.renderer.column_revision(id).is_some());
            }
            // One revision per column, not one for the batch.
            let mut revisions: Vec<u64> = ids
                .iter()
                .map(|id| chart.renderer.column_revision(id).unwrap())
                .collect();
            revisions.sort_unstable();
            revisions.dedup();
            assert_eq!(revisions.len(), N);

            // A second batch naming one of the same ids is rejected whole.
            let clash = vec![format!("fresh"), ids[0].clone()];
            let small: Vec<f64> = vec![0.0; 2 * N];
            assert!(chart.register_columns_f64(clash, &small, N).is_err());
            assert!(chart.renderer.logical_column("fresh").is_none());
        }

        // ---- #152 index invariants at their producers ----

        /// One chart a selection is read from: its columns, its box record and
        /// ragged columns, the config edit and series that draw them, the pick and
        /// the keys `selection_details` answers with.
        struct SelectionScene {
            name: &'static str,
            columns: Vec<(String, Vec<f64>)>,
            record: Option<(&'static str, Vec<(&'static str, Vec<f64>)>)>,
            ragged: Vec<(&'static str, Vec<f64>, Vec<f64>)>,
            edit: fn(&mut renderer::Config),
            series: renderer::SeriesConfig,
            pick: renderer::config::PickedDataRef,
            keys: Vec<String>,
        }

        fn selection_scenes() -> Vec<SelectionScene> {
            use renderer::config::{Category, PickedDataRef};
            use renderer::data_config::{
                BarOrientation, BoxPlotSeriesLabels, BoxPlotStyle, CategoryBarLabelFormat,
                CategoryBarSeriesLabels, CategoryBarStyle, DataBarStyleConfig, FieldFillConfig,
                FillMode, GridLayout, LineInterpolation, MatrixOrientation, MatrixRef,
                PointColorMap, PointSizeMap, RangeColor, RangeProgress, RangeStateStyle,
                RangeWhiskers, Shading, SizeScale, VectorColorMap, VectorComponents, VectorPivot,
                VectorScale, VectorStyle, Violin, ViolinBody, ViolinHalf, ViolinStyle,
            };
            use renderer::{Color, DataRenderType, SeriesConfig};

            const AXES: usize = 20;
            let column = |id: &str, data: &[f64]| (id.to_string(), data.to_vec());
            let series = |id: &str, x: &str, y: &str, render_type| SeriesConfig {
                series_id: id.into(),
                source_id: None,
                label: None,
                x_column: x.into(),
                y_column: y.into(),
                z_column: None,
                render_type,
            };
            let point = |id: &str, point_index| PickedDataRef::Point {
                source_id: None,
                series_id: id.into(),
                point_index,
            };
            let bin = |id: &str, bin_index| PickedDataRef::HistogramBin {
                source_id: None,
                series_id: id.into(),
                bin_index,
                half: None,
            };
            let cell = |id: &str, x_index, y_index| PickedDataRef::MatrixCell {
                source_id: None,
                series_id: id.into(),
                x_index,
                y_index,
            };
            let keys = |names: &[&str]| names.iter().map(|k| k.to_string()).collect::<Vec<_>>();
            let scatter = |color_map, size_map| renderer::DataScatterStyleConfig {
                point_color: Color::BLACK,
                point_shape: renderer::ScatterShape::CircleFilled,
                point_size: 4.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: color_map,
                point_size_map: size_map,
            };
            let vector_style = |color_map| VectorStyle {
                color: Color::BLACK,
                width_px: 2.0,
                scale: VectorScale::Data { factor: 1.0 },
                pivot: VectorPivot::Tail,
                head_length_px: 8.0,
                head_width_px: 8.0,
                min_length_px: 0.0,
                color_map,
            };
            let matrix = |ids: &[&str], grid_layout| MatrixRef {
                columns: ids.iter().map(|id| id.to_string()).collect(),
                orientation: MatrixOrientation::ColumnsAreX,
                grid_layout,
            };
            let heatmap = |grid_layout| {
                DataRenderType::Field(renderer::data_config::FieldSeries {
                    matrix: matrix(&["z0", "z1"], grid_layout),
                    fill: Some(FieldFillConfig {
                        mode: FillMode::Continuous,
                        shading: Shading::Flat,
                        opacity: 1.0,
                    }),
                    contour: None,
                    surface: None,
                })
            };
            let bar_style = || CategoryBarStyle::default();
            let box_plot = |violin| DataRenderType::BoxPlot {
                color: Color::from_rgb8(40, 120, 200),
                style: BoxPlotStyle::default(),
                labels: BoxPlotSeriesLabels::default(),
                outliers: None,
                overrides: None,
                violin,
                points: None,
            };
            fn colorbar(config: &mut renderer::Config) {
                config.colorbar = Some(renderer::default::default_colorbar_options());
            }
            fn three_categories(config: &mut renderer::Config) {
                config.bottom_x.categories = Some(
                    ["a", "b", "c"]
                        .map(|id| Category::new(id, id.to_uppercase()))
                        .to_vec(),
                );
            }
            fn ternary(config: &mut renderer::Config) {
                for axis in [
                    &mut config.bottom_x,
                    &mut config.left_y,
                    &mut config.right_y,
                ] {
                    axis.min = 0.0;
                    axis.max = 100.0;
                    axis.major_spacing = 20.0;
                }
                config.set_ternary(Some(renderer::ternary::TernaryOptions {}));
            }
            fn parallel(config: &mut renderer::Config) {
                use renderer::parallel::{ParallelAxis, ParallelOptions};
                colorbar(config);
                config.bottom_x.categories = Some(
                    (0..AXES)
                        .map(|i| Category::new(format!("p{i}"), format!("P{i}")))
                        .collect(),
                );
                config.left_y.min = 0.0;
                config.left_y.max = 1.0;
                config.left_y.categories = None;
                config.set_parallel(Some(ParallelOptions {
                    axes: (0..AXES).map(|_| ParallelAxis::new(0.0, 10.0)).collect(),
                }));
            }
            let record_fields = |names: &[&'static str]| {
                names
                    .iter()
                    .enumerate()
                    .map(|(k, name)| (*name, vec![k as f64, k as f64 + 1.0, k as f64 + 2.0]))
                    .collect::<Vec<_>>()
            };
            let all_fields = renderer::BOX_PLOT_RECORD
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>();
            let required = renderer::BOX_PLOT_RECORD
                .iter()
                .filter(|(_, required)| *required)
                .map(|(name, _)| *name)
                .collect::<Vec<_>>();
            let parallel_columns: Vec<String> = (0..AXES).map(|i| format!("pa{i}")).collect();
            let mut parallel_data: Vec<(String, Vec<f64>)> = parallel_columns
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    (
                        id.clone(),
                        vec![i as f64 * 0.25, 5.0, 9.0 - i as f64 * 0.25],
                    )
                })
                .collect();
            parallel_data.push(column("pcol", &[0.1, 0.5, 0.9]));
            let mut parallel_line = renderer::DataLineStyleConfig::default();
            parallel_line.interpolation = LineInterpolation::Linear;

            vec![
                SelectionScene {
                    name: "scatter with colour and size maps",
                    columns: vec![
                        column("sx", &[0.0, 1.0, 2.0, 3.0]),
                        column("sy", &[3.0, 1.0, 4.0, 1.0]),
                        column("sc", &[0.1, 0.2, 0.3, 0.4]),
                        column("ss", &[1.0, 2.0, 3.0, 4.0]),
                    ],
                    record: None,
                    ragged: vec![],
                    edit: colorbar,
                    series: series(
                        "scatter",
                        "sx",
                        "sy",
                        DataRenderType::Xy(renderer::data_config::XySeries {
                            scatter: Some(scatter(
                                Some(PointColorMap {
                                    column: "sc".into(),
                                }),
                                Some(PointSizeMap {
                                    column: "ss".into(),
                                    domain_min: 1.0,
                                    domain_max: 4.0,
                                    min_px: 2.0,
                                    max_px: 8.0,
                                    scale: SizeScale::Area,
                                }),
                            )),
                            line: None,
                            errors: None,
                            stem: None,
                        }),
                    ),
                    pick: point("scatter", 2),
                    keys: keys(&["x", "y", "color_value", "size_value"]),
                },
                SelectionScene {
                    name: "point vector with a colour column",
                    columns: vec![
                        column("vx", &[0.0, 1.0, 2.0]),
                        column("vy", &[0.0, 1.0, 2.0]),
                        column("vu", &[1.0, 0.5, 0.25]),
                        column("vv", &[0.0, 0.5, 1.0]),
                        column("vc", &[2.0, 3.0, 4.0]),
                    ],
                    record: None,
                    ragged: vec![],
                    edit: colorbar,
                    series: series(
                        "arrows",
                        "vx",
                        "vy",
                        DataRenderType::Vector {
                            components: VectorComponents::Points {
                                u: "vu".into(),
                                v: "vv".into(),
                                w: None,
                            },
                            style: vector_style(Some(VectorColorMap::Column("vc".into()))),
                        },
                    ),
                    pick: point("arrows", 1),
                    keys: keys(&["x", "y", "u", "v", "color_value"]),
                },
                SelectionScene {
                    name: "3D point vector with its height and z component",
                    columns: vec![
                        column("ax", &[0.2, 0.5, 0.8]),
                        column("ay", &[0.3, 0.5, 0.7]),
                        column("az", &[0.1, 0.4, 0.6]),
                        column("au", &[0.1, 0.0, -0.1]),
                        column("av", &[0.0, 0.1, 0.0]),
                        column("aw", &[0.2, 0.3, 0.1]),
                    ],
                    record: None,
                    ragged: vec![],
                    edit: |config| {
                        config.set_view_3d(Some(renderer::view3d::View3d::default()));
                    },
                    series: SeriesConfig {
                        z_column: Some("az".into()),
                        ..series(
                            "arrows3d",
                            "ax",
                            "ay",
                            DataRenderType::Vector {
                                components: VectorComponents::Points {
                                    u: "au".into(),
                                    v: "av".into(),
                                    w: Some("aw".into()),
                                },
                                style: vector_style(None),
                            },
                        )
                    },
                    pick: point("arrows3d", 1),
                    keys: keys(&["x", "y", "z", "u", "v", "w"]),
                },
                SelectionScene {
                    name: "parallel rows over twenty axes with a colour column",
                    columns: parallel_data,
                    record: None,
                    ragged: vec![],
                    edit: parallel,
                    series: series(
                        "rows",
                        &parallel_columns[0],
                        &parallel_columns[1],
                        DataRenderType::Parallel {
                            columns: parallel_columns.clone(),
                            line: parallel_line,
                            color_column: Some("pcol".into()),
                        },
                    ),
                    pick: point("rows", 1),
                    keys: keys(&["x", "y", "values", "color_value"]),
                },
                SelectionScene {
                    name: "histogram",
                    columns: vec![
                        column("edges", &[0.0, 1.0, 2.5, 4.0]),
                        column("heights", &[1.0, 2.0, 3.0]),
                    ],
                    record: None,
                    ragged: vec![],
                    edit: |_| {},
                    series: series(
                        "hist",
                        "edges",
                        "heights",
                        DataRenderType::Histogram {
                            bar: DataBarStyleConfig {
                                fill_color: Color::BLACK,
                                border_color: Color::BLACK,
                                border_width: 0.0,
                                baseline: 0.0,
                                gap_px: 0.0,
                                width_ratio: 1.0,
                                orientation: BarOrientation::Vertical,
                                bar_style_overrides: None,
                            },
                        },
                    ),
                    pick: bin("hist", 1),
                    keys: keys(&["bin_start", "bin_end", "value", "orientation"]),
                },
                SelectionScene {
                    name: "category bars",
                    columns: vec![
                        column("cpos", &[0.0, 1.0, 2.0]),
                        column("cval", &[5.0, 6.0, 7.0]),
                    ],
                    record: None,
                    ragged: vec![],
                    edit: three_categories,
                    series: series(
                        "bars",
                        "cpos",
                        "cval",
                        DataRenderType::CategoryBars {
                            color: Color::from_rgb8(40, 120, 200),
                            style: bar_style(),
                            labels: CategoryBarSeriesLabels::default(),
                            label_format: CategoryBarLabelFormat::Value,
                            overrides: None,
                        },
                    ),
                    pick: bin("bars", 1),
                    keys: keys(&["category_id", "value", "direction", "mode"]),
                },
                SelectionScene {
                    name: "range bars with every role",
                    columns: vec![
                        column("rpos", &[0.0, 1.0, 2.0]),
                        column("rend", &[4.0, 5.0, 6.0]),
                        column("rstart", &[1.0, 2.0, 3.0]),
                        column("rlow", &[0.5, 1.5, 2.5]),
                        column("rhigh", &[4.5, 5.5, 6.5]),
                        column("rstate", &[0.0, 1.0, 0.0]),
                        column("rprogress", &[0.2, 0.5, 1.0]),
                    ],
                    record: None,
                    ragged: vec![],
                    edit: three_categories,
                    series: series(
                        "ranges",
                        "rpos",
                        "rend",
                        DataRenderType::RangeBars {
                            color: Color::from_rgb8(40, 120, 200),
                            style: bar_style(),
                            start: "rstart".into(),
                            whiskers: Some(RangeWhiskers {
                                low: "rlow".into(),
                                high: "rhigh".into(),
                                width: 0.3,
                                color: None,
                            }),
                            color_by: RangeColor::State {
                                column: "rstate".into(),
                                styles: vec![
                                    RangeStateStyle {
                                        color: Color::from_rgb8(40, 120, 200),
                                    },
                                    RangeStateStyle {
                                        color: Color::from_rgb8(200, 80, 40),
                                    },
                                ],
                            },
                            progress: Some(RangeProgress {
                                column: "rprogress".into(),
                                remaining_opacity: 0.4,
                            }),
                            connector: None,
                            slot_width: None,
                            labels: CategoryBarSeriesLabels::default(),
                        },
                    ),
                    pick: bin("ranges", 1),
                    keys: keys(&[
                        "position",
                        "end",
                        "start",
                        "low",
                        "high",
                        "state",
                        "progress",
                        "category_id",
                    ]),
                },
                SelectionScene {
                    name: "box record with every field",
                    columns: vec![column("bpos", &[0.0, 1.0, 2.0])],
                    record: Some(("box9", record_fields(&all_fields))),
                    ragged: vec![],
                    edit: three_categories,
                    series: series("box9", "bpos", "box9", box_plot(None)),
                    pick: bin("box9", 1),
                    keys: {
                        let mut k = keys(&all_fields);
                        k.push("category_id".into());
                        k
                    },
                },
                SelectionScene {
                    name: "box record with the required fields",
                    columns: vec![column("bpos", &[0.0, 1.0, 2.0])],
                    record: Some(("box5", record_fields(&required))),
                    ragged: vec![],
                    edit: three_categories,
                    series: series("box5", "bpos", "box5", box_plot(None)),
                    pick: bin("box5", 1),
                    keys: {
                        let mut k = keys(&required);
                        k.push("category_id".into());
                        k
                    },
                },
                SelectionScene {
                    name: "violin without boxes",
                    columns: vec![
                        column("bpos", &[0.0, 1.0, 2.0]),
                        column("vdensity", &[0.1, 0.5, 0.1, 0.2, 0.6, 0.2, 0.3, 0.7, 0.3]),
                    ],
                    record: None,
                    ragged: vec![(
                        "vcurve",
                        vec![1.0, 2.0, 3.0, 1.5, 2.5, 3.5, 2.0, 3.0, 4.0],
                        vec![0.0, 3.0, 6.0, 9.0],
                    )],
                    edit: three_categories,
                    series: series(
                        "violin",
                        "bpos",
                        "",
                        box_plot(Some(Violin {
                            body: ViolinBody::Symmetric(ViolinHalf {
                                curve: "vcurve".into(),
                                density: "vdensity".into(),
                                density_scale: 0.5,
                                style: ViolinStyle {
                                    fill: None,
                                    outline: true,
                                    outline_width: 1.0,
                                    outline_color: Color::BLACK,
                                },
                            }),
                            box_width: 0.2,
                        })),
                    ),
                    pick: bin("violin", 1),
                    keys: keys(&["category_id", "direction"]),
                },
                SelectionScene {
                    name: "heatmap with cell edges",
                    columns: vec![
                        column("hx", &[0.0, 1.0, 2.0]),
                        column("hy", &[0.0, 1.0, 2.0, 3.0]),
                        column("z0", &[1.0, 2.0, 3.0]),
                        column("z1", &[4.0, 5.0, 6.0]),
                    ],
                    record: None,
                    ragged: vec![],
                    edit: colorbar,
                    series: series("edges", "hx", "hy", heatmap(GridLayout::Edges)),
                    pick: cell("edges", 1, 2),
                    keys: keys(&["x", "y", "value", "x_index", "y_index"]),
                },
                SelectionScene {
                    name: "hexagonal heatmap",
                    columns: vec![
                        column("hx", &[0.0, 1.0]),
                        column("hy", &[0.0, 1.0, 2.0]),
                        column("z0", &[1.0, 2.0, 3.0]),
                        column("z1", &[4.0, 5.0, 6.0]),
                    ],
                    record: None,
                    ragged: vec![],
                    edit: colorbar,
                    series: series("hex", "hx", "hy", heatmap(GridLayout::Hexagonal)),
                    pick: cell("hex", 1, 1),
                    keys: keys(&["x", "y", "value"]),
                },
                SelectionScene {
                    name: "grid vector with a colour grid",
                    columns: vec![
                        column("gx", &[0.0, 1.0]),
                        column("gy", &[0.0, 1.0, 2.0]),
                        column("gu0", &[1.0, 1.0, 1.0]),
                        column("gu1", &[0.5, 0.5, 0.5]),
                        column("gv0", &[0.0, 0.2, 0.4]),
                        column("gv1", &[0.1, 0.3, 0.5]),
                        column("gc0", &[1.0, 2.0, 3.0]),
                        column("gc1", &[4.0, 5.0, 6.0]),
                    ],
                    record: None,
                    ragged: vec![],
                    edit: colorbar,
                    series: series(
                        "grid",
                        "gx",
                        "gy",
                        DataRenderType::Vector {
                            components: VectorComponents::Grid {
                                u: matrix(&["gu0", "gu1"], GridLayout::Centers),
                                v: matrix(&["gv0", "gv1"], GridLayout::Centers),
                            },
                            style: vector_style(Some(VectorColorMap::Grid(matrix(
                                &["gc0", "gc1"],
                                GridLayout::Centers,
                            )))),
                        },
                    ),
                    pick: cell("grid", 1, 2),
                    keys: keys(&["x", "y", "u", "v", "color_value"]),
                },
                SelectionScene {
                    name: "ternary point with its third component",
                    columns: vec![
                        column("ta", &[20.0, 30.0, 50.0]),
                        column("tb", &[30.0, 30.0, 25.0]),
                        column("tc", &[50.0, 40.0, 25.0]),
                    ],
                    record: None,
                    ragged: vec![],
                    edit: ternary,
                    series: series(
                        "mix",
                        "ta",
                        "tb",
                        DataRenderType::Ternary {
                            c_column: "tc".into(),
                            scatter: Some(scatter(None, None)),
                            line: None,
                            region: None,
                        },
                    ),
                    pick: point("mix", 2),
                    keys: keys(&["x", "y", "c"]),
                },
                SelectionScene {
                    name: "rgb image",
                    columns: vec![
                        column("ix", &[0.0, 1.0]),
                        column("iy", &[0.0, 1.0, 2.0]),
                        column("r0", &[0.1, 0.2, 0.3]),
                        column("r1", &[0.4, 0.5, 0.6]),
                        column("g0", &[0.2, 0.3, 0.4]),
                        column("g1", &[0.5, 0.6, 0.7]),
                        column("b0", &[0.3, 0.4, 0.5]),
                        column("b1", &[0.6, 0.7, 0.8]),
                    ],
                    record: None,
                    ragged: vec![],
                    edit: |_| {},
                    series: series(
                        "image",
                        "ix",
                        "iy",
                        DataRenderType::RgbImage {
                            red: matrix(&["r0", "r1"], GridLayout::Centers),
                            green: matrix(&["g0", "g1"], GridLayout::Centers),
                            blue: matrix(&["b0", "b1"], GridLayout::Centers),
                            range: [0.0, 1.0],
                            shading: Shading::Flat,
                            opacity: 1.0,
                        },
                    ),
                    pick: cell("image", 1, 2),
                    keys: keys(&["x", "y", "red", "green", "blue"]),
                },
            ]
        }

        /// A chart drawing `scene`: its columns, record and ragged columns
        /// registered through the JS API, then its config and series set.
        async fn scene_chart(scene: &SelectionScene) -> FiggyChart {
            let mut chart = FiggyChart::create(canvas()).await.expect("create chart");
            for (id, data) in &scene.columns {
                chart
                    .register_column_f64(id, data)
                    .unwrap_or_else(|e| panic!("{}: column {id}: {e:?}", scene.name));
            }
            if let Some((id, fields)) = &scene.record {
                let object = js_sys::Object::new();
                for (name, data) in fields {
                    js_sys::Reflect::set(
                        &object,
                        &JsValue::from_str(name),
                        &js_sys::Float64Array::from(&data[..]).into(),
                    )
                    .unwrap();
                }
                chart
                    .register_record(id, object.into())
                    .unwrap_or_else(|e| panic!("{}: record: {e:?}", scene.name));
            }
            for (id, values, offsets) in &scene.ragged {
                chart
                    .register_ragged(id, values, offsets)
                    .unwrap_or_else(|e| panic!("{}: ragged: {e:?}", scene.name));
            }
            let mut config = chart.chart_config().unwrap().clone();
            (scene.edit)(&mut config);
            chart
                .set_config(&serde_json::to_string(&config).unwrap())
                .unwrap_or_else(|e| panic!("{}: config: {e:?}", scene.name));
            chart
                .set_series(&serde_json::to_string(&[scene.series.clone()]).unwrap())
                .unwrap_or_else(|e| panic!("{}: series: {e:?}", scene.name));
            chart
        }

        /// SELECTION_READBACK_LEN (#152): `read_selection_samples` answers
        /// one value per sample, `values[k]` the registered value of
        /// `samples[k]`, for none, one and 64 samples over two columns (past
        /// `MAX_RECORD_FIELDS` and the seven range bar roles); a sample past
        /// its column or of a missing column is an error.
        #[wasm_bindgen_test(async)]
        async fn selection_readback_answers_one_value_per_sample() {
            const LEN: usize = 32;
            let mut chart = FiggyChart::create(canvas()).await.expect("create chart");
            let a: Vec<f64> = (0..LEN).map(|i| i as f64 * 0.25 - 3.0).collect();
            let b: Vec<f64> = (0..LEN).map(|i| 1000.0 + i as f64 * 0.5).collect();
            chart.register_column_f64("ra", &a).unwrap();
            chart.register_column_f64("rb", &b).unwrap();
            for count in [0usize, 1, 64] {
                let samples: Vec<(String, usize)> = (0..count)
                    .map(|k| {
                        let column = if k % 2 == 0 { "ra" } else { "rb" };
                        (column.to_string(), (k * 7) % LEN)
                    })
                    .collect();
                let values = chart.read_selection_samples(&samples).await.unwrap();
                assert_eq!(values.len(), samples.len(), "{count} samples");
                for (k, (column, index)) in samples.iter().enumerate() {
                    let registered = if column == "ra" { a[*index] } else { b[*index] };
                    assert_eq!(values[k], registered, "sample {k} of {count}");
                }
            }
            assert!(
                chart
                    .read_selection_samples(&[("ra".into(), LEN)])
                    .await
                    .is_err()
            );
            assert!(
                chart
                    .read_selection_samples(&[("missing".into(), 0)])
                    .await
                    .is_err()
            );
        }

        /// SELECTION_READBACK_LEN and BOX_RECORD_SCHEMA_FITS (#152): every
        /// branch of `selection_details` indexes the readback of the samples
        /// it built, from a box record with all nine fields or the five
        /// required ones to a violin without boxes (no sample at all),
        /// twenty parallel axes and every range bar role. Each answers JSON
        /// with its keys; the box statistics and range bar roles are the
        /// registered values of the picked row.
        #[wasm_bindgen_test(async)]
        async fn selection_details_answers_every_branch() {
            for scene in selection_scenes() {
                let mut chart = scene_chart(&scene).await;
                let mut config = chart.chart_config().unwrap().clone();
                config.picked_data = Some(renderer::config::DataSelectionsConfig {
                    refs: vec![scene.pick.clone()],
                    ..Default::default()
                });
                chart
                    .set_config(&serde_json::to_string(&config).unwrap())
                    .unwrap_or_else(|e| panic!("{}: pick: {e:?}", scene.name));
                let json = chart
                    .selection_details()
                    .await
                    .unwrap_or_else(|e| panic!("{}: details: {e:?}", scene.name));
                let details: serde_json::Value = serde_json::from_str(&json).unwrap();
                for key in &scene.keys {
                    assert!(
                        details.get(key).is_some(),
                        "{}: no {key} in {details}",
                        scene.name
                    );
                }
                // Row 1 of a record field `k` holds `k + 1`.
                if let Some((_, fields)) = &scene.record {
                    for (k, (name, _)) in fields.iter().enumerate() {
                        assert_eq!(details[*name], k as f64 + 1.0, "{}: {name}", scene.name);
                    }
                    for (name, _) in renderer::BOX_PLOT_RECORD {
                        if !fields.iter().any(|(field, _)| field == name) {
                            assert!(details.get(*name).is_none(), "{}: {name}", scene.name);
                        }
                    }
                }
                if scene.name.starts_with("range bars") {
                    for (role, value) in [
                        ("position", 1.0),
                        ("end", 5.0),
                        ("start", 2.0),
                        ("low", 1.5),
                        ("high", 5.5),
                        ("state", 1.0),
                        ("progress", 0.5),
                    ] {
                        assert_eq!(details[role], value, "{role}");
                    }
                }
                if scene.name.starts_with("parallel") {
                    assert_eq!(details["values"].as_array().map(Vec::len), Some(20));
                }
                if scene.name.starts_with("3D point vector") {
                    for (key, value) in [("z", 0.4), ("u", 0.0), ("v", 0.1), ("w", 0.3)] {
                        assert_eq!(details[key], value, "{key}");
                    }
                }
            }
        }

        const CATEGORICAL_DOCUMENT: &str = r#"{
            "categories": [
                {"id": "q1", "label": "Q1"},
                {"id": "q2", "label": "Q2"},
                {"id": "q3", "label": "Q3"}
            ],
            "series": [
                {"id": "north", "label": "North", "values": [12.0, 18.0, 9.0],
                 "color": {"r": 0.33, "g": 0.56, "b": 0.83, "a": 1.0}},
                {"id": "south", "label": "South", "values": [18.0, null, 14.0],
                 "color": {"r": 1.0, "g": 0.65, "b": 0.41, "a": 1.0}}
            ],
            "mode": "Stacked",
            "value_title": "Revenue"
        }"#;

        const BOXPLOT_DOCUMENT: &str = r#"{
            "categories": [
                {"id": "ctl", "label": "Control"},
                {"id": "low", "label": "Low"},
                {"id": "high", "label": "High"}
            ],
            "series": [
                {"id": "a", "label": "A", "color": {"r": 0.25, "g": 0.56, "b": 0.82, "a": 1.0},
                 "values": [
                    {"whisker_low": 1, "q1": 2, "median": 3, "q3": 4, "whisker_high": 5,
                     "mean": 3.2, "sample_count": 40, "outliers": [9.0]},
                    null,
                    {"whisker_low": 2, "q1": 3, "median": 4, "q3": 5, "whisker_high": 6}
                 ]},
                {"id": "b", "label": "B", "color": {"r": 0.9, "g": 0.66, "b": 0.25, "a": 1.0},
                 "values": [
                    {"whisker_low": 0, "q1": 1, "median": 2, "q3": 3, "whisker_high": 4},
                    {"whisker_low": 1, "q1": 2, "median": 2.5, "q3": 3, "whisker_high": 4},
                    {"whisker_low": 1, "q1": 2, "median": 3, "q3": 4, "whisker_high": 5}
                 ]}
            ],
            "value_title": "Response"
        }"#;

        const RADAR_DOCUMENT: &str = r#"{
            "categories": [
                {"id": "speed", "label": "Speed"},
                {"id": "range", "label": "Range"},
                {"id": "cost", "label": "Cost"},
                {"id": "comfort", "label": "Comfort"}
            ],
            "series": [
                {"id": "a", "label": "A", "values": [4, 3.5, 2, 4.5],
                 "color": {"r": 0.25, "g": 0.56, "b": 0.82, "a": 1.0}},
                {"id": "b", "label": "B", "values": [3, 5, 3.25, 3],
                 "color": {"r": 0.9, "g": 0.66, "b": 0.25, "a": 1.0}}
            ],
            "selected": {"series_id": "a", "category_id": "range"}
        }"#;

        /// Remove every registered column of `chart` in turn.
        fn remove_every_column(chart: &mut FiggyChart, at: &str) {
            let mut ids = chart
                .renderer
                .logical_column_ids()
                .map(str::to_string)
                .collect::<Vec<_>>();
            ids.sort();
            for id in ids {
                chart
                    .remove_column(&id)
                    .unwrap_or_else(|error| panic!("{at}: remove_column {id}: {error:?}"));
            }
        }

        /// Every call that changes the series or their labels either commits
        /// or is rejected whole, and the chart's series and legend live only
        /// in the renderer (#152, #160): line upserts, label edits,
        /// series removal, `set_series` with 0, 1 and 200 series, the three
        /// document loads, `load_demo` twice, and the removal of each column
        /// a series of every kind reads (a line's, a colour or size map's,
        /// a ternary component, a box plot's positions and violin density,
        /// a range bar's extra columns, a matrix grid column, a vector
        /// component), its record and ragged columns.
        #[wasm_bindgen_test(async)]
        async fn every_series_change_commits_or_rejects_whole() {
            let mut chart = FiggyChart::create(canvas()).await.expect("create chart");
            chart.register_column_f64("lx", &[0.0, 1.0, 2.0]).unwrap();
            chart.register_column_f64("ly", &[2.0, 3.0, 4.0]).unwrap();
            chart.add_line_series("a", "lx", "ly", 2.0, "A").unwrap();
            chart.add_line_series("a", "lx", "ly", 3.0, "").unwrap();
            chart.add_line_series("b", "lx", "ly", 2.0, "B").unwrap();
            assert!(
                chart
                    .add_line_series("c", "lx", "missing", 2.0, "C")
                    .is_err()
            );
            assert!(chart.set_series_label("a", "Alpha").unwrap());
            assert!(chart.set_series_label("a", "").unwrap());
            assert!(!chart.set_series_label("nope", "x").unwrap());
            assert!(chart.remove_series("b").unwrap());
            assert!(!chart.remove_series("nope").unwrap());

            let lines = |ids: &mut dyn Iterator<Item = String>| -> String {
                let series: Vec<SeriesConfig> = ids
                    .map(|id| SeriesConfig {
                        series_id: id,
                        source_id: None,
                        label: None,
                        x_column: "lx".into(),
                        y_column: "ly".into(),
                        z_column: None,
                        render_type: DataRenderType::Xy(renderer::data_config::XySeries {
                            scatter: None,
                            line: Some(DataLineStyleConfig::default()),
                            errors: None,
                            stem: None,
                        }),
                    })
                    .collect();
                serde_json::to_string(&series).unwrap()
            };
            for count in [0usize, 1, 200] {
                chart
                    .set_series(&lines(&mut (0..count).map(|i| format!("s{i}"))))
                    .unwrap();
            }
            // Survivors keep their labels, new ids join.
            chart
                .set_series(&lines(
                    &mut (0..100)
                        .map(|i| format!("s{i}"))
                        .chain((0..10).map(|i| format!("t{i}"))),
                ))
                .unwrap();
            assert!(chart.set_series("not json").is_err());
            let mut unknown: Vec<SeriesConfig> =
                serde_json::from_str(&chart.get_series().unwrap()).unwrap();
            unknown[0].y_column = "missing".into();
            assert!(
                chart
                    .set_series(&serde_json::to_string(&unknown).unwrap())
                    .is_err()
            );
            for _ in 0..2 {
                chart.load_demo().unwrap();
            }

            chart
                .set_categorical_document(CATEGORICAL_DOCUMENT)
                .unwrap();
            assert!(chart.set_categorical_document("{").is_err());
            remove_every_column(&mut chart, "categorical document");
            chart.set_boxplot_document(BOXPLOT_DOCUMENT).unwrap();
            assert!(chart.set_boxplot_document("{").is_err());
            remove_every_column(&mut chart, "box plot document");
            chart.set_radar_document(RADAR_DOCUMENT).unwrap();
            assert!(chart.set_radar_document("{").is_err());
            remove_every_column(&mut chart, "radar document");
            assert!(!chart.remove_column("missing").unwrap());

            // Each column a series of every kind reads, removed and put
            // back: the series goes with it and comes back with it.
            for scene in selection_scenes() {
                let mut chart = scene_chart(&scene).await;
                let series_json = serde_json::to_string(&[scene.series.clone()]).unwrap();
                for (id, data) in &scene.columns {
                    assert!(chart.remove_column(id).unwrap(), "{}: {id}", scene.name);
                    assert!(
                        chart.chart_series().unwrap().is_empty(),
                        "{}: a series reading {id} stays",
                        scene.name
                    );
                    chart.register_column_f64(id, data).unwrap();
                    chart.set_series(&series_json).unwrap();
                }
                let composites = scene
                    .record
                    .iter()
                    .map(|(id, _)| *id)
                    .chain(scene.ragged.iter().map(|(id, _, _)| *id));
                for id in composites {
                    // A record or ragged id is not a registry column.
                    let _removed = chart.remove_column(id);
                }
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::*;

#[cfg(target_arch = "wasm32")]
mod radial;
