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
use crate::text::{rich_segments_from_text, RichSegment, RichText};

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

/// The character form of a [`ScatterShape`] (Liberation Sans covers the
/// geometric-shapes glyphs used here).
pub fn scatter_shape_char(shape: &ScatterShape) -> char {
    match shape {
        ScatterShape::Circle => '○',
        ScatterShape::CircleFilled => '●',
        ScatterShape::Square => '□',
        ScatterShape::SquareFilled => '■',
        ScatterShape::Triangle => '△',
        ScatterShape::TriangleFilled => '▲',
        ScatterShape::Diamond => '◇',
        ScatterShape::DiamondFilled => '◆',
        ScatterShape::Cross => '×',
    }
}

/// Every symbol form spans exactly this many em (× legend font size), so
/// marks line up regardless of shape: a lone line, a lone glyph, and a
/// line-through-glyph all advance the pen by the same fixed field.
pub const SYMBOL_FIELD_EM: f32 = 2.0;

/// Line sample: a drawn rule filling the whole symbol field — exact length,
/// unlike any dash glyph.
fn line_symbol(color: Color) -> Vec<RichSegment> {
    vec![RichSegment::rule(SYMBOL_FIELD_EM, Some(color))]
}

/// Point sample: the shape's glyph centered in the symbol field.
fn scatter_symbol(shape: &ScatterShape, color: Color) -> Vec<RichSegment> {
    vec![RichSegment::fielded(scatter_shape_char(shape), SYMBOL_FIELD_EM, Some(color))]
}

/// Line + point sample: rule — glyph — rule, the three fields summing to
/// exactly [`SYMBOL_FIELD_EM`] so the combined mark is as long as a lone
/// line mark.
fn line_scatter_symbol(shape: &ScatterShape, color: Color) -> Vec<RichSegment> {
    let glyph_em = 0.7;
    let rule_em = (SYMBOL_FIELD_EM - glyph_em) * 0.5;
    vec![
        RichSegment::rule(rule_em, Some(color)),
        RichSegment::fielded(scatter_shape_char(shape), glyph_em, Some(color)),
        RichSegment::rule(rule_em, Some(color)),
    ]
}

/// Symbol segments for the convenience-builder [`LegendEntryKind`]: each
/// segment carries `color: Some(color)` so the mark keeps the series color
/// independent of the document-level text color.
pub fn symbol_segments(kind: &LegendEntryKind, color: Color) -> Vec<RichSegment> {
    match kind {
        LegendEntryKind::Line => line_symbol(color),
        LegendEntryKind::Scatter => scatter_symbol(&ScatterShape::CircleFilled, color),
        LegendEntryKind::LineScatter => {
            line_scatter_symbol(&ScatterShape::CircleFilled, color)
        }
    }
}

/// Symbol segments matching a series declaration: line / scatter / combined,
/// shape and color taken from the render type's sub-styles.
pub fn series_symbol_segments(cfg: &SeriesConfig) -> Vec<RichSegment> {
    match &cfg.render_type {
        DataRenderType::Line { line } => line_symbol(line.line_color),
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
            line_scatter_symbol(&scatter.point_shape, line.line_color)
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
    if !content.segments.is_empty() {
        content.segments.push(RichSegment::plain('\n'));
    }
    content.segments.extend(symbol);
    content.segments.push(RichSegment::plain(' '));
    content.segments.push(RichSegment::plain('\t'));
    content.segments.extend(rich_segments_from_text(label));
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
        SeriesConfig {
            series_id: "s".into(),
            label: None,
            x_column: "x".into(),
            y_column: "y".into(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
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
            ScatterShape::Diamond,
            ScatterShape::Cross,
            ScatterShape::CircleFilled,
            ScatterShape::SquareFilled,
            ScatterShape::TriangleFilled,
            ScatterShape::DiamondFilled,
        ] {
            let c = scatter_shape_char(&shape);
            assert!(!c.is_ascii_alphanumeric(), "{shape:?} → {c}");
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
        append_legend_entry(&mut content, symbol_segments(&LegendEntryKind::Line, Color::BLACK), "a");
        append_legend_entry(&mut content, symbol_segments(&LegendEntryKind::Line, Color::BLACK), "b");

        let newlines = content.segments.iter().filter(|s| s.text == '\n').count();
        assert_eq!(newlines, 1);
        // Entry shape: symbol, space, '\t' column break, label — twice.
        // The tab gives every entry the same symbol-field width at draw time.
        let texts: String = content.segments.iter().map(|s| s.text).collect();
        assert_eq!(texts, "— \ta\n— \tb");
        // Labels inherit the document color (no override).
        assert!(content.segments.iter().filter(|s| s.text == 'a').all(|s| s.color.is_none()));
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
        append_legend_entry(&mut content, symbol_segments(&LegendEntryKind::Line, red), "a");
        content.segments.push(RichSegment::plain(' '));
        content.segments.extend(symbol_segments(&LegendEntryKind::Scatter, red));
        content.segments.push(RichSegment::plain(' '));
        content.segments.extend(rich_segments_from_text("b"));

        assert!(content.segments.iter().all(|s| s.text != '\n'));
        let texts: String = content.segments.iter().map(|s| s.text).collect();
        assert_eq!(texts, "— \ta ● b");
    }
}
