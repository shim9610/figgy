//! The order, pace and outcome of a browser renderer's asynchronous
//! pipeline compiles (#299). The browser side that issues the compiles is
//! `browser_compile::CompileQueue`.
//!
//! A browser compiles the render pipelines of a renderer's shader set in
//! the background. Issuing them all at once made a pipeline the screen
//! needs wait behind every one issued before it, since the browser works
//! in issue order. This schedule keeps at most [`CompileSchedule::limit`]
//! compiles in flight ([`compile_limit`]), issues what the screen asks for
//! ([`CompileSchedule::prioritize`]) before what it does not, and records
//! how each compile ended, so a frame draws a series only once its
//! pipelines are ready and a later synchronous creation finds them in the
//! browser's cache. An issued compile is never cancelled.
//!
//! Every scheduled compile ends in exactly one [`CompileOutcome`]. A waiter
//! is handed the indices it waits for and reads each outcome, so a wait
//! always ends: the browser settles every `createRenderPipelineAsync`
//! promise (it resolves on success and on device loss, and rejects with a
//! `GPUPipelineError` otherwise), device loss ends every compile not
//! settled yet, an upgrade supersedes the compiles it no longer issues, and
//! dropping the queue cancels the rest.
//!
//! It holds no GPU object and runs on every target, so its rules are tested
//! without a browser.

use std::collections::VecDeque;

use crate::shader_assembly::ShaderSet;

/// Compiles in flight when the platform does not report its core count.
pub(crate) const FALLBACK_COMPILE_LIMIT: usize = 4;

/// How many compiles a renderer keeps in flight (#299): the number of
/// logical cores the platform reports (`navigator.hardwareConcurrency` in a
/// browser), or [`FALLBACK_COMPILE_LIMIT`] when it reports none, zero or a
/// value that is not a positive whole number.
pub(crate) fn compile_limit(hardware_concurrency: Option<f64>) -> usize {
    match hardware_concurrency {
        Some(cores) if cores.is_finite() && cores >= 1.0 => cores as usize,
        _ => FALLBACK_COMPILE_LIMIT,
    }
}

/// Why the browser rejected a compile: the `reason` of the
/// `GPUPipelineError` it rejected with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RejectReason {
    /// `"validation"`: the descriptor or shader is invalid for the device.
    Validation,
    /// `"internal"`: the implementation failed, for example out of memory.
    Internal,
    /// The rejection was not a `GPUPipelineError` (a call that threw).
    Other,
}

impl RejectReason {
    /// The reason a `GPUPipelineError` carries, by its `reason` string.
    pub(crate) fn from_pipeline_error(reason: Option<&str>) -> Self {
        match reason {
            Some("validation") => Self::Validation,
            Some("internal") => Self::Internal,
            _ => Self::Other,
        }
    }
}

/// How one scheduled compile ended. Each compile ends in exactly one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CompileOutcome {
    /// The browser resolved the compile: a synchronous creation of the same
    /// pipeline is a cache hit.
    Ready,
    /// The browser rejected the compile. A draw that needs the pipeline
    /// creates it, which reports the error through the usual path.
    Rejected {
        reason: RejectReason,
        message: String,
    },
    /// The device was lost before the compile settled. The renderer's
    /// device-loss handling owns what follows.
    DeviceLost,
    /// Not issued: the renderer moved to the other shader set first, and
    /// the compile at index `by` of that set replaces it.
    Superseded { by: usize },
    /// Not issued: the renderer moved to the other shader set first, and
    /// that set has no compile with this label.
    Retired,
    /// The queue was dropped (its renderer is gone) before the compile
    /// settled.
    Cancelled,
}

/// Where one pipeline's compile is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CompileState {
    /// Not issued yet.
    Waiting,
    /// Issued, not settled.
    Compiling,
    /// Ended, for good.
    Settled(CompileOutcome),
}

/// What a frame may do with the pipelines that carry one label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LabelReadiness {
    /// No compile of this label is scheduled: the draw creates it as it
    /// always did.
    Untracked,
    /// Every compile of the label is ready: draw.
    Ready,
    /// At least one is still waiting or compiling: leave the series out.
    Pending,
    /// Every compile has ended and at least one did not end ready
    /// (rejected, device lost, retired or cancelled): draw, so the
    /// synchronous creation reports the error through the usual path. A
    /// frame never waits on it.
    Ended,
}

