//! Single creation boundary for shader compilation diagnostics.
//!
//! Normal builds forward to wgpu. Unit-test builds can record the actual
//! descriptors, including generated WGSL and explicit layouts, so the
//! browser benchmark exercises the same pipelines as the native renderer.
//!
//! Natively every shader module and pipeline is created on the renderer's
//! own compile thread ([`on_compile_stack`], #222): a backend's shader
//! compiler can need more stack than a host thread has (WARP needs 2 to
//! 4 MiB for the largest pipelines; a Windows main thread has 1 MiB), so
//! the renderer brings its own instead of asking every host for a larger
//! one.

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "shader_compile_capture.rs"]
pub(crate) mod capture;

#[path = "workgroup_init.rs"]
mod workgroup_init;
pub(crate) use workgroup_init::compilation_options as workgroup_options;

#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn source_key(source: &str) -> String {
    // Stable content identity, not a security digest. The contract regression
    // also compares every descriptor against the real Rust constructors.
    let hash = source.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}

/// The compile thread's stack (#222): four times the largest need measured
/// on WARP, and only reserved address space until a compile touches it.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const COMPILE_STACK_BYTES: usize = 16 << 20;

/// The compile thread's name, which a test reads back.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const COMPILE_THREAD_NAME: &str = "figgy shader compile";

/// `create` on the compile thread, a scoped thread of
/// [`COMPILE_STACK_BYTES`] (#222).
///
/// wgpu's error scopes are per thread, so the compile thread pushes the
/// three the renderer reads (out of memory, validation, internal) around
/// the call and pops them. When they are clear the object is returned and
/// the caller's scopes saw nothing, as before. When one captured an error,
/// the thread did not start or it panicked, `create` runs again on the
/// calling thread, so the error lands in the caller's scope (`gpu_checked`,
/// the export scopes) exactly as it did there, and a panic inside wgpu
/// re-raises there. Errors are deterministic, so the second call reports
/// the same one.
#[cfg(not(target_arch = "wasm32"))]
fn on_compile_stack<T: Send>(device: &wgpu::Device, create: impl Fn() -> T + Sync) -> T {
    let compiled = std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name(COMPILE_THREAD_NAME.to_owned())
            .stack_size(COMPILE_STACK_BYTES)
            .spawn_scoped(scope, || {
                let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
                let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
                let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
                let value = create();
                // Popped in reverse push order, every one, before any is read.
                let clear = [
                    scope_clear(internal.pop()),
                    scope_clear(validation.pop()),
                    scope_clear(out_of_memory.pop()),
                ];
                clear.iter().all(|&clear| clear).then_some(value)
            })
            .ok()
            .and_then(|thread| thread.join().ok())
            .flatten()
    });
    match compiled {
        Some(value) => value,
        None => create(),
    }
}

/// Whether a popped native scope resolved clear at once. `wgpu_core` hands
/// back a ready future; a pending or captured one is not clear.
#[cfg(not(target_arch = "wasm32"))]
fn scope_clear(popped: impl std::future::Future<Output = Option<wgpu::Error>>) -> bool {
    let mut popped = std::pin::pin!(popped);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    matches!(
        popped.as_mut().poll(&mut context),
        std::task::Poll::Ready(None)
    )
}

/// `desc` with its WGSL borrowed, so the compile thread and a retry on the
/// calling thread read the one source rather than copies of it.
#[cfg(not(target_arch = "wasm32"))]
fn borrowed<'a>(desc: &'a wgpu::ShaderModuleDescriptor<'_>) -> wgpu::ShaderModuleDescriptor<'a> {
    let source = match &desc.source {
        wgpu::ShaderSource::Wgsl(wgsl) => {
            wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(wgsl))
        }
        other => other.clone(),
    };
    wgpu::ShaderModuleDescriptor {
        label: desc.label,
        source,
    }
}

#[inline]
pub(crate) fn shader_module(
    device: &wgpu::Device,
    desc: wgpu::ShaderModuleDescriptor<'_>,
) -> wgpu::ShaderModule {
    #[cfg(all(test, not(target_arch = "wasm32")))]
    let started = std::time::Instant::now();
    #[cfg(all(test, not(target_arch = "wasm32")))]
    let recorded_desc = desc.clone();
    #[cfg(not(target_arch = "wasm32"))]
    let result = on_compile_stack(device, || device.create_shader_module(borrowed(&desc)));
    // The browser has no threads and compiles in its GPU process.
    #[cfg(target_arch = "wasm32")]
    let result = device.create_shader_module(desc);
    #[cfg(all(test, not(target_arch = "wasm32")))]
    capture::shader(&result, &recorded_desc, started.elapsed());
    result
}

#[inline]
pub(crate) fn bind_group_layout(
    device: &wgpu::Device,
    desc: &wgpu::BindGroupLayoutDescriptor<'_>,
) -> wgpu::BindGroupLayout {
    let result = device.create_bind_group_layout(desc);
    #[cfg(all(test, not(target_arch = "wasm32")))]
    capture::bindings(&result, desc);
    result
}

