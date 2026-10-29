//! The point picker's series registry: series identities, slots, the
//! registry and its prepared transitions, and the per-series variant choice.

use super::*;

/// The part of a series' exact variant its build fixes, from the built
/// slot's `flags`, dash length and base shape and whether it carries error
/// bars and a fill: the module for its primitive class, the dash walk for a
/// dashed line, the error-bar and fill paths, and the marker shape when every
/// point has the series' shape. `prepare_series_buffer` stores it on the
/// slot (`PickSeriesGpu::exact_key`); [`series_exact_variant`] adds the
/// query's bits.
pub(super) fn series_exact_key(
    flags: u32,
    line_dash_len: u32,
    base_shape_id: u32,
    errorbar: bool,
    area: bool,
    path_arrows: bool,
    category_jitter: bool,
) -> PickExactKey {
    let scatter = flags & FLAG_SCATTER != 0;
    let line = flags & FLAG_LINE != 0;
    let shader = if flags & FLAG_VECTOR != 0 {
        PickShader::Vector
    } else {
        match (scatter, line) {
            (true, true) => PickShader::MarkerLine,
            (false, true) => PickShader::Line,
            _ => PickShader::Marker,
        }
    };
    let mut bits = if line && line_dash_len > 0 {
        exact_variant::DASH
    } else {
        0
    };
    if scatter && errorbar {
        bits |= exact_variant::ERRORBAR;
    }
    if line && area {
        bits |= exact_variant::AREA;
    }
    if line && path_arrows {
        bits |= exact_variant::PATH_ARROWS;
    }
    if scatter && category_jitter {
        bits |= exact_variant::CATEGORY_JITTER;
    }
    // Without a style map every point has the series' shape.
    let shape = if scatter && flags & FLAG_STYLE_MAP == 0 {
        base_shape_id
    } else {
        PickExactKey::ANY_SHAPE
    };
    PickExactKey {
        shader,
        bits,
        shape,
    }
}

/// The exact module and variant that pick `series` as the query draws it:
/// the slot's stored key (its primitive class, the dash walk for a dashed
/// line and its shape) plus the query's bits, the styled marker and line
/// paths when the draw style uses them and the polar paths on polar axes.
pub(super) fn series_exact_variant(series: &PickSeriesGpu, query: &GpuPickQuery) -> PickExactKey {
    let mut key = series.exact_key;
    // A box plot's raw points draw precise markers under every style.
    if series.flags & FLAG_SCATTER != 0
        && key.shader != PickShader::Vector
        && key.bits & exact_variant::CATEGORY_JITTER == 0
        && query.draw_style.marker != PICK_MARKER_PRECISE
    {
        key.bits |= exact_variant::STYLED_MARKER;
    }
    if series.flags & FLAG_LINE != 0 && query.draw_style.line != PICK_LINE_PRECISE {
        key.bits |= exact_variant::STYLED_LINE;
    }
    if query.transform.polar_angle[0] > 0.5 {
        key.bits |= exact_variant::POLAR;
    }
    key
}

/// The exact variant a streamed chunk of `series` scans with: the resident
/// series' own variant, so a streamed pick answers bit for bit what the
/// resident pick does, without the dash walk, the error bars and the fill.
/// A stream binds no arc prefix, so with or without that walk its dash gaps
/// pick like its dashes; streaming is not extended (rule 8), so a streamed
/// chunk carries neither error bars nor a fill (`prepare_series_buffer`
/// builds them for resident series only) and its variant leaves their paths
/// out.
pub(super) fn stream_exact_variant(series: &PickSeriesGpu, query: &GpuPickQuery) -> PickExactKey {
    let mut key = series_exact_variant(series, query);
    key.bits &= !(exact_variant::DASH | exact_variant::ERRORBAR | exact_variant::AREA);
    key
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct GpuPickSeriesId(pub(super) u32);

#[cfg(test)]
impl GpuPickSeriesId {
    pub(super) fn index(self) -> u32 {
        self.0
    }
}

#[derive(Clone)]
pub(super) struct PickIdentity {
    pub(super) source_id: Option<String>,
    pub(super) series_id: String,
}

#[derive(Clone)]
pub(super) struct PickSeriesSlot {
    pub(super) identity: PickIdentity,
    pub(super) gpu: PickSeriesGpu,
}

pub(super) struct PickRegistry {
    pub(super) slots: Vec<PickSeriesSlot>,
    pub(super) identities: Arc<[PickIdentity]>,
    pub(super) reduce: ReduceResources,
    pub(super) pool_identity: Option<PoolIdentity>,
    pub(super) pool_layout_generation: u64,
    pub(super) generation: u64,
}

/// One slot in a complete successor registry.
pub(crate) enum GpuPickRegistrySlot<'a> {
    /// Reuse GPU resources from `current_index`, optionally with new identity
    /// strings. The target pool is still validated and relocation-rebound.
    Reuse {
        current_index: usize,
        source_id: Option<String>,
        series_id: String,
    },
    Build(GpuPickSeriesDescriptor<'a>),
}

/// Engine-bound, fully prepared registry successor.
///
/// Its exclusive borrow makes wrong-engine and stale-generation commits
/// unrepresentable. Dropping it leaves the engine unchanged; `commit` only
/// moves the already-complete successor state.
#[must_use = "a prepared GPU-pick transition has no effect until it is committed"]
pub(crate) struct PreparedPickRegistryTransition<'engine> {
    pub(super) engine: &'engine mut GpuPickEngine,
    pub(super) next: Option<PickRegistry>,
}

impl PreparedPickRegistryTransition<'_> {
    pub(crate) fn commit(mut self) {
        self.engine.registry = self
            .next
            .take()
            .expect("prepared GPU-pick transition always owns a successor");
    }
}

/// The arc prefix of the series at registry position `index`, when one was
/// supplied and holds a value for each of its `point_count` points.
pub(super) fn series_arc(
    arcs: &[Option<Arc<wgpu::Buffer>>],
    index: usize,
    point_count: u32,
) -> Option<&wgpu::Buffer> {
    arcs.get(index).and_then(Option::as_deref).filter(|buffer| {
        buffer.size() >= u64::from(point_count) * crate::data_render::line_arc::ARC_PAIR_BYTES
    })
}

/// The identities of a registry's series in registry order.
pub(crate) struct GpuPickSeriesIdentities(pub(super) Arc<[PickIdentity]>);

impl GpuPickSeriesIdentities {
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn series_ids(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|identity| identity.series_id.as_str())
    }
}
