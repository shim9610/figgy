//! GPU reduction for drawable-series and legacy errorbar fit bounds.
//!
//! The column pool remains the sole per-value source. The initial pass binds
//! that pool once, addresses packed `(hi, lo)` lanes through checked offsets,
//! and exhaustively reduces the rows that can produce a primitive. No
//! value-sized CPU allocation, shadow column, sampling, or downsampling is
//! involved. Readback contains only the six `(hi, lo)` bounds consumed by the
//! CPU-owned axis SSoT; no source endpoint or errorbar provenance crosses the
//! GPU boundary.

use std::num::NonZeroU64;
use std::sync::Arc;

use futures_channel::oneshot;

use crate::data::COLUMN_VALUE_BYTES;
use crate::data_config::DataRenderType;
use crate::data_render::ColumnHandle;
use crate::gpu_memory::{
    ChargeTally, GpuByteCharge, GpuLedger, GpuResourceKind, charged_buffer, charged_buffer_init,
};
use crate::init::{InitEvent, finished, observe_value, started};

const INIT_SCOPE: &str = "renderer.errorbar_extent";

const WORKGROUP_SIZE: u32 = 64;
const LEGACY_AXIS_MODE: u32 = 5;
const FIELD_EDGES_CELLS_MODE: u32 = 6;
const FIELD_EDGES_SAMPLES_MODE: u32 = 7;
const FIELD_CENTERS_CELLS_MODE: u32 = 8;
const FIELD_CENTERS_SAMPLES_MODE: u32 = 9;
const EMPTY_MINIMUM_BOUND: [f32; 2] = [f32::MAX, f32::MAX];
const EMPTY_MAXIMUM_BOUND: [f32; 2] = [-f32::MAX, -f32::MAX];

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct AxisStateGpu {
    minimum: [f32; 2],
    maximum: [f32; 2],
    minimum_positive: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SeriesStateGpu {
    x: AxisStateGpu,
    y: AxisStateGpu,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ParamsGpu {
    offsets_0: [u32; 4],
    offsets_1: [u32; 4],
    lengths_0: [u32; 4],
    lengths_1: [u32; 4],
}

#[derive(Clone, Copy)]
struct ColumnRangeGpu {
    offset: u32,
    len: u32,
}

const SERIES_STATE_BYTES: u64 = std::mem::size_of::<SeriesStateGpu>() as u64;

const _: [(); 24] = [(); std::mem::size_of::<AxisStateGpu>()];
const _: [(); 48] = [(); std::mem::size_of::<SeriesStateGpu>()];
const _: [(); 64] = [(); std::mem::size_of::<ParamsGpu>()];

/// Scalar fit extent reconstructed from the GPU-reduced `(hi, lo)` bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuErrorbarExtent {
    pub min: f64,
    pub max: f64,
    pub min_positive: Option<f64>,
}

/// Exact paired x/y result for one drawable series.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuSeriesExtent {
    pub x: GpuErrorbarExtent,
    pub y: GpuErrorbarExtent,
}

/// Normalized primitive domains used by the series fit-bound reducer.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GpuSeriesExtentMode {
    /// Only endpoints belonging to a finite adjacent line segment.
    Line = 0,
    /// Every finite paired x/y point.
    Points = 1,
    /// Finite paired points plus active x errorbar endpoints.
    PointsX = 2,
    /// Finite paired points plus active y errorbar endpoints.
    PointsY = 3,
    /// Finite paired points plus active x and y errorbar endpoints.
    PointsXY = 4,
}

/// The coordinate lattice whose exact outer bounds a matrix-backed primitive
/// paints. These four cases mirror `field_columnar.wgsl`'s
/// `cell_edge_pair`/`sample_point_pair` choice; the CPU supplies only the
/// resolved cell counts, while the GPU reads and combines the coordinate pairs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GpuFieldExtentMode {
    EdgesCells,
    EdgesSamples,
    CentersCells,
    CentersSamples,
}

impl GpuFieldExtentMode {
    fn shader_mode(self) -> u32 {
        match self {
            Self::EdgesCells => FIELD_EDGES_CELLS_MODE,
            Self::EdgesSamples => FIELD_EDGES_SAMPLES_MODE,
            Self::CentersCells => FIELD_CENTERS_CELLS_MODE,
            Self::CentersSamples => FIELD_CENTERS_SAMPLES_MODE,
        }
    }
}

/// One normalized GPU fit domain. Paired point/line/errorbar series reduce all
/// drawable rows; fields read only the outer coordinate pairs of the exact
/// lattice their render entry uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GpuSeriesFitMode {
    Paired(GpuSeriesExtentMode),
    Field(GpuFieldExtentMode),
}

impl GpuSeriesFitMode {
    pub fn from_render_type(render_type: &DataRenderType) -> Option<Self> {
        if let Some(mode) = GpuSeriesExtentMode::from_render_type(render_type) {
            return Some(Self::Paired(mode));
        }
        use crate::data_config::{GridLayout, Shading};
        let (matrix, samples) = match render_type {
            DataRenderType::Heatmap { matrix, fill } => {
                (matrix, matches!(fill.shading, Shading::Interpolated))
            }
            DataRenderType::Contour { matrix, .. } => (matrix, true),
            DataRenderType::HeatmapContour { matrix, fill, .. } => {
                // The fit covers the union of fill and isolines. A flat fill
                // reaches cell edges; an interpolated fill and the contour both
                // stop at sample points.
                (matrix, matches!(fill.shading, Shading::Interpolated))
            }
            DataRenderType::Histogram { .. } => return None,
            DataRenderType::Scatter { .. }
            | DataRenderType::Line { .. }
            | DataRenderType::ScatterLine { .. }
            | DataRenderType::ScatterErrorbarX { .. }
            | DataRenderType::ScatterErrorbarY { .. }
            | DataRenderType::ScatterErrorbarXY { .. }
            | DataRenderType::LineScatterErrorbarX { .. }
            | DataRenderType::LineScatterErrorbarY { .. }
            | DataRenderType::LineScatterErrorbarXY { .. } => {
                unreachable!("paired render types returned above")
            }
        };
        let mode = match (&matrix.grid_layout, samples) {
            (GridLayout::Edges, false) => GpuFieldExtentMode::EdgesCells,
            (GridLayout::Edges, true) => GpuFieldExtentMode::EdgesSamples,
            (GridLayout::Centers, false) => GpuFieldExtentMode::CentersCells,
            (GridLayout::Centers, true) => GpuFieldExtentMode::CentersSamples,
        };
        Some(Self::Field(mode))
    }
}