/// One scheduled compile: one pipeline descriptor of one shader set.
#[derive(Debug)]
pub(crate) struct ScheduledCompile {
    pub(crate) set: ShaderSet,
    /// The pipeline label (`figgy line columnar pipeline`).
    pub(crate) label: String,
    /// The progress stage it reports under (`precise line`).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) stage: &'static str,
    pub(crate) state: CompileState,
}

/// What a waiter learns about one compile it waits for, following a
/// superseded compile to the one that replaced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WaitStep {
    /// Still waiting or compiling (the index is the compile, after
    /// following any supersession).
    Pending(usize),
    /// Ended with this outcome (never [`CompileOutcome::Superseded`]).
    Ended(CompileOutcome),
}

/// The compiles of one renderer, in the order they are to be issued.
#[derive(Debug)]
pub(crate) struct CompileSchedule {
    limit: usize,
    in_flight: usize,
    /// The most compiles ever in flight at once.
    peak: usize,
    /// Every compile ever scheduled, by index. The order of issue is
    /// `waiting`'s.
    compiles: Vec<ScheduledCompile>,
    /// Indices into `compiles` not issued yet, next first.
    waiting: VecDeque<usize>,
    /// Compiles that have ended so far, for a frame that waits on one.
    settled: u64,
    /// Whether the device was lost: nothing is issued after it.
    lost: bool,
}

impl CompileSchedule {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit: limit.max(1),
            in_flight: 0,
            peak: 0,
            compiles: Vec::new(),
            waiting: VecDeque::new(),
            settled: 0,
            lost: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    #[cfg(test)]
    pub(crate) fn in_flight(&self) -> usize {
        self.in_flight
    }

    /// The most compiles that were ever in flight at once.
    #[cfg(test)]
    pub(crate) fn peak_in_flight(&self) -> usize {
        self.peak
    }

    /// Compiles that have ended so far. It only grows.
    pub(crate) fn settled(&self) -> u64 {
        self.settled
    }

    /// Whether any compile is waiting or in flight.
    pub(crate) fn busy(&self) -> bool {
        self.in_flight > 0 || !self.waiting.is_empty()
    }

