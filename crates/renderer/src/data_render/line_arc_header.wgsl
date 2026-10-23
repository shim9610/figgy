// Arc-length prefix for dashed lines — fully GPU-resident compute scan.
//
// The dash phase must advance with the polyline's cumulative PIXEL arc
// length, which depends on the data→pixel transform. This pipeline produces
// the per-point prefix directly on the GPU so the column data never has to
// come back to the CPU (the column pool keeps no CPU copies — that contract
// is load-bearing; do not reintroduce shadows to "simplify" this).
//
// Entry points, dispatched by `line_arc.rs`:
//   1. `seg_init`     — dst[start+i] = pixel length of segment (g-1, g) at
//                       global index g = start+i; 0 at g=0. Non-finite
//                       endpoints (NaN gaps, log of ≤0) contribute 0 so the
//                       phase stays continuous across gaps. A step mode
//                       (`params.mode`) sums the stepped pieces instead of
//                       the straight distance.
//   2. `scan_block`   — in-place per-256-block inclusive scan of the chunk,
//                       block totals written to `sums` (Hillis–Steele in
//                       workgroup memory).
//   3. `add_offsets`  — dst[start+i] += scanned_sums[block(i) - 1], block > 0.
//   4. `apply_carry`  — dst[start+i] += carry[0]: folds the running total of
//                       every previous chunk into this one.
//   5. `update_carry` — carry[0] = dst[start+len-1] (single thread), run
//                       AFTER apply_carry so the total stays cumulative.
//
// One chunk covers up to min(dispatch-limit × 256, 256³) points via the
// recursive scan (dst → sums0 → sums1); larger polylines are processed as
// SEQUENTIAL chunks in one encoder with the carry chain linking them — n is
// bounded only by pool memory, never by dispatch limits. The result buffer
// doubles as the line pipeline's vertex slots 4/5.