impl GpuSeriesExtentMode {
    /// The paired-reducer domain for a render type, or `None` when its extent
    /// does not come from this reducer at all.
    ///
    /// Every mode here reduces over **index-aligned pairs** — pair `i` is
    /// `(x[i], y[i])`, and the pass runs to `min(x.len(), y.len())`. That is the
    /// right domain for a series whose two columns are the same points, and the
    /// wrong one for the four field / bar types, in a way that would not look
    /// like a failure:
    ///
    /// - A histogram is `(edges = n + 1, counts = n)`. Pairing stops at `n`, so
    ///   the last edge never reaches the x extent and auto-fit clips the final
    ///   bar's far side.
    /// - A matrix' `x_column` and `y_column` are the grid's two coordinate axes
    ///   with independent lengths. Pairing a 100-wide x against a 50-tall y
    ///   would report x's extent as `x[49]`, and auto-fit would show half the
    ///   field.
    ///
    /// Histograms are fitted synchronously from their edge/count metadata.
    /// Matrix fields instead use [`GpuSeriesFitMode::Field`], because their
    /// rendered outer bounds may be derived midpoints or extrapolated half-cells
    /// rather than either coordinate column's raw minimum and maximum.
    pub fn from_render_type(render_type: &DataRenderType) -> Option<Self> {
        match render_type {
            DataRenderType::Line { .. } => Some(Self::Line),
            DataRenderType::Scatter { .. } | DataRenderType::ScatterLine { .. } => {
                Some(Self::Points)
            }
            DataRenderType::ScatterErrorbarX { .. }
            | DataRenderType::LineScatterErrorbarX { .. } => Some(Self::PointsX),
            DataRenderType::ScatterErrorbarY { .. }
            | DataRenderType::LineScatterErrorbarY { .. } => Some(Self::PointsY),
            DataRenderType::ScatterErrorbarXY { .. }
            | DataRenderType::LineScatterErrorbarXY { .. } => Some(Self::PointsXY),
            DataRenderType::Histogram { .. }
            | DataRenderType::Heatmap { .. }
            | DataRenderType::Contour { .. }
            | DataRenderType::HeatmapContour { .. } => None,
        }
    }

    fn has_x_errors(self) -> bool {
        matches!(self, Self::PointsX | Self::PointsXY)
    }

    fn has_y_errors(self) -> bool {
        matches!(self, Self::PointsY | Self::PointsXY)
    }
}

/// Role-labelled renderer column ids for a series extent submission.
///
/// Only active error directions are supplied. Inactive error directions are
/// virtual zero lanes and do not participate in the error-domain minimum.
#[derive(Clone, Copy, Debug)]
pub struct GpuSeriesExtentColumnIds<'a> {
    pub x: &'a str,
    pub y: &'a str,
    pub x_lower: Option<&'a str>,
    pub x_upper: Option<&'a str>,
    pub y_lower: Option<&'a str>,
    pub y_upper: Option<&'a str>,
}

/// Resolved live handles after Renderer has validated ids/generations.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GpuSeriesExtentColumns {
    pub(crate) x: ColumnHandle,
    pub(crate) y: ColumnHandle,
    pub(crate) x_lower: Option<ColumnHandle>,
    pub(crate) x_upper: Option<ColumnHandle>,
    pub(crate) y_lower: Option<ColumnHandle>,
    pub(crate) y_upper: Option<ColumnHandle>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct GpuFieldExtentColumns {
    pub(crate) x: ColumnHandle,
    pub(crate) y: ColumnHandle,
    pub(crate) x_cells: u32,
    pub(crate) y_cells: u32,
}

/// Validation, submission, or detached-readback failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuErrorbarError {
    UnknownColumn {
        role: &'static str,
        id: String,
    },
    StaleHandle {
        role: &'static str,
        id: String,
        generation: u32,
        current: u32,
    },
    MissingSeriesColumn {
        role: &'static str,
        mode: GpuSeriesExtentMode,
    },
    EmptyColumn {
        role: &'static str,
    },
    ValueCountTooLarge {
        role: &'static str,
        len: usize,
    },
    ColumnOffsetTooLarge {
        role: &'static str,
        offset: u64,
    },
    InvalidColumnRange {
        role: &'static str,
        offset: u64,
        required: u64,
        available: u64,
        pool_size: u64,
    },
    MisalignedColumnOffset {
        role: &'static str,
        offset: u64,
        alignment: u32,
    },
    StorageBindingTooLarge {
        role: &'static str,
        requested: u64,
        limit: u64,
    },
    NoDispatchCapacity,
    /// All hosts must complete mutable `ensure_errorbar_extent_engine`
    /// preparation before shared extent submission. The shared path only reads
    /// a successfully published engine and never creates compute pipelines.
    EngineNotReady,
    AsyncCompileFailed(String),
    ReadbackSenderDropped,
    ReadbackMapFailed(String),
    DevicePollFailed(String),
    CorruptResult,
}

impl std::fmt::Display for GpuErrorbarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownColumn { role, id } => write!(f, "unknown {role} column id: {id}"),
            Self::StaleHandle {
                role,
                id,
                generation,
                current,
            } => write!(
                f,
                "stale {role} column handle for {id} \
                 (handle generation {generation}, pool generation {current})"
            ),
            Self::MissingSeriesColumn { role, mode } => {
                write!(f, "{mode:?} series extent requires a {role} column")
            }
            Self::EmptyColumn { role } => write!(f, "{role} column is empty"),
            Self::ValueCountTooLarge { role, len } => {
                write!(f, "{role} column length {len} exceeds u32 indexing")
            }
            Self::ColumnOffsetTooLarge { role, offset } => {
                write!(f, "{role} pool offset {offset} exceeds u32 lane indexing")
            }
            Self::InvalidColumnRange {
                role,
                offset,
                required,
                available,
                pool_size,
            } => write!(
                f,
                "{role} pool range is invalid: offset={offset}, required={required}, \
                 handle_bytes={available}, pool_bytes={pool_size}"
            ),
            Self::MisalignedColumnOffset {
                role,
                offset,
                alignment,
            } => write!(
                f,
                "{role} pool offset {offset} is not aligned to {alignment} bytes"
            ),
            Self::StorageBindingTooLarge {
                role,
                requested,
                limit,
            } => write!(
                f,
                "{role} storage binding requires {requested} bytes, device limit is {limit}"
            ),
            Self::NoDispatchCapacity => write!(f, "device exposes no usable extent dispatch"),
            Self::EngineNotReady => write!(
                f,
                "extent engine is not ready; call ensure_errorbar_extent_engine first"
            ),
            Self::AsyncCompileFailed(reason) => {
                write!(f, "async extent pipeline compile failed: {reason}")
            }
            Self::ReadbackSenderDropped => write!(f, "extent readback callback sender was dropped"),
            Self::ReadbackMapFailed(reason) => write!(f, "extent readback map failed: {reason}"),
            Self::DevicePollFailed(reason) => {
                write!(f, "extent submission poll failed: {reason}")
            }
            Self::CorruptResult => write!(f, "GPU returned an invalid extent record"),
        }
    }
}

impl std::error::Error for GpuErrorbarError {}

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

