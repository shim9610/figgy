//! GPU columnar memory pool — column-sized slabs in one big buffer, with an
//! offset table and ping-pong defrag.
//!
//! - One `primary` GPU buffer holds every column, packed by offset.
//! - `slots: HashMap<ColumnId, ColumnSlot>` is the SSoT mapping id → byte
//!   range; `free: Vec<FreeRegion>` tracks holes (first-fit on add, coalesce
//!   with neighbors on remove).
//! - Column sources write pairs and collect `min_positive` in one pass over
//!   the mapped staging range. wgpu 29+ maps that range as write-only, so the
//!   renderer never reads the mapped bytes back.
//! - `defragment` packs survivors into a backup buffer with GPU-internal
//!   copies (no PCIe traffic) then swaps `primary <-> backup`.
//!
//! # Source kinds
//!
//! Per-point data enters the pool in one of three kinds, all through
//! [`ColumnSource`]:
//!
//! - **Columns**: one `ColumnSource` per id ([`ColumnPool::add_column`],
//!   [`ColumnPool::add_columns`], the upserts). A [`ColumnSlot`] holds its
//!   byte range and scalar statistics.
//! - **Records** ([`RecordSource`], [`ColumnPool::upsert_record`]): a fixed
//!   set of named scalar fields per row, checked by name against a static
//!   [`RecordSchema`]. Every field is a column of the pool, uploaded together
//!   in one batch transaction (one region, one staging write); a
//!   [`RecordSlot`] names the field columns in schema order in a fixed array
//!   of [`MAX_RECORD_FIELDS`] and keeps the per-field [`RecordStats`].
//! - **Ragged columns** ([`RaggedSource`], [`ColumnPool::upsert_ragged`]):
//!   variable-length groups of values, group `g` being
//!   `values[offsets[g]..offsets[g + 1]]`. The values and the offsets are two
//!   columns of the pool uploaded in one batch; a [`RaggedSlot`] names them
//!   and keeps the [`RaggedStats`], measured while the offsets are written
//!   (a non-monotone offsets column is rejected).
//!
//! The pool has no integer column type, so ragged offsets are `(hi, lo)` f32
//! pairs like every other column; offsets up to 2^24 are exact in the high
//! lane, which is why a ragged column holds at most 2^24 values. A shader
//! reads a record field or a ragged column by its pool lane base
//! (`handle.offset / 4`), as every column (`SHADER_COMMON.md`).
//!
//! The member columns of a record or ragged column are owned by it: their ids
//! are internal, the host cannot replace, remove or stream them, and deleting
//! the record ([`ColumnPool::remove_record`], [`ColumnPool::remove_ragged`])
//! frees them. Records and ragged columns are resident only: streaming is not
//! extended to them, and the renderer rejects a streamed id on either side.
//!
//! `ColumnHandle` carries a public `generation` value. Mutations that can
//! invalidate positional handles, including removal, replacement, relocation,
//! and clear, bump it so callers can detect staleness with `is_valid_handle`
//! and re-fetch with `handle_for`.
//!
//! Auto-defrag is opt-in via [`DefragPolicy`] (default `Manual`). With
//! `OnAllocFailure`, an `OutOfSpace` from `add_column` triggers one
//! `defragment()` and a single retry.
//!
//! All offsets and sizes are [`ALIGN`] = 256-byte aligned to satisfy wgpu's
//! storage-binding alignment; vertex slices reuse the same value to keep
//! mode switching free of caveats.

use std::{collections::HashMap, fmt, sync::Arc};

use crate::gpu_memory::RetiredBytes;
use wgpu::{Buffer, BufferDescriptor, BufferUsages, Device, Queue};

use crate::data::{
    COLUMN_VALUE_BYTES, ColumnPairWriter, ColumnSource, ColumnUploadStats, HiLoColumnSource,
    StreamColumnSource,
};

// Defined in the model crate (`model::data`); re-exported here so
// `data_render::ColumnId` stays a valid path.
pub use crate::data::ColumnId;

mod alloc;
mod monitor;
mod transactions;

