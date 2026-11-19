//! Member statistics measured while a composite's staging is written, and
//! the record and ragged source checks (#311).

use super::*;

/// Scalar statistics of one member column, measured from the encoded pairs
/// in the write observer.
#[derive(Clone, Copy)]
pub(super) struct ValueTally {
    min: f64,
    max: f64,
    finite: u32,
    nan: u32,
}

impl ValueTally {
    pub(super) const EMPTY: Self = Self {
        min: f64::INFINITY,
        max: f64::NEG_INFINITY,
        finite: 0,
        nan: 0,
    };

    pub(super) fn record(&mut self, hi: f32, lo: f32) {
        let value = hi as f64 + lo as f64;
        if value.is_nan() {
            self.nan = self.nan.saturating_add(1);
        } else if value.is_finite() && (hi + lo).is_finite() {
            self.finite = self.finite.saturating_add(1);
            self.min = self.min.min(value);
            self.max = self.max.max(value);
        }
    }
}

/// The offsets checks of a ragged column, measured in the write observer in
/// one pass: no value is kept beyond the previous one.
pub(super) struct OffsetTally {
    next_row: usize,
    in_order: bool,
    integral: bool,
    non_decreasing: bool,
    first: f64,
    previous: f64,
    max_group: f64,
}

impl OffsetTally {
    pub(super) fn record(&mut self, row: usize, hi: f32, lo: f32) {
        let value = hi as f64 + lo as f64;
        if row != self.next_row {
            self.in_order = false;
        }
        self.next_row = row.saturating_add(1);
        if !(value.is_finite() && value >= 0.0 && value.fract() == 0.0) {
            self.integral = false;
        }
        if row == 0 {
            self.first = value;
        } else if value >= self.previous {
            self.max_group = self.max_group.max(value - self.previous);
        } else {
            self.non_decreasing = false;
        }
        self.previous = value;
    }
}

/// The index checks of a triangle column (#210), measured in the write
/// observer in one pass: no index is kept, only the range.
pub(super) struct IndexTally {
    pub(super) next_row: usize,
    pub(super) in_order: bool,
    pub(super) integral: bool,
    pub(super) min: f64,
    pub(super) max: f64,
}

impl IndexTally {
    pub(super) fn record(&mut self, row: usize, hi: f32, lo: f32) {
        let value = hi as f64 + lo as f64;
        if row != self.next_row {
            self.in_order = false;
        }
        self.next_row = row.saturating_add(1);
        if !(value.is_finite() && value >= 0.0 && value <= u32::MAX as f64 && value.fract() == 0.0)
        {
            self.integral = false;
            return;
        }
        self.min = self.min.min(value);
        self.max = self.max.max(value);
    }
}

pub(super) fn composite_error(id: &str, reason: crate::RuleKind) -> crate::FiggyError {
    crate::FiggyError::InvalidSeriesConfig {
        series_id: id.to_owned(),
        kind: reason,
    }
}

/// Remove `old` member columns inside a provisional member batch, let the
/// caller prepare dependent state against the final pool, and publish both.
/// Any failure drops the removal (restoring the old members) and then the
/// batch (un-inserting the new ones): the new members were written to
/// regions that were free before the batch, so the old bytes are untouched.
pub(super) fn finish_member_batch<T>(
    batch: ColumnBatchInsert<'_>,
    old: &[&str],
    prepare: impl FnOnce(&ColumnPool) -> crate::Result<T>,
) -> crate::Result<T> {
    let removal = batch.pool.begin_remove_columns(old)?;
    let value = prepare(&*removal.pool)?;
    removal.commit();
    batch.commit();
    Ok(value)
}

