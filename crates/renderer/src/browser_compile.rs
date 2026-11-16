//! Async prewarm uses descriptors captured from and checked against the real
//! pipeline constructors. Auto-layout guesses cannot warm an explicit layout.
use js_sys::{Array, Function, Object, Promise, Reflect};
use wasm_bindgen::{JsCast, JsValue};

use crate::compile_schedule::{CompileOutcome, WaitStep};

thread_local! {
    /// The bundled contract, parsed once per thread. A parse failure (a
    /// corrupted bundle) is returned by every prewarm that reads it.
    static CONTRACT: Result<JsValue, JsValue> =
        js_sys::JSON::parse(include_str!("shader_pipeline_contract.json"));
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
    // A full module compiles to the pipelines of its basic twin, which the contract stores (#299).
    let key = crate::gpu_compile::source_key(crate::shader_assembly::contract_source(source));
    CONTRACT.with(|contract| {
        let contract = contract.as_ref().map_err(|e| error(e.clone()))?;
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
                    .ok_or("a pipeline descriptor does not stringify")?,
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
        let mut matching = Vec::new();
        for desc in &candidates {
            let entry_point = get(&get(desc, "compute")?, "entryPoint")?;
            if entry_point.as_string().as_deref() == Some(entry) {
                matching.push(desc);
            }
        }
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
        settle(call(&device, "createComputePipelineAsync", desc)?, &module).await?;
        if index + 1 == entries.len() || entries[index + 1].0 != *stage {
            crate::init::finished(observer, scope, stage);
        }
        crate::init::yield_init_frame().await;
    }
    Ok(())
}

/// One WGSL source and the `(stage, pipeline label)` pairs its prewarm
/// reports, in order.
pub(crate) type RenderSource<'a> = (&'a str, &'a [(&'static str, &'static str)]);

/// Compile the render pipelines of `sources` that `feature` includes (all
/// of them for `None`). Every `createRenderPipelineAsync` call of every
/// source is made before the first is awaited, so the browser compiles the
/// set together instead of one pipeline per await (#299). The stages are
/// then reported in order, each as a started/finished pair.
pub(crate) async fn render(
    device: &wgpu::Device,
    sources: &[RenderSource<'_>],
    format: wgpu::TextureFormat,
    samples: u32,
    feature: Option<crate::PreparationFeature>,
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
    // One `[Promise, module]` pair per included stage, in stage order.
    let pending = Array::new();
    let mut issued = Ok(());
    for (source, stages) in sources {
        issued = issue_render(&device, source, stages, format, samples, feature, &pending);
        if issued.is_err() {
            break;
        }
    }
    let settled = match issued {
        Ok(()) => settle_render(sources, feature, &pending, observer).await,
        Err(reason) => Err(reason),
    };
    if settled.is_err() {
        abandon(&pending).await;
    }
    crate::init::yield_init_frame().await;
    settled
}

fn included(feature: Option<crate::PreparationFeature>, label: &str) -> bool {
    feature.is_none_or(|f| f.includes_label(label))
}

/// Start one source's compiles. For each stage `feature` includes, one
/// Promise (of all the stage's pipelines) and the module are pushed to
/// `pending`.
fn issue_render(
    device: &JsValue,
    source: &str,
    stages: &[(&'static str, &'static str)],
    format: &str,
    samples: u32,
    feature: Option<crate::PreparationFeature>,
    pending: &Array,
) -> Result<(), String> {
    let stages: Vec<_> = stages
        .iter()
        .copied()
        .filter(|(_, label)| included(feature, label))
        .collect();
    if stages.is_empty() {
        return Ok(());
    }
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
        if get(&desc, "label")?
            .as_string()
            .is_some_and(|label| included(feature, &label))
        {
            candidates.push(desc);
        }
    }
    let module = shader(device, source, "figgy render prewarm")?;
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
        let compiles = Array::new();
        for desc in selected {
            populate(device, &desc, &module)?;
            for target in Array::from(&get(&get(&desc, "fragment")?, "targets")?).iter() {
                // A colour target takes the canvas format; an ID target
                // (the 3D pick's `rgba32uint`) keeps its own.
                if !target.is_null()
                    && get(&target, "format")?.as_string().as_deref() != Some("rgba32uint")
                {
                    set(&target, "format", &format.into())?;
                }
            }
            compiles.push(&call(device, "createRenderPipelineAsync", &desc)?);
        }
        // Pushed before the next stage can fail, so `abandon` observes it.
        pending.push(&Promise::all(&compiles));
        pending.push(&module);
    }
    Ok(())
}