pub use alloc::{ColumnDefragment, DefragPolicy, FreeRegion, GrowthPolicy};
use alloc::{RegionUndo, align_up, alloc_region, alloc_region_from, batch_region_bytes};
use alloc::{coalesce_regions, column_raw_bytes, column_region_bytes, out_of_space};
use alloc::{plan_batch_chunks, streamed_batch_region_bytes, try_align_up};
pub(crate) use monitor::PoolIdentity;
pub use monitor::{GpuAllocCtx, GpuBudget};
use monitor::{buffer_ceiling, create_buffer_checked};
pub(crate) use transactions::StreamedColumnUpload;
pub use transactions::{ColumnBatchInsert, ColumnRemoval, ColumnUpsert};
pub(crate) use transactions::{ColumnBatchUpsert, ResidentBatchCapacityError};

/// Alignment (in bytes) for every offset and size in the pool.
pub const ALIGN: u64 = 256;

/// One column's occupied region inside the pool.
#[derive(Debug, Clone)]
pub struct ColumnSlot {
    pub id: ColumnId,
    pub offset: u64,
    pub byte_size: u64,
    pub len_values: usize,
    pub generation: u32,
    /// Captured from `ColumnSource::min/max` at `add_column` time so auto-fit
    /// can read the range without rescanning data that lives on the GPU.
    pub min: f64,
    pub max: f64,
    /// Smallest strictly-positive value, collected during upload — the
    /// log-axis auto-fit lower bound when the data contains zeros or
    /// negatives. `None` when no positive value exists.
    ///
    /// Scalar stats like this are the ONLY per-value information retained on
    /// the CPU. The pool deliberately keeps no copy of the data itself —
    /// per-point geometry (dashed-line arc length) is computed on the GPU
    /// (`line_arc.wgsl`). Do not reintroduce CPU shadows.
    pub min_positive: Option<f64>,
    /// The stacked extent of the batch the column was uploaded in, when that
    /// batch could be a bar group (see [`StackExtent`]). `None` for a column
    /// uploaded on its own or in a larger batch; replacing the column clears
    /// it. A scalar statistic like `min_positive`, not a copy of the data.
    pub stack_extent: Option<StackExtent>,
}

/// The stacked extent of a bar group, measured while its value columns are
/// uploaded together ([`ColumnPool::add_columns`]): the upload pass adds every
/// finite value of row `i` to that row's positive or negative total in a
/// fixed `[f64; MAX_CATEGORIES]` accumulator, and the extent is the lowest
/// negative and the highest positive total. A stacked category bar group fits
/// its value axis to it (`Chart::auto_fit_layer_union`).
///
/// Every column of the batch carries the same record. A batch measures one
/// when it has at most `MAX_BAR_SERIES` columns of at most `MAX_CATEGORIES`
/// values each. A bar group's columns hold its extent exactly when they all
/// carry the record of one batch and the batch had exactly those columns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StackExtent {
    /// The lowest negative row total, 0 when no row has a negative value.
    pub min: f64,
    /// The highest positive row total, 0 when no row has a positive value.
    pub max: f64,
    /// The batch: the allocation epoch of its first column.
    pub batch: u64,
    /// The number of columns in the batch.
    pub members: usize,
}

/// The most fields one record holds. The box plot's schema has nine;
/// twelve leaves room for three more without changing the slot layout, and
/// twelve field lane bases fill exactly three `vec4<u32>` of a shader
/// uniform.
pub const MAX_RECORD_FIELDS: usize = 12;

/// The most values one ragged column holds: its offsets are f32 pairs, and
/// every integer up to 2^24 is exact in the high lane.
pub const MAX_RAGGED_VALUES: usize = 1 << 24;

/// Separates a record or ragged column id from its member column ids. Record
/// and ragged ids may not contain it.
const MEMBER_SEPARATOR: char = '\u{1f}';

/// The id of a record source in the pool. Records, ragged columns and
/// columns share one id space.
pub type RecordId = ColumnId;

/// The id of a ragged column in the pool. Records, ragged columns and
/// columns share one id space.
pub type RaggedId = ColumnId;

