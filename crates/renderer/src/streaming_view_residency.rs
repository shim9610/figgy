//! Conservative screen-geometry tests for a chart-local packed view cache.
//!
//! The source row identity never changes. These tests decide which source
//! rows must be retained, including both ends of a line that crosses the
//! viewport even when neither endpoint lies inside it. Rejected caches must
//! fall back to the existing exact stream draw, never omit geometry.

use crate::Config;
use crate::config::{AxisOptions, AxisScale};
use crate::data_config::{DataRenderType, SeriesConfig};
use crate::data_config::ErrorRef;
use crate::config::DrawStyle;
use crate::line::LineStylePreset;
use crate::streaming_upload::{RecordedChunk, UploadedColumn};
use crate::streaming::{ColumnRange, SourceEncoding};
use crate::gpu_memory::{GpuLedger, GpuResourceKind, TrackedBuffer};
use std::sync::Arc;
use super::StreamDrawPhase;

pub(super) struct ViewPackedCandidate {
    pub(super) view: ViewBounds,
    // At most one completed coalescing page plus the current source chunk.
    // Older pages are uploaded to GPU immediately, never retained on CPU.
    pub(super) chunks: Vec<PhasePackedChunk>,
    gpu_chunks: Vec<GpuPhaseChunk>,
    pub(super) packed_bytes: u64,
    pub(super) limit: u64,
    pub(super) rejected: Option<PackReject>,
}

pub(super) struct PhasePackedChunk {
    pub(super) series: usize,
    pub(super) phase: StreamDrawPhase,
    pub(super) chunk: PackedViewChunk,
}

pub(super) struct GpuViewCache {
    pub(super) view: ViewBounds,
    pub(super) chunks: Vec<GpuPhaseChunk>,
    pub(super) packed_bytes: u64,
}

pub(super) struct GpuPhaseChunk {
    pub(super) series: usize,
    pub(super) phase: StreamDrawPhase,
    pub(super) chunk: RecordedChunk,
    pub(super) ranges: Vec<ColumnRange>,
    /// Source-index runs in this page, in packed-buffer order. The CPU packer
    /// already knows these row identities; no GPU data readback is needed.
    pub(super) source_runs: Vec<SourceRun>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SourceRun {
    pub(super) source_start: u32,
    pub(super) len: u32,
    pub(super) packed_start: u32,
}

impl GpuPhaseChunk {
    pub(super) fn source_index(&self, packed_index: u32) -> Option<u32> {
        source_in_runs(&self.source_runs, packed_index)
    }

    pub(super) fn packed_index(&self, source_index: u32) -> Option<u32> {
        let index = self.source_runs.partition_point(|run| run.source_start <= source_index);
        let run = self.source_runs.get(index.checked_sub(1)?)?;
        let offset = source_index.checked_sub(run.source_start)?;
        (offset < run.len).then_some(run.packed_start + offset)
    }

    pub(super) fn next_source_index(&self, source_index: u32, forward: bool) -> Option<u32> {
        next_in_runs(&self.source_runs, source_index, forward)
    }
}

fn source_in_runs(runs: &[SourceRun], packed_index: u32) -> Option<u32> {
    let index = runs.partition_point(|run| run.packed_start <= packed_index);
    let run = runs.get(index.checked_sub(1)?)?;
    let offset = packed_index.checked_sub(run.packed_start)?;
    (offset < run.len).then_some(run.source_start + offset)
}

fn next_in_runs(runs: &[SourceRun], source_index: u32, forward: bool) -> Option<u32> {
        if forward {
            let index = runs.partition_point(|run| run.source_start <= source_index);
            if let Some(previous) = index.checked_sub(1).and_then(|i| runs.get(i)) {
                if source_index.checked_sub(previous.source_start)
                    .and_then(|offset| offset.checked_add(1))
                    .is_some_and(|offset| offset < previous.len) {
                    return Some(source_index + 1);
                }
            }
            runs.get(index).map(|run| run.source_start)
        } else {
            let index = runs.partition_point(|run| run.source_start < source_index);
            let previous = runs.get(index.checked_sub(1)?)?;
            Some(source_index.saturating_sub(1).min(previous.source_start + previous.len - 1))
        }
}

fn runs_from_rows(rows: &[u64]) -> Result<Vec<SourceRun>, PackReject> {
    let mut source_runs: Vec<SourceRun> = Vec::new();
    for (local, &row) in rows.iter().enumerate() {
        if row == u64::MAX { continue; }
        let source = u32::try_from(row).map_err(|_| PackReject::WorkingSetExceeded)?;
        if let Some(last) = source_runs.last_mut() {
            if last.source_start.checked_add(last.len) == Some(source) {
                last.len += 1;
                continue;
            }
        }
        source_runs.try_reserve(1).map_err(|_| PackReject::AllocationFailed)?;
        source_runs.push(SourceRun {
            source_start: source,
            len: 1,
            packed_start: u32::try_from(local).map_err(|_| PackReject::WorkingSetExceeded)?,
        });
    }
    Ok(source_runs)
}

impl GpuViewCache {
    pub(super) fn next_source_index(&self, series: usize, source_index: u32, forward: bool) -> Option<u32> {
        self.chunks.iter()
            .filter(|page| page.series == series && page.phase != StreamDrawPhase::Errorbar)
            .filter_map(|page| page.next_source_index(source_index, forward))
            .reduce(|a, b| if forward { a.min(b) } else { a.max(b) })
    }

