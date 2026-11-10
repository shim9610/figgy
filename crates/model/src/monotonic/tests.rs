//! Unit tests of the monotonic core (#271). The oracle draws every sample
//! and compares, per column, the span the line touches and the column's
//! entry and exit samples with the polyline the envelope implies.

use std::cell::{Cell, RefCell};
use std::ops::Range;

use super::element::sealed::Sealed;
use super::*;

// ---------------------------------------------------------------------------
// Helpers

/// A small deterministic generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[-1, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }
}

/// A random walk with spikes, so extremes fall anywhere in a column.
fn walk(n: usize, seed: u64) -> Vec<f64> {
    let mut rng = Rng(seed | 1);
    let mut y = 0.0;
    (0..n)
        .map(|_| {
            y += rng.unit();
            if rng.next().is_multiple_of(97) {
                y + 40.0 * rng.unit()
            } else {
                y
            }
        })
        .collect()
}

/// A walk with gaps: single NaNs, runs of NaNs and infinities.
fn gappy(n: usize, seed: u64) -> Vec<f64> {
    let mut data = walk(n, seed);
    let mut rng = Rng(seed ^ 0x55);
    for v in data.iter_mut() {
        let r = rng.next() % 1000;
        if r < 20 {
            *v = f64::NAN;
        } else if r < 21 {
            *v = f64::INFINITY;
        }
    }
    let mut i = 0;
    while i + 40 < n {
        if rng.next().is_multiple_of(7) {
            let run = (rng.next() % 30) as usize;
            data[i..i + run].fill(f64::NAN);
        }
        i += 97;
    }
    data
}

fn full<S: MonotonicSource + ?Sized>(
    start: f64,
    interval: f64,
    source: &S,
    mapping: &MonotonicMapping,
) -> MonotonicEnvelope {
    let mut envelope = MonotonicEnvelope::empty();
    envelope
        .set_monotonic(&MonotonicGroup::new(start, interval, source), mapping)
        .expect("set");
    envelope
}

/// A full reduction of `source` on `envelope`'s own (anchored, shifted)
/// column grid.
fn on_grid<S: MonotonicSource + ?Sized>(
    envelope: &MonotonicEnvelope,
    source: &S,
) -> MonotonicEnvelope {
    let mut fresh = MonotonicEnvelope::empty();
    fresh
        .build_on(
            source,
            *envelope.grid().unwrap(),
            envelope.mapping().unwrap(),
        )
        .expect("build on grid");
    fresh
}

/// A cell as bits: count, links, indices and value bits.
type CellBits = (u8, u8, [usize; 4], [u64; 4]);

/// Every cell of every channel as bits, so NaN compares equal to itself.
fn bits(envelope: &MonotonicEnvelope) -> Vec<CellBits> {
    (0..envelope.channel_count())
        .flat_map(|n| envelope.channel_cells(n))
        .map(|c| (c.count, c.links, c.index, c.value.map(f64::to_bits)))
        .collect()
}

/// Same cells and length.
fn assert_cells(a: &MonotonicEnvelope, b: &MonotonicEnvelope) {
    assert_eq!(a.len(), b.len(), "len");
    assert_eq!(a.channel_count(), b.channel_count(), "channels");
    let (x, y) = (bits(a), bits(b));
    assert_eq!(x.len(), y.len(), "cell count");
    for (k, (p, q)) in x.iter().zip(&y).enumerate() {
        assert_eq!(p, q, "cell {k}");
    }
}

/// Same cells, length, mapping and view.
fn assert_same(a: &MonotonicEnvelope, b: &MonotonicEnvelope) {
    assert_cells(a, b);
    assert_eq!(a.mapping(), b.mapping(), "mapping");
    assert_eq!(a.view(), b.view(), "view");
}

/// A segment in column units: `(x0, y0, x1, y1)`, `x0 <= x1`.
type Segment = (f64, f64, f64, f64);

/// The span segment `s` touches in column `c`, `[c, c + 1)`.
fn touch(s: Segment, c: usize) -> Option<(f64, f64)> {
    let (x0, y0, x1, y1) = s;
    let (l, r) = (c as f64, c as f64 + 1.0);
    if x1 < l || x0 >= r {
        return None;
    }
    let at = |x: f64| y0 + (y1 - y0) * (x - x0) / (x1 - x0);
    let ya = if x0 >= l { y0 } else { at(l) };
    let yb = if x1 <= r { y1 } else { at(r) };
    Some((ya.min(yb), ya.max(yb)))
}

/// The span every column `0..columns` touches.
fn spans(segments: &[Segment], columns: usize) -> Vec<Option<(f64, f64)>> {
    let mut out = vec![None; columns];
    for &s in segments {
        let from = s.0.floor().max(0.0) as usize;
        let to = (s.2.floor().max(0.0) as usize).min(columns - 1);
        for (c, slot) in out.iter_mut().enumerate().take(to + 1).skip(from) {
            if let Some((lo, hi)) = touch(s, c) {
                *slot = Some(match *slot {
                    Some((a, b)) => (lo.min(a), hi.max(b)),
                    None => (lo, hi),
                });
            }
        }
    }
    out
}

/// Every segment drawn when every sample in `from..=to` is drawn.
fn reference_segments(
    grid: &MonotonicGrid,
    from: usize,
    to: usize,
    value: &dyn Fn(usize) -> f64,
) -> Vec<Segment> {
    let mut out = Vec::new();
    for i in from..to {
        let (a, b) = (value(i), value(i + 1));
        if a.is_finite() && b.is_finite() {
            out.push((grid.position(i), a, grid.position(i + 1), b));
        }
    }
    out
}

/// The segments of the polyline the envelope implies for one channel.
fn envelope_segments(grid: &MonotonicGrid, cells: &[EnvelopeCell]) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut previous: Option<(usize, f64)> = None;
    let seg = |p: (usize, f64), q: (usize, f64)| (grid.position(p.0), p.1, grid.position(q.0), q.1);
    for cell in cells {
        let points: Vec<_> = cell.points().collect();
        let Some(&first) = points.first() else {
            continue;
        };
        if let Some(p) = previous
            && EnvelopeCell::joins(p, first)
        {
            out.push(seg(p, first));
        }
        for k in 0..points.len() - 1 {
            if cell.linked(k) {
                out.push(seg(points[k], points[k + 1]));
            }
        }
        previous = points.last().copied();
    }
    out
}