/// The record rules (#311, shared by [`ColumnPool::upsert_record`] and the
/// batch transaction): the fields by schema slot and the row count, or the
/// rule `source` breaks.
pub(super) fn check_record_source<'s>(
    id: &str,
    source: &RecordSource<'s>,
) -> crate::Result<([Option<&'s dyn ColumnSource>; MAX_RECORD_FIELDS], u32)> {
    let schema = source.schema;
    if schema.len() > MAX_RECORD_FIELDS {
        return Err(composite_error(
            &id,
            crate::RuleKind::RecordSchemaFields {
                fields: schema.len() as u64,
                max: MAX_RECORD_FIELDS as u64,
            },
        ));
    }
    if source.fields.len() > MAX_RECORD_FIELDS {
        return Err(composite_error(
            &id,
            crate::RuleKind::RecordSourceFields {
                fields: source.fields.len() as u64,
                max: MAX_RECORD_FIELDS as u64,
            },
        ));
    }

    // Fields by schema slot.
    let mut by_slot: [Option<&dyn ColumnSource>; MAX_RECORD_FIELDS] = [None; MAX_RECORD_FIELDS];
    for (name, field) in source.fields {
        let Some(slot) = schema.iter().position(|(known, _)| known == name) else {
            return Err(composite_error(
                &id,
                crate::RuleKind::RecordUnknownField {
                    name: name.to_string(),
                },
            ));
        };
        if by_slot[slot].is_some() {
            return Err(composite_error(
                &id,
                crate::RuleKind::RecordFieldTwice {
                    name: name.to_string(),
                },
            ));
        }
        by_slot[slot] = Some(*field);
    }
    let mut len: Option<(usize, &str)> = None;
    for (slot, (name, required)) in schema.iter().enumerate() {
        let Some(field) = by_slot[slot] else {
            if *required {
                return Err(composite_error(
                    &id,
                    crate::RuleKind::RecordFieldMissing {
                        name: name.to_string(),
                    },
                ));
            }
            continue;
        };
        match len {
            None => len = Some((field.len(), *name)),
            Some((rows, first)) if rows != field.len() => {
                return Err(composite_error(
                    &id,
                    crate::RuleKind::RecordFieldLength {
                        name: name.to_string(),
                        len: field.len() as u64,
                        first: first.to_string(),
                        rows: rows as u64,
                    },
                ));
            }
            Some(_) => {}
        }
    }
    let Some((len, _)) = len else {
        return Err(composite_error(&id, crate::RuleKind::RecordWithoutFields));
    };
    if len == 0 {
        return Err(composite_error(&id, crate::RuleKind::RecordWithoutRows));
    }
    let rows = u32::try_from(len).map_err(|_| {
        composite_error(
            &id,
            crate::RuleKind::RecordRowsOverflow {
                max: u64::from(u32::MAX),
            },
        )
    })?;

    Ok((by_slot, rows))
}

/// The shape of a ragged source (#311): what its checks before the upload
/// established.
#[derive(Clone, Copy)]
pub(super) struct RaggedShape {
    total: usize,
    pub(super) total32: u32,
    offsets_len: usize,
    pub(super) groups: u32,
}

/// The ragged rules checked before the upload (#311, shared by
/// [`ColumnPool::upsert_ragged`] and the batch transaction).
pub(super) fn check_ragged_source(id: &str, source: &RaggedSource<'_>) -> crate::Result<RaggedShape> {
    let total = source.values.len();
    if total == 0 {
        return Err(composite_error(&id, crate::RuleKind::RaggedEmpty));
    }
    // Offsets index the values as `u32`: the GPU's index space, as a
    // column's lane base.
    let total32 = u32::try_from(total).map_err(|_| {
        composite_error(
            &id,
            crate::RuleKind::RaggedValuesOverflow {
                max: u64::from(u32::MAX),
                total: total as u64,
            },
        )
    })?;
    let offsets_len = source.offsets.len();
    if offsets_len < 2 {
        return Err(composite_error(
            &id,
            crate::RuleKind::RaggedOffsetCount {
                offsets: offsets_len as u64,
            },
        ));
    }
    let groups = u32::try_from(offsets_len - 1).map_err(|_| {
        composite_error(
            &id,
            crate::RuleKind::RaggedGroupsOverflow {
                max: u64::from(u32::MAX),
            },
        )
    })?;
    Ok(RaggedShape {
        total,
        total32,
        offsets_len,
        groups,
    })
}

impl OffsetTally {
    pub(super) fn new() -> Self {
        Self {
            next_row: 0,
            in_order: true,
            integral: true,
            non_decreasing: true,
            first: f64::NAN,
            previous: f64::NAN,
            max_group: 0.0,
        }
    }

    /// The offsets rules measured while the offsets were written (#311,
    /// shared by [`ColumnPool::upsert_ragged`] and the batch transaction).
    pub(super) fn check(&self, id: &str, shape: &RaggedShape) -> crate::Result<()> {
        if !self.in_order || self.next_row != shape.offsets_len {
            return Err(composite_error(id, crate::RuleKind::RaggedOffsetsOrder));
        }
        if !self.integral {
            return Err(composite_error(id, crate::RuleKind::RaggedOffsetsInteger));
        }
        if self.first != 0.0 {
            return Err(composite_error(
                id,
                crate::RuleKind::RaggedOffsetsStart {
                    first: self.first.to_string(),
                },
            ));
        }
        if !self.non_decreasing {
            return Err(composite_error(
                id,
                crate::RuleKind::RaggedOffsetsDecreasing,
            ));
        }
        if self.previous != shape.total as f64 {
            return Err(composite_error(
                id,
                crate::RuleKind::RaggedLastOffset {
                    last: self.previous.to_string(),
                    total: shape.total as u64,
                },
            ));
        }
        Ok(())
    }

    /// The [`RaggedStats`] of a ragged column whose values measured `values`.
    pub(super) fn ragged_stats(&self, shape: &RaggedShape, values: &ValueTally) -> RaggedStats {
        RaggedStats {
            groups: shape.groups,
            total: shape.total32,
            min: values.min,
            max: values.max,
            finite: values.finite,
            nan: values.nan,
            non_decreasing: self.non_decreasing,
            max_group: self.max_group as u32,
        }
    }
}

/// The [`RecordStats`] of a record of `rows` rows whose present fields
/// measured `fields`, each with its schema slot (#311, shared by
/// [`ColumnPool::upsert_record`] and the batch transaction).
pub(super) fn record_stats(rows: u32, fields: impl Iterator<Item = (usize, ValueTally)>) -> RecordStats {
    let mut stats = RecordStats {
        len: rows,
        present: [false; MAX_RECORD_FIELDS],
        min: [f64::INFINITY; MAX_RECORD_FIELDS],
        max: [f64::NEG_INFINITY; MAX_RECORD_FIELDS],
        finite: [0; MAX_RECORD_FIELDS],
        nan: [0; MAX_RECORD_FIELDS],
    };
    for (slot, tally) in fields {
        stats.present[slot] = true;
        stats.min[slot] = tally.min;
        stats.max[slot] = tally.max;
        stats.finite[slot] = tally.finite;
        stats.nan[slot] = tally.nan;
    }
    stats
}

/// The id of member `name` of composite `id`, uploaded with `token` (#311).
pub(super) fn member_id(id: &str, token: u64, name: &str) -> ColumnId {
    format!("{id}{MEMBER_SEPARATOR}{token}{MEMBER_SEPARATOR}{name}")
}
