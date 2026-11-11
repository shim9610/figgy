//! The envelope cache of one monotonic group and the calls that build it
//! (#271). [`MonotonicEnvelope::set_monotonic`] reduces in full from the
//! source; [`MonotonicEnvelope::append_monotonic`] merges only the samples it
//! is given, and with [`MonotonicAppend::sliding`] moves the window by the
//! same count. No call but the full one reads earlier samples, so the host is
//! free to drop old data.

use std::ops::Range;

use super::element::MonotonicElement;
use super::grid::{MonotonicGrid, VisibleSlice};
use super::reduce::{BATCH, ColumnState, EnvelopeCell, extend, fill, finish, scan_frames};
use super::source::{MonotonicRun, MonotonicSource, MonotonicSourceRef, with_monotonic_source};
use super::{MappingChange, MonotonicError, MonotonicMapping};

/// The options of [`MonotonicEnvelope::append_monotonic`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonotonicAppend {
    /// Keep the window's length: the x range moves by exactly
    /// `new samples × interval`, and the columns that fully leave it on the
    /// left are dropped.
    pub sliding: bool,
}

/// The result of a call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonotonicUpdate {
    /// The samples the group holds after the call (its logical length).
    pub len: usize,
    /// Whether the source's channels differ in length, so only their common
    /// extent was taken.
    pub truncated: bool,
    /// Whole columns dropped from the front by a slide.
    pub dropped: usize,
    /// The x range after the call, for the host's x axis (a slide moves it).
    pub x_range: (f64, f64),
    /// The offset of the envelope's columns from the x range, in pixel
    /// columns, `0 <= offset < 1`: the draw shifts the columns left by it.
    pub pixel_offset: f64,
}

/// How dense the current window is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonotonicView {
    /// Never built, or the last full call failed.
    Empty,
    /// At most 4 visible samples per pixel column: every column keeps all of
    /// its samples, so the cells hold every visible sample.
    Sparse(VisibleSlice),
    /// More: dense columns hold their M4 points.
    Dense(VisibleSlice),
}

/// The scalars an envelope was built with.
#[derive(Clone, Debug, PartialEq)]
struct Built {
    /// The current mapping: the full call's, its x range moved by slides.
    mapping: MonotonicMapping,
    /// The full call's mapping, the anchor of the column grid.
    anchor: MonotonicMapping,
    grid: MonotonicGrid,
    channels: usize,
    /// The logical number of samples so far.
    len: usize,
    truncated: bool,
    /// Samples slid since the full call.
    slid: usize,
}

/// The CPU cache of one monotonic group: the M4 accumulator of every column
/// of every channel and the scalars it was built with. Never a copy of the
/// data (M3): its one buffer holds `channels × (width + 3)` column states,
/// sized by the plot width and the channel count, never by the sample count.
///
/// Per channel, the slots are: the lead edge sample (the last sample left of
/// column 0); columns `0..width`; column `width`, right of the plot (kept in
/// full so a slide by part of a column can show it); and the first sample
/// right of column `width` (the trail edge).
#[derive(Clone, Debug)]
pub struct MonotonicEnvelope {
    built: Option<Built>,
    /// The column states, channel-major, `channels × (width + 3)` (#271,
    /// M3).
    states: Vec<ColumnState>,
}

impl MonotonicEnvelope {
    /// An envelope that holds nothing yet. It allocates on the first full
    /// call.
    pub const fn empty() -> Self {
        Self {
            built: None,
            states: Vec::new(),
        }
    }

    /// The full call: reduces `source` (its samples, `start` and
    /// `interval`) on `mapping` from scratch, every time.
    /// Reads the lead edge sample, the samples of columns `0..=width` and the
    /// trail edge sample, nothing else.
    pub fn set_monotonic<S: MonotonicSource + ?Sized>(
        &mut self,
        source: &S,
        mapping: &MonotonicMapping,
    ) -> Result<MonotonicUpdate, MonotonicError> {
        let grid = MonotonicGrid::new(source.start(), source.interval(), mapping)?;
        self.build_on(source, grid, mapping)
    }

    /// [`Self::set_monotonic`] for a source of any element type.
    pub fn set_monotonic_ref(
        &mut self,
        source: MonotonicSourceRef<'_>,
        mapping: &MonotonicMapping,
    ) -> Result<MonotonicUpdate, MonotonicError> {
        with_monotonic_source!(source, s => self.set_monotonic(s, mapping))
    }

