//! Legend model — the whole legend is **one rich inline document**
//! ([`Legend::content`]).
//!
//! Design points:
//! - **Symbols are fixed-width rich segments.** A line sample is a drawn
//!   horizontal rule whose em dash text is only a fallback, while a scatter
//!   sample is a geometric-shape glyph (`●`, `■`, …) centered in its field.
//!   Each symbol form occupies the same 2 em total advance and carries a
//!   per-segment `RichSegment.color` override, so marks line up and use the
//!   same layout path on screen and in PNG export.
//! - **Everything is explicit in the SSoT.** Line breaks are `'\n'` segments,
//!   and symbols remain inline segment sequences, so a one-line legend,
//!   mid-text symbols, and any custom arrangement are all explicit.
//! - **Font and font size are live.** `content.font` / `content.font_size`
//!   (and per-segment overrides) are consumed at draw time, so SSoT edits
//!   after composition still apply.
//!
//! Composition helpers cover the common cases: [`symbol_segments`] /
//! [`series_symbol_segments`] build a colored symbol, [`append_legend_entry`]
//! appends "fixed-width symbol + space + tab + label" as a new line. Fully
//! custom layouts edit `content.segments` directly.

use crate::color::Color;
use crate::colormap::ColorMap;
use crate::data_config::{DataRenderType, ScatterShape, SeriesConfig};
use crate::line::LineStylePreset;
use crate::text::{CharSet, RichSegment, RichText, rich_segments_from_text};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LegendCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Sample kinds for the convenience builders (mirrors what a series draws).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LegendEntryKind {
    /// Line sample only.
    Line,
    /// Point sample only.
    Scatter,
    /// Line + point sample.
    LineScatter,
}

/// The compact legend character for a [`ScatterShape`]. Exact geometric
/// glyphs are used only when the bundled Liberation Sans face covers them;
/// pentagon/hexagon/octagon fall back to familiar marker-code letters.
pub fn scatter_shape_char(shape: &ScatterShape) -> char {
    match shape {
        ScatterShape::Circle => '○',
        ScatterShape::CircleFilled => '●',
        ScatterShape::Square => '□',
        ScatterShape::SquareFilled => '■',
        ScatterShape::Triangle => '▲',
        ScatterShape::TriangleFilled => '▲',
        ScatterShape::TriangleDown => '▼',
        ScatterShape::TriangleDownFilled => '▼',
        ScatterShape::TriangleLeft => '◄',
        ScatterShape::TriangleLeftFilled => '◄',
        ScatterShape::TriangleRight => '►',
        ScatterShape::TriangleRightFilled => '►',
        ScatterShape::Diamond => '◊',
        ScatterShape::DiamondFilled => '♦',
        ScatterShape::Cross => '×',
        ScatterShape::CrossFilled => '×',
        ScatterShape::Plus => '+',
        ScatterShape::PlusFilled => '+',
        ScatterShape::Pentagon => 'p',
        ScatterShape::PentagonFilled => 'p',
        ScatterShape::Hexagon => 'h',
        ScatterShape::HexagonFilled => 'h',
        ScatterShape::Octagon => '8',
        ScatterShape::OctagonFilled => '8',
        ScatterShape::Star => '*',
        ScatterShape::StarFilled => '*',
    }
}

/// Every symbol form spans exactly this many em (× legend font size), so
/// marks line up regardless of shape: a lone line, a lone glyph, and a
/// line-through-glyph all advance the pen by the same fixed field.
pub const SYMBOL_FIELD_EM: f32 = 2.0;
const LEGEND_DASH_NOMINAL_FONT_PX: f32 = 14.0;

fn rule_dash_em(style: LineStylePreset) -> Option<Vec<f32>> {
    let pattern = style.pattern();
    if pattern.is_empty() {
        None
    } else {
        Some(
            pattern
                .iter()
                .map(|v| v / LEGEND_DASH_NOMINAL_FONT_PX)
                .collect(),
        )
    }
}

#[cfg(test)]
mod legend_update_tests {
    use super::*;
    use crate::data_config::DataLineStyleConfig;
    use crate::line::LineStylePreset;
    use crate::text::TextAlign;

    fn line_cfg(color: Color, line_style: LineStylePreset) -> SeriesConfig {
        SeriesConfig {
            series_id: "s".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type: DataRenderType::Xy(crate::data_config::XySeries {
                scatter: None,
                line: Some(DataLineStyleConfig {
                    line_style,
                    line_color: color,
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                }),
                errors: None,
                stem: None,
            }),
        }
    }

    fn empty_content() -> RichText {
        RichText {
            segments: Vec::new(),
            color: Color::BLACK,
            font_size: 14.0,
            font: String::new(),
            align: TextAlign::Left,
        }
    }

    #[test]
    fn dashed_series_symbol_carries_rule_dash() {
        let red = Color::new(1.0, 0.0, 0.0, 1.0);
        let segs = series_symbol_segments(&line_cfg(red, LineStylePreset::Dash), None);
        assert_eq!(segs.len(), 1);
        assert!(segs[0].rule);
        assert_eq!(segs[0].color, Some(red));
        assert_eq!(
            segs[0].rule_dash.as_deref(),
            Some(&[8.0 / 14.0, 4.0 / 14.0][..])
        );
    }

    #[test]
    fn symbol_update_preserves_user_text_and_label_newlines() {
        let red = Color::new(1.0, 0.0, 0.0, 1.0);
        let blue = Color::new(0.0, 0.0, 1.0, 1.0);
        let green = Color::new(0.0, 0.8, 0.0, 1.0);
        let mut content = empty_content();
        append_legend_entry(
            &mut content,
            series_symbol_segments(&line_cfg(red, LineStylePreset::Solid), None),
            "line 1\nline 2",
        );
        append_legend_entry(
            &mut content,
            series_symbol_segments(&line_cfg(blue, LineStylePreset::Solid), None),
            "beta",
        );

        let series = [
            line_cfg(green, LineStylePreset::Dash),
            line_cfg(red, LineStylePreset::Dot),
        ];
        assert_eq!(
            update_legend_symbols_preserving_text(&mut content, &series, None),
            2
        );
        let text: String = content.segments.iter().map(|s| s.text).collect();

        assert_eq!(legend_entry_count(&content), 2);
        assert!(text.contains("line 1\nline 2"));
        assert!(text.contains("beta"));
        assert_eq!(content.segments[0].color, Some(green));
        assert!(content.segments[0].rule_dash.is_some());
    }