    pub(super) fn point_page(&self, series: usize, source_index: u32) -> Option<(&GpuPhaseChunk, u32)> {
        // Scatter carries the point itself; a line-only series uses its line
        // endpoint. Errorbar geometry is never a point-selection target.
        [StreamDrawPhase::Scatter, StreamDrawPhase::Line].into_iter().find_map(|phase| {
            self.chunks.iter().filter(|page| page.series == series && page.phase == phase)
                .find_map(|page| page.packed_index(source_index).map(|local| (page, local)))
        })
    }
}

impl GpuViewCache {
    pub(super) fn upload(
        candidate: ViewPackedCandidate,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ledger: &Arc<GpuLedger>,
        memory_budget: u64,
        current_gpu_bytes: u64,
    ) -> Result<Self, PackReject> {
        if candidate.rejected.is_some() {
            return Err(PackReject::WorkingSetExceeded);
        }
        let view = candidate.view;
        let mut chunks = candidate.gpu_chunks;
        chunks.try_reserve(candidate.chunks.len()).map_err(|_| PackReject::AllocationFailed)?;
        let mut current_gpu_bytes = current_gpu_bytes;
        for phase in candidate.chunks {
            let bytes = phase.chunk.byte_len().ok_or(PackReject::WorkingSetExceeded)?;
            current_gpu_bytes = current_gpu_bytes.checked_add(bytes)
                .filter(|total| *total <= memory_budget)
                .ok_or(PackReject::WorkingSetExceeded)?;
            chunks.push(upload_phase(phase, device, queue, ledger)?);
        }
        Ok(Self { view, chunks, packed_bytes: candidate.packed_bytes })
    }

    pub(super) fn covers(&self, config: &Config) -> bool {
        ViewBounds::from_config(config).is_some_and(|next| self.view.covers(next))
    }
}

fn upload_phase(
    phase: PhasePackedChunk,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    ledger: &Arc<GpuLedger>,
) -> Result<GpuPhaseChunk, PackReject> {
    let row_count = phase.chunk.rows.len();
    let source_runs = runs_from_rows(&phase.chunk.rows)?;
    let mut pair_bytes = 0u64;
    for column in &phase.chunk.columns {
        pair_bytes = pair_bytes.checked_add(column.len() as u64)
            .ok_or(PackReject::WorkingSetExceeded)?;
    }
    let total = pair_bytes;
    if total == 0 || total > device.limits().max_buffer_size
        || total > u64::from(device.limits().max_storage_buffer_binding_size)
    {
        return Err(PackReject::WorkingSetExceeded);
    }
    let mut columns = Vec::new();
    let mut ranges = Vec::new();
    columns.try_reserve_exact(phase.chunk.columns.len()).map_err(|_| PackReject::AllocationFailed)?;
    ranges.try_reserve_exact(phase.chunk.columns.len()).map_err(|_| PackReject::AllocationFailed)?;
    let buffer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // gpu-alloc: ViewResident
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("figgy view-local packed rows"),
            size: total,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    })).map_err(|_| PackReject::AllocationFailed)?;
    let work = TrackedBuffer::new(ledger, GpuResourceKind::ViewResident, buffer);
    let mut offset = 0u64;
    for (index, bytes) in phase.chunk.columns.iter().enumerate() {
        let len = u64::try_from(row_count).map_err(|_| PackReject::WorkingSetExceeded)?;
        let range = ColumnRange {
            column: index as u64, revision: 0, source_len: len,
            offset: 0, len, encoding: SourceEncoding::HiLoF32,
        };
        queue.write_buffer(&work, offset, bytes);
        columns.push(UploadedColumn {
            range, offset_bytes: offset, pair_bytes: bytes.len() as u64,
            statistics: None,
        });
        ranges.push(range);
        offset += bytes.len() as u64;
    }
    Ok(GpuPhaseChunk {
        series: phase.series,
        phase: phase.phase,
        chunk: RecordedChunk::from_packed_view(work, columns),
        ranges,
        source_runs,
    })
}

