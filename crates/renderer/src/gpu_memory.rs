//! GPU memory accounting — the renderer's single funnel.
//!
//! Every GPU byte figgy holds is either inside the column pool (the data
//! itself) or outside it (panel textures, MSAA targets, export targets and
//! readbacks, arc-scan scratch, pick scratch, uniforms, LUTs). The pool can
//! always report its own bytes from its capacity, so this module accounts for
//! *everything else* and the renderer grafts the pool row in when it reports.
//!
//! Why a ledger instead of walking the live objects: buffers deliberately
//! disappear into bind groups (`line_arc::ArcScratch` names only the two
//! buffers `dispatch` writes — wgpu keeps the rest alive through the bind
//! group), so a walk over named fields would under-report exactly where the
//! per-series scratch lives. Charging at the allocation and crediting at the
//! owner's `Drop` sees those bytes; nothing has to expose a handle it does not
//! otherwise need.
//!
//! Two rules make the number safe to spend against a budget:
//!
//! 1. **Released is not free yet.** A dropped handle moves its bytes from
//!    `live` to `retired` rather than out of the report, because wgpu defers
//!    the device-side release until the queue drains. `end_submission` clears
//!    `retired` until the host reports the corresponding queue submission.
//! 2. **The total is never cached.** [`GpuLedger::snapshot`] is cheap
//!    (a handful of relaxed loads) and callers read it immediately before an
//!    allocation decision, so a budget check never runs against a stale sum.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Number of rows in a [`GpuMemoryUsage`] report.
pub const GPU_RESOURCE_KIND_COUNT: usize = 12;

/// The kind every GPU allocation is charged to.
///
/// One row per kind in the report, so a memory regression names the
/// subsystem that caused it instead of a single opaque total.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum GpuResourceKind {
    /// The column pool's `primary` (+ `backup` while one exists). Reported by
    /// the pool itself, not charged through the ledger.
    ColumnPool,
    /// Per-panel grid and decoration textures.
    PanelTexture,
    /// Multisampled render attachment for the window or an export.
    MsaaTarget,
    /// Offscreen colour target for `render_to_*`.
    ExportTarget,
    /// `MAP_READ` buffers for export readback, pick results, and GPU extents.
    Readback,
    /// Per-series arc-scan prefix, carry, chunk sums, and star-pass state.
    ArcScan,
    /// GPU errorbar/extent reduction scratch.
    ErrorbarScratch,
    /// GPU pick candidate, gate-mask, and style buffers.
    PickScratch,
    /// Transform / style / params uniforms and small style-row storage.
    Uniform,
    /// Baked lookup textures (PSF, blackbody, colormap).
    Lut,
    /// A field's grid `(base, len)` table, contour levels, colourmap stops,
    /// per-level colours and per-block contour lookup metadata.
    /// Proportional to the grid's **column count**, so it is a real term in the
    /// scaling curve rather than a constant — which is why it has its own row
    /// instead of sharing `Uniform`.
    FieldTable,
    /// Contour label candidates, selected anchors, indirect args, params and
    /// the CPU-baked label atlas.
    ContourScratch,
}

impl GpuResourceKind {
    /// Every kind, in report order.
    pub const ALL: [GpuResourceKind; GPU_RESOURCE_KIND_COUNT] = [
        GpuResourceKind::ColumnPool,
        GpuResourceKind::PanelTexture,
        GpuResourceKind::MsaaTarget,
        GpuResourceKind::ExportTarget,
        GpuResourceKind::Readback,
        GpuResourceKind::ArcScan,
        GpuResourceKind::ErrorbarScratch,
        GpuResourceKind::PickScratch,
        GpuResourceKind::Uniform,
        GpuResourceKind::Lut,
        GpuResourceKind::FieldTable,
        GpuResourceKind::ContourScratch,
    ];

    /// Row index in a [`GpuMemoryUsage`] report.
    pub const fn index(self) -> usize {
        match self {
            GpuResourceKind::ColumnPool => 0,
            GpuResourceKind::PanelTexture => 1,
            GpuResourceKind::MsaaTarget => 2,
            GpuResourceKind::ExportTarget => 3,
            GpuResourceKind::Readback => 4,
            GpuResourceKind::ArcScan => 5,
            GpuResourceKind::ErrorbarScratch => 6,
            GpuResourceKind::PickScratch => 7,
            GpuResourceKind::Uniform => 8,
            GpuResourceKind::Lut => 9,
            GpuResourceKind::FieldTable => 10,
            GpuResourceKind::ContourScratch => 11,
        }
    }