#[cfg(target_arch = "wasm32")]
async fn warm_extent_pipelines_js(device: &wgpu::Device) -> Result<(), GpuErrorbarError> {
    use js_sys::{Array, Function, Object, Promise, Reflect};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;

    let gpu_device = device.as_webgpu().ok_or_else(|| {
        GpuErrorbarError::AsyncCompileFailed("wgpu device is not a WebGPU handle".into())
    })?;
    let device_js = JsValue::from(gpu_device.clone());

    let js_err = |error: JsValue| {
        GpuErrorbarError::AsyncCompileFailed(
            error.as_string().unwrap_or_else(|| format!("{error:?}")),
        )
    };
    let call1 = |name: &str, arg: &JsValue| -> Result<JsValue, GpuErrorbarError> {
        let method = Reflect::get(&device_js, &JsValue::from_str(name)).map_err(js_err)?;
        let method = method
            .dyn_into::<Function>()
            .map_err(|_| GpuErrorbarError::AsyncCompileFailed(format!("missing {name}")))?;
        method.call1(&device_js, arg).map_err(js_err)
    };
    let set = |obj: &Object, key: &str, value: &JsValue| -> Result<(), GpuErrorbarError> {
        Reflect::set(obj, &JsValue::from_str(key), value).map_err(js_err)?;
        Ok(())
    };

    let shader_desc = Object::new();
    set(
        &shader_desc,
        "label",
        &JsValue::from_str("figgy series fit-bound shader"),
    )?;
    set(
        &shader_desc,
        "code",
        &JsValue::from_str(include_str!("gpu_errorbar.wgsl")),
    )?;
    let shader = call1("createShaderModule", shader_desc.as_ref())?;

    let compute_vis = JsValue::from_f64(4.0);
    let read_only = Object::new();
    set(&read_only, "type", &JsValue::from_str("read-only-storage"))?;
    let storage = Object::new();
    set(&storage, "type", &JsValue::from_str("storage"))?;
    let uniform = Object::new();
    set(&uniform, "type", &JsValue::from_str("uniform"))?;
    set(
        &uniform,
        "minBindingSize",
        &JsValue::from_f64(std::mem::size_of::<ParamsGpu>() as f64),
    )?;

    let entry = |binding: u32, buffer: &Object| -> Result<Object, GpuErrorbarError> {
        let object = Object::new();
        set(&object, "binding", &JsValue::from_f64(f64::from(binding)))?;
        set(&object, "visibility", &compute_vis)?;
        set(&object, "buffer", buffer.as_ref())?;
        Ok(object)
    };
    let entries = Array::of3(
        entry(0, &read_only)?.as_ref(),
        entry(1, &storage)?.as_ref(),
        entry(2, &uniform)?.as_ref(),
    );
    let layout_desc = Object::new();
    set(&layout_desc, "entries", entries.as_ref())?;
    let values_bgl = call1("createBindGroupLayout", layout_desc.as_ref())?;
    let states_bgl = call1("createBindGroupLayout", layout_desc.as_ref())?;

    let pipeline_layout = |label: &str, bgl: &JsValue| -> Result<JsValue, GpuErrorbarError> {
        let desc = Object::new();
        set(&desc, "label", &JsValue::from_str(label))?;
        set(&desc, "bindGroupLayouts", Array::of1(bgl).as_ref())?;
        call1("createPipelineLayout", desc.as_ref())
    };
    let values_layout = pipeline_layout("figgy series fit values pipeline layout", &values_bgl)?;
    let states_layout = pipeline_layout("figgy series fit states pipeline layout", &states_bgl)?;

    for (label, layout, entry_point) in [
        (
            "figgy series fit initial pipeline",
            values_layout,
            "reduce_values",
        ),
        (
            "figgy series fit state pipeline",
            states_layout,
            "reduce_states",
        ),
    ] {
        let stage = Object::new();
        set(&stage, "module", &shader)?;
        set(&stage, "entryPoint", &JsValue::from_str(entry_point))?;
        let desc = Object::new();
        set(&desc, "label", &JsValue::from_str(label))?;
        set(&desc, "layout", &layout)?;
        set(&desc, "compute", stage.as_ref())?;
        let promise = call1("createComputePipelineAsync", desc.as_ref())?;
        if let Err(error) = JsFuture::from(Promise::from(promise)).await {
            let mut reason = error.as_string().unwrap_or_else(|| format!("{error:?}"));
            let diagnostics = shader_compilation_errors(&shader).await;
            if !diagnostics.is_empty() {
                reason.push_str("; shader diagnostics: ");
                reason.push_str(&diagnostics.join(" | "));
            }
            return Err(GpuErrorbarError::AsyncCompileFailed(reason));
        }
    }
    Ok(())
}

/// Pipelines and layouts intended to be created once and owned by Renderer.
pub struct GpuErrorbarExtentEngine {
    values_layout: wgpu::BindGroupLayout,
    states_layout: wgpu::BindGroupLayout,
    reduce_values: wgpu::ComputePipeline,
    reduce_states: wgpu::ComputePipeline,
    /// Where per-ticket scratch is charged. Held by the engine rather than
    /// passed per call so the whole `begin_*` chain keeps its signatures — the
    /// engine outlives every ticket it issues.
    ledger: Arc<GpuLedger>,
}

impl GpuErrorbarExtentEngine {
    /// Engine whose scratch is charged to a ledger only it can see.
    ///
    /// For callers with no renderer to report to (tests, standalone use). The
    /// renderer builds its engine with [`Self::new_tracked`] so extent work
    /// shows up in `Renderer::gpu_memory_usage`.
    pub fn new(device: &wgpu::Device) -> Self {
        // host-alloc: W1-a
        Self::new_tracked(device, Arc::new(GpuLedger::new()))
    }

    /// Engine whose per-ticket scratch is charged to `ledger`.
    pub fn new_tracked(device: &wgpu::Device, ledger: Arc<GpuLedger>) -> Self {
        let mut noop = |_| {};
        Self::new_observed_tracked(device, ledger, &mut noop)
    }

    pub fn new_observed(device: &wgpu::Device, observer: &mut dyn FnMut(InitEvent)) -> Self {
        // host-alloc: W1-a
        Self::new_observed_tracked(device, Arc::new(GpuLedger::new()), observer)
    }

