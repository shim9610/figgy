//! The M4 reduction of one column of one channel (#271), as an accumulator
//! that is closed under appending: a column reduced from its samples in one
//! piece equals the same column fed in any number of pieces. So an append
//! merges new samples into the last, partial column without its old
//! samples, and a full call is the same accumulator fed once.
//!
//! - A column with at most 4 samples keeps them all.
//! - A denser column keeps its first and last samples and the lowest and
//!   highest *drawn* samples (the first lowest and the last highest): finite
//!   samples with a finite neighbour in the same column, the ends of the
//!   segments inside it. The first and last samples join the neighbouring
//!   columns, so a column is reduced from its own samples alone.
//!
//! The scans are chunked so the compiler can vectorize them; no SIMD crate.

use super::element::MonotonicElement;
use super::source::StridedChannel;

/// The most points a cell holds.
pub const CELL_POINTS: usize = 4;

/// Lanes of the chunked loops.
const LANES: usize = 8;

/// The drawn form of one column of one channel: up to 4 points in x order,
/// each with the logical sample index it came from (so a pick maps back to
/// the host's data) and its value in data units.
///
/// The polyline the envelope implies joins point `k` to point `k + 1` of a
/// cell when [`Self::linked`] says so, and the last point of a cell to the
/// first point of the next non-empty cell when both are finite and their
/// indices are consecutive ([`EnvelopeCell::joins`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvelopeCell {
    /// The logical sample index of each point; `count` are valid.
    pub index: [usize; CELL_POINTS],
    /// The value of each point in data units; NaN for a gap.
    pub value: [f64; CELL_POINTS],
    /// The number of valid points, `0..=4`.
    pub count: u8,
    /// Bit `k` set: the segment from point `k` to point `k + 1` is drawn.
    pub links: u8,
}

impl EnvelopeCell {
    /// A column without samples.
    pub const EMPTY: Self = Self {
        index: [0; CELL_POINTS],
        value: [0.0; CELL_POINTS],
        count: 0,
        links: 0,
    };

    /// The valid points, `(index, value)`, in x order.
    pub fn points(&self) -> impl Iterator<Item = (usize, f64)> + '_ {
        self.index
            .iter()
            .copied()
            .zip(self.value.iter().copied())
            .take(usize::from(self.count))
    }

    /// The first point.
    pub fn first(&self) -> Option<(usize, f64)> {
        self.points().next()
    }

    /// The last point.
    pub fn last(&self) -> Option<(usize, f64)> {
        self.points().last()
    }

    /// Whether the segment from point `k` to point `k + 1` is drawn.
    pub fn linked(&self, k: usize) -> bool {
        k + 1 < usize::from(self.count) && self.links & (1 << k) != 0
    }

    /// Whether the line joins the last point of one cell to the first point
    /// of the next non-empty one: consecutive samples, both finite.
    pub fn joins(from: (usize, f64), to: (usize, f64)) -> bool {
        from.0.checked_add(1) == Some(to.0) && from.1.is_finite() && to.1.is_finite()
    }

    /// The finite point nearest to `y`, the first of equals.
    pub fn nearest(&self, y: f64) -> Option<(usize, f64)> {
        self.points()
            .filter(|p| p.1.is_finite())
            .fold(None, |best: Option<(usize, f64)>, p| match best {
                Some(b) if (b.1 - y).abs() <= (p.1 - y).abs() => Some(b),
                _ => Some(p),
            })
    }

    fn push(&mut self, index: usize, value: f64) {
        let at = usize::from(self.count);
        if at < CELL_POINTS {
            self.index[at] = index;
            self.value[at] = value;
            self.count += 1;
        }
    }
}

/// A kept sample: its logical index, its value and the run of finite
/// samples it belongs to within its column (two finite samples are joined by
/// drawn segments exactly when their runs are equal).
#[derive(Clone, Copy, Debug)]
struct Kept {
    index: usize,
    value: f64,
    run: u64,
}

