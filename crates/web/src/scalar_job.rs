use std::{
    cell::RefCell,
    future::poll_fn,
    rc::Rc,
    task::{Poll, Waker},
};

use renderer::FitExtent;

type ScalarExtentResult = Result<Option<FitExtent>, String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScalarExtentStatus {
    Pending,
    Succeeded,
    RetryableFailed,
    TerminalFailed,
}

enum ScalarExtentState {
    Pending { waiters: Vec<Waker> },
    Succeeded(Option<FitExtent>),
    RetryableFailed(String),
    TerminalFailed(String),
}

/// A detached GPU job that retains only its eventual scalar extent.
///
/// The column values and readback buffer stay owned by WebGPU/the query ticket;
/// this shared state contains no per-value data and needs no thread lock on the
/// single-threaded wasm host.
pub(crate) struct ScalarExtentJob {
    state: RefCell<ScalarExtentState>,
}

impl ScalarExtentJob {
    pub(crate) fn pending() -> Rc<Self> {
        Rc::new(Self {
            state: RefCell::new(ScalarExtentState::Pending {
                waiters: Vec::new(),
            }),
        })
    }

    pub(crate) fn status(&self) -> ScalarExtentStatus {
        match &*self.state.borrow() {
            ScalarExtentState::Pending { .. } => ScalarExtentStatus::Pending,
            ScalarExtentState::Succeeded(_) => ScalarExtentStatus::Succeeded,
            ScalarExtentState::RetryableFailed(_) => ScalarExtentStatus::RetryableFailed,
            ScalarExtentState::TerminalFailed(_) => ScalarExtentStatus::TerminalFailed,
        }
    }

    pub(crate) fn complete_success(&self, extent: Option<FitExtent>) {
        self.complete(ScalarExtentState::Succeeded(extent));
    }

    pub(crate) fn complete_retryable_failure(&self, error: String) {
        self.complete(ScalarExtentState::RetryableFailed(error));
    }

    pub(crate) fn complete_terminal_failure(&self, error: String) {
        self.complete(ScalarExtentState::TerminalFailed(error));
    }

    fn complete(&self, completed: ScalarExtentState) {
        let waiters = {
            let mut state = self.state.borrow_mut();
            let ScalarExtentState::Pending { waiters } = &mut *state else {
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
    pub(crate) fn result(&self) -> Option<ScalarExtentResult> {
        match &*self.state.borrow() {
            ScalarExtentState::Pending { .. } => None,
            ScalarExtentState::Succeeded(extent) => Some(Ok(*extent)),
            ScalarExtentState::RetryableFailed(error)
            | ScalarExtentState::TerminalFailed(error) => Some(Err(error.clone())),
        }
    }

    pub(crate) async fn wait(self: Rc<Self>) -> ScalarExtentResult {
        poll_fn(move |cx| {
            let mut state = self.state.borrow_mut();
            match &mut *state {
                ScalarExtentState::Pending { waiters } => {
                    if !waiters.iter().any(|waiter| waiter.will_wake(cx.waker())) {
                        waiters.push(cx.waker().clone());
                    }
                    Poll::Pending
                }
                ScalarExtentState::Succeeded(extent) => Poll::Ready(Ok(*extent)),
                ScalarExtentState::RetryableFailed(error)
                | ScalarExtentState::TerminalFailed(error) => Poll::Ready(Err(error.clone())),
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
        let job = ScalarExtentJob::pending();
        assert_eq!(job.status(), ScalarExtentStatus::Pending);
        assert!(job.result().is_none());
    }

    #[test]
    fn successful_extent_is_retained() {
        let job = ScalarExtentJob::pending();
        let extent = FitExtent {
            min: -2.0,
            max: 7.5,
            min_positive: Some(0.25),
        };

        job.complete_success(Some(extent));

        assert_eq!(job.status(), ScalarExtentStatus::Succeeded);
        assert_eq!(job.result(), Some(Ok(Some(extent))));
    }

    #[test]
    fn successful_empty_extent_is_retained() {
        let job = ScalarExtentJob::pending();

        job.complete_success(None);

        assert_eq!(job.status(), ScalarExtentStatus::Succeeded);
        assert_eq!(job.result(), Some(Ok(None)));
    }

    #[test]
    fn retryable_failure_is_explicit() {
        let job = ScalarExtentJob::pending();

        job.complete_retryable_failure("readback failed".to_string());

        assert_eq!(job.status(), ScalarExtentStatus::RetryableFailed);
        assert_eq!(job.result(), Some(Err("readback failed".to_string())));
    }

    #[test]
    fn terminal_failure_is_explicit() {
        let job = ScalarExtentJob::pending();

        job.complete_terminal_failure("invalid binding".to_string());

        assert_eq!(job.status(), ScalarExtentStatus::TerminalFailed);
        assert_eq!(job.result(), Some(Err("invalid binding".to_string())));
    }

    #[test]
    fn first_completion_wins() {
        let job = ScalarExtentJob::pending();
        job.complete_retryable_failure("first".to_string());
        job.complete_success(None);
        job.complete_terminal_failure("third".to_string());

        assert_eq!(job.status(), ScalarExtentStatus::RetryableFailed);
        assert_eq!(job.result(), Some(Err("first".to_string())));
    }

    #[test]
    fn wait_transitions_from_pending_to_ready() {
        let job = ScalarExtentJob::pending();
        let mut future = Box::pin(Rc::clone(&job).wait());
        let mut context = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut context).is_pending());

        job.complete_success(None);
        assert_eq!(future.as_mut().poll(&mut context), Poll::Ready(Ok(None)));
    }
}
