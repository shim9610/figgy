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
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
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
    pollster::block_on(adapter.request_device(descriptor))
        .expect("required GPU test: device initialization failed")
}

pub fn device() -> (wgpu::Device, wgpu::Queue) {
    let instance = instance();
    let adapter = adapter(&instance);
    eprintln!("required GPU test adapter: {:?}", adapter.get_info());
    request_device(&adapter, &wgpu::DeviceDescriptor::default())
}