/// A record schema: the field names in slot order, each with whether the
/// field is required. Static per consumer; a record's fields are checked by
/// name against it at upsert.
pub type RecordSchema = [(&'static str, bool)];

/// The box plot's record schema: one row per box. The five summary fields
/// are required; `mean`, `sample_count`, `ci_low` and `ci_high` are optional,
/// and an optional field that is not supplied reads as NaN. A missing box is
/// a row whose every field is NaN.
pub const BOX_PLOT_RECORD: &RecordSchema = &[
    ("median", true),
    ("q1", true),
    ("q3", true),
    ("whisker_low", true),
    ("whisker_high", true),
    ("mean", false),
    ("sample_count", false),
    ("ci_low", false),
    ("ci_high", false),
];

/// A record: named scalar fields of the same length, one row per element.
///
/// `fields` pairs a schema field name with its column source, in any order.
/// Every name must be a field of `schema`, every required field of `schema`
/// must be present, no name may repeat, and every source must have the same
/// non-zero length; [`ColumnPool::upsert_record`] rejects anything else with
/// an explicit error.
#[derive(Clone, Copy)]
pub struct RecordSource<'a> {
    pub schema: &'static RecordSchema,
    pub fields: &'a [(&'a str, &'a dyn ColumnSource)],
}

/// A ragged column: `offsets.len() - 1` groups of values, group `g` being
/// `values[offsets[g]..offsets[g + 1]]`.
///
/// `offsets` starts at 0, never decreases, holds integers and ends at
/// `values.len()`. Both sources are non-empty: a host whose every group is
/// empty registers no ragged column. [`ColumnPool::upsert_ragged`] checks the
/// offsets while it writes them and rejects anything else with an explicit
/// error.
#[derive(Clone, Copy)]
pub struct RaggedSource<'a> {
    pub values: &'a dyn ColumnSource,
    pub offsets: &'a dyn ColumnSource,
}

/// The scalar statistics of a record, measured from the encoded `(hi, lo)`
/// values while the fields were written. Index `k` is schema field `k`;
/// a field that is not present has `present[k] == false`, no counts and an
/// empty range (`min = +inf`, `max = -inf`), as has a field without a
/// finite value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordStats {
    /// Rows: the length of every field.
    pub len: u32,
    pub present: [bool; MAX_RECORD_FIELDS],
    /// Smallest finite value per field.
    pub min: [f64; MAX_RECORD_FIELDS],
    /// Largest finite value per field.
    pub max: [f64; MAX_RECORD_FIELDS],
    /// Finite values per field.
    pub finite: [u32; MAX_RECORD_FIELDS],
    /// NaN values per field (infinities are `len - finite - nan`).
    pub nan: [u32; MAX_RECORD_FIELDS],
}

/// The scalar statistics of a ragged column, measured while it was written.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RaggedStats {
    pub groups: u32,
    /// Values in all groups: `offsets[groups]`.
    pub total: u32,
    /// Smallest finite value, `+inf` without one.
    pub min: f64,
    /// Largest finite value, `-inf` without one.
    pub max: f64,
    pub finite: u32,
    pub nan: u32,
    /// Whether the offsets never decrease. Always `true` for a resident
    /// ragged column: an upload whose offsets decrease is rejected.
    pub non_decreasing: bool,
    /// The length of the longest group.
    pub max_group: u32,
}

/// A resident record: its member columns in schema order, a fixed array with
/// no `Vec`. `epoch` is the allocation epoch of its first member column; a
/// replacement always has a larger one.
#[derive(Debug, Clone)]
pub struct RecordSlot {
    pub schema: &'static RecordSchema,
    /// The pool column of schema field `k`, `None` when it was not supplied.
    pub fields: [Option<ColumnId>; MAX_RECORD_FIELDS],
    pub len: u32,
    pub epoch: u64,
    pub stats: RecordStats,
}

/// A resident ragged column: its two member columns.
#[derive(Debug, Clone)]
pub struct RaggedSlot {
    pub values: ColumnId,
    pub offsets: ColumnId,
    pub groups: u32,
    pub total: u32,
    pub epoch: u64,
    pub stats: RaggedStats,
}

/// A record looked up in the pool ([`ColumnPool::record`]).
#[derive(Clone, Copy)]
pub struct RecordRef<'a> {
    pool: &'a ColumnPool,
    slot: &'a RecordSlot,
}

impl<'a> RecordRef<'a> {
    pub fn slot(&self) -> &'a RecordSlot {
        self.slot
    }

    pub fn stats(&self) -> RecordStats {
        self.slot.stats
    }

    /// The handle of schema field `index`, `None` when it was not supplied.
    pub fn field_at(&self, index: usize) -> Option<ColumnHandle> {
        let id = self.slot.fields.get(index)?.as_deref()?;
        self.pool.handle_for(id)
    }

    /// The handle of the field named `name`, `None` when the schema has no
    /// such field or it was not supplied.
    pub fn field(&self, name: &str) -> Option<ColumnHandle> {
        let index = self
            .slot
            .schema
            .iter()
            .position(|(field, _)| *field == name)?;
        self.field_at(index)
    }
}

