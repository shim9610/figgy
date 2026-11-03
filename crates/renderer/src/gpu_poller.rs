//! Native completion servicing for asynchronous GPU waits (#160).
//!
//! wgpu runs a buffer map's callback, and `on_submitted_work_done`'s, only
//! inside `Queue::submit`, `Device::poll` or `Instance::poll_all`. On native
//! nothing calls those on its own, so an `.await` on a map would never wake.
//! A [`Poller`] owns one resident thread per device bundle: while
//! asynchronous waits are pending the thread blocks in the device's poll
//! until the submitted work completes, which runs the callbacks and wakes the
//! waiting tasks; with nothing pending it parks. A blocking wait polls on the
//! caller's thread instead and needs no thread; a bundle built for blocking
//! use only has none.
//!
//! The protocol uses two counters and the thread's park token, no lock:
//!
//! - `owners` counts the [`Poller`] handles: the bundle's clones and the
//!   [`WaitMark`] of every wait not yet settled. When it reaches zero no new
//!   wait can be registered, and the thread exits once nothing is pending.
//! - `pending` counts the registered waits whose callback has not run.
//!   [`WaitMark::register`] adds one and unparks the thread;
//!   [`CallbackMark::fire`], called from the wgpu callback, removes it. Each
//!   wait counts at most once, whichever of the two runs first: a callback
//!   that runs before the registration leaves nothing to count.
//! - The thread polls while `pending` is above zero, and otherwise parks.
//!   `unpark` before `park` makes `park` return at once, so a registration
//!   between the thread's check and its park is never lost.
//!
//! `loom` checks the protocol in every interleaving of the `loom_model`
//! tests below (`cargo test --release` in `tools/loom-poller`); the GPU tests
//! check it against wgpu.

#[cfg(loom)]
use loom::{
    sync::Arc,
    sync::atomic::{AtomicU8, AtomicUsize, Ordering},
    thread,
};
#[cfg(not(loom))]
use std::{
    sync::Arc,
    sync::atomic::{AtomicU8, AtomicUsize, Ordering},
    thread,
};

/// The callback of the wait has run.
const FIRED: u8 = 1;
/// The wait is counted as pending.
const REGISTERED: u8 = 2;

/// What the poll thread drives: one blocking device poll per call.
pub(crate) trait Drive: Send + 'static {
    /// Block until the work submitted so far completes and run its
    /// callbacks. `false` reports a failed poll (a lost device): the thread
    /// then parks until the next registration or the last owner's drop.
    fn wait(&self) -> bool;
}

/// The counters a poll thread shares with its owners and waits.
struct Shared {
    owners: AtomicUsize,
    pending: AtomicUsize,
}

/// One device bundle's poll thread. Cloning adds an owner; the thread exits
/// after the last owner is dropped and no wait is pending.
pub(crate) struct Poller {
    shared: Arc<Shared>,
    thread: thread::Thread,
}

impl std::fmt::Debug for Poller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Poller")
            .field("owners", &self.shared.owners.load(Ordering::Relaxed))
            .field("pending", &self.shared.pending.load(Ordering::Relaxed))
            .finish()
    }
}

/// The OS could not start the poll thread.
#[derive(Debug)]
pub(crate) struct SpawnError(pub(crate) std::io::Error);

impl Poller {
    /// Start the poll thread over `drive`.
    pub(crate) fn start<D: Drive>(drive: D) -> Result<Self, SpawnError> {
        // Dropping the handle detaches the thread: it exits on its own.
        Self::start_thread(drive).map(|(poller, _handle)| poller)
    }

    /// [`Self::start`], also returning the thread's handle so the model
    /// tests can join it.
    fn start_thread<D: Drive>(drive: D) -> Result<(Self, thread::JoinHandle<()>), SpawnError> {
        let shared = Arc::new(Shared {
            owners: AtomicUsize::new(1),
            pending: AtomicUsize::new(0),
        });
        Self::start_on(shared, drive)
    }