    /// Stable name for reports and test failure messages.
    pub const fn label(self) -> &'static str {
        match self {
            GpuResourceKind::ColumnPool => "column pool",
            GpuResourceKind::PanelTexture => "panel texture",
            GpuResourceKind::MsaaTarget => "msaa target",
            GpuResourceKind::ExportTarget => "export target",
            GpuResourceKind::Readback => "readback",
            GpuResourceKind::ArcScan => "arc scan",
            GpuResourceKind::ErrorbarScratch => "errorbar scratch",
            GpuResourceKind::PickScratch => "pick scratch",
            GpuResourceKind::Uniform => "uniform",
            GpuResourceKind::Lut => "lut",
            GpuResourceKind::FieldTable => "field table",
            GpuResourceKind::ContourScratch => "contour scratch",
        }
    }
}

/// Exact byte cost of a live texture, samples and mip levels included.
pub fn texture_bytes(texture: &wgpu::Texture) -> u64 {
    texture_extent_bytes(
        texture.width(),
        texture.height(),
        texture.depth_or_array_layers(),
        texture.format(),
        texture.sample_count(),
        texture.mip_level_count(),
    )
}

/// Exact byte cost of a texture described but not yet created.
///
/// Used where the charge has to be computed before the resource exists (a
/// pre-allocation budget check) and where the descriptor is the only thing at
/// hand.
pub fn texture_desc_bytes(desc: &wgpu::TextureDescriptor<'_>) -> u64 {
    texture_extent_bytes(
        desc.size.width,
        desc.size.height,
        desc.size.depth_or_array_layers,
        desc.format,
        desc.sample_count,
        desc.mip_level_count,
    )
}

fn texture_extent_bytes(
    width: u32,
    height: u32,
    layers: u32,
    format: wgpu::TextureFormat,
    sample_count: u32,
    mip_level_count: u32,
) -> u64 {
    // `block_copy_size(None)` is `None` only for depth/stencil combinations
    // figgy never allocates; charge those 4 bytes per texel rather than
    // silently reporting zero.
    let block_bytes = u64::from(format.block_copy_size(None).unwrap_or(4));
    let (block_w, block_h) = format.block_dimensions();
    let samples = u64::from(sample_count.max(1));
    let layers = u64::from(layers.max(1));
    let mut total = 0u64;
    for level in 0..mip_level_count.max(1) {
        let w = (width >> level).max(1);
        let h = (height >> level).max(1);
        let blocks_x = u64::from(w.div_ceil(block_w.max(1)));
        let blocks_y = u64::from(h.div_ceil(block_h.max(1)));
        total = total.saturating_add(
            blocks_x
                .saturating_mul(blocks_y)
                .saturating_mul(layers)
                .saturating_mul(block_bytes)
                .saturating_mul(samples),
        );
    }
    total
}

/// Running GPU byte counts, shared by every renderer subsystem.
///
/// Interior mutability is by atomics, not a lock: the draw phase holds `&self`
/// and must be able to charge an allocation without a mutable borrow, and the
/// renderer takes no new locks (see the design's invariant list).
#[derive(Debug)]
pub struct GpuLedger {
    live: [AtomicU64; GPU_RESOURCE_KIND_COUNT],
    retired: [AtomicU64; GPU_RESOURCE_KIND_COUNT],
    creations: [AtomicU64; GPU_RESOURCE_KIND_COUNT],
    peak_bytes: AtomicU64,
}

impl Default for GpuLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuLedger {
    pub fn new() -> Self {
        Self {
            live: std::array::from_fn(|_| AtomicU64::new(0)),
            retired: std::array::from_fn(|_| AtomicU64::new(0)),
            creations: std::array::from_fn(|_| AtomicU64::new(0)),
            peak_bytes: AtomicU64::new(0),
        }
    }