    /// [`Self::append_monotonic`] for a source of any element type.
    pub fn append_monotonic_ref(
        &mut self,
        new: MonotonicSourceRef<'_>,
        options: MonotonicAppend,
    ) -> Result<MonotonicUpdate, MonotonicError> {
        with_monotonic_source!(new, s => self.append_monotonic(s, options))
    }

    /// Reduces `source` in full on `grid` (which may be shifted), recording
    /// `mapping`. The full call is this on a fresh grid; tests also build on
    /// a slid envelope's own grid, to compare with it.
    pub(crate) fn build_on<S: MonotonicSource + ?Sized>(
        &mut self,
        source: &S,
        grid: MonotonicGrid,
        mapping: &MonotonicMapping,
    ) -> Result<MonotonicUpdate, MonotonicError> {
        let channels = source.channel_count();
        self.built = None;
        allocate(&mut self.states, channels, grid.width())?;
        let len = source.len();
        let range = read_range(&grid, &self.states, 0, len, channels);
        ingest(&mut self.states, &grid, channels, source, range, 0);
        let truncated = source.truncated();
        self.built = Some(Built {
            mapping: mapping.clone(),
            anchor: mapping.clone(),
            grid,
            channels,
            len,
            truncated,
            slid: 0,
        });
        self.update(0)
    }

    /// The append call: `new` holds only the new samples, every channel the
    /// same count, in either layout. Sample `j` of it is logical sample
    /// `len + j`, at `x = start + (len + j) * interval`, where `len` is the
    /// envelope's own count. Nothing is compared and the scale never changes:
    /// the new samples continue the grid of the full call, so the source's
    /// `start` and `interval` are not read.
    ///
    /// The new samples inside the kept columns are merged into the envelope
    /// (the last, partial column merges exactly without its old samples);
    /// later ones are counted, not kept.
    ///
    /// With `sliding`, the x range then moves by exactly
    /// `new.len() × interval`. The columns stay on the grid they were built
    /// on: the columns that fully leave the window on the left are dropped,
    /// and the fraction of a column the range moved past the grid is
    /// reported as [`MonotonicUpdate::pixel_offset`] for the draw. A slide
    /// never reads old samples.
    pub fn append_monotonic<S: MonotonicSource + ?Sized>(
        &mut self,
        new: &S,
        options: MonotonicAppend,
    ) -> Result<MonotonicUpdate, MonotonicError> {
        let Some(built) = self.built.as_ref() else {
            return Err(MonotonicError::NotBuilt);
        };
        if new.channel_count() != built.channels {
            return Err(MonotonicError::ChannelCountChanged {
                expected: built.channels,
                found: new.channel_count(),
            });
        }
        let m = new.len();
        let len = built
            .len
            .checked_add(m)
            .ok_or(MonotonicError::CountOverflow)?;
        let mut dropped = 0;
        let mut grid = built.grid;
        let mut next = built.clone();
        if options.sliding {
            let slid = built
                .slid
                .checked_add(m)
                .ok_or(MonotonicError::CountOverflow)?;
            let target = window_start(&grid, slid);
            dropped = target.saturating_sub(grid.shift());
            if dropped > 0 {
                // The old samples past the trail edge were counted, not kept:
                // a window that moves over them needs the source.
                let trail = grid.column_start(grid.width() + 1, built.len);
                if built.len > trail.saturating_add(1) {
                    return Err(MonotonicError::SlidePastKept);
                }
            }
            let moved = slid as f64 * grid.interval();
            next.mapping.x_min = built.anchor.x_min + moved;
            next.mapping.x_max = built.anchor.x_max + moved;
            next.slid = slid;
            if dropped > 0 {
                drop_front(&mut self.states, &grid, built.channels, dropped);
                grid = grid.shifted(dropped);
            }
        }
        let base = built.len;
        let range = read_range(&grid, &self.states, base, len, built.channels);
        let range = range.start.saturating_sub(base)..range.end.saturating_sub(base);
        ingest(&mut self.states, &grid, built.channels, new, range, base);
        next.grid = grid;
        next.len = len;
        next.truncated = new.truncated();
        self.built = Some(next);
        self.update(dropped)
    }

