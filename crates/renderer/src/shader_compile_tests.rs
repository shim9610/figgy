use super::*;
use crate::gpu_compile::capture;
use std::collections::HashSet;

#[test]
fn single_pipeline_builders_compile_only_the_requested_shader() {
    let (device, _) = crate::data_render::shared_device().expect("required GPU adapter");
    for which in 0..9 {
        let session = capture::Session::begin();
        let texture = data_render::create_texture_bind_group_layout(&device);
        let transform = data_render::create_scatter_transform_bind_group_layout(&device);
        let style = data_render::create_style_bind_group_layout(&device);
        let mapping = data_render::create_per_point_style_map_bind_group_layout(&device);
        let field = data_render::create_field_data_bind_group_layout(&device);
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let _pipeline = match which {
            0 => data_render::create_fullscreen_textured_pipeline(&device, &texture, format),
            1 => data_render::create_line_columnar_pipeline(&device, &transform, &style, format),
            2 => data_render::create_scatter_columnar_pipeline(&device, &transform, &style, format),
            3 => data_render::create_scatter_columnar_mapped_pipeline(
                &device, &transform, &style, &mapping, format, 1,
            ),
            4 => {
                data_render::create_errorbar_columnar_pipeline(&device, &transform, &style, format)
            }
            5 => data_render::create_errorbar_columnar_mapped_pipeline(
                &device, &transform, &style, &mapping, format, 1,
            ),
            6 => data_render::create_bar_columnar_pipeline(&device, &transform, &style, format),
            7 => data_render::create_bar_columnar_mapped_pipeline(
                &device, &transform, &style, &mapping, format,
            ),
            8 => data_render::create_field_columnar_pipeline(
                &device, &transform, &style, &field, format,
            ),
            _ => unreachable!(),
        };
        let captured = session.finish();
        assert_eq!(
            captured.sources.len(),
            1,
            "builder {which} loaded unrelated shaders"
        );
        assert_eq!(
            captured
                .timings
                .iter()
                .filter(|r| r["kind"] == "module")
                .count(),
            1
        );
        assert_eq!(captured.pipelines.len(), 1);
    }
}

/// Exercise real constructors rather than a second, approximate list of GPU
/// descriptors. Exporting the capture also drives the WebGPU timing test.
#[test]
fn all_shader_compilation_times_and_entry_coverage() {
    let (device, queue) = crate::data_render::shared_device().expect("required GPU adapter");
    let gpu = RendererDevice::new(device.clone(), queue.clone());
    let session = capture::Session::begin();
    for samples in [1, 4] {
        let mut renderer = Renderer::try_new_with_sample_count(
            gpu.clone(),
            wgpu::TextureFormat::Rgba8Unorm,
            64 * 1024,
            samples,
        )
        .unwrap();
        pollster::block_on(renderer.prewarm_all()).unwrap();
        // No shader or pipeline creation at all, even a compiler-cache hit.
        let before = capture::counts();
        pollster::block_on(renderer.prewarm_all()).unwrap();
        assert_eq!(before, capture::counts());
        if samples == 1 {
            data_render::create_bar_columnar_pipeline(
                &device,
                &renderer.transform_bgl,
                &renderer.style_bgl,
                wgpu::TextureFormat::Rgba8Unorm,
            );
            data_render::create_bar_columnar_mapped_pipeline(
                &device,
                &renderer.transform_bgl,
                &renderer.style_bgl,
                &renderer.per_point_style_map_bgl,
                wgpu::TextureFormat::Rgba8Unorm,
            );
        }
        data_render::stream_field::Pipelines::new(
            &device,
            &renderer.pipelines.shaders.field,
            wgpu::TextureFormat::Rgba8Unorm,
            samples,
        )
        .unwrap();
        streaming_surface::StreamTransfer::new(&device, wgpu::TextureFormat::Rgba8Unorm, samples)
            .unwrap();
        streaming_surface::compile_replay_for_test(
            &device,
            streaming_surface::StreamSurfaceSpec {
                width: 64,
                height: 64,
                format: wgpu::TextureFormat::Rgba8Unorm,
                sample_count: samples,
            },
        );
    }
    crate::radial::RadialRenderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm).unwrap();
    crate::categorical::CategoricalRenderer::new(gpu.clone(), wgpu::TextureFormat::Rgba8Unorm)
        .unwrap();
    crate::boxplot::BoxPlotRenderer::new(gpu, wgpu::TextureFormat::Rgba8Unorm).unwrap();
    let capture = session.finish();
    if let Some(path) = std::env::var_os("FIGGY_SHADER_COMPILE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&capture.json()).unwrap()).unwrap();
    }

    // Every shipped WGSL source must occur in a measured module. This includes
    // templates whose declarations are inserted by the production constructor.
    fn wgsl_files(path: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                wgsl_files(&path, files);
            } else if path.extension().is_some_and(|e| e == "wgsl") {
                files.push(path);
            }
        }
    }
    let mut files = Vec::new();
    wgsl_files(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    for file in files {
        let source = std::fs::read_to_string(&file).unwrap();
        assert!(
            capture
                .sources
                .iter()
                .any(|s| s["code"].as_str().unwrap().contains(&source)),
            "Shader missing from compilation test: {}",
            file.display()
        );
    }
    let mut used = HashSet::new();
    for pipeline in &capture.pipelines {
        for stage in ["compute", "vertex", "fragment"] {
            let s = &pipeline["descriptor"][stage];
            if s.is_object() {
                used.insert((
                    s["module"].as_u64().unwrap() as usize,
                    s["entryPoint"].as_str().unwrap().to_owned(),
                ));
            }
        }
    }
    let mut missing = Vec::new();
    for (index, source) in capture.sources.iter().enumerate() {
        let module = wgpu::naga::front::wgsl::parse_str(source["code"].as_str().unwrap()).unwrap();
        for entry in module.entry_points {
            if !used.contains(&(index, entry.name.clone())) {
                missing.push(format!("{} in {}", entry.name, source["label"]));
            }
        }
    }
    assert!(missing.is_empty(), "Unmeasured entries: {missing:?}");
    // This generated metadata is also the browser prewarm descriptor source.
    // No shader code is duplicated there. A changed shader/layout cannot ship
    // with an obsolete prewarm definition unnoticed.
    let contract = capture.browser_contract();
    let contract_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shader_pipeline_contract.json");
    if std::env::var_os("FIGGY_UPDATE_SHADER_CONTRACT").as_deref()
        == Some(std::ffi::OsStr::new("1"))
    {
        std::fs::write(
            &contract_path,
            format!("{}\n", serde_json::to_string_pretty(&contract).unwrap()),
        )
        .unwrap();
    }
    let stored: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&contract_path).expect("Generate the shader prewarm contract"),
    )
    .unwrap();
    assert_eq!(
        stored, contract,
        "Prewarm contract differs from actual pipelines; regenerate with FIGGY_UPDATE_SHADER_CONTRACT=1 and rerun without that flag"
    );
    let limit: f64 = std::env::var("FIGGY_SHADER_COMPILE_MAX_MS")
        .map(|v| v.parse().expect("positive milliseconds"))
        .unwrap_or(30_000.0);
    assert!(limit.is_finite() && limit > 0.0);
    for timing in &capture.timings {
        println!("SHADER_COMPILE {timing}");
        assert!(
            timing["ms"].as_f64().unwrap() <= limit,
            "Compilation exceeded {limit}ms: {timing}"
        );
    }
    println!(
        "SHADER_COMPILE_SUMMARY modules={} pipelines={}",
        capture.sources.len(),
        capture.pipelines.len()
    );
}
