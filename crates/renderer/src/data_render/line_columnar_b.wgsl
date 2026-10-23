/// The sketch line's perpendicular displacement at arc position `arc_px`:
/// `amplitude_px · noise(arc / wavelength)`. A wavelength <= 0 disables the
/// wobble — mirrors the CPU stroker's guard (sketch.rs) so GPU and deco
/// layers degrade identically.
fn sketch_line_offset(arc_px: f32, amplitude_px: f32, wavelength_px: f32, seed: u32) -> f32 {
    let wobble = max(amplitude_px, 0.0) * sketch_noise(arc_px / max(wavelength_px, 1e-6), seed);
    return select(0.0, wobble, wavelength_px > 0.0);
}

// ───── end shared line stroke ─────
