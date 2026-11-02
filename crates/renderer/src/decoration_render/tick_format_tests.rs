//! Tick label formatting drawn on the CPU raster (moved with the
//! formatter to `model::chrome`, #160).

use super::*;
use crate::raster::Canvas;
use crate::text::{MathKind, RichSegment, RichText};
use crate::text_render::{FontPolicy, draw_rich_text, measure_rich_text};

/// The label as the superscript flags wrote it before D3: the node
/// dropped, its exponent slot flagged.
fn as_flags(rt: &RichText) -> RichText {
    RichText {
        segments: rt
            .segments
            .iter()
            .filter(|s| s.math_node.is_none())
            .map(|s| RichSegment {
                superscript: s.math_parent.is_some_and(|p| p.slot == 1),
                math_parent: None,
                ..s.clone()
            })
            .collect(),
        ..rt.clone()
    }
}

fn pixels(rt: &RichText, policy: FontPolicy) -> Vec<u8> {
    let mut canvas = Canvas::new(120, 48).unwrap();
    draw_rich_text(&mut canvas, rt, (6.3, 30.6), policy).unwrap();
    canvas.into_rgba()
}

// D3: the Power label is a text-mode `Scripts` node, and it measures and
// draws exactly as the superscript flags it replaces.
#[test]
fn power_labels_are_a_scripts_node_with_the_flags_pixels() {
    let mut ls = crate::default::default_axis_options_x().label_style;
    for size in [11.0, 14.0, 28.0] {
        ls.font_size = size;
        for value in [1.0e-3, 1.5e-3, -2.0e5, 1.0e10, 7.25e-12, -1.0e-1] {
            let label = format_tick_power(value, 3, &ls);
            assert_eq!(label.validate_math(), Ok(()));
            let node = label
                .segments
                .iter()
                .find_map(|s| s.math_node)
                .expect("a Scripts node");
            assert_eq!(node.kind, MathKind::Scripts);
            let flags = as_flags(&label);
            assert!(flags.segments.iter().any(|s| s.superscript));
            for policy in [FontPolicy::Standard, FontPolicy::Handwritten] {
                assert_eq!(
                    measure_rich_text(&label, policy),
                    measure_rich_text(&flags, policy),
                    "{value} at {size}"
                );
                assert_eq!(pixels(&label, policy), pixels(&flags, policy), "{value}");
            }
        }
    }
}