    /// Checks that the envelope was built for `current`. Any change to the
    /// pixel column of a sample (x range, plot width in device pixels, x
    /// scale, x breaks, x inversion) needs the source: the envelope keeps no
    /// copy of the data, so it cannot rebuild itself, and is never drawn for
    /// another mapping. A slide moves the recorded x range with it.
    pub fn check_mapping(&self, current: &MonotonicMapping) -> Result<(), MonotonicError> {
        let Some(built) = &self.built else {
            return Err(MonotonicError::SourceRequired {
                change: MappingChange::NotBuilt,
            });
        };
        match built.mapping.change_to(current) {
            Some(change) => Err(MonotonicError::SourceRequired { change }),
            None => Ok(()),
        }
    }

    /// How dense the current window is.
    pub fn view(&self) -> MonotonicView {
        let Some(b) = &self.built else {
            return MonotonicView::Empty;
        };
        let slice = b.grid.visible(b.len, b.mapping.x_min, b.mapping.x_max);
        if slice.samples() > b.grid.plot_width().saturating_mul(4) {
            MonotonicView::Dense(slice)
        } else {
            MonotonicView::Sparse(slice)
        }
    }

    /// The current mapping. A slide moves its x range.
    pub fn mapping(&self) -> Option<&MonotonicMapping> {
        self.built.as_ref().map(|b| &b.mapping)
    }

    /// The mapping the columns are aligned to: the current one with the x
    /// range of the column grid. A full call on it builds the same columns.
    pub fn aligned_mapping(&self) -> Option<MonotonicMapping> {
        let b = self.built.as_ref()?;
        let (x_min, x_max) = b.grid.aligned_range();
        Some(MonotonicMapping {
            x_min,
            x_max,
            ..b.mapping.clone()
        })
    }

    /// The offset of the columns from the current x range, in pixel columns,
    /// `0 <= offset < 1`. Zero after a full call.
    pub fn pixel_offset(&self) -> f64 {
        self.built.as_ref().map_or(0.0, |b| {
            let s = b.slid as f64 * b.grid.interval() / b.grid.x_per_pixel();
            s - b.grid.shift() as f64
        })
    }

    /// Whether the columns are aligned to the current x range: no pixel
    /// offset, and a full call on the current mapping would put every sample
    /// in the same column (checked by arithmetic on the column boundaries,
    /// without the data).
    pub fn is_grid_aligned(&self) -> bool {
        let Some(b) = &self.built else {
            return false;
        };
        if self.pixel_offset() != 0.0 {
            return false;
        }
        let Ok(fresh) = MonotonicGrid::new(b.grid.start(), b.grid.interval(), &b.mapping) else {
            return false;
        };
        (0..=b.grid.width() + 1)
            .all(|c| fresh.column_start(c, b.len) == b.grid.column_start(c, b.len))
    }

    /// The column grid.
    pub fn grid(&self) -> Option<&MonotonicGrid> {
        self.built.as_ref().map(|b| &b.grid)
    }