    pub fn new_observed_tracked(
        device: &wgpu::Device,
        ledger: Arc<GpuLedger>,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Self {
        observe_value(observer, INIT_SCOPE, "limits", || device.limits());
        started(observer, INIT_SCOPE, "setup");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figgy series fit-bound shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_errorbar.wgsl").into()),
        });

        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let uniform = wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(std::mem::size_of::<ParamsGpu>() as u64),
            },
            count: None,
        };
        let make_layout = |label| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries: &[storage(0, true), storage(1, false), uniform],
            })
        };
        let values_layout = make_layout("figgy series fit values layout");
        let states_layout = make_layout("figgy series fit states layout");

        let values_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("figgy series fit values pipeline layout"),
                bind_group_layouts: &[Some(&values_layout)],
                immediate_size: 0,
            });
        let states_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("figgy series fit states pipeline layout"),
                bind_group_layouts: &[Some(&states_layout)],
                immediate_size: 0,
            });
        finished(observer, INIT_SCOPE, "setup");
        let make_pipeline = |label, layout, entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let reduce_values = observe_value(observer, INIT_SCOPE, "reduce_values", || {
            make_pipeline(
                "figgy series fit initial pipeline",
                &values_pipeline_layout,
                "reduce_values",
            )
        });
        let reduce_states = observe_value(observer, INIT_SCOPE, "reduce_states", || {
            make_pipeline(
                "figgy series fit state pipeline",
                &states_pipeline_layout,
                "reduce_states",
            )
        });

        Self {
            values_layout,
            states_layout,
            reduce_values,
            reduce_states,
            ledger,
        }
    }

    pub async fn new_observed_async(
        device: &wgpu::Device,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Self {
        // No renderer to report to on this path; see `new`.
        // host-alloc: W1-a
        let ledger = Arc::new(GpuLedger::new());
        use crate::init::{observe_value_async, yield_init_frame};
        observe_value_async(observer, INIT_SCOPE, "limits", || device.limits()).await;
        started(observer, INIT_SCOPE, "setup");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("figgy series fit-bound shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_errorbar.wgsl").into()),
        });

        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let uniform = wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(std::mem::size_of::<ParamsGpu>() as u64),
            },
            count: None,
        };
        let make_layout = |label| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries: &[storage(0, true), storage(1, false), uniform],
            })
        };
        let values_layout = make_layout("figgy series fit values layout");
        let states_layout = make_layout("figgy series fit states layout");

        let values_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("figgy series fit values pipeline layout"),
                bind_group_layouts: &[Some(&values_layout)],
                immediate_size: 0,
            });
        let states_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("figgy series fit states pipeline layout"),
                bind_group_layouts: &[Some(&states_layout)],
                immediate_size: 0,
            });
        finished(observer, INIT_SCOPE, "setup");
        yield_init_frame().await;
        let make_pipeline = |label, layout, entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let reduce_values = observe_value_async(observer, INIT_SCOPE, "reduce_values", || {
            make_pipeline(
                "figgy series fit initial pipeline",
                &values_pipeline_layout,
                "reduce_values",
            )
        })
        .await;
        let reduce_states = observe_value_async(observer, INIT_SCOPE, "reduce_states", || {
            make_pipeline(
                "figgy series fit state pipeline",
                &states_pipeline_layout,
                "reduce_states",
            )
        })
        .await;

        Self {
            values_layout,
            states_layout,
            reduce_values,
            reduce_states,
            ledger,
        }
    }

    /// Compile the extent compute pipelines through the browser's async API.
    ///
    /// Native is a no-op. On wasm this calls `createComputePipelineAsync` so
    /// Dawn/driver compilation is not deferred to the first `queue.submit()`.
    /// The resulting JS pipelines are discarded; the wgpu engine created
    /// afterwards should hit the device compilation cache.
    pub async fn warm_device_async(device: &wgpu::Device) -> Result<(), GpuErrorbarError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = device;
            Ok(())
        }
        #[cfg(target_arch = "wasm32")]
        {
            warm_extent_pipelines_js(device).await
        }
    }

    /// Reduce one drawable series to the six bounds consumed by the axis SSoT.
    pub(crate) fn begin_series(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool_buffer: &wgpu::Buffer,
        mode: GpuSeriesExtentMode,
        columns: GpuSeriesExtentColumns,
    ) -> Result<GpuSeriesExtentTicket, GpuErrorbarError> {
        let x = checked_range(pool_buffer, &columns.x, columns.x.len_values, "x")?;
        let y = checked_range(pool_buffer, &columns.y, columns.y.len_values, "y")?;
        let input_len = x.len.min(y.len);
        if input_len == 0 {
            return Err(GpuErrorbarError::EmptyColumn {
                role: "paired series",
            });
        }

        let required = |column: Option<ColumnHandle>, role: &'static str, active: bool| {
            if !active {
                return Ok(ColumnRangeGpu { offset: 0, len: 0 });
            }
            let handle = column.ok_or(GpuErrorbarError::MissingSeriesColumn { role, mode })?;
            checked_range(pool_buffer, &handle, handle.len_values, role)
        };
        let x_lower = required(columns.x_lower, "x lower error", mode.has_x_errors())?;
        let x_upper = required(columns.x_upper, "x upper error", mode.has_x_errors())?;
        let y_lower = required(columns.y_lower, "y lower error", mode.has_y_errors())?;
        let y_upper = required(columns.y_upper, "y upper error", mode.has_y_errors())?;

        let params = ParamsGpu {
            offsets_0: [x.offset, y.offset, x_lower.offset, x_upper.offset],
            offsets_1: [y_lower.offset, y_upper.offset, mode as u32, 0],
            lengths_0: [x.len, y.len, x_lower.len, x_upper.len],
            lengths_1: [y_lower.len, y_upper.len, input_len, 0],
        };
        self.begin_raw(device, queue, pool_buffer, params)
            .map(|core| GpuSeriesExtentTicket { core })
    }

    /// Resolve the exact outer bounds of a field's rendered coordinate lattice.
    /// Only one invocation is needed: the GPU reads at most the two endpoint
    /// pairs per axis and returns the same six compact bounds as the row reducer.
    pub(crate) fn begin_field(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool_buffer: &wgpu::Buffer,
        mode: GpuFieldExtentMode,
        columns: GpuFieldExtentColumns,
    ) -> Result<GpuSeriesExtentTicket, GpuErrorbarError> {
        let x = checked_range(pool_buffer, &columns.x, columns.x.len_values, "field x")?;
        let y = checked_range(pool_buffer, &columns.y, columns.y.len_values, "field y")?;
        if columns.x_cells == 0 || columns.y_cells == 0 {
            return Err(GpuErrorbarError::EmptyColumn {
                role: "field drawable lattice",
            });
        }
        let params = ParamsGpu {
            offsets_0: [x.offset, y.offset, 0, 0],
            offsets_1: [0, 0, mode.shader_mode(), 0],
            lengths_0: [x.len, y.len, 0, 0],
            // x/y resolved cell counts; one logical reduction input; dispatch
            // group count is filled by `begin_raw`.
            lengths_1: [columns.x_cells, columns.y_cells, 1, 0],
        };
        self.begin_raw(device, queue, pool_buffer, params)
            .map(|core| GpuSeriesExtentTicket { core })
    }

    /// Preserve the original three-column errorbar reducer contract.
    ///
    /// Missing rows in a shorter error column and non-finite error pairs are
    /// zero, exactly as before. This is intentionally distinct from the
    /// drawable-series direction predicate.
    pub fn begin(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool_buffer: &wgpu::Buffer,
        values: ColumnHandle,
        lower_errors: ColumnHandle,
        upper_errors: ColumnHandle,
    ) -> Result<GpuErrorbarExtentTicket, GpuErrorbarError> {
        let value = checked_range(pool_buffer, &values, values.len_values, "value")?;
        if value.len == 0 {
            return Err(GpuErrorbarError::EmptyColumn { role: "value" });
        }
        let lower_bound_len = lower_errors.len_values.min(values.len_values);
        let upper_bound_len = upper_errors.len_values.min(values.len_values);
        let lower = checked_range(pool_buffer, &lower_errors, lower_bound_len, "lower error")?;
        let upper = checked_range(pool_buffer, &upper_errors, upper_bound_len, "upper error")?;
        if lower.len == 0 {
            return Err(GpuErrorbarError::EmptyColumn {
                role: "lower error",
            });
        }
        if upper.len == 0 {
            return Err(GpuErrorbarError::EmptyColumn {
                role: "upper error",
            });
        }

        let params = ParamsGpu {
            offsets_0: [value.offset, 0, lower.offset, upper.offset],
            offsets_1: [0, 0, LEGACY_AXIS_MODE, 0],
            lengths_0: [value.len, 0, lower.len, upper.len],
            lengths_1: [0, 0, value.len, 0],
        };
        self.begin_raw(device, queue, pool_buffer, params)
            .map(|core| GpuErrorbarExtentTicket { core })
    }

    fn begin_raw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool_buffer: &wgpu::Buffer,
        mut params: ParamsGpu,
    ) -> Result<GpuExtentTicketCore, GpuErrorbarError> {
        let limits = device.limits();
        let pool_binding = checked_pool_binding(pool_buffer, &limits)?;
        let input_len = params.lengths_1[2];
        let max_scratch_groups = limits.max_storage_buffer_binding_size / SERIES_STATE_BYTES;
        let dispatch_cap = u64::from(limits.max_compute_workgroups_per_dimension)
            .min(max_scratch_groups)
            .min(u64::from(u32::MAX)) as u32;
        if dispatch_cap == 0 {
            return Err(GpuErrorbarError::NoDispatchCapacity);
        }

        let first_groups = div_ceil(input_len, WORKGROUP_SIZE).min(dispatch_cap);
        let first_scratch_bytes = u64::from(first_groups) * SERIES_STATE_BYTES;
        let second_capacity = div_ceil(first_groups, WORKGROUP_SIZE).max(1);
        let second_scratch_bytes = u64::from(second_capacity) * SERIES_STATE_BYTES;
        // Two tallies: the reduction scratch (including every params uniform the
        // bind groups end up owning) and the MAP_READ buffer, which is its own
        // row. Both are handed to the ticket as one charge each.
        let scratch_tally = ChargeTally::new();
        let readback_tally = ChargeTally::new();
        // gpu-alloc: ErrorbarScratch
        let scratch_a = charged_buffer(
            &scratch_tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy series fit scratch A"),
                size: first_scratch_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
        );
        // gpu-alloc: ErrorbarScratch
        let scratch_b = charged_buffer(
            &scratch_tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy series fit scratch B"),
                size: second_scratch_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
        );
        // gpu-alloc: Readback
        let readback = charged_buffer(
            &readback_tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy series fit detached readback"),
                size: SERIES_STATE_BYTES,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            },
        );

        params.lengths_1[3] = first_groups;
        // The params uniforms are owned by their bind groups; `params_buffer`
        // tallies them into the same lump as the scratch.
        let initial_params = params_buffer(
            &scratch_tally,
            device,
            "figgy series fit value params",
            params,
        );
        let initial_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy series fit value bindings"),
            layout: &self.values_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(pool_binding),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: scratch_a.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: initial_params.as_entire_binding(),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("figgy series fit-bound encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy series fit values pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.reduce_values);
            pass.set_bind_group(0, &initial_bind_group, &[]);
            pass.dispatch_workgroups(first_groups, 1, 1);
        }

        let mut current_len = first_groups;
        let mut current_is_a = true;
        while current_len > 1 {
            let groups = div_ceil(current_len, WORKGROUP_SIZE);
            let state_params = params_buffer(
                &scratch_tally,
                device,
                "figgy series fit state params",
                ParamsGpu {
                    offsets_0: [0; 4],
                    offsets_1: [0, 0, params.offsets_1[2], 0],
                    lengths_0: [0; 4],
                    lengths_1: [0, 0, current_len, groups],
                },
            );
            let (input, output) = if current_is_a {
                (&scratch_a, &scratch_b)
            } else {
                (&scratch_b, &scratch_a)
            };
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("figgy series fit state bindings"),
                layout: &self.states_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: input.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: state_params.as_entire_binding(),
                    },
                ],
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("figgy series fit state pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.reduce_states);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(groups, 1, 1);
            }
            current_len = groups;
            current_is_a = !current_is_a;
        }

        let final_buffer = if current_is_a { &scratch_a } else { &scratch_b };
        encoder.copy_buffer_to_buffer(final_buffer, 0, &readback, 0, SERIES_STATE_BYTES);
        let (sender, receiver) = oneshot::channel();
        encoder.map_buffer_on_submit(
            &readback,
            wgpu::MapMode::Read,
            0..SERIES_STATE_BYTES,
            move |result| {
                let _ = sender.send(result);
            },
        );
        let submission = queue.submit(std::iter::once(encoder.finish()));
        Ok(GpuExtentTicketCore {
            device: device.clone(),
            readback,
            receiver,
            submission,
            _scratch_charge: scratch_tally
                .into_charge(&self.ledger, GpuResourceKind::ErrorbarScratch),
            _readback_charge: readback_tally.into_charge(&self.ledger, GpuResourceKind::Readback),
        })
    }
}

