//! The source of a monotonic group (#271): the [`MonotonicSource`] trait,
//! through which the host lends its samples for one call, and the built-in
//! impls for plain and interleaved slices.
//!
//! A source lends the samples a call asks for as contiguous runs (a ring
//! buffer lends at most two), each read in place, so the reduction gets
//! contiguous memory to vectorize and nothing is copied or allocated. The
//! envelope keeps no reference to a source after a call (M1, M3).

use std::ops::Range;

use super::MonotonicError;
use super::element::MonotonicElement;

/// The host's samples, lent for one call.
///
/// Samples are numbered `0..len()` in the source; every channel holds the
/// same number (a frame holds one sample of every channel).
pub trait MonotonicSource {
    /// The element type of every channel.
    type Element: MonotonicElement;

    /// The x of sample 0: sample `i` is at `start + i × interval`. Read by
    /// the full call; an append continues the grid of the full call and
    /// reads neither this nor [`Self::interval`].
    fn start(&self) -> f64;

    /// The x distance between two samples, finite and positive.
    fn interval(&self) -> f64;

    /// The number of samples every channel holds.
    fn len(&self) -> usize;

    /// Whether the source holds no sample.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of channels.
    fn channel_count(&self) -> usize;

    /// Whether the host's channels differ in length, so the source lends
    /// only their common extent.
    fn truncated(&self) -> bool {
        false
    }

    /// Lends samples `range` (clamped to `0..len()`) to `visit` as
    /// contiguous runs, in order, covering the range without gaps or
    /// overlaps. The core asks only for the samples it reads.
    fn lend(&self, range: Range<usize>, visit: &mut dyn FnMut(MonotonicRun<'_, Self::Element>));
}

/// A contiguous run of samples lent by a [`MonotonicSource`]: samples
/// `first..first + frames` of every channel, read in place.
#[derive(Clone, Copy, Debug)]
pub struct MonotonicRun<'a, T> {
    first: usize,
    frames: usize,
    layout: RunLayout<'a, T>,
}

#[derive(Clone, Copy, Debug)]
enum RunLayout<'a, T> {
    /// Channel `n`, run sample `i`: `slices[n][offset + i]`.
    Slices {
        slices: &'a [&'a [T]],
        offset: usize,
    },
    /// Channel `n`, run sample `i`:
    /// `values[(offset + i) * stride + columns[n]]`.
    Interleaved {
        values: &'a [T],
        stride: usize,
        columns: &'a [usize],
        offset: usize,
    },
}

impl<'a, T: MonotonicElement> MonotonicRun<'a, T> {
    /// A run of `frames` samples whose first is source sample `first`, held
    /// at `slices[n][offset..offset + frames]` for channel `n`. `None` when a
    /// slice is too short.
    pub fn slices(
        first: usize,
        frames: usize,
        slices: &'a [&'a [T]],
        offset: usize,
    ) -> Option<Self> {
        let end = offset.checked_add(frames)?;
        slices.iter().all(|s| s.len() >= end).then_some(Self {
            first,
            frames,
            layout: RunLayout::Slices { slices, offset },
        })
    }

    /// A run of `frames` frames whose first is source sample `first`, held
    /// from frame `offset` of an interleaved buffer with `stride` values per
    /// frame; channel `n` reads column `columns[n]`. `None` when the buffer
    /// is too short, the stride is zero or a column lies outside the frame.
    pub fn interleaved(
        first: usize,
        frames: usize,
        values: &'a [T],
        stride: usize,
        columns: &'a [usize],
        offset: usize,
    ) -> Option<Self> {
        let end = offset.checked_add(frames)?.checked_mul(stride)?;
        let valid = stride > 0 && values.len() >= end && columns.iter().all(|&c| c < stride);
        valid.then_some(Self {
            first,
            frames,
            layout: RunLayout::Interleaved {
                values,
                stride,
                columns,
                offset,
            },
        })
    }

    /// The source index of the run's first sample.
    pub fn first(&self) -> usize {
        self.first
    }

    /// The number of samples per channel.
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Channel `n` of the run: its sample `i` is source sample `first + i`.
    pub(crate) fn channel(&self, n: usize) -> Option<StridedChannel<'a, T>> {
        match self.layout {
            RunLayout::Slices { slices, offset } => {
                let s = slices
                    .get(n)?
                    .get(offset..offset.checked_add(self.frames)?)?;
                Some(StridedChannel {
                    values: s,
                    offset: 0,
                    stride: 1,
                })
            }
            RunLayout::Interleaved { columns, .. } => {
                let (frames, stride, _) = self.interleaved_parts()?;
                Some(StridedChannel {
                    values: frames,
                    offset: *columns.get(n)?,
                    stride,
                })
            }
        }
    }

    /// The frames of an interleaved run (`frames × stride` values from the
    /// run's first frame), the stride and the drawn columns.
    pub(crate) fn interleaved_parts(&self) -> Option<(&'a [T], usize, &'a [usize])> {
        match self.layout {
            RunLayout::Slices { .. } => None,
            RunLayout::Interleaved {
                values,
                stride,
                columns,
                offset,
            } => {
                let from = offset.checked_mul(stride)?;
                let to = offset.checked_add(self.frames)?.checked_mul(stride)?;
                Some((values.get(from..to)?, stride, columns))
            }
        }
    }
}

