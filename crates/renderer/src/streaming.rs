//! Bounded, payload-free streaming admission and completion bookkeeping.
//!
//! The Renderer supplies source/view stamps from its own SSOT. This module
//! neither registers columns nor submits GPU work. Receipts must be completed
//! only after the corresponding GPU submission actually completes.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourceStamp(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ViewEpoch(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JobId(u64);

impl JobId {
    pub(crate) fn sequence(self) -> u64 {
        self.0
    }
}

pub(crate) use crate::streaming_source::StreamEncoding as SourceEncoding;

impl SourceEncoding {
    pub(crate) fn bytes_per_value(self) -> u64 {
        match self {
            Self::ScalarF32 => 4,
            Self::HiLoF32 => 8,
        }
    }
}

/// Logical identity and revision are supplied by the Renderer registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ColumnRange {
    pub column: u64,
    pub revision: u64,
    pub source_len: u64,
    pub offset: u64,
    pub len: u64,
    pub encoding: SourceEncoding,
}

impl ColumnRange {
    pub(crate) fn byte_len(self) -> Result<u64, StreamError> {
        let end = self
            .offset
            .checked_add(self.len)
            .ok_or(StreamError::Overflow)?;
        if self.len == 0 || end > self.source_len || self.source_len > u32::MAX as u64 {
            return Err(StreamError::InvalidRange);
        }
        self.len
            .checked_mul(self.encoding.bytes_per_value())
            .ok_or(StreamError::Overflow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StreamLimits {
    pub max_jobs: usize,
    pub max_slots: usize,
    pub max_columns_per_request: usize,
    pub max_chunk_bytes: u64,
    pub max_in_flight_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamError {
    InvalidLimits,
    TooManyJobs,
    InvalidRange,
    Overflow,
    TooLarge,
    Stale,
    WrongState,
    InvalidPayload,
    WriterFailed,
    AllocationFailed,
}

/// Opaque identity; cannot be forged by a host or confused with another runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RequestTicket {
    renderer: u64,
    request: u64,
    job: JobId,
    source: SourceStamp,
    view: ViewEpoch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SubmissionReceipt {
    ticket: RequestTicket,
    serial: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestStatus {
    Ready(RequestTicket),
    Backpressure,
}

/// Borrowed only for `accept`; never stored by the scheduler.
pub(crate) struct ColumnInput<'a> {
    pub range: ColumnRange,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotState {
    Requested,
    Recorded,
    Submitted(SubmissionReceipt),
}

struct Slot {
    ticket: RequestTicket,
    columns: Vec<ColumnRange>,
    charged_bytes: u64,
    state: SlotState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveJob {
    chart_key: u64,
    auxiliary: bool,
    id: JobId,
    source: SourceStamp,
    view: ViewEpoch,
}

/// Chart-scoped active jobs sharing bounded outstanding work and byte limits.
///
/// `charged_bytes` reserves BOTH staging and GPU work storage (two hi/lo
/// buffers) even for scalar input. The renderer must separately budget target,
/// halo and derived resources in its global ledger. This is not a resident-cap
/// admission proof, and must never be used as one.
pub(crate) struct StreamScheduler {
    renderer: u64,
    limits: StreamLimits,
    next_job: u64,
    next_request: u64,
    next_submission: u64,
    active: Vec<ActiveJob>,
    slots: Vec<Slot>,
    charged_bytes: u64,
}

impl StreamScheduler {
    /// Renderer identity must be unique among live Renderer instances.
    pub(crate) fn new(renderer: u64, limits: StreamLimits) -> Result<Self, StreamError> {
        if limits.max_jobs == 0
            || limits.max_slots == 0
            || limits.max_columns_per_request == 0
            || limits.max_chunk_bytes == 0
            || limits.max_in_flight_bytes == 0
            || limits.max_chunk_bytes > limits.max_in_flight_bytes
        {
            return Err(StreamError::InvalidLimits);
        }
        Ok(Self {
            renderer,
            limits,
            next_job: 0,
            next_request: 0,
            next_submission: 0,
            active: Vec::new(),
            slots: Vec::new(),
            charged_bytes: 0,
        })
    }

    pub(crate) fn start_job(
        &mut self,
        chart_key: u64,
        source: SourceStamp,
        view: ViewEpoch,
    ) -> Result<JobId, StreamError> {
        let previous = self
            .active
            .iter()
            .find(|job| job.chart_key == chart_key && !job.auxiliary)
            .map(|job| job.id);
        if previous.is_none()
            && !self.active.iter().any(|job| job.chart_key == chart_key)
            && self.active_jobs() >= self.limits.max_jobs
        {
            return Err(StreamError::TooManyJobs);
        }
        let next = self.next_job.checked_add(1).ok_or(StreamError::Overflow)?;
        if previous.is_none() {
            self.active
                .try_reserve(1)
                .map_err(|_| StreamError::AllocationFailed)?;
        }
        // Validate before cancellation so a rejected replacement preserves its job.
        if let Some(job) = previous {
            self.cancel(job);
        }
        self.next_job = next;
        let job = JobId(next);
        self.active.push(ActiveJob {
            chart_key,
            auxiliary: false,
            id: job,
            source,
            view,
        });
        Ok(job)
    }

    pub(crate) fn active_jobs(&self) -> usize {
        self.active.iter().enumerate().filter(|(index, job)| {
            !self.active[..*index].iter().any(|prior| prior.chart_key == job.chart_key)
        }).count()
    }

    /// One independent replay per live chart; all slots and bytes remain shared.
    pub(crate) fn start_auxiliary(&mut self, chart_key: u64, source: SourceStamp, view: ViewEpoch) -> Result<JobId, StreamError> {
        if self.job_for_chart(chart_key).is_none() || self.active.iter().any(|job| job.chart_key == chart_key && job.auxiliary) {
            return Err(StreamError::TooManyJobs);
        }
        let next = self.next_job.checked_add(1).ok_or(StreamError::Overflow)?;
        self.active.try_reserve(1).map_err(|_| StreamError::AllocationFailed)?;
        let id = JobId(next);
        self.active.push(ActiveJob { chart_key, auxiliary: true, id, source, view });
        self.next_job = next;
        Ok(id)
    }

    pub(crate) fn contains_job(&self, chart_key: u64, id: JobId) -> bool {
        self.active.iter().any(|job| job.chart_key == chart_key && job.id == id)
    }

    pub(crate) fn cancel_chart(&mut self, chart_key: u64) {
        if let Some(job) = self
            .active
            .iter()
            .find(|job| job.chart_key == chart_key && !job.auxiliary)
            .map(|job| job.id)
        {
            self.cancel(job);
        }
    }

    pub(crate) fn charged_bytes(&self) -> u64 {
        self.charged_bytes
    }
    pub(crate) fn occupied_slots(&self) -> usize {
        self.slots.len()
    }

    pub(crate) fn job_usage(&self, sequence: u64) -> (usize, u64) {
        self.slots.iter().filter(|slot| slot.ticket.job.0 == sequence)
            .fold((0, 0), |(count, bytes), slot| (count + 1, bytes + slot.charged_bytes))
    }

    pub(crate) fn limits(&self) -> StreamLimits {
        self.limits
    }

    /// Prune against current Renderer authority without copying chart state.
    pub(crate) fn retain_jobs(
        &mut self,
        mut valid: impl FnMut(JobId, u64, SourceStamp, ViewEpoch) -> bool,
    ) {
        let mut index = 0;
        while index < self.active.len() {
            let job = self.active[index];
            if valid(job.id, job.chart_key, job.source, job.view) {
                index += 1;
            } else {
                self.cancel(job.id);
            }
        }
    }

    pub(crate) fn job_for_chart(&self, chart: u64) -> Option<JobId> {
        self.active
            .iter()
            .find(|job| job.chart_key == chart && !job.auxiliary)
            .map(|job| job.id)
    }

    pub(crate) fn has_slots_for_job(&self, job: JobId) -> bool {
        self.slots.iter().any(|slot| slot.ticket.job == job)
    }

    pub(crate) fn is_requested(&self, ticket: RequestTicket) -> bool {
        self.active_ticket(ticket)
            && self
                .slots
                .iter()
                .any(|slot| slot.ticket == ticket && slot.state == SlotState::Requested)
    }

    pub(crate) fn contains_ticket(&self, ticket: RequestTicket) -> bool {
        self.slots.iter().any(|slot| slot.ticket == ticket)
    }

    pub(crate) fn request(
        &mut self,
        job: JobId,
        columns: &[ColumnRange],
    ) -> Result<RequestStatus, StreamError> {
        let ActiveJob { source, view, .. } = *self
            .active
            .iter()
            .find(|current| current.id == job)
            .ok_or(StreamError::Stale)?;
        if columns.is_empty() || columns.len() > self.limits.max_columns_per_request {
            return Err(StreamError::InvalidRange);
        }
        let mut input_bytes = 0u64;
        let mut charged_bytes = 0u64;
        for column in columns {
            input_bytes = input_bytes
                .checked_add(column.byte_len()?)
                .ok_or(StreamError::Overflow)?;
            // Pair staging + pair work buffer; host-owned input is not retained.
            charged_bytes = charged_bytes
                .checked_add(column.len.checked_mul(16).ok_or(StreamError::Overflow)?)
                .ok_or(StreamError::Overflow)?;
        }
        if input_bytes > self.limits.max_chunk_bytes
            || charged_bytes > self.limits.max_in_flight_bytes
        {
            return Err(StreamError::TooLarge);
        }
        let next_charge = self
            .charged_bytes
            .checked_add(charged_bytes)
            .ok_or(StreamError::Overflow)?;
        if self.slots.len() >= self.limits.max_slots
            || next_charge > self.limits.max_in_flight_bytes
        {
            return Ok(RequestStatus::Backpressure);
        }
        let request = self
            .next_request
            .checked_add(1)
            .ok_or(StreamError::Overflow)?;
        let ticket = RequestTicket {
            renderer: self.renderer,
            request,
            job,
            source,
            view,
        };
        let mut owned_columns = Vec::new();
        owned_columns
            .try_reserve_exact(columns.len())
            .map_err(|_| StreamError::AllocationFailed)?;
        owned_columns.extend_from_slice(columns);
        self.slots
            .try_reserve(1)
            .map_err(|_| StreamError::AllocationFailed)?;
        self.slots.push(Slot {
            ticket,
            columns: owned_columns,
            charged_bytes,
            state: SlotState::Requested,
        });
        self.next_request = request;
        self.charged_bytes = next_charge;
        Ok(RequestStatus::Ready(ticket))
    }

    fn active_ticket(&self, ticket: RequestTicket) -> bool {
        ticket.renderer == self.renderer
            && self.active.iter().any(|job| {
                job.id == ticket.job && job.source == ticket.source && job.view == ticket.view
            })
    }

    fn slot_index(&self, ticket: RequestTicket) -> Result<usize, StreamError> {
        self.slots
            .iter()
            .position(|slot| slot.ticket == ticket)
            .ok_or(StreamError::Stale)
    }

    /// Validate every range and payload before invoking the write-only staging
    /// adapter. `record` must be transactional: on error/panic it must discard
    /// its temporary GPU resources/commands, never publish partial work.
    /// Scheduler state stays Requested on writer error or unwind.
    pub(crate) fn accept<T>(
        &mut self,
        ticket: RequestTicket,
        inputs: &[ColumnInput<'_>],
        record: impl FnOnce(&[ColumnInput<'_>]) -> Result<T, StreamError>,
    ) -> Result<T, StreamError> {
        if !self.active_ticket(ticket) {
            return Err(StreamError::Stale);
        }
        let index = self.slot_index(ticket)?;
        let slot = &self.slots[index];
        if slot.state != SlotState::Requested {
            return Err(StreamError::WrongState);
        }
        if inputs.len() != slot.columns.len() {
            return Err(StreamError::InvalidPayload);
        }
        for (input, expected) in inputs.iter().zip(&slot.columns) {
            if input.range != *expected
                || u64::try_from(input.bytes.len()).map_err(|_| StreamError::Overflow)?
                    != expected.byte_len()?
            {
                return Err(StreamError::InvalidPayload);
            }
        }
        let recorded = record(inputs)?;
        self.slots[index].state = SlotState::Recorded;
        Ok(recorded)
    }

    /// Accept the exact ranges already owned by this ticket. Payload validation
    /// is performed by the renderer's `ColumnSource` binding layer before this
    /// call; the scheduler exposes only its immutable expected ranges to the
    /// transactional recorder.
    pub(crate) fn accept_expected<T>(
        &mut self,
        ticket: RequestTicket,
        record: impl FnOnce(&[ColumnRange]) -> Result<T, StreamError>,
    ) -> Result<T, StreamError> {
        if !self.active_ticket(ticket) {
            return Err(StreamError::Stale);
        }
        let index = self.slot_index(ticket)?;
        let slot = &self.slots[index];
        if slot.state != SlotState::Requested {
            return Err(StreamError::WrongState);
        }
        let recorded = record(&slot.columns)?;
        self.slots[index].state = SlotState::Recorded;
        Ok(recorded)
    }

    /// Associate a recorded command with an actual submission. Recorded work
    /// cancelled before submission may still be submitted to its OLD target;
    /// the owner must retain that target and never reinterpret its epoch.
    pub(crate) fn submit(
        &mut self,
        ticket: RequestTicket,
    ) -> Result<SubmissionReceipt, StreamError> {
        let index = self.slot_index(ticket)?;
        if self.slots[index].state != SlotState::Recorded {
            return Err(StreamError::WrongState);
        }
        let serial = self
            .next_submission
            .checked_add(1)
            .ok_or(StreamError::Overflow)?;
        let receipt = SubmissionReceipt { ticket, serial };
        self.next_submission = serial;
        self.slots[index].state = SlotState::Submitted(receipt);
        Ok(receipt)
    }

    /// Release a provider reservation that has not recorded GPU work.
    pub(crate) fn discard_requested(&mut self, ticket: RequestTicket) -> Result<(), StreamError> {
        let index = self.slot_index(ticket)?;
        if self.slots[index].state != SlotState::Requested {
            return Err(StreamError::WrongState);
        }
        self.release(index);
        Ok(())
    }

    /// Explicit proof that an unsubmitted recorded command was discarded.
    pub(crate) fn discard_recorded(&mut self, ticket: RequestTicket) -> Result<(), StreamError> {
        let index = self.slot_index(ticket)?;
        if self.slots[index].state != SlotState::Recorded {
            return Err(StreamError::WrongState);
        }
        self.release(index);
        Ok(())
    }

    /// Non-blocking cancellation. Submitted AND recorded bytes remain charged.
    pub(crate) fn cancel(&mut self, job: JobId) {
        if let Some(index) = self.active.iter().position(|current| current.id == job) {
            self.active.swap_remove(index);
        }
        let mut index = 0;
        while index < self.slots.len() {
            if self.slots[index].ticket.job == job
                && self.slots[index].state == SlotState::Requested
            {
                self.release(index);
            } else {
                index += 1;
            }
        }
    }

    /// Exact receipt completion only; out-of-order callbacks are supported.
    /// A queue submit, cancel, frame boundary, or newer callback is NOT proof.
    pub(crate) fn complete(&mut self, receipt: SubmissionReceipt) -> Result<(), StreamError> {
        let index = self.slot_index(receipt.ticket)?;
        if self.slots[index].state != SlotState::Submitted(receipt) {
            return Err(StreamError::WrongState);
        }
        self.release(index);
        Ok(())
    }

    fn release(&mut self, index: usize) {
        let slot = self.slots.swap_remove(index);
        self.charged_bytes -= slot.charged_bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scheduler(slots: usize, bytes: u64) -> StreamScheduler {
        StreamScheduler::new(
            7,
            StreamLimits {
                max_jobs: 2,
                max_slots: slots,
                max_columns_per_request: 16,
                max_chunk_bytes: bytes,
                max_in_flight_bytes: bytes,
            },
        )
        .unwrap()
    }

    #[test]
    fn auxiliary_jobs_share_admission_without_replacing_display_or_escaping_chart_bound() {
        let mut s = StreamScheduler::new(1, StreamLimits { max_jobs: 1, max_slots: 1,
            max_columns_per_request: 4, max_chunk_bytes: 64, max_in_flight_bytes: 64 }).unwrap();
        let display = s.start_job(1, SourceStamp(1), ViewEpoch(1)).unwrap();
        let aux = s.start_auxiliary(1, SourceStamp(1), ViewEpoch(1)).unwrap();
        assert_eq!(s.job_for_chart(1), Some(display));
        assert_eq!(s.active_jobs(), 1);
        assert_eq!(s.start_auxiliary(1, SourceStamp(1), ViewEpoch(1)), Err(StreamError::TooManyJobs));
        let range = ColumnRange { column: 0, revision: 1, source_len: 1, offset: 0, len: 1, encoding: SourceEncoding::ScalarF32 };
        let RequestStatus::Ready(pending) = s.request(aux, &[range]).unwrap() else { panic!() };
        assert_eq!(s.request(display, &[range]).unwrap(), RequestStatus::Backpressure);
        s.cancel_chart(1);
        assert!(s.is_requested(pending));
        assert_eq!(s.active_jobs(), 1);
        assert_eq!(s.start_job(2, SourceStamp(1), ViewEpoch(1)), Err(StreamError::TooManyJobs));
        let replacement = s.start_job(1, SourceStamp(2), ViewEpoch(2)).unwrap();
        assert!(s.contains_job(1, aux));
        s.cancel(aux);
        assert_eq!(s.job_for_chart(1), Some(replacement));
        assert_eq!(s.occupied_slots(), 0);
    }
    fn column(id: u64, encoding: SourceEncoding) -> ColumnRange {
        ColumnRange {
            column: id,
            revision: 3,
            source_len: 100,
            offset: 10,
            len: 2,
            encoding,
        }
    }
    fn ticket(s: &mut StreamScheduler, job: JobId, cols: &[ColumnRange]) -> RequestTicket {
        match s.request(job, cols).unwrap() {
            RequestStatus::Ready(t) => t,
            RequestStatus::Backpressure => panic!("unexpected backpressure"),
        }
    }
    fn record(s: &mut StreamScheduler, t: RequestTicket, col: ColumnRange) {
        let bytes = [0u8; 16];
        s.accept::<()>(
            t,
            &[ColumnInput {
                range: col,
                bytes: &bytes[..col.byte_len().unwrap() as usize],
            }],
            |_| Ok(()),
        )
        .unwrap();
    }

    #[test]
    fn concurrent_charts_cancel_independently() {
        let mut s = scheduler(2, 64);
        let col = column(1, SourceEncoding::ScalarF32);
        let a = s.start_job(10, SourceStamp(1), ViewEpoch(1)).unwrap();
        let ta = ticket(&mut s, a, &[col]);
        let b = s.start_job(20, SourceStamp(1), ViewEpoch(1)).unwrap();
        let tb = ticket(&mut s, b, &[col]);
        assert_eq!(s.active_jobs(), 2);
        assert_eq!(s.charged_bytes(), 64);
        s.cancel_chart(10);
        assert_eq!(s.active_jobs(), 1);
        assert_eq!(s.charged_bytes(), 32);
        assert_eq!(
            s.accept::<()>(ta, &[], |_| unreachable!()),
            Err(StreamError::Stale)
        );
        record(&mut s, tb, col);
        s.cancel(a);
        assert_eq!(s.active_jobs(), 1);
        let rb = s.submit(tb).unwrap();
        s.complete(rb).unwrap();
        s.cancel_chart(20);
        assert_eq!(s.active_jobs(), 0);
    }

    #[test]
    fn replacement_preserves_other_chart_and_old_completion_identity() {
        let mut s = scheduler(3, 96);
        let col = column(1, SourceEncoding::ScalarF32);
        let a = s.start_job(10, SourceStamp(1), ViewEpoch(1)).unwrap();
        let ta = ticket(&mut s, a, &[col]);
        record(&mut s, ta, col);
        let ra = s.submit(ta).unwrap();
        let b = s.start_job(20, SourceStamp(1), ViewEpoch(1)).unwrap();
        let tb = ticket(&mut s, b, &[col]);
        let new_a = s.start_job(10, SourceStamp(1), ViewEpoch(1)).unwrap();
        let new_ta = ticket(&mut s, new_a, &[col]);
        assert_eq!(s.request(a, &[col]), Err(StreamError::Stale));
        assert_eq!(
            s.accept::<()>(ta, &[], |_| unreachable!()),
            Err(StreamError::Stale)
        );
        record(&mut s, tb, col);
        record(&mut s, new_ta, col);
        let new_ra = s.submit(new_ta).unwrap();
        s.complete(ra).unwrap();
        assert_eq!(s.charged_bytes(), 64);
        assert_eq!(s.active_jobs(), 2);
        assert_eq!(s.complete(ra), Err(StreamError::Stale));
        s.complete(new_ra).unwrap();
        assert_eq!(s.charged_bytes(), 32);
        s.discard_recorded(tb).unwrap();
    }

    #[test]
    fn concurrent_charts_share_slot_and_byte_backpressure() {
        for (slots, bytes) in [(1, 64), (2, 32)] {
            let mut s = scheduler(slots, bytes);
            let col = column(1, SourceEncoding::ScalarF32);
            let a = s.start_job(10, SourceStamp(1), ViewEpoch(1)).unwrap();
            let b = s.start_job(20, SourceStamp(2), ViewEpoch(2)).unwrap();
            let ta = ticket(&mut s, a, &[col]);
            record(&mut s, ta, col);
            s.cancel(a);
            assert_eq!(s.request(b, &[col]), Ok(RequestStatus::Backpressure));
            let ra = s.submit(ta).unwrap();
            assert_eq!(s.request(b, &[col]), Ok(RequestStatus::Backpressure));
            s.complete(ra).unwrap();
            let tb = ticket(&mut s, b, &[col]);
            record(&mut s, tb, col);
            assert_eq!(s.charged_bytes(), 32);
        }
    }

    #[test]
    fn job_metadata_is_bounded_and_failed_start_preserves_jobs() {
        let mut s = scheduler(2, 64);
        let col = column(1, SourceEncoding::ScalarF32);
        let a = s.start_job(10, SourceStamp(1), ViewEpoch(1)).unwrap();
        let b = s.start_job(20, SourceStamp(2), ViewEpoch(2)).unwrap();
        let ta = ticket(&mut s, a, &[col]);
        let tb = ticket(&mut s, b, &[col]);
        let old_active = s.active.clone();
        let old_next = s.next_job;
        assert_eq!(
            s.start_job(30, SourceStamp(3), ViewEpoch(3)),
            Err(StreamError::TooManyJobs)
        );
        assert_eq!(s.active, old_active);
        assert_eq!(s.next_job, old_next);
        assert_eq!(s.charged_bytes(), 64);
        s.next_job = u64::MAX;
        assert_eq!(
            s.start_job(10, SourceStamp(3), ViewEpoch(3)),
            Err(StreamError::Overflow)
        );
        assert_eq!(s.active, old_active);
        record(&mut s, ta, col);
        record(&mut s, tb, col);
        s.discard_recorded(ta).unwrap();
        s.discard_recorded(tb).unwrap();
        s.next_job = old_next;
        for epoch in 3..1000 {
            s.start_job(10, SourceStamp(1), ViewEpoch(epoch)).unwrap();
            assert_eq!(s.active_jobs(), 2);
            s.cancel_chart(10);
            assert_eq!(s.active_jobs(), 1);
            assert_eq!(s.occupied_slots(), 0);
        }
        s.cancel_chart(20);
        assert_eq!(s.active_jobs(), 0);
        s.limits.max_jobs = 0;
        assert!(matches!(
            StreamScheduler::new(1, s.limits),
            Err(StreamError::InvalidLimits)
        ));
    }

    #[test]
    fn arbitrary_columns_and_hi_lo_sizing() {
        let mut s = scheduler(2, 4096);
        let job = s.start_job(1, SourceStamp(1), ViewEpoch(1)).unwrap();
        let cols: Vec<_> = (0..9)
            .map(|i| {
                column(
                    i,
                    if i % 2 == 0 {
                        SourceEncoding::ScalarF32
                    } else {
                        SourceEncoding::HiLoF32
                    },
                )
            })
            .collect();
        assert_eq!(cols[0].byte_len(), Ok(8));
        assert_eq!(cols[1].byte_len(), Ok(16));
        let t = ticket(&mut s, job, &cols);
        let bytes = [0u8; 16];
        let inputs: Vec<_> = cols
            .iter()
            .map(|col| ColumnInput {
                range: *col,
                bytes: &bytes[..col.byte_len().unwrap() as usize],
            })
            .collect();
        s.accept::<()>(t, &inputs, |_| Ok(())).unwrap();
        assert_eq!(s.charged_bytes(), 9 * 32);
        assert_eq!(
            s.accept::<()>(t, &inputs, |_| panic!("duplicate writer")),
            Err(StreamError::WrongState)
        );
    }

    #[test]
    fn failed_acceptance_and_writer_panic_preserve_request() {
        let mut s = scheduler(1, 32);
        let job = s.start_job(1, SourceStamp(1), ViewEpoch(1)).unwrap();
        let col = column(1, SourceEncoding::ScalarF32);
        let t = ticket(&mut s, job, &[col]);
        let bytes = [0; 8];
        let mut inputs = [ColumnInput {
            range: col,
            bytes: &bytes[..4],
        }];
        assert_eq!(
            s.accept::<()>(t, &inputs, |_| panic!("invalid input writer")),
            Err(StreamError::InvalidPayload)
        );
        inputs[0].bytes = &bytes;
        inputs[0].range.revision += 1;
        assert_eq!(
            s.accept::<()>(t, &inputs, |_| panic!("wrong revision writer")),
            Err(StreamError::InvalidPayload)
        );
        inputs[0].range = col;
        assert_eq!(
            s.accept::<()>(t, &inputs, |_| Err(StreamError::WriterFailed)),
            Err(StreamError::WriterFailed)
        );
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = s.accept::<()>(t, &inputs, |_| panic!("writer"));
        }));
        assert!(panic.is_err());
        assert_eq!(s.charged_bytes(), 32);
        record(&mut s, t, col);
    }

    #[test]
    fn cancellation_late_reply_and_exact_out_of_order_completion() {
        let mut s = scheduler(2, 64);
        let col = column(1, SourceEncoding::ScalarF32);
        let a = s.start_job(1, SourceStamp(1), ViewEpoch(1)).unwrap();
        let ta = ticket(&mut s, a, &[col]);
        record(&mut s, ta, col);
        let ra = s.submit(ta).unwrap();
        s.cancel(a);
        assert_eq!(s.charged_bytes(), 32);
        let b = s.start_job(1, SourceStamp(2), ViewEpoch(2)).unwrap();
        let tb = ticket(&mut s, b, &[col]);
        assert_eq!(s.request(b, &[col]), Ok(RequestStatus::Backpressure));
        assert_eq!(
            s.accept::<()>(ta, &[], |_| panic!("late writer")),
            Err(StreamError::Stale)
        );
        record(&mut s, tb, col);
        let rb = s.submit(tb).unwrap();
        s.complete(rb).unwrap();
        assert_eq!(s.charged_bytes(), 32);
        assert_eq!(s.complete(rb), Err(StreamError::Stale));
        s.complete(ra).unwrap();
        assert_eq!(s.charged_bytes(), 0);
    }

    #[test]
    fn recorded_cancel_keeps_storage_until_submit_or_discard() {
        let mut s = scheduler(2, 64);
        let col = column(1, SourceEncoding::HiLoF32);
        let a = s.start_job(1, SourceStamp(1), ViewEpoch(1)).unwrap();
        let ta = ticket(&mut s, a, &[col]);
        record(&mut s, ta, col);
        let b = s.start_job(1, SourceStamp(1), ViewEpoch(2)).unwrap();
        let tb = ticket(&mut s, b, &[col]);
        record(&mut s, tb, col);
        assert_eq!(s.charged_bytes(), 64);
        let ra = s.submit(ta).unwrap();
        s.cancel(b);
        assert_eq!(s.charged_bytes(), 64);
        s.discard_recorded(tb).unwrap();
        s.complete(ra).unwrap();
        assert_eq!(s.occupied_slots(), 0);
    }

    #[test]
    fn requested_cancel_and_stamps_invalidate_old_identity() {
        let mut s = scheduler(1, 32);
        let col = column(1, SourceEncoding::ScalarF32);
        let a = s.start_job(1, SourceStamp(1), ViewEpoch(1)).unwrap();
        let ta = ticket(&mut s, a, &[col]);
        let b = s.start_job(1, SourceStamp(2), ViewEpoch(1)).unwrap();
        assert_eq!(s.charged_bytes(), 0);
        let tb = ticket(&mut s, b, &[col]);
        assert_ne!(ta, tb);
        assert_eq!(
            s.accept::<()>(ta, &[], |_| unreachable!()),
            Err(StreamError::Stale)
        );
        assert_eq!(s.request(a, &[col]), Err(StreamError::Stale));
    }

    #[test]
    fn limits_overflow_and_failed_start_are_atomic() {
        assert!(matches!(
            StreamScheduler::new(
                1,
                StreamLimits {
                    max_jobs: 2,
                    max_slots: 0,
                    max_columns_per_request: 1,
                    max_chunk_bytes: 1,
                    max_in_flight_bytes: 1
                }
            ),
            Err(StreamError::InvalidLimits)
        ));
        let mut col = column(1, SourceEncoding::HiLoF32);
        col.offset = u64::MAX;
        assert_eq!(col.byte_len(), Err(StreamError::Overflow));
        col.offset = 99;
        assert_eq!(col.byte_len(), Err(StreamError::InvalidRange));
        let mut s = scheduler(1, 16);
        let a = s.start_job(1, SourceStamp(1), ViewEpoch(1)).unwrap();
        assert_eq!(
            s.request(a, &[column(1, SourceEncoding::ScalarF32)]),
            Err(StreamError::TooLarge)
        );
        s.next_job = u64::MAX;
        assert_eq!(
            s.start_job(1, SourceStamp(2), ViewEpoch(2)),
            Err(StreamError::Overflow)
        );
        assert_eq!(
            s.active,
            vec![ActiveJob {
                chart_key: 1,
                auxiliary: false,
                id: a,
                source: SourceStamp(1),
                view: ViewEpoch(1)
            }]
        );
        assert_eq!(s.charged_bytes(), 0);
    }
}