struct GpuExtentTicketCore {
    device: wgpu::Device,
    readback: wgpu::Buffer,
    receiver: oneshot::Receiver<Result<(), wgpu::BufferAsyncError>>,
    submission: wgpu::SubmissionIndex,
    /// Charges for the reduction scratch and the MAP_READ buffer. They are
    /// credited back when this core is destructured on resolve or dropped on
    /// abandonment, which is exactly when the buffers go away.
    _scratch_charge: GpuByteCharge,
    _readback_charge: GpuByteCharge,
}

impl GpuExtentTicketCore {
    async fn resolve(self) -> Result<SeriesStateGpu, GpuErrorbarError> {
        let Self {
            device,
            readback,
            receiver,
            submission,
            // Dropped here: resolving destroys the scratch and the readback.
            _scratch_charge,
            _readback_charge,
        } = self;

        #[cfg(not(target_arch = "wasm32"))]
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission.clone()),
                timeout: None,
            })
            .map_err(|error| GpuErrorbarError::DevicePollFailed(format!("{error:?}")))?;
        #[cfg(target_arch = "wasm32")]
        let _ = (device, submission);

        receiver
            .await
            .map_err(|_| GpuErrorbarError::ReadbackSenderDropped)?
            .map_err(|error| GpuErrorbarError::ReadbackMapFailed(format!("{error:?}")))?;
        let slice = readback.slice(0..SERIES_STATE_BYTES);
        let mapped = slice
            .get_mapped_range()
            .map_err(|error| GpuErrorbarError::ReadbackMapFailed(format!("{error:?}")))?;
        let state = bytemuck::pod_read_unaligned::<SeriesStateGpu>(&mapped);
        drop(mapped);
        readback.unmap();
        Ok(state)
    }
}

/// Owned one-shot readback for a drawable-series extent.
pub struct GpuSeriesExtentTicket {
    core: GpuExtentTicketCore,
}

impl GpuSeriesExtentTicket {
    pub async fn resolve(self) -> Result<Option<GpuSeriesExtent>, GpuErrorbarError> {
        decode_series_state(self.core.resolve().await?)
    }
}

/// Owned one-shot readback preserving the legacy scalar result.
pub struct GpuErrorbarExtentTicket {
    core: GpuExtentTicketCore,
}

impl GpuErrorbarExtentTicket {
    pub async fn resolve(self) -> Result<Option<GpuErrorbarExtent>, GpuErrorbarError> {
        let state = self.core.resolve().await?;
        decode_axis_state(state.x)
    }
}

fn checked_u32_len(role: &'static str, len: usize) -> Result<u32, GpuErrorbarError> {
    u32::try_from(len).map_err(|_| GpuErrorbarError::ValueCountTooLarge { role, len })
}