/// A ragged column looked up in the pool ([`ColumnPool::ragged`]).
#[derive(Clone, Copy)]
pub struct RaggedRef<'a> {
    pool: &'a ColumnPool,
    slot: &'a RaggedSlot,
}

impl<'a> RaggedRef<'a> {
    pub fn slot(&self) -> &'a RaggedSlot {
        self.slot
    }

    pub fn stats(&self) -> RaggedStats {
        self.slot.stats
    }

    pub fn values(&self) -> ColumnHandle {
        self.pool
            .handle_for(&self.slot.values)
            .expect("a ragged column's values are resident")
    }

    pub fn offsets(&self) -> ColumnHandle {
        self.pool
            .handle_for(&self.slot.offsets)
            .expect("a ragged column's offsets are resident")
    }
}

/// Lightweight handle handed out to the chart layer. `generation` lets
/// callers detect a stale handle after any invalidating pool mutation.
#[derive(Debug, Clone, Copy)]
pub struct ColumnHandle {
    pub generation: u32,
    pub offset: u64,
    pub byte_size: u64,
    pub len_values: usize,
}

impl ColumnHandle {
    /// Pass directly into `pool.buffer().slice(byte_range)`.
    pub fn byte_range(&self) -> std::ops::Range<u64> {
        self.offset..(self.offset + self.byte_size)
    }
}

/// Allocation failure modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllocError {
    /// No free region is large enough (try defrag then retry).
    OutOfSpace {
        requested: u64,
        largest_free: u64,
        total_free: u64,
    },
    /// Requested GPU buffer is larger than the device can allocate.
    ResourceLimit {
        resource: &'static str,
        requested: u64,
        limit: u64,
    },
    /// A requested allocation would exceed the host-declared renderer budget.
    BudgetExceeded {
        resource: &'static str,
        requested: u64,
        limit: u64,
    },
    /// Resource creation failed despite satisfying static device limits.
    AllocationFailed {
        resource: &'static str,
        reason: String,
    },
    /// A column with this id already exists.
    DuplicateId(ColumnId),
    /// Source has length zero.
    EmptySource,
    /// A monotonic identity counter cannot advance without wrapping.
    CounterExhausted { counter: &'static str },
}

impl std::fmt::Display for AllocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllocError::OutOfSpace {
                requested,
                largest_free,
                total_free,
            } => write!(
                f,
                "ColumnPool out of space: need {} bytes, largest free = {}, total free = {}",
                requested, largest_free, total_free
            ),
            AllocError::ResourceLimit {
                resource,
                requested,
                limit,
            } => write!(
                f,
                "{resource} exceeds GPU buffer limit: requested {requested}, limit {limit}"
            ),
            AllocError::BudgetExceeded {
                resource,
                requested,
                limit,
            } => write!(
                f,
                "{resource} exceeds renderer memory budget: requested {requested}, limit {limit}"
            ),
            AllocError::AllocationFailed { resource, reason } => {
                write!(f, "{resource} allocation failed: {reason}")
            }
            AllocError::DuplicateId(id) => write!(f, "ColumnPool duplicate id: {id}"),
            AllocError::EmptySource => write!(f, "ColumnPool: empty source not allowed"),
            AllocError::CounterExhausted { counter } => {
                write!(f, "ColumnPool {counter} exhausted")
            }
        }
    }
}

impl std::error::Error for AllocError {}