impl ViewPackedCandidate {
    pub(super) fn new(view: ViewBounds, limit: u64) -> Self {
        Self { view, chunks: Vec::new(), gpu_chunks: Vec::new(), packed_bytes: 0, limit, rejected: None }
    }

    pub(super) fn for_chart(config: &Config, series: &[SeriesConfig], limit: u64) -> Option<Self> {
        if !matches!(config.draw_style, DrawStyle::Precise) || series.is_empty() {
            return None;
        }
        let supported = series.iter().all(|item| match &item.render_type {
            DataRenderType::Histogram { .. }
            | DataRenderType::Heatmap { .. }
            | DataRenderType::Contour { .. }
            | DataRenderType::HeatmapContour { .. } => false,
            other => {
                let scatter_ok = super::extract_scatter(other).is_none_or(|scatter| {
                    scatter.point_style_index_column.is_none()
                        && scatter.point_style_table.is_none()
                        && scatter.point_style_overrides.is_none()
                });
                let line_ok = super::extract_line(other).is_none_or(|line| {
                    matches!(line.line_style, LineStylePreset::Solid)
                });
                let error_ok = super::extract_errorbar_style(other).is_none_or(|style| {
                    style.error_bar_style_index_column.is_none()
                        && style.error_bar_style_table.is_none()
                        && style.error_bar_style_overrides.is_none()
                });
                scatter_ok && line_ok && error_ok
            }
        });
        if !supported { return None; }
        Some(Self::new(ViewBounds::from_config(config)?, limit))
    }

    pub(super) fn push(&mut self, series: usize, phase: StreamDrawPhase, chunk: PackedViewChunk) {
        if self.rejected.is_some() { return; }
        if chunk.rows.is_empty() { return; }
        let Some(bytes) = chunk.byte_len() else {
            self.reject(PackReject::WorkingSetExceeded);
            return;
        };
        // Consecutive source chunks belong to the same series and primitive
        // pass. Coalesce sparse visible rows so a billion-row stream does not
        // leave one tiny GPU buffer and one pick dispatch per source chunk.
        // Line chunks retain an explicit break at the old draw boundary.
        let merge = self.chunks.last().is_some_and(|last| {
            last.series == series && last.phase == phase
                && last.chunk.columns.len() == chunk.columns.len()
                && last.chunk.byte_len().and_then(|old| old.checked_add(bytes))
                    .is_some_and(|size| size <= 8 * 1024 * 1024)
        });
        let separator_bytes = if merge && phase == StreamDrawPhase::Line {
            (chunk.columns.len() as u64).checked_mul(8)
        } else { Some(0) };
        let Some(extra) = separator_bytes.and_then(|size| size.checked_add(bytes)) else {
            self.reject(PackReject::WorkingSetExceeded);
            return;
        };
        let Some(total) = self.packed_bytes.checked_add(extra) else {
            self.reject(PackReject::WorkingSetExceeded);
            return;
        };
        if total > self.limit {
            // The non-merged representation has no extra separator.
            if merge && self.packed_bytes.checked_add(bytes).is_some_and(|size| size <= self.limit) {
                self.push_unmerged(series, phase, chunk, bytes);
                return;
            }
            self.reject(PackReject::WorkingSetExceeded);
            return;
        }
        if merge {
            let last = &mut self.chunks.last_mut().unwrap().chunk;
            let separator = usize::from(phase == StreamDrawPhase::Line);
            if last.rows.try_reserve(chunk.rows.len() + separator).is_err() {
                self.reject(PackReject::AllocationFailed);
                return;
            }
            for (old, next) in last.columns.iter_mut().zip(&chunk.columns) {
                if old.try_reserve(next.len() + separator * 8).is_err() {
                    self.reject(PackReject::AllocationFailed);
                    return;
                }
            }
            if separator != 0 {
                last.rows.push(u64::MAX);
                for old in &mut last.columns {
                    old.extend_from_slice(&f32::NAN.to_le_bytes());
                    old.extend_from_slice(&0f32.to_le_bytes());
                }
            }
            last.rows.extend(chunk.rows);
            for (old, mut next) in last.columns.iter_mut().zip(chunk.columns) {
                old.append(&mut next);
            }
            self.packed_bytes = total;
            return;
        }
        self.push_unmerged(series, phase, chunk, bytes);
    }

