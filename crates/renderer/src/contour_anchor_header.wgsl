// Contour label anchor selection — where each label goes, chosen on the GPU.
//
// The labels' *content* is baked on the CPU before the frame (it comes from
// `levels`, which is config); only the *position* is a GPU fact. This is that
// position, and it is computed from the field itself rather than from a list of
// traced segments. The implicit-field path replaced marching squares with
// form, and this shader is the label half of that change.
//
// **Seed lattice, then Newton.** The data area is divided into cells of about
// `spacing_px`. Each (cell, level) pair seeds one candidate at the cell centre
// and projects it onto that level's isoline:
//
//     p <- p - (z(p) - level) * grad z / |grad z|^2
//
// Four steps. The field is bilinear, so away from a stationary point that is two
// or three more than enough. The tangent falls out exactly as `perp(grad z)` —
// no finite difference through the transform, no polyline to walk.
//
// **Determinism is structural here, not a rule.** The candidate set is a fixed
// lattice and every candidate writes its own slot (`level * cells + cell`), so
// there is not one atomic operation in the pass. The previous design picked
// winners by `atomicMin` over `atomicAdd`-assigned segment indices, which was
// only stable because a software rasterizer happens to run serially.
//
// Two passes:
//
//   1. `anchor_project` — one invocation per (cell, level); writes a candidate
//      or marks the slot empty. Parallel, no communication.
//   2. `anchor_select`   — one invocation, serial: keeps candidates that clear
//      `spacing_px` and cannot overlap, then gives any level that came out empty
//      the candidate farthest from the labels already kept.
//
// A seed's projection is kept only if it lands back inside that seed's own
// lattice cell. That is what stops a dozen neighbouring seeds from converging on
// the same stretch of one isoline, and it needs no communication at all.