/// GPU column slab + CPU-side offset table.
pub struct ColumnPool {
    identity: PoolIdentity,
    primary: Buffer,
    capacity: u64,
    slots: HashMap<ColumnId, ColumnSlot>,
    /// Resident records; their member columns are in `slots`.
    records: HashMap<RecordId, RecordSlot>,
    /// Resident ragged columns; their member columns are in `slots`.
    raggeds: HashMap<RaggedId, RaggedSlot>,
    /// Kept sorted by offset (coalesce and first-fit both rely on this).
    free: Vec<FreeRegion>,
    generation: u32,
    allocation_epochs: HashMap<ColumnId, u64>,
    allocation_epoch_counter: u64,
    layout_generation: u64,
    /// Ping-pong target for defrag. Lazily created on the first defrag,
    /// then alternates with `primary`.
    backup: Option<Buffer>,
    /// Whether to auto-defrag on alloc failure. Default `Manual`.
    pub defrag_policy: DefragPolicy,
    /// Whether an upload that does not fit may enlarge the pool.
    /// Default `Fixed` — released hosts read `OutOfSpace` as final.
    pub growth_policy: GrowthPolicy,
    /// Bytes whose buffer handle this pool has already dropped but whose
    /// device memory the queue may not have released yet. A budget must keep
    /// counting them until the corresponding queue completion.
    retired_bytes: Arc<RetiredBytes>,
    /// High-water mark of [`Self::gpu_bytes`] plus whatever transient buffer
    /// coexisted with it (upload staging, candidate primary, the new slab
    /// during a growth copy). Live bytes are derived from the buffers this
    /// pool holds and so cannot drift, but a derivation can never see a
    /// transient that lives and dies inside one call — and that transient is
    /// exactly what an upload or growth peak is made of.
    peak_bytes: u64,
    /// Device buffers created since construction. Counts objects, not bytes,
    /// so a scenario can assert how many allocations it caused.
    buffer_creations: u64,
}

impl ColumnPool {
    /// The stacked extent of the bar group whose value columns are `ids`:
    /// `Some` exactly when every one of them carries the [`StackExtent`] of
    /// one upload batch that had exactly these columns, `None` when one of
    /// them was uploaded on its own, replaced, or in another batch.
    pub fn stack_extent<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Option<StackExtent> {
        let mut found: Option<StackExtent> = None;
        let mut count = 0usize;
        for id in ids {
            let extent = self.slots.get(id)?.stack_extent?;
            if found.is_some_and(|f| f != extent) {
                return None;
            }
            found = Some(extent);
            count += 1;
        }
        found.filter(|extent| extent.members == count)
    }