    fn push_unmerged(&mut self, series: usize, phase: StreamDrawPhase, chunk: PackedViewChunk, bytes: u64) {
        if self.chunks.try_reserve(1).is_err() {
            self.reject(PackReject::AllocationFailed);
            return;
        }
        self.chunks.push(PhasePackedChunk { series, phase, chunk });
        self.packed_bytes += bytes;
    }

    /// Flush every completed CPU page while retaining only the coalescing
    /// tail. The configured cap has already been checked by `push`; the total
    /// GPU budget is checked against live allocations before each page.
    pub(super) fn flush_completed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ledger: &Arc<GpuLedger>,
        memory_budget: Option<u64>,
        current_gpu_bytes: u64,
    ) {
        if self.rejected.is_some() || self.chunks.is_empty() { return; }
        let ready = self.chunks.len() >= 2
            || self.chunks[0].chunk.byte_len().is_some_and(|bytes| bytes >= 8 * 1024 * 1024);
        if !ready { return; }
        let Some(budget) = memory_budget else {
            self.reject(PackReject::MemoryBudgetUnset);
            return;
        };
        let Some(bytes) = self.chunks[0].chunk.byte_len() else {
            self.reject(PackReject::WorkingSetExceeded);
            return;
        };
        if current_gpu_bytes.checked_add(bytes).is_none_or(|total| total > budget) {
            self.reject(PackReject::WorkingSetExceeded);
            return;
        }
        if self.gpu_chunks.try_reserve(1).is_err() {
            self.reject(PackReject::AllocationFailed);
            return;
        }
        let phase = self.chunks.remove(0);
        match upload_phase(phase, device, queue, ledger) {
            Ok(chunk) => self.gpu_chunks.push(chunk),
            Err(reason) => self.reject(reason),
        }
    }

    pub(super) fn reject(&mut self, reason: PackReject) {
        self.chunks.clear();
        self.gpu_chunks.clear();
        self.packed_bytes = 0;
        self.rejected = Some(reason);
    }
}

/// One phase of one submitted stream chunk, retaining only original rows whose
/// geometry can reach the current data area. `rows` contains original source
/// indices; `u64::MAX` is a non-source separator for disjoint line runs.
#[derive(Debug)]
pub(super) struct PackedViewChunk {
    pub(super) rows: Vec<u64>,
    pub(super) columns: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PackReject {
    InvalidChunk,
    AllocationFailed,
    WorkingSetExceeded,
    MemoryBudgetUnset,
    UnsupportedGeometry,
    Disabled,
}

impl PackReject {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::InvalidChunk => "invalid_chunk",
            Self::AllocationFailed => "allocation_failed",
            Self::WorkingSetExceeded => "working_set_exceeded",
            Self::MemoryBudgetUnset => "memory_budget_unset",
            Self::UnsupportedGeometry => "unsupported_geometry",
            Self::Disabled => "disabled",
        }
    }
}

/// A bounded, temporary tap of the pairs produced by the upload writer. A
/// source may write its pairs in any order; the final value at each index is
/// the one submitted to GPU staging. This is discarded after the view rows are
/// selected, so it is never a whole-column CPU mirror.
pub(super) struct ChunkPairCapture {
    pairs: Vec<Vec<[f32; 2]>>,
}

impl ChunkPairCapture {
    pub(super) fn new(lengths: impl IntoIterator<Item = u64>) -> Result<Self, PackReject> {
        let mut pairs = Vec::new();
        for len in lengths {
            pairs.try_reserve(1).map_err(|_| PackReject::AllocationFailed)?;
            let len = usize::try_from(len).map_err(|_| PackReject::InvalidChunk)?;
            let mut column = Vec::new();
            column.try_reserve_exact(len).map_err(|_| PackReject::AllocationFailed)?;
            column.resize(len, [f32::NAN, 0.0]);
            pairs.push(column);
        }
        Ok(Self { pairs })
    }

    pub(super) fn record(&mut self, column: usize, row: usize, hi: f32, lo: f32) {
        if let Some(pair) = self.pairs.get_mut(column).and_then(|values| values.get_mut(row)) {
            *pair = [hi, lo];
        }
    }

    pub(super) fn pack(
        &self,
        view: ViewBounds,
        layout: &[UploadedColumn],
        y_index: usize,
        line: bool,
        visual_radius_px: f64,
        errors: Option<ErrorPairColumns>,
        max_bytes: u64,
    ) -> Result<PackedViewChunk, PackReject> {
        pack_visible_chunk(view, layout, &self.pairs, y_index, line, visual_radius_px, errors, max_bytes)
    }
}