impl Kept {
    const NONE: Self = Self {
        index: 0,
        value: 0.0,
        run: 0,
    };

    fn finite(&self) -> bool {
        self.value.is_finite()
    }
}

/// The sample before the column's last, in the column, is finite.
const LAST_PREV_FINITE: u8 = 1;
/// `kept[1]` holds the lowest drawn sample.
const HAS_LO: u8 = 2;
/// `kept[2]` holds the highest drawn sample.
const HAS_HI: u8 = 4;

/// The samples a column count as dense from.
const DENSE: u8 = CELL_POINTS as u8 + 1;

/// The accumulator of one column of one channel. Sparse (at most 4
/// samples): `kept` holds them all. Dense: `kept` is the first sample, the
/// lowest and highest drawn samples committed so far, and the last sample,
/// whose drawn state waits for its right neighbour.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ColumnState {
    kept: [Kept; CELL_POINTS],
    /// Gaps seen so far in the column: the run of its next finite sample.
    run: u64,
    /// Samples seen, saturating at [`DENSE`].
    n: u8,
    flags: u8,
}

impl ColumnState {
    pub(crate) const EMPTY: Self = Self {
        kept: [Kept::NONE; CELL_POINTS],
        run: 0,
        n: 0,
        flags: 0,
    };

    /// A slot holding one sample (an edge sample).
    pub(crate) fn single(index: usize, value: f64) -> Self {
        let mut state = Self::EMPTY;
        state.push(index, value);
        state
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.n == 0
    }

    fn flag(&self, f: u8) -> bool {
        self.flags & f != 0
    }

    fn set(&mut self, f: u8, on: bool) {
        if on {
            self.flags |= f;
        } else {
            self.flags &= !f;
        }
    }

    fn last_kept(&self) -> Option<Kept> {
        match self.n {
            0 => None,
            n if n < DENSE => Some(self.kept[usize::from(n) - 1]),
            _ => Some(self.kept[3]),
        }
    }

    /// The column's last sample.
    pub(crate) fn last_point(&self) -> Option<(usize, f64)> {
        self.last_kept().map(|k| (k.index, k.value))
    }

    fn commit_lo(&mut self, p: Kept) {
        if !self.flag(HAS_LO) || p.value < self.kept[1].value {
            self.kept[1] = p;
            self.flags |= HAS_LO;
        }
    }

    fn commit_hi(&mut self, p: Kept) {
        if !self.flag(HAS_HI) || p.value >= self.kept[2].value {
            self.kept[2] = p;
            self.flags |= HAS_HI;
        }
    }

    /// Commits a drawn sample, in index order.
    fn commit(&mut self, p: Kept) {
        self.commit_lo(p);
        self.commit_hi(p);
    }

    /// Appends one sample.
    pub(crate) fn push(&mut self, index: usize, value: f64) {
        let finite = value.is_finite();
        if !finite {
            self.run = self.run.saturating_add(1);
        }
        let p = Kept {
            index,
            value,
            run: self.run,
        };
        match self.n {
            0 => {
                self.kept[0] = p;
                self.n = 1;
                self.set(LAST_PREV_FINITE, false);
            }
            n if n < CELL_POINTS as u8 => {
                let old = self.kept[usize::from(n) - 1];
                self.kept[usize::from(n)] = p;
                self.n = n + 1;
                self.set(LAST_PREV_FINITE, old.finite());
            }
            n if n == CELL_POINTS as u8 => {
                // Turning dense: the 4 kept samples are all of the column's,
                // and the new one is the right neighbour of the last.
                let s = self.kept;
                self.flags &= !(HAS_LO | HAS_HI);
                for k in 0..CELL_POINTS {
                    let prev = k > 0 && s[k - 1].finite();
                    let next = if k + 1 == CELL_POINTS {
                        finite
                    } else {
                        s[k + 1].finite()
                    };
                    if s[k].finite() && (prev || next) {
                        self.commit(s[k]);
                    }
                }
                self.kept[0] = s[0];
                self.kept[3] = p;
                self.set(LAST_PREV_FINITE, s[3].finite());
                self.n = DENSE;
            }
            _ => {
                let old = self.kept[3];
                if old.finite() && (self.flag(LAST_PREV_FINITE) || finite) {
                    self.commit(old);
                }
                self.set(LAST_PREV_FINITE, old.finite());
                self.kept[3] = p;
            }
        }
    }