    /// New pool. `capacity_bytes` is rounded up to a multiple of `ALIGN`.
    pub fn new(ctx: GpuAllocCtx<'_>, capacity_bytes: u64) -> Result<Self, AllocError> {
        let device = ctx.device;
        let max_buffer_size = buffer_ceiling(device);
        let requested = capacity_bytes.max(ALIGN);
        let capacity = try_align_up(requested, ALIGN).ok_or(AllocError::ResourceLimit {
            resource: "column pool buffer",
            requested,
            limit: max_buffer_size,
        })?;
        if capacity > max_buffer_size {
            return Err(AllocError::ResourceLimit {
                resource: "column pool buffer",
                requested: capacity,
                limit: max_buffer_size,
            });
        }
        let primary_desc = BufferDescriptor {
            label: Some("figgy column pool primary"),
            size: capacity,
            // VERTEX | STORAGE so the pool can serve both binding kinds.
            // COPY_DST for staging→primary uploads, COPY_SRC for the
            // primary→backup defrag copy.
            usage: BufferUsages::VERTEX
                | BufferUsages::STORAGE
                | BufferUsages::COPY_DST
                | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        };
        let primary = create_buffer_checked(device, &primary_desc, "column pool buffer")?;
        Ok(Self {
            identity: PoolIdentity::new(),
            primary,
            capacity,
            slots: HashMap::new(),
            records: HashMap::new(),
            raggeds: HashMap::new(),
            free: vec![FreeRegion {
                offset: 0,
                size: capacity,
            }],
            generation: 0,
            allocation_epochs: HashMap::new(),
            allocation_epoch_counter: 0,
            layout_generation: 0,
            backup: None,
            defrag_policy: DefragPolicy::Manual,
            growth_policy: GrowthPolicy::Fixed,
            retired_bytes: Arc::default(),
            peak_bytes: capacity,
            buffer_creations: 1,
        })
    }

    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    pub fn buffer(&self) -> &Buffer {
        &self.primary
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub(crate) fn layout_generation(&self) -> u64 {
        self.layout_generation
    }

    pub(crate) fn allocation_epoch(&self, id: &str) -> Option<u64> {
        self.allocation_epochs.get(id).copied()
    }

    fn checked_generation_successor(&self) -> Result<u32, AllocError> {
        self.generation
            .checked_add(1)
            .ok_or(AllocError::CounterExhausted {
                counter: "public generation",
            })
    }

    fn checked_layout_successor(&self) -> Result<u64, AllocError> {
        self.layout_generation
            .checked_add(1)
            .ok_or(AllocError::CounterExhausted {
                counter: "layout generation",
            })
    }

    fn checked_allocation_epoch_successor(&self) -> Result<u64, AllocError> {
        self.allocation_epoch_counter
            .checked_add(1)
            .ok_or(AllocError::CounterExhausted {
                counter: "allocation epoch",
            })
    }

    pub fn slot(&self, id: &str) -> Option<&ColumnSlot> {
        self.slots.get(id)
    }

    /// Drop all columns, records and ragged columns. The primary buffer is reused (capacity unchanged).
    /// Bumps public and layout generations; allocation epochs are removed
    /// without resetting their monotonic issuer.
    pub fn clear(&mut self) -> Result<(), AllocError> {
        let next_generation = self.checked_generation_successor()?;
        let next_layout_generation = self.checked_layout_successor()?;
        self.slots.clear();
        self.records.clear();
        self.raggeds.clear();
        self.allocation_epochs.clear();
        self.free.clear();
        self.free.push(FreeRegion {
            offset: 0,
            size: self.capacity,
        });
        self.generation = next_generation;
        self.layout_generation = next_layout_generation;
        Ok(())
    }

    pub fn handle_for(&self, id: &str) -> Option<ColumnHandle> {
        self.slots.get(id).map(|s| ColumnHandle {
            generation: s.generation,
            offset: s.offset,
            byte_size: s.byte_size,
            len_values: s.len_values,
        })
    }

    /// True if the handle is still valid for the current public pool state.
    /// Removal, replacement, relocation, or clear invalidates previously
    /// issued handles.
    pub fn is_valid_handle(&self, h: &ColumnHandle) -> bool {
        h.generation == self.generation
    }

    /// Add a column to the pool with a zero-copy stream upload.
    ///
    /// When `defrag_policy == OnAllocFailure` and the first attempt returns
    /// `OutOfSpace`, this calls `defragment()` once and retries; the original
    /// `OutOfSpace` is returned if the retry still fails.
    pub fn add_column(
        &mut self,
        id: ColumnId,
        source: &dyn ColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnHandle, AllocError> {
        // Only clone `id` for retry under OnAllocFailure; the default path allocates nothing extra.
        // Clone `id` only for the retry rungs; with both policies off the
        // default path allocates nothing extra.
        let defrag_retry = self.defrag_policy == DefragPolicy::OnAllocFailure;
        let grow_retry = self.growth_policy == GrowthPolicy::OnAllocFailure;
        let retry_id = (defrag_retry || grow_retry).then(|| id.clone());
        let first = self.try_add_column(id, source, ctx);
        let Err(e @ AllocError::OutOfSpace { .. }) = first else {
            return first;
        };
        let Some(rid) = retry_id else {
            return Err(e);
        };
        // Rung 1: defrag and retry.
        if defrag_retry {
            self.defragment(ctx)?;
            match self.try_add_column(rid.clone(), source, ctx) {
                Err(AllocError::OutOfSpace { .. }) if grow_retry => {}
                other => return other,
            }
        }
        if !grow_retry {
            return Err(e);
        }
        // Rung 2: enlarge and retry. A growth that cannot happen — ceiling,
        // budget, or a failed allocation — leaves the original OutOfSpace as
        // the answer, so what callers read does not change.
        let needed = try_align_up(
            (source.len() as u64).saturating_mul(COLUMN_VALUE_BYTES as u64),
            ALIGN,
        )
        .unwrap_or(u64::MAX);
        if self.grow_for_pending_upload(ctx, needed).is_err() {
            return Err(e);
        }
        self.try_add_column(rid, source, ctx)
    }

    /// Add many columns in one transaction: **one** contiguous pool region,
    /// **one** staging buffer, **one** copy, **one** submit.
    ///
    /// This is the upload path a matrix declaration needs. Calling
    /// [`Self::add_column`] per column costs a first-fit search, a staging
    /// buffer, a command encoder and a `queue.submit` *each*; at the thousands
    /// of columns a grid has, that is the whole cost of the upload. Nothing
    /// here is matrix-specific, so any host with many columns gets the same
    /// path (design §B.2, where it is named `add_matrix_columns`).
    ///
    /// What it does **not** do, unlike the four-column demo batch it replaces:
    /// it does not duplicate the whole pool buffer, it does not stage per
    /// column, and it copies no allocator plan. The region is taken as a single
    /// reservation, so a failure rolls back by returning that one region; slots
    /// are published only once every fallible step is behind us, which is why
    /// the maps are `try_reserve`d up front rather than cloned.
    ///
    /// Every id must be new and distinct within the batch — `DuplicateId`
    /// otherwise, with nothing uploaded. An empty slice is a no-op.
    pub fn add_columns(
        &mut self,
        columns: &[(&str, &dyn ColumnSource)],
        ctx: GpuAllocCtx<'_>,
    ) -> Result<(), AllocError> {
        self.begin_add_columns(columns, ctx)?.commit();
        Ok(())
    }

    pub fn add_hilo_column(
        &mut self,
        id: ColumnId,
        source: &dyn HiLoColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnHandle, AllocError> {
        // Clone `id` only for the retry rungs; with both policies off the
        // default path allocates nothing extra.
        let defrag_retry = self.defrag_policy == DefragPolicy::OnAllocFailure;
        let grow_retry = self.growth_policy == GrowthPolicy::OnAllocFailure;
        let retry_id = (defrag_retry || grow_retry).then(|| id.clone());
        let first = self.try_add_hilo_column(id, source, ctx);
        let Err(e @ AllocError::OutOfSpace { .. }) = first else {
            return first;
        };
        let Some(rid) = retry_id else {
            return Err(e);
        };
        // Rung 1: defrag and retry.
        if defrag_retry {
            self.defragment(ctx)?;
            match self.try_add_hilo_column(rid.clone(), source, ctx) {
                Err(AllocError::OutOfSpace { .. }) if grow_retry => {}
                other => return other,
            }
        }
        if !grow_retry {
            return Err(e);
        }
        // Rung 2: enlarge and retry. A growth that cannot happen — ceiling,
        // budget, or a failed allocation — leaves the original OutOfSpace as
        // the answer, so what callers read does not change.
        let needed = try_align_up(
            (source.len() as u64).saturating_mul(COLUMN_VALUE_BYTES as u64),
            ALIGN,
        )
        .unwrap_or(u64::MAX);
        if self.grow_for_pending_upload(ctx, needed).is_err() {
            return Err(e);
        }
        self.try_add_hilo_column(rid, source, ctx)
    }

    /// Failure-atomic scalar upsert when no dependent batch preparation is
    /// needed between prepare and commit.
    pub fn upsert_column(
        &mut self,
        id: ColumnId,
        source: &dyn ColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnHandle, AllocError> {
        Ok(self.begin_upsert_column(id, source, ctx)?.commit())
    }

    /// Failure-atomic hi/lo upsert when no dependent batch preparation is
    /// needed between prepare and commit.
    pub fn upsert_hilo_column(
        &mut self,
        id: ColumnId,
        source: &dyn HiLoColumnSource,
        ctx: GpuAllocCtx<'_>,
    ) -> Result<ColumnHandle, AllocError> {
        Ok(self.begin_upsert_hilo_column(id, source, ctx)?.commit())
    }

    /// Remove a column. Returns its region to the free list and coalesces
    /// with neighbors. Public handles are invalidated, the removed allocation
    /// epoch is discarded, and the layout generation remains unchanged.
    pub fn remove_column(&mut self, id: &str) -> Result<bool, AllocError> {
        let Some(removal) = self.begin_remove_column(id)? else {
            return Ok(false);
        };
        Ok(removal.commit())
    }

    /// Relayout into a larger buffer and publish immediately.
    pub fn grow_to(
        &mut self,
        ctx: GpuAllocCtx<'_>,
        target_capacity: u64,
    ) -> Result<bool, AllocError> {
        if target_capacity < self.capacity {
            return Err(AllocError::AllocationFailed {
                resource: "column pool growth",
                reason: "shrinking through grow_to is not supported".into(),
            });
        }
        Ok(self.begin_relayout(ctx, target_capacity)?.commit())
    }

    /// Pack every live column tightly and publish immediately.
    pub fn defragment(&mut self, ctx: GpuAllocCtx<'_>) -> Result<bool, AllocError> {
        Ok(self.begin_defragment(ctx)?.commit())
    }
}

#[cfg(test)]
mod tests;