#[inline]
pub(crate) fn pipeline_layout(
    device: &wgpu::Device,
    desc: &wgpu::PipelineLayoutDescriptor<'_>,
) -> wgpu::PipelineLayout {
    let result = device.create_pipeline_layout(desc);
    #[cfg(all(test, not(target_arch = "wasm32")))]
    capture::layout(&result, desc);
    result
}

#[inline]
pub(crate) fn compute_pipeline(
    device: &wgpu::Device,
    desc: &wgpu::ComputePipelineDescriptor<'_>,
) -> wgpu::ComputePipeline {
    #[cfg(all(test, not(target_arch = "wasm32")))]
    let started = std::time::Instant::now();
    #[cfg(not(target_arch = "wasm32"))]
    let result = on_compile_stack(device, || device.create_compute_pipeline(desc));
    #[cfg(target_arch = "wasm32")]
    let result = device.create_compute_pipeline(desc);
    #[cfg(all(test, not(target_arch = "wasm32")))]
    capture::compute(desc, started.elapsed());
    result
}

#[inline]
pub(crate) fn render_pipeline(
    device: &wgpu::Device,
    desc: &wgpu::RenderPipelineDescriptor<'_>,
) -> wgpu::RenderPipeline {
    #[cfg(all(test, not(target_arch = "wasm32")))]
    let started = std::time::Instant::now();
    #[cfg(not(target_arch = "wasm32"))]
    let result = on_compile_stack(device, || device.create_render_pipeline(desc));
    #[cfg(target_arch = "wasm32")]
    let result = device.create_render_pipeline(desc);
    #[cfg(all(test, not(target_arch = "wasm32")))]
    capture::render(desc, started.elapsed());
    result
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod compile_thread_tests {
    use super::*;
    use crate::renderer::{DeviceHealth, gpu_checked};

    fn gpu() -> (wgpu::Device, wgpu::Queue) {
        let instance = crate::data_render::create_instance();
        let adapter = crate::test_gpu::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
        )
        .expect("compile thread GPU test requires an adapter");
        crate::test_gpu::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("compile thread GPU device creation failed")
    }

    const PROBE: &str = "@group(0) @binding(0) var<storage, read_write> out: array<u32>;\n\
        @compute @workgroup_size(1) fn main() { out[0] = 1u; }";

    /// #222: shader modules and pipelines are created on the compile thread,
    /// whatever thread asks for them, and the thread is named so.
    #[test]
    fn creation_runs_on_the_compile_thread() {
        let (device, _queue) = gpu();
        let on = on_compile_stack(&device, || std::thread::current().name().map(str::to_owned));
        assert_eq!(on.as_deref(), Some(COMPILE_THREAD_NAME));
        // A caller on a small stack still gets its pipeline: the compile
        // runs on the compile thread's stack, not this one.
        let small = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                let module = shader_module(
                    &device,
                    wgpu::ShaderModuleDescriptor {
                        label: Some("compile thread probe"),
                        source: wgpu::ShaderSource::Wgsl(PROBE.into()),
                    },
                );
                let health = DeviceHealth::watch(&device);
                let _pipeline = compute_pipeline(
                    &device,
                    &wgpu::ComputePipelineDescriptor {
                        label: Some("compile thread probe"),
                        layout: None,
                        module: &module,
                        entry_point: Some("main"),
                        compilation_options: Default::default(),
                        cache: None,
                    },
                );
                health.check()
            })
            .expect("spawn");
        small.join().expect("probe thread").expect("no GPU error");
    }

    /// #222: a creation that fails on the compile thread reports into the
    /// caller's own error scope (`gpu_checked`) at once, as it did when the
    /// caller created it, and nothing reaches the uncaptured-error handler.
    #[test]
    fn a_failed_creation_reports_into_the_callers_scope() {
        let (device, _queue) = gpu();
        let health = DeviceHealth::watch(&device);
        let module = shader_module(
            &device,
            wgpu::ShaderModuleDescriptor {
                label: Some("compile thread probe"),
                source: wgpu::ShaderSource::Wgsl(PROBE.into()),
            },
        );
        let result = gpu_checked(&device, "compile thread probe", || {
            compute_pipeline(
                &device,
                &wgpu::ComputePipelineDescriptor {
                    label: Some("compile thread probe"),
                    layout: None,
                    module: &module,
                    // No such entry: a validation error.
                    entry_point: Some("missing"),
                    compilation_options: Default::default(),
                    cache: None,
                },
            )
        });
        assert!(
            matches!(result, Err(crate::FiggyError::GpuValidation { .. })),
            "the caller's scope captures the error: {:?}",
            result.map(|_| ())
        );
        health
            .check()
            .expect("nothing reaches the uncaptured-error handler");
    }
}
