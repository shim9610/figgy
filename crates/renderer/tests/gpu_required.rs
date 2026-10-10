#![cfg(not(target_arch = "wasm32"))]
#[path = "support/gpu.rs"]
mod gpu;

#[test]
fn required_gpu_initialization() {
    let (device, queue) = gpu::device();
    queue.submit([]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .expect("GPU submission must complete");
}

#[test]
#[should_panic(expected = "required GPU test: device initialization failed")]
fn rejected_device_limits_fail_the_test_harness() {
    let instance = gpu::instance();
    let adapter = gpu::adapter(&instance);
    let mut descriptor = wgpu::DeviceDescriptor::default();
    descriptor.required_limits.max_bind_groups = adapter
        .limits()
        .max_bind_groups
        .checked_add(1)
        .expect("adapter bind group limit fits u32");
    let _ = gpu::request_device(&adapter, &descriptor);
}