    /// The logical number of samples the group holds.
    pub fn len(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.len)
    }

    /// Whether the group holds no sample.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the last call's source had channels of different lengths.
    pub fn truncated(&self) -> bool {
        self.built.as_ref().is_some_and(|b| b.truncated)
    }

    /// The number of channels.
    pub fn channel_count(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.channels)
    }

    /// The slots per channel: `width + 3` (lead edge, `width + 1` columns,
    /// trail edge).
    pub fn slots(&self) -> usize {
        self.built.as_ref().map_or(0, |b| b.grid.width() + 3)
    }

    /// Slot `slot` of channel `channel` in its drawn form: slot 0 is the lead
    /// edge sample, slot `1 + c` column `c` (`c <= width`), and slot
    /// `width + 2` the trail edge sample.
    pub fn cell(&self, channel: usize, slot: usize) -> Option<EnvelopeCell> {
        let per = self.slots();
        if slot >= per || channel >= self.channel_count() {
            return None;
        }
        self.states.get(channel * per + slot).map(ColumnState::cell)
    }

    /// Every slot of one channel in its drawn form, in x order.
    pub fn channel_cells(&self, channel: usize) -> impl Iterator<Item = EnvelopeCell> + '_ {
        let per = self.slots();
        let from = if channel < self.channel_count() {
            channel * per
        } else {
            0
        };
        let take = if channel < self.channel_count() {
            per
        } else {
            0
        };
        self.states
            .iter()
            .skip(from)
            .take(take)
            .map(ColumnState::cell)
    }

    /// The logical samples of envelope column `c`, by arithmetic.
    pub fn column_samples(&self, c: usize) -> Option<Range<usize>> {
        let b = self.built.as_ref()?;
        let from = b.grid.column_start(c, b.len);
        Some(from..b.grid.column_start(c + 1, b.len).max(from))
    }

    /// The envelope column under an x, `None` left of column 0 or past the
    /// kept columns.
    pub fn column_of_x(&self, x: f64) -> Option<usize> {
        let b = self.built.as_ref()?;
        b.grid.column_of_x(x).filter(|&c| c <= b.grid.width())
    }

    /// A pick without the source: the kept point of `channel` in envelope
    /// column `column` nearest to `y`, as (logical sample index, value). A
    /// dense column's drawn pixels come from exactly these points.
    pub fn pick(&self, channel: usize, column: usize, y: f64) -> Option<(usize, f64)> {
        self.cell(channel, column.checked_add(1)?)?.nearest(y)
    }

    fn update(&self, dropped: usize) -> Result<MonotonicUpdate, MonotonicError> {
        let b = self.built.as_ref().ok_or(MonotonicError::NotBuilt)?;
        Ok(MonotonicUpdate {
            len: b.len,
            truncated: b.truncated,
            dropped,
            x_range: (b.mapping.x_min, b.mapping.x_max),
            pixel_offset: self.pixel_offset(),
        })
    }
}

/// The whole columns the window start has moved past after `slid` samples.
fn window_start(grid: &MonotonicGrid, slid: usize) -> usize {
    let s = (slid as f64 * grid.interval() / grid.x_per_pixel()).floor();
    if s > 0.0 { s as usize } else { 0 }
}

/// Sizes the state buffer for `channels × (width + 3)` empty states,
/// reusing its allocation. Fails only when the memory cannot be allocated.
fn allocate(
    states: &mut Vec<ColumnState>,
    channels: usize,
    width: usize,
) -> Result<(), MonotonicError> {
    let total = width
        .checked_add(3)
        .and_then(|per| per.checked_mul(channels))
        .ok_or(MonotonicError::OutOfMemory)?;
    states.clear();
    states
        .try_reserve_exact(total)
        .map_err(|_| MonotonicError::OutOfMemory)?;
    states.resize(total, ColumnState::EMPTY);
    Ok(())
}

/// Drops the first `dropped` columns of every channel. The new lead edge is
/// the last kept point of the columns that left (or the old lead). The trail
/// edge sample moves to the column it now falls in.
fn drop_front(states: &mut [ColumnState], old: &MonotonicGrid, channels: usize, dropped: usize) {
    let new = old.shifted(dropped);
    let width = old.width();
    let per = width + 3;
    let trail_slot = width + 2;
    let leaving = dropped.min(width + 1);
    for channel in 0..channels {
        let Some(block) = states.get_mut(channel * per..(channel + 1) * per) else {
            return;
        };
        let trail = std::mem::replace(&mut block[trail_slot], ColumnState::EMPTY);
        let trail_point = trail.last_point();
        let trail_column = trail_point.and_then(|(i, _)| new.column(i));
        let lead = match trail_point {
            Some(p) if trail_column.is_none() => Some(p),
            _ => (0..=leaving).rev().find_map(|s| block[s].last_point()),
        };
        block.copy_within(1 + leaving..trail_slot, 1);
        block[trail_slot - leaving..trail_slot].fill(ColumnState::EMPTY);
        block[0] = lead.map_or(ColumnState::EMPTY, |(i, v)| ColumnState::single(i, v));
        if let Some(c) = trail_column {
            block[(c + 1).min(trail_slot)] = trail;
        }
    }
}

