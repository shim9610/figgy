//! Async prewarm uses descriptors captured from and checked against the real
//! pipeline constructors. Auto-layout guesses cannot warm an explicit layout.
use js_sys::{Array, Function, Object, Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue};

thread_local! {
    static CONTRACT: JsValue = js_sys::JSON::parse(include_str!("shader_pipeline_contract.json"))
        .expect("validated shader pipeline contract");
}

fn error(e: JsValue) -> String {
    format!("WebGPU pipeline compilation failed: {e:?}")
}
fn get(o: &JsValue, key: &str) -> Result<JsValue, String> {
    Reflect::get(o, &key.into()).map_err(error)
}
fn set(o: &JsValue, key: &str, value: &JsValue) -> Result<(), String> {
    Reflect::set(o, &key.into(), value)
        .map(|_| ())
        .map_err(error)
}
fn call(device: &JsValue, method: &str, desc: &JsValue) -> Result<JsValue, String> {
    get(device, method)?
        .dyn_into::<Function>()
        .map_err(error)?
        .call1(device, desc)
        .map_err(error)
}

/// Owned JS descriptors; never attach GPU handles to the process-wide metadata.
fn descriptors(source: &str, kind: &str) -> Result<Vec<JsValue>, String> {
    let key = crate::gpu_compile::source_key(source);
    CONTRACT.with(|contract| {
        let mut out = Vec::new();
        for p in Array::from(&get(contract, "pipelines")?).iter() {
            if get(&p, "source")?.as_string().as_deref() != Some(&key)
                || get(&p, "kind")?.as_string().as_deref() != Some(kind)
            {
                continue;
            }
            let original = get(&p, "descriptor")?;
            let desc = js_sys::JSON::parse(
                &js_sys::JSON::stringify(&original)
                    .map_err(error)?
                    .as_string()
                    .unwrap(),
            )
            .map_err(error)?;
            if let Some(index) = get(&desc, "layout")?.as_f64() {
                let groups = Array::from(&get(contract, "layouts")?).get(index as u32);
                set(&desc, "layout", &groups)?;
            }
            out.push(desc);
        }
        if out.is_empty() {
            return Err(format!(
                "Missing checked {kind} prewarm contract for shader {key}"
            ));
        }
        Ok(out)
    })
}

fn shader(device: &JsValue, source: &str, label: &str) -> Result<JsValue, String> {
    let desc = Object::new();
    set(&desc, "label", &label.into())?;
    set(&desc, "code", &source.into())?;
    call(device, "createShaderModule", &desc)
}

fn populate(device: &JsValue, desc: &JsValue, module: &JsValue) -> Result<(), String> {
    let layout = get(desc, "layout")?;
    if !Array::is_array(&layout) {
        return Err("Explicit prewarm layout required".into());
    }
    let groups = Array::new();
    for entries in Array::from(&layout).iter() {
        if entries.is_null() {
            groups.push(&JsValue::NULL);
            continue;
        }
        let group = Object::new();
        set(&group, "entries", &entries)?;
        groups.push(&call(device, "createBindGroupLayout", &group)?);
    }
    let pipeline_layout = Object::new();
    set(&pipeline_layout, "bindGroupLayouts", &groups)?;
    set(
        desc,
        "layout",
        &call(device, "createPipelineLayout", &pipeline_layout)?,
    )?;
    for stage in ["compute", "vertex", "fragment"] {
        let stage = get(desc, stage)?;
        if stage.is_object() {
            set(&stage, "module", module)?;
        }
    }
    Ok(())
}

pub(crate) async fn compute(
    device: &wgpu::Device,
    scope: &'static str,
    source: &str,
    entries: &[(&'static str, &'static str)],
    observer: &mut dyn FnMut(crate::InitEvent),
) -> Result<(), String> {
    let device = JsValue::from(device.as_webgpu().ok_or("WebGPU device required")?.clone());
    let candidates = descriptors(source, "compute")?;
    let module = shader(&device, source, scope)?;
    for (index, (stage, entry)) in entries.iter().enumerate() {
        let matching: Vec<_> = candidates
            .iter()
            .filter(|desc| {
                get(desc, "compute")
                    .and_then(|s| get(&s, "entryPoint"))
                    .ok()
                    .and_then(|s| s.as_string())
                    .as_deref()
                    == Some(entry)
            })
            .collect();
        if matching.len() != 1 {
            return Err(format!(
                "Expected one checked layout for {scope}/{entry}, found {}",
                matching.len()
            ));
        }
        let desc = matching[0];
        populate(&device, desc, &module)?;
        // Consecutive entries may belong to one existing progress stage.
        // Splitting a kernel must not change the host's progress contract.
        if index == 0 || entries[index - 1].0 != *stage {
            crate::init::started(observer, scope, stage);
        }
        compile(&device, "createComputePipelineAsync", desc, &module).await?;
        if index + 1 == entries.len() || entries[index + 1].0 != *stage {
            crate::init::finished(observer, scope, stage);
        }
        crate::init::yield_init_frame().await;
    }
    Ok(())
}