/// One channel of a run, read in place: sample `i` is
/// `values[offset + i * stride]`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StridedChannel<'a, T> {
    values: &'a [T],
    offset: usize,
    stride: usize,
}

impl<'a, T: MonotonicElement> StridedChannel<'a, T> {
    /// Sample `i`, or `None` past the run.
    #[inline]
    pub(crate) fn get(&self, i: usize) -> Option<T> {
        let at = i.checked_mul(self.stride)?.checked_add(self.offset)?;
        self.values.get(at).copied()
    }

    /// Samples `a..b` as one contiguous slice, when the stride is 1.
    #[inline]
    pub(crate) fn contiguous(&self, a: usize, b: usize) -> Option<&'a [T]> {
        if self.stride == 1 {
            self.values.get(a..b)
        } else {
            None
        }
    }
}

/// The built-in source over one slice per channel. When the slices differ in
/// length it lends their common extent and reports
/// [`MonotonicSource::truncated`].
#[derive(Clone, Copy, Debug)]
pub struct MonotonicSlices<'a, T> {
    start: f64,
    interval: f64,
    channels: &'a [&'a [T]],
}

impl<'a, T: MonotonicElement> MonotonicSlices<'a, T> {
    /// Sample `i` of every channel is at `x = start + i × interval`.
    pub fn new(start: f64, interval: f64, channels: &'a [&'a [T]]) -> Self {
        Self {
            start,
            interval,
            channels,
        }
    }
}

impl<T: MonotonicElement> MonotonicSource for MonotonicSlices<'_, T> {
    type Element = T;

    fn start(&self) -> f64 {
        self.start
    }

    fn interval(&self) -> f64 {
        self.interval
    }

    fn len(&self) -> usize {
        self.channels.iter().map(|c| c.len()).min().unwrap_or(0)
    }

    fn channel_count(&self) -> usize {
        self.channels.len()
    }

    fn truncated(&self) -> bool {
        let len = self.len();
        self.channels.iter().any(|c| c.len() != len)
    }

    fn lend(&self, range: Range<usize>, visit: &mut dyn FnMut(MonotonicRun<'_, T>)) {
        let end = range.end.min(self.len());
        if range.start < end
            && let Some(run) =
                MonotonicRun::slices(range.start, end - range.start, self.channels, range.start)
        {
            visit(run);
        }
    }
}

/// The built-in source over one interleaved, sample-major buffer: frame `i`
/// is `values[i * stride .. (i + 1) * stride]`, and channel `n` draws column
/// `columns[n]`. A column no channel names is never read. It holds
/// `values.len() / stride` frames: a trailing partial frame is not counted
/// until it is complete.
#[derive(Clone, Copy, Debug)]
pub struct MonotonicInterleaved<'a, T> {
    start: f64,
    interval: f64,
    values: &'a [T],
    stride: usize,
    columns: &'a [usize],
}

impl<'a, T: MonotonicElement> MonotonicInterleaved<'a, T> {
    /// Frame `i` is at `x = start + i × interval`. Fails for a zero stride
    /// or a column outside the frame.
    pub fn new(
        start: f64,
        interval: f64,
        values: &'a [T],
        stride: usize,
        columns: &'a [usize],
    ) -> Result<Self, MonotonicError> {
        if stride == 0 {
            return Err(MonotonicError::InvalidStride);
        }
        if let Some(&column) = columns.iter().find(|&&c| c >= stride) {
            return Err(MonotonicError::InvalidColumn { column, stride });
        }
        Ok(Self {
            start,
            interval,
            values,
            stride,
            columns,
        })
    }
}