fn checked_range(
    pool_buffer: &wgpu::Buffer,
    handle: &ColumnHandle,
    bound_values: usize,
    role: &'static str,
) -> Result<ColumnRangeGpu, GpuErrorbarError> {
    let len = checked_u32_len(role, bound_values)?;
    let required = (bound_values as u64)
        .checked_mul(COLUMN_VALUE_BYTES as u64)
        .ok_or(GpuErrorbarError::ValueCountTooLarge {
            role,
            len: bound_values,
        })?;
    let pool_size = pool_buffer.size();
    let end = handle.offset.checked_add(required);
    if required > handle.byte_size || end.is_none_or(|end| end > pool_size) {
        return Err(GpuErrorbarError::InvalidColumnRange {
            role,
            offset: handle.offset,
            required,
            available: handle.byte_size,
            pool_size,
        });
    }
    let lane_alignment = COLUMN_VALUE_BYTES as u32;
    if !handle.offset.is_multiple_of(u64::from(lane_alignment)) {
        return Err(GpuErrorbarError::MisalignedColumnOffset {
            role,
            offset: handle.offset,
            alignment: lane_alignment,
        });
    }
    let offset = u32::try_from(handle.offset / COLUMN_VALUE_BYTES as u64).map_err(|_| {
        GpuErrorbarError::ColumnOffsetTooLarge {
            role,
            offset: handle.offset,
        }
    })?;
    Ok(ColumnRangeGpu { offset, len })
}

fn checked_pool_binding<'a>(
    pool_buffer: &'a wgpu::Buffer,
    limits: &wgpu::Limits,
) -> Result<wgpu::BufferBinding<'a>, GpuErrorbarError> {
    let requested = pool_buffer.size();
    let limit = limits.max_storage_buffer_binding_size;
    if requested > limit {
        return Err(GpuErrorbarError::StorageBindingTooLarge {
            role: "column pool",
            requested,
            limit,
        });
    }
    let size = NonZeroU64::new(requested).ok_or(GpuErrorbarError::EmptyColumn {
        role: "column pool",
    })?;
    Ok(wgpu::BufferBinding {
        buffer: pool_buffer,
        offset: 0,
        size: Some(size),
    })
}

fn div_ceil(value: u32, divisor: u32) -> u32 {
    value / divisor + u32::from(!value.is_multiple_of(divisor))
}

fn params_buffer(
    tally: &ChargeTally,
    device: &wgpu::Device,
    label: &'static str,
    params: ParamsGpu,
) -> wgpu::Buffer {
    // gpu-alloc: ErrorbarScratch
    charged_buffer_init(
        tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        },
    )
}

fn decode_bound(bound: [f32; 2]) -> Result<f64, GpuErrorbarError> {
    if !bound.into_iter().all(f32::is_finite) {
        return Err(GpuErrorbarError::CorruptResult);
    }
    Ok(bound[0] as f64 + bound[1] as f64)
}

fn decode_axis_state(state: AxisStateGpu) -> Result<Option<GpuErrorbarExtent>, GpuErrorbarError> {
    if state.minimum == EMPTY_MINIMUM_BOUND
        && state.maximum == EMPTY_MAXIMUM_BOUND
        && state.minimum_positive == EMPTY_MINIMUM_BOUND
    {
        return Ok(None);
    }

    let min = decode_bound(state.minimum)?;
    let max = decode_bound(state.maximum)?;
    let min_positive = if max > 0.0 {
        Some(decode_bound(state.minimum_positive)?)
    } else if state.minimum_positive == EMPTY_MINIMUM_BOUND {
        None
    } else {
        return Err(GpuErrorbarError::CorruptResult);
    };
    if min > max || min_positive.is_some_and(|value| value <= 0.0 || value > max) {
        return Err(GpuErrorbarError::CorruptResult);
    }
    Ok(Some(GpuErrorbarExtent {
        min,
        max,
        min_positive,
    }))
}