#[derive(Clone, Copy)]
pub(super) struct ErrorPairColumns {
    x: Option<(usize, usize)>,
    y: Option<(usize, usize)>,
}

impl ErrorPairColumns {
    pub(super) fn for_series(series: &SeriesConfig) -> Result<Self, PackReject> {
        let columns = super::stream_phase_columns(series, StreamDrawPhase::Errorbar);
        let position = |id: &str| columns.ids[..columns.count].iter()
            .position(|column| *column == id).ok_or(PackReject::InvalidChunk);
        let pair = |error: Option<&ErrorRef>| -> Result<Option<(usize, usize)>, PackReject> {
            match error {
                None => Ok(None),
                Some(ErrorRef::Symmetric { column }) => {
                    let index = position(column)?;
                    Ok(Some((index, index)))
                }
                Some(ErrorRef::Asymmetric { lower, upper }) => {
                    Ok(Some((position(lower)?, position(upper)?)))
                }
            }
        };
        Ok(Self {
            x: pair(super::extract_err_x(&series.render_type))?,
            y: pair(super::extract_err_y(&series.render_type))?,
        })
    }
}

impl PackedViewChunk {
    pub(super) fn byte_len(&self) -> Option<u64> {
        self.columns.iter().try_fold(
            0u64,
            |sum, column| sum.checked_add(column.len() as u64),
        )
    }
}