/// The oracle: in every kept column (`0..=width`) the envelope of
/// `channel` touches the same span as drawing every sample, keeps the
/// column's first and last samples (all of them for a column of at most 4),
/// and the edge slots hold the samples just outside the kept columns.
/// `value(i)` is logical sample `i`.
fn check_oracle(envelope: &MonotonicEnvelope, channel: usize, value: &dyn Fn(usize) -> f64) {
    let grid = envelope.grid().expect("grid");
    let width = grid.width();
    let len = envelope.len();
    let cells: Vec<EnvelopeCell> = envelope.channel_cells(channel).collect();
    assert_eq!(cells.len(), width + 3);
    let samples = |c: usize| envelope.column_samples(c).expect("column");
    let lead = samples(0).start.checked_sub(1);
    let trail = Some(samples(width + 1).start).filter(|&t| t < len);
    assert_eq!(cells[0].first().map(|p| p.0), lead, "lead edge");
    assert_eq!(cells[width + 2].first().map(|p| p.0), trail, "trail edge");
    let from = lead.unwrap_or(samples(0).start);
    let to = trail.unwrap_or(samples(width).end.saturating_sub(1));
    if from >= len {
        return;
    }
    let reference = spans(&reference_segments(grid, from, to, value), width + 1);
    let reduced = spans(&envelope_segments(grid, &cells), width + 1);
    for c in 0..=width {
        let range = samples(c);
        for i in range.clone() {
            assert_eq!(grid.column(i), Some(c), "sample {i} column");
        }
        assert_eq!(reference[c], reduced[c], "column {c} span ({range:?})");
        let cell = cells[c + 1];
        if range.is_empty() {
            assert_eq!(cell.count, 0, "column {c} is empty");
            continue;
        }
        let (first, last) = (range.start, range.end - 1);
        assert_eq!(cell.first().map(|p| p.0), Some(first), "column {c} entry");
        assert_eq!(cell.last().map(|p| p.0), Some(last), "column {c} exit");
        if range.len() <= 4 {
            let kept: Vec<_> = cell.points().map(|p| p.0).collect();
            assert_eq!(kept, range.collect::<Vec<_>>(), "sparse column {c} kept");
        }
        for (i, v) in cell.points() {
            assert_eq!(v.to_bits(), value(i).to_bits(), "column {c} value at {i}");
        }
    }
}

/// One-channel `f64` slices.
fn slices<'a>(channels: &'a [&'a [f64]]) -> MonotonicSlices<'a, f64> {
    MonotonicSlices::new(channels)
}

// ---------------------------------------------------------------------------
// Test sources: a ring buffer that wraps, a counting wrapper, and a probe
// element that proves which values are read.

/// A ring buffer of per-channel slices: logical sample `i` sits at
/// `(head + i) % capacity`. It lends a wrapped range as two runs.
struct Ring<'a> {
    channels: &'a [&'a [f64]],
    head: usize,
    len: usize,
}

impl MonotonicSource for Ring<'_> {
    type Element = f64;

    fn len(&self) -> usize {
        self.len
    }

    fn channel_count(&self) -> usize {
        self.channels.len()
    }

    fn lend(&self, range: Range<usize>, visit: &mut dyn FnMut(MonotonicRun<'_, f64>)) {
        let cap = self.channels.first().map_or(0, |c| c.len());
        let end = range.end.min(self.len);
        let mut i = range.start;
        while i < end {
            let at = (self.head + i) % cap;
            let frames = (end - i).min(cap - at);
            visit(MonotonicRun::slices(i, frames, self.channels, at).expect("ring run"));
            i += frames;
        }
    }
}

/// Owned ring buffers holding `data[c][range]`, wrapped mid-way.
fn ring_buffers(data: &[Vec<f64>], range: Range<usize>) -> (Vec<Vec<f64>>, usize) {
    let n = range.len();
    let cap = n + 3;
    let head = n / 2 + 3;
    let buffers = data
        .iter()
        .map(|d| {
            let mut buf = vec![f64::NAN; cap];
            for (i, &v) in d[range.clone()].iter().enumerate() {
                buf[(head + i) % cap] = v;
            }
            buf
        })
        .collect();
    (buffers, head)
}

/// Records every range a source lends.
struct Counting<'a, S: ?Sized> {
    inner: &'a S,
    lent: RefCell<Vec<Range<usize>>>,
}

impl<'a, S: MonotonicSource + ?Sized> Counting<'a, S> {
    fn new(inner: &'a S) -> Self {
        Self {
            inner,
            lent: RefCell::new(Vec::new()),
        }
    }

    /// The lent ranges, merged where they touch.
    fn lent(&self) -> Vec<Range<usize>> {
        let mut out: Vec<Range<usize>> = Vec::new();
        for r in self.lent.borrow().iter().filter(|r| !r.is_empty()) {
            match out.last_mut() {
                Some(last) if last.end == r.start => last.end = r.end,
                _ => out.push(r.clone()),
            }
        }
        out
    }
}

impl<S: MonotonicSource + ?Sized> MonotonicSource for Counting<'_, S> {
    type Element = S::Element;

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn channel_count(&self) -> usize {
        self.inner.channel_count()
    }

    fn truncated(&self) -> bool {
        self.inner.truncated()
    }

    fn lend(&self, range: Range<usize>, visit: &mut dyn FnMut(MonotonicRun<'_, S::Element>)) {
        self.lent.borrow_mut().push(range.clone());
        self.inner.lend(range, visit);
    }
}

/// The source layouts the equivalence tests run on.
#[derive(Clone, Copy, Debug)]
enum Layout {
    Slices,
    /// Frames of `channels + 1` values; column 0 is unused (NaN).
    Interleaved,
    Ring,
}

const LAYOUTS: [Layout; 3] = [Layout::Slices, Layout::Interleaved, Layout::Ring];

/// Runs `f` on `data[c][range]` lent in `layout`.
fn with_source<R>(
    data: &[Vec<f64>],
    range: Range<usize>,
    layout: Layout,
    f: impl FnOnce(&dyn MonotonicSource<Element = f64>) -> R,
) -> R {
    match layout {
        Layout::Slices => {
            let views: Vec<&[f64]> = data.iter().map(|d| &d[range.clone()]).collect();
            f(&MonotonicSlices::new(&views))
        }
        Layout::Interleaved => {
            let stride = data.len() + 1;
            let frames: Vec<f64> = range
                .clone()
                .flat_map(|i| std::iter::once(f64::NAN).chain(data.iter().map(move |d| d[i])))
                .collect();
            let columns: Vec<usize> = (1..stride).collect();
            f(&MonotonicInterleaved::new(&frames, stride, &columns).expect("interleaved"))
        }
        Layout::Ring => {
            let len = range.len();
            let (buffers, head) = ring_buffers(data, range);
            let views: Vec<&[f64]> = buffers.iter().map(Vec::as_slice).collect();
            f(&Ring {
                channels: &views,
                head,
                len,
            })
        }
    }
}