fn decode_series_state(state: SeriesStateGpu) -> Result<Option<GpuSeriesExtent>, GpuErrorbarError> {
    match (decode_axis_state(state.x)?, decode_axis_state(state.y)?) {
        (None, None) => Ok(None),
        (Some(x), Some(y)) => Ok(Some(GpuSeriesExtent { x, y })),
        _ => Err(GpuErrorbarError::CorruptResult),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Column, split_f64_to_f32_pair};
    use crate::data_render::ColumnPool;

    fn f32_column(values: Vec<f32>) -> Column<f32> {
        let finite: Vec<_> = values
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .collect();
        Column {
            data: values,
            min: finite.iter().copied().fold(f32::INFINITY, f32::min),
            max: finite.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        }
    }

    fn f64_column(values: Vec<f64>) -> Column<f64> {
        let finite: Vec<_> = values
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .collect();
        Column {
            data: values,
            min: finite.iter().copied().fold(f64::INFINITY, f64::min),
            max: finite.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        }
    }

    fn add_f32(
        pool: &mut ColumnPool,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: &str,
        values: Vec<f32>,
    ) -> ColumnHandle {
        pool.add_column(
            id.into(),
            &f32_column(values),
            crate::data_render::GpuAllocCtx::unbudgeted(device, queue),
        )
        .unwrap()
    }

    fn series_columns(x: ColumnHandle, y: ColumnHandle) -> GpuSeriesExtentColumns {
        GpuSeriesExtentColumns {
            x,
            y,
            x_lower: None,
            x_upper: None,
            y_lower: None,
            y_upper: None,
        }
    }

    fn resolve_series(
        engine: &GpuErrorbarExtentEngine,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool: &ColumnPool,
        mode: GpuSeriesExtentMode,
        columns: GpuSeriesExtentColumns,
    ) -> Option<GpuSeriesExtent> {
        pollster::block_on(
            engine
                .begin_series(device, queue, pool.buffer(), mode, columns)
                .unwrap()
                .resolve(),
        )
        .unwrap()
    }

    fn resolve_field(
        engine: &GpuErrorbarExtentEngine,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool: &ColumnPool,
        mode: GpuFieldExtentMode,
        columns: GpuFieldExtentColumns,
    ) -> GpuSeriesExtent {
        pollster::block_on(
            engine
                .begin_field(device, queue, pool.buffer(), mode, columns)
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .expect("field coordinates produce an extent")
    }

    #[test]
    fn fit_readback_is_only_six_hilo_axis_bounds() {
        assert_eq!(
            SERIES_STATE_BYTES,
            6 * std::mem::size_of::<[f32; 2]>() as u64
        );
        assert_eq!(std::mem::size_of::<AxisStateGpu>(), 3 * 8);
        assert_eq!(std::mem::size_of::<SeriesStateGpu>(), 6 * 8);

        let shader = include_str!("gpu_errorbar.wgsl");
        for forbidden in [
            "struct Endpoint",
            "source_index",
            "SignedAccumulator",
            "ACC_WORDS",
        ] {
            assert!(
                !shader.contains(forbidden),
                "fit shader leaked non-SSoT state: {forbidden}"
            );
        }
    }

    #[test]
    fn field_extent_matches_the_rendered_cell_or_sample_lattice() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        let x = add_f32(
            &mut pool,
            &device,
            &queue,
            "field-fit-x",
            vec![-3.1, -2.9, 2.9, 3.1],
        );
        let y = add_f32(
            &mut pool,
            &device,
            &queue,
            "field-fit-y",
            vec![-2.5, -2.3, 2.3, 2.5],
        );
        let engine = GpuErrorbarExtentEngine::new(&device);

        let samples = resolve_field(
            &engine,
            &device,
            &queue,
            &pool,
            GpuFieldExtentMode::EdgesSamples,
            GpuFieldExtentColumns {
                x,
                y,
                x_cells: 3,
                y_cells: 3,
            },
        );
        assert!((samples.x.min - (-3.0)).abs() < 1.0e-6);
        assert!((samples.x.max - 3.0).abs() < 1.0e-6);
        assert!((samples.y.min - (-2.4)).abs() < 1.0e-6);
        assert!((samples.y.max - 2.4).abs() < 1.0e-6);

        let cells = resolve_field(
            &engine,
            &device,
            &queue,
            &pool,
            GpuFieldExtentMode::EdgesCells,
            GpuFieldExtentColumns {
                x,
                y,
                x_cells: 3,
                y_cells: 3,
            },
        );
        assert!((cells.x.min - (-3.1)).abs() < 1.0e-6);
        assert!((cells.x.max - 3.1).abs() < 1.0e-6);
        assert!((cells.y.min - (-2.5)).abs() < 1.0e-6);
        assert!((cells.y.max - 2.5).abs() < 1.0e-6);

        let centres_as_cells = resolve_field(
            &engine,
            &device,
            &queue,
            &pool,
            GpuFieldExtentMode::CentersCells,
            GpuFieldExtentColumns {
                x,
                y,
                x_cells: 4,
                y_cells: 4,
            },
        );
        assert!((centres_as_cells.x.min - (-3.2)).abs() < 1.0e-6);
        assert!((centres_as_cells.x.max - 3.2).abs() < 1.0e-6);
        assert!((centres_as_cells.y.min - (-2.6)).abs() < 1.0e-6);
        assert!((centres_as_cells.y.max - 2.6).abs() < 1.0e-6);
    }

    #[test]
    fn legacy_extent_matches_nan_and_short_error_oracle() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        let values = add_f32(
            &mut pool,
            &device,
            &queue,
            "legacy-values",
            vec![f32::NAN, 2.0, 4.0],
        );
        let lower = add_f32(
            &mut pool,
            &device,
            &queue,
            "legacy-lower",
            vec![100.0, f32::NAN],
        );
        let upper = add_f32(
            &mut pool,
            &device,
            &queue,
            "legacy-upper",
            vec![100.0, f32::NAN],
        );
        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = pollster::block_on(
            engine
                .begin(&device, &queue, pool.buffer(), values, lower, upper)
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            extent,
            GpuErrorbarExtent {
                min: 2.0,
                max: 4.0,
                min_positive: Some(2.0),
            }
        );
    }

    #[test]
    fn legacy_extent_preserves_overflow_hilo_and_multi_pass() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            128 * 1024,
        )
        .unwrap();
        let values = add_f32(
            &mut pool,
            &device,
            &queue,
            "legacy-many-values",
            (0..1000).map(|value| value as f32).collect(),
        );
        let lower = add_f32(
            &mut pool,
            &device,
            &queue,
            "legacy-many-lower",
            vec![5.0; 1000],
        );
        let upper = add_f32(
            &mut pool,
            &device,
            &queue,
            "legacy-many-upper",
            vec![7.0; 1000],
        );
        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = pollster::block_on(
            engine
                .begin(&device, &queue, pool.buffer(), values, lower, upper)
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(extent.min, -5.0);
        assert_eq!(extent.max, 1006.0);
        assert_eq!(extent.min_positive, Some(1.0));

        let mut overflow_pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        let max_value = add_f32(
            &mut overflow_pool,
            &device,
            &queue,
            "legacy-max-value",
            vec![f32::MAX],
        );
        let max_error = add_f32(
            &mut overflow_pool,
            &device,
            &queue,
            "legacy-max-error",
            vec![f32::MAX],
        );
        let overflow = pollster::block_on(
            engine
                .begin(
                    &device,
                    &queue,
                    overflow_pool.buffer(),
                    max_value,
                    max_error,
                    max_error,
                )
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(overflow.min, 0.0);
        assert_eq!(overflow.max, f32::MAX as f64 + f32::MAX as f64);
    }

    #[test]
    fn points_keep_base_tail_beyond_short_error_domain() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        let x = add_f32(&mut pool, &device, &queue, "series-x", vec![0.0, 1.0, 50.0]);
        let y = add_f32(
            &mut pool,
            &device,
            &queue,
            "series-y",
            vec![10.0, 20.0, 30.0],
        );
        let lower = add_f32(&mut pool, &device, &queue, "series-x-lower", vec![5.0, 0.0]);
        let upper = add_f32(&mut pool, &device, &queue, "series-x-upper", vec![0.0, 5.0]);
        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = resolve_series(
            &engine,
            &device,
            &queue,
            &pool,
            GpuSeriesExtentMode::PointsX,
            GpuSeriesExtentColumns {
                x,
                y,
                x_lower: Some(lower),
                x_upper: Some(upper),
                y_lower: None,
                y_upper: None,
            },
        )
        .unwrap();
        assert_eq!((extent.x.min, extent.x.max), (-5.0, 50.0));
        assert_eq!((extent.y.min, extent.y.max), (10.0, 30.0));
        assert_eq!(extent.x.min_positive, Some(1.0));
    }

    #[test]
    fn line_uses_only_finite_adjacent_segment_endpoints() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        let x = add_f32(
            &mut pool,
            &device,
            &queue,
            "line-x",
            vec![0.0, 1.0, 100.0, 200.0],
        );
        let y = add_f32(
            &mut pool,
            &device,
            &queue,
            "line-y",
            vec![0.0, 1.0, f32::NAN, 3.0],
        );
        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = resolve_series(
            &engine,
            &device,
            &queue,
            &pool,
            GpuSeriesExtentMode::Line,
            series_columns(x, y),
        )
        .unwrap();
        assert_eq!((extent.x.min, extent.x.max), (0.0, 1.0));
        assert_eq!((extent.y.min, extent.y.max), (0.0, 1.0));

        let mut single_pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        let x = add_f32(&mut single_pool, &device, &queue, "single-x", vec![1.0]);
        let y = add_f32(&mut single_pool, &device, &queue, "single-y", vec![2.0]);
        assert!(
            resolve_series(
                &engine,
                &device,
                &queue,
                &single_pool,
                GpuSeriesExtentMode::Line,
                series_columns(x, y),
            )
            .is_none()
        );
    }

    #[test]
    fn all_point_modes_and_one_sided_nan_match_renderer_direction_policy() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        let x = add_f32(&mut pool, &device, &queue, "modes-x", vec![1.0, 2.0]);
        let y = add_f32(&mut pool, &device, &queue, "modes-y", vec![10.0, 20.0]);
        let x_lo = add_f32(&mut pool, &device, &queue, "modes-x-lo", vec![1.0, 1.0]);
        let x_hi = add_f32(&mut pool, &device, &queue, "modes-x-hi", vec![2.0, 2.0]);
        let y_lo = add_f32(
            &mut pool,
            &device,
            &queue,
            "modes-y-lo",
            vec![f32::NAN, 3.0],
        );
        let y_hi = add_f32(&mut pool, &device, &queue, "modes-y-hi", vec![1000.0, 4.0]);
        let engine = GpuErrorbarExtentEngine::new(&device);
        for mode in [
            GpuSeriesExtentMode::Points,
            GpuSeriesExtentMode::PointsX,
            GpuSeriesExtentMode::PointsY,
            GpuSeriesExtentMode::PointsXY,
        ] {
            let extent = resolve_series(
                &engine,
                &device,
                &queue,
                &pool,
                mode,
                GpuSeriesExtentColumns {
                    x,
                    y,
                    x_lower: mode.has_x_errors().then_some(x_lo),
                    x_upper: mode.has_x_errors().then_some(x_hi),
                    y_lower: mode.has_y_errors().then_some(y_lo),
                    y_upper: mode.has_y_errors().then_some(y_hi),
                },
            )
            .unwrap();
            assert_eq!(
                (extent.x.min, extent.x.max),
                if mode.has_x_errors() {
                    (0.0, 4.0)
                } else {
                    (1.0, 2.0)
                }
            );
            assert_eq!(
                (extent.y.min, extent.y.max),
                if mode.has_y_errors() {
                    (10.0, 24.0)
                } else {
                    (10.0, 20.0)
                }
            );
        }
    }

    #[test]
    fn paired_invalid_rows_exclude_base_and_error_endpoints() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        let x = add_f32(
            &mut pool,
            &device,
            &queue,
            "paired-invalid-x",
            vec![1.0, 2.0, 3.0],
        );
        let y = add_f32(
            &mut pool,
            &device,
            &queue,
            "paired-invalid-y",
            vec![10.0, f32::NAN, f32::INFINITY],
        );
        let lower = add_f32(
            &mut pool,
            &device,
            &queue,
            "paired-invalid-lo",
            vec![100.0; 3],
        );
        let upper = add_f32(
            &mut pool,
            &device,
            &queue,
            "paired-invalid-hi",
            vec![100.0; 3],
        );
        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = resolve_series(
            &engine,
            &device,
            &queue,
            &pool,
            GpuSeriesExtentMode::PointsX,
            GpuSeriesExtentColumns {
                x,
                y,
                x_lower: Some(lower),
                x_upper: Some(upper),
                y_lower: None,
                y_upper: None,
            },
        )
        .unwrap();
        assert_eq!((extent.x.min, extent.x.max), (-99.0, 101.0));
        assert_eq!((extent.y.min, extent.y.max), (10.0, 10.0));
    }

    #[test]
    fn enabled_direction_keeps_finite_endpoint_when_opposite_error_is_infinite() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        let x = add_f32(&mut pool, &device, &queue, "one-infinite-x", vec![10.0]);
        let y = add_f32(&mut pool, &device, &queue, "one-infinite-y", vec![20.0]);
        let lower = add_f32(&mut pool, &device, &queue, "one-infinite-lo", vec![2.0]);
        let upper = add_f32(
            &mut pool,
            &device,
            &queue,
            "one-infinite-hi",
            vec![f32::INFINITY],
        );
        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = resolve_series(
            &engine,
            &device,
            &queue,
            &pool,
            GpuSeriesExtentMode::PointsX,
            GpuSeriesExtentColumns {
                x,
                y,
                x_lower: Some(lower),
                x_upper: Some(upper),
                y_lower: None,
                y_upper: None,
            },
        )
        .unwrap();
        assert_eq!((extent.x.min, extent.x.max), (8.0, 10.0));
    }

    #[test]
    fn xy_errors_share_six_column_domain_without_truncating_base_tail() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        let x = add_f32(
            &mut pool,
            &device,
            &queue,
            "six-domain-x",
            vec![0.0, 10.0, 100.0],
        );
        let y = add_f32(
            &mut pool,
            &device,
            &queue,
            "six-domain-y",
            vec![0.0, 20.0, 200.0],
        );
        let x_lower = add_f32(&mut pool, &device, &queue, "six-domain-x-lo", vec![1.0; 3]);
        let x_upper = add_f32(&mut pool, &device, &queue, "six-domain-x-hi", vec![1.0; 3]);
        let y_lower = add_f32(&mut pool, &device, &queue, "six-domain-y-lo", vec![2.0]);
        let y_upper = add_f32(&mut pool, &device, &queue, "six-domain-y-hi", vec![2.0]);
        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = resolve_series(
            &engine,
            &device,
            &queue,
            &pool,
            GpuSeriesExtentMode::PointsXY,
            GpuSeriesExtentColumns {
                x,
                y,
                x_lower: Some(x_lower),
                x_upper: Some(x_upper),
                y_lower: Some(y_lower),
                y_upper: Some(y_upper),
            },
        )
        .unwrap();
        assert_eq!((extent.x.min, extent.x.max), (-1.0, 100.0));
        assert_eq!((extent.y.min, extent.y.max), (-2.0, 200.0));
    }

    #[test]
    fn subnormal_min_positive_is_exact() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let tiny = f32::from_bits(1);
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        let x = add_f32(&mut pool, &device, &queue, "subnormal-x", vec![-tiny, tiny]);
        let y = add_f32(
            &mut pool,
            &device,
            &queue,
            "subnormal-y",
            vec![tiny * 2.0, tiny * 3.0],
        );
        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = resolve_series(
            &engine,
            &device,
            &queue,
            &pool,
            GpuSeriesExtentMode::Points,
            series_columns(x, y),
        )
        .unwrap();
        assert_eq!(extent.x.min_positive, Some(tiny as f64));
        assert_eq!(extent.y.min_positive, Some((tiny * 2.0) as f64));
    }

    #[test]
    fn hilo_error_endpoints_preserve_residual_lanes() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let raw_value = 1_700_000_000_000.125_f64;
        let raw_lower = 0.25_f64;
        let raw_upper = 0.75_f64;
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        let x = pool
            .add_hilo_column(
                "hilo-error-x".into(),
                &f64_column(vec![raw_value]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        let y = pool
            .add_hilo_column(
                "hilo-error-y".into(),
                &f64_column(vec![1.0]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        let lower = pool
            .add_hilo_column(
                "hilo-error-lo".into(),
                &f64_column(vec![raw_lower]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        let upper = pool
            .add_hilo_column(
                "hilo-error-hi".into(),
                &f64_column(vec![raw_upper]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        let pair_sum = |value: f64| {
            let (hi, lo) = split_f64_to_f32_pair(value);
            hi as f64 + lo as f64
        };
        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = resolve_series(
            &engine,
            &device,
            &queue,
            &pool,
            GpuSeriesExtentMode::PointsX,
            GpuSeriesExtentColumns {
                x,
                y,
                x_lower: Some(lower),
                x_upper: Some(upper),
                y_lower: None,
                y_upper: None,
            },
        )
        .unwrap();
        assert_eq!(extent.x.min, pair_sum(raw_value) - pair_sum(raw_lower));
        assert_eq!(extent.x.max, pair_sum(raw_value) + pair_sum(raw_upper));
    }

    #[test]
    fn series_extent_preserves_hilo_residuals_and_min_positive() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        let raw = [1_700_000_000_000.125_f64, 1_700_000_000_000.875_f64];
        let x = pool
            .add_hilo_column(
                "hilo-series-x".into(),
                &f64_column(raw.to_vec()),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        let y = pool
            .add_hilo_column(
                "hilo-series-y".into(),
                &f64_column(vec![-2.0, 0.25]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        let pair_sum = |value: f64| {
            let (hi, lo) = split_f64_to_f32_pair(value);
            hi as f64 + lo as f64
        };
        let engine = GpuErrorbarExtentEngine::new(&device);
        let extent = resolve_series(
            &engine,
            &device,
            &queue,
            &pool,
            GpuSeriesExtentMode::Points,
            series_columns(x, y),
        )
        .unwrap();
        assert_eq!(extent.x.min, pair_sum(raw[0]));
        assert_eq!(extent.x.max, pair_sum(raw[1]));
        assert_eq!(extent.y.min_positive, Some(0.25));
    }
}