    /// Appends a run of at least 3 finite samples to a dense column at once:
    /// `lo` is the first lowest and `hi` the last highest of all but the
    /// last, which is `tail`. Equal to pushing them one by one.
    fn absorb(&mut self, lo: (usize, f64), hi: (usize, f64), tail: (usize, f64)) {
        let old = self.kept[3];
        if old.finite() {
            self.commit(old);
        }
        let run = self.run;
        self.commit_lo(Kept {
            index: lo.0,
            value: lo.1,
            run,
        });
        self.commit_hi(Kept {
            index: hi.0,
            value: hi.1,
            run,
        });
        self.kept[3] = Kept {
            index: tail.0,
            value: tail.1,
            run,
        };
        self.set(LAST_PREV_FINITE, true);
    }

    /// The drawn form of the column.
    pub(crate) fn cell(&self) -> EnvelopeCell {
        let mut cell = EnvelopeCell::EMPTY;
        if self.n < DENSE {
            let mut previous_finite = false;
            for (k, p) in self.kept.iter().take(usize::from(self.n)).enumerate() {
                if k > 0 && previous_finite && p.finite() {
                    cell.links |= 1 << (k - 1);
                }
                cell.push(p.index, p.value);
                previous_finite = p.finite();
            }
            return cell;
        }
        // Dense: the last sample is drawn inside the column when its left
        // neighbour is finite.
        let mut probe = *self;
        let last = self.kept[3];
        if last.finite() && self.flag(LAST_PREV_FINITE) {
            probe.commit(last);
        }
        let lo = probe.flag(HAS_LO).then_some(probe.kept[1]);
        let hi = probe.flag(HAS_HI).then_some(probe.kept[2]);
        let (e1, e2) = match (lo, hi) {
            (Some(l), Some(h)) if h.index < l.index => (Some(h), Some(l)),
            other => other,
        };
        // Candidates in x order; equal indices merge, keeping the extreme
        // mark.
        let ordered = [
            (Some(self.kept[0]), false),
            (e1, true),
            (e2, true),
            (Some(self.kept[3]), false),
        ];
        let mut merged = [(Kept::NONE, false); CELL_POINTS];
        let mut count = 0usize;
        for (candidate, extreme) in ordered {
            let Some(c) = candidate else { continue };
            if count > 0 && merged[count - 1].0.index == c.index {
                merged[count - 1].1 |= extreme;
            } else {
                merged[count] = (c, extreme);
                count += 1;
            }
        }
        for k in 0..count {
            let (c, extreme) = merged[k];
            if k > 0 {
                let (p, p_extreme) = merged[k - 1];
                let joined = (p_extreme && extreme) || (p.finite() && c.finite() && p.run == c.run);
                if joined {
                    cell.links |= 1 << (k - 1);
                }
            }
            cell.push(c.index, c.value);
        }
        cell
    }

    /// Whether the column is dense: past 4 samples.
    pub(crate) fn is_dense(&self) -> bool {
        self.n >= DENSE
    }
}

/// Appends samples `a..b` of `channel` (logical index `base + i`) to
/// `state`.
pub(crate) fn extend<T: MonotonicElement>(
    state: &mut ColumnState,
    channel: &StridedChannel<'_, T>,
    a: usize,
    b: usize,
    base: usize,
) {
    let i = fill(state, channel, a, b, base);
    let scanned = channel
        .contiguous(i, b.saturating_sub(2))
        .filter(|_| b >= i + 4)
        .and_then(scan_contiguous)
        .map(|(lo, hi)| (i + lo, i + hi));
    finish(state, channel, i, b, base, scanned);
}

