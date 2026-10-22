//! The point picker's series registry: series identities, slots, the
//! registry and its prepared transitions, and the per-series variant choice.

use super::*;

/// The exact module and variant that pick `series` as the query draws it:
/// its primitive class, the styled marker and line paths when the draw
/// style uses them, the dash walk for a dashed line and the polar paths on
/// polar axes.
pub(super) fn series_exact_variant(series: &PickSeriesGpu, query: &GpuPickQuery) -> PickExactKey {
    let scatter = series.flags & FLAG_SCATTER != 0;
    let line = series.flags & FLAG_LINE != 0;
    let shader = if series.flags & FLAG_VECTOR != 0 {
        PickShader::Vector
    } else {
        match (scatter, line) {
            (true, true) => PickShader::MarkerLine,
            (false, true) => PickShader::Line,
            _ => PickShader::Marker,
        }
    };
    let mut bits = 0;
    if scatter && shader != PickShader::Vector && query.draw_style.marker != PICK_MARKER_PRECISE {
        bits |= exact_variant::STYLED_MARKER;
    }
    if line {
        if series.line_dash_len > 0 {
            bits |= exact_variant::DASH;
        }
        if query.draw_style.line != PICK_LINE_PRECISE {
            bits |= exact_variant::STYLED_LINE;
        }
    }
    if query.transform.polar_angle[0] > 0.5 {
        bits |= exact_variant::POLAR;
    }
    // Without a style map every point has the series' shape.
    let shape = if scatter && series.flags & FLAG_STYLE_MAP == 0 {
        series.base_shape_id
    } else {
        PickExactKey::ANY_SHAPE
    };
    PickExactKey {
        shader,
        bits,
        shape,
    }
}

/// The exact variant a streamed chunk of `series` scans with: the resident
/// series' own variant, so a streamed pick answers bit for bit what the
/// resident pick does, without the dash walk. A stream binds no arc prefix,
/// so with or without that walk its dash gaps pick like its dashes.
pub(super) fn stream_exact_variant(series: &PickSeriesGpu, query: &GpuPickQuery) -> PickExactKey {
    let mut key = series_exact_variant(series, query);
    key.bits &= !exact_variant::DASH;
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
    arcs.get(index)
        .and_then(Option::as_deref)
        .filter(|buffer| buffer.size() >= u64::from(point_count) * 4)
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
