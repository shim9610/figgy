//! Tick settings the axis raster drew differently from how they were written.
//!
//! The SSoT accepts whatever the host sends: a live spacing input passes
//! through 0 mid-edit, and a mis-typed magnitude can ask for a billion ticks.
//! The axis raster's tick walkers guard against values that would blank an
//! axis or walk it in millions of steps and draw a substitute instead. Each
//! substitution is described by a [`RenderWarning`].
//!
//! [`crate::ChartView::render_warnings`] holds the warnings of the view's
//! latest raster. They are derived when `create_chart_view` or a successful
//! `refresh_axis` rasterizes the view, from the same chart, and never per
//! frame.

use std::fmt;

use crate::axis_render::MAX_LOG_DECADE_STEP;

/// The axis a [`RenderWarning`] is about, named after its `Config` field.
///
/// `Display` prints the field path a host would point a user at
/// (`bottom_x`, `colorbar.axis`). New axes may be added as new variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WarningAxis {
    /// `Config::bottom_x`.
    BottomX,
    /// `Config::top_x`.
    TopX,
    /// `Config::left_y`.
    LeftY,
    /// `Config::right_y`.
    RightY,
    /// The colourbar's value axis, `ColorBarOptions::axis` of `Config::colorbar`.
    ColorBar,
}

impl fmt::Display for WarningAxis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BottomX => "bottom_x",
            Self::TopX => "top_x",
            Self::LeftY => "left_y",
            Self::RightY => "right_y",
            Self::ColorBar => "colorbar.axis",
        })
    }
}

/// Why the linear tick walker did not walk a requested `major_spacing`.
///
/// New reasons may be added as new variants. The fields of existing variants
/// are frozen.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SpacingRejection {
    /// NaN or infinite.
    NonFinite,
    /// Zero or negative.
    NotPositive,
    /// Too small to move either end of the axis range at `f64` precision
    /// (`min + spacing == min` and `max - spacing == max`), so the walk could
    /// not advance.
    BelowResolution,
    /// The axis range holds more than `limit` intervals of this spacing.
    TooManyTicks {
        /// The most major intervals the walker draws (1000).
        limit: usize,
    },
}

/// A tick setting the axis raster replaced with one it can draw.
///
/// Every variant names the axis, the value as written (`requested`) and the
/// value the raster actually drew (`used`). `used` has the type of the
/// `AxisOptions` field it stands in for, so it can be written back as is.
/// Floating-point values are reported exactly as the renderer saw them, so
/// `requested` can be NaN, which never compares equal.
///
/// `Display` writes one line naming the setting and both values.
///
/// New kinds of warning may be added as new variants. The fields of existing
/// variants are frozen.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum RenderWarning {
    /// A linear axis' `major_spacing` was not walked. The raster drew the
    /// automatic spacing for the current range, and subdivided it for the
    /// minor ticks.
    MajorSpacingReplaced {
        /// The axis whose setting was replaced.
        axis: WarningAxis,
        /// `major_spacing` as written.
        requested: f64,
        /// The major spacing drawn.
        used: f64,
        /// Why `requested` was not walked.
        reason: SpacingRejection,
    },
    /// A linear axis' `minor_count` is more than the raster draws per major
    /// interval (99).
    MinorCountClamped {
        /// The axis whose setting was clamped.
        axis: WarningAxis,
        /// `minor_count` as written.
        requested: usize,
        /// Minor ticks drawn per major interval.
        used: usize,
    },
    /// A logarithmic axis' `major_spacing`, the decade step between major
    /// ticks, is not a whole number from 1 to `i32::MAX`: a fractional step
    /// is floored, one below 1 or non-finite draws 1, and one past
    /// `i32::MAX` draws `i32::MAX`.
    LogDecadeStepAdjusted {
        /// The axis whose setting was adjusted.
        axis: WarningAxis,
        /// `major_spacing` as written.
        requested: f64,
        /// The whole number of decades drawn between major ticks.
        used: f64,
    },
}

impl RenderWarning {
    /// The axis this warning is about.
    pub fn axis(&self) -> WarningAxis {
        match self {
            Self::MajorSpacingReplaced { axis, .. }
            | Self::MinorCountClamped { axis, .. }
            | Self::LogDecadeStepAdjusted { axis, .. } => *axis,
        }
    }
}

impl fmt::Display for RenderWarning {
    // `{:?}` for the spacings: Debug switches to exponent form at extreme
    // magnitudes ("1e-12"), where Display would spell out every zero.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MajorSpacingReplaced {
                axis,
                requested,
                used,
                reason,
            } => {
                write!(f, "{axis}.major_spacing {requested:?} ")?;
                match reason {
                    SpacingRejection::NonFinite => f.write_str("is not finite")?,
                    SpacingRejection::NotPositive => f.write_str("is not positive")?,
                    SpacingRejection::BelowResolution => {
                        f.write_str("is below the floating-point resolution of the axis range")?
                    }
                    SpacingRejection::TooManyTicks { limit } => {
                        write!(f, "would need more than {limit} major intervals")?
                    }
                }
                write!(f, "; drawn at automatic spacing {used:?} instead")
            }
            Self::MinorCountClamped {
                axis,
                requested,
                used,
            } => write!(
                f,
                "{axis}.minor_count {requested} is over the limit; \
                 drawn with {used} minor ticks per major interval instead"
            ),
            Self::LogDecadeStepAdjusted {
                axis,
                requested,
                used,
            } => write!(
                f,
                "{axis}.major_spacing {requested:?} is not a whole number of decades \
                 from 1 to {max}; drawn with a {used}-decade step instead",
                max = MAX_LOG_DECADE_STEP
            ),
        }
    }
}
