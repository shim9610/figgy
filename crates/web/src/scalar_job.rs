use std::{
    cell::RefCell,
    future::poll_fn,
    rc::Rc,
    task::{Poll, Waker},
};

use renderer::FitExtent;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SeriesFitExtent {
    pub(crate) x: FitExtent,
    pub(crate) y: FitExtent,
}

type SeriesExtentResult = Result<Option<SeriesFitExtent>, String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SeriesExtentStatus {
    Pending,
    Succeeded,
    RetryableFailed,
    TerminalFailed,
}

enum SeriesExtentState {
    Pending { waiters: Vec<Waker> },
    Succeeded(Option<SeriesFitExtent>),
    RetryableFailed(String),
    TerminalFailed(String),
}

/// A detached GPU job that retains only its eventual per-axis series extent.
///
/// The column values and readback buffer stay owned by WebGPU/the query ticket;
/// this shared state contains no per-value data and needs no thread lock on the
/// single-threaded wasm host.
pub(crate) struct SeriesExtentJob {
    state: RefCell<SeriesExtentState>,
}

impl SeriesExtentJob {
    pub(crate) fn pending() -> Rc<Self> {
        Rc::new(Self {
            state: RefCell::new(SeriesExtentState::Pending {
                waiters: Vec::new(),
            }),
        })
    }

    pub(crate) fn status(&self) -> SeriesExtentStatus {
        match &*self.state.borrow() {
            SeriesExtentState::Pending { .. } => SeriesExtentStatus::Pending,
            SeriesExtentState::Succeeded(_) => SeriesExtentStatus::Succeeded,
            SeriesExtentState::RetryableFailed(_) => SeriesExtentStatus::RetryableFailed,
            SeriesExtentState::TerminalFailed(_) => SeriesExtentStatus::TerminalFailed,
        }
    }

    pub(crate) fn complete_success(&self, extent: Option<SeriesFitExtent>) {
        self.complete(SeriesExtentState::Succeeded(extent));
    }

    pub(crate) fn complete_retryable_failure(&self, error: String) {
        self.complete(SeriesExtentState::RetryableFailed(error));
    }

    pub(crate) fn complete_terminal_failure(&self, error: String) {
        self.complete(SeriesExtentState::TerminalFailed(error));
    }

    fn complete(&self, completed: SeriesExtentState) {
        let waiters = {
            let mut state = self.state.borrow_mut();
            let SeriesExtentState::Pending { waiters } = &mut *state else {
                return;
            };
            let waiters = std::mem::take(waiters);
            *state = completed;
            waiters
        };
        for waiter in waiters {
            waiter.wake();
        }
    }

    #[cfg(test)]
    pub(crate) fn result(&self) -> Option<SeriesExtentResult> {
        match &*self.state.borrow() {
            SeriesExtentState::Pending { .. } => None,
            SeriesExtentState::Succeeded(extent) => Some(Ok(*extent)),
            SeriesExtentState::RetryableFailed(error)
            | SeriesExtentState::TerminalFailed(error) => Some(Err(error.clone())),
        }
    }

    pub(crate) async fn wait(self: Rc<Self>) -> SeriesExtentResult {
        poll_fn(move |cx| {
            let mut state = self.state.borrow_mut();
            match &mut *state {
                SeriesExtentState::Pending { waiters } => {
                    if !waiters.iter().any(|waiter| waiter.will_wake(cx.waker())) {
                        waiters.push(cx.waker().clone());
                    }
                    Poll::Pending
                }
                SeriesExtentState::Succeeded(extent) => Poll::Ready(Ok(*extent)),
                SeriesExtentState::RetryableFailed(error)
                | SeriesExtentState::TerminalFailed(error) => Poll::Ready(Err(error.clone())),
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{future::Future, task::Context};

    #[test]
    fn new_job_is_pending() {
        let job = SeriesExtentJob::pending();
        assert_eq!(job.status(), SeriesExtentStatus::Pending);
        assert!(job.result().is_none());
    }

    #[test]
    fn successful_axis_pair_is_retained() {
        let job = SeriesExtentJob::pending();
        let extent = SeriesFitExtent {
            x: FitExtent {
                min: -2.0,
                max: 7.5,
                min_positive: Some(0.25),
            },
            y: FitExtent {
                min: -9.0,
                max: 3.0,
                min_positive: Some(0.5),
            },
        };

        job.complete_success(Some(extent));

        assert_eq!(job.status(), SeriesExtentStatus::Succeeded);
        assert_eq!(job.result(), Some(Ok(Some(extent))));
    }

    #[test]
    fn successful_empty_extent_is_retained() {
        let job = SeriesExtentJob::pending();

        job.complete_success(None);

        assert_eq!(job.status(), SeriesExtentStatus::Succeeded);
        assert_eq!(job.result(), Some(Ok(None)));
    }

    #[test]
    fn retryable_failure_is_explicit() {
        let job = SeriesExtentJob::pending();

        job.complete_retryable_failure("readback failed".to_string());

        assert_eq!(job.status(), SeriesExtentStatus::RetryableFailed);
        assert_eq!(job.result(), Some(Err("readback failed".to_string())));
    }

    #[test]
    fn terminal_failure_is_explicit() {
        let job = SeriesExtentJob::pending();

        job.complete_terminal_failure("invalid binding".to_string());

        assert_eq!(job.status(), SeriesExtentStatus::TerminalFailed);
        assert_eq!(job.result(), Some(Err("invalid binding".to_string())));
    }

    #[test]
    fn first_completion_wins() {
        let job = SeriesExtentJob::pending();
        job.complete_retryable_failure("first".to_string());
        job.complete_success(None);
        job.complete_terminal_failure("third".to_string());

        assert_eq!(job.status(), SeriesExtentStatus::RetryableFailed);
        assert_eq!(job.result(), Some(Err("first".to_string())));
    }

    #[test]
    fn wait_transitions_from_pending_to_ready() {
        let job = SeriesExtentJob::pending();
        let mut future = Box::pin(Rc::clone(&job).wait());
        let mut context = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut context).is_pending());

        job.complete_success(None);
        assert_eq!(future.as_mut().poll(&mut context), Poll::Ready(Ok(None)));
    }
}