    #[test]
    fn label_update_and_remove_touch_only_target_entry() {
        let mut content = empty_content();
        append_legend_entry(
            &mut content,
            series_symbol_segments(&line_cfg(Color::BLACK, LineStylePreset::Solid), None),
            "alpha",
        );
        append_legend_entry(
            &mut content,
            series_symbol_segments(&line_cfg(Color::BLACK, LineStylePreset::Solid), None),
            "beta",
        );

        set_legend_entry_label(
            &mut content,
            1,
            series_symbol_segments(&line_cfg(Color::BLACK, LineStylePreset::Solid), None),
            rich_segments_from_text("gamma"),
        );
        let text: String = content.segments.iter().map(|s| s.text).collect();
        assert!(text.contains("alpha"));
        assert!(text.contains("gamma"));
        assert!(!text.contains("beta"));

        assert!(remove_legend_entry(&mut content, 0));
        let text: String = content.segments.iter().map(|s| s.text).collect();
        assert!(!text.contains("alpha"));
        assert!(text.contains("gamma"));
        assert_eq!(legend_entry_count(&content), 1);
    }

    #[test]
    fn custom_legend_without_auto_entry_shape_is_left_untouched() {
        let mut content = RichText::plain("custom legend text", Color::BLACK, 14.0, "");
        let before = content.clone();
        assert_eq!(
            update_legend_symbols_preserving_text(
                &mut content,
                &[line_cfg(Color::BLACK, LineStylePreset::Solid)],
                None,
            ),
            0
        );
        assert_eq!(content, before);
        assert!(!remove_legend_entry(&mut content, 0));
    }
}

/// Line sample: a drawn rule filling the whole symbol field — exact length,
/// unlike any dash glyph. `band` paints the fill colour under the rule, inside
/// the same fixed field, for a series that fills the area under its line.
fn line_symbol(color: Color, style: LineStylePreset, band: Option<Color>) -> Vec<RichSegment> {
    let seg = match rule_dash_em(style) {
        Some(dash) => RichSegment::dashed_rule(SYMBOL_FIELD_EM, Some(color), dash),
        None => RichSegment::rule(SYMBOL_FIELD_EM, Some(color)),
    };
    vec![seg.with_rule_band(band)]
}

/// Point sample: the shape's glyph centered in the symbol field.
fn scatter_symbol(shape: &ScatterShape, color: Color) -> Vec<RichSegment> {
    vec![RichSegment::fielded(
        scatter_shape_char(shape),
        SYMBOL_FIELD_EM,
        Some(color),
    )]
}

/// Line + point sample: rule — glyph — rule, the three fields summing to
/// exactly [`SYMBOL_FIELD_EM`] so the combined mark is as long as a lone
/// line mark.
fn line_scatter_symbol(shape: &ScatterShape, color: Color) -> Vec<RichSegment> {
    line_scatter_symbol_with_style(shape, color, LineStylePreset::Solid, None)
}

fn line_scatter_symbol_with_style(
    shape: &ScatterShape,
    color: Color,
    style: LineStylePreset,
    band: Option<Color>,
) -> Vec<RichSegment> {
    let glyph_em = 0.7;
    let rule_em = (SYMBOL_FIELD_EM - glyph_em) * 0.5;
    let rule = |width_em| {
        match rule_dash_em(style) {
            Some(dash) => RichSegment::dashed_rule(width_em, Some(color), dash),
            None => RichSegment::rule(width_em, Some(color)),
        }
        .with_rule_band(band)
    };
    vec![
        rule(rule_em),
        RichSegment::fielded(scatter_shape_char(shape), glyph_em, Some(color)),
        rule(rule_em),
    ]
}

/// Arrow sample: a rule for the shaft and the right-pointing head glyph, the
/// two fields summing to exactly [`SYMBOL_FIELD_EM`] like the other marks.
fn vector_symbol(color: Color) -> Vec<RichSegment> {
    let glyph_em = 0.6;
    vec![
        RichSegment::rule(SYMBOL_FIELD_EM - glyph_em, Some(color)),
        RichSegment::fielded(
            scatter_shape_char(&ScatterShape::TriangleRightFilled),
            glyph_em,
            Some(color),
        ),
    ]
}

/// Symbol segments for the convenience-builder [`LegendEntryKind`]: each
/// segment carries `color: Some(color)` so the mark keeps the series color
/// independent of the document-level text color.
pub fn symbol_segments(kind: &LegendEntryKind, color: Color) -> Vec<RichSegment> {
    match kind {
        LegendEntryKind::Line => line_symbol(color, LineStylePreset::Solid, None),
        LegendEntryKind::Scatter => scatter_symbol(&ScatterShape::CircleFilled, color),
        LegendEntryKind::LineScatter => line_scatter_symbol(&ScatterShape::CircleFilled, color),
    }
}

/// Blank mark of exactly [`SYMBOL_FIELD_EM`] — a label-only legend row.
///
/// Used for a heatmap when no colour map is known: its colours are a ramp,
/// not a single swatch. It is a spacer rather than an empty vec because every series
/// must still produce exactly one entry: legend entry `i` **is** series `i`
/// (`update_legend_symbols_preserving_text` zips the two positionally, and the
/// web layer passes series indices to `set_legend_entry_label`), so a skipped
/// entry would re-point every later row's symbol at the wrong series. Full field
/// width also keeps the tab-aligned symbol column the same width on every row.
fn blank_symbol() -> Vec<RichSegment> {
    vec![RichSegment::fielded(' ', SYMBOL_FIELD_EM, None)]
}

/// Steps of a colour map swatch across the symbol field.
const COLORMAP_SWATCH_STEPS: usize = 6;