    /// Charge `bytes` to `kind` and count one creation.
    ///
    /// Called at the allocation, so the creation count is exactly "how many
    /// GPU objects this scenario made" — the number the allocation gate
    /// asserts against.
    pub fn record_alloc(&self, kind: GpuResourceKind, bytes: u64) {
        let i = kind.index();
        self.live[i].fetch_add(bytes, Ordering::Relaxed);
        self.creations[i].fetch_add(1, Ordering::Relaxed);
        self.bump_peak();
    }

    /// Move `bytes` from live to retired: the handle is gone, the device may
    /// still hold the memory until the queue drains.
    pub fn record_retire(&self, kind: GpuResourceKind, bytes: u64) {
        let i = kind.index();
        // Saturating so a mis-paired credit reports zero rather than wrapping
        // to a nonsense total that would refuse every later allocation.
        let _ = self.live[i].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |live| {
            Some(live.saturating_sub(bytes))
        });
        self.retired[i].fetch_add(bytes, Ordering::Relaxed);
    }

    /// Submission boundary: the host has handed every recorded reference to the
    /// queue, so opaque command-buffer ownership no longer needs this retired
    /// accounting backstop.
    pub fn end_submission(&self) {
        for slot in &self.retired {
            slot.store(0, Ordering::Relaxed);
        }
    }

    /// Live + retired across every kind — the number a budget is spent against.
    pub fn total_bytes(&self) -> u64 {
        let mut total = 0u64;
        for i in 0..GPU_RESOURCE_KIND_COUNT {
            total = total
                .saturating_add(self.live[i].load(Ordering::Relaxed))
                .saturating_add(self.retired[i].load(Ordering::Relaxed));
        }
        total
    }

    /// Immutable report of every row.
    pub fn snapshot(&self) -> GpuMemoryUsage {
        GpuMemoryUsage {
            live: std::array::from_fn(|i| self.live[i].load(Ordering::Relaxed)),
            retired: std::array::from_fn(|i| self.retired[i].load(Ordering::Relaxed)),
            creations: std::array::from_fn(|i| self.creations[i].load(Ordering::Relaxed)),
            peak_bytes: self.peak_bytes.load(Ordering::Relaxed),
        }
    }

    /// Highest `total_bytes` seen since construction.
    ///
    /// Sampled at each charge, so it catches a transient that no external
    /// observer could have called `snapshot` in time to see — the staging
    /// buffer alive beside a fresh column, or the old and new pool buffers
    /// coexisting across a growth copy.
    pub fn peak_bytes(&self) -> u64 {
        self.peak_bytes.load(Ordering::Relaxed)
    }

    /// Forget the recorded peak; the next charge starts a fresh high-water mark.
    pub fn reset_peak(&self) {
        self.peak_bytes.store(self.total_bytes(), Ordering::Relaxed);
    }

    fn bump_peak(&self) {
        let total = self.total_bytes();
        let _ = self
            .peak_bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |peak| {
                (total > peak).then_some(total)
            });
    }
}

/// A snapshot of the ledger, one row per [`GpuResourceKind`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuMemoryUsage {
    live: [u64; GPU_RESOURCE_KIND_COUNT],
    retired: [u64; GPU_RESOURCE_KIND_COUNT],
    creations: [u64; GPU_RESOURCE_KIND_COUNT],
    peak_bytes: u64,
}

impl GpuMemoryUsage {
    /// Overwrite one row.
    ///
    /// The renderer grafts the [`GpuResourceKind::ColumnPool`] row in from the
    /// pool's own capacity: the pool is the authority on its bytes, and
    /// charging them twice (once at its `create_buffer`, once from capacity)
    /// would double-count exactly the term the memory-scaling curve measures.
    pub fn with_kind(mut self, kind: GpuResourceKind, live: u64, retired: u64) -> Self {
        let i = kind.index();
        self.live[i] = live;
        self.retired[i] = retired;
        self
    }

    /// Add `count` creations to one row (the pool reports its own).
    pub fn with_creations(mut self, kind: GpuResourceKind, count: u64) -> Self {
        self.creations[kind.index()] = count;
        self
    }

    /// Raise the reported peak to at least `bytes`.
    pub fn with_peak_at_least(mut self, bytes: u64) -> Self {
        self.peak_bytes = self.peak_bytes.max(bytes);
        self
    }

    pub fn live_bytes(&self) -> u64 {
        self.live.iter().fold(0u64, |a, b| a.saturating_add(*b))
    }