thread_local! {
    /// Lowest and highest index read, and the number of reads.
    static READS: Cell<(usize, usize, usize)> = const { Cell::new((usize::MAX, 0, 0)) };
}

#[derive(Clone, Copy, Debug)]
struct Probe {
    value: f64,
    index: usize,
    /// Reading it is a test failure: a value the call must not read.
    poisoned: bool,
}

impl Probe {
    fn read(self) -> f64 {
        assert!(
            !self.poisoned,
            "read a value it must not read: {}",
            self.index
        );
        READS.with(|r| {
            let (lo, hi, n) = r.get();
            r.set((lo.min(self.index), hi.max(self.index), n + 1));
        });
        self.value
    }
}

fn reset_reads() {
    READS.with(|r| r.set((usize::MAX, 0, 0)));
}

/// The lowest and highest index read since the last reset.
fn reads() -> Option<(usize, usize)> {
    let (lo, hi, n) = READS.with(Cell::get);
    (n > 0).then_some((lo, hi))
}

impl Sealed for Probe {}

impl MonotonicElement for Probe {
    fn is_gap(self) -> bool {
        !self.read().is_finite()
    }
    fn to_f64(self) -> f64 {
        self.read()
    }
    fn less(self, other: Self) -> bool {
        self.read() < other.read()
    }
    fn same(self, other: Self) -> bool {
        self.read() == other.read()
    }
    fn gap_mark(self) -> Self {
        Probe {
            value: self.read() - self.value,
            ..self
        }
    }
    fn mark_or(self, other: Self) -> Self {
        // Marks are derived values, not samples: no read is recorded.
        Probe {
            value: self.value + other.value,
            ..self
        }
    }
}