/// Colour map swatch: the map's ramp, low to high, as equal flat bands
/// filling [`SYMBOL_FIELD_EM`], each band a rule over a fill band of its
/// colour. The colours are the colourbar's, not dimmed by a fill opacity, so
/// the row reads as "this series is drawn with that map".
fn colormap_symbol(map: &ColorMap) -> Vec<RichSegment> {
    let width = SYMBOL_FIELD_EM / COLORMAP_SWATCH_STEPS as f32;
    (0..COLORMAP_SWATCH_STEPS)
        .map(|step| {
            let color = map.sample((step as f32 + 0.5) / COLORMAP_SWATCH_STEPS as f32);
            RichSegment::rule(width, Some(color)).with_rule_band(Some(color))
        })
        .collect()
}

/// Symbol segments matching a series declaration: line / scatter / combined,
/// shape and color taken from the render type's sub-styles.
///
/// A line with a fill carries a band of the fill colour under its rule
/// ([`DataLineStyleConfig::effective_fill_color`]), inside the same fixed
/// symbol field. A heatmap shows a swatch of `colormap`, the colour map that
/// draws it (its chart's colourbar map); without one the mark is blank.
///
/// [`DataLineStyleConfig::effective_fill_color`]: crate::data_config::DataLineStyleConfig::effective_fill_color
pub fn series_symbol_segments(cfg: &SeriesConfig, colormap: Option<&ColorMap>) -> Vec<RichSegment> {
    let mut symbol = series_mark(cfg, colormap);
    // The symbol names its series, so updates pair it by id (`series_id`).
    for segment in &mut symbol {
        segment.series_id = Some(cfg.series_id.clone());
    }
    symbol
}

/// The mark that stands for `cfg`, before it is tagged with its series.
fn series_mark(cfg: &SeriesConfig, colormap: Option<&ColorMap>) -> Vec<RichSegment> {
    match &cfg.render_type {
        DataRenderType::Xy(crate::data_config::XySeries {
            scatter: None,
            line: Some(line),
            ..
        }) => line_symbol(line.line_color, line.line_style, line_fill_band(line)),
        // Neither markers nor a line: nothing is drawn, so nothing stands
        // for it (`XySeries::validate` refuses it).
        DataRenderType::Xy(crate::data_config::XySeries {
            scatter: None,
            line: None,
            ..
        }) => blank_symbol(),
        // A bar reads as a filled swatch; the glyph is already in the shape
        // table, so no new symbol machinery.
        DataRenderType::Histogram { bar } => {
            scatter_symbol(&ScatterShape::SquareFilled, bar.fill_color)
        }
        // Contour lines are lines. When a fill is under them the line is still
        // the mark — the fill's colours belong to the colourbar.
        DataRenderType::Field(crate::data_config::FieldSeries {
            contour: Some(contour),
            ..
        }) => line_symbol(contour.line.line_color, contour.line.line_style, None),
        DataRenderType::Field(crate::data_config::FieldSeries { contour: None, .. }) => {
            colormap.map_or_else(blank_symbol, colormap_symbol)
        }
        // A mesh reads as a field (#210): its contour line, or the
        // colourbar's ramp under a fill alone.
        DataRenderType::TriMesh(crate::tri_mesh::TriMeshSeries {
            contour: Some(contour),
            ..
        }) => line_symbol(contour.line.line_color, contour.line.line_style, None),
        DataRenderType::TriMesh(crate::tri_mesh::TriMeshSeries { contour: None, .. }) => {
            colormap.map_or_else(blank_symbol, colormap_symbol)
        }
        // An image's colours are its data: no mark stands for them.
        DataRenderType::RgbImage { .. } => blank_symbol(),
        // A level reads as its stroke (#203).
        DataRenderType::EnergyLevels(levels) => {
            line_symbol(levels.style.level.color, LineStylePreset::Solid, None)
        }
        // A diagram reads as its line (#234).
        DataRenderType::Feynman(diagram) => {
            line_symbol(diagram.style.color, LineStylePreset::Solid, None)
        }
        // A monotonic channel reads as its line (#271).
        DataRenderType::MonotonicLine(series) => {
            line_symbol(series.line.line_color, series.line.line_style, None)
        }
        // An arrow: a rule for the shaft and the right-pointing head glyph,
        // in the series colour. A colour map's colours belong to the colourbar.
        DataRenderType::Vector { style, .. } => vector_symbol(style.color),
        // Streamlines read as their line; with arrowheads, as an arrow in
        // the line colour. A colour map's colours belong to the colourbar.
        DataRenderType::Streamlines { line, arrows, .. } => match arrows {
            Some(_) => vector_symbol(line.line_color),
            None => line_symbol(line.line_color, line.line_style, None),
        },
        // A parallel series reads as its rows' line; a colour column's
        // colours belong to the colourbar.
        DataRenderType::Parallel { line, .. } => {
            line_symbol(line.line_color, line.line_style, None)
        }
        // A category bar is a filled swatch like a histogram bar.
        DataRenderType::CategoryBars { color, .. } => {
            scatter_symbol(&ScatterShape::SquareFilled, *color)
        }
        // A box is a filled swatch in the box colour, like a bar.
        DataRenderType::BoxPlot { color, .. } => {
            scatter_symbol(&ScatterShape::SquareFilled, *color)
        }
        // A range bar is a filled swatch in the series colour, like a bar;
        // direction and state colours are read from the bars.
        DataRenderType::RangeBars { color, .. } => {
            scatter_symbol(&ScatterShape::SquareFilled, *color)
        }
        // A radar series reads as the line it is drawn with: its outline
        // over its fill band, with its vertex marker when it has one.
        DataRenderType::Radar {
            color,
            style,
            labels,
        } => {
            let radar = crate::data_config::RadarRef {
                color: *color,
                style,
                labels,
            };
            let line = radar.line_style();
            let band = line_fill_band(&line);
            match radar.scatter_style() {
                Some(scatter) => line_scatter_symbol_with_style(
                    &scatter.point_shape,
                    radar.color,
                    LineStylePreset::Solid,
                    band,
                ),
                None => line_symbol(radar.color, LineStylePreset::Solid, band),
            }
        }
        DataRenderType::Xy(crate::data_config::XySeries {
            scatter: Some(scatter),
            line: None,
            ..
        }) => scatter_symbol(&scatter.point_shape, scatter.point_color),
        DataRenderType::Xy(crate::data_config::XySeries {
            scatter: Some(scatter),
            line: Some(line),
            ..
        }) => line_scatter_symbol_with_style(
            &scatter.point_shape,
            line.line_color,
            line.line_style,
            line_fill_band(line),
        ),
        // A ternary series reads as what it draws: markers, a line, or both,
        // over its region fill's band when it has one; a fill alone is a
        // band under an invisible rule.
        DataRenderType::Ternary {
            scatter,
            line,
            region,
            ..
        } => {
            let band = region.as_ref().map(|region| {
                let [r, g, b, a] = region.color_premul();
                if a > 0.0 {
                    Color::new(r / a, g / a, b / a, a)
                } else {
                    Color::new(0.0, 0.0, 0.0, 0.0)
                }
            });
            match (scatter, line) {
                (Some(scatter), Some(line)) => line_scatter_symbol_with_style(
                    &scatter.point_shape,
                    line.line_color,
                    line.line_style,
                    band,
                ),
                (Some(scatter), None) => scatter_symbol(&scatter.point_shape, scatter.point_color),
                (None, Some(line)) => line_symbol(line.line_color, line.line_style, band),
                (None, None) => match band {
                    Some(band) => {
                        vec![
                            RichSegment::rule(SYMBOL_FIELD_EM, Some(band))
                                .with_rule_band(Some(band)),
                        ]
                    }
                    None => blank_symbol(),
                },
            }
        }
    }
}

