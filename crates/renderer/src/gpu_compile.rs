//! Single creation boundary for shader compilation diagnostics.
//!
//! Normal builds forward directly to wgpu. Unit-test builds can record the
//! actual descriptors, including generated WGSL and explicit layouts, so the
//! browser benchmark exercises the same pipelines as the native renderer.

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "shader_compile_capture.rs"]
pub(crate) mod capture;

#[cfg(any(test, target_arch = "wasm32"))]
pub(crate) fn source_key(source: &str) -> String {
    // Stable content identity, not a security digest. The contract regression
    // also compares every descriptor against the real Rust constructors.
    let hash = source.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
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
    let result = device.create_render_pipeline(desc);
    #[cfg(all(test, not(target_arch = "wasm32")))]
    capture::render(desc, started.elapsed());
    result
}
