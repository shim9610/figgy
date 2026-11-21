//! Monotonic-x line series (#271): the renderer side of the M4 envelope.
//!
//! The CPU core ([`crate::monotonic`]) reduces a group's samples to its
//! envelope. This module keeps one envelope per group and per x-to-pixel
//! mapping its series are drawn at (one, unless a subplot shows the group at
//! another range or width), uploads each series' channel to its two pool
//! columns, and checks wherever a frame, an export or a pick is prepared
//! that the envelope was built for the mapping it is drawn at. The source is
//! an argument to every call and is never retained (M1, M3).
//!
//! # Upload layout
//!
//! A series' envelope is drawn by the line pipeline as it draws any line:
//! from two pool columns, x and y, one segment per pair of consecutive rows.
//! Each envelope slot (the lead edge, the `width + 1` columns, the trail
//! edge) takes [`ROWS_PER_SLOT`] rows: each of the cell's 4 points followed
//! by its join marker. A marker repeats its point when the line goes on to
//! the next point and is NaN when it breaks there; a cell with fewer than 4
//! points repeats its last one. A repeated point makes a zero-length
//! segment, which the stroke collapses to nothing, and a NaN row masks both
//! segments that touch it, as a NaN sample does in any line. An empty slot
//! repeats the last point before it while the line runs across it and is
//! NaN otherwise. So the rows are a fixed function of the slots: row `r`
//! belongs to slot `r / 8` and point `(r % 8) / 2`, which is how a pick's
//! row maps back to a logical sample index and a selection's index to a row,
//! by arithmetic and one look at the envelope. x is `start + index ×
//! interval` in data units, so the projection places every kept point
//! exactly where a full draw places it, slide offset or not.

use std::sync::Arc;

use super::*;
use crate::data::ColumnPairWriter;
use crate::monotonic::{
    CELL_POINTS, EnvelopeCell, MappingChange, MonotonicAppend, MonotonicEnvelope, MonotonicError,
    MonotonicMapping, MonotonicSource, MonotonicSourceRef, MonotonicUpdate,
};

mod export;
mod plan;
mod resize;
mod selection;
mod state;

pub use state::*;