/// Append one "entry" to the legend document: a `'\n'` segment when content
/// already exists, then the symbol segments, one space, a `'\t'` column
/// separator, then the label (sub/superscript unicode and `'\n'` are
/// interpreted by the segment mapper).
///
/// The `'\t'` makes the symbol field a table column: the text engine sizes
/// each label column consistently across rows. The symbol sequence itself
/// always totals [`SYMBOL_FIELD_EM`], whether it is a rule, a centered glyph,
/// or rule + glyph + rule. Callers wanting a one-line legend can extend
/// `content.segments` themselves without the `'\n'`.
pub fn append_legend_entry(content: &mut RichText, symbol: Vec<RichSegment>, label: &str) {
    append_legend_entry_rich(content, symbol, rich_segments_from_text(label));
}

/// The legend band colour of a line series: its fill colour when it has a
/// fill, nothing otherwise.
fn line_fill_band(line: &crate::data_config::DataLineStyleConfig) -> Option<Color> {
    line.fill.as_ref().map(|_| line.effective_fill_color())
}

/// Append one legend entry using already-styled label segments.
pub fn append_legend_entry_rich(
    content: &mut RichText,
    symbol: Vec<RichSegment>,
    label: Vec<RichSegment>,
) {
    if !content.segments.is_empty() {
        content.segments.push(RichSegment::plain('\n'));
    }
    content.segments.extend(symbol);
    content.segments.push(RichSegment::plain(' '));
    content.segments.push(RichSegment::plain('\t'));
    content.segments.extend(label);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegendEntrySpan {
    symbol: Range<usize>,
    text: Range<usize>,
    remove: Range<usize>,
}

impl LegendEntrySpan {
    /// The series this entry's symbol names; `None` for a symbol that names
    /// none (a hand-made symbol, or one written before symbols named their
    /// series).
    fn series_id<'a>(&self, content: &'a RichText) -> Option<&'a str> {
        content.segments[self.symbol.start].series_id.as_deref()
    }
}

/// The series entry `position` stands for: the series its symbol names, or,
/// for a symbol that names none, the series at the same position (how every
/// entry was paired before symbols named their series).
fn entry_series<'s>(
    named: Option<&str>,
    position: usize,
    series: &'s [SeriesConfig],
) -> Option<&'s SeriesConfig> {
    match named {
        Some(id) => series.iter().find(|cfg| cfg.series_id == id),
        None => series.get(position),
    }
}

fn is_plain_space(seg: &RichSegment) -> bool {
    seg.text == ' '
        && !seg.bold
        && !seg.italic
        && !seg.underline
        && !seg.superscript
        && !seg.subscript
        && seg.char_set == CharSet::Text
        && seg.color.is_none()
        && seg.font_size.is_none()
        && seg.field_em.is_none()
        && !seg.rule
        && seg.rule_dash.is_none()
}

fn symbol_anchor(segments: &[RichSegment], start: usize) -> Option<(usize, usize)> {
    if start >= segments.len() || segments[start].field_em.is_none() {
        return None;
    }

    let mut i = start;
    while i < segments.len() && segments[i].field_em.is_some() {
        i += 1;
    }
    let symbol_end = i;
    while i < segments.len() && is_plain_space(&segments[i]) {
        i += 1;
    }
    (i < segments.len() && segments[i].text == '\t').then_some((symbol_end, i))
}

fn legend_entry_spans(content: &RichText) -> Vec<LegendEntrySpan> {
    let segments = &content.segments;
    let mut anchors = Vec::new();
    let mut i = 0;
    while i < segments.len() {
        if let Some((symbol_end, tab)) = symbol_anchor(segments, i) {
            anchors.push((i, symbol_end, tab));
            i = tab + 1;
        } else {
            i += 1;
        }
    }

    let mut spans = Vec::with_capacity(anchors.len());
    for (idx, &(symbol_start, symbol_end, tab)) in anchors.iter().enumerate() {
        let next_start = anchors.get(idx + 1).map(|(start, _, _)| *start);
        let mut text_end = next_start.unwrap_or(segments.len());
        if next_start.is_some() && text_end > tab + 1 && segments[text_end - 1].text == '\n' {
            text_end -= 1;
        }

        let remove_start = if symbol_start > 0 && segments[symbol_start - 1].text == '\n' {
            symbol_start - 1
        } else {
            symbol_start
        };
        let remove_end = if idx == 0 {
            next_start
                .filter(|&start| start > 0 && segments[start - 1].text == '\n')
                .unwrap_or(text_end)
        } else {
            text_end
        };

        spans.push(LegendEntrySpan {
            symbol: symbol_start..symbol_end,
            text: tab + 1..text_end,
            remove: remove_start..remove_end,
        });
    }
    spans
}

