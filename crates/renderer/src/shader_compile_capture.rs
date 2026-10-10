use serde_json::{Value, json};
use std::{cell::RefCell, collections::HashMap, time::Duration};

#[derive(Default)]
pub(crate) struct Capture {
    shaders: HashMap<wgpu::ShaderModule, usize>,
    bindings: HashMap<wgpu::BindGroupLayout, Value>,
    layouts: HashMap<wgpu::PipelineLayout, Value>,
    pub sources: Vec<Value>,
    pub pipelines: Vec<Value>,
    pub timings: Vec<Value>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Capture>> = const { RefCell::new(None) };
}

pub(crate) struct Session;
impl Session {
    pub fn begin() -> Self {
        ACTIVE.with(|slot| assert!(slot.borrow_mut().replace(Capture::default()).is_none()));
        Self
    }
    pub fn finish(self) -> Capture {
        ACTIVE.with(|slot| slot.borrow_mut().take().unwrap())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        ACTIVE.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}

fn record(f: impl FnOnce(&mut Capture)) {
    ACTIVE.with(|slot| {
        if let Some(c) = slot.borrow_mut().as_mut() {
            f(c);
        }
    });
}

pub(crate) fn counts() -> (usize, usize) {
    ACTIVE.with(|slot| {
        let slot = slot.borrow();
        let c = slot.as_ref().unwrap();
        (c.timings.len(), c.pipelines.len())
    })
}

fn web(value: impl std::fmt::Debug) -> String {
    let s = format!("{value:?}");
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('-');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

pub(crate) fn shader(
    shader: &wgpu::ShaderModule,
    desc: &wgpu::ShaderModuleDescriptor<'_>,
    elapsed: Duration,
) {
    record(|c| {
        let wgpu::ShaderSource::Wgsl(code) = &desc.source else {
            panic!("Unmeasured shader source");
        };
        let index = c
            .sources
            .iter()
            .position(|v| v["code"] == code.as_ref())
            .unwrap_or_else(|| {
                let index = c.sources.len();
                c.sources.push(json!({"label":desc.label,"code":code}));
                index
            });
        c.shaders.insert(shader.clone(), index);
        c.timings.push(json!({"kind":"module","source":index,"label":desc.label,"ms":elapsed.as_secs_f64()*1000.0}));
    });
}

pub(crate) fn bindings(layout: &wgpu::BindGroupLayout, desc: &wgpu::BindGroupLayoutDescriptor<'_>) {
    record(|c| {
        let entries: Vec<_> = desc.entries.iter().map(|e| {
            assert!(e.count.is_none(), "Binding arrays require benchmark support");
            let mut out = json!({"binding":e.binding,"visibility":e.visibility.bits()});
            match e.ty {
                wgpu::BindingType::Buffer { ty, has_dynamic_offset, min_binding_size } => {
                    let ty = match ty {
                        wgpu::BufferBindingType::Uniform => "uniform",
                        wgpu::BufferBindingType::Storage { read_only: true } => "read-only-storage",
                        wgpu::BufferBindingType::Storage { read_only: false } => "storage",
                    };
                    out["buffer"] = json!({"type":ty,"hasDynamicOffset":has_dynamic_offset,"minBindingSize":min_binding_size.map_or(0, |x| x.get())});
                }
                wgpu::BindingType::Sampler(ty) => out["sampler"] = json!({"type":web(ty)}),
                wgpu::BindingType::Texture { sample_type, view_dimension, multisampled } => {
                    let sample_type = match sample_type {
                        wgpu::TextureSampleType::Float { filterable: true } => "float",
                        wgpu::TextureSampleType::Float { filterable: false } => "unfilterable-float",
                        wgpu::TextureSampleType::Depth => "depth",
                        wgpu::TextureSampleType::Sint => "sint",
                        wgpu::TextureSampleType::Uint => "uint",
                    };
                    let dimension = match view_dimension {
                        wgpu::TextureViewDimension::D1 => "1d",
                        wgpu::TextureViewDimension::D2 => "2d",
                        wgpu::TextureViewDimension::D2Array => "2d-array",
                        wgpu::TextureViewDimension::Cube => "cube",
                        wgpu::TextureViewDimension::CubeArray => "cube-array",
                        wgpu::TextureViewDimension::D3 => "3d",
                    };
                    out["texture"] = json!({"sampleType":sample_type,"viewDimension":dimension,"multisampled":multisampled});
                }
                other => panic!("Unmeasured binding type: {other:?}"),
            }
            out
        }).collect();
        c.bindings.insert(layout.clone(), json!(entries));
    });
}

pub(crate) fn layout(layout: &wgpu::PipelineLayout, desc: &wgpu::PipelineLayoutDescriptor<'_>) {
    record(|c| {
        assert_eq!(desc.immediate_size, 0);
        let groups: Vec<_> = desc
            .bind_group_layouts
            .iter()
            .map(|g| match g {
                Some(g) => c
                    .bindings
                    .get(g)
                    .expect("Bind group layout escaped capture")
                    .clone(),
                None => Value::Null,
            })
            .collect();
        c.layouts.insert(layout.clone(), json!(groups));
    });
}

impl Capture {
    fn layout(&self, layout: Option<&wgpu::PipelineLayout>) -> Value {
        layout.map_or_else(
            || json!("auto"),
            |l| {
                self.layouts
                    .get(l)
                    .expect("Pipeline layout escaped capture")
                    .clone()
            },
        )
    }
    fn stage(
        &self,
        module: &wgpu::ShaderModule,
        entry: Option<&str>,
        options: &wgpu::PipelineCompilationOptions<'_>,
    ) -> Value {
        assert!(
            options.zero_initialize_workgroup_memory,
            "Benchmark must retain workgroup initialization"
        );
        json!({"module":self.shaders.get(module).expect("Shader escaped capture"),"entryPoint":entry.expect("Explicit entry required"),"constants":options.constants.iter().map(|(k,v)| (k.to_string(),*v)).collect::<std::collections::BTreeMap<_,_>>()})
    }
    fn pipeline(&mut self, kind: &str, desc: Value, elapsed: Duration) {
        let index = self.pipelines.len();
        self.timings.push(json!({"kind":kind,"pipeline":index,"label":desc["label"],"ms":elapsed.as_secs_f64()*1000.0}));
        self.pipelines.push(json!({"kind":kind,"descriptor":desc}));
    }
    pub fn json(&self) -> Value {
        json!({"schema_version":1,"sources":self.sources,"pipelines":self.pipelines,"timings":self.timings})
    }

    pub fn browser_contract(&self) -> Value {
        let mut layouts = Vec::new();
        for pipeline in &self.pipelines {
            let layout = &pipeline["descriptor"]["layout"];
            if layout.is_array() && !layouts.contains(layout) {
                layouts.push(layout.clone());
            }
        }
        layouts.sort_by_key(Value::to_string);
        let mut pipelines = Vec::new();
        for pipeline in &self.pipelines {
            let mut p = pipeline.clone();
            let desc = &mut p["descriptor"];
            let mut source = None;
            for stage in ["compute", "vertex", "fragment"] {
                if !desc[stage].is_object() {
                    continue;
                }
                let index = desc[stage]["module"].as_u64().unwrap() as usize;
                let key = super::source_key(self.sources[index]["code"].as_str().unwrap());
                if let Some(previous) = &source {
                    assert_eq!(
                        previous, &key,
                        "Mixed-module pipeline needs a separate contract"
                    );
                }
                source = Some(key);
                desc[stage].as_object_mut().unwrap().remove("module");
            }
            if desc["layout"].is_array() {
                desc["layout"] = json!(layouts.iter().position(|l| l == &desc["layout"]).unwrap());
            }
            p["source"] = json!(source.unwrap());
            if !pipelines.contains(&p) {
                pipelines.push(p);
            }
        }
        pipelines.sort_by_key(Value::to_string);
        json!({"schema_version":1,"layouts":layouts,"pipelines":pipelines})
    }
}

pub(crate) fn compute(desc: &wgpu::ComputePipelineDescriptor<'_>, elapsed: Duration) {
    record(|c| {
        let value = json!({"label":desc.label,"layout":c.layout(desc.layout),"compute":c.stage(desc.module,desc.entry_point,&desc.compilation_options)});
        c.pipeline("compute", value, elapsed);
    });
}

fn blend(component: wgpu::BlendComponent) -> Value {
    json!({"srcFactor":web(component.src_factor),"dstFactor":web(component.dst_factor),"operation":web(component.operation)})
}

pub(crate) fn render(desc: &wgpu::RenderPipelineDescriptor<'_>, elapsed: Duration) {
    record(|c| {
        assert!(
            desc.depth_stencil.is_none(),
            "Add depth/stencil to the benchmark before introducing a new pipeline"
        );
        assert!(desc.multiview_mask.is_none());
        assert_eq!(desc.primitive.polygon_mode, wgpu::PolygonMode::Fill);
        assert!(!desc.primitive.conservative);
        let mut vertex = c.stage(
            desc.vertex.module,
            desc.vertex.entry_point,
            &desc.vertex.compilation_options,
        );
        vertex["buffers"] = json!(desc.vertex.buffers.iter().map(|b| b.as_ref().map(|b| json!({
            "arrayStride":b.array_stride,"stepMode":web(b.step_mode),
            "attributes":b.attributes.iter().map(|a| json!({"format":format!("{:?}",a.format).to_lowercase(),"offset":a.offset,"shaderLocation":a.shader_location})).collect::<Vec<_>>()
        }))).collect::<Vec<_>>());
        let fragment = desc
            .fragment
            .as_ref()
            .expect("Add vertex-only pipeline support");
        let mut fragment_value = c.stage(
            fragment.module,
            fragment.entry_point,
            &fragment.compilation_options,
        );
        fragment_value["targets"] = json!(fragment.targets.iter().map(|t| t.as_ref().map(|t| {
            let mut value = json!({"format":format!("{:?}",t.format).to_lowercase().replace("srgb", "-srgb"),"writeMask":t.write_mask.bits()});
            if let Some(b) = t.blend { value["blend"] = json!({"color":blend(b.color),"alpha":blend(b.alpha)}); }
            value
        })).collect::<Vec<_>>());
        let mut primitive = json!({"topology":web(desc.primitive.topology),"frontFace":web(desc.primitive.front_face),"cullMode":desc.primitive.cull_mode.map_or("none".into(),web),"unclippedDepth":desc.primitive.unclipped_depth});
        if let Some(format) = desc.primitive.strip_index_format {
            primitive["stripIndexFormat"] = json!(web(format));
        }
        let value = json!({"label":desc.label,"layout":c.layout(desc.layout),"vertex":vertex,"fragment":fragment_value,"primitive":primitive,
            "multisample":{"count":desc.multisample.count,"mask":desc.multisample.mask as u32,"alphaToCoverageEnabled":desc.multisample.alpha_to_coverage_enabled}});
        c.pipeline("render", value, elapsed);
    });
}