    pub fn retired_bytes(&self) -> u64 {
        self.retired.iter().fold(0u64, |a, b| a.saturating_add(*b))
    }

    /// Live + retired: what the device may still be holding.
    pub fn total_bytes(&self) -> u64 {
        self.live_bytes().saturating_add(self.retired_bytes())
    }

    /// The pool row — the data itself.
    pub fn pool_bytes(&self) -> u64 {
        self.bytes_of(GpuResourceKind::ColumnPool)
    }

    /// Everything that is not the pool.
    ///
    /// This is the value that rides into the pool as
    /// `GpuAllocCtx::external_bytes`: the pool adds its own current capacity
    /// and the capacity it is about to allocate, so counting pool bytes here
    /// too would charge them twice.
    pub fn external_bytes(&self) -> u64 {
        self.total_bytes().saturating_sub(self.pool_bytes())
    }

    /// Live + retired for one kind.
    pub fn bytes_of(&self, kind: GpuResourceKind) -> u64 {
        let i = kind.index();
        self.live[i].saturating_add(self.retired[i])
    }

    pub fn live_bytes_of(&self, kind: GpuResourceKind) -> u64 {
        self.live[kind.index()]
    }

    pub fn retired_bytes_of(&self, kind: GpuResourceKind) -> u64 {
        self.retired[kind.index()]
    }

    /// How many GPU objects of this kind have been created since startup.
    pub fn creations_of(&self, kind: GpuResourceKind) -> u64 {
        self.creations[kind.index()]
    }

    pub fn total_creations(&self) -> u64 {
        self.creations
            .iter()
            .fold(0u64, |a, b| a.saturating_add(*b))
    }

    /// Highest total seen, including transients no observer could sample.
    pub fn peak_bytes(&self) -> u64 {
        self.peak_bytes
    }

    /// One line per non-empty row — for test failures and diagnostics.
    pub fn report(&self) -> String {
        // host-alloc: W1-c
        let mut out = String::new();
        for kind in GpuResourceKind::ALL {
            let live = self.live_bytes_of(kind);
            let retired = self.retired_bytes_of(kind);
            let creations = self.creations_of(kind);
            if live == 0 && retired == 0 && creations == 0 {
                continue;
            }
            // host-alloc: W1-c
            out.push_str(&format!(
                "{:<18} live {:>12} retired {:>12} creations {:>6}\n",
                kind.label(),
                live,
                retired,
                creations
            ));
        }
        // host-alloc: W1-c
        out.push_str(&format!(
            "{:<18} live {:>12} retired {:>12} peak {:>12}\n",
            "TOTAL",
            self.live_bytes(),
            self.retired_bytes(),
            self.peak_bytes
        ));
        out
    }
}

/// A buffer charged to a ledger for as long as this owner or a prepared clone
/// of its accounting lifetime keeps the corresponding GPU handles alive.
///
/// `Deref` keeps every existing use (`&buffer`, `buffer.as_entire_binding()`,
/// `buffer.slice(..)`) spelled the same, so wrapping an allocation is a
/// one-line change at the creation site and nothing downstream moves.
#[derive(Debug)]
pub struct TrackedBuffer {
    buffer: wgpu::Buffer,
    charge: SharedCharge,
}

impl TrackedBuffer {
    /// Charge `buffer`'s size to `kind` and hold the charge.
    pub fn new(ledger: &Arc<GpuLedger>, kind: GpuResourceKind, buffer: wgpu::Buffer) -> Self {
        let bytes = buffer.size();
        Self {
            buffer,
            charge: shared_byte_charge(ledger, kind, bytes),
        }
    }

    /// The charged size, as accounted (not re-read from the buffer).
    pub fn charged_bytes(&self) -> u64 {
        self.charge.charged_bytes()
    }

    pub fn kind(&self) -> GpuResourceKind {
        self.charge.kind()
    }

    /// Clone the accounting lifetime together with another GPU handle that
    /// keeps this buffer alive (for example a prepared bind group).
    pub fn shared_charge(&self) -> SharedCharge {
        Arc::clone(&self.charge)
    }
}

impl std::ops::Deref for TrackedBuffer {
    type Target = wgpu::Buffer;
    fn deref(&self) -> &wgpu::Buffer {
        &self.buffer
    }
}