/// Inspect the already-encoded `(hi, lo)` staging pairs. No original column
/// is reread or retained. The caller charges every packed row and separator
/// against the chart-local cap before publishing this candidate.
pub(super) fn pack_visible_chunk(
    view: ViewBounds,
    layout: &[UploadedColumn],
    pairs: &[Vec<[f32; 2]>],
    y_index: usize,
    line: bool,
    visual_radius_px: f64,
    errors: Option<ErrorPairColumns>,
    max_bytes: u64,
) -> Result<PackedViewChunk, PackReject> {
    let first = layout.first().ok_or(PackReject::InvalidChunk)?;
    let y = pairs.get(y_index).ok_or(PackReject::InvalidChunk)?;
    let len = usize::try_from(first.range.len).map_err(|_| PackReject::InvalidChunk)?;
    if pairs.len() != layout.len() || pairs.iter().any(|column| column.len() != len)
        || layout.iter().any(|column| column.range.offset != first.range.offset || column.range.len != first.range.len) {
        return Err(PackReject::InvalidChunk);
    }
    let point = |index: usize| -> Option<(f64, f64)> {
        let x = pairs.first()?.get(index)?;
        let y = y.get(index)?;
        Some((x[0] as f64 + x[1] as f64, y[0] as f64 + y[1] as f64))
    };
    let mut selected = Vec::new();
    selected.try_reserve_exact(len).map_err(|_| PackReject::AllocationFailed)?;
    selected.resize(len, false);
    if line {
        for index in 0..len.saturating_sub(1) {
            if let (Some(a), Some(b)) = (point(index), point(index + 1))
                && view.segment_intersects(a, b, visual_radius_px)
            {
                selected[index] = true;
                selected[index + 1] = true;
            }
        }
    } else {
        for (index, needed) in selected.iter_mut().enumerate() {
            *needed = point(index).is_some_and(|xy| view.point_intersects(xy, visual_radius_px));
        }
    }
    if let Some(errors) = errors {
        let value = |column: usize, index: usize| -> Option<f64> {
            let pair = pairs.get(column)?.get(index)?;
            let result = pair[0] as f64 + pair[1] as f64;
            result.is_finite().then_some(result)
        };
        for (index, needed) in selected.iter_mut().enumerate() {
            if *needed { continue; }
            let Some((x, y)) = point(index) else { continue; };
            if let Some((lower, upper)) = errors.x
                && let (Some(lower), Some(upper)) = (value(lower, index), value(upper, index))
            {
                let a = (x - lower, y);
                let b = (x + upper, y);
                *needed |= view.segment_intersects(a, b, visual_radius_px)
                    || view.point_intersects(a, visual_radius_px)
                    || view.point_intersects(b, visual_radius_px);
            }
            if let Some((lower, upper)) = errors.y
                && let (Some(lower), Some(upper)) = (value(lower, index), value(upper, index))
            {
                let a = (x, y - lower);
                let b = (x, y + upper);
                *needed |= view.segment_intersects(a, b, visual_radius_px)
                    || view.point_intersects(a, visual_radius_px)
                    || view.point_intersects(b, visual_radius_px);
            }
        }
    }
    let kept = selected.iter().filter(|&&needed| needed).count();
    let breaks = if line {
        selected.iter().enumerate().filter(|(index, needed)| {
            **needed && *index > 0 && !selected[*index - 1]
        }).count()
    } else { 0 };
    let output_rows = kept.checked_add(breaks).ok_or(PackReject::WorkingSetExceeded)?;
    let row_bytes = (layout.len() as u64).checked_mul(8)
        .and_then(|bytes| bytes.checked_mul(output_rows as u64))
        .ok_or(PackReject::WorkingSetExceeded)?;
    if row_bytes > max_bytes { return Err(PackReject::WorkingSetExceeded); }
    let mut rows = Vec::new();
    rows.try_reserve_exact(output_rows).map_err(|_| PackReject::AllocationFailed)?;
    let mut columns = Vec::new();
    columns.try_reserve_exact(layout.len()).map_err(|_| PackReject::AllocationFailed)?;
    for _ in layout {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(output_rows.checked_mul(8).ok_or(PackReject::WorkingSetExceeded)?)
            .map_err(|_| PackReject::AllocationFailed)?;
        columns.push(bytes);
    }
    for (index, needed) in selected.iter().copied().enumerate() {
        if !needed { continue; }
        if line && index > 0 && !selected[index - 1] {
            rows.push(u64::MAX);
            for bytes in &mut columns {
                bytes.extend_from_slice(&f32::NAN.to_le_bytes());
                bytes.extend_from_slice(&0f32.to_le_bytes());
            }
        }
        rows.push(first.range.offset + index as u64);
        for (bytes, column) in columns.iter_mut().zip(pairs) {
            let pair = column.get(index).ok_or(PackReject::InvalidChunk)?;
            bytes.extend_from_slice(&pair[0].to_le_bytes());
            bytes.extend_from_slice(&pair[1].to_le_bytes());
        }
    }
    Ok(PackedViewChunk { rows, columns })
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ViewBounds {
    x: AxisProjection,
    y: AxisProjection,
    width: f64,
    height: f64,
}

#[derive(Clone, Copy, Debug)]
struct AxisProjection {
    min: f64,
    span: f64,
    logarithmic: bool,
    inverted: bool,
}

impl AxisProjection {
    fn new(axis: &AxisOptions) -> Option<Self> {
        let logarithmic = matches!(axis.scale, AxisScale::Logarithmic);
        let (min, max) = if logarithmic {
            (axis.min.log10(), axis.max.log10())
        } else {
            (axis.min, axis.max)
        };
        let span = max - min;
        (min.is_finite() && span.is_finite() && span > 0.0).then_some(Self {
            min,
            span,
            logarithmic,
            inverted: axis.inverted,
        })
    }

    fn project(self, value: f64) -> Option<f64> {
        if !value.is_finite() || (self.logarithmic && value <= 0.0) {
            return None;
        }
        let value = if self.logarithmic { value.log10() } else { value };
        let projected = (value - self.min) / self.span;
        projected.is_finite().then_some(if self.inverted {
            1.0 - projected
        } else {
            projected
        })
    }
}

impl ViewBounds {
    fn covers(self, next: Self) -> bool {
        let axis = |old: AxisProjection, new: AxisProjection| {
            old.logarithmic == new.logarithmic
                && old.inverted == new.inverted
                && new.min >= old.min
                && new.min + new.span <= old.min + old.span
        };
        next.width >= self.width && next.height >= self.height
            && axis(self.x, next.x) && axis(self.y, next.y)
    }

    pub(super) fn from_config(config: &Config) -> Option<Self> {
        let area = config.data_area().ok()?.0;
        if area.width == 0 || area.height == 0 {
            return None;
        }
        Some(Self {
            x: AxisProjection::new(&config.bottom_x)?,
            y: AxisProjection::new(&config.left_y)?,
            width: f64::from(area.width),
            height: f64::from(area.height),
        })
    }

    fn project(self, point: (f64, f64)) -> Option<(f64, f64)> {
        Some((self.x.project(point.0)?, self.y.project(point.1)?))
    }

    pub(super) fn point_intersects(self, point: (f64, f64), radius_px: f64) -> bool {
        let Some((x, y)) = self.project(point) else { return false };
        let pad_x = radius_px.max(0.0) / self.width;
        let pad_y = radius_px.max(0.0) / self.height;
        x >= -pad_x && x <= 1.0 + pad_x && y >= -pad_y && y <= 1.0 + pad_y
    }

    pub(super) fn segment_intersects(
        self,
        first: (f64, f64),
        second: (f64, f64),
        half_width_px: f64,
    ) -> bool {
        let (Some(a), Some(b)) = (self.project(first), self.project(second)) else {
            return false;
        };
        let pad_x = half_width_px.max(0.0) / self.width;
        let pad_y = half_width_px.max(0.0) / self.height;
        // Liang–Barsky clipping against the padded data-area rectangle.
        let mut enter = 0.0f64;
        let mut leave = 1.0f64;
        for (origin, delta, lower, upper) in [
            (a.0, b.0 - a.0, -pad_x, 1.0 + pad_x),
            (a.1, b.1 - a.1, -pad_y, 1.0 + pad_y),
        ] {
            if delta == 0.0 {
                if origin < lower || origin > upper { return false; }
                continue;
            }
            let first_t = (lower - origin) / delta;
            let second_t = (upper - origin) / delta;
            enter = enter.max(first_t.min(second_t));
            leave = leave.min(first_t.max(second_t));
            if enter > leave { return false; }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::streaming::{ColumnRange, SourceEncoding};

    fn encoded(values: &[f32]) -> Vec<[f32; 2]> {
        values.iter().map(|&value| [value, 0.0]).collect()
    }

    fn test_layout(len: u64) -> [UploadedColumn; 2] {
        [0, 1].map(|column| UploadedColumn {
            range: ColumnRange { column, revision: 7, source_len: len, offset: 0, len, encoding: SourceEncoding::ScalarF32 },
            offset_bytes: column * len * 8,
            pair_bytes: len * 8,
            statistics: None,
        })
    }

    #[test]
    fn crossing_segment_retains_both_offscreen_endpoints() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(crate::layout::Rect { x: 0, y: 0, width: 400, height: 300 });
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 10.0;
        config.left_y.min = 0.0;
        config.left_y.max = 10.0;
        let view = ViewBounds::from_config(&config).unwrap();
        assert!(!view.point_intersects((-5.0, 5.0), 0.0));
        assert!(!view.point_intersects((15.0, 5.0), 0.0));
        assert!(view.segment_intersects((-5.0, 5.0), (15.0, 5.0), 0.0));
        assert!(!view.segment_intersects((-5.0, 15.0), (15.0, 15.0), 0.0));
    }

    #[test]
    fn invalid_log_points_do_not_enter_cache() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(crate::layout::Rect { x: 0, y: 0, width: 400, height: 300 });
        config.bottom_x.scale = AxisScale::Logarithmic;
        config.bottom_x.min = 1.0;
        config.bottom_x.max = 100.0;
        let view = ViewBounds::from_config(&config).unwrap();
        assert!(!view.point_intersects((-1.0, 0.5), 0.0));
        assert!(!view.segment_intersects((-1.0, 0.5), (10.0, 0.5), 0.0));
    }

    #[test]
    fn packing_preserves_original_rows_and_charges_gap_separators() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(crate::layout::Rect { x: 0, y: 0, width: 400, height: 300 });
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 10.0;
        config.left_y.min = 0.0;
        config.left_y.max = 10.0;
        let view = ViewBounds::from_config(&config).unwrap();
        let x = encoded(&[-5.0, 5.0, 15.0, 20.0, 20.0, 15.0, 5.0, -5.0]);
        let y = encoded(&[5.0, 5.0, 5.0, 15.0, 20.0, 5.0, 5.0, 5.0]);
        let pairs = [x, y];
        let layout = test_layout(8);
        let chunk = pack_visible_chunk(view, &layout, &pairs, 1, true, 0.0, None, 112).unwrap();
        assert_eq!(chunk.rows, [0, 1, 2, u64::MAX, 5, 6, 7]);
        assert_eq!(chunk.byte_len(), Some(112));
        assert_eq!(pack_visible_chunk(view, &layout, &pairs, 1, true, 0.0, None, 111).unwrap_err(), PackReject::WorkingSetExceeded);
    }

    #[test]
    fn sparse_source_runs_preserve_packed_offsets_and_skip_gaps() {
        let runs = runs_from_rows(&[10, 11, u64::MAX, 20, 21, 30]).unwrap();
        assert_eq!(runs, [
            SourceRun { source_start: 10, len: 2, packed_start: 0 },
            SourceRun { source_start: 20, len: 2, packed_start: 3 },
            SourceRun { source_start: 30, len: 1, packed_start: 5 },
        ]);
        assert_eq!(next_in_runs(&runs, 10, true), Some(11));
        assert_eq!(source_in_runs(&runs, 0), Some(10));
        assert_eq!(source_in_runs(&runs, 1), Some(11));
        assert_eq!(source_in_runs(&runs, 2), None, "line separator is never a source row");
        assert_eq!(source_in_runs(&runs, 3), Some(20));
        assert_eq!(source_in_runs(&runs, 5), Some(30));
        assert_eq!(next_in_runs(&runs, 11, true), Some(20));
        assert_eq!(next_in_runs(&runs, 21, true), Some(30));
        assert_eq!(next_in_runs(&runs, 30, true), None);
        assert_eq!(next_in_runs(&runs, 30, false), Some(21));
        assert_eq!(next_in_runs(&runs, 20, false), Some(11));
        assert_eq!(next_in_runs(&runs, 10, false), None);
    }

    #[test]
    fn errorbar_endpoint_inside_view_keeps_offscreen_anchor() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(crate::layout::Rect { x: 0, y: 0, width: 400, height: 300 });
        config.bottom_x.min = 0.0;
        config.bottom_x.max = 10.0;
        config.left_y.min = 0.0;
        config.left_y.max = 10.0;
        let view = ViewBounds::from_config(&config).unwrap();
        let pairs = [encoded(&[5.0]), encoded(&[12.0]), encoded(&[5.0])];
        let layout = [0, 1, 2].map(|column| UploadedColumn {
            range: ColumnRange {
                column, revision: 1, source_len: 1, offset: 0, len: 1,
                encoding: SourceEncoding::ScalarF32,
            },
            offset_bytes: column * 8, pair_bytes: 8, statistics: None,
        });
        let packed = pack_visible_chunk(
            view, &layout, &pairs, 1, false, 3.0,
            Some(ErrorPairColumns { x: None, y: Some((2, 2)) }), 28,
        ).unwrap();
        assert_eq!(packed.rows, [0]);
    }

    #[test]
    fn wholly_offscreen_chunk_does_not_cancel_view_cache() {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(crate::layout::Rect { x: 0, y: 0, width: 400, height: 300 });
        let view = ViewBounds::from_config(&config).unwrap();
        let layout = test_layout(2);
        let pairs = [encoded(&[-100.0, -90.0]), encoded(&[-100.0, -90.0])];
        let empty = pack_visible_chunk(view, &layout, &pairs, 1, true, 0.0, None, 500_000_000).unwrap();
        assert!(empty.rows.is_empty());
        let mut candidate = ViewPackedCandidate::new(view, 500_000_000);
        candidate.push(0, StreamDrawPhase::Line, empty);
        assert!(candidate.chunks.is_empty());
        assert_eq!(candidate.packed_bytes, 0);
        assert_eq!(candidate.rejected, None);
    }

    #[test]
    fn completed_page_moves_to_gpu_before_stream_completion_and_uses_configured_limit() {
        let (device, queue) = crate::data_render::shared_device().expect("stream GPU required");
        let ledger = Arc::new(GpuLedger::new());
        let config = crate::default::default_config();
        let view = ViewBounds::from_config(&config).unwrap();
        let one_row = |row| PackedViewChunk {
            rows: vec![row],
            columns: vec![vec![0; 8], vec![0; 8]],
        };
        let mut candidate = ViewPackedCandidate::new(view, 32);
        candidate.push(0, StreamDrawPhase::Line, one_row(0));
        candidate.push(0, StreamDrawPhase::Scatter, one_row(1));
        assert_eq!(candidate.packed_bytes, 32);
        candidate.flush_completed(&device, &queue, &ledger, Some(32), 0);
        assert_eq!(candidate.gpu_chunks.len(), 1);
        assert_eq!(candidate.chunks.len(), 1, "only the current CPU page remains");
        assert_eq!(candidate.rejected, None);

        let mut too_small = ViewPackedCandidate::new(view, 31);
        too_small.push(0, StreamDrawPhase::Line, one_row(0));
        too_small.push(0, StreamDrawPhase::Scatter, one_row(1));
        assert_eq!(too_small.rejected, Some(PackReject::WorkingSetExceeded));

        let mut budget_refused = ViewPackedCandidate::new(view, 32);
        budget_refused.push(0, StreamDrawPhase::Line, one_row(0));
        budget_refused.push(0, StreamDrawPhase::Scatter, one_row(1));
        budget_refused.flush_completed(&device, &queue, &ledger, Some(15), 0);
        assert_eq!(budget_refused.rejected, Some(PackReject::WorkingSetExceeded));
        assert!(budget_refused.gpu_chunks.is_empty());
    }
}