impl<T: MonotonicElement> MonotonicSource for MonotonicInterleaved<'_, T> {
    type Element = T;

    fn start(&self) -> f64 {
        self.start
    }

    fn interval(&self) -> f64 {
        self.interval
    }

    fn len(&self) -> usize {
        self.values.len() / self.stride
    }

    fn channel_count(&self) -> usize {
        self.columns.len()
    }

    fn lend(&self, range: Range<usize>, visit: &mut dyn FnMut(MonotonicRun<'_, T>)) {
        let end = range.end.min(self.len());
        if range.start < end
            && let Some(run) = MonotonicRun::interleaved(
                range.start,
                end - range.start,
                self.values,
                self.stride,
                self.columns,
                range.start,
            )
        {
            visit(run);
        }
    }
}

/// A borrowed source of element type `T` as [`MonotonicSourceRef`] holds
/// it: `Sync` on native targets, so a call that borrows sources (an export)
/// keeps its future `Send`; any source in a browser, which runs one thread.
#[cfg(not(target_arch = "wasm32"))]
pub type DynMonotonicSource<'a, T> = dyn MonotonicSource<Element = T> + Sync + 'a;
/// A borrowed source of element type `T` as [`MonotonicSourceRef`] holds
/// it: `Sync` on native targets, so a call that borrows sources (an export)
/// keeps its future `Send`; any source in a browser, which runs one thread.
#[cfg(target_arch = "wasm32")]
pub type DynMonotonicSource<'a, T> = dyn MonotonicSource<Element = T> + 'a;

/// A [`MonotonicSource`] of any supported element type, borrowed for one
/// call (#271): what a call that takes sources of several element types at
/// once (an export) passes. It holds a reference, never the samples.
#[derive(Clone, Copy)]
pub enum MonotonicSourceRef<'a> {
    F64(&'a DynMonotonicSource<'a, f64>),
    F32(&'a DynMonotonicSource<'a, f32>),
    I32(&'a DynMonotonicSource<'a, i32>),
    I16(&'a DynMonotonicSource<'a, i16>),
    U16(&'a DynMonotonicSource<'a, u16>),
    U8(&'a DynMonotonicSource<'a, u8>),
}

impl std::fmt::Debug for MonotonicSourceRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonotonicSourceRef")
            .field("len", &self.len())
            .field("channels", &self.channel_count())
            .finish()
    }
}

/// Runs `$body` with `$source` bound to the typed source inside a
/// [`MonotonicSourceRef`].
macro_rules! with_monotonic_source {
    ($r:expr, $source:ident => $body:expr) => {
        match $r {
            MonotonicSourceRef::F64($source) => $body,
            MonotonicSourceRef::F32($source) => $body,
            MonotonicSourceRef::I32($source) => $body,
            MonotonicSourceRef::I16($source) => $body,
            MonotonicSourceRef::U16($source) => $body,
            MonotonicSourceRef::U8($source) => $body,
        }
    };
}
pub(super) use with_monotonic_source;

impl<'a> MonotonicSourceRef<'a> {
    /// `source`, whatever its element type.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new<S: MonotonicSource + Sync + 'a>(source: &'a S) -> Self {
        S::Element::erase(source)
    }

    /// `source`, whatever its element type.
    #[cfg(target_arch = "wasm32")]
    pub fn new<S: MonotonicSource + 'a>(source: &'a S) -> Self {
        S::Element::erase(source)
    }

    /// The x of sample 0.
    pub fn start(&self) -> f64 {
        with_monotonic_source!(*self, s => s.start())
    }

    /// The x distance between two samples.
    pub fn interval(&self) -> f64 {
        with_monotonic_source!(*self, s => s.interval())
    }

    /// The number of samples every channel holds.
    pub fn len(&self) -> usize {
        with_monotonic_source!(*self, s => s.len())
    }

    /// Whether the source holds no sample.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of channels.
    pub fn channel_count(&self) -> usize {
        with_monotonic_source!(*self, s => s.channel_count())
    }

    /// Whether the host's channels differ in length.
    pub fn truncated(&self) -> bool {
        with_monotonic_source!(*self, s => s.truncated())
    }
}
