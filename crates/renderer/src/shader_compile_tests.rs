use super::*;
use crate::gpu_compile::capture;
use std::collections::HashSet;

/// The annotation layer's fullscreen pipeline builder loads only its own
/// shader module.
#[test]
fn fullscreen_textured_builder_compiles_only_its_shader() {
    let (device, _) = crate::data_render::shared_device().expect("required GPU adapter");
    let session = capture::Session::begin();
    let texture = data_render::create_texture_bind_group_layout(&device);
    let _pipeline = data_render::create_fullscreen_textured_pipeline(
        &device,
        &texture,
        wgpu::TextureFormat::Rgba8Unorm,
    );
    let captured = session.finish();
    assert_eq!(captured.sources.len(), 1, "loaded unrelated shaders");
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

/// Exercise real constructors rather than a second, approximate list of GPU
/// descriptors. Exporting the capture also drives the WebGPU timing test.
#[test]
fn all_shader_compilation_times_and_entry_coverage() {
    let (device, queue) = crate::data_render::shared_device().expect("required GPU adapter");
    let gpu = RendererDevice::new(device.clone(), queue.clone()).expect("figgy poll thread");
    let session = capture::Session::begin();
    // Both shader sets (#299): a new renderer's basic modules, and
    // the full modules of one that has been handed a chart that needs them. The
    // same constructors run for each, so every entry of every module is
    // measured in both.
    for (set, samples) in [
        (ShaderSet::Basic, 1),
        (ShaderSet::Basic, 4),
        (ShaderSet::Full, 1),
        (ShaderSet::Full, 4),
    ] {
        let mut renderer = Renderer::try_new_with_sample_count(
            gpu.clone(),
            wgpu::TextureFormat::Rgba8Unorm,
            64 * 1024,
            samples,
        )
        .unwrap();
        assert_eq!(renderer.shader_set(), ShaderSet::Basic);
        renderer.admit_full_shaders(set == ShaderSet::Full);
        assert_eq!(renderer.shader_set(), set);
        crate::test_gpu::block_on(renderer.prewarm_all()).unwrap();
        // No shader or pipeline creation at all, even a compiler-cache hit.
        let before = capture::counts();
        crate::test_gpu::block_on(renderer.prewarm_all()).unwrap();
        assert_eq!(before, capture::counts());
        // The exact pick modules compile on the first pick that needs each;
        // measure every module (with every path) here.
        if samples == 1 {
            let bundle = renderer.picker.ready_bundle().unwrap();
            for shader in crate::gpu_pick::PickShader::EXACT {
                let key = crate::gpu_pick::PickExactKey {
                    shader,
                    ..crate::gpu_pick::PickExactKey::ALL_PATHS
                };
                renderer.picker.exact.ensure(&bundle, key).unwrap();
            }
            // So do the typed data pick kernels (#327).
            let data = renderer.picker.ready_data_bundle().unwrap();
            for kernel in crate::gpu_data_pick::DataPickKernel::ALL {
                renderer.picker.data_kernels.ensure(&data, kernel).unwrap();
            }
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
    crate::radial::RadialRenderer::try_new(gpu, wgpu::TextureFormat::Rgba8Unorm).unwrap();
    // The pool's compact upload expansion (#317), created on the first
    // single-lane upload.
    crate::data_render::column_pool::ColumnExpand::new(&device).unwrap();
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
        let compute = &pipeline["descriptor"]["compute"];
        if matches!(
            compute["entryPoint"].as_str(),
            Some(
                "reduce_values"
                    | "reduce_states"
                    | "pick_exact_candidates"
                    | "pick_reduce_candidates"
                    | "pick_histogram_bin"
                    | "pick_category_bar"
                    | "pick_box_plot"
                    | "pick_range_bar"
                    | "pick_violin"
                    | "pick_value_labels"
                    | "scan_block"
            )
        ) {
            let explicit =
                compute["constants"]["FIGGY_EXPLICIT_WORKGROUP_ZERO"].as_f64() == Some(1.0);
            assert_eq!(
                explicit,
                cfg!(target_os = "windows") && device.adapter_info().backend == wgpu::Backend::Dx12,
                "incorrect workgroup initialization policy: {compute}"
            );
        }
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
    // Both assemblies of every module by cuts were compiled.
    for (name, by_set) in crate::shader_assembly::MODULES_BY_SET {
        for (which, assembled) in [("basic", by_set.basic), ("full", by_set.full)] {
            assert!(
                capture
                    .sources
                    .iter()
                    .any(|s| s["code"].as_str().unwrap() == assembled.source),
                "{name} ({which}) was not compiled"
            );
        }
    }
    // This generated metadata is also the browser prewarm descriptor source.
    // No shader code is duplicated there. A changed shader/layout cannot ship
    // with an obsolete prewarm definition unnoticed.
    let contract = capture.browser_contract();
    // A full module compiles to the same pipeline descriptors as its basic
    // twin, so the contract stores each once,
    // under the basic source (#299): no entry is keyed by a full source,
    // and every basic source that has pipelines is there.
    for (name, by_set) in crate::shader_assembly::MODULES_BY_SET {
        let full_key = crate::gpu_compile::source_key(by_set.full.source);
        let basic_key = crate::gpu_compile::source_key(by_set.basic.source);
        let keyed = |key: &str| {
            contract["pipelines"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| p["source"].as_str() == Some(key))
                .count()
        };
        assert_eq!(keyed(&full_key), 0, "{name}");
        assert!(keyed(&basic_key) > 0, "{name}");
    }
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
