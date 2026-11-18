//! Native test GPU setup. Initialization errors fail tests; they never become skips.
//! Environment backend selection applies only to the test harness, not production.
#![allow(dead_code)]

pub fn instance() -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    // Exercise request_adapter's real error path on every OS in a subprocess,
    // without changing system drivers or production initialization.
    if std::env::var("FIGGY_TEST_DISABLE_ADAPTERS").as_deref() == Ok("1") {
        descriptor.backends = wgpu::Backends::empty();
    }
    wgpu::Instance::new(descriptor)
}

pub fn adapter(instance: &wgpu::Instance) -> wgpu::Adapter {
    block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        apply_limit_buckets: false,
        force_fallback_adapter: std::env::var("FIGGY_TEST_FORCE_FALLBACK").as_deref() == Ok("1"),
        ..Default::default()
    }))
    .expect("required GPU test: adapter initialization failed")
}

pub fn request_device(
    adapter: &wgpu::Adapter,
    descriptor: &wgpu::DeviceDescriptor<'_>,
) -> (wgpu::Device, wgpu::Queue) {
    block_on(adapter.request_device(descriptor))
        .expect("required GPU test: device initialization failed")
}

pub fn device() -> (wgpu::Device, wgpu::Queue) {
    let instance = instance();
    let adapter = adapter(&instance);
    eprintln!("required GPU test adapter: {:?}", adapter.get_info());
    request_device(&adapter, &wgpu::DeviceDescriptor::default())
}

/// Run `future` to completion on this thread: the tests' executor for the
/// async API (#160), so the repository needs no executor crate. The thread
/// parks until the future's waker unparks it; a spurious wake-up only polls
/// again.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    /// Wakes the blocked thread.
    struct Unpark(std::thread::Thread);

    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

/// [`block_on`] as a method: `future.block_on()`.
pub trait BlockOn: std::future::Future + Sized {
    fn block_on(self) -> Self::Output {
        block_on(self)
    }
}

impl<F: std::future::Future> BlockOn for F {}
