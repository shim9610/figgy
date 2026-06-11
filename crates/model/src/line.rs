

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LineStylePreset {
    /// Solid line (no pattern).
    #[default]
    Solid,
    /// Standard dash: `[8, 4]`.
    Dash,
    /// Dotted: `[2, 3]`.
    Dot,
    /// Dash-dot: `[8, 4, 2, 4]`.
    DashDot,
    /// Dash-dot-dot: `[8, 4, 2, 4, 2, 4]`.
    DashDotDot,
    /// Short dash: `[4, 3]`.
    ShortDash,
    /// Short dot (dense dots): `[1, 2]`.
    ShortDot,
    /// Short dash-dot: `[4, 3, 1, 3]`.
    ShortDashDot,
    /// Long dash: `[14, 4]`.
    LongDash,
    /// Long dash-dot: `[14, 4, 2, 4]`.
    LongDashDot,
    /// Long dash-dot-dot: `[14, 4, 2, 4, 2, 4]`.
    LongDashDotDot,
}

impl LineStylePreset {
    /// Dash pattern as sequential `[on, off, ...]` pixel lengths at 1× scale.
    /// Empty slice = solid. Consumers (CPU raster dash, GPU `Style.dash`)
    /// apply their own scale factor to each entry.
    pub fn pattern(&self) -> &'static [f32] {
        match self {
            LineStylePreset::Solid => &[],
            LineStylePreset::Dash => &[8.0, 4.0],
            LineStylePreset::Dot => &[2.0, 3.0],
            LineStylePreset::DashDot => &[8.0, 4.0, 2.0, 4.0],
            LineStylePreset::DashDotDot => &[8.0, 4.0, 2.0, 4.0, 2.0, 4.0],
            LineStylePreset::ShortDash => &[4.0, 3.0],
            LineStylePreset::ShortDot => &[1.0, 2.0],
            LineStylePreset::ShortDashDot => &[4.0, 3.0, 1.0, 3.0],
            LineStylePreset::LongDash => &[14.0, 4.0],
            LineStylePreset::LongDashDot => &[14.0, 4.0, 2.0, 4.0],
            LineStylePreset::LongDashDotDot => &[14.0, 4.0, 2.0, 4.0, 2.0, 4.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solid_pattern_is_empty() {
        assert!(LineStylePreset::Solid.pattern().is_empty());
    }

    /// The longest presets stay within the GPU dash capacity of 8 scalars
    /// (`Style.dash` = 2 × vec4 in SHADER_COMMON.md §2).
    #[test]
    fn longest_presets_have_six_entries() {
        assert_eq!(LineStylePreset::DashDotDot.pattern().len(), 6);
        assert_eq!(LineStylePreset::LongDashDotDot.pattern().len(), 6);
    }
}