/// Pushes samples from `a` one by one until the column is dense; returns
/// the first sample not pushed.
pub(crate) fn fill<T: MonotonicElement>(
    state: &mut ColumnState,
    channel: &StridedChannel<'_, T>,
    a: usize,
    b: usize,
    base: usize,
) -> usize {
    let mut i = a;
    while i < b && !state.is_dense() {
        if let Some(v) = channel.get(i) {
            state.push(base + i, v.to_f64());
        }
        i += 1;
    }
    i
}

/// Appends samples `i..b` of a dense column. `scanned` is the first lowest
/// and last highest of `i..b - 2` when they are all finite and there are at
/// least 2 of them; then the run is absorbed at once when sample `b - 2` is
/// finite too. Anything else goes one by one.
pub(crate) fn finish<T: MonotonicElement>(
    state: &mut ColumnState,
    channel: &StridedChannel<'_, T>,
    i: usize,
    b: usize,
    base: usize,
    scanned: Option<(usize, usize)>,
) {
    let value = |k: usize| channel.get(k).map(|v| v.to_f64());
    let mut from = i;
    if let Some((lo, hi)) = scanned
        && b >= i + 4
        && let (Some(l), Some(h), Some(t)) = (value(lo), value(hi), value(b - 2))
        && t.is_finite()
    {
        state.absorb((base + lo, l), (base + hi, h), (base + b - 2, t));
        from = b - 1;
    }
    for k in from..b {
        if let Some(v) = value(k) {
            state.push(base + k, v);
        }
    }
}

/// Samples per block of the one-pass scan: each block's extremes come from
/// [`LANES`] independent accumulators, and only the block holding the
/// result is scanned again for its position.
const BLOCK: usize = 8 * LANES;

/// The min, max and gap mark of one non-empty block, in lanes. Kept out of
/// line: inlined into the block loop it measured about half as fast.
#[inline(never)]
fn block_extremes<T: MonotonicElement>(block: &[T], seed: T) -> (T, T, T) {
    let mut lo = [seed; LANES];
    let mut hi = [seed; LANES];
    let mut mark = [seed.gap_mark(); LANES];
    let (chunks, rest) = block.as_chunks::<LANES>();
    for chunk in chunks {
        for l in 0..LANES {
            let v = chunk[l];
            lo[l] = if v.less(lo[l]) { v } else { lo[l] };
            hi[l] = if hi[l].less(v) { v } else { hi[l] };
            mark[l] = mark[l].mark_or(v.gap_mark());
        }
    }
    for &v in rest {
        lo[0] = if v.less(lo[0]) { v } else { lo[0] };
        hi[0] = if hi[0].less(v) { v } else { hi[0] };
        mark[0] = mark[0].mark_or(v.gap_mark());
    }
    let (mut min, mut max, mut gap) = (lo[0], hi[0], mark[0]);
    for l in 1..LANES {
        min = if lo[l].less(min) { lo[l] } else { min };
        max = if max.less(hi[l]) { hi[l] } else { max };
        gap = gap.mark_or(mark[l]);
    }
    (min, max, gap)
}

/// One pass over a contiguous run: the offsets of its first lowest and its
/// last highest value, `None` when it is empty or holds a gap.
#[inline]
pub(crate) fn scan_contiguous<T: MonotonicElement>(s: &[T]) -> Option<(usize, usize)> {
    let mut blocks = s.chunks(BLOCK).enumerate();
    let (_, first) = blocks.next()?;
    let &seed = first.first()?;
    let (mut lo, mut hi, mut mark) = block_extremes(first, seed);
    let (mut lo_block, mut hi_block) = (0, 0);
    for (b, block) in blocks {
        let Some(&seed) = block.first() else { break };
        let (block_lo, block_hi, block_mark) = block_extremes(block, seed);
        if block_lo.less(lo) {
            lo = block_lo;
            lo_block = b;
        }
        if !block_hi.less(hi) {
            hi = block_hi;
            hi_block = b;
        }
        mark = mark.mark_or(block_mark);
    }
    if mark.is_gap() {
        return None;
    }
    let block = |b: usize| s.get(b * BLOCK..((b + 1) * BLOCK).min(s.len()));
    let lo_at = block(lo_block)?.iter().position(|v| v.same(lo))? + lo_block * BLOCK;
    let hi_at = block(hi_block)?.iter().rposition(|v| v.same(hi))? + hi_block * BLOCK;
    Some((lo_at, hi_at))
}