/// Await what [`issue_render`] started, reporting each stage in order.
async fn settle_render(
    sources: &[RenderSource<'_>],
    feature: Option<crate::PreparationFeature>,
    pending: &Array,
    observer: &mut dyn FnMut(crate::InitEvent),
) -> Result<(), String> {
    let mut next = 0;
    for (_, stages) in sources {
        for (stage, label) in stages.iter() {
            if !included(feature, label) {
                continue;
            }
            crate::init::started(observer, "renderer.prewarm.async", stage);
            settle(pending.get(next), &pending.get(next + 1)).await?;
            crate::init::finished(observer, "renderer.prewarm.async", stage);
            next += 2;
        }
    }
    Ok(())
}

/// Wait for every compile still in flight after a failure, so that none of
/// them rejects unobserved. The failure itself is already being returned.
async fn abandon(pending: &Array) {
    if wasm_bindgen_futures::JsFuture::from(Promise::all_settled(pending))
        .await
        .is_err()
    {
        // `allSettled` never rejects.
    }
}

#[cfg(target_arch = "wasm32")]
async fn shader_compilation_errors(shader: &wasm_bindgen::JsValue) -> Vec<String> {
    use js_sys::{Array, Function, Promise, Reflect};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;

    // Best-effort diagnostics for a compilation failure that is reported
    // anyway: a browser that cannot describe the failure yields no lines.
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

/// Await one issued compile; a failure carries the module's diagnostics.
async fn settle(compiled: JsValue, module: &JsValue) -> Result<(), String> {
    match wasm_bindgen_futures::JsFuture::from(Promise::from(compiled)).await {
        Ok(_) => Ok(()),
        Err(failure) => {
            let details = shader_compilation_errors(module).await;
            Err(format!("{}; {}", error(failure), details.join("; ")))
        }
    }
}

/// A renderer's background compiles in the browser (#299): the render
/// pipelines of its shader set, issued through `createRenderPipelineAsync`
/// at most [`CompileSchedule::limit`] at a time, what the screen asks for
/// first ([`crate::compile_schedule`]). Shared by the renderer and the
/// futures that await each compile; the page has one thread, so a
/// `RefCell` borrow is never contended and never held across an await. It
/// is single-threaded state, not lock-based synchronization (rule 4).
///
/// Every compile ends in one [`CompileOutcome`]: the future awaiting the
/// browser's promise classifies how it settled, the device's `lost`
/// promise ends every open compile as device lost, an upgrade supersedes
/// the compiles it no longer issues, and dropping the queue cancels the
/// rest. Each ending resolves the queue's change signal, which a waiter
/// awaits instead of polling frames, so a wait always ends.
#[derive(Clone)]
pub(crate) struct CompileQueue(std::rc::Rc<std::cell::RefCell<QueueState>>);

struct QueueState {
    device: JsValue,
    format: wgpu::TextureFormat,
    samples: u32,
    schedule: crate::compile_schedule::CompileSchedule,
    /// The populated descriptor of each scheduled compile, by its index.
    descriptors: Vec<JsValue>,
    /// Resolves at the next change of any compile's state; replaced each
    /// time it resolves.
    changed: ChangeSignal,
}

/// A promise and the function that resolves it.
struct ChangeSignal {
    promise: Promise,
    resolve: Function,
}

impl ChangeSignal {
    fn new() -> Self {
        let mut resolve = None;
        // The executor runs before `Promise::new` returns.
        let promise = Promise::new(&mut |done, _| resolve = Some(done));
        match resolve {
            Some(resolve) => Self { promise, resolve },
            None => Self {
                promise: Promise::resolve(&JsValue::UNDEFINED),
                resolve: Function::new_no_args(""),
            },
        }
    }
}

impl QueueState {
    /// Wake every waiter: resolve the change signal and arm a new one.
    fn notify(&mut self) {
        let signal = std::mem::replace(&mut self.changed, ChangeSignal::new());
        if signal.resolve.call0(&JsValue::UNDEFINED).is_err() {
            // A promise's own resolve function does not throw: there is no
            // failure to report.
        }
    }
}

impl Drop for QueueState {
    /// The renderer is gone: every compile not ended yet ends as cancelled,
    /// and a waiter wakes to see it.
    fn drop(&mut self) {
        self.schedule.cancel_all();
        if self.changed.resolve.call0(&JsValue::UNDEFINED).is_err() {
            // A promise's own resolve function does not throw: there is no
            // failure to report.
        }
    }
}

/// How the browser settled one issued compile.
fn outcome_of(settled: Result<JsValue, JsValue>) -> crate::compile_schedule::CompileOutcome {
    use crate::compile_schedule::{CompileOutcome, RejectReason};
    match settled {
        // The browser also resolves on device loss (with an invalid
        // pipeline); the device's `lost` promise ends the compile as device
        // lost when it settles, and a draw after it takes the device-loss
        // path.
        Ok(_) => CompileOutcome::Ready,
        Err(failure) => {
            let reason = Reflect::get(&failure, &"reason".into())
                .ok()
                .and_then(|reason| reason.as_string());
            CompileOutcome::Rejected {
                reason: RejectReason::from_pipeline_error(reason.as_deref()),
                message: error(failure),
            }
        }
    }
}

impl CompileQueue {
    /// A queue for a renderer that draws to `format` with `samples` per
    /// pixel, keeping as many compiles in flight as the browser reports
    /// cores (`navigator.hardwareConcurrency`). It listens to the device's
    /// `lost` promise, which ends every open compile.
    pub(crate) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<Self, String> {
        let device = JsValue::from(device.as_webgpu().ok_or("WebGPU device required")?.clone());
        let cores = Reflect::get(&js_sys::global(), &"navigator".into())
            .and_then(|navigator| Reflect::get(&navigator, &"hardwareConcurrency".into()))
            .ok()
            .and_then(|cores| cores.as_f64());
        let limit = crate::compile_schedule::compile_limit(cores);
        let lost = get(&device, "lost")?;
        let queue = Self(std::rc::Rc::new(std::cell::RefCell::new(QueueState {
            device,
            format,
            samples,
            schedule: crate::compile_schedule::CompileSchedule::new(limit),
            descriptors: Vec::new(),
            changed: ChangeSignal::new(),
        })));
        // The listener holds the queue weakly: a renderer dropped before
        // its device is lost is not kept alive by it.
        let weak = std::rc::Rc::downgrade(&queue.0);
        let on_lost = wasm_bindgen::closure::Closure::once_into_js(move |_: JsValue| {
            if let Some(state) = weak.upgrade() {
                let mut state = state.borrow_mut();
                state.schedule.device_lost();
                state.notify();
            }
        });
        let then: Function = get(&lost, "then")?
            .dyn_into()
            .map_err(|_| "GPUDevice.lost has no then".to_owned())?;
        then.call1(&lost, &on_lost).map_err(error)?;
        Ok(queue)
    }

    /// Resolves at the next change of any compile's state, or at once when
    /// nothing is waiting or compiling.
    pub(crate) async fn changed(&self) {
        let promise = {
            let state = self.0.borrow();
            if !state.schedule.busy() {
                return;
            }
            state.changed.promise.clone()
        };
        if wasm_bindgen_futures::JsFuture::from(promise).await.is_err() {
            // The change signal is only ever resolved, never rejected: there
            // is no failure to report.
        }
    }

    /// Whether any compile is waiting or in flight.
    pub(crate) fn busy(&self) -> bool {
        self.0.borrow().schedule.busy()
    }

    /// Schedule every render pipeline of `sources` for `set`, in stage
    /// order, after what is already scheduled, and issue as many as the
    /// limit allows. A source's pipelines beyond its listed stages report
    /// under its last stage, as the prewarm always did.
    pub(crate) fn schedule(
        &self,
        shader_set: crate::shader_assembly::ShaderSet,
        sources: &[RenderSource<'_>],
    ) -> Result<(), String> {
        {
            let mut state = self.0.borrow_mut();
            let format = target_format(state.format)?;
            let samples = state.samples;
            for (source, stages) in sources {
                let Some(&(last_stage, _)) = stages.last() else {
                    continue;
                };
                let mut candidates = Vec::new();
                for desc in descriptors(source, "render")? {
                    if !Array::is_array(&get(&desc, "layout")?) {
                        continue;
                    }
                    if get(&get(&desc, "multisample")?, "count")?.as_f64()
                        != Some(f64::from(samples))
                    {
                        continue;
                    }
                    candidates.push(desc);
                }
                let module = shader(&state.device, source, "figgy render prewarm")?;
                let label_of =
                    |desc: &JsValue| get(desc, "label").map(|l| l.as_string().unwrap_or_default());
                // The listed stages first, in order; the rest after them.
                let mut ordered = Vec::new();
                for (stage, label) in stages.iter() {
                    for desc in &candidates {
                        if label_of(desc)? == *label {
                            ordered.push((*stage, desc.clone()));
                        }
                    }
                }
                for desc in &candidates {
                    let label = label_of(desc)?;
                    if !stages.iter().any(|(_, listed)| *listed == label) {
                        ordered.push((last_stage, desc.clone()));
                    }
                }
                for (stage, desc) in ordered {
                    populate(&state.device, &desc, &module)?;
                    for target in Array::from(&get(&get(&desc, "fragment")?, "targets")?).iter() {
                        if !target.is_null()
                            && get(&target, "format")?.as_string().as_deref() != Some("rgba32uint")
                        {
                            set(&target, "format", &format.into())?;
                        }
                    }
                    let label = label_of(&desc)?;
                    state.schedule.push(shader_set, label, stage);
                    state.descriptors.push(desc);
                }
            }
        }
        self.pump();
        Ok(())
    }

    /// Issue compiles while fewer than the limit are in flight.
    fn pump(&self) {
        loop {
            let (index, compiled) = {
                let mut state = self.0.borrow_mut();
                let Some(index) = state.schedule.issue_next() else {
                    return;
                };
                let compiled = call(
                    &state.device,
                    "createRenderPipelineAsync",
                    &state.descriptors[index],
                );
                (index, compiled)
            };
            // The future holds the queue weakly: once its renderer is
            // dropped (which cancels every open compile), a settling compile
            // issues no further one.
            let queue = std::rc::Rc::downgrade(&self.0);
            match compiled {
                Ok(promise) => wasm_bindgen_futures::spawn_local(async move {
                    let settled =
                        wasm_bindgen_futures::JsFuture::from(Promise::from(promise)).await;
                    if let Some(state) = queue.upgrade() {
                        {
                            let mut state = state.borrow_mut();
                            state.schedule.settle(index, outcome_of(settled));
                            state.notify();
                        }
                        CompileQueue(state).pump();
                    }
                }),
                // The call threw: the compile ends here, rejected.
                Err(reason) => {
                    let mut state = self.0.borrow_mut();
                    state.schedule.settle(
                        index,
                        crate::compile_schedule::CompileOutcome::Rejected {
                            reason: crate::compile_schedule::RejectReason::Other,
                            message: reason,
                        },
                    );
                    state.notify();
                }
            }
        }
    }

    /// Issue the waiting compiles of `set` labelled `label` before every
    /// other waiting one.
    pub(crate) fn prioritize(&self, set: crate::shader_assembly::ShaderSet, label: &str) {
        let moved = self.0.borrow_mut().schedule.prioritize(set, label);
        if moved {
            self.pump();
        }
    }

    /// Schedule the module whose render pipelines include `label` when it
    /// is one compiled only on demand (`browser_on_demand_source`: vector,
    /// 3D surface, ternary region) and none of its pipelines is scheduled
    /// for `set` yet, and issue `label` ahead of every waiting compile.
    /// Returns whether `label` is now waiting or compiling, so the frame
    /// leaves its series out; `false` leaves the draw to create it, as for a
    /// pipeline no queue tracks.
    pub(crate) fn schedule_on_demand(
        &self,
        set: crate::shader_assembly::ShaderSet,
        label: &str,
    ) -> Result<bool, String> {
        use crate::compile_schedule::LabelReadiness;
        let Some(source) = crate::data_render::browser_on_demand_source(set, label) else {
            return Ok(false);
        };
        // Scheduled once, by its first listed pipeline: a label alone does
        // not tell, since another module can compile a pipeline with the same
        // label (the field module's floor contour, for one).
        let scheduled = source
            .1
            .first()
            .is_some_and(|(_, first)| self.readiness(set, first) != LabelReadiness::Untracked);
        if !scheduled {
            self.schedule(set, &[source])?;
        }
        self.prioritize(set, label);
        Ok(self.readiness(set, label) == LabelReadiness::Pending)
    }

    /// What a frame may do with the pipelines of `set` labelled `label`.
    pub(crate) fn readiness(
        &self,
        set: crate::shader_assembly::ShaderSet,
        label: &str,
    ) -> crate::compile_schedule::LabelReadiness {
        self.0.borrow().schedule.readiness(set, label)
    }

    /// The compiles of `set` as `(total, ready, compiling, failed)`.
    pub(crate) fn counts(
        &self,
        set: crate::shader_assembly::ShaderSet,
    ) -> (usize, usize, usize, usize) {
        self.0.borrow().schedule.counts(set)
    }

    /// Compiles ended so far; it only grows.
    pub(crate) fn settled(&self) -> u64 {
        self.0.borrow().schedule.settled()
    }

    /// Whether this queue serves a target of `format` and `samples`.
    pub(crate) fn serves(&self, format: wgpu::TextureFormat, samples: u32) -> bool {
        let state = self.0.borrow();
        state.format == format && state.samples == samples
    }

    /// Move to `set` (a renderer never goes back): `set`'s compiles are
    /// scheduled, and the other set's compiles not issued yet end as
    /// superseded by their twins in `set`.
    pub(crate) fn switch_to(
        &self,
        from: crate::shader_assembly::ShaderSet,
        set: crate::shader_assembly::ShaderSet,
        sources: &[RenderSource<'_>],
    ) -> Result<(), String> {
        self.schedule(set, sources)?;
        let mut state = self.0.borrow_mut();
        state.schedule.supersede(from, set);
        state.notify();
        Ok(())
    }

    /// Wait for every scheduled compile of `set` whose label `include`
    /// accepts, issuing them first, and report each stage started and
    /// finished in schedule order. It wakes on each change of a compile's
    /// state, never by polling, and ends: a compile superseded by an
    /// upgrade is followed to its replacement, and a compile that ended in
    /// any way but ready fails the wait with how it ended.
    pub(crate) async fn wait_for(
        &self,
        set: crate::shader_assembly::ShaderSet,
        include: impl Fn(&str) -> bool,
        observer: &mut dyn FnMut(crate::InitEvent),
    ) -> Result<(), String> {
        // The stages in schedule order, each with its compiles.
        let indices: Vec<usize> = {
            let state = self.0.borrow();
            (0..state.schedule.len())
                .filter(|&i| {
                    let compile = state.schedule.compile(i);
                    compile.set == set && include(&compile.label)
                })
                .collect()
        };
        for &index in indices.iter().rev() {
            let label = self.0.borrow().schedule.compile(index).label.clone();
            self.prioritize(set, &label);
        }
        let mut position = 0;
        while position < indices.len() {
            let stage = self.0.borrow().schedule.compile(indices[position]).stage;
            let end = indices[position..]
                .iter()
                .position(|&i| self.0.borrow().schedule.compile(i).stage != stage)
                .map_or(indices.len(), |n| position + n);
            crate::init::started(observer, "renderer.prewarm.async", stage);
            for &index in &indices[position..end] {
                loop {
                    let step = self.0.borrow().schedule.wait_step(index);
                    match step {
                        WaitStep::Pending(_) => self.changed().await,
                        WaitStep::Ended(CompileOutcome::Ready) => break,
                        WaitStep::Ended(ended) => return Err(ended_message(&ended)),
                    }
                }
            }
            crate::init::finished(observer, "renderer.prewarm.async", stage);
            position = end;
        }
        Ok(())
    }
}

/// Why a waited-for compile did not end ready.
fn ended_message(ended: &CompileOutcome) -> String {
    match ended {
        CompileOutcome::Ready => "the pipeline compiled".to_owned(),
        CompileOutcome::Rejected { reason, message } => {
            format!("the browser rejected the pipeline ({reason:?}): {message}")
        }
        CompileOutcome::DeviceLost => "the GPU device was lost".to_owned(),
        CompileOutcome::Superseded { .. } | CompileOutcome::Retired => {
            "the renderer moved to the other shader set".to_owned()
        }
        CompileOutcome::Cancelled => "the renderer was dropped".to_owned(),
    }
}

/// The JS name of a target format the contract's descriptors take.
fn target_format(format: wgpu::TextureFormat) -> Result<&'static str, String> {
    Ok(match format {
        wgpu::TextureFormat::Rgba8Unorm => "rgba8unorm",
        wgpu::TextureFormat::Bgra8Unorm => "bgra8unorm",
        wgpu::TextureFormat::Rgba8UnormSrgb => "rgba8unorm-srgb",
        wgpu::TextureFormat::Bgra8UnormSrgb => "bgra8unorm-srgb",
        other => return Err(format!("Unsupported prewarm target: {other:?}")),
    })
}
