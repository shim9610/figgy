// Columnar errorbar shader.
//
// Six columns (x, y, err_y_lo, err_y_hi, err_x_lo, err_x_hi) bound as
// per-instance slots. Each instance produces 36 vertices on a TriangleList:
// six axis-aligned quads (6 vertices each) — Y stem, cap @ y_lo, cap @ y_hi,
// X stem, cap @ x_lo, cap @ x_hi. Stem endpoints live in data space; caps
// span ±style.cap_half_px (pixels) perpendicular to their stem. Each quad is
// expanded perpendicular to its own axis by half its stroke width in pixels
// (stems: style.line_width_px, caps: style.cap_width_px); pixel offsets are
// converted to NDC via transform.pixel_to_ndc.
//
// `style.primitive_flags` is the direction authority: bit 0 enables Y and
// bit 1 enables X. Inactive vertex slots may bind any sufficiently long
// column because the shader collapses that direction before reading errors.
//
// `scale_log.{x,y}` = 1.0 applies log10 to both the input coords and the
// computed err endpoints.