/// A texture charged to a ledger for as long as this owner or a prepared clone
/// of its accounting lifetime keeps the corresponding GPU handles alive.
#[derive(Debug)]
pub struct TrackedTexture {
    texture: wgpu::Texture,
    charge: SharedCharge,
}

impl TrackedTexture {
    pub fn new(ledger: &Arc<GpuLedger>, kind: GpuResourceKind, texture: wgpu::Texture) -> Self {
        let bytes = texture_bytes(&texture);
        Self {
            texture,
            charge: shared_byte_charge(ledger, kind, bytes),
        }
    }

    pub fn charged_bytes(&self) -> u64 {
        self.charge.charged_bytes()
    }

    pub fn kind(&self) -> GpuResourceKind {
        self.charge.kind()
    }

    /// Clone the accounting lifetime together with another GPU handle that
    /// keeps this texture alive (for example a prepared bind group).
    pub fn shared_charge(&self) -> SharedCharge {
        Arc::clone(&self.charge)
    }
}

impl std::ops::Deref for TrackedTexture {
    type Target = wgpu::Texture;
    fn deref(&self) -> &wgpu::Texture {
        &self.texture
    }
}

/// Running total for a lump charge, held while an owner builds its buffers.
///
/// The reason this type exists rather than a plain `u64`: buffers are created
/// through [`charged_buffer`] / [`charged_buffer_init`], which take a tally, so
/// there is **no path that allocates and forgets to charge**. Before this, each
/// owner summed its buffer sizes by hand — `line_arc` next to each creation,
/// `gpu_pick` eighty lines away from them — and a new buffer only had to miss
/// that sum to make the ledger under-report silently. The sizes now come from
/// the descriptor the device is handed, so they cannot disagree with it either.
///
/// `Cell` rather than `&mut` because the creation helpers are called from `Fn`
/// closures that also capture the device.
#[derive(Debug, Default)]
pub struct ChargeTally {
    bytes: std::cell::Cell<u64>,
}

impl ChargeTally {
    pub fn new() -> Self {
        Self {
            bytes: std::cell::Cell::new(0),
        }
    }

    /// Bytes tallied so far.
    pub fn bytes(&self) -> u64 {
        self.bytes.get()
    }

    /// Add bytes the device was asked for outside of the helpers below.
    ///
    /// Only for resources the helpers cannot create — a texture, or a buffer
    /// built by a wrapper this module does not own. Prefer the helpers: they
    /// take the size from the descriptor, this takes your word for it.
    pub fn add(&self, bytes: u64) {
        self.bytes.set(self.bytes.get().saturating_add(bytes));
    }

    /// Hand the tallied bytes to the ledger as one charge.
    ///
    /// The returned charge is the owner's to hold; dropping it credits every
    /// byte tallied here back at once, which is the right lifetime for buffers
    /// that live and die with their owner's bind groups.
    pub fn into_charge(self, ledger: &Arc<GpuLedger>, kind: GpuResourceKind) -> GpuByteCharge {
        GpuByteCharge::new(ledger, kind, self.bytes.get())
    }
}

/// Create a buffer and tally exactly the size the device was asked for.
pub fn charged_buffer(
    tally: &ChargeTally,
    device: &wgpu::Device,
    desc: &wgpu::BufferDescriptor<'_>,
) -> wgpu::Buffer {
    tally.add(desc.size);
    // gpu-alloc: caller
    device.create_buffer(desc)
}

/// Create an initialized buffer and tally the size wgpu will actually allocate.
///
/// `create_buffer_init` rounds the contents up to `COPY_BUFFER_ALIGNMENT` (and
/// to at least that much for non-empty contents), so charging `contents.len()`
/// would under-report by up to three bytes per buffer. Empty contents allocate
/// a zero-sized buffer.
pub fn charged_buffer_init(
    tally: &ChargeTally,
    device: &wgpu::Device,
    desc: &wgpu::util::BufferInitDescriptor<'_>,
) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    tally.add(buffer_init_bytes(desc.contents.len() as u64));
    // gpu-alloc: caller
    device.create_buffer_init(desc)
}

/// Bytes `create_buffer_init` allocates for `contents_len` bytes of contents.
fn buffer_init_bytes(contents_len: u64) -> u64 {
    if contents_len == 0 {
        return 0;
    }
    let align_mask = wgpu::COPY_BUFFER_ALIGNMENT - 1;
    ((contents_len + align_mask) & !align_mask).max(wgpu::COPY_BUFFER_ALIGNMENT)
}