/// Channels of one interleaved run scanned together in one pass over the
/// frames. A loop width, not a limit: any number of channels is scanned in
/// batches of this size (rule 10).
pub(crate) const BATCH: usize = 8;

/// Frames per block of the interleaved scan, as [`BLOCK`] for a contiguous
/// run.
const FRAME_BLOCK: usize = 64;

/// Scans frames `a..b` of up to [`BATCH`] columns of one interleaved run
/// (`frames` holds whole frames of `stride` values, `columns[j] < stride`),
/// walking the frames once for all of them, so each frame's cache line is
/// read once per batch. Per column: the frames of its first lowest and last
/// highest value, `None` when it holds a gap. Columns no channel names are
/// never read.
pub(crate) fn scan_frames<T: MonotonicElement>(
    frames: &[T],
    stride: usize,
    columns: &[usize],
    a: usize,
    b: usize,
) -> [Option<(usize, usize)>; BATCH] {
    let mut out = [None; BATCH];
    let k = columns.len().min(BATCH);
    let n = b.saturating_sub(a);
    let block = a
        .checked_mul(stride)
        .zip(b.checked_mul(stride))
        .and_then(|(from, to)| frames.get(from..to));
    let Some(block) = block.filter(|_| n > 0 && columns[..k].iter().all(|&c| c < stride)) else {
        return out;
    };
    let mut col = [0usize; BATCH];
    col[..k].copy_from_slice(&columns[..k]);
    let Some(&any) = block.get(col[0]) else {
        return out;
    };
    let mut lo = [any; BATCH];
    let mut hi = [any; BATCH];
    let mut lo_block = [0usize; BATCH];
    let mut hi_block = [0usize; BATCH];
    let mut mark = [any.gap_mark(); BATCH];
    for (bi, group) in block.chunks(stride.saturating_mul(FRAME_BLOCK)).enumerate() {
        let mut block_lo = lo;
        for j in 0..k {
            block_lo[j] = group[col[j]];
        }
        let mut block_hi = block_lo;
        for frame in group.chunks_exact(stride) {
            for j in 0..k {
                let v = frame[col[j]];
                block_lo[j] = if v.less(block_lo[j]) { v } else { block_lo[j] };
                block_hi[j] = if block_hi[j].less(v) { v } else { block_hi[j] };
                mark[j] = mark[j].mark_or(v.gap_mark());
            }
        }
        for j in 0..k {
            if bi == 0 || block_lo[j].less(lo[j]) {
                lo[j] = block_lo[j];
                lo_block[j] = bi;
            }
            if bi == 0 || !block_hi[j].less(hi[j]) {
                hi[j] = block_hi[j];
                hi_block[j] = bi;
            }
        }
    }
    let at = |f: usize, j: usize| block.get(f * stride + col[j]).copied();
    for j in 0..k {
        if mark[j].is_gap() {
            continue;
        }
        let blocks = |bi: usize| bi * FRAME_BLOCK..((bi + 1) * FRAME_BLOCK).min(n);
        let lo_f = blocks(lo_block[j]).find(|&f| at(f, j).is_some_and(|v| v.same(lo[j])));
        let hi_f = blocks(hi_block[j])
            .rev()
            .find(|&f| at(f, j).is_some_and(|v| v.same(hi[j])));
        out[j] = lo_f.zip(hi_f).map(|(l, h)| (a + l, a + h));
    }
    out
}