/// The logical samples a call reads, within `from..len`: from the lead edge
/// sample (the last left of column 0) through the trail edge sample (the
/// first right of column `width`), less what the envelope already holds.
fn read_range(
    grid: &MonotonicGrid,
    states: &[ColumnState],
    from: usize,
    len: usize,
    channels: usize,
) -> Range<usize> {
    let i0 = grid.column_start(0, len.saturating_add(1));
    let start = if i0 >= 1 && i0 <= len {
        i0 - 1
    } else {
        i0.min(len)
    };
    let trail = grid.column_start(grid.width() + 1, len);
    let trail_slot = grid.width() + 2;
    let trail_held = channels > 0 && states.get(trail_slot).is_some_and(|s| !s.is_empty());
    let end = if trail < len && !trail_held {
        trail + 1
    } else {
        trail
    };
    let start = start.max(from);
    start..end.max(start)
}

/// Merges logical samples `base + range` (source samples `range`) into the
/// states, each into its column.
fn ingest<S: MonotonicSource + ?Sized>(
    states: &mut [ColumnState],
    grid: &MonotonicGrid,
    channels: usize,
    source: &S,
    range: Range<usize>,
    base: usize,
) {
    if range.is_empty() {
        return;
    }
    let mut done = false;
    source.lend(range, &mut |run| {
        if !done {
            done = ingest_run(states, grid, channels, &run, base);
        }
    });
}

/// Merges one lent run. Returns `true` once the trail edge is reached, after
/// which nothing more is kept.
fn ingest_run<T: MonotonicElement>(
    states: &mut [ColumnState],
    grid: &MonotonicGrid,
    channels: usize,
    run: &MonotonicRun<'_, T>,
    base: usize,
) -> bool {
    let width = grid.width();
    let per = width + 3;
    let run_base = base + run.first();
    let frames = run.frames();
    let value = |n: usize, r: usize| run.channel(n).and_then(|c| c.get(r)).map(|v| v.to_f64());
    let mut r = 0;
    while r < frames {
        let i = run_base + r;
        match grid.column(i) {
            None => {
                // Left of column 0: the latest is the lead edge sample.
                for n in 0..channels {
                    if let (Some(v), Some(state)) = (value(n, r), states.get_mut(n * per)) {
                        *state = ColumnState::single(i, v);
                    }
                }
                r += 1;
            }
            Some(c) if c <= width => {
                let next = grid.column_start(c + 1, run_base + frames);
                let end = next.max(i + 1) - run_base;
                let slot = c + 1;
                match run.interleaved_parts() {
                    Some((frames_buf, stride, columns)) => interleaved_column(
                        states, slot, per, channels, run, frames_buf, stride, columns, r, end,
                        run_base,
                    ),
                    None => {
                        for n in 0..channels {
                            if let (Some(channel), Some(state)) =
                                (run.channel(n), states.get_mut(n * per + slot))
                            {
                                extend(state, &channel, r, end, run_base);
                            }
                        }
                    }
                }
                r = end;
            }
            Some(_) => {
                // Right of the kept columns: only the trail edge sample.
                for n in 0..channels {
                    if let (Some(v), Some(state)) =
                        (value(n, r), states.get_mut(n * per + width + 2))
                        && state.is_empty()
                    {
                        state.push(i, v);
                    }
                }
                return true;
            }
        }
    }
    false
}

/// Merges frames `from..end` of an interleaved run into one column of every
/// channel, scanning a batch of channels per pass over the frames.
#[allow(clippy::too_many_arguments)]
fn interleaved_column<T: MonotonicElement>(
    states: &mut [ColumnState],
    slot: usize,
    per: usize,
    channels: usize,
    run: &MonotonicRun<'_, T>,
    frames: &[T],
    stride: usize,
    columns: &[usize],
    from: usize,
    end: usize,
    run_base: usize,
) {
    if from >= end {
        return;
    }
    // Every channel's column holds the same number of samples, so the
    // one-by-one fill stops at the same frame for all of them.
    let mut i = from;
    for n in 0..channels {
        if let (Some(channel), Some(state)) = (run.channel(n), states.get_mut(n * per + slot)) {
            i = fill(state, &channel, from, end, run_base);
        }
    }
    let wide = end >= i + 4;
    for (b, batch) in columns.chunks(BATCH).enumerate() {
        let scanned = if wide {
            scan_frames(frames, stride, batch, i, end - 2)
        } else {
            [None; BATCH]
        };
        for (j, found) in scanned.into_iter().take(batch.len()).enumerate() {
            let n = b * BATCH + j;
            if let (Some(channel), Some(state)) = (run.channel(n), states.get_mut(n * per + slot)) {
                finish(state, &channel, i, end, run_base, found);
            }
        }
    }
}