    /// Start the thread over counters that already hold one owner: this
    /// poller.
    fn start_on<D: Drive>(
        shared: Arc<Shared>,
        drive: D,
    ) -> Result<(Self, thread::JoinHandle<()>), SpawnError> {
        let state = Arc::clone(&shared);
        let handle = thread::Builder::new()
            .name("figgy-gpu-poll".into())
            .spawn(move || run(&state, &drive))
            .map_err(SpawnError)?;
        let poller = Self {
            shared,
            thread: handle.thread().clone(),
        };
        Ok((poller, handle))
    }

    /// The marks of one wait: the callback's, moved into the wgpu callback,
    /// and the waiter's, kept by the readback. The waiter's mark is an owner
    /// until it is dropped.
    pub(crate) fn marks(&self) -> (CallbackMark, WaitMark) {
        let flags = Arc::new(AtomicU8::new(0));
        (
            CallbackMark::new(&self.shared, &flags),
            WaitMark::new(self, flags),
        )
    }
}

impl Clone for Poller {
    fn clone(&self) -> Self {
        self.shared.owners.fetch_add(1, Ordering::AcqRel);
        Self {
            shared: Arc::clone(&self.shared),
            thread: self.thread.clone(),
        }
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        if self.shared.owners.fetch_sub(1, Ordering::AcqRel) == 1 {
            // The last owner: wake the thread so it can exit.
            self.thread.unpark();
        }
    }
}

/// The wgpu callback's share of one wait.
pub(crate) struct CallbackMark {
    shared: Arc<Shared>,
    flags: Arc<AtomicU8>,
}

impl CallbackMark {
    fn new(shared: &Arc<Shared>, flags: &Arc<AtomicU8>) -> Self {
        Self {
            shared: Arc::clone(shared),
            flags: Arc::clone(flags),
        }
    }

