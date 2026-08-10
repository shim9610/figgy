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
}