/// A lump charge shared by every clone of its owner.
///
/// Prepared views/results and picker/reducer clones can point at the *same*
/// device resources — wgpu handles are refcounted internally. An
/// inline charge would be cloned with the struct: double-charged on create,
/// double-credited on drop, and the report would drift by the number of live
/// clones. Behind a shared refcount it is credited when the last clone dies,
/// which is when the last handle to those buffers dies.
///
/// Each GPU lifetime owner creates one shared charge. Cloning the owner reuses
/// it, and an exact-key frame path does not allocate another charge.
pub type SharedCharge = Arc<GpuByteCharge>;

/// Hand a tally to the ledger as a charge every clone of the owner shares.
///
/// The single place this allocation happens, so the allowlist has one site to
/// point at instead of one per owner.
pub fn shared_charge(
    tally: ChargeTally,
    ledger: &Arc<GpuLedger>,
    kind: GpuResourceKind,
) -> SharedCharge {
    shared_byte_charge(ledger, kind, tally.bytes())
}

fn shared_byte_charge(ledger: &Arc<GpuLedger>, kind: GpuResourceKind, bytes: u64) -> SharedCharge {
    // host-alloc: W1-b
    Arc::new(GpuByteCharge::new(ledger, kind, bytes))
}

/// A charge held on behalf of bytes whose handle is not itself trackable.
///
/// `line_arc` deliberately drops the chunk-sums and params buffers into its
/// bind groups (wgpu keeps bound resources alive), so no wrapper can observe
/// their lifetime. The owner charges them as one lump at build time and holds
/// this guard; the last shared owner moves the lump to retired accounting.
#[derive(Debug)]
pub struct GpuByteCharge {
    ledger: Arc<GpuLedger>,
    kind: GpuResourceKind,
    bytes: u64,
}

impl GpuByteCharge {
    pub fn new(ledger: &Arc<GpuLedger>, kind: GpuResourceKind, bytes: u64) -> Self {
        ledger.record_alloc(kind, bytes);
        Self {
            ledger: Arc::clone(ledger),
            kind,
            bytes,
        }
    }

    pub fn charged_bytes(&self) -> u64 {
        self.bytes
    }

    pub fn kind(&self) -> GpuResourceKind {
        self.kind
    }
}