    pub(crate) fn compile(&self, index: usize) -> &ScheduledCompile {
        &self.compiles[index]
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn len(&self) -> usize {
        self.compiles.len()
    }

    /// Schedule one compile after every waiting one; its index. After the
    /// device was lost it ends at once as [`CompileOutcome::DeviceLost`].
    pub(crate) fn push(&mut self, set: ShaderSet, label: String, stage: &'static str) -> usize {
        let index = self.compiles.len();
        let state = if self.lost {
            self.settled += 1;
            CompileState::Settled(CompileOutcome::DeviceLost)
        } else {
            self.waiting.push_back(index);
            CompileState::Waiting
        };
        self.compiles.push(ScheduledCompile {
            set,
            label,
            stage,
            state,
        });
        index
    }

    /// The next compile to issue, marked compiling, while fewer than the
    /// limit are in flight; `None` when the limit is reached or nothing
    /// waits.
    pub(crate) fn issue_next(&mut self) -> Option<usize> {
        if self.in_flight >= self.limit {
            return None;
        }
        let index = self.waiting.pop_front()?;
        self.compiles[index].state = CompileState::Compiling;
        self.in_flight += 1;
        self.peak = self.peak.max(self.in_flight);
        Some(index)
    }

    /// Record how issued compile `index` ended. A compile that already
    /// ended (device loss got there first) keeps its outcome.
    pub(crate) fn settle(&mut self, index: usize, outcome: CompileOutcome) {
        let compile = &mut self.compiles[index];
        if compile.state != CompileState::Compiling {
            return;
        }
        compile.state = CompileState::Settled(outcome);
        self.in_flight -= 1;
        self.settled += 1;
    }

    /// End every compile not ended yet with `outcome`, and issue nothing
    /// more.
    fn end_all(&mut self, outcome: &CompileOutcome) {
        for compile in &mut self.compiles {
            if matches!(
                compile.state,
                CompileState::Waiting | CompileState::Compiling
            ) {
                compile.state = CompileState::Settled(outcome.clone());
                self.settled += 1;
            }
        }
        self.waiting.clear();
        self.in_flight = 0;
    }

    /// The device was lost: every compile not ended yet ends as
    /// [`CompileOutcome::DeviceLost`], and any scheduled later does too.
    pub(crate) fn device_lost(&mut self) {
        self.lost = true;
        self.end_all(&CompileOutcome::DeviceLost);
    }

    /// The queue is going away: every compile not ended yet ends as
    /// [`CompileOutcome::Cancelled`].
    pub(crate) fn cancel_all(&mut self) {
        self.end_all(&CompileOutcome::Cancelled);
    }

    /// Move every waiting compile of `set` labelled `label` ahead of all
    /// other waiting ones; an issued one is left as it is. Returns whether
    /// one moved.
    pub(crate) fn prioritize(&mut self, set: ShaderSet, label: &str) -> bool {
        let mut moved = 0;
        let mut i = 0;
        while i < self.waiting.len() {
            let index = self.waiting[i];
            let compile = &self.compiles[index];
            if compile.set == set && compile.label == label && i >= moved {
                self.waiting.remove(i);
                self.waiting.insert(moved, index);
                moved += 1;
            }
            i += 1;
        }
        moved > 0
    }

    /// The renderer moved from `from` to `to`, whose compiles are already
    /// scheduled: every compile of `from` not issued yet ends as superseded
    /// by the compile of `to` with its label, or as retired when `to` has
    /// none. Issued ones run to the end.
    pub(crate) fn supersede(&mut self, from: ShaderSet, to: ShaderSet) {
        let compiles = &mut self.compiles;
        let mut ended = 0;
        self.waiting.retain(|&index| {
            if compiles[index].set != from {
                return true;
            }
            let label = &compiles[index].label;
            let outcome = match compiles
                .iter()
                .position(|other| other.set == to && other.label == *label)
            {
                Some(by) => CompileOutcome::Superseded { by },
                None => CompileOutcome::Retired,
            };
            compiles[index].state = CompileState::Settled(outcome);
            ended += 1;
            false
        });
        self.settled += ended;
    }

    /// Where a waiter stands with compile `index`, following a superseded
    /// compile to the one that replaced it.
    pub(crate) fn wait_step(&self, mut index: usize) -> WaitStep {
        // A supersession points at a compile scheduled later, so the chain
        // only moves forward and ends.
        loop {
            match &self.compiles[index].state {
                CompileState::Waiting | CompileState::Compiling => return WaitStep::Pending(index),
                CompileState::Settled(CompileOutcome::Superseded { by }) if *by > index => {
                    index = *by;
                }
                CompileState::Settled(CompileOutcome::Superseded { .. }) => {
                    return WaitStep::Ended(CompileOutcome::Retired);
                }
                CompileState::Settled(outcome) => return WaitStep::Ended(outcome.clone()),
            }
        }
    }

    /// The compiles of `set` as `(total, ready, compiling, failed)`, a
    /// waiting compile counted as compiling and every other ending but a
    /// supersession or retirement counted as failed.
    pub(crate) fn counts(&self, set: ShaderSet) -> (usize, usize, usize, usize) {
        let (mut total, mut ready, mut compiling, mut failed) = (0, 0, 0, 0);
        for compile in self.compiles.iter().filter(|compile| compile.set == set) {
            match &compile.state {
                CompileState::Settled(CompileOutcome::Ready) => ready += 1,
                CompileState::Settled(
                    CompileOutcome::Superseded { .. } | CompileOutcome::Retired,
                ) => continue,
                CompileState::Settled(_) => failed += 1,
                CompileState::Waiting | CompileState::Compiling => compiling += 1,
            }
            total += 1;
        }
        (total, ready, compiling, failed)
    }

    /// What a frame may do with the pipelines of `set` labelled `label`.
    pub(crate) fn readiness(&self, set: ShaderSet, label: &str) -> LabelReadiness {
        let mut tracked = false;
        let mut all_ready = true;
        for compile in &self.compiles {
            if compile.set != set || compile.label != label {
                continue;
            }
            tracked = true;
            match &compile.state {
                CompileState::Waiting | CompileState::Compiling => {
                    return LabelReadiness::Pending;
                }
                CompileState::Settled(CompileOutcome::Ready) => {}
                CompileState::Settled(_) => all_ready = false,
            }
        }
        match (tracked, all_ready) {
            (false, _) => LabelReadiness::Untracked,
            (true, true) => LabelReadiness::Ready,
            (true, false) => LabelReadiness::Ended,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "figgy line columnar pipeline";
    const FIELD: &str = "figgy field columnar pipeline";

    fn schedule(limit: usize, labels: &[&str]) -> CompileSchedule {
        let mut schedule = CompileSchedule::new(limit);
        for label in labels {
            schedule.push(ShaderSet::Basic, (*label).to_owned(), "stage");
        }
        schedule
    }

    fn rejected(reason: RejectReason) -> CompileOutcome {
        CompileOutcome::Rejected {
            reason,
            message: "rejected".to_owned(),
        }
    }

    /// The core count when it is reported, the fallback when it is not.
    #[test]
    fn the_limit_is_the_core_count_or_the_fallback() {
        assert_eq!(compile_limit(Some(8.0)), 8);
        assert_eq!(compile_limit(Some(1.0)), 1);
        assert_eq!(compile_limit(Some(12.9)), 12);
        for missing in [
            None,
            Some(0.0),
            Some(-2.0),
            Some(0.5),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            assert_eq!(
                compile_limit(missing),
                FALLBACK_COMPILE_LIMIT,
                "{missing:?}"
            );
        }
        assert_eq!(CompileSchedule::new(0).limit(), 1);
    }

    /// A `GPUPipelineError` reason maps to its kind; anything else is other.
    #[test]
    fn a_rejection_is_classified_by_its_pipeline_error_reason() {
        assert_eq!(
            RejectReason::from_pipeline_error(Some("validation")),
            RejectReason::Validation
        );
        assert_eq!(
            RejectReason::from_pipeline_error(Some("internal")),
            RejectReason::Internal
        );
        for other in [None, Some(""), Some("Validation"), Some("oom")] {
            assert_eq!(
                RejectReason::from_pipeline_error(other),
                RejectReason::Other
            );
        }
    }

    /// Never more than the limit in flight, however the compiles settle,
    /// and every compile ends.
    #[test]
    fn the_in_flight_count_never_exceeds_the_limit() {
        let labels: Vec<String> = (0..40).map(|i| format!("pipeline {i}")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        for limit in [1, 3, 4, 16] {
            let mut s = schedule(limit, &refs);
            let mut flying = Vec::new();
            let mut step = 0usize;
            loop {
                while let Some(index) = s.issue_next() {
                    flying.push(index);
                }
                assert!(s.in_flight() <= limit);
                assert_eq!(s.in_flight(), flying.len());
                if flying.is_empty() {
                    break;
                }
                // Settle an arbitrary one, sometimes as a rejection.
                let index = flying.remove(step % flying.len());
                let outcome = match step % 7 {
                    0 => rejected(RejectReason::Validation),
                    3 => rejected(RejectReason::Internal),
                    _ => CompileOutcome::Ready,
                };
                s.settle(index, outcome);
                step += 1;
            }
            assert!(!s.busy());
            assert_eq!(s.peak_in_flight(), limit.min(labels.len()));
            assert_eq!(s.settled(), labels.len() as u64);
            let (total, ready, compiling, failed) = s.counts(ShaderSet::Basic);
            assert_eq!((total, compiling), (labels.len(), 0));
            assert_eq!(ready + failed, total);
            assert!(failed > 0);
        }
    }

    /// A pipeline a screen asks for while others compile is issued before
    /// every background one still waiting; issued ones are not touched.
    #[test]
    fn a_requested_pipeline_is_issued_before_the_background_ones() {
        let mut s = schedule(2, &["a", "b", LINE, "c", "d", FIELD, "e"]);
        assert_eq!(s.issue_next(), Some(0));
        assert_eq!(s.issue_next(), Some(1));
        assert_eq!(s.issue_next(), None);
        assert!(s.prioritize(ShaderSet::Basic, FIELD));
        assert!(!s.prioritize(ShaderSet::Full, FIELD));
        // A label already issued does not move.
        assert!(!s.prioritize(ShaderSet::Basic, "a"));
        s.settle(0, CompileOutcome::Ready);
        assert_eq!(s.issue_next(), Some(5), "the field is next");
        s.settle(1, CompileOutcome::Ready);
        assert_eq!(s.issue_next(), Some(2));
        assert_eq!(
            s.readiness(ShaderSet::Basic, FIELD),
            LabelReadiness::Pending
        );
        s.settle(5, CompileOutcome::Ready);
        assert_eq!(s.readiness(ShaderSet::Basic, FIELD), LabelReadiness::Ready);
        assert_eq!(
            s.readiness(ShaderSet::Basic, "nothing"),
            LabelReadiness::Untracked
        );
        assert_eq!(
            s.readiness(ShaderSet::Full, FIELD),
            LabelReadiness::Untracked
        );
    }

    /// A label is ready only when every compile carrying it is; one that
    /// ended otherwise makes it ended, which a frame draws and never waits
    /// on.
    #[test]
    fn a_label_waits_until_all_its_compiles_ended() {
        let mut s = schedule(4, &[LINE, LINE]);
        let (a, b) = (s.issue_next().unwrap(), s.issue_next().unwrap());
        s.settle(a, CompileOutcome::Ready);
        assert_eq!(s.readiness(ShaderSet::Basic, LINE), LabelReadiness::Pending);
        s.settle(b, rejected(RejectReason::Internal));
        assert_eq!(s.readiness(ShaderSet::Basic, LINE), LabelReadiness::Ended);
        assert_eq!(
            s.wait_step(b),
            WaitStep::Ended(rejected(RejectReason::Internal))
        );
        // A second settle of the same compile changes nothing.
        s.settle(b, CompileOutcome::Ready);
        assert_eq!(s.settled(), 2);
        assert_eq!(s.in_flight(), 0);
    }

    /// Moving to the full set supersedes the basic compiles not issued yet
    /// with their full twins, retires one without a twin, and keeps the
    /// issued one running; a waiter follows the supersession.
    #[test]
    fn an_upgrade_supersedes_the_waiting_basic_compiles() {
        let mut s = schedule(1, &["a", "b", "c"]);
        let first = s.issue_next().unwrap();
        let full_a = s.push(ShaderSet::Full, "a".to_owned(), "stage");
        let full_b = s.push(ShaderSet::Full, "b".to_owned(), "stage");
        s.supersede(ShaderSet::Basic, ShaderSet::Full);
        assert_eq!(
            s.compile(1).state,
            CompileState::Settled(CompileOutcome::Superseded { by: full_b })
        );
        assert_eq!(
            s.compile(2).state,
            CompileState::Settled(CompileOutcome::Retired)
        );
        assert_eq!(s.wait_step(1), WaitStep::Pending(full_b));
        s.settle(first, CompileOutcome::Ready);
        assert_eq!(s.issue_next(), Some(full_a));
        s.settle(full_a, CompileOutcome::Ready);
        assert_eq!(s.issue_next(), Some(full_b));
        assert_eq!(s.issue_next(), None);
        s.settle(full_b, CompileOutcome::Ready);
        assert_eq!(s.wait_step(1), WaitStep::Ended(CompileOutcome::Ready));
        assert_eq!(s.wait_step(2), WaitStep::Ended(CompileOutcome::Retired));
        assert_eq!(s.readiness(ShaderSet::Basic, "a"), LabelReadiness::Ready);
        assert_eq!(s.counts(ShaderSet::Basic), (1, 1, 0, 0));
        assert_eq!(s.counts(ShaderSet::Full), (2, 2, 0, 0));
        assert!(!s.busy());
    }

    /// Device loss ends every compile not ended yet, issued or waiting, and
    /// any scheduled after it; a late browser settle changes nothing.
    #[test]
    fn device_loss_ends_every_open_compile() {
        let mut s = schedule(1, &["a", "b"]);
        let a = s.issue_next().unwrap();
        s.device_lost();
        assert!(!s.busy());
        for index in [a, 1] {
            assert_eq!(
                s.wait_step(index),
                WaitStep::Ended(CompileOutcome::DeviceLost)
            );
        }
        s.settle(a, CompileOutcome::Ready);
        assert_eq!(s.wait_step(a), WaitStep::Ended(CompileOutcome::DeviceLost));
        let late = s.push(ShaderSet::Full, "a".to_owned(), "stage");
        assert_eq!(
            s.wait_step(late),
            WaitStep::Ended(CompileOutcome::DeviceLost)
        );
        assert_eq!(s.issue_next(), None);
        assert_eq!(s.readiness(ShaderSet::Basic, "a"), LabelReadiness::Ended);
    }

    /// Cancelling ends every compile not ended yet and keeps the ones that
    /// ended.
    #[test]
    fn cancelling_ends_the_open_compiles_only() {
        let mut s = schedule(2, &["a", "b", "c"]);
        let a = s.issue_next().unwrap();
        let b = s.issue_next().unwrap();
        s.settle(a, CompileOutcome::Ready);
        s.cancel_all();
        assert!(!s.busy());
        assert_eq!(s.wait_step(a), WaitStep::Ended(CompileOutcome::Ready));
        for index in [b, 2] {
            assert_eq!(
                s.wait_step(index),
                WaitStep::Ended(CompileOutcome::Cancelled)
            );
        }
        assert_eq!(s.settled(), 3);
    }
}
