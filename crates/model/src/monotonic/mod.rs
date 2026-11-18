//! The CPU core of the monotonic-x line chart (#271): fast, large, evenly
//! spaced data (`x = start + i * interval`) drawn at display resolution.
//!
//! Per pixel column the core reduces the samples to **M4** (first, min, max
//! and last, in x order) on the CPU, so a renderer draws about
//! `4 × plot width` vertices per channel. A column with at most 4 samples
//! keeps them all.
//!
//! - **Source.** The host lends its samples through [`MonotonicSource`], as
//!   contiguous runs ([`MonotonicRun`]); [`MonotonicSlices`] and
//!   [`MonotonicInterleaved`] are the built-in impls. Elements are `f64`,
//!   `f32`, `i32`, `i16`, `u16` or `u8` ([`MonotonicElement`]). The source is
//!   an argument to every call and is never retained.
//! - **Calls.** [`MonotonicEnvelope::set_monotonic`] takes the source,
//!   which carries `start` and `interval`, and reduces in full.
//!   [`MonotonicEnvelope::append_monotonic`] takes only new samples and
//!   merges them; with [`MonotonicAppend::sliding`] the window moves by the
//!   same count. No call but the full one reads earlier samples.
//! - **Staleness.** The envelope records the [`MonotonicMapping`] it was
//!   built for. [`MonotonicEnvelope::check_mapping`] refuses any other
//!   mapping with [`MonotonicError::SourceRequired`]: only the source can
//!   rebuild it.
//!
//! See `MONOTONIC.md` in this crate for the rules (M1–M7).

mod config;
mod element;
mod envelope;
mod grid;
mod reduce;
mod source;

#[cfg(test)]
mod bench;
#[cfg(test)]
mod tests;

pub use config::{MonotonicAxis, MonotonicConfig, same_mapping};
pub use element::MonotonicElement;
pub use envelope::{MonotonicAppend, MonotonicEnvelope, MonotonicUpdate, MonotonicView};
pub use grid::{MonotonicGrid, VisibleSlice};
pub use reduce::{CELL_POINTS, EnvelopeCell};
pub use source::{
    DynMonotonicSource, MonotonicInterleaved, MonotonicRun, MonotonicSlices, MonotonicSource,
    MonotonicSourceRef,
};

use crate::config::AxisScale;
use crate::layout::BreakMap;

/// The x-to-pixel mapping an envelope is built for: everything that decides
/// the pixel column of a sample. A change to any field needs the source
/// ([`MonotonicEnvelope::check_mapping`]); the y range, colours, styles and
/// texts do not, since the envelope is in data units.
#[derive(Clone, Debug, PartialEq)]
pub struct MonotonicMapping {
    /// The x range, in data units.
    pub x_min: f64,
    pub x_max: f64,
    /// The plot width in device pixels: a resize, a layout change that moves
    /// the plot's edges, a DPI change and a 2x export all change it.
    pub plot_width_px: u32,
    /// The x axis scale. Only a linear axis is accepted.
    pub x_scale: AxisScale,
    /// The x axis breaks. Only an unbroken axis is accepted.
    pub x_breaks: BreakMap,
    /// Whether the x axis is inverted.
    pub x_inverted: bool,
    /// Whole pixel columns kept beyond each edge of the plot, reduced like
    /// the columns inside it: a stroke reaches into the plot from samples
    /// up to its half width (and its antialiasing) outside it, so a renderer
    /// keeps a fixed number of them, set when the envelope is built. Zero
    /// keeps only the edge samples.
    pub margin_px: u32,
}

impl MonotonicMapping {
    /// A linear, unbroken, not inverted mapping.
    pub fn linear(x_min: f64, x_max: f64, plot_width_px: u32) -> Self {
        Self {
            x_min,
            x_max,
            plot_width_px,
            x_scale: AxisScale::Linear,
            x_breaks: BreakMap::IDENTITY,
            x_inverted: false,
            margin_px: 0,
        }
    }

    /// The first field in which `current` differs from `self`, `None` when
    /// they are equal. The x range compares bit for bit.
    pub fn change_to(&self, current: &Self) -> Option<MappingChange> {
        if self.x_min.to_bits() != current.x_min.to_bits()
            || self.x_max.to_bits() != current.x_max.to_bits()
        {
            Some(MappingChange::XRange)
        } else if self.plot_width_px != current.plot_width_px || self.margin_px != current.margin_px
        {
            Some(MappingChange::PlotWidth)
        } else if self.x_scale != current.x_scale {
            Some(MappingChange::XScale)
        } else if self.x_breaks != current.x_breaks {
            Some(MappingChange::XBreaks)
        } else if self.x_inverted != current.x_inverted {
            Some(MappingChange::XInversion)
        } else {
            None
        }
    }
}

/// Which part of the mapping changed since the envelope was built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappingChange {
    /// The envelope was never built, or its last call failed.
    NotBuilt,
    XRange,
    PlotWidth,
    XScale,
    XBreaks,
    XInversion,
}

/// Why a monotonic call failed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MonotonicError {
    /// The envelope was built for another mapping and holds no data to
    /// rebuild from: the call needs the source.
    SourceRequired { change: MappingChange },
    /// `start` is not finite, or `interval` is not finite and positive.
    InvalidGrid { start: f64, interval: f64 },
    /// The x range is not finite and increasing, or the width is zero.
    InvalidMapping {
        x_min: f64,
        x_max: f64,
        plot_width_px: u32,
    },
    /// The x scale is not linear or the x axis is broken: not meaningful for
    /// this source, refused (#271).
    UnsupportedXMapping,
    /// An interleaved buffer with a zero stride.
    InvalidStride,
    /// An interleaved column outside the frame.
    InvalidColumn { column: usize, stride: usize },
    /// An append before any full call.
    NotBuilt,
    /// An append with another number of channels: that is a full call.
    ChannelCountChanged { expected: usize, found: usize },
    /// A slide would move the window over old samples that were counted but
    /// not kept (they lay past the trail edge): that needs a full call with
    /// the source.
    SlidePastKept,
    /// The logical sample count would overflow.
    CountOverflow,
    /// The envelope's memory could not be allocated.
    OutOfMemory,
}

impl std::fmt::Display for MonotonicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SourceRequired { change } => write!(
                f,
                "the x mapping changed ({change:?}) since the envelope was built; \
                 the call needs the source"
            ),
            Self::InvalidGrid { start, interval } => write!(
                f,
                "start {start} must be finite and interval {interval} finite and positive"
            ),
            Self::InvalidMapping {
                x_min,
                x_max,
                plot_width_px,
            } => write!(
                f,
                "x range {x_min}..{x_max} must be finite and increasing, \
                 and plot width {plot_width_px} positive"
            ),
            Self::UnsupportedXMapping => {
                f.write_str("a monotonic series needs a linear, unbroken x axis")
            }
            Self::InvalidStride => f.write_str("an interleaved stride must be positive"),
            Self::InvalidColumn { column, stride } => {
                write!(f, "column {column} lies outside a frame of {stride} values")
            }
            Self::NotBuilt => f.write_str("an append needs a full call first"),
            Self::ChannelCountChanged { expected, found } => write!(
                f,
                "an append keeps the {expected} channels, found {found}; that is a full call"
            ),
            Self::SlidePastKept => f.write_str(
                "the slide moves over samples that were counted but not kept; \
                 it needs a full call with the source",
            ),
            Self::CountOverflow => f.write_str("the sample count would overflow"),
            Self::OutOfMemory => f.write_str("the envelope's memory could not be allocated"),
        }
    }
}

impl std::error::Error for MonotonicError {}
