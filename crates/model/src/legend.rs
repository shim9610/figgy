//! Legend model — the whole legend is **one rich inline document**
//! ([`Legend::content`]).
//!
//! Design points:
//! - **Symbols are text.** A line sample is an em dash, a scatter sample is
//!   a geometric-shape glyph (`●`, `■`, …), colored via the per-segment
//!   `RichSegment.color` override. That buys the whole rich-text feature set
//!   (size, bold/italic, sub/super, greek) for marks, identical rendering on
//!   screen and in PNG export, and one layout engine for the entire legend.
//! - **Everything is explicit in the SSoT.** Line breaks are `'\n'` segments,
//!   symbols are ordinary inline segments — so a one-line legend, mid-text
//!   symbols, and any custom arrangement are all just segment sequences.
//! - **Font and font size are live.** `content.font` / `content.font_size`
//!   (and per-segment overrides) are consumed at draw time, so SSoT edits
//!   after composition still apply.
//!
//! Composition helpers cover the common cases: [`symbol_segments`] /
//! [`series_symbol_segments`] build a colored symbol, [`append_legend_entry`]
//! appends "symbol + space + label" as a new line. Fully custom layouts edit
//! `content.segments` directly.

use crate::color::Color;
use crate::data_config::{DataRenderType, ScatterShape, SeriesConfig};
use crate::line::LineStylePreset;
use crate::text::{RichSegment, RichText, rich_segments_from_text};
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

    fn line_cfg(color: Color, line_style: LineStylePreset) -> SeriesConfig {
        SeriesConfig {
            series_id: "s".into(),
            source_id: None,
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style,
                    line_color: color,
                    line_width: 1.0,
                },
            },
        }
    }

    fn empty_content() -> RichText {
        RichText {
            segments: Vec::new(),
            color: Color::BLACK,
            font_size: 14.0,
            font: String::new(),
        }
    }

    #[test]
    fn dashed_series_symbol_carries_rule_dash() {
        let red = Color::new(1.0, 0.0, 0.0, 1.0);
        let segs = series_symbol_segments(&line_cfg(red, LineStylePreset::Dash));
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
            series_symbol_segments(&line_cfg(red, LineStylePreset::Solid)),
            "line 1\nline 2",
        );
        append_legend_entry(
            &mut content,
            series_symbol_segments(&line_cfg(blue, LineStylePreset::Solid)),
            "beta",
        );

        let series = [
            line_cfg(green, LineStylePreset::Dash),
            line_cfg(red, LineStylePreset::Dot),
        ];
        assert_eq!(
            update_legend_symbols_preserving_text(&mut content, &series),
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
            series_symbol_segments(&line_cfg(Color::BLACK, LineStylePreset::Solid)),
            "alpha",
        );
        append_legend_entry(
            &mut content,
            series_symbol_segments(&line_cfg(Color::BLACK, LineStylePreset::Solid)),
            "beta",
        );

        set_legend_entry_label(
            &mut content,
            1,
            series_symbol_segments(&line_cfg(Color::BLACK, LineStylePreset::Solid)),
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
                &[line_cfg(Color::BLACK, LineStylePreset::Solid)]
            ),
            0
        );
        assert_eq!(content, before);
        assert!(!remove_legend_entry(&mut content, 0));
    }
}

/// Line sample: a drawn rule filling the whole symbol field — exact length,
/// unlike any dash glyph.
fn line_symbol(color: Color, style: LineStylePreset) -> Vec<RichSegment> {
    let seg = match rule_dash_em(style) {
        Some(dash) => RichSegment::dashed_rule(SYMBOL_FIELD_EM, Some(color), dash),
        None => RichSegment::rule(SYMBOL_FIELD_EM, Some(color)),
    };
    vec![seg]
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
    line_scatter_symbol_with_style(shape, color, LineStylePreset::Solid)
}

fn line_scatter_symbol_with_style(
    shape: &ScatterShape,
    color: Color,
    style: LineStylePreset,
) -> Vec<RichSegment> {
    let glyph_em = 0.7;
    let rule_em = (SYMBOL_FIELD_EM - glyph_em) * 0.5;
    let rule = |width_em| match rule_dash_em(style) {
        Some(dash) => RichSegment::dashed_rule(width_em, Some(color), dash),
        None => RichSegment::rule(width_em, Some(color)),
    };
    vec![
        rule(rule_em),
        RichSegment::fielded(scatter_shape_char(shape), glyph_em, Some(color)),
        rule(rule_em),
    ]
}