    /// The callback has run: a registered wait is no longer pending. Only the
    /// first call has an effect.
    pub(crate) fn fire(&self) {
        let previous = self.flags.fetch_or(FIRED, Ordering::AcqRel);
        if previous & FIRED == 0 && previous & REGISTERED != 0 {
            self.shared.pending.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// The waiter's share of one wait.
pub(crate) struct WaitMark {
    poller: Poller,
    flags: Arc<AtomicU8>,
}

impl WaitMark {
    /// The waiter's mark holds an owner, so the thread outlives every wait
    /// that may still be registered.
    fn new(poller: &Poller, flags: Arc<AtomicU8>) -> Self {
        Self {
            poller: poller.clone(),
            flags,
        }
    }

    /// Count the wait as pending and wake the poll thread to serve it. Only
    /// the first call has an effect, and a wait whose callback has already
    /// run is not counted.
    pub(crate) fn register(&self) {
        if self.flags.load(Ordering::Acquire) != 0 {
            // Already counted, or the callback has run: nothing to wait for.
            return;
        }
        let shared = &self.poller.shared;
        shared.pending.fetch_add(1, Ordering::AcqRel);
        let previous = self.flags.fetch_or(REGISTERED, Ordering::AcqRel);
        if previous & FIRED != 0 {
            // The callback ran between the check and the count: it did not
            // remove this wait, so the count is undone here.
            shared.pending.fetch_sub(1, Ordering::AcqRel);
            return;
        }
        self.poller.thread.unpark();
    }
}

/// The poll thread: poll while waits are pending, park while none is, and
/// exit once no owner is left and nothing is pending.
fn run(shared: &Shared, drive: &impl Drive) {
    loop {
        if shared.pending.load(Ordering::Acquire) > 0 {
            if drive.wait() {
                continue;
            }
            // The poll failed (a lost device): polling again would fail at
            // once. Without an owner nothing will wake the thread, so it
            // exits; otherwise it parks until a registration or the last
            // owner's drop.
            if shared.owners.load(Ordering::Acquire) == 0 {
                return;
            }
        } else if shared.owners.load(Ordering::Acquire) == 0 {
            // Without an owner no wait can be registered and `pending` can
            // only fall. A registration made before the last owner left is
            // seen by this second read.
            if shared.pending.load(Ordering::Acquire) == 0 {
                return;
            }
            continue;
        }
        // An `unpark` since the checks above makes this return at once.
        thread::park();
    }
}

#[cfg(all(test, loom))]
mod loom_model {
    use super::*;
    use loom::sync::atomic::AtomicBool;

    /// Two waits as the device sees them: a submitted map whose callback the
    /// next poll runs.
    struct Device {
        submitted: [AtomicBool; 2],
        marks: [CallbackMark; 2],
        /// Polls in progress, to check that one thread polls at a time.
        polling: AtomicUsize,
        /// The next poll fails, as on a lost device, and runs no callback.
        fail_next: AtomicBool,
        /// Set once that failure has happened.
        failed: AtomicBool,
    }

    impl Device {
        /// Run the callback of every submitted map once.
        fn poll(&self) -> bool {
            let mut ran = false;
            for (submitted, mark) in self.submitted.iter().zip(&self.marks) {
                if submitted.swap(false, Ordering::AcqRel) {
                    mark.fire();
                    ran = true;
                }
            }
            ran
        }
    }

    struct FakeDrive(Arc<Device>);

    impl Drive for FakeDrive {
        fn wait(&self) -> bool {
            if self.0.fail_next.swap(false, Ordering::AcqRel) {
                self.0.failed.swap(true, Ordering::AcqRel);
                return false;
            }
            let others = self.0.polling.fetch_add(1, Ordering::AcqRel);
            assert_eq!(others, 0, "two threads polled at once");
            let ran = self.0.poll();
            self.0.polling.fetch_sub(1, Ordering::AcqRel);
            if !ran {
                // Nothing completed: a real poll returns at once too. The
                // count is above zero only while a registration is between
                // its count and its flag, so this ends once it moves on.
                thread::yield_now();
            }
            true
        }
    }

    /// The model's bundle: a started poller, the device it polls with the
    /// callback marks of two waits, the waiters' marks and the waits' flags.
    fn setup() -> (
        Poller,
        thread::JoinHandle<()>,
        Arc<Device>,
        [WaitMark; 2],
        [Arc<AtomicU8>; 2],
    ) {
        let shared = Arc::new(Shared {
            owners: AtomicUsize::new(1),
            pending: AtomicUsize::new(0),
        });
        let flags = [Arc::new(AtomicU8::new(0)), Arc::new(AtomicU8::new(0))];
        // The marks are made as `Poller::marks` makes them; the callbacks'
        // must exist before the device the thread polls.
        let callback = |i: usize| CallbackMark::new(&shared, &flags[i]);
        let device = Arc::new(Device {
            submitted: [AtomicBool::new(false), AtomicBool::new(false)],
            marks: [callback(0), callback(1)],
            polling: AtomicUsize::new(0),
            fail_next: AtomicBool::new(false),
            failed: AtomicBool::new(false),
        });
        let (poller, handle) = Poller::start_on(shared, FakeDrive(Arc::clone(&device))).unwrap();
        let wait = |i: usize| WaitMark::new(&poller, Arc::clone(&flags[i]));
        let waits = [wait(0), wait(1)];
        (poller, handle, device, waits, flags)
    }

    /// Submit wait `index`: the next poll runs its callback.
    fn submit(device: &Device, index: usize) {
        // A read-modify-write, like every access to `submitted`: loom orders
        // a plain store against another thread's swap only partially, and
        // would let the swap miss it forever.
        device.submitted[index].swap(true, Ordering::AcqRel);
    }

    /// After the thread has exited: every callback ran and no poll is in
    /// progress. The thread exiting at all shows nothing stayed counted, and
    /// the search ending shows it never polled in a loop while idle: an idle
    /// loop would make loom's executions unbounded.
    fn check_settled(device: &Device, marks: &[Arc<AtomicU8>]) {
        for flags in marks {
            assert_ne!(
                flags.load(Ordering::Acquire) & FIRED,
                0,
                "a callback never ran"
            );
        }
        assert_eq!(device.polling.load(Ordering::Acquire), 0);
    }

    /// Explore the model's interleavings with at most the bound's forced
    /// switches between threads.
    fn model(f: impl Fn() + Sync + Send + 'static) {
        let mut builder = loom::model::Builder::new();
        // `Builder::new` reads `LOOM_MAX_PREEMPTIONS`.
        builder.preemption_bound.get_or_insert(PREEMPTIONS);
        builder.check(f);
    }

    /// The default bound on forced switches between threads, used unless
    /// `LOOM_MAX_PREEMPTIONS` sets one: the unbounded search of these models
    /// does not finish in practice, and loom recommends a small bound, since
    /// most concurrency bugs need few preemptions to show. Four takes under
    /// a minute; five takes a few minutes.
    const PREEMPTIONS: usize = 4;

    /// Two tasks submit and register concurrently while the bundle is
    /// dropped: both are served, one thread polls at a time, and the thread
    /// exits once both are settled.
    #[test]
    fn concurrent_registrations_are_served_and_the_thread_exits() {
        model(|| {
            let (poller, handle, device, [w0, w1], flags) = setup();
            let other = {
                let device = Arc::clone(&device);
                thread::spawn(move || {
                    submit(&device, 1);
                    w1.register();
                    drop(w1);
                })
            };
            submit(&device, 0);
            w0.register();
            drop(w0);
            drop(poller);
            other.join().unwrap();
            handle.join().unwrap();
            check_settled(&device, &flags);
        });
    }

    /// The host polls on its own thread (a blocking wait, or its frame's
    /// submit) while a task registers: the callback runs before, during or
    /// after the registration, and the thread still exits.
    #[test]
    fn a_callback_racing_the_registration_settles_the_count() {
        model(|| {
            let (poller, handle, device, [w0, w1], flags) = setup();
            submit(&device, 0);
            let host = {
                let device = Arc::clone(&device);
                thread::spawn(move || {
                    device.poll();
                })
            };
            w0.register();
            drop(w0);
            host.join().unwrap();
            // The second wait is never submitted or registered.
            device.marks[1].fire();
            drop(w1);
            drop(poller);
            handle.join().unwrap();
            check_settled(&device, &flags);
        });
    }

    /// The bundle is dropped while a wait is pending: the thread serves the
    /// wait and then exits.
    #[test]
    fn a_wait_outliving_the_bundle_is_served() {
        model(|| {
            let (poller, handle, device, [w0, w1], flags) = setup();
            drop(poller);
            submit(&device, 0);
            w0.register();
            drop(w0);
            device.marks[1].fire();
            drop(w1);
            handle.join().unwrap();
            check_settled(&device, &flags);
        });
    }

    /// An idle bundle's thread parks instead of polling, and a registration
    /// after the idle period is served: no lost wake-up.
    #[test]
    fn an_idle_thread_parks_and_wakes_for_the_next_wait() {
        model(|| {
            let (poller, handle, device, [w0, w1], flags) = setup();
            submit(&device, 0);
            w0.register();
            drop(w0);
            // The thread may be idle by now; the next wait must wake it.
            submit(&device, 1);
            w1.register();
            drop(w1);
            drop(poller);
            handle.join().unwrap();
            check_settled(&device, &flags);
        });
    }

    /// A failed poll parks the thread with the wait still counted; the next
    /// registration wakes it, and the retried poll serves both waits.
    #[test]
    fn a_failed_poll_is_retried_by_the_next_registration() {
        model(|| {
            let (poller, handle, device, [w0, w1], flags) = setup();
            device.fail_next.swap(true, Ordering::AcqRel);
            submit(&device, 0);
            w0.register();
            // Register the second wait only after the failure.
            while !device.failed.fetch_or(false, Ordering::AcqRel) {
                thread::yield_now();
            }
            submit(&device, 1);
            w1.register();
            // Both waits stay owned until served, as an awaiting future
            // keeps its mark.
            while flags
                .iter()
                .any(|f| f.fetch_or(0, Ordering::AcqRel) & FIRED == 0)
            {
                thread::yield_now();
            }
            drop((w0, w1, poller));
            handle.join().unwrap();
            check_settled(&device, &flags);
        });
    }
}