/// Number of auto-format legend entries recognized in a rich legend document.
pub fn legend_entry_count(content: &RichText) -> usize {
    legend_entry_spans(content).len()
}

/// Replace each recognized entry's symbol with its series' current mark and
/// keep every user text segment untouched. An entry is paired with the
/// series its symbol names ([`RichSegment::series_id`]), so series order and
/// series without an entry do not matter; a symbol that names no series is
/// paired by position. An entry whose series is gone keeps its symbol.
/// Custom layouts that do not expose the auto symbol+tab shape are left
/// unchanged. `colormap` is the chart's colourbar map, which a heatmap mark
/// shows ([`series_symbol_segments`]). Returns the number of symbols
/// replaced.
pub fn update_legend_symbols_preserving_text(
    content: &mut RichText,
    series: &[SeriesConfig],
    colormap: Option<&ColorMap>,
) -> usize {
    let spans = legend_entry_spans(content);
    let mut replaced = 0;
    // From the last entry, so a splice leaves earlier spans in place.
    for (position, span) in spans.into_iter().enumerate().rev() {
        let Some(cfg) = entry_series(span.series_id(content), position, series) else {
            continue;
        };
        let symbol = series_symbol_segments(cfg, colormap);
        content.segments.splice(span.symbol, symbol);
        replaced += 1;
    }
    replaced
}

/// The entry that stands for series `series_id`: the one whose symbol names
/// it, or, when no symbol names it, the entry at the series' position in
/// `series` if that entry's symbol names no series (a legend written before
/// symbols named their series). `None` when the series has no entry.
pub fn legend_entry_of_series(
    content: &RichText,
    series: &[SeriesConfig],
    series_id: &str,
) -> Option<usize> {
    let spans = legend_entry_spans(content);
    if let Some(index) = spans
        .iter()
        .position(|span| span.series_id(content) == Some(series_id))
    {
        return Some(index);
    }
    let position = series.iter().position(|cfg| cfg.series_id == series_id)?;
    spans
        .get(position)
        .filter(|span| span.series_id(content).is_none())
        .map(|_| position)
}

/// The series entry `entry_index`'s symbol names, if it names one.
pub fn legend_entry_series_id(content: &RichText, entry_index: usize) -> Option<&str> {
    legend_entry_spans(content)
        .get(entry_index)
        .and_then(|span| span.series_id(content))
}

/// Set one recognized legend entry label while preserving the surrounding
/// document. If the entry does not exist, append a new auto-format entry.
pub fn set_legend_entry_label(
    content: &mut RichText,
    entry_index: usize,
    symbol: Vec<RichSegment>,
    label: Vec<RichSegment>,
) {
    let spans = legend_entry_spans(content);
    if let Some(span) = spans.get(entry_index) {
        content.segments.splice(span.text.clone(), label);
    } else {
        append_legend_entry_rich(content, symbol, label);
    }
}

/// Remove one recognized auto-format legend entry. Returns false when the
/// document does not expose that entry shape.
pub fn remove_legend_entry(content: &mut RichText, entry_index: usize) -> bool {
    let spans = legend_entry_spans(content);
    let Some(span) = spans.get(entry_index) else {
        return false;
    };
    content.segments.drain(span.remove.clone());
    true
}

