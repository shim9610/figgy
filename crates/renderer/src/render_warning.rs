//! Tick settings the axis raster drew differently from how they were written.
//!
//! The SSoT accepts whatever the host sends: a live spacing input passes
//! through 0 mid-edit, and a mis-typed magnitude can ask for a billion ticks.
//! The axis raster's tick walkers guard against values that would blank an
//! axis or walk it in millions of steps and draw a substitute instead. Each
//! substitution is described by a [`RenderWarning`].
//!
//! The annotation layer reports in the same way: an item whose value or
//! anchor cannot be placed is not an error of the document, it is skipped by
//! the annotation raster and described by [`RenderWarning::AnnotationSkipped`].
//!
//! [`crate::ChartView::render_warnings`] holds the warnings of the view's
//! latest raster. They are derived when `create_chart_view` or a successful
//! `refresh_axis` / `refresh_annotations` rasterizes the view, from the same
//! chart, and never per frame.

use std::fmt;

use crate::tick::MAX_LOG_DECADE_STEP;

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
    /// The Y axis of overlay layer `index`, `OverlayLayer::y_axis` of
    /// `Config::overlays[index]`. A layered view merges every panel's
    /// warnings into one list, and this tag says which layer one is about.
    OverlayY(usize),
    /// An axis of subplot `index`: `axis` names it inside
    /// `Config::subplots[index].config`.
    Subplot { index: usize, axis: SubplotAxis },
}

/// An axis inside one subplot's chart, named after its field there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SubplotAxis {
    BottomX,
    TopX,
    LeftY,
    RightY,
    ColorBar,
    /// The Y axis of the subplot's overlay `index`.
    OverlayY(usize),
}

impl SubplotAxis {
    /// The subplot-local name of an axis a subplot panel reported, or `None`
    /// for an axis that is not one of a single chart's own.
    pub fn from_panel(axis: WarningAxis) -> Option<Self> {
        Some(match axis {
            WarningAxis::BottomX => Self::BottomX,
            WarningAxis::TopX => Self::TopX,
            WarningAxis::LeftY => Self::LeftY,
            WarningAxis::RightY => Self::RightY,
            WarningAxis::ColorBar => Self::ColorBar,
            WarningAxis::OverlayY(index) => Self::OverlayY(index),
            WarningAxis::Subplot { .. } => return None,
        })
    }
}

impl fmt::Display for SubplotAxis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BottomX => f.write_str("bottom_x"),
            Self::TopX => f.write_str("top_x"),
            Self::LeftY => f.write_str("left_y"),
            Self::RightY => f.write_str("right_y"),
            Self::ColorBar => f.write_str("colorbar.axis"),
            Self::OverlayY(index) => write!(f, "overlays[{index}].y_axis"),
        }
    }
}

impl fmt::Display for WarningAxis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BottomX => f.write_str("bottom_x"),
            Self::TopX => f.write_str("top_x"),
            Self::LeftY => f.write_str("left_y"),
            Self::RightY => f.write_str("right_y"),
            Self::ColorBar => f.write_str("colorbar.axis"),
            Self::OverlayY(index) => write!(f, "overlays[{index}].y_axis"),
            Self::Subplot { index, axis } => write!(f, "subplots[{index}].config.{axis}"),
        }
    }
}

/// Why the linear tick walker did not walk a requested `major_spacing`.
/// Defined with the walkers in [`crate::tick`].
pub use crate::tick::SpacingRejection;

/// Why the annotation raster skipped an item.
///
/// New reasons may be added as new variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AnnotationSkip {
    /// A value or anchor coordinate is NaN or infinite.
    NonFinite,
    /// A data anchor, reference line or span on a chart whose axes carry no
    /// values (a pie, donut, radar or polar chart), so only ratio anchors can
    /// be placed.
    NoValueAxis,
    /// The value or anchor has no position on its axis: outside the axis
    /// range for a reference line, a span entirely outside it, a text box or
    /// arrow endpoint outside the panel, or a non-positive value on a
    /// logarithmic axis.
    OutOfRange,
    /// A text item without any text.
    EmptyText,
}

/// A setting the raster drew differently from how it was written, or an
/// annotation it did not draw.
///
/// The tick variants name the axis, the value as written (`requested`) and
/// the value the raster actually drew (`used`). `used` has the type of the
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
    /// `annotations.items[index]` was not drawn and not registered in the
    /// annotation hit map. The item itself is kept and still serializes.
    AnnotationSkipped {
        /// Index into `AnnotationLayer::items`.
        index: usize,
        /// Why it has no position.
        reason: AnnotationSkip,
    },
}

impl RenderWarning {
    /// The axis this warning is about; `None` for an annotation warning.
    pub fn axis(&self) -> Option<WarningAxis> {
        match self {
            Self::MajorSpacingReplaced { axis, .. }
            | Self::MinorCountClamped { axis, .. }
            | Self::LogDecadeStepAdjusted { axis, .. } => Some(*axis),
            Self::AnnotationSkipped { .. } => None,
        }
    }

    /// The same warning about `axis`; a warning that is not about an axis is
    /// returned unchanged. Overlay panels are rasterized from an
    /// internal config whose right axis is the overlay's, so their warnings
    /// are re-tagged with the layer they belong to.
    pub fn with_axis(mut self, axis: WarningAxis) -> Self {
        match &mut self {
            Self::MajorSpacingReplaced { axis: slot, .. }
            | Self::MinorCountClamped { axis: slot, .. }
            | Self::LogDecadeStepAdjusted { axis: slot, .. } => *slot = axis,
            // Not about an axis; nothing to re-tag.
            Self::AnnotationSkipped { .. } => {}
        }
        self
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
                    // `SpacingRejection` is `#[non_exhaustive]` in the model.
                    other => write!(f, "was rejected ({other:?})")?,
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
            Self::AnnotationSkipped { index, reason } => write!(
                f,
                "annotations.items[{index}] was not drawn: {}",
                match reason {
                    AnnotationSkip::NonFinite => "a value or anchor is not finite",
                    AnnotationSkip::NoValueAxis =>
                        "this chart has no value axis for data coordinates",
                    AnnotationSkip::OutOfRange => "its position is outside the axis range or panel",
                    AnnotationSkip::EmptyText => "it has no text",
                }
            ),
        }
    }
}