/// Probes over `data`, poisoned outside `allowed`.
fn probes(data: &[f64], allowed: Range<usize>) -> Vec<Probe> {
    data.iter()
        .enumerate()
        .map(|(index, &value)| Probe {
            value,
            index,
            poisoned: !allowed.contains(&index),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Full call

#[test]
fn dense_view_matches_every_sample() {
    let data = walk(5000, 7);
    let channels: &[&[f64]] = &[&data];
    // Partial first and last columns: x_min and x_max fall between samples.
    for (x_min, x_max, width) in [
        (3.3, 4321.7, 97),
        (0.0, 4999.0, 100),
        (-250.5, 6000.25, 211),
        (1234.0, 1234.0 + 64.0 * 9.0, 64),
    ] {
        let mapping = MonotonicMapping::linear(x_min, x_max, width);
        let envelope = full(0.0, 1.0, &slices(channels), &mapping);
        assert!(envelope.is_grid_aligned());
        assert_eq!(envelope.pixel_offset(), 0.0);
        check_oracle(&envelope, 0, &|i| data[i]);
    }
}

#[test]
fn data_starting_and_ending_inside_the_range() {
    let data = walk(3000, 11);
    let channels: &[&[f64]] = &[&data];
    let mapping = MonotonicMapping::linear(-100.0, 700.0, 50);
    let envelope = full(17.25, 0.2, &slices(channels), &mapping);
    let MonotonicView::Dense(slice) = envelope.view() else {
        panic!("dense")
    };
    assert_eq!(
        (slice.first, slice.end, slice.lead, slice.trail),
        (0, 3000, None, None)
    );
    check_oracle(&envelope, 0, &|i| data[i]);
    let cells: Vec<_> = envelope.channel_cells(0).collect();
    assert_eq!(cells[1].count, 0, "column 0 lies before the data");
    assert_eq!(cells[50].count, 0, "column 49 lies past the data");
}

#[test]
fn sparse_view_keeps_every_sample() {
    let data = walk(10_000, 3);
    let channels: &[&[f64]] = &[&data];
    // 400 visible samples on 100 pixel columns: 4 per column.
    let mapping = MonotonicMapping::linear(1000.0, 1400.0, 100);
    let envelope = full(0.0, 1.0, &slices(channels), &mapping);
    let MonotonicView::Sparse(slice) = envelope.view() else {
        panic!("sparse")
    };
    assert_eq!((slice.first, slice.end), (1000, 1400));
    assert_eq!((slice.lead, slice.trail), (Some(999), Some(1400)));
    check_oracle(&envelope, 0, &|i| data[i]);
    // One more visible sample makes it dense.
    let mapping = MonotonicMapping::linear(1000.0, 1400.5, 100);
    let envelope = full(0.0, 1.0, &slices(channels), &mapping);
    assert!(matches!(envelope.view(), MonotonicView::Dense(_)));
    check_oracle(&envelope, 0, &|i| data[i]);
}

#[test]
fn line_crossing_the_plot_without_a_visible_sample() {
    let data = [1.0, 5.0, -3.0];
    let channels: &[&[f64]] = &[&data];
    let mapping = MonotonicMapping::linear(0.25, 0.75, 10);
    let envelope = full(0.0, 1.0, &slices(channels), &mapping);
    let MonotonicView::Sparse(slice) = envelope.view() else {
        panic!("sparse")
    };
    assert_eq!(slice.samples(), 0);
    assert_eq!((slice.lead, slice.trail), (Some(0), Some(1)));
    let cells: Vec<_> = envelope.channel_cells(0).collect();
    assert_eq!(cells[0].first(), Some((0, 1.0)));
    // Sample 1 at x = 1 lies in column 15, past the kept columns: it is the
    // trail edge.
    assert_eq!(cells[12].first(), Some((1, 5.0)));
    check_oracle(&envelope, 0, &|i| data[i]);
}

#[test]
fn mixed_view_keeps_sparse_columns_and_reduces_dense_ones() {
    // 4.5 samples per pixel column: columns of 4 (kept) and 5 (reduced).
    let data = walk(2000, 13);
    let channels: &[&[f64]] = &[&data];
    let mapping = MonotonicMapping::linear(0.0, 450.0, 100);
    let envelope = full(0.0, 1.0, &slices(channels), &mapping);
    check_oracle(&envelope, 0, &|i| data[i]);
    let sizes: Vec<_> = (0..100)
        .map(|c| envelope.column_samples(c).unwrap().len())
        .collect();
    assert!(sizes.contains(&4) && sizes.contains(&5), "{sizes:?}");
}

#[test]
fn zooming_switches_between_sparse_and_dense() {
    let data = walk(100_000, 17);
    let channels: &[&[f64]] = &[&data];
    let mut span = 100_000.0;
    let mut seen = (false, false);
    while span > 10.0 {
        let mapping = MonotonicMapping::linear(5000.0, 5000.0 + span, 300);
        let envelope = full(0.0, 1.0, &slices(channels), &mapping);
        match envelope.view() {
            MonotonicView::Dense(_) => seen.0 = true,
            MonotonicView::Sparse(_) => seen.1 = true,
            MonotonicView::Empty => panic!("built"),
        }
        check_oracle(&envelope, 0, &|i| data[i]);
        span /= 1.7;
    }
    assert_eq!(seen, (true, true));
}

// ---------------------------------------------------------------------------
// Gaps and element types

#[test]
fn nan_gaps_match_every_sample() {
    let mut data = gappy(6000, 23);
    data[100..400].fill(f64::NAN);
    for i in (1000..1200).step_by(2) {
        data[i] = f64::NAN;
    }
    for i in (2000..2300).step_by(3) {
        data[i] = f64::NAN;
    }
    data[3000..3010].fill(f64::NAN);
    data[3011..3030].fill(f64::NAN);
    data[4000] = f64::NEG_INFINITY;
    let channels: &[&[f64]] = &[&data];
    for (x_min, x_max, width) in [(0.0, 5999.0, 100), (13.7, 5011.3, 77), (90.0, 410.0, 40)] {
        let mapping = MonotonicMapping::linear(x_min, x_max, width);
        let envelope = full(0.0, 1.0, &slices(channels), &mapping);
        check_oracle(&envelope, 0, &|i| data[i]);
    }
}

#[test]
fn every_gap_pattern_in_a_small_column() {
    // Every gap mask of 21 samples on 3 columns of 7 for masks below 512,
    // and every 997th past it; fed in full and one sample at a time.
    let base = walk(21, 31);
    let mapping = MonotonicMapping::linear(0.0, 21.0, 3);
    for mask in 0u32..(1 << 21) {
        if mask % 997 != 0 && mask >= 1 << 9 {
            continue;
        }
        let data: Vec<f64> = base
            .iter()
            .enumerate()
            .map(|(i, &v)| if mask & (1 << i) != 0 { f64::NAN } else { v })
            .collect();
        let channels: &[&[f64]] = &[&data];
        let envelope = full(0.0, 1.0, &slices(channels), &mapping);
        check_oracle(&envelope, 0, &|i| data[i]);
        let empty: &[&[f64]] = &[&data[..0]];
        let mut grown = full(0.0, 1.0, &slices(empty), &mapping);
        for i in 0..data.len() {
            let one: &[&[f64]] = &[&data[i..i + 1]];
            grown
                .append_monotonic(&slices(one), MonotonicAppend { sliding: false })
                .unwrap();
        }
        assert_same(&grown, &envelope);
    }
}

#[test]
fn ties_and_gaps_in_any_split() {
    // Values in {0, 1, 2} with gaps, and flat runs between gaps: extremes
    // tie everywhere.
    let mut rng = Rng(113);
    let mut data: Vec<f64> = (0..6000)
        .map(|_| match rng.next() % 20 {
            0..=2 => f64::NAN,
            r => (r % 3) as f64,
        })
        .collect();
    for c in (0..5800).step_by(240) {
        data[c..c + 40].fill(5.0);
        data[c + 3] = f64::NAN;
        data[c + 7] = f64::NAN;
        data[c + 8] = f64::NAN;
    }
    let channels: &[&[f64]] = &[&data];
    for width in [50, 150, 1499] {
        let mapping = MonotonicMapping::linear(0.0, 6000.0, width);
        let envelope = full(0.0, 1.0, &slices(channels), &mapping);
        check_oracle(&envelope, 0, &|i| data[i]);
        let empty: &[&[f64]] = &[&data[..0]];
        let mut grown = full(0.0, 1.0, &slices(empty), &mapping);
        let mut len = 0;
        while len < data.len() {
            let k = ((rng.next() % 97) as usize + 1).min(data.len() - len);
            let new: &[&[f64]] = &[&data[len..len + k]];
            grown
                .append_monotonic(&slices(new), MonotonicAppend { sliding: false })
                .unwrap();
            len += k;
        }
        assert_same(&grown, &envelope);
    }
}

fn check_type<T: MonotonicElement>(data: &[T], value: &dyn Fn(usize) -> f64) {
    let converted: Vec<f64> = data.iter().map(|v| v.to_f64()).collect();
    let mapping = MonotonicMapping::linear(0.5, data.len() as f64 - 7.5, 61);
    let typed: &[&[T]] = &[data];
    let wide: &[&[f64]] = &[&converted];
    let a = full(0.0, 1.0, &MonotonicSlices::new(typed), &mapping);
    let b = full(0.0, 1.0, &slices(wide), &mapping);
    assert_same(&a, &b);
    check_oracle(&a, 0, value);
}

#[test]
fn every_element_type_gives_its_f64_envelope() {
    let n = 7919;
    let mut rng = Rng(41);
    let spread = |min: f64, max: f64, rng: &mut Rng| {
        let mut v: Vec<f64> = (0..n)
            .map(|_| min + (max - min) * (rng.unit() + 1.0) / 2.0)
            .collect();
        // The full range, at both ends, and repeated extremes (ties).
        v[17] = min;
        v[18] = max;
        v[4000] = min;
        v[4003] = max;
        v[n - 3] = max;
        v
    };
    let ints = spread(i32::MIN as f64, i32::MAX as f64, &mut rng);
    let d: Vec<i32> = ints.iter().map(|&v| v.round() as i32).collect();
    check_type(&d, &|i| f64::from(d[i]));
    let ints = spread(-32768.0, 32767.0, &mut rng);
    let d: Vec<i16> = ints.iter().map(|&v| v.round() as i16).collect();
    assert!(d.contains(&i16::MIN) && d.contains(&i16::MAX));
    check_type(&d, &|i| f64::from(d[i]));
    let ints = spread(0.0, 65535.0, &mut rng);
    let d: Vec<u16> = ints.iter().map(|&v| v.round() as u16).collect();
    check_type(&d, &|i| f64::from(d[i]));
    let ints = spread(0.0, 255.0, &mut rng);
    let d: Vec<u8> = ints.iter().map(|&v| v.round() as u8).collect();
    assert!(d.contains(&u8::MIN) && d.contains(&u8::MAX));
    check_type(&d, &|i| f64::from(d[i]));
    let mut d: Vec<f32> = walk(n, 43).iter().map(|&v| v as f32).collect();
    d[100] = f32::NAN;
    d[2000..2100].fill(f32::NAN);
    d[50] = f32::MAX;
    d[51] = f32::MIN;
    check_type(&d, &|i| f64::from(d[i]));
    for scale in [1e-300, 1e-9, 1.0, 1e9, 1e300] {
        let d: Vec<f64> = walk(n, 47).iter().map(|&v| v * scale).collect();
        check_type(&d, &|i| d[i]);
    }
}

// ---------------------------------------------------------------------------
// Layouts and sources

#[test]
fn interleaved_and_slices_give_identical_envelopes() {
    let n = 3001;
    let a = walk(n, 51);
    let b = walk(n, 52);
    let mut c = walk(n, 53);
    c[500..900].fill(f64::NAN);
    // Frames of 5 values: a timestamp column and an unused column are never
    // read, and a trailing partial frame of 3 values is not counted.
    let stride = 5;
    let mut frames = Vec::new();
    for i in 0..n {
        let probe = |value, poisoned| Probe {
            value,
            index: i,
            poisoned,
        };
        frames.extend([
            probe(i as f64, true),
            probe(c[i], false),
            probe(a[i], false),
            probe(0.0, true),
            probe(b[i], false),
        ]);
    }
    frames.extend([0.0, 1.0, 2.0].map(|value| Probe {
        value,
        index: n,
        poisoned: true,
    }));
    let columns = [2, 4, 1];
    let mapping = MonotonicMapping::linear(10.5, 2900.0, 113);
    let interleaved = MonotonicInterleaved::new(&frames, stride, &columns).unwrap();
    assert_eq!(interleaved.len(), n);
    let x = full(0.0, 1.0, &interleaved, &mapping);
    let (pa, pb, pc) = (probes(&a, 0..n), probes(&b, 0..n), probes(&c, 0..n));
    let channels: &[&[Probe]] = &[&pa, &pb, &pc];
    let y = full(0.0, 1.0, &MonotonicSlices::new(channels), &mapping);
    assert_same(&x, &y);
    check_oracle(&x, 0, &|i| a[i]);
    check_oracle(&x, 1, &|i| b[i]);
    check_oracle(&x, 2, &|i| c[i]);
}

#[test]
fn interleaved_rejects_bad_columns() {
    let values = [0.0f64; 12];
    assert_eq!(
        MonotonicInterleaved::new(&values, 0, &[]).err(),
        Some(MonotonicError::InvalidStride)
    );
    assert_eq!(
        MonotonicInterleaved::new(&values, 3, &[0, 3]).err(),
        Some(MonotonicError::InvalidColumn {
            column: 3,
            stride: 3
        })
    );
    assert!(MonotonicRun::interleaved(0, 5, &values, 3, &[0], 0).is_none());
    assert!(MonotonicRun::slices(0, 13, &[&values[..]], 0).is_none());
}

#[test]
fn a_ring_buffer_gives_the_slice_envelopes() {
    let data = vec![gappy(30_000, 57), walk(30_000, 58)];
    let mapping = MonotonicMapping::linear(0.0, 10_000.0, 500);
    // Full call over a wrapped buffer.
    let a = with_source(&data, 0..30_000, Layout::Ring, |s| {
        full(0.0, 1.0, s, &mapping)
    });
    let b = with_source(&data, 0..30_000, Layout::Slices, |s| {
        full(0.0, 1.0, s, &mapping)
    });
    assert_same(&a, &b);
    // Appends and slides of wrapped buffers holding only the new samples.
    let mut ring = with_source(&data, 0..4000, Layout::Ring, |s| {
        full(0.0, 1.0, s, &mapping)
    });
    let mut plain = with_source(&data, 0..4000, Layout::Slices, |s| {
        full(0.0, 1.0, s, &mapping)
    });
    let mut len = 4000;
    for (k, sliding) in [
        (3000, false),
        (3001, false),
        (777, true),
        (5000, true),
        (40, true),
    ] {
        let options = MonotonicAppend { sliding };
        with_source(&data, len..len + k, Layout::Ring, |s| {
            ring.append_monotonic(s, options)
        })
        .unwrap();
        with_source(&data, len..len + k, Layout::Slices, |s| {
            plain.append_monotonic(s, options)
        })
        .unwrap();
        len += k;
        assert_same(&ring, &plain);
    }
}

// ---------------------------------------------------------------------------
// What a call reads

#[test]
fn a_full_call_reads_only_the_kept_columns_and_two_edges() {
    let data = walk(50_000, 61);
    let mapping = MonotonicMapping::linear(12_345.6, 31_000.0, 140);
    let channels: &[&[f64]] = &[&data];
    let plain = slices(channels);
    let counting = Counting::new(&plain);
    let envelope = full(0.0, 1.0, &counting, &mapping);
    let grid = envelope.grid().unwrap();
    // The lead edge (last sample left of column 0) through the trail edge
    // (first sample of column width + 1).
    let lead = grid.column_start(0, data.len()) - 1;
    let trail = grid.column_start(141, data.len());
    assert_eq!((lead, trail), (12_345, 31_134));
    assert_eq!(counting.lent(), vec![lead..trail + 1]);
    // And the values: poisoned outside, in both layouts.
    let input = probes(&data, lead..trail + 1);
    let channels: &[&[Probe]] = &[&input];
    reset_reads();
    full(0.0, 1.0, &MonotonicSlices::new(channels), &mapping);
    assert_eq!(reads(), Some((lead, trail)));
    let frames: Vec<Probe> = input
        .iter()
        .flat_map(|&p| {
            let unused = Probe {
                poisoned: true,
                ..p
            };
            [unused, p, unused]
        })
        .collect();
    reset_reads();
    full(
        0.0,
        1.0,
        &MonotonicInterleaved::new(&frames, 3, &[1]).unwrap(),
        &mapping,
    );
    assert_eq!(reads(), Some((lead, trail)));
}

#[test]
fn an_append_reads_only_new_samples_up_to_the_trail_edge() {
    let data = walk(40_000, 67);
    let mapping = MonotonicMapping::linear(0.0, 30_000.0, 100);
    let early: &[&[f64]] = &[&data[..20_150]];
    let mut envelope = full(0.0, 1.0, &slices(early), &mapping);
    // The new samples 20_150..40_000, held from index 0: only those up to
    // the trail edge (the first of column 101, at 30_300) are read.
    let new = probes(&data[20_150..], 0..30_300 - 20_150 + 1);
    let channels: &[&[Probe]] = &[&new];
    let source = MonotonicSlices::new(channels);
    let counting = Counting::new(&source);
    reset_reads();
    let update = envelope
        .append_monotonic(&counting, MonotonicAppend { sliding: false })
        .unwrap();
    assert_eq!(update.len, 40_000);
    assert_eq!(counting.lent(), vec![0..30_300 - 20_150 + 1]);
    assert_eq!(reads(), Some((0, 30_300 - 20_150)));
    // More samples change nothing and are not read.
    let more = probes(&data[..100], 0..0);
    let channels: &[&[Probe]] = &[&more];
    envelope
        .append_monotonic(
            &MonotonicSlices::new(channels),
            MonotonicAppend { sliding: false },
        )
        .unwrap();
    assert_eq!(envelope.len(), 40_100);
}

#[test]
fn a_slide_reads_only_new_samples() {
    let data = walk(50_000, 97);
    let mapping = MonotonicMapping::linear(0.0, 10_000.0, 1000);
    let first: &[&[f64]] = &[&data[..10_000]];
    let mut envelope = full(0.0, 1.0, &slices(first), &mapping);
    let mut len = 10_000;
    for k in [2000, 15, 4321, 10, 1] {
        let new = probes(&data[len..len + k], 0..k);
        let channels: &[&[Probe]] = &[&new];
        let source = MonotonicSlices::new(channels);
        let counting = Counting::new(&source);
        envelope
            .append_monotonic(&counting, MonotonicAppend { sliding: true })
            .unwrap();
        assert!(counting.lent().iter().all(|r| r.end <= k));
        len += k;
    }
    assert_eq!(envelope.len(), len);
}

// ---------------------------------------------------------------------------
// Append

#[test]
fn appends_equal_a_full_reduction() {
    let data = vec![gappy(60_000, 71), walk(60_000, 72)];
    for width in [1000, 2000] {
        for layout in LAYOUTS {
            let mapping = MonotonicMapping::linear(3.5, 50_000.0, width);
            let mut envelope = with_source(&data, 0..0, layout, |s| full(0.0, 1.0, s, &mapping));
            let mut len = 0;
            for k in [
                1, 7, 300, 4, 1000, 333, 9999, 1, 2, 5000, 6000, 17_000, 20_000,
            ] {
                with_source(&data, len..len + k, layout, |s| {
                    envelope.append_monotonic(s, MonotonicAppend { sliding: false })
                })
                .unwrap();
                len += k;
                let reference = with_source(&data, 0..len, layout, |s| full(0.0, 1.0, s, &mapping));
                assert_same(&envelope, &reference);
            }
            check_oracle(&envelope, 0, &|i| data[0][i]);
            check_oracle(&envelope, 1, &|i| data[1][i]);
        }
    }
}

#[test]
fn an_append_past_the_range_end_keeps_the_trail_edge() {
    let data = walk(20_000, 73);
    let mapping = MonotonicMapping::linear(0.0, 9_999.5, 100);
    let start: &[&[f64]] = &[&data[..8_000]];
    let mut envelope = full(0.0, 1.0, &slices(start), &mapping);
    let rest: &[&[f64]] = &[&data[8_000..]];
    envelope
        .append_monotonic(&slices(rest), MonotonicAppend { sliding: false })
        .unwrap();
    let all: &[&[f64]] = &[&data];
    assert_same(&envelope, &full(0.0, 1.0, &slices(all), &mapping));
    let MonotonicView::Dense(slice) = envelope.view() else {
        panic!("dense")
    };
    assert_eq!((slice.end, slice.trail), (10_000, Some(10_000)));
    check_oracle(&envelope, 0, &|i| data[i]);
    // A range change is a full call with the source.
    let wider = MonotonicMapping::linear(0.0, 19_999.0, 100);
    assert!(envelope.check_mapping(&wider).is_err());
    envelope
        .set_monotonic(&MonotonicGroup::new(0.0, 1.0, &slices(all)), &wider)
        .unwrap();
    assert_same(&envelope, &full(0.0, 1.0, &slices(all), &wider));
}

#[test]
fn append_errors() {
    let data = walk(1000, 79);
    let channels: &[&[f64]] = &[&data[..500]];
    let mapping = MonotonicMapping::linear(0.0, 999.0, 10);
    let mut envelope = MonotonicEnvelope::empty();
    let plain = MonotonicAppend { sliding: false };
    assert_eq!(
        envelope.append_monotonic(&slices(channels), plain).err(),
        Some(MonotonicError::NotBuilt)
    );
    envelope
        .set_monotonic(&MonotonicGroup::new(0.0, 1.0, &slices(channels)), &mapping)
        .unwrap();
    let two: &[&[f64]] = &[&data, &data];
    assert_eq!(
        envelope.append_monotonic(&slices(two), plain).err(),
        Some(MonotonicError::ChannelCountChanged {
            expected: 1,
            found: 2
        })
    );
}

// ---------------------------------------------------------------------------
// Sliding append

/// Builds a window of `window` samples on `0..window × interval`, then slides
/// by each `k`, passing only the new samples. After every step the envelope
/// equals a full reduction on the same anchored column grid over all the
/// data, its x range moved by exactly the slid samples and its offset the
/// exact fractional column shift. When it reports itself aligned, a full
/// call on the moved x range gives the same envelope too.
fn slide_steps(
    data: &[Vec<f64>],
    window: usize,
    initial: usize,
    width: u32,
    steps: &[usize],
    interval: f64,
    layout: Layout,
) -> Vec<usize> {
    let x_max = window as f64 * interval;
    let mapping = MonotonicMapping::linear(0.0, x_max, width);
    let x_per_pixel = x_max / f64::from(width);
    let mut envelope = with_source(data, 0..initial, layout, |s| {
        full(0.0, interval, s, &mapping)
    });
    let mut len = initial;
    let mut slid = 0;
    let mut dropped = Vec::new();
    for &k in steps {
        let update = with_source(data, len..len + k, layout, |s| {
            envelope.append_monotonic(s, MonotonicAppend { sliding: true })
        })
        .unwrap();
        len += k;
        slid += k;
        dropped.push(update.dropped);
        let moved = slid as f64 * interval;
        assert_eq!(update.x_range, (moved, x_max + moved), "x range");
        let s = moved / x_per_pixel;
        assert_eq!(update.pixel_offset, s - s.floor(), "offset");
        assert_eq!(envelope.grid().unwrap().shift(), s.floor() as usize);
        assert_eq!(envelope.mapping().unwrap().x_min, moved);
        let reference = with_source(data, 0..len, layout, |s| on_grid(&envelope, s));
        assert_cells(&envelope, &reference);
        if update.pixel_offset != 0.0 {
            assert!(!envelope.is_grid_aligned());
        } else if interval == 1.0 && x_per_pixel.fract() == 0.0 {
            // Exact arithmetic: a whole-column slide stays aligned.
            assert!(envelope.is_grid_aligned());
        }
        if envelope.is_grid_aligned() {
            let current = envelope.mapping().unwrap().clone();
            let same = with_source(data, 0..len, layout, |s| full(0.0, interval, s, &current));
            assert_cells(&envelope, &same);
        }
        for (n, d) in data.iter().enumerate() {
            check_oracle(&envelope, n, &|i| d[i]);
        }
    }
    dropped
}

#[test]
fn whole_column_slides_drop_columns() {
    let data = vec![gappy(80_000, 83), walk(80_000, 84)];
    for layout in LAYOUTS {
        // 20 and 10 samples per pixel column: k = 20, 60, 2000 are whole
        // columns at width 1000.
        let dropped = slide_steps(
            &data,
            20_000,
            20_000,
            1000,
            &[20, 60, 2000, 20, 8000],
            1.0,
            layout,
        );
        assert_eq!(dropped, [1, 3, 100, 1, 400]);
        let dropped = slide_steps(
            &data,
            20_000,
            20_000,
            2000,
            &[20, 60, 2000, 20, 8000],
            1.0,
            layout,
        );
        assert_eq!(dropped, [2, 6, 200, 2, 800]);
    }
}

#[test]
fn fractional_slides_keep_the_anchored_grid() {
    let data = vec![gappy(80_000, 89), walk(80_000, 90)];
    for layout in LAYOUTS {
        // 20 samples per column: 15 is three quarters of a column.
        let dropped = slide_steps(
            &data,
            20_000,
            20_000,
            1000,
            &[15, 3, 2, 7, 1, 1234, 13, 5],
            1.0,
            layout,
        );
        assert_eq!(dropped, [0, 0, 1, 0, 0, 62, 0, 1]);
        let dropped = slide_steps(
            &data,
            20_000,
            20_000,
            2000,
            &[15, 3, 2, 7, 1, 1234, 13, 5],
            1.0,
            layout,
        );
        assert_eq!(dropped, [1, 0, 1, 0, 0, 124, 1, 1]);
        // A dyadic interval, 2.5 samples per column: a sparse window.
        slide_steps(
            &data,
            5_000,
            5_000,
            2000,
            &[3, 1, 4, 1, 5, 9, 2, 6],
            0.25,
            layout,
        );
        // 2.5 columns per sample: most columns are empty, and the full call
        // holds two samples past the window, the second (the trail edge) in
        // column width + 2. Slides move it into the kept columns.
        slide_steps(&data, 400, 402, 1000, &[1, 2, 3, 1, 7, 400, 1], 1.0, layout);
    }
}

#[test]
fn a_slide_past_samples_not_kept_needs_the_source() {
    let data = walk(30_000, 101);
    let mapping = MonotonicMapping::linear(0.0, 10_000.0, 100);
    let all: &[&[f64]] = &[&data[..20_000]];
    let mut envelope = full(0.0, 1.0, &slices(all), &mapping);
    let new: &[&[f64]] = &[&data[20_000..20_500]];
    assert_eq!(
        envelope
            .append_monotonic(&slices(new), MonotonicAppend { sliding: true })
            .err(),
        Some(MonotonicError::SlidePastKept)
    );
    // Less than a column: nothing new comes into the kept columns.
    let new: &[&[f64]] = &[&data[20_000..20_050]];
    envelope
        .append_monotonic(&slices(new), MonotonicAppend { sliding: true })
        .unwrap();
}

#[test]
fn many_channels_through_appends_and_slides() {
    let channels_n = 8;
    let data: Vec<Vec<f64>> = (0..channels_n)
        .map(|c| gappy(90_000, 300 + c as u64))
        .collect();
    for layout in LAYOUTS {
        let mapping = MonotonicMapping::linear(0.0, 20_000.0, 1000);
        let mut envelope = with_source(&data, 0..0, layout, |s| full(0.0, 1.0, s, &mapping));
        let mut len = 0;
        for (k, sliding) in [
            (5000, false),
            (5000, false),
            (10_000, false),
            (5000, true),
            (10_007, true),
            (3, true),
            (30_000, true),
        ] {
            with_source(&data, len..len + k, layout, |s| {
                envelope.append_monotonic(s, MonotonicAppend { sliding })
            })
            .unwrap();
            len += k;
            let reference = with_source(&data, 0..len, layout, |s| on_grid(&envelope, s));
            assert_cells(&envelope, &reference);
        }
    }
}

// ---------------------------------------------------------------------------
// Picks

#[test]
fn picks_need_no_source() {
    let data = walk(100_000, 107);
    let channels: &[&[f64]] = &[&data];
    let mapping = MonotonicMapping::linear(0.0, 100_000.0, 500);
    let envelope = full(0.0, 1.0, &slices(channels), &mapping);
    for c in [0, 17, 250, 499] {
        let range = envelope.column_samples(c).unwrap();
        let column = &data[range.clone()];
        let max = column.iter().copied().fold(f64::MIN, f64::max);
        let min = column.iter().copied().fold(f64::MAX, f64::min);
        let (hi, v) = envelope.pick(0, c, max + 1.0).unwrap();
        assert_eq!((data[hi], v), (max, max));
        assert!(range.contains(&hi));
        let (lo, v) = envelope.pick(0, c, min - 1.0).unwrap();
        assert_eq!((data[lo], v), (min, min));
        let x = c as f64 * 200.0 + 3.0;
        assert_eq!(envelope.column_of_x(x), Some(c));
    }
    assert_eq!(envelope.column_of_x(-1.0), None);
    assert_eq!(envelope.pick(0, 5000, 0.0), None);
}

// ---------------------------------------------------------------------------
// Mapping record

#[test]
fn a_mapping_change_requires_the_source() {
    let data = walk(10_000, 101);
    let channels: &[&[f64]] = &[&data];
    let source = slices(channels);
    let group = MonotonicGroup::new(0.0, 1.0, &source);
    let mapping = MonotonicMapping::linear(0.0, 9_000.0, 300);
    let mut envelope = MonotonicEnvelope::empty();
    assert_eq!(
        envelope.check_mapping(&mapping),
        Err(MonotonicError::SourceRequired {
            change: MappingChange::NotBuilt
        })
    );
    envelope.set_monotonic(&group, &mapping).unwrap();
    assert_eq!(envelope.check_mapping(&mapping), Ok(()));
    let broken = {
        use crate::config::{AxisBreak, AxisBreaks};
        let mut axis = crate::defaults::axis_options_x();
        axis.min = 0.0;
        axis.max = 9_000.0;
        axis.breaks = Some(AxisBreaks {
            ranges: vec![AxisBreak::new(1000.0, 2000.0)],
            gap_px: 10.0,
            ..crate::defaults::axis_breaks()
        });
        let map = crate::layout::BreakMap::of(&axis, 300.0);
        assert!(map.is_broken());
        map
    };
    let changes = [
        (
            MonotonicMapping {
                x_min: 1.0,
                ..mapping.clone()
            },
            MappingChange::XRange,
        ),
        (
            MonotonicMapping {
                x_max: 9_000.5,
                ..mapping.clone()
            },
            MappingChange::XRange,
        ),
        (
            // A resize, or the 2x export.
            MonotonicMapping {
                plot_width_px: 600,
                ..mapping.clone()
            },
            MappingChange::PlotWidth,
        ),
        (
            MonotonicMapping {
                x_scale: crate::config::AxisScale::Logarithmic,
                ..mapping.clone()
            },
            MappingChange::XScale,
        ),
        (
            MonotonicMapping {
                x_breaks: broken,
                ..mapping.clone()
            },
            MappingChange::XBreaks,
        ),
        (
            MonotonicMapping {
                x_inverted: true,
                ..mapping.clone()
            },
            MappingChange::XInversion,
        ),
    ];
    for (current, change) in changes {
        assert_eq!(
            envelope.check_mapping(&current),
            Err(MonotonicError::SourceRequired { change })
        );
        let mut rebuilt = envelope.clone();
        match rebuilt.set_monotonic(&group, &current) {
            Ok(_) => {
                assert_eq!(rebuilt.check_mapping(&current), Ok(()));
                assert_same(&rebuilt, &full(0.0, 1.0, &slices(channels), &current));
            }
            Err(e) => assert_eq!(e, MonotonicError::UnsupportedXMapping),
        }
    }
    // A slide moves the recorded range: the moved range checks out.
    let new: &[&[f64]] = &[&data[..100]];
    let small: &[&[f64]] = &[&data[..9_000]];
    envelope
        .set_monotonic(&MonotonicGroup::new(0.0, 1.0, &slices(small)), &mapping)
        .unwrap();
    let update = envelope
        .append_monotonic(&slices(new), MonotonicAppend { sliding: true })
        .unwrap();
    let moved = MonotonicMapping {
        x_min: update.x_range.0,
        x_max: update.x_range.1,
        ..mapping.clone()
    };
    assert_eq!(envelope.check_mapping(&moved), Ok(()));
    assert!(envelope.check_mapping(&mapping).is_err());
}

#[test]
fn invalid_grids_and_mappings_are_refused() {
    let data = [1.0, 2.0];
    let channels: &[&[f64]] = &[&data];
    let source = slices(channels);
    let mapping = MonotonicMapping::linear(0.0, 1.0, 10);
    let mut envelope = MonotonicEnvelope::empty();
    for (start, interval) in [
        (f64::NAN, 1.0),
        (0.0, 0.0),
        (0.0, -1.0),
        (0.0, f64::INFINITY),
    ] {
        assert!(matches!(
            envelope.set_monotonic(&MonotonicGroup::new(start, interval, &source), &mapping),
            Err(MonotonicError::InvalidGrid { .. })
        ));
    }
    for (x_min, x_max, width) in [
        (1.0, 1.0, 10),
        (2.0, 1.0, 10),
        (0.0, f64::NAN, 10),
        (0.0, 1.0, 0),
    ] {
        assert!(matches!(
            envelope.set_monotonic(
                &MonotonicGroup::new(0.0, 1.0, &source),
                &MonotonicMapping::linear(x_min, x_max, width)
            ),
            Err(MonotonicError::InvalidMapping { .. })
        ));
    }
}

// ---------------------------------------------------------------------------
// Groups

#[test]
fn mismatched_channels_take_their_common_extent() {
    let a = walk(5000, 103);
    let b = walk(4321, 104);
    let mapping = MonotonicMapping::linear(0.0, 6000.0, 50);
    let channels: &[&[f64]] = &[&a[..3000], &b[..2500]];
    let mut envelope = MonotonicEnvelope::empty();
    let update = envelope
        .set_monotonic(&MonotonicGroup::new(0.0, 1.0, &slices(channels)), &mapping)
        .unwrap();
    assert!(update.truncated && envelope.truncated());
    assert_eq!(update.len, 2500);
    let more: &[&[f64]] = &[&a[2500..5000], &b[2500..4321]];
    let update = envelope
        .append_monotonic(&slices(more), MonotonicAppend { sliding: false })
        .unwrap();
    assert!(update.truncated);
    assert_eq!(update.len, 4321);
    let cut: &[&[f64]] = &[&a[..4321], &b];
    let expected = full(0.0, 1.0, &slices(cut), &mapping);
    assert!(!expected.truncated());
    assert_cells(&envelope, &expected);
}

#[test]
fn a_thousand_channels_have_no_cap() {
    let channels_n = 1000;
    let n = 2000;
    let data: Vec<Vec<f64>> = (0..channels_n).map(|c| walk(n, 200 + c as u64)).collect();
    let views: Vec<&[f64]> = data.iter().map(Vec::as_slice).collect();
    let mapping = MonotonicMapping::linear(0.0, (n - 1) as f64, 100);
    let envelope = full(0.0, 1.0, &slices(&views), &mapping);
    assert_eq!(envelope.channel_count(), channels_n);
    assert_eq!(bits(&envelope).len(), channels_n * 103);
    for c in (0..channels_n).step_by(37).chain([channels_n - 1]) {
        let alone: &[&[f64]] = &[&data[c]];
        let single = full(0.0, 1.0, &slices(alone), &mapping);
        assert!(envelope.channel_cells(c).eq(single.channel_cells(0)));
        check_oracle(&envelope, c, &|i| data[c][i]);
    }
    let interleaved = with_source(&data, 0..n, Layout::Interleaved, |s| {
        full(0.0, 1.0, s, &mapping)
    });
    assert_cells(&interleaved, &envelope);
}
