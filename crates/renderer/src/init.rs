//! Timestamp-free renderer initialization diagnostics.
//!
//! Callers own the clock so the same events can be measured with a native
//! monotonic clock or the browser's `performance.now()`.

/// Schema version for [`InitEvent`].
pub const INIT_EVENT_SCHEMA_VERSION: u32 = 1;

/// Boundary emitted for an initialization stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitPhase {
    Started,
    Finished,
}

/// A stable, timestamp-free initialization event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitEvent {
    pub schema_version: u32,
    pub scope: &'static str,
    pub stage: &'static str,
    pub phase: InitPhase,
}

impl InitEvent {
    pub const fn new(scope: &'static str, stage: &'static str, phase: InitPhase) -> Self {
        Self {
            schema_version: INIT_EVENT_SCHEMA_VERSION,
            scope,
            stage,
            phase,
        }
    }
}

pub(crate) fn started(
    observer: &mut dyn FnMut(InitEvent),
    scope: &'static str,
    stage: &'static str,
) {
    observer(InitEvent::new(scope, stage, InitPhase::Started));
}

pub(crate) fn finished(
    observer: &mut dyn FnMut(InitEvent),
    scope: &'static str,
    stage: &'static str,
) {
    observer(InitEvent::new(scope, stage, InitPhase::Finished));
}

pub(crate) fn observe_value<T>(
    observer: &mut dyn FnMut(InitEvent),
    scope: &'static str,
    stage: &'static str,
    operation: impl FnOnce() -> T,
) -> T {
    started(observer, scope, stage);
    let value = operation();
    finished(observer, scope, stage);
    value
}

pub(crate) fn observe_result<T, E>(
    observer: &mut dyn FnMut(InitEvent),
    scope: &'static str,
    stage: &'static str,
    operation: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    started(observer, scope, stage);
    let value = operation()?;
    finished(observer, scope, stage);
    Ok(value)
}

/// Yield one animation frame on wasm so a host loading bar can paint.
/// Native init has no event loop to return to, so this is a no-op there.
pub(crate) async fn yield_init_frame() {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(window) = web_sys::window() else {
            return;
        };
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            if window.request_animation_frame(&resolve).is_err() {
                let _ = resolve.call0(&wasm_bindgen::JsValue::UNDEFINED);
            }
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}

pub(crate) async fn observe_value_async<T>(
    observer: &mut dyn FnMut(InitEvent),
    scope: &'static str,
    stage: &'static str,
    operation: impl FnOnce() -> T,
) -> T {
    started(observer, scope, stage);
    let value = operation();
    finished(observer, scope, stage);
    yield_init_frame().await;
    value
}

pub(crate) async fn observe_result_async<T, E>(
    observer: &mut dyn FnMut(InitEvent),
    scope: &'static str,
    stage: &'static str,
    operation: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    started(observer, scope, stage);
    let value = operation()?;
    finished(observer, scope, stage);
    yield_init_frame().await;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_stage_has_a_complete_pair() {
        let mut events = Vec::new();
        let value = observe_value(&mut |event| events.push(event), "test", "value", || 42);

        assert_eq!(value, 42);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].phase, InitPhase::Started);
        assert_eq!(events[1].phase, InitPhase::Finished);
    }

    #[test]
    fn failed_stage_keeps_the_last_started_boundary() {
        let mut events = Vec::new();
        let result: Result<(), &'static str> =
            observe_result(&mut |event| events.push(event), "test", "result", || {
                Err("expected")
            });

        assert_eq!(result, Err("expected"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].phase, InitPhase::Started);
    }

    #[test]
    fn panicking_stage_keeps_the_last_started_boundary() {
        let mut events = Vec::new();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            observe_value(&mut |event| events.push(event), "test", "panic", || {
                panic!("expected")
            });
        }));

        assert!(panic.is_err());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].phase, InitPhase::Started);
    }

    #[test]
    fn async_observe_matches_sync_pairs_when_yield_is_noop() {
        let mut events = Vec::new();
        let value = pollster::block_on(observe_value_async(
            &mut |event| events.push(event),
            "test",
            "async",
            || 7,
        ));
        assert_eq!(value, 7);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].phase, InitPhase::Started);
        assert_eq!(events[1].phase, InitPhase::Finished);
    }
}
