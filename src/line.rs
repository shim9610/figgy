

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
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
