// Contour label render — one quad per anchor, textured with a pre-baked label.
//
// The labels' *content* is known before the frame starts: a label reads
// `levels[i]`, and the levels are config. So the CPU bakes each level's string
// into one cell of a 2D label atlas with the ordinary text stack
// (`measure_plain_text` / `draw_plain_text` / `FontPolicy`), colour and
// background included, and this shader only has to *place* that image.
//
// **A label image atlas, not a glyph atlas.** Whole strings are baked, so
// per-character font fallback, the sketch handwriting face and `FontPolicy` all
// stay in the single existing text stack — there is no second one. That is the
// whole reason the pipeline can be one-directional: no GPU→CPU readback, no
// ticket, and everything finishes inside the frame.
//
// Atlas layout: every cell has the same 2D stride and at least one transparent
// texel of gutter on every side. Level `i` maps to `(i % columns, i / columns)`
// and reports its content width `w_i` separately, so the quad keeps its measured
// size. Sampling stays between the first and last texel centres of that content;
// linear filtering therefore cannot blend in a neighbouring label.
//
// The anchor record arrives as an **instance vertex buffer**, which is the same
// 32 B layout whether `anchor_select` wrote it on the GPU or the renderer wrote a
// host's `ContourLabelConfig::anchors` override into it. One record format, one
// draw path.
//
// The screen angle is the anchor's data-space tangent projected as a finite
// difference — exact on a linear axis, the local direction on a logarithmic one
// — flipped by 180° when it would leave the text reading right-to-left.