/// Symbol segments for the convenience-builder [`LegendEntryKind`]: each
/// segment carries `color: Some(color)` so the mark keeps the series color
/// independent of the document-level text color.
pub fn symbol_segments(kind: &LegendEntryKind, color: Color) -> Vec<RichSegment> {
    match kind {
        LegendEntryKind::Line => line_symbol(color, LineStylePreset::Solid),
        LegendEntryKind::Scatter => scatter_symbol(&ScatterShape::CircleFilled, color),
        LegendEntryKind::LineScatter => line_scatter_symbol(&ScatterShape::CircleFilled, color),
    }
}

/// Symbol segments matching a series declaration: line / scatter / combined,
/// shape and color taken from the render type's sub-styles.
pub fn series_symbol_segments(cfg: &SeriesConfig) -> Vec<RichSegment> {
    match &cfg.render_type {
        DataRenderType::Line { line } => line_symbol(line.line_color, line.line_style),
        DataRenderType::Scatter { scatter }
        | DataRenderType::ScatterErrorbarX { scatter, .. }
        | DataRenderType::ScatterErrorbarY { scatter, .. }
        | DataRenderType::ScatterErrorbarXY { scatter, .. } => {
            scatter_symbol(&scatter.point_shape, scatter.point_color)
        }
        DataRenderType::ScatterLine { scatter, line }
        | DataRenderType::LineScatterErrorbarX { scatter, line, .. }
        | DataRenderType::LineScatterErrorbarY { scatter, line, .. }
        | DataRenderType::LineScatterErrorbarXY { scatter, line, .. } => {
            line_scatter_symbol_with_style(&scatter.point_shape, line.line_color, line.line_style)
        }
    }
}

/// Append one "entry" to the legend document: a `'\n'` segment when content
/// already exists, then the symbol segments, one space, a `'\t'` column
/// separator, then the label (sub/superscript unicode and `'\n'` are
/// interpreted by the segment mapper).
///
/// The `'\t'` makes the symbol field a table column: the text engine sizes
/// every column to its widest cell, so labels line up even when symbol
/// forms differ in width (`—` vs `–●–`). Callers wanting a one-line legend
/// can extend `content.segments` themselves without the `'\n'`.
pub fn append_legend_entry(content: &mut RichText, symbol: Vec<RichSegment>, label: &str) {
    append_legend_entry_rich(content, symbol, rich_segments_from_text(label));
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

fn is_plain_space(seg: &RichSegment) -> bool {
    seg.text == ' '
        && !seg.bold
        && !seg.italic
        && !seg.underline
        && !seg.superscript
        && !seg.subscript
        && !seg.greek
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

/// Replace recognized entry symbols in order and keep every user text segment
/// untouched. Custom layouts that do not expose the auto symbol+tab shape are
/// left unchanged.
pub fn update_legend_symbols_preserving_text(
    content: &mut RichText,
    series: &[SeriesConfig],
) -> usize {
    let spans = legend_entry_spans(content);
    let mut replaced = 0;
    for (span, cfg) in spans.into_iter().zip(series.iter()).rev() {
        content
            .segments
            .splice(span.symbol, series_symbol_segments(cfg));
        replaced += 1;
    }
    replaced
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
    /// symbols are ordinary inline segments (glyph char + color override),
    /// so line breaks, symbol positions, and mid-text symbols are all
    /// explicit in the SSoT. Font + font_size of this RichText are live —
    /// consumed at draw time.
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
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style,
                    line_color: color,
                    line_width: 1.0,
                },
            },
        }
    }

    fn empty_content() -> RichText {
        RichText {
            segments: Vec::new(),
            color: Color::BLACK,
            font_size: 14.0,
            font: String::new(),
        }
    }

    #[test]
    fn series_symbol_takes_series_color_as_override() {
        let red = Color::new(1.0, 0.0, 0.0, 1.0);
        let segs = series_symbol_segments(&line_cfg(red));
        assert_eq!(segs.len(), 1); // "—"
        assert_eq!(segs[0].text, '—');
        assert_eq!(segs[0].color, Some(red));
        assert_eq!(segs[0].font_size, None); // size stays live via the document
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
}
