use std::{
    cell::RefCell,
    future::poll_fn,
    rc::Rc,
    task::{Poll, Waker},
};

use renderer::FitExtent;

type ScalarExtentResult = Result<Option<FitExtent>, String>;

#[derive(Default)]
struct ScalarExtentState {
    result: Option<ScalarExtentResult>,
    waiters: Vec<Waker>,
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
            state: RefCell::new(ScalarExtentState::default()),
        })
    }

    pub(crate) fn complete(&self, result: ScalarExtentResult) {
        let waiters = {
            let mut state = self.state.borrow_mut();
            if state.result.is_some() {
                return;
            }
            state.result = Some(result);
            std::mem::take(&mut state.waiters)
        };
        for waiter in waiters {
            waiter.wake();
        }
    }

    #[cfg(test)]
    pub(crate) fn result(&self) -> Option<ScalarExtentResult> {
        self.state.borrow().result.clone()
    }

    pub(crate) async fn wait(self: Rc<Self>) -> ScalarExtentResult {
        poll_fn(move |cx| {
            let mut state = self.state.borrow_mut();
            if let Some(result) = &state.result {
                return Poll::Ready(result.clone());
            }
            if !state
                .waiters
                .iter()
                .any(|waiter| waiter.will_wake(cx.waker()))
            {
                state.waiters.push(cx.waker().clone());
            }
            Poll::Pending
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{future::Future, task::Context};

    #[test]
    fn completion_retains_only_the_scalar_result() {
        let job = ScalarExtentJob::pending();
        assert!(job.result().is_none());

        let extent = FitExtent {
            min: -2.0,
            max: 7.5,
            min_positive: Some(0.25),
        };
        job.complete(Ok(Some(extent)));
        assert_eq!(job.result(), Some(Ok(Some(extent))));
    }

    #[test]
    fn first_completion_wins() {
        let job = ScalarExtentJob::pending();
        job.complete(Err("first".to_string()));
        job.complete(Ok(None));
        assert_eq!(job.result(), Some(Err("first".to_string())));
    }

    #[test]
    fn wait_transitions_from_pending_to_ready() {
        let job = ScalarExtentJob::pending();
        let mut future = Box::pin(Rc::clone(&job).wait());
        let mut context = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut context).is_pending());

        job.complete(Ok(None));
        assert_eq!(future.as_mut().poll(&mut context), Poll::Ready(Ok(None)));
    }
}