pub(crate) async fn render(
    device: &wgpu::Device,
    source: &str,
    stages: &[(&'static str, &'static str)],
    format: wgpu::TextureFormat,
    samples: u32,
    observer: &mut dyn FnMut(crate::InitEvent),
) -> Result<(), String> {
    let format = match format {
        wgpu::TextureFormat::Rgba8Unorm => "rgba8unorm",
        wgpu::TextureFormat::Bgra8Unorm => "bgra8unorm",
        wgpu::TextureFormat::Rgba8UnormSrgb => "rgba8unorm-srgb",
        wgpu::TextureFormat::Bgra8UnormSrgb => "bgra8unorm-srgb",
        other => return Err(format!("Unsupported prewarm target: {other:?}")),
    };
    let device = JsValue::from(device.as_webgpu().ok_or("WebGPU device required")?.clone());
    let mut candidates = Vec::new();
    for desc in descriptors(source, "render")? {
        // Auto-layout streaming pipelines are constructed lazily by their own
        // runtime. An auto-layout guess here would only compile them twice.
        if !Array::is_array(&get(&desc, "layout")?) {
            continue;
        }
        if get(&get(&desc, "multisample")?, "count")?.as_f64() != Some(f64::from(samples)) {
            continue;
        }
        candidates.push(desc);
    }
    let module = shader(&device, source, "figgy render prewarm")?;
    for (index, (stage, label)) in stages.iter().enumerate() {
        let mut selected = Vec::new();
        let mut rest = Vec::new();
        for desc in candidates {
            if get(&desc, "label")?.as_string().as_deref() == Some(label) {
                selected.push(desc);
            } else {
                rest.push(desc);
            }
        }
        if selected.is_empty() {
            return Err(format!(
                "No checked render pipeline for {stage}, samples={samples}"
            ));
        }
        // Additional variants belong to the source's final stage. Preserve
        // the existing initialization progress event names and ordering.
        if index + 1 == stages.len() {
            selected.append(&mut rest);
        }
        candidates = rest;
        crate::init::started(observer, "renderer.prewarm.async", stage);
        for desc in selected {
            populate(&device, &desc, &module)?;
            for target in Array::from(&get(&get(&desc, "fragment")?, "targets")?).iter() {
                if !target.is_null() {
                    set(&target, "format", &format.into())?;
                }
            }
            compile(&device, "createRenderPipelineAsync", &desc, &module).await?;
        }
        crate::init::finished(observer, "renderer.prewarm.async", stage);
        crate::init::yield_init_frame().await;
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
async fn shader_compilation_errors(shader: &wasm_bindgen::JsValue) -> Vec<String> {
    use js_sys::{Array, Function, Promise, Reflect};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    let Ok(method) = Reflect::get(shader, &JsValue::from_str("getCompilationInfo")) else {
        return Vec::new();
    };
    let Ok(method) = method.dyn_into::<Function>() else {
        return Vec::new();
    };
    let Ok(promise) = method.call0(shader) else {
        return Vec::new();
    };
    let Ok(info) = JsFuture::from(Promise::from(promise)).await else {
        return Vec::new();
    };
    let Ok(messages) = Reflect::get(&info, &JsValue::from_str("messages")) else {
        return Vec::new();
    };

    Array::from(&messages)
        .iter()
        .filter_map(|message| {
            let severity = Reflect::get(&message, &JsValue::from_str("type"))
                .ok()?
                .as_string()?;
            if severity != "error" {
                return None;
            }
            let text = Reflect::get(&message, &JsValue::from_str("message"))
                .ok()?
                .as_string()?;
            let line = Reflect::get(&message, &JsValue::from_str("lineNum"))
                .ok()
                .and_then(|value| value.as_f64())
                .unwrap_or_default() as u32;
            let column = Reflect::get(&message, &JsValue::from_str("linePos"))
                .ok()
                .and_then(|value| value.as_f64())
                .unwrap_or_default() as u32;
            Some(format!("line {line}:{column}: {text}"))
        })
        .collect()
}

async fn compile(
    device: &JsValue,
    method: &str,
    desc: &JsValue,
    module: &JsValue,
) -> Result<(), String> {
    let promise = call(device, method, desc)?;
    match wasm_bindgen_futures::JsFuture::from(Promise::from(promise)).await {
        Ok(_) => Ok(()),
        Err(failure) => {
            let details = shader_compilation_errors(module).await;
            Err(format!("{}; {}", error(failure), details.join("; ")))
        }
    }
}
