//! Step interpolation geometry shared by every CPU consumer of a line series:
//! the style flags the line shader reads, the per-instance strip vertex
//! count, and the data-space pieces the CPU picker walks.
//!
//! The GPU twins are `step_piece` / `strip_slot` in
//! `data_render/line_columnar.wgsl` (shared byte-for-byte with
//! `line_arc.wgsl`) and `pick_step_piece` in `gpu_pick.wgsl`. All of them
//! split a segment `a → b` into the same axis-aligned pieces, with the
//! step-mid riser at the **data-space** midpoint of the two x values, so the
//! drawn line, its dash phase, its arc length and both pickers agree.

use crate::data_config::LineInterpolation;

/// `Style.primitive_flags` bit position of the two-bit step mode.
pub(crate) const LINE_STEP_SHIFT: u32 = 2;
/// `Style.primitive_flags` bit the area entry reads: fill to a second series'
/// y column (slots 4/5) instead of the baseline in `Style.dash[1].xy`.
pub(crate) const AREA_FILL_TO_SERIES: u32 = 1 << 4;

/// The shader's step mode code: 0 linear, 1 step-pre, 2 step-post, 3 step-mid.
pub(crate) fn step_mode(interpolation: LineInterpolation) -> u32 {
    match interpolation {
        LineInterpolation::Linear => 0,
        LineInterpolation::StepPre => 1,
        LineInterpolation::StepPost => 2,
        LineInterpolation::StepMid => 3,
    }
}

/// The step mode placed at its `primitive_flags` position.
pub(crate) fn step_flag_bits(interpolation: LineInterpolation) -> u32 {
    step_mode(interpolation) << LINE_STEP_SHIFT
}

/// Vertices one instance draws when every piece is a strip of `per_piece`
/// vertices and consecutive pieces are joined by two duplicated vertices:
/// `pieces · per_piece + 2 · (pieces − 1)`. A one-piece instance is the plain
/// strip. Twin of `strip_slot` in `line_columnar.wgsl`.
pub(crate) fn strip_vertex_count(per_piece: u32, interpolation: LineInterpolation) -> u32 {
    let pieces = interpolation.piece_count();
    pieces * per_piece + 2 * (pieces - 1)
}

/// One axis-aligned piece of a stepped segment, in data units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StepPiece {
    pub a: [f32; 2],
    pub b: [f32; 2],
}

/// The pieces a segment `a → b` is drawn as. Linear: the segment itself.
/// Step-pre: riser at `a.x`, run at `b.y`. Step-post: run at `a.y`, riser at
/// `b.x`. Step-mid: run, riser at the data-space midpoint of the x values,
/// run. Returned in draw order; the unused tail of the array is never read.
pub(crate) fn step_pieces(
    a: [f32; 2],
    b: [f32; 2],
    interpolation: LineInterpolation,
) -> ([StepPiece; 3], usize) {
    let [ax, ay] = a;
    let [bx, by] = b;
    let whole = StepPiece { a, b };
    match interpolation {
        LineInterpolation::Linear => ([whole; 3], 1),
        LineInterpolation::StepPre => (
            [
                StepPiece { a, b: [ax, by] },
                StepPiece { a: [ax, by], b },
                whole,
            ],
            2,
        ),
        LineInterpolation::StepPost => (
            [
                StepPiece { a, b: [bx, ay] },
                StepPiece { a: [bx, ay], b },
                whole,
            ],
            2,
        ),
        LineInterpolation::StepMid => {
            // The same expression the shaders use on the pair's hi lane.
            let xm = (ax + bx) * 0.5;
            (
                [
                    StepPiece { a, b: [xm, ay] },
                    StepPiece {
                        a: [xm, ay],
                        b: [xm, by],
                    },
                    StepPiece { a: [xm, by], b },
                ],
                3,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_counts_match_the_shader_joint_rule() {
        assert_eq!(strip_vertex_count(4, LineInterpolation::Linear), 4);
        assert_eq!(strip_vertex_count(4, LineInterpolation::StepPre), 10);
        assert_eq!(strip_vertex_count(4, LineInterpolation::StepPost), 10);
        assert_eq!(strip_vertex_count(4, LineInterpolation::StepMid), 16);
        assert_eq!(strip_vertex_count(18, LineInterpolation::StepMid), 58);
    }

    #[test]
    fn pieces_are_axis_aligned_and_chain_from_a_to_b() {
        let a = [1.0, 2.0];
        let b = [5.0, -4.0];
        for mode in [
            LineInterpolation::Linear,
            LineInterpolation::StepPre,
            LineInterpolation::StepPost,
            LineInterpolation::StepMid,
        ] {
            let (pieces, count) = step_pieces(a, b, mode);
            assert_eq!(count as u32, mode.piece_count());
            assert_eq!(pieces[0].a, a);
            assert_eq!(pieces[count - 1].b, b);
            for pair in pieces[..count].windows(2) {
                assert_eq!(pair[0].b, pair[1].a, "{mode:?} pieces must chain");
            }
            if mode != LineInterpolation::Linear {
                for piece in &pieces[..count] {
                    assert!(
                        piece.a[0] == piece.b[0] || piece.a[1] == piece.b[1],
                        "{mode:?} piece {piece:?} is not axis-aligned"
                    );
                }
            }
        }
        let (mid, _) = step_pieces(a, b, LineInterpolation::StepMid);
        assert_eq!(
            mid[1].a[0], 3.0,
            "the step-mid riser sits at the data midpoint"
        );
        let (post, _) = step_pieces(a, b, LineInterpolation::StepPost);
        assert_eq!(post[0].b, [5.0, 2.0]);
        let (pre, _) = step_pieces(a, b, LineInterpolation::StepPre);
        assert_eq!(pre[0].b, [1.0, -4.0]);
    }

    #[test]
    fn flag_bits_sit_above_the_errorbar_bits() {
        assert_eq!(step_flag_bits(LineInterpolation::Linear), 0);
        assert_eq!(step_flag_bits(LineInterpolation::StepMid), 0b1100);
        assert_eq!(AREA_FILL_TO_SERIES, 0b1_0000);
    }
}