impl Drop for GpuByteCharge {
    fn drop(&mut self) {
        self.ledger.record_retire(self.kind, self.bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_indices_are_unique_and_cover_every_row() {
        let mut seen = [false; GPU_RESOURCE_KIND_COUNT];
        for kind in GpuResourceKind::ALL {
            let i = kind.index();
            assert!(!seen[i], "duplicate index for {}", kind.label());
            seen[i] = true;
        }
        assert!(seen.iter().all(|s| *s), "a row has no kind: {seen:?}");
    }

    #[test]
    fn rgba8_texture_bytes_are_width_times_height_times_four() {
        let desc = wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 800,
                height: 600,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        assert_eq!(texture_desc_bytes(&desc), 800 * 600 * 4);
    }

    #[test]
    fn multisampling_multiplies_the_charge() {
        let mut desc = wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 256,
                height: 128,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        let single = texture_desc_bytes(&desc);
        desc.sample_count = 4;
        assert_eq!(texture_desc_bytes(&desc), single * 4);
    }

    #[test]
    fn buffer_init_bytes_match_wgpu_padding() {
        // wgpu rounds contents up to COPY_BUFFER_ALIGNMENT and never allocates
        // less than that for non-empty contents; charging contents.len() would
        // under-report.
        assert_eq!(buffer_init_bytes(0), 0);
        assert_eq!(buffer_init_bytes(1), wgpu::COPY_BUFFER_ALIGNMENT);
        assert_eq!(buffer_init_bytes(4), 4);
        assert_eq!(buffer_init_bytes(5), 8);
        assert_eq!(buffer_init_bytes(96), 96);
    }

    #[test]
    fn a_tally_becomes_one_charge_credited_as_a_lump() {
        let ledger = Arc::new(GpuLedger::new());
        let tally = ChargeTally::new();
        tally.add(1024);
        tally.add(2048);
        assert_eq!(tally.bytes(), 3072);

        let charge = tally.into_charge(&ledger, GpuResourceKind::ArcScan);
        assert_eq!(charge.charged_bytes(), 3072);
        assert_eq!(
            ledger.snapshot().live_bytes_of(GpuResourceKind::ArcScan),
            3072
        );
        assert_eq!(
            ledger.snapshot().creations_of(GpuResourceKind::ArcScan),
            1,
            "a lump is one creation, whatever it covers"
        );

        drop(charge);
        assert_eq!(ledger.snapshot().live_bytes_of(GpuResourceKind::ArcScan), 0);
        assert_eq!(
            ledger.snapshot().retired_bytes_of(GpuResourceKind::ArcScan),
            3072
        );
    }

    #[test]
    fn retired_bytes_stay_in_the_total_until_the_submission_boundary() {
        let ledger = GpuLedger::new();
        ledger.record_alloc(GpuResourceKind::PanelTexture, 4096);
        assert_eq!(ledger.total_bytes(), 4096);

        ledger.record_retire(GpuResourceKind::PanelTexture, 4096);
        assert_eq!(
            ledger.total_bytes(),
            4096,
            "a dropped handle does not free device memory yet"
        );
        let usage = ledger.snapshot();
        assert_eq!(usage.live_bytes(), 0);
        assert_eq!(usage.retired_bytes(), 4096);

        ledger.end_submission();
        assert_eq!(ledger.total_bytes(), 0);
    }

    #[test]
    fn peak_records_a_transient_no_observer_could_sample() {
        let ledger = GpuLedger::new();
        ledger.record_alloc(GpuResourceKind::ColumnPool, 1000);
        // Old and new coexist, then the old one goes away and the frame ends.
        ledger.record_alloc(GpuResourceKind::ColumnPool, 2000);
        ledger.record_retire(GpuResourceKind::ColumnPool, 1000);
        ledger.end_submission();

        assert_eq!(ledger.total_bytes(), 2000, "settled");
        assert_eq!(ledger.peak_bytes(), 3000, "peak kept the coexistence");
        ledger.reset_peak();
        assert_eq!(ledger.peak_bytes(), 2000);
    }

    #[test]
    fn mispaired_credit_saturates_instead_of_wrapping() {
        let ledger = GpuLedger::new();
        ledger.record_alloc(GpuResourceKind::Uniform, 64);
        ledger.record_retire(GpuResourceKind::Uniform, 4096);
        ledger.end_submission();
        assert_eq!(
            ledger.total_bytes(),
            0,
            "an over-credit must not wrap to a huge live total"
        );
    }

    #[test]
    fn external_bytes_exclude_the_pool_row() {
        let usage = GpuMemoryUsage::default()
            .with_kind(GpuResourceKind::ColumnPool, 8 * 1024 * 1024, 0)
            .with_kind(GpuResourceKind::PanelTexture, 2 * 1024 * 1024, 0)
            .with_kind(GpuResourceKind::MsaaTarget, 1024 * 1024, 0);
        assert_eq!(usage.pool_bytes(), 8 * 1024 * 1024);
        assert_eq!(usage.external_bytes(), 3 * 1024 * 1024);
        assert_eq!(usage.total_bytes(), 11 * 1024 * 1024);
    }

    #[test]
    fn creations_count_objects_not_bytes() {
        let ledger = GpuLedger::new();
        ledger.record_alloc(GpuResourceKind::ArcScan, 128);
        ledger.record_alloc(GpuResourceKind::ArcScan, 256);
        let usage = ledger.snapshot();
        assert_eq!(usage.creations_of(GpuResourceKind::ArcScan), 2);
        assert_eq!(usage.creations_of(GpuResourceKind::PickScratch), 0);
        assert_eq!(usage.total_creations(), 2);
        assert_eq!(usage.bytes_of(GpuResourceKind::ArcScan), 384);
    }

    #[test]
    fn report_lists_only_rows_that_were_touched() {
        let ledger = GpuLedger::new();
        ledger.record_alloc(GpuResourceKind::Lut, 1024);
        let text = ledger.snapshot().report();
        assert!(text.contains("lut"), "{text}");
        assert!(!text.contains("pick scratch"), "{text}");
        assert!(text.contains("TOTAL"), "{text}");
    }
}