/// Legend box rendered in one corner of the chart's data area.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Legend {
    pub visible: bool,
    /// The whole legend as one rich document. `'\n'` segments break lines;
    /// symbol fields are fixed-width inline segments with color overrides.
    /// Line symbols draw rules across their fields and retain an em dash only
    /// as fallback text; scatter glyphs are centered in their fields. Thus
    /// line breaks, symbol positions, and mid-text symbols remain explicit in
    /// the SSoT. Font + font_size of this RichText are consumed at draw time.
    pub content: RichText,
    pub corner: LegendCorner,
    /// Visual offset from the corner anchor (drag-to-move lands here).
    pub offset_x: f32,
    pub offset_y: f32,
    /// Inset from the data_area inner corner + padding inside the box.
    pub padding: f32,
    /// Box background and border.
    pub bg_color: Color,
    pub border_color: Color,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_config::DataLineStyleConfig;
    use crate::line::LineStylePreset;
    use crate::text::TextAlign;

    fn line_cfg(color: Color) -> SeriesConfig {
        line_cfg_style(color, LineStylePreset::Solid)
    }

    fn line_cfg_style(color: Color, line_style: LineStylePreset) -> SeriesConfig {
        SeriesConfig {
            series_id: "s".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type: DataRenderType::Xy(crate::data_config::XySeries {
                scatter: None,
                line: Some(DataLineStyleConfig {
                    line_style,
                    line_color: color,
                    line_width: 1.0,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                }),
                errors: None,
                stem: None,
            }),
        }
    }

    fn empty_content() -> RichText {
        RichText {
            segments: Vec::new(),
            color: Color::BLACK,
            font_size: 14.0,
            font: String::new(),
            align: TextAlign::Left,
        }
    }

    #[test]
    fn series_symbol_takes_series_color_as_override() {
        let red = Color::new(1.0, 0.0, 0.0, 1.0);
        let segs = series_symbol_segments(&line_cfg(red), None);
        assert_eq!(segs.len(), 1); // "—"
        assert_eq!(segs[0].text, '—');
        assert_eq!(segs[0].color, Some(red));
        assert_eq!(segs[0].font_size, None); // size stays live via the document
    }

    // A filled line carries its fill colour as a band under the rule, and a
    // line without a fill carries none: the band is the legend's only
    // difference between the two, so the symbol field keeps its width.
    #[test]
    fn filled_line_symbols_carry_the_fill_band_under_every_rule() {
        use crate::data_config::{DataLineStyleConfig, DataScatterStyleConfig, LineFill};

        let red = Color::new(1.0, 0.0, 0.0, 1.0);
        let mut filled = line_cfg(red);
        let DataRenderType::Xy(crate::data_config::XySeries {
            scatter: None,
            line: Some(line),
            errors: None,
            ..
        }) = &mut filled.render_type
        else {
            unreachable!()
        };
        line.fill = Some(LineFill::ToBaseline(0.0));
        let segs = series_symbol_segments(&filled, None);
        assert_eq!(segs.len(), 1);
        assert!(segs[0].rule);
        assert_eq!(segs[0].color, Some(red));
        assert_eq!(
            segs[0].rule_band,
            Some(Color::new(
                1.0,
                0.0,
                0.0,
                DataLineStyleConfig::DEFAULT_FILL_ALPHA
            )),
            "the default band is the line colour at the default fill alpha"
        );
        assert_eq!(
            series_symbol_segments(&line_cfg(red), None)[0].rule_band,
            None
        );

        // An explicit fill colour wins, and a line + marker mark bands both
        // half rules while the glyph between them stays plain.
        let blue = Color::new(0.0, 0.0, 1.0, 0.5);
        let combined = SeriesConfig {
            render_type: DataRenderType::Xy(crate::data_config::XySeries {
                scatter: Some(DataScatterStyleConfig {
                    point_color: red,
                    point_shape: ScatterShape::CircleFilled,
                    point_size: 3.0,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                    point_color_map: None,
                    point_size_map: None,
                }),
                line: Some(DataLineStyleConfig {
                    fill: Some(LineFill::ToSeries("other".into())),
                    fill_color: Some(blue),
                    ..crate::defaults::data_line_style_config()
                }),
                errors: None,
                stem: None,
            }),
            ..line_cfg(red)
        };
        let segs = series_symbol_segments(&combined, None);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].rule_band, Some(blue));
        assert_eq!(segs[1].rule_band, None);
        assert_eq!(segs[2].rule_band, Some(blue));
    }

    #[test]
    fn scatter_symbols_map_every_shape_to_a_glyph() {
        for shape in [
            ScatterShape::Circle,
            ScatterShape::Square,
            ScatterShape::Triangle,
            ScatterShape::TriangleDown,
            ScatterShape::TriangleLeft,
            ScatterShape::TriangleRight,
            ScatterShape::Diamond,
            ScatterShape::Cross,
            ScatterShape::Plus,
            ScatterShape::Pentagon,
            ScatterShape::Hexagon,
            ScatterShape::Octagon,
            ScatterShape::Star,
            ScatterShape::CircleFilled,
            ScatterShape::SquareFilled,
            ScatterShape::TriangleFilled,
            ScatterShape::TriangleDownFilled,
            ScatterShape::TriangleLeftFilled,
            ScatterShape::TriangleRightFilled,
            ScatterShape::DiamondFilled,
            ScatterShape::CrossFilled,
            ScatterShape::PlusFilled,
            ScatterShape::PentagonFilled,
            ScatterShape::HexagonFilled,
            ScatterShape::OctagonFilled,
            ScatterShape::StarFilled,
        ] {
            let c = scatter_shape_char(&shape);
            assert!(!c.is_control(), "{shape:?} → {c}");
        }
    }

    #[test]
    fn kind_symbols_carry_color_override_on_every_segment() {
        let blue = Color::new(0.0, 0.0, 1.0, 1.0);
        for kind in [
            LegendEntryKind::Line,
            LegendEntryKind::Scatter,
            LegendEntryKind::LineScatter,
        ] {
            let segs = symbol_segments(&kind, blue);
            assert!(!segs.is_empty());
            assert!(segs.iter().all(|s| s.color == Some(blue)), "{kind:?}");
        }
    }

    #[test]
    fn two_appends_put_explicit_newline_between_entries() {
        let mut content = empty_content();
        append_legend_entry(
            &mut content,
            symbol_segments(&LegendEntryKind::Line, Color::BLACK),
            "a",
        );
        append_legend_entry(
            &mut content,
            symbol_segments(&LegendEntryKind::Line, Color::BLACK),
            "b",
        );

        let newlines = content.segments.iter().filter(|s| s.text == '\n').count();
        assert_eq!(newlines, 1);
        // Entry shape: symbol, space, '\t' column break, label — twice.
        // The tab gives every entry the same symbol-field width at draw time.
        let texts: String = content.segments.iter().map(|s| s.text).collect();
        assert_eq!(texts, "— \ta\n— \tb");
        // Labels inherit the document color (no override).
        assert!(
            content
                .segments
                .iter()
                .filter(|s| s.text == 'a')
                .all(|s| s.color.is_none())
        );
    }

    #[test]
    fn label_newlines_stay_explicit_in_content() {
        let mut content = empty_content();
        append_legend_entry(
            &mut content,
            symbol_segments(&LegendEntryKind::Line, Color::BLACK),
            "line 1\nline 2",
        );
        let newlines = content.segments.iter().filter(|s| s.text == '\n').count();
        assert_eq!(newlines, 1);
    }

    #[test]
    fn one_line_composition_when_caller_joins_without_newline() {
        // A fully one-line legend: the caller extends the document directly
        // instead of letting `append_legend_entry` insert the '\n'.
        let red = Color::new(1.0, 0.0, 0.0, 1.0);
        let mut content = empty_content();
        append_legend_entry(
            &mut content,
            symbol_segments(&LegendEntryKind::Line, red),
            "a",
        );
        content.segments.push(RichSegment::plain(' '));
        content
            .segments
            .extend(symbol_segments(&LegendEntryKind::Scatter, red));
        content.segments.push(RichSegment::plain(' '));
        content.segments.extend(rich_segments_from_text("b"));

        assert!(content.segments.iter().all(|s| s.text != '\n'));
        let texts: String = content.segments.iter().map(|s| s.text).collect();
        assert_eq!(texts, "— \ta ● b");
    }

    fn field_series(render_type: DataRenderType) -> SeriesConfig {
        SeriesConfig {
            series_id: "f".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            z_column: None,
            render_type,
        }
    }

    fn bar(color: Color) -> DataRenderType {
        DataRenderType::Histogram {
            bar: crate::data_config::DataBarStyleConfig {
                fill_color: color,
                border_color: Color::BLACK,
                border_width: 1.0,
                baseline: 0.0,
                gap_px: 1.0,
                width_ratio: 1.0,
                orientation: crate::data_config::BarOrientation::Vertical,
                bar_style_overrides: None,
            },
        }
    }

    fn contour(color: Color) -> crate::data_config::ContourConfig {
        crate::data_config::ContourConfig {
            levels: vec![1.0],
            line: DataLineStyleConfig {
                line_style: LineStylePreset::Dash,
                line_color: color,
                line_width: 1.0,
                fill: None,
                fill_color: None,
                interpolation: crate::data_config::LineInterpolation::Linear,
            },
            per_level_color: None,
            labels: None,
        }
    }

    fn matrix() -> crate::data_config::MatrixRef {
        crate::data_config::MatrixRef {
            columns: vec!["z0".into()],
            orientation: crate::data_config::MatrixOrientation::ColumnsAreX,
            grid_layout: crate::data_config::GridLayout::Centers,
        }
    }

    fn fill() -> crate::data_config::FieldFillConfig {
        crate::data_config::FieldFillConfig {
            mode: crate::data_config::FillMode::Continuous,
            shading: crate::data_config::Shading::Flat,
            opacity: 1.0,
        }
    }

    // A bar reads as a swatch, a contour as a line — in the colours those
    // primitives are actually drawn with.
    #[test]
    fn field_series_symbols_come_from_the_primitive_that_is_drawn() {
        let swatch = series_symbol_segments(&field_series(bar(Color::from_rgb8(1, 2, 3))), None);
        assert_eq!(swatch.len(), 1);
        assert_eq!(swatch[0].color, Some(Color::from_rgb8(1, 2, 3)));
        assert_eq!(
            swatch[0].text,
            scatter_shape_char(&crate::data_config::ScatterShape::SquareFilled)
        );

        let level_color = Color::from_rgb8(4, 5, 6);
        for render_type in [
            DataRenderType::Field(crate::data_config::FieldSeries {
                matrix: matrix(),
                fill: None,
                contour: Some(contour(level_color)),
                surface: None,
            }),
            DataRenderType::Field(crate::data_config::FieldSeries {
                matrix: matrix(),
                fill: Some(fill()),
                contour: Some(contour(level_color)),
                surface: None,
            }),
        ] {
            let mark = series_symbol_segments(&field_series(render_type), None);
            assert_eq!(mark.len(), 1, "a contour mark is one rule");
            assert_eq!(mark[0].color, Some(level_color));
            // Dashed, because the contour line is — the mark reports the style
            // the line is drawn with, not a generic rule.
            assert!(mark[0].rule, "a contour mark is a drawn rule");
            assert!(mark[0].rule_dash.is_some(), "dashed contour, dashed mark");
        }
    }

    // With its chart's colour map, a heatmap's mark is that map's ramp: equal
    // bands across the symbol field, low to high, in the colourbar's colours
    // whatever the fill opacity. The bands form one symbol, so the entry is
    // still one entry.
    #[test]
    fn a_heatmap_with_a_colormap_shows_a_swatch_of_it() {
        let heatmap = field_series(DataRenderType::Field(crate::data_config::FieldSeries {
            matrix: matrix(),
            fill: Some(crate::data_config::FieldFillConfig {
                opacity: 0.5,
                ..fill()
            }),
            contour: None,
            surface: None,
        }));
        let map = ColorMap::Viridis;
        let mark = series_symbol_segments(&heatmap, Some(&map));
        assert_eq!(mark.len(), COLORMAP_SWATCH_STEPS);
        let width: f32 = mark.iter().map(|seg| seg.field_em.unwrap()).sum();
        assert!((width - SYMBOL_FIELD_EM).abs() < 1e-5, "{width}");
        let low = map.sample(0.5 / COLORMAP_SWATCH_STEPS as f32);
        let high = map.sample(1.0 - 0.5 / COLORMAP_SWATCH_STEPS as f32);
        for (seg, expected) in [(mark.first(), low), (mark.last(), high)] {
            let seg = seg.unwrap();
            assert!(seg.rule);
            assert_eq!(seg.color, seg.rule_band);
            assert_eq!(seg.color, Some(expected));
        }

        let series = [line_cfg(Color::from_rgb8(10, 0, 0)), heatmap];
        let mut content = RichText {
            segments: Vec::new(),
            color: Color::BLACK,
            font_size: 12.0,
            font: String::new(),
            align: TextAlign::Left,
        };
        for cfg in &series {
            append_legend_entry(
                &mut content,
                series_symbol_segments(cfg, Some(&map)),
                "label",
            );
        }
        assert_eq!(legend_entry_count(&content), series.len());
        let before = content.clone();
        assert_eq!(
            update_legend_symbols_preserving_text(&mut content, &series, Some(&map)),
            series.len()
        );
        assert_eq!(content, before);
    }

    // Without a colour map a heatmap has no swatch to show. The
    // mark is blank — but it is still exactly one full-width segment, because
    // legend entry `i` is series `i` and the symbol column is tab-aligned across
    // rows. An empty vec would re-point every later row at the wrong series.
    #[test]
    fn a_heatmap_gets_a_blank_full_width_mark_not_no_entry() {
        let heatmap = field_series(DataRenderType::Field(crate::data_config::FieldSeries {
            matrix: matrix(),
            fill: Some(fill()),
            contour: None,
            surface: None,
        }));
        let mark = series_symbol_segments(&heatmap, None);
        assert_eq!(mark.len(), 1);
        assert_eq!(mark[0].field_em, Some(SYMBOL_FIELD_EM));
        assert_eq!(mark[0].text, ' ');

        // One entry per series, in series order, with the heatmap in the middle.
        let series = [
            line_cfg(Color::from_rgb8(10, 0, 0)),
            heatmap,
            field_series(bar(Color::from_rgb8(0, 10, 0))),
        ];
        let mut content = RichText {
            segments: Vec::new(),
            color: Color::BLACK,
            font_size: 12.0,
            font: String::new(),
            align: TextAlign::Left,
        };
        for cfg in &series {
            append_legend_entry(&mut content, series_symbol_segments(cfg, None), "label");
        }
        assert_eq!(legend_entry_count(&content), series.len());
        assert_eq!(
            update_legend_symbols_preserving_text(&mut content, &series, None),
            series.len(),
            "every series must own exactly one entry, or later symbols shift"
        );
    }

    fn named(id: &str, color: Color) -> SeriesConfig {
        SeriesConfig {
            series_id: id.into(),
            ..line_cfg(color)
        }
    }

    /// The colour of entry `i`'s symbol.
    fn entry_color(content: &RichText, i: usize) -> Option<Color> {
        legend_entry_spans(content)
            .get(i)
            .and_then(|span| content.segments[span.symbol.start].color)
    }

    fn entry_label(content: &RichText, i: usize) -> String {
        let span = &legend_entry_spans(content)[i];
        content.segments[span.text.clone()]
            .iter()
            .map(|seg| seg.text)
            .collect()
    }

    // The auto legend lists only the series that have a label. With an
    // unlabeled series first, the one entry is the second series', and a
    // restyle must give it the second series' symbol, not the first's.
    #[test]
    fn a_restyle_keeps_each_symbol_with_its_series_when_one_has_no_entry() {
        let red = Color::new(1.0, 0.0, 0.0, 1.0);
        let blue = Color::new(0.0, 0.0, 1.0, 1.0);
        let green = Color::new(0.0, 1.0, 0.0, 1.0);
        let unlabeled = named("a", red);
        let labeled = named("b", blue);
        let mut content = empty_content();
        append_legend_entry(&mut content, series_symbol_segments(&labeled, None), "B");

        let restyled = named("b", green);
        update_legend_symbols_preserving_text(&mut content, &[unlabeled, restyled], None);

        assert_eq!(legend_entry_count(&content), 1);
        assert_eq!(entry_label(&content, 0), "B");
        assert_eq!(entry_color(&content, 0), Some(green));
    }

    // Reordering the series does not move symbols between entries.
    #[test]
    fn a_reorder_keeps_each_symbol_with_its_series() {
        let red = Color::new(1.0, 0.0, 0.0, 1.0);
        let blue = Color::new(0.0, 0.0, 1.0, 1.0);
        let a = named("a", red);
        let b = named("b", blue);
        let mut content = empty_content();
        append_legend_entry(&mut content, series_symbol_segments(&a, None), "A");
        append_legend_entry(&mut content, series_symbol_segments(&b, None), "B");

        update_legend_symbols_preserving_text(&mut content, &[b, a], None);

        assert_eq!(entry_label(&content, 0), "A");
        assert_eq!(entry_color(&content, 0), Some(red));
        assert_eq!(entry_label(&content, 1), "B");
        assert_eq!(entry_color(&content, 1), Some(blue));
    }

    // A symbol names its series, and the entry is found by that name.
    #[test]
    fn an_entry_is_found_by_the_series_its_symbol_names() {
        let a = named("a", Color::BLACK);
        let b = named("b", Color::BLACK);
        let c = named("c", Color::BLACK);
        let mut content = empty_content();
        append_legend_entry(&mut content, series_symbol_segments(&c, None), "C");
        append_legend_entry(&mut content, series_symbol_segments(&a, None), "A");
        let series = [a, b, c];

        assert_eq!(legend_entry_of_series(&content, &series, "a"), Some(1));
        assert_eq!(legend_entry_of_series(&content, &series, "c"), Some(0));
        assert_eq!(legend_entry_of_series(&content, &series, "b"), None);
        assert_eq!(legend_entry_series_id(&content, 0), Some("c"));
        assert_eq!(legend_entry_series_id(&content, 5), None);
    }

    // A legend written before symbols named their series pairs by position,
    // as it always did, and a restyle names the series from then on.
    #[test]
    fn symbols_that_name_no_series_pair_by_position() {
        let red = Color::new(1.0, 0.0, 0.0, 1.0);
        let blue = Color::new(0.0, 0.0, 1.0, 1.0);
        let a = named("a", red);
        let b = named("b", blue);
        let mut content = empty_content();
        append_legend_entry(
            &mut content,
            symbol_segments(&LegendEntryKind::Line, Color::BLACK),
            "A",
        );
        append_legend_entry(
            &mut content,
            symbol_segments(&LegendEntryKind::Line, Color::BLACK),
            "B",
        );
        let series = [a, b];
        assert_eq!(legend_entry_series_id(&content, 0), None);
        assert_eq!(legend_entry_of_series(&content, &series, "b"), Some(1));

        assert_eq!(
            update_legend_symbols_preserving_text(&mut content, &series, None),
            2
        );
        assert_eq!(entry_color(&content, 0), Some(red));
        assert_eq!(entry_color(&content, 1), Some(blue));
        assert_eq!(legend_entry_series_id(&content, 1), Some("b"));
    }

    // An entry whose series is gone keeps its symbol and its text.
    #[test]
    fn an_entry_whose_series_is_gone_keeps_its_symbol() {
        let red = Color::new(1.0, 0.0, 0.0, 1.0);
        let gone = named("gone", red);
        let mut content = empty_content();
        append_legend_entry(&mut content, series_symbol_segments(&gone, None), "Gone");
        let before = content.clone();

        let replaced =
            update_legend_symbols_preserving_text(&mut content, &[named("x", Color::BLACK)], None);

        assert_eq!(replaced, 0);
        assert_eq!(content, before);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn the_series_id_round_trips_and_is_absent_from_other_segments() {
        let mut content = empty_content();
        append_legend_entry(
            &mut content,
            series_symbol_segments(&named("a", Color::BLACK), None),
            "A",
        );
        let json = serde_json::to_string(&content).unwrap();
        assert_eq!(json.matches("\"series_id\":\"a\"").count(), 1);
        let back: RichText = serde_json::from_str(&json).unwrap();
        assert_eq!(back, content);
        // A document written before the field still parses: no id.
        let plain = serde_json::to_string(&RichSegment::plain('x')).unwrap();
        assert!(!plain.contains("series_id"));
        let parsed: RichSegment = serde_json::from_str(&plain).unwrap();
        assert_eq!(parsed.series_id, None);
    }
}
