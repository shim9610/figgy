//! Exact GPU picking for histogram bins and matrix-backed field primitives.
//!
//! The compute entry points live in the same WGSL modules as the render entry
//! points. Histogram picking calls the bar renderer's rectangle helper;
//! heatmap/contour picking calls the field renderer's `locate`,
//! `contour_sample`, and quadratic contour-distance helpers; grid vector
//! picking runs the assembled vector shader's `pick_vector_cell`, which walks
//! the arrows with the draw's own sample, projection and glyph functions. The CPU submits
//! only transforms, lane bases, style scalars, and identity indices. It never
//! reads or mirrors source values from [`ColumnPool`](crate::data_render::ColumnPool).

use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;

use crate::data_render::{ColumnHandle, ColumnPool, ScatterTransform};
use crate::gpu_compile::{storage_entry, uniform_entry};
use crate::gpu_memory::{
    ChargeTally, GpuByteCharge, GpuLedger, GpuResourceKind, SharedCharge, charged_buffer,
    charged_buffer_init, shared_charge,
};
use crate::gpu_readback::GpuReadback;
use crate::init::{InitEvent, finished, started};
use crate::pick::PickedData;

const INIT_SCOPE: &str = "renderer.gpu_data_pick";
const DATA_PICK_CANDIDATE_BYTES: u64 = 32;
pub(crate) const DATA_PICK_FLAG_FILL: u32 = 1;
pub(crate) const DATA_PICK_FLAG_CONTOUR: u32 = 2;
/// Resident fills pick the cell painted at the cursor's pixel centre, and
/// only where its colour is not clear (`field_pick_cell`).
pub(crate) const DATA_PICK_FLAG_PAINTED: u32 = 4;
pub(crate) const DATA_PICK_KIND_HISTOGRAM_BIN: u32 = 1;
pub(crate) const DATA_PICK_KIND_MATRIX_CELL: u32 = 2;
pub(crate) const DATA_PICK_KIND_CONTOUR_LEVEL: u32 = 3;
/// A category bar, box plot or radar series' value label
/// (`value_label.wgsl::pick_value_labels`): `index0` is its category.
pub(crate) const DATA_PICK_KIND_VALUE_LABEL: u32 = 4;
/// A triangle mesh series' triangle (`mesh_entry.wgsl::pick_tri_mesh`,
/// #210): `index0` is the triangle, `index1` its vertex nearest the cursor.
pub(crate) const DATA_PICK_KIND_MESH_TRIANGLE: u32 = 5;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct DataPickQueryGpu {
    pub(crate) cursor_ndc_t: [f32; 4],
    pub(crate) limits: [f32; 4],
    pub(crate) data: [u32; 4],
    pub(crate) bases: [u32; 4],
    pub(crate) baseline: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct DataPickCandidateGpu {
    pub(crate) valid: u32,
    pub(crate) paint_order: u32,
    pub(crate) kind: u32,
    pub(crate) index0: u32,
    pub(crate) index1: u32,
    pub(crate) index2: u32,
    pub(crate) distance_px: f32,
    pub(crate) primitive_order: u32,
}

const _: () = assert!(std::mem::size_of::<DataPickQueryGpu>() == 80);
const _: () = assert!(std::mem::size_of::<DataPickCandidateGpu>() == 32);

/// What a typed pick's field and vector-grid series keep alive until the
/// pick resolves: the group-2 charge and a contour's label placement.
struct FieldPickHold {
    _charge: SharedCharge,
    _label: Option<crate::gpu_contour::ContourLabelSnapshot>,
}

#[derive(Clone)]
struct DataPickIdentity {
    source_id: Option<String>,
    series_id: String,
    paint_order: u32,
    /// A violin's candidates name the half they hit in `index1`
    /// (`VIOLIN_SIDE_*`, 0 for a symmetric body).
    violin: bool,
    /// A range bar's or range link's candidates carry a drawn length in
    /// render pixels in `index1` (f32 bits): the bar's, or the link's
    /// predecessor's (`PickedData::HistogramBin::extent_px`).
    extent: bool,
}

pub(crate) enum GpuDataPickGeometry {
    Histogram {
        edges: ColumnHandle,
        values: ColumnHandle,
        bin_count: u32,
        baseline: [f32; 2],
        gap_px: f32,
        width_ratio: f32,
        horizontal: bool,
        style_map: Option<wgpu::BindGroup>,
    },
    Field {
        field_bg: wgpu::BindGroup,
        charge: SharedCharge,
        has_fill: bool,
        has_contour: bool,
        contour_width_px: f32,
        /// The fill's premultiplied `nan_color` (the fill's group-1 style),
        /// so the cell pick sees the colour the fill paints.
        nan_color_premul: [f32; 4],
        /// A labelled contour's label placement at the pick's transform: its
        /// gaps paint no contour, so they pick none.
        label: Option<crate::gpu_contour::ContourLabelSnapshot>,
    },
    /// A grid vector's arrows, picked as the cells they are drawn for: the
    /// series' vector group 2 (pool, grid table, stops, params at the pick's
    /// display scale).
    VectorGrid {
        vector_bg: wgpu::BindGroup,
        charge: SharedCharge,
    },
    /// A category bar series' bars, picked by the category bar module's own
    /// entry (`pick_category_bar`) over the draw's group 3 and override
    /// table at the pick's display scale. A bar answers as a histogram bin
    /// whose index is its row.
    CategoryBars {
        data: crate::data_render::CategoryBarData,
        /// The rows its kernel tests, which size its dispatch (#445).
        rows: u32,
    },
    /// A box plot series' boxes, picked by the box plot entry
    /// (`pick_box_plot`) over the draw's group 3 and style map at the
    /// pick's display scale: every part of a box (body, whiskers, caps,
    /// median, mean, outliers) answers its box as a histogram bin whose
    /// index is its row.
    BoxPlot {
        data: crate::data_render::CategoryBarData,
        /// The rows its kernel tests, which size its dispatch (#445).
        rows: u32,
    },
    /// A range bar series' bars, picked by the range bar entry
    /// (`pick_range_bar`) over the draw's group 3 and style map at the
    /// pick's display scale: the body, the whisker and the connector of a
    /// bar answer it as a histogram bin whose index is its row.
    RangeBars {
        data: crate::data_render::CategoryBarData,
        /// The rows its kernel tests, which size its dispatch (#445).
        rows: u32,
    },
    /// A range link series' links (#357), picked by the range link entry
    /// (`pick_range_links`) over the group 3 of the bars they join, at the
    /// pick's display scale: a link's segments and head answer it as a
    /// histogram bin whose index is its row.
    RangeLinks {
        data: crate::data_render::CategoryBarData,
        /// The rows its kernel tests, which size its dispatch (#445).
        rows: u32,
    },
    /// A box plot series' violins, picked by the violin entry
    /// (`pick_violin`) over the violin draw's group 3 and style map at the
    /// pick's display scale: one test per category and half, answering the
    /// category as a histogram bin.
    Violin {
        data: crate::data_render::CategoryBarData,
        /// The rows its kernel tests, which size its dispatch (#445).
        rows: u32,
    },
    /// A series' value labels, picked by the bar module's label entry
    /// (`pick_value_labels`) over the draw's label uniform, category bar
    /// group 3 and style map at the pick's display scale. A label answers as
    /// [`DATA_PICK_KIND_VALUE_LABEL`] whose index is its category.
    ValueLabels {
        data: crate::data_render::CategoryBarData,
        labels: crate::data_render::ValueLabelData,
    },
    /// A parallel coordinates series' rows (#141), picked by the line
    /// module's entry (`pick_parallel`) over the draw's group 3 at the
    /// pick's display scale: the nearest segment's row answers as a point
    /// whose index is the row.
    Parallel {
        data: crate::data_render::ParallelData,
    },
    /// A triangle mesh series' triangles and contour strokes (#210), picked
    /// by the mesh module's entry (`pick_tri_mesh`) over the draw's group 2
    /// at the pick's display scale: a triangle answers as
    /// [`DATA_PICK_KIND_MESH_TRIANGLE`], a stroke as a contour level whose
    /// `x_index` is the triangle.
    TriMesh {
        field_bg: wgpu::BindGroup,
        charge: SharedCharge,
        /// At least the triangles its kernel tests, which size its
        /// dispatch (#445).
        triangles: u32,
        has_fill: bool,
        has_contour: bool,
        contour_width_px: f32,
        /// The fill's premultiplied `nan_color`, as for a field.
        nan_color_premul: [f32; 4],
    },
}

pub(crate) struct GpuDataPickSeries {
    pub(crate) source_id: Option<String>,
    pub(crate) series_id: String,
    pub(crate) paint_order: u32,
    pub(crate) geometry: GpuDataPickGeometry,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GpuDataPickQuery {
    pub(crate) transform: ScatterTransform,
    pub(crate) chart_rect_px: [f32; 4],
    pub(crate) data_area_px: Option<[f32; 4]>,
    pub(crate) canvas_position_px: [f32; 2],
    pub(crate) max_distance_px: f32,
}

pub(crate) struct GpuStreamDataPick {
    bundle: Arc<DataPickPipelineBundle>,
    /// The running reductions of a data candidate and of the point answer,
    /// created by the first streamed typed pick (#327).
    reduce_data: wgpu::ComputePipeline,
    reduce_point: wgpu::ComputePipeline,
    query: GpuDataPickQuery,
    identities: Arc<[DataPickIdentity]>,
    candidate: wgpu::Buffer,
    best: wgpu::Buffer,
    charge: SharedCharge,
    has_chunks: bool,
    enabled: bool,
}

pub(crate) struct GpuStreamFieldQuery {
    pub query: wgpu::Buffer,
    pub candidate: wgpu::Buffer,
    pub _query_charge: SharedCharge,
    pub _candidate_charge: SharedCharge,
}

impl DataPickPipelineBundle {
    /// Begin a streamed typed pick; its two reduction kernels are created
    /// from `kernels` when this is the first one (#327).
    pub(crate) fn begin_stream(
        self: &Arc<Self>,
        kernels: &mut DataPickKernels,
        query: GpuDataPickQuery,
        identities: Vec<(Option<String>, String)>,
    ) -> Result<GpuStreamDataPick, crate::gpu_pick::GpuPickError> {
        let reduce_data = kernels
            .ensure(self, DataPickKernel::StreamReduceData)?
            .clone();
        let reduce_point = kernels
            .ensure(self, DataPickKernel::StreamReducePoint)?
            .clone();
        let tally = ChargeTally::new();
        let make_scalar = |label| {
            charged_buffer(
                &tally,
                &self.device,
                &wgpu::BufferDescriptor {
                    label: Some(label),
                    size: DATA_PICK_CANDIDATE_BYTES,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            )
            .map_err(crate::gpu_pick::GpuPickError::from_figgy)
        };
        let candidate = make_scalar("figgy stream typed pick candidate")?;
        let best = make_scalar("figgy stream typed pick best")?;
        let [cx, cy] = query.canvas_position_px;
        let [x, y, w, h] = query.chart_rect_px;
        let enabled = [cx, cy, x, y, w, h, query.max_distance_px]
            .into_iter()
            .all(f32::is_finite)
            && w > 0.0
            && h > 0.0
            && query.max_distance_px >= 0.0
            && query.data_area_px.is_none_or(|[x, y, w, h]| {
                [x, y, w, h].into_iter().all(f32::is_finite)
                    && cx >= x
                    && cx <= x + w
                    && cy >= y
                    && cy <= y + h
            });
        Ok(GpuStreamDataPick {
            bundle: Arc::clone(self),
            reduce_data,
            reduce_point,
            query,
            identities: identities
                .into_iter()
                .enumerate()
                .map(|(index, (source_id, series_id))| DataPickIdentity {
                    source_id,
                    series_id,
                    paint_order: index as u32,
                    violin: false,
                    extent: false,
                })
                .collect(),
            candidate,
            best,
            charge: shared_charge(tally, &self.ledger, GpuResourceKind::PickScratch),
            has_chunks: false,
            enabled,
        })
    }
}

impl GpuStreamDataPick {
    pub(crate) fn field_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn field_query(
        &self,
        paint_order: u32,
    ) -> Result<GpuStreamFieldQuery, crate::gpu_pick::GpuPickError> {
        if paint_order as usize >= self.identities.len() {
            return Err(crate::gpu_pick::GpuPickError::InvalidGpuResult);
        }
        let [cx, cy] = self.query.canvas_position_px;
        let [x, y, w, h] = self.query.chart_rect_px;
        let nx = ((cx - x) / w) * 2.0 - 1.0;
        let ny = 1.0 - ((cy - y) / h) * 2.0;
        let params = DataPickQueryGpu {
            cursor_ndc_t: [nx, ny, (nx + 1.0) * 0.5, (ny + 1.0) * 0.5],
            limits: [self.query.max_distance_px, 0.0, 0.0, 0.0],
            data: [DATA_PICK_FLAG_FILL, paint_order, 0, 0],
            bases: [0; 4],
            baseline: [0.0; 4],
        };
        let tally = ChargeTally::new();
        let query = charged_buffer_init(
            &tally,
            &self.bundle.device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("stream field pick query"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            },
        )
        .map_err(crate::gpu_pick::GpuPickError::from_figgy)?;
        Ok(GpuStreamFieldQuery {
            query,
            candidate: self.candidate.clone(),
            _query_charge: shared_charge(tally, &self.bundle.ledger, GpuResourceKind::PickScratch),
            _candidate_charge: Arc::clone(&self.charge),
        })
    }

    pub(crate) fn encode_field_candidate(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let submitted_chunks = self.has_chunks;
        self.accumulate(encoder, &self.candidate.clone(), false);
        self.has_chunks = submitted_chunks;
    }

    pub(crate) fn commit_field_candidate(&mut self) {
        self.has_chunks = true;
    }
    pub(crate) const PERSISTENT_BYTES: u64 = 2 * DATA_PICK_CANDIDATE_BYTES;
    pub(crate) const READBACK_BYTES: u64 = DATA_PICK_CANDIDATE_BYTES;

    fn accumulate(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Buffer,
        point: bool,
    ) {
        if !self.has_chunks {
            encoder.clear_buffer(&self.best, 0, None);
        }
        let bg = self
            .bundle
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("figgy stream typed pick running reduction bg"),
                layout: &self.bundle.stream_reduce_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: input.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.best.as_entire_binding(),
                    },
                ],
            });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("figgy stream typed pick running reduction"),
            timestamp_writes: None,
        });
        pass.set_pipeline(if point {
            &self.reduce_point
        } else {
            &self.reduce_data
        });
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(1, 1, 1);
        self.has_chunks = true;
    }

    /// Reduce the final point/line answer on the GPU. Point and typed streams
    /// must use the same chart-paint-order identity table.
    pub(crate) fn encode_point_result(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        point: &crate::gpu_pick::GpuStreamPick,
    ) {
        if self.enabled
            && let Some(buffer) = point.result_buffer()
        {
            self.accumulate(encoder, buffer, true);
        }
    }

    /// All source chunks and the optional point merge must already be submitted.
    pub(crate) fn finish(self) -> Result<GpuDataPickTicket, crate::gpu_pick::GpuPickError> {
        if !self.has_chunks {
            return Ok(GpuDataPickTicket {
                state: GpuDataPickTicketState::Stream(DataPickTicket::ready_none()),
            });
        }
        let tally = ChargeTally::new();
        let readback = charged_buffer(
            &tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy stream typed pick readback"),
                size: DATA_PICK_CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        )
        .map_err(crate::gpu_pick::GpuPickError::from_figgy)?;
        let mut encoder =
            self.bundle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy stream typed pick finish"),
                });
        encoder.copy_buffer_to_buffer(&self.best, 0, &readback, 0, DATA_PICK_CANDIDATE_BYTES);
        self.bundle.queue.submit([encoder.finish()]);
        Ok(GpuDataPickTicket {
            state: GpuDataPickTicketState::Stream(DataPickTicket {
                state: DataPickTicketState::Pending {
                    readback: GpuReadback::map(
                        &self.bundle.device,
                        &self.bundle.health,
                        &readback,
                        Some(DATA_PICK_CANDIDATE_BYTES),
                    ),
                    identities: self.identities,
                    gpu_reduced: true,
                    slot: None,
                    _prepared: None,
                    _field_charges: vec![FieldPickHold {
                        _charge: self.charge,
                        _label: None,
                    }],
                    _scratch_charge: Some(
                        ChargeTally::new()
                            .into_charge(&self.bundle.ledger, GpuResourceKind::PickScratch),
                    ),
                    _readback_charge: Some(
                        tally.into_charge(&self.bundle.ledger, GpuResourceKind::Readback),
                    ),
                },
            }),
        })
    }
}

/// The shader modules the typed data pick's compute entries live in, next to
/// the render entries they share geometry with.
#[derive(Clone, Copy)]
pub(crate) struct DataPickShaders<'a> {
    pub(crate) bar: &'a wgpu::ShaderModule,
    pub(crate) field: &'a wgpu::ShaderModule,
    /// The assembled grid vector module (`VectorInput::Grid`).
    pub(crate) vector_grid: &'a wgpu::ShaderModule,
    /// The line module, whose `pick_parallel` answers parallel rows (#141).
    pub(crate) line: &'a wgpu::ShaderModule,
    /// Which shader set the four modules were assembled as (#299):
    /// the bundle's own mesh module follows it.
    pub(crate) set: crate::shader_assembly::ShaderSet,
}

/// The query of a category bar or box plot pick: the cursor, the pick
/// distance and the paint order; the group 3 params hold the rest.
fn category_axis_pick_query(
    cursor_ndc_t: [f32; 4],
    max_distance_px: f32,
    paint_order: u32,
) -> DataPickQueryGpu {
    DataPickQueryGpu {
        cursor_ndc_t,
        limits: [max_distance_px, 0.0, 0.0, 0.0],
        data: [0, paint_order, 0, 0],
        bases: [0; 4],
        baseline: [0.0; 4],
    }
}

/// One typed data pick compute kernel. Each is created on the first pick
/// that dispatches it ([`DataPickKernels`], #327), never ahead of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DataPickKernel {
    Histogram,
    FieldContour,
    FieldCell,
    VectorCell,
    CategoryBar,
    BoxPlot,
    RangeBar,
    RangeLink,
    Violin,
    Parallel,
    TriMesh,
    ValueLabels,
    /// The streamed typed pick's running reduction of a data candidate.
    StreamReduceData,
    /// The streamed typed pick's running reduction of the point answer.
    StreamReducePoint,
    /// The second pass of a histogram pick (#445): its workgroup partials
    /// folded into the series' answer, over the histogram layout.
    ReduceHistogram,
    /// The second pass of a category bar, box plot, range bar, range link
    /// or violin pick (#445), over the category layout.
    ReduceCategory,
    /// The second pass of a value label pick (#445).
    ReduceValueLabels,
    /// The second pass of a parallel coordinates pick (#445).
    ReduceParallel,
    /// The second pass of a triangle mesh pick (#445), with its contour
    /// test.
    ReduceTriMesh,
}

impl DataPickKernel {
    pub(crate) const ALL: [Self; 19] = [
        Self::Histogram,
        Self::FieldContour,
        Self::FieldCell,
        Self::VectorCell,
        Self::CategoryBar,
        Self::BoxPlot,
        Self::RangeBar,
        Self::RangeLink,
        Self::Violin,
        Self::Parallel,
        Self::TriMesh,
        Self::ValueLabels,
        Self::StreamReduceData,
        Self::StreamReducePoint,
        Self::ReduceHistogram,
        Self::ReduceCategory,
        Self::ReduceValueLabels,
        Self::ReduceParallel,
        Self::ReduceTriMesh,
    ];

    /// The second pass that folds this body kernel's workgroup partials
    /// (#445); `None` for a kernel that writes its series' answer itself.
    pub(crate) const fn reducer(self) -> Option<Self> {
        match self {
            Self::Histogram => Some(Self::ReduceHistogram),
            Self::CategoryBar | Self::BoxPlot | Self::RangeBar | Self::RangeLink | Self::Violin => {
                Some(Self::ReduceCategory)
            }
            Self::ValueLabels => Some(Self::ReduceValueLabels),
            Self::Parallel => Some(Self::ReduceParallel),
            Self::TriMesh => Some(Self::ReduceTriMesh),
            _ => None,
        }
    }

    /// Workgroup partials one body workgroup writes (#445): a triangle
    /// mesh writes its fill's and its contour test's.
    const fn partials_per_group(self) -> u64 {
        match self {
            Self::TriMesh => 2,
            _ => 1,
        }
    }

    /// Its slot in [`DataPickKernels`] and its bit in [`DataPickKernelSet`].
    const fn index(self) -> usize {
        self as usize
    }

    /// The compute entry point it runs.
    pub(crate) const fn entry(self) -> &'static str {
        match self {
            Self::Histogram => "pick_histogram_bin",
            Self::FieldContour => "pick_field_contour",
            Self::FieldCell => "pick_field_cell",
            Self::VectorCell => "pick_vector_cell",
            Self::CategoryBar => "pick_category_bar",
            Self::BoxPlot => "pick_box_plot",
            Self::RangeBar => "pick_range_bar",
            Self::RangeLink => "pick_range_links",
            Self::Violin => "pick_violin",
            Self::Parallel => "pick_parallel",
            Self::TriMesh => "pick_tri_mesh",
            Self::ValueLabels => "pick_value_labels",
            Self::StreamReduceData => "accumulate_data",
            Self::StreamReducePoint => "accumulate_point",
            Self::ReduceHistogram | Self::ReduceCategory | Self::ReduceValueLabels => {
                "pick_reduce_data"
            }
            Self::ReduceParallel => "pick_reduce_parallel",
            Self::ReduceTriMesh => "pick_reduce_tri_mesh",
        }
    }

    /// Its pipeline's label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Histogram => "figgy histogram data pick pipeline",
            Self::FieldContour => "figgy field contour data pick pipeline",
            Self::FieldCell => "figgy field cell data pick pipeline",
            Self::VectorCell => "figgy vector grid data pick pipeline",
            Self::CategoryBar => "figgy category bar data pick pipeline",
            Self::BoxPlot => "figgy box plot data pick pipeline",
            Self::RangeBar => "figgy range bar data pick pipeline",
            Self::RangeLink => "figgy range link data pick pipeline",
            Self::Violin => "figgy violin data pick pipeline",
            Self::Parallel => "figgy parallel data pick pipeline",
            Self::TriMesh => "figgy triangle mesh data pick pipeline",
            Self::ValueLabels => "figgy value label data pick pipeline",
            Self::StreamReduceData => "accumulate_data",
            Self::StreamReducePoint => "accumulate_point",
            Self::ReduceHistogram => "figgy histogram data pick reduction pipeline",
            Self::ReduceCategory => "figgy category data pick reduction pipeline",
            Self::ReduceValueLabels => "figgy value label data pick reduction pipeline",
            Self::ReduceParallel => "figgy parallel data pick reduction pipeline",
            Self::ReduceTriMesh => "figgy triangle mesh data pick reduction pipeline",
        }
    }

    /// The WGSL source its pipeline compiles, for the shader set `set`.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn source(self, set: crate::shader_assembly::ShaderSet) -> &'static str {
        match self {
            Self::Histogram
            | Self::CategoryBar
            | Self::BoxPlot
            | Self::RangeBar
            | Self::RangeLink
            | Self::Violin
            | Self::ValueLabels
            | Self::ReduceHistogram
            | Self::ReduceCategory
            | Self::ReduceValueLabels => crate::data_render::BAR_SHADER_BY_SET.of(set).source,
            Self::FieldContour | Self::FieldCell => {
                crate::data_render::FIELD_SHADER_BY_SET.of(set).source
            }
            Self::VectorCell => crate::data_render::vector::assemble_by_set(
                crate::data_render::vector::VectorInput::Grid,
                set,
            ),
            Self::Parallel | Self::ReduceParallel => {
                crate::data_render::LINE_SHADER_BY_SET.of(set).source
            }
            Self::TriMesh | Self::ReduceTriMesh => {
                crate::data_render::tri_mesh::TRI_MESH_SHADER_BY_SET
                    .of(set)
                    .source
            }
            Self::StreamReduceData | Self::StreamReducePoint => STREAM_REDUCE_SHADER,
        }
    }
}

const STREAM_REDUCE_SHADER: &str = include_str!("gpu_stream_data_pick.wgsl");

/// The single-workgroup body kernels the parallel picks replaced (#445),
/// for tests: one workgroup of 64 lanes strides over every row, then folds
/// the lanes, as each kernel did before. Each is appended to its own
/// module and writes the first workgroup partial, which the module's
/// reduction folds into the series' answer.
#[cfg(test)]
mod single_workgroup_oracle {
    use super::DataPickKernel;
    use crate::shader_assembly::ShaderSet;

    /// The module source with the oracle of `kernel` appended, and the
    /// oracle's entry point.
    pub(super) fn source(kernel: DataPickKernel, set: ShaderSet) -> (String, String) {
        let bar = |count: &str, candidate: &str| {
            format!(
                r#"
@compute @workgroup_size(64)
fn oracle_{entry}(@builtin(local_invocation_index) local_index: u32) {{
    figgy_zero_workgroup(local_index);
    var best = data_pick_invalid();
    var row = local_index;
    while (row < {count}) {{
        let candidate = {candidate};
        if (data_pick_better(candidate, best)) {{
            best = candidate;
        }}
        row = row + 64u;
    }}
    let folded = data_pick_fold(local_index, best);
    if (local_index == 0u) {{
        data_pick_partials[0] = folded;
    }}
}}
"#,
                entry = kernel.entry(),
            )
        };
        let base = match kernel {
            DataPickKernel::Parallel => crate::data_render::LINE_SHADER_BY_SET.of(set).source,
            DataPickKernel::TriMesh => {
                crate::data_render::tri_mesh::TRI_MESH_SHADER_BY_SET
                    .of(set)
                    .source
            }
            _ => crate::data_render::BAR_SHADER_BY_SET.of(set).source,
        };
        let oracle = match kernel {
            DataPickKernel::Histogram => bar("data_pick_query.data.z", "data_pick_bar(row)"),
            DataPickKernel::CategoryBar => bar("category_params.count", "category_pick_bar(row)"),
            DataPickKernel::BoxPlot => bar("category_params.count", "box_pick_row(row)"),
            DataPickKernel::RangeBar => bar("category_params.count", "range_pick_row(row)"),
            DataPickKernel::RangeLink => bar("range_link_count()", "range_link_pick(row)"),
            DataPickKernel::Violin => bar("category_params.count", "violin_pick_row(row)"),
            DataPickKernel::ValueLabels => bar("value_label.shape.y", "value_label_pick(row)"),
            DataPickKernel::Parallel => r#"
@compute @workgroup_size(64)
fn oracle_pick_parallel(@builtin(local_invocation_index) local_index: u32) {
    figgy_zero_workgroup(local_index);
    var best = parallel_pick_invalid();
    let segments = max(parallel_params.axes, 2u) - 1u;
    let count = parallel_params.rows * segments;
    var index = local_index;
    while (index < count) {
        let candidate = parallel_pick_candidate(index);
        if (parallel_pick_better(candidate, best)) {
            best = candidate;
        }
        index = index + 64u;
    }
    let folded = parallel_pick_fold(local_index, best);
    if (local_index == 0u) {
        data_pick_partials[0] = folded;
    }
}
"#
            .to_owned(),
            DataPickKernel::TriMesh => r#"
@compute @workgroup_size(64)
fn oracle_pick_tri_mesh(@builtin(local_invocation_index) local_index: u32) {
    figgy_zero_workgroup(local_index);
    let cursor = mesh_pick_cursor();
    let fill = (data_pick_query.data.x & DATA_PICK_FLAG_FILL) != 0u;
    let contour = (data_pick_query.data.x & DATA_PICK_FLAG_CONTOUR) != 0u;
    var best = mesh_pick_invalid();
    var near = mesh_pick_invalid();
    let count = mesh_triangle_count();
    var tri = local_index;
    while (tri < count) {
        if (fill) {
            let candidate = mesh_pick_candidate(tri, cursor);
            if (mesh_pick_better(candidate, best)) {
                best = candidate;
            }
        }
        if (contour) {
            let candidate = mesh_pick_near_candidate(tri, cursor);
            if (mesh_pick_better(candidate, near)) {
                near = candidate;
            }
        }
        tri = tri + 64u;
    }
    mesh_pick_fold(local_index, best, near);
    if (local_index == 0u) {
        data_pick_partials[0] = mesh_pick_shared[0];
        data_pick_partials[1] = mesh_pick_near[0];
    }
}
"#
            .to_owned(),
            _ => panic!("{kernel:?} has no single-workgroup oracle"),
        };
        (
            format!("{base}\n{oracle}"),
            format!("oracle_{}", kernel.entry()),
        )
    }
}

/// A set of [`DataPickKernel`]s, one bit each: the kernels a pick of one
/// chart dispatches (#327).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DataPickKernelSet(u32);

impl DataPickKernelSet {
    pub(crate) const EMPTY: Self = Self(0);

    /// The set with `kernel` and the second pass that folds its partials
    /// (#445), which every dispatch of it runs.
    pub(crate) const fn with(self, kernel: DataPickKernel) -> Self {
        let reducer = match kernel.reducer() {
            Some(reducer) => 1 << reducer.index(),
            None => 0,
        };
        Self(self.0 | (1 << kernel.index()) | reducer)
    }

    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) const fn contains(self, kernel: DataPickKernel) -> bool {
        self.0 & (1 << kernel.index()) != 0
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn iter(self) -> impl Iterator<Item = DataPickKernel> {
        DataPickKernel::ALL
            .into_iter()
            .filter(move |kernel| self.contains(*kernel))
    }
}

/// The typed data pick kernels created so far (#327). Each is created on
/// the first pick that dispatches it and kept for the renderer's
/// lifetime; owned by the renderer's picker next to its exact pipelines
/// (`PickExactPipelines`) and passed to each pick, so no lock guards it.
#[derive(Default)]
pub(crate) struct DataPickKernels {
    pipelines: [Option<wgpu::ComputePipeline>; DataPickKernel::ALL.len()],
    /// The bundle's own triangle mesh module: the draw compiles its own
    /// only when a mesh is drawn.
    tri_mesh_module: Option<wgpu::ShaderModule>,
    /// The streamed typed pick's reduction module, shared by both reducers.
    stream_module: Option<wgpu::ShaderModule>,
    /// Tests: compile every body kernel as the single-workgroup kernel it
    /// replaced (#445), one workgroup striding over every row, to compare
    /// the parallel picks with. Set before any kernel is created.
    #[cfg(test)]
    pub(crate) single_workgroup: bool,
    /// Tests: a smaller per-dimension workgroup limit, so a test reaches
    /// the two-dimensional dispatch with rows it can afford (#445).
    #[cfg(test)]
    pub(crate) max_groups_per_dimension: Option<u32>,
}

impl DataPickKernels {
    /// The pipeline of `kernel`, created from `bundle` if it is not yet.
    pub(crate) fn ensure(
        &mut self,
        bundle: &DataPickPipelineBundle,
        kernel: DataPickKernel,
    ) -> Result<&wgpu::ComputePipeline, crate::gpu_pick::GpuPickError> {
        #[cfg(test)]
        let single_workgroup = self.single_workgroup && kernel.reducer().is_some();
        let Self {
            pipelines,
            tri_mesh_module,
            stream_module,
            ..
        } = self;
        let slot =
            pipelines
                .get_mut(kernel.index())
                .ok_or(crate::gpu_pick::GpuPickError::Internal(
                    "every data pick kernel has a slot",
                ))?;
        if slot.is_none() {
            let device = &bundle.device;
            // The streamed reductions keep no cache: streaming is not
            // extended (rule 8, #478).
            let cache = match kernel {
                DataPickKernel::StreamReduceData | DataPickKernel::StreamReducePoint => None,
                _ => bundle.pipeline_cache.as_ref(),
            };
            let (layout, module) = match kernel {
                DataPickKernel::Histogram => (&bundle.histogram_layout, &bundle.bar),
                DataPickKernel::FieldContour | DataPickKernel::FieldCell => {
                    (&bundle.field_layout, &bundle.field)
                }
                DataPickKernel::VectorCell => (&bundle.vector_layout, &bundle.vector_grid),
                DataPickKernel::CategoryBar
                | DataPickKernel::BoxPlot
                | DataPickKernel::RangeBar
                | DataPickKernel::RangeLink
                | DataPickKernel::Violin => (&bundle.category_layout, &bundle.bar),
                DataPickKernel::ReduceHistogram => (&bundle.histogram_layout, &bundle.bar),
                DataPickKernel::ReduceCategory => (&bundle.category_layout, &bundle.bar),
                DataPickKernel::ReduceValueLabels => (&bundle.value_label_layout, &bundle.bar),
                DataPickKernel::Parallel | DataPickKernel::ReduceParallel => {
                    (&bundle.parallel_layout, &bundle.line)
                }
                DataPickKernel::TriMesh | DataPickKernel::ReduceTriMesh => (
                    &bundle.tri_mesh_layout,
                    &*tri_mesh_module.get_or_insert_with(|| {
                        crate::data_render::tri_mesh::shader_module(device, bundle.set)
                    }),
                ),
                DataPickKernel::ValueLabels => (&bundle.value_label_layout, &bundle.bar),
                DataPickKernel::StreamReduceData | DataPickKernel::StreamReducePoint => (
                    &bundle.stream_reduce_layout,
                    &*stream_module.get_or_insert_with(|| {
                        crate::gpu_compile::shader_module(
                            device,
                            wgpu::ShaderModuleDescriptor {
                                label: Some("figgy streamed typed pick reduction shader"),
                                source: wgpu::ShaderSource::Wgsl(STREAM_REDUCE_SHADER.into()),
                            },
                        )
                    }),
                ),
            };
            // The field, vector and reduction entries need no workgroup
            // zeroing workaround; the others are listed in
            // `workgroup_init::EXPLICIT_ZERO_ENTRIES`.
            let compilation_options = match kernel {
                DataPickKernel::FieldContour
                | DataPickKernel::FieldCell
                | DataPickKernel::VectorCell
                | DataPickKernel::StreamReduceData
                | DataPickKernel::StreamReducePoint => wgpu::PipelineCompilationOptions::default(),
                _ => crate::gpu_compile::workgroup_options(device),
            };
            #[cfg(test)]
            if single_workgroup {
                let (source, entry) = single_workgroup_oracle::source(kernel, bundle.set);
                let module = crate::gpu_compile::shader_module(
                    device,
                    wgpu::ShaderModuleDescriptor {
                        label: Some("figgy single-workgroup data pick oracle"),
                        source: wgpu::ShaderSource::Wgsl(source.into()),
                    },
                );
                *slot = Some(crate::gpu_compile::compute_pipeline(
                    device,
                    cache,
                    &wgpu::ComputePipelineDescriptor {
                        label: Some(kernel.label()),
                        layout: Some(layout),
                        module: &module,
                        entry_point: Some(&entry),
                        compilation_options: compilation_options.clone(),
                        cache: None,
                    },
                ));
            }
            if slot.is_none() {
                *slot = Some(crate::gpu_compile::compute_pipeline(
                    device,
                    cache,
                    &wgpu::ComputePipelineDescriptor {
                        label: Some(kernel.label()),
                        layout: Some(layout),
                        module,
                        entry_point: Some(kernel.entry()),
                        compilation_options,
                        cache: None,
                    },
                ));
            }
        }
        slot.as_ref().ok_or(crate::gpu_pick::GpuPickError::Internal(
            "a data pick kernel was just created",
        ))
    }

    /// The pipeline of `kernel` when it has been created.
    #[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
    pub(crate) fn get(&self, kernel: DataPickKernel) -> Option<&wgpu::ComputePipeline> {
        self.pipelines.get(kernel.index())?.as_ref()
    }

    /// Drop each kernel `keep` leaves out (#363); the shared modules stay.
    pub(crate) fn retain(&mut self, keep: DataPickKernelSet) {
        for kernel in DataPickKernel::ALL {
            if !keep.contains(kernel)
                && let Some(slot) = self.pipelines.get_mut(kernel.index())
            {
                *slot = None;
            }
        }
    }
}

/// Immutable layouts and placeholders shared by every chart and pick query
/// on one renderer. The kernels themselves are created on demand
/// ([`DataPickKernels`], #327).
pub(crate) struct DataPickPipelineBundle {
    /// Which shader set every module of the bundle was assembled as
    /// (#299).
    pub(crate) set: crate::shader_assembly::ShaderSet,
    device: wgpu::Device,
    queue: wgpu::Queue,
    ledger: Arc<GpuLedger>,
    /// The device's health record; every typed pick readback polls through it.
    health: crate::renderer::DeviceHealth,
    /// The renderer's pipeline cache (#478). The streamed reduction kernels
    /// do not use it (rule 8).
    pipeline_cache: Option<wgpu::PipelineCache>,
    /// Group 0 of every resident kernel: the pick's display transform.
    transform_bgl: wgpu::BindGroupLayout,
    query_bgl: wgpu::BindGroupLayout,
    /// Group 1 of a body kernel that spreads over many workgroups (#445):
    /// the query (1), the pool (2), its workgroup partials (3) and the
    /// series' answer (8).
    partials_query_bgl: wgpu::BindGroupLayout,
    /// The renderer's modules the kernels' entries live in.
    bar: wgpu::ShaderModule,
    field: wgpu::ShaderModule,
    vector_grid: wgpu::ShaderModule,
    line: wgpu::ShaderModule,
    histogram_layout: wgpu::PipelineLayout,
    /// A field series picks with its two kernels in turn: the contour, then
    /// the cell unless the contour wrote a candidate (`field_columnar.wgsl`,
    /// #222).
    field_layout: wgpu::PipelineLayout,
    vector_layout: wgpu::PipelineLayout,
    /// Category bars, box plots, range bars and violins.
    category_layout: wgpu::PipelineLayout,
    /// The parallel row pick over the parallel data layout as group 3.
    parallel_layout: wgpu::PipelineLayout,
    /// The triangle mesh pick over the field's layout as group 2 (#210).
    tri_mesh_layout: wgpu::PipelineLayout,
    /// Group 1 of the value label pick: the query (1, 2), its workgroup
    /// partials (3), the series' label uniform (4), its label cells (7)
    /// and its answer (8).
    value_label_query_bgl: wgpu::BindGroupLayout,
    value_label_layout: wgpu::PipelineLayout,
    empty_bar_style_map: wgpu::BindGroup,
    /// Group 3 of the field pick for a contour without labels.
    empty_label_gap: wgpu::BindGroup,
    _empty_label_gap_charge: SharedCharge,
    stream_reduce_bgl: wgpu::BindGroupLayout,
    stream_reduce_layout: wgpu::PipelineLayout,
}

impl DataPickPipelineBundle {
    /// The bundle's layouts and placeholders. No kernel is created here:
    /// each is created by the first pick that dispatches it (#327).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_observed(
        device: wgpu::Device,
        queue: wgpu::Queue,
        ledger: Arc<GpuLedger>,
        health: crate::renderer::DeviceHealth,
        pipeline_cache: Option<wgpu::PipelineCache>,
        shaders: DataPickShaders<'_>,
        transform_bgl: &wgpu::BindGroupLayout,
        field_bgl: &wgpu::BindGroupLayout,
        bar_style_bgl: &wgpu::BindGroupLayout,
        vector_bgl: &wgpu::BindGroupLayout,
        category_bgl: &wgpu::BindGroupLayout,
        parallel_bgl: &wgpu::BindGroupLayout,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Arc<Self>, crate::gpu_pick::GpuPickError> {
        let DataPickShaders {
            bar,
            field,
            vector_grid,
            line,
            set,
        } = shaders;
        started(observer, INIT_SCOPE, "setup");
        let query_bgl = crate::gpu_compile::bind_group_layout(
            &device,
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("figgy typed data pick query bgl"),
                entries: &[
                    uniform_entry(1),
                    storage_entry(2, true),
                    storage_entry(3, false),
                ],
            },
        );
        let partials_query_bgl = crate::gpu_compile::bind_group_layout(
            &device,
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("figgy typed data pick partials query bgl"),
                entries: &[
                    uniform_entry(1),
                    storage_entry(2, true),
                    storage_entry(3, false),
                    storage_entry(8, false),
                ],
            },
        );
        let histogram_layout = crate::gpu_compile::pipeline_layout(
            &device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("figgy histogram data pick layout"),
                bind_group_layouts: &[
                    Some(transform_bgl),
                    Some(&partials_query_bgl),
                    Some(bar_style_bgl),
                ],
                immediate_size: 0,
            },
        );
        // Group 3: the contour's label gaps (`contour_label_gap_contains`).
        let label_gap_bgl = crate::gpu_contour::create_label_gap_bind_group_layout(&device);
        let field_layout = crate::gpu_compile::pipeline_layout(
            &device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("figgy field data pick layout"),
                bind_group_layouts: &[
                    Some(transform_bgl),
                    Some(&query_bgl),
                    Some(field_bgl),
                    Some(&label_gap_bgl),
                ],
                immediate_size: 0,
            },
        );
        let (empty_label_gap, empty_label_gap_charge) =
            crate::gpu_contour::create_empty_label_gap_bind_group(&device, &ledger, &label_gap_bgl)
                .map_err(crate::gpu_pick::GpuPickError::from_figgy)?;
        let vector_layout = crate::gpu_compile::pipeline_layout(
            &device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("figgy vector grid data pick layout"),
                bind_group_layouts: &[Some(transform_bgl), Some(&query_bgl), Some(vector_bgl)],
                immediate_size: 0,
            },
        );
        let category_layout = crate::gpu_compile::pipeline_layout(
            &device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("figgy category bar data pick layout"),
                bind_group_layouts: &[
                    Some(transform_bgl),
                    Some(&partials_query_bgl),
                    Some(bar_style_bgl),
                    Some(category_bgl),
                ],
                immediate_size: 0,
            },
        );
        let parallel_layout = crate::gpu_compile::pipeline_layout(
            &device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("figgy parallel data pick layout"),
                bind_group_layouts: &[
                    Some(transform_bgl),
                    Some(&partials_query_bgl),
                    None,
                    Some(parallel_bgl),
                ],
                immediate_size: 0,
            },
        );
        let tri_mesh_layout = crate::gpu_compile::pipeline_layout(
            &device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("figgy triangle mesh data pick layout"),
                bind_group_layouts: &[
                    Some(transform_bgl),
                    Some(&partials_query_bgl),
                    Some(field_bgl),
                ],
                immediate_size: 0,
            },
        );
        let value_label_query_bgl = crate::gpu_compile::bind_group_layout(
            &device,
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("figgy value label pick query bgl"),
                entries: &[
                    uniform_entry(1),
                    storage_entry(2, true),
                    storage_entry(3, false),
                    uniform_entry(4),
                    storage_entry(7, true),
                    storage_entry(8, false),
                ],
            },
        );
        let value_label_layout = crate::gpu_compile::pipeline_layout(
            &device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("figgy value label data pick layout"),
                bind_group_layouts: &[
                    Some(transform_bgl),
                    Some(&value_label_query_bgl),
                    Some(bar_style_bgl),
                    Some(category_bgl),
                ],
                immediate_size: 0,
            },
        );
        let empty_bar_style_map =
            crate::data_render::create_bar_style_map_with_slots(&device, bar_style_bgl, &[], &[])
                .map_err(crate::gpu_pick::GpuPickError::from_figgy)?
                .bind_group;
        let stream_reduce_bgl = crate::gpu_compile::bind_group_layout(
            &device,
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("figgy streamed typed pick reduction bgl"),
                entries: &[storage_entry(0, true), storage_entry(1, false)],
            },
        );
        let stream_reduce_layout = crate::gpu_compile::pipeline_layout(
            &device,
            &wgpu::PipelineLayoutDescriptor {
                label: Some("figgy streamed typed pick reduction layout"),
                bind_group_layouts: &[Some(&stream_reduce_bgl)],
                immediate_size: 0,
            },
        );
        finished(observer, INIT_SCOPE, "setup");
        Ok(Arc::new(Self {
            set,
            device,
            queue,
            ledger,
            health,
            pipeline_cache,
            transform_bgl: transform_bgl.clone(),
            query_bgl,
            partials_query_bgl,
            bar: bar.clone(),
            field: field.clone(),
            vector_grid: vector_grid.clone(),
            line: line.clone(),
            histogram_layout,
            field_layout,
            vector_layout,
            category_layout,
            parallel_layout,
            tri_mesh_layout,
            value_label_query_bgl,
            value_label_layout,
            empty_bar_style_map,
            empty_label_gap,
            _empty_label_gap_charge: empty_label_gap_charge,
            stream_reduce_bgl,
            stream_reduce_layout,
        }))
    }

    /// Prepare one typed data pick's work (#445): every series' query
    /// template, bind groups, kernels and dispatch size, and the scratch
    /// buffers they write. The display transform, the cursor and the pick
    /// distance are left to [`Self::encode`], so the renderer keeps the result while the
    /// series and the display it was prepared for are unchanged and a pick
    /// at another cursor reuses it. Each series' kernel is created from
    /// `kernels` when it is the first pick that dispatches it (#327).
    pub(crate) fn prepare(
        &self,
        kernels: &mut DataPickKernels,
        pool: &ColumnPool,
        series: Vec<GpuDataPickSeries>,
    ) -> Result<Arc<PreparedDataPick>, crate::gpu_pick::GpuPickError> {
        let candidate_count = u64::try_from(series.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick candidate count",
            }
        })?;
        let candidate_bytes = candidate_count
            .checked_mul(DATA_PICK_CANDIDATE_BYTES)
            .ok_or(crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick candidate bytes",
            })?;
        let limits = self.device.limits();
        let candidate_stride =
            u64::from(limits.min_storage_buffer_offset_alignment).max(DATA_PICK_CANDIDATE_BYTES);
        let query_stride = u64::from(limits.min_uniform_buffer_offset_alignment)
            .max(std::mem::size_of::<DataPickQueryGpu>() as u64);
        let output_bytes = candidate_count.checked_mul(candidate_stride).ok_or(
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick aligned output bytes",
            },
        )?;
        let query_bytes = candidate_count.checked_mul(query_stride).ok_or(
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick query bytes",
            },
        )?;
        let largest = candidate_bytes.max(output_bytes).max(query_bytes);
        if largest > limits.max_buffer_size {
            return Err(crate::gpu_pick::GpuPickError::DeviceLimit {
                resource: "typed data pick candidate buffer",
                requested: largest,
                limit: limits.max_buffer_size,
            });
        }

        let scratch_tally = ChargeTally::new();
        let transform_buffer = charged_buffer(
            &scratch_tally,
            &self.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy typed data pick transform"),
                size: std::mem::size_of::<ScatterTransform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        )
        .map_err(crate::gpu_pick::GpuPickError::from_figgy)?;
        let transform_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy typed data pick transform bg"),
            layout: &self.transform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: transform_buffer.as_entire_binding(),
            }],
        });
        let candidates = charged_buffer(
            &scratch_tally,
            &self.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy typed data pick candidates"),
                size: output_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
        )
        .map_err(crate::gpu_pick::GpuPickError::from_figgy)?;
        let queries = charged_buffer(
            &scratch_tally,
            &self.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy typed data pick queries"),
                size: query_bytes,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        )
        .map_err(crate::gpu_pick::GpuPickError::from_figgy)?;

        let max_groups = self.max_groups_per_dimension(kernels);
        let (staged, identities, field_charges) =
            self.stage_data_pick_series(kernels, series, max_groups)?;

        // One partials buffer for every body series, each at an offset the
        // device can bind (#445).
        let partial_alignment = u64::from(limits.min_storage_buffer_offset_alignment).max(1);
        let mut partials_bytes = 0u64;
        for item in &staged {
            if let Some(bytes) = item.partial_bytes()? {
                partials_bytes = partials_bytes
                    .checked_next_multiple_of(partial_alignment)
                    .and_then(|offset| offset.checked_add(bytes))
                    .ok_or(crate::gpu_pick::GpuPickError::AllocationFailed {
                        resource: "typed data pick partial bytes",
                    })?;
            }
        }
        if partials_bytes > limits.max_buffer_size {
            return Err(crate::gpu_pick::GpuPickError::DeviceLimit {
                resource: "typed data pick partials buffer",
                requested: partials_bytes,
                limit: limits.max_buffer_size,
            });
        }
        let partials = if partials_bytes == 0 {
            None
        } else {
            Some(
                charged_buffer(
                    &scratch_tally,
                    &self.device,
                    &wgpu::BufferDescriptor {
                        label: Some("figgy typed data pick workgroup partials"),
                        size: partials_bytes,
                        usage: wgpu::BufferUsages::STORAGE,
                        mapped_at_creation: false,
                    },
                )
                .map_err(crate::gpu_pick::GpuPickError::from_figgy)?,
            )
        };

        let mut prepared = Vec::new();
        prepared.try_reserve_exact(staged.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick prepared series",
            }
        })?;
        let mut params = Vec::new();
        params.try_reserve_exact(staged.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick query templates",
            }
        })?;
        let mut partial_offset = 0u64;
        for (index, item) in staged.into_iter().enumerate() {
            let at = |stride: u64, resource| {
                u64::try_from(index)
                    .ok()
                    .and_then(|index| index.checked_mul(stride))
                    .ok_or(crate::gpu_pick::GpuPickError::AllocationFailed { resource })
            };
            let output_offset = at(candidate_stride, "typed data pick output offset")?;
            let query_offset = at(query_stride, "typed data pick query offset")?;
            let output = wgpu::BufferBinding {
                buffer: &candidates,
                offset: output_offset,
                size: NonZeroU64::new(DATA_PICK_CANDIDATE_BYTES),
            };
            let query_entry = wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &queries,
                    offset: query_offset,
                    size: NonZeroU64::new(std::mem::size_of::<DataPickQueryGpu>() as u64),
                }),
            };
            let pool_entry = wgpu::BindGroupEntry {
                binding: 2,
                resource: pool.buffer().as_entire_binding(),
            };
            let query_bg = match (item.partial_bytes()?, &partials) {
                (Some(bytes), Some(partials)) => {
                    partial_offset = partial_offset
                        .checked_next_multiple_of(partial_alignment)
                        .ok_or(crate::gpu_pick::GpuPickError::AllocationFailed {
                            resource: "typed data pick partial offset",
                        })?;
                    let partial_entry = wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: partials,
                            offset: partial_offset,
                            size: NonZeroU64::new(bytes),
                        }),
                    };
                    partial_offset += bytes;
                    let output_entry = wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::Buffer(output),
                    };
                    match &item.label_params {
                        None => self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("figgy typed data pick partials query bg"),
                            layout: &self.partials_query_bgl,
                            entries: &[query_entry, pool_entry, partial_entry, output_entry],
                        }),
                        Some((label_params, cells)) => {
                            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("figgy value label pick query bg"),
                                layout: &self.value_label_query_bgl,
                                entries: &[
                                    query_entry,
                                    pool_entry,
                                    partial_entry,
                                    output_entry,
                                    wgpu::BindGroupEntry {
                                        binding: 4,
                                        resource: label_params.as_entire_binding(),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 7,
                                        resource: cells.as_entire_binding(),
                                    },
                                ],
                            })
                        }
                    }
                }
                _ => self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("figgy typed data pick query bg"),
                    layout: &self.query_bgl,
                    entries: &[
                        query_entry,
                        pool_entry,
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Buffer(output),
                        },
                    ],
                }),
            };
            params.push(item.params);
            prepared.push(PreparedSeries {
                query_bg,
                kind: item.kind,
                pipeline: item.pipeline,
                reduce: item.reduce,
                dispatch: item.dispatch,
            });
        }
        Ok(Arc::new(PreparedDataPick {
            transform: transform_buffer,
            transform_bg,
            series: prepared,
            identities: identities.into(),
            params,
            queries,
            query_stride,
            candidates,
            candidate_count,
            candidate_stride,
            _field_charges: field_charges,
            _scratch_charge: scratch_tally.into_charge(&self.ledger, GpuResourceKind::PickScratch),
        }))
    }

    /// The largest workgroup count of one dispatch dimension (#445).
    fn max_groups_per_dimension(&self, kernels: &DataPickKernels) -> u32 {
        #[cfg(test)]
        if let Some(limit) = kernels.max_groups_per_dimension {
            return limit;
        }
        #[cfg(not(test))]
        let _ = kernels;
        self.device.limits().max_compute_workgroups_per_dimension
    }

    /// Encode one typed data pick of `prepared` at `query`'s cursor and
    /// pick distance: its commands and readback, which run only when
    /// [`EncodedDataPick::submit`] submits them and are released unrun
    /// when it is dropped (#381: a layer other than the picked one runs its
    /// typed pick only when its series owns the ink at the cursor). The
    /// readback comes from `readbacks`, a ring of two reused across picks
    /// (#445). `prepared` must be held by nothing but the caller's cache:
    /// its queries are written here for this pick.
    pub(crate) fn encode(
        &self,
        prepared: &Arc<PreparedDataPick>,
        readbacks: &mut DataPickReadbacks,
        query: GpuDataPickQuery,
    ) -> Result<EncodedDataPick, crate::gpu_pick::GpuPickError> {
        let [cursor_x, cursor_y] = query.canvas_position_px;
        if !cursor_x.is_finite()
            || !cursor_y.is_finite()
            || !query.max_distance_px.is_finite()
            || query.max_distance_px < 0.0
        {
            return Ok(EncodedDataPick::none());
        }
        let [chart_x, chart_y, chart_width, chart_height] = query.chart_rect_px;
        if !chart_x.is_finite()
            || !chart_y.is_finite()
            || !chart_width.is_finite()
            || !chart_height.is_finite()
            || chart_width <= 0.0
            || chart_height <= 0.0
        {
            return Ok(EncodedDataPick::none());
        }
        if let Some([x, y, width, height]) = query.data_area_px {
            let x1 = x + width;
            let y1 = y + height;
            if !x.is_finite()
                || !y.is_finite()
                || !width.is_finite()
                || !height.is_finite()
                || cursor_x < x
                || cursor_x > x1
                || cursor_y < y
                || cursor_y > y1
            {
                return Ok(EncodedDataPick::none());
            }
        }
        if prepared.series.is_empty() {
            return Ok(EncodedDataPick::none());
        }

        let cursor_ndc_x = ((cursor_x - chart_x) / chart_width) * 2.0 - 1.0;
        let cursor_ndc_y = 1.0 - ((cursor_y - chart_y) / chart_height) * 2.0;
        let cursor = [
            cursor_ndc_x,
            cursor_ndc_y,
            (cursor_ndc_x + 1.0) * 0.5,
            (cursor_ndc_y + 1.0) * 0.5,
        ];
        self.queue
            .write_buffer(&prepared.transform, 0, bytemuck::bytes_of(&query.transform));
        for (index, template) in prepared.params.iter().enumerate() {
            let mut params = *template;
            params.cursor_ndc_t = cursor;
            params.limits[0] = query.max_distance_px;
            self.queue.write_buffer(
                &prepared.queries,
                index as u64 * prepared.query_stride,
                bytemuck::bytes_of(&params),
            );
        }

        let candidate_bytes = prepared.candidate_count * DATA_PICK_CANDIDATE_BYTES;
        let readback = readbacks.take(&self.device, &self.ledger, candidate_bytes)?;
        let commands = self.encode_data_pick(prepared, &readback.buffer);

        Ok(EncodedDataPick {
            encoded: Some(EncodedState {
                commands,
                readback,
                candidate_bytes,
                prepared: Arc::clone(prepared),
                device: self.device.clone(),
                queue: self.queue.clone(),
                health: self.health.clone(),
            }),
        })
    }

    /// [`Self::prepare`]'s per-series staging: each series' query
    /// template, kernels, dispatch and identity, in series order.
    fn stage_data_pick_series(
        &self,
        kernels: &mut DataPickKernels,
        series: Vec<GpuDataPickSeries>,
        max_groups: u32,
    ) -> Result<
        (Vec<StagedSeries>, Vec<DataPickIdentity>, Vec<FieldPickHold>),
        crate::gpu_pick::GpuPickError,
    > {
        let mut staged = Vec::new();
        staged.try_reserve_exact(series.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick staged series",
            }
        })?;
        let mut identities = Vec::new();
        identities.try_reserve_exact(series.len()).map_err(|_| {
            crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick identities",
            }
        })?;
        let mut field_charges = Vec::new();
        // The cursor and the pick distance are written by each pick.
        let cursor = [0.0; 4];
        let max_distance_px = 0.0;

        for item in series {
            // A value label pick binds the series' label uniform beside the
            // query in group 1.
            let mut label_params = None;
            let violin = matches!(item.geometry, GpuDataPickGeometry::Violin { .. });
            let extent = matches!(
                item.geometry,
                GpuDataPickGeometry::RangeBars { .. } | GpuDataPickGeometry::RangeLinks { .. }
            );
            // The rows a body kernel spreads over its workgroups (#445).
            let mut rows = 0u32;
            let (params, kind) = match item.geometry {
                GpuDataPickGeometry::Histogram {
                    edges,
                    values,
                    bin_count,
                    baseline,
                    gap_px,
                    width_ratio,
                    horizontal,
                    style_map,
                } => {
                    rows = bin_count;
                    (
                        DataPickQueryGpu {
                            cursor_ndc_t: cursor,
                            limits: [max_distance_px, gap_px, width_ratio, 0.0],
                            data: [0, item.paint_order, bin_count, u32::from(horizontal)],
                            bases: [lane_base(edges)?, lane_base(values)?, 0, 0],
                            baseline: [baseline[0], baseline[1], 0.0, 0.0],
                        },
                        PreparedKind::Histogram(
                            style_map.unwrap_or_else(|| self.empty_bar_style_map.clone()),
                        ),
                    )
                }
                GpuDataPickGeometry::Field {
                    field_bg,
                    charge,
                    has_fill,
                    has_contour,
                    contour_width_px,
                    nan_color_premul,
                    label,
                } => {
                    let mut flags = 0;
                    if has_fill {
                        flags |= DATA_PICK_FLAG_FILL | DATA_PICK_FLAG_PAINTED;
                    }
                    if has_contour {
                        flags |= DATA_PICK_FLAG_CONTOUR;
                    }
                    let label_gap = label.as_ref().map_or_else(
                        || self.empty_label_gap.clone(),
                        |label| label.label_gap_bind_group().clone(),
                    );
                    field_charges.push(FieldPickHold {
                        _charge: charge,
                        _label: label,
                    });
                    (
                        DataPickQueryGpu {
                            cursor_ndc_t: cursor,
                            limits: [max_distance_px, 0.0, contour_width_px, 0.0],
                            data: [flags, item.paint_order, 0, 0],
                            bases: [0; 4],
                            baseline: nan_color_premul,
                        },
                        PreparedKind::Field(
                            field_bg,
                            label_gap,
                            kernels.ensure(self, DataPickKernel::FieldContour)?.clone(),
                        ),
                    )
                }
                GpuDataPickGeometry::VectorGrid { vector_bg, charge } => {
                    field_charges.push(FieldPickHold {
                        _charge: charge,
                        _label: None,
                    });
                    (
                        DataPickQueryGpu {
                            cursor_ndc_t: cursor,
                            limits: [max_distance_px, 0.0, 0.0, 0.0],
                            data: [0, item.paint_order, 0, 0],
                            bases: [0; 4],
                            baseline: [0.0; 4],
                        },
                        PreparedKind::VectorGrid(vector_bg),
                    )
                }
                GpuDataPickGeometry::CategoryBars { data, rows: count } => {
                    rows = count;
                    field_charges.push(FieldPickHold {
                        _charge: Arc::clone(&data.charge),
                        _label: None,
                    });
                    (
                        category_axis_pick_query(cursor, max_distance_px, item.paint_order),
                        PreparedKind::CategoryBars(data),
                    )
                }
                GpuDataPickGeometry::BoxPlot { data, rows: count } => {
                    rows = count;
                    field_charges.push(FieldPickHold {
                        _charge: Arc::clone(&data.charge),
                        _label: None,
                    });
                    (
                        category_axis_pick_query(cursor, max_distance_px, item.paint_order),
                        PreparedKind::BoxPlot(data),
                    )
                }
                GpuDataPickGeometry::RangeBars { data, rows: count } => {
                    rows = count;
                    field_charges.push(FieldPickHold {
                        _charge: Arc::clone(&data.charge),
                        _label: None,
                    });
                    (
                        category_axis_pick_query(cursor, max_distance_px, item.paint_order),
                        PreparedKind::RangeBars(data),
                    )
                }
                GpuDataPickGeometry::RangeLinks { data, rows: count } => {
                    rows = count;
                    field_charges.push(FieldPickHold {
                        _charge: Arc::clone(&data.charge),
                        _label: None,
                    });
                    (
                        category_axis_pick_query(cursor, max_distance_px, item.paint_order),
                        PreparedKind::RangeLinks(data),
                    )
                }
                GpuDataPickGeometry::Violin { data, rows: count } => {
                    rows = count;
                    field_charges.push(FieldPickHold {
                        _charge: Arc::clone(&data.charge),
                        _label: None,
                    });
                    (
                        category_axis_pick_query(cursor, max_distance_px, item.paint_order),
                        PreparedKind::Violin(data),
                    )
                }
                GpuDataPickGeometry::Parallel { data } => {
                    rows = data.instances();
                    field_charges.push(FieldPickHold {
                        _charge: Arc::clone(&data.charge),
                        _label: None,
                    });
                    (
                        category_axis_pick_query(cursor, max_distance_px, item.paint_order),
                        PreparedKind::Parallel(data),
                    )
                }
                GpuDataPickGeometry::TriMesh {
                    field_bg,
                    charge,
                    triangles,
                    has_fill,
                    has_contour,
                    contour_width_px,
                    nan_color_premul,
                } => {
                    rows = triangles;
                    let mut flags = 0;
                    if has_fill {
                        flags |= DATA_PICK_FLAG_FILL;
                    }
                    if has_contour {
                        flags |= DATA_PICK_FLAG_CONTOUR;
                    }
                    field_charges.push(FieldPickHold {
                        _charge: charge,
                        _label: None,
                    });
                    (
                        DataPickQueryGpu {
                            cursor_ndc_t: cursor,
                            limits: [max_distance_px, 0.0, contour_width_px, 0.0],
                            data: [flags, item.paint_order, 0, 0],
                            bases: [0; 4],
                            baseline: nan_color_premul,
                        },
                        PreparedKind::TriMesh(field_bg),
                    )
                }
                GpuDataPickGeometry::ValueLabels { data, labels } => {
                    rows = labels.count;
                    field_charges.push(FieldPickHold {
                        _charge: Arc::clone(&data.charge),
                        _label: None,
                    });
                    field_charges.push(FieldPickHold {
                        _charge: Arc::clone(&labels.charge),
                        _label: None,
                    });
                    label_params = Some((labels.params.clone(), labels.cells.clone()));
                    (
                        category_axis_pick_query(cursor, max_distance_px, item.paint_order),
                        PreparedKind::ValueLabels(data),
                    )
                }
            };
            identities.push(DataPickIdentity {
                source_id: item.source_id,
                series_id: item.series_id,
                paint_order: item.paint_order,
                violin,
                extent,
            });
            let kernel = match &kind {
                PreparedKind::Histogram(_) => DataPickKernel::Histogram,
                PreparedKind::Field(..) => DataPickKernel::FieldCell,
                PreparedKind::VectorGrid(_) => DataPickKernel::VectorCell,
                PreparedKind::CategoryBars(_) => DataPickKernel::CategoryBar,
                PreparedKind::BoxPlot(_) => DataPickKernel::BoxPlot,
                PreparedKind::RangeBars(_) => DataPickKernel::RangeBar,
                PreparedKind::RangeLinks(_) => DataPickKernel::RangeLink,
                PreparedKind::Violin(_) => DataPickKernel::Violin,
                PreparedKind::ValueLabels(_) => DataPickKernel::ValueLabels,
                PreparedKind::Parallel(_) => DataPickKernel::Parallel,
                PreparedKind::TriMesh(_) => DataPickKernel::TriMesh,
            };
            let pipeline = kernels.ensure(self, kernel)?.clone();
            let (reduce, dispatch) = match kernel.reducer() {
                Some(reducer) => (
                    Some(kernels.ensure(self, reducer)?.clone()),
                    BodyDispatch::of(kernels, rows, max_groups, kernel.partials_per_group()),
                ),
                None => (None, BodyDispatch::SINGLE),
            };
            staged.push(StagedSeries {
                params,
                kind,
                pipeline,
                reduce,
                dispatch,
                label_params,
            });
        }
        Ok((staged, identities, field_charges))
    }

    /// [`Self::encode`]'s dispatches and the copies into the readback: each
    /// body series' workgroups, then its reduction (#445).
    fn encode_data_pick(
        &self,
        prepared: &PreparedDataPick,
        readback: &wgpu::Buffer,
    ) -> wgpu::CommandBuffer {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy typed data pick encoder"),
            });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy typed data pick pass"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &prepared.transform_bg, &[]);
            for item in &prepared.series {
                match &item.kind {
                    PreparedKind::Histogram(style_map) => {
                        pass.set_bind_group(2, style_map, &[]);
                    }
                    PreparedKind::Field(field_bg, label_gap, contour) => {
                        // The contour first; the cell dispatch below reads
                        // its candidate and keeps it when it is valid.
                        pass.set_pipeline(contour);
                        pass.set_bind_group(2, field_bg, &[]);
                        pass.set_bind_group(3, label_gap, &[]);
                        pass.set_bind_group(1, &item.query_bg, &[]);
                        pass.dispatch_workgroups(1, 1, 1);
                    }
                    PreparedKind::VectorGrid(vector_bg) => {
                        pass.set_bind_group(2, vector_bg, &[]);
                    }
                    PreparedKind::CategoryBars(data)
                    | PreparedKind::BoxPlot(data)
                    | PreparedKind::RangeBars(data)
                    | PreparedKind::RangeLinks(data)
                    | PreparedKind::Violin(data)
                    | PreparedKind::ValueLabels(data) => {
                        pass.set_bind_group(2, &data.style_map, &[]);
                        pass.set_bind_group(3, &data.bind_group, &[]);
                    }
                    PreparedKind::Parallel(data) => {
                        pass.set_bind_group(3, &data.bind_group, &[]);
                    }
                    PreparedKind::TriMesh(field_bg) => {
                        pass.set_bind_group(2, field_bg, &[]);
                    }
                }
                pass.set_pipeline(&item.pipeline);
                pass.set_bind_group(1, &item.query_bg, &[]);
                pass.dispatch_workgroups(item.dispatch.x, item.dispatch.y, 1);
                if let Some(reduce) = &item.reduce {
                    pass.set_pipeline(reduce);
                    pass.dispatch_workgroups(1, 1, 1);
                }
            }
        }
        for index in 0..prepared.candidate_count {
            encoder.copy_buffer_to_buffer(
                &prepared.candidates,
                index * prepared.candidate_stride,
                readback,
                index * DATA_PICK_CANDIDATE_BYTES,
                DATA_PICK_CANDIDATE_BYTES,
            );
        }
        encoder.finish()
    }
}

/// A typed data pick's prepared work (#445): every series' bind groups,
/// kernels, dispatch size and query template, and the scratch buffers they
/// write. The renderer keeps it while the chart's series and the display
/// it was prepared for are unchanged; each pick writes only its cursor and
/// pick distance into `queries`. A pick's encoded commands and its ticket
/// hold it, so a cache holding the only other reference knows no pick is
/// still using it (allocation identity, `ARCHITECTURE.md`).
pub(crate) struct PreparedDataPick {
    transform: wgpu::Buffer,
    transform_bg: wgpu::BindGroup,
    series: Vec<PreparedSeries>,
    identities: Arc<[DataPickIdentity]>,
    /// Each series' query with the cursor and the pick distance left to
    /// the pick.
    params: Vec<DataPickQueryGpu>,
    queries: wgpu::Buffer,
    query_stride: u64,
    candidates: wgpu::Buffer,
    candidate_count: u64,
    candidate_stride: u64,
    _field_charges: Vec<FieldPickHold>,
    _scratch_charge: GpuByteCharge,
}

/// A body kernel's dispatch (#445): one lane per row over
/// `ceil(rows / 64)` workgroups, split into two dimensions past the
/// per-dimension limit. Every workgroup writes its partials, so the
/// partials hold `x * y` of them.
#[derive(Clone, Copy)]
struct BodyDispatch {
    x: u32,
    y: u32,
    /// Partials per workgroup (a triangle mesh writes two).
    per_group: u64,
}

impl BodyDispatch {
    /// A kernel that writes its series' answer itself.
    const SINGLE: Self = Self {
        x: 1,
        y: 1,
        per_group: 0,
    };

    fn of(kernels: &DataPickKernels, rows: u32, max_groups: u32, per_group: u64) -> Self {
        #[cfg(test)]
        if kernels.single_workgroup {
            return Self {
                x: 1,
                y: 1,
                per_group,
            };
        }
        #[cfg(not(test))]
        let _ = kernels;
        let groups = rows.div_ceil(DATA_PICK_WORKGROUP_SIZE).max(1);
        let max_groups = max_groups.max(1);
        let x = groups.min(max_groups);
        Self {
            x,
            y: groups.div_ceil(x),
            per_group,
        }
    }
}

/// Lanes per workgroup of every typed body kernel (`DATA_PICK_WORKGROUP_SIZE`,
/// `PARALLEL_PICK_WORKGROUP_SIZE`, `MESH_PICK_WORKGROUP_SIZE`).
const DATA_PICK_WORKGROUP_SIZE: u32 = 64;

struct StagedSeries {
    params: DataPickQueryGpu,
    kind: PreparedKind,
    pipeline: wgpu::ComputePipeline,
    reduce: Option<wgpu::ComputePipeline>,
    dispatch: BodyDispatch,
    label_params: Option<(wgpu::Buffer, wgpu::Buffer)>,
}

impl StagedSeries {
    /// Bytes of its workgroup partials; `None` for a kernel without them.
    fn partial_bytes(&self) -> Result<Option<u64>, crate::gpu_pick::GpuPickError> {
        if self.reduce.is_none() {
            return Ok(None);
        }
        u64::from(self.dispatch.x)
            .checked_mul(u64::from(self.dispatch.y))
            .and_then(|groups| groups.checked_mul(self.dispatch.per_group))
            .and_then(|partials| partials.checked_mul(DATA_PICK_CANDIDATE_BYTES))
            .map(Some)
            .ok_or(crate::gpu_pick::GpuPickError::AllocationFailed {
                resource: "typed data pick partial bytes",
            })
    }
}

/// A readback buffer of the ring (#445). `idle` is set once the pick that
/// read it has unmapped it; a ticket dropped before then leaves it unset,
/// and the ring replaces the buffer instead of copying into one whose map
/// may still be pending.
pub(crate) struct ReadbackSlot {
    buffer: wgpu::Buffer,
    idle: std::sync::atomic::AtomicBool,
    _charge: GpuByteCharge,
}

/// The typed data pick's readback buffers (#445): a ring of two reused
/// across picks. A slot is reused when no ticket holds it (allocation
/// identity: the ring's reference is the only one) and its last pick
/// unmapped it; otherwise the ring makes a new buffer in its place.
#[derive(Default)]
pub(crate) struct DataPickReadbacks {
    ring: [Option<Arc<ReadbackSlot>>; 2],
    next: usize,
    /// Readback buffers created, for tests.
    #[cfg(test)]
    pub(crate) created: usize,
}

impl DataPickReadbacks {
    fn take(
        &mut self,
        device: &wgpu::Device,
        ledger: &Arc<GpuLedger>,
        bytes: u64,
    ) -> Result<Arc<ReadbackSlot>, crate::gpu_pick::GpuPickError> {
        use std::sync::atomic::Ordering;
        let reusable = |slot: &Option<Arc<ReadbackSlot>>| {
            slot.as_ref().is_some_and(|slot| {
                Arc::strong_count(slot) == 1
                    && slot.idle.load(Ordering::Acquire)
                    && slot.buffer.size() >= bytes
            })
        };
        let first = self.next % self.ring.len();
        let second = (first + 1) % self.ring.len();
        let at = if reusable(&self.ring[first]) {
            Some(first)
        } else if reusable(&self.ring[second]) {
            Some(second)
        } else {
            None
        };
        if let Some(at) = at
            && let Some(slot) = &self.ring[at]
        {
            slot.idle.store(false, Ordering::Release);
            self.next = at + 1;
            return Ok(Arc::clone(slot));
        }
        // A slot no ticket holds is replaced; with both held, the older
        // one's place is taken and its ticket keeps its own buffer.
        let at = if self.ring[first]
            .as_ref()
            .is_none_or(|slot| Arc::strong_count(slot) == 1)
        {
            first
        } else if self.ring[second]
            .as_ref()
            .is_none_or(|slot| Arc::strong_count(slot) == 1)
        {
            second
        } else {
            first
        };
        let size = self.ring[at]
            .as_ref()
            .map_or(bytes, |slot| slot.buffer.size().max(bytes));
        let tally = ChargeTally::new();
        let buffer = charged_buffer(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy typed data pick readback"),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        )
        .map_err(crate::gpu_pick::GpuPickError::from_figgy)?;
        #[cfg(test)]
        {
            self.created += 1;
        }
        let slot = Arc::new(ReadbackSlot {
            buffer,
            idle: std::sync::atomic::AtomicBool::new(false),
            _charge: tally.into_charge(ledger, GpuResourceKind::Readback),
        });
        self.ring[at] = Some(Arc::clone(&slot));
        self.next = at + 1;
        Ok(slot)
    }

    /// Drop the ring's buffers; a ticket still holding one keeps it.
    pub(crate) fn clear(&mut self) {
        self.ring = [None, None];
    }
}

/// A typed data pick encoded and not submitted (#381): its commands and
/// buffers. [`Self::submit`] runs it; dropped, it never runs.
pub(crate) struct EncodedDataPick {
    /// `None`: nothing to pick (no series, or a cursor or chart the pick
    /// cannot place).
    encoded: Option<EncodedState>,
}

struct EncodedState {
    commands: wgpu::CommandBuffer,
    readback: Arc<ReadbackSlot>,
    candidate_bytes: u64,
    prepared: Arc<PreparedDataPick>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    health: crate::renderer::DeviceHealth,
}

impl EncodedDataPick {
    fn none() -> Self {
        Self { encoded: None }
    }

    /// Submit the pick and request its readback.
    pub(crate) fn submit(mut self) -> DataPickTicket {
        let Some(encoded) = self.encoded.take() else {
            return DataPickTicket::ready_none();
        };
        encoded.queue.submit(std::iter::once(encoded.commands));
        DataPickTicket {
            state: DataPickTicketState::Pending {
                readback: GpuReadback::map(
                    &encoded.device,
                    &encoded.health,
                    &encoded.readback.buffer,
                    Some(encoded.candidate_bytes),
                ),
                identities: Arc::clone(&encoded.prepared.identities),
                gpu_reduced: false,
                slot: Some(encoded.readback),
                _prepared: Some(encoded.prepared),
                _field_charges: Vec::new(),
                _scratch_charge: None,
                _readback_charge: None,
            },
        }
    }
}

impl Drop for EncodedDataPick {
    /// A pick dropped unrun never mapped its readback: the ring may copy
    /// into it again (#445).
    fn drop(&mut self) {
        if let Some(encoded) = &self.encoded {
            encoded
                .readback
                .idle
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }
}

enum PreparedKind {
    Histogram(wgpu::BindGroup),
    /// The field and label gap groups, and the contour kernel
    /// dispatched before the cell kernel.
    Field(wgpu::BindGroup, wgpu::BindGroup, wgpu::ComputePipeline),
    VectorGrid(wgpu::BindGroup),
    CategoryBars(crate::data_render::CategoryBarData),
    BoxPlot(crate::data_render::CategoryBarData),
    RangeBars(crate::data_render::CategoryBarData),
    RangeLinks(crate::data_render::CategoryBarData),
    Violin(crate::data_render::CategoryBarData),
    ValueLabels(crate::data_render::CategoryBarData),
    Parallel(crate::data_render::ParallelData),
    TriMesh(wgpu::BindGroup),
}
struct PreparedSeries {
    query_bg: wgpu::BindGroup,
    kind: PreparedKind,
    /// The kernel the series dispatches; a field's cell kernel,
    /// after its contour kernel.
    pipeline: wgpu::ComputePipeline,
    /// The second pass that folds a body kernel's workgroup partials into
    /// the series' answer (#445).
    reduce: Option<wgpu::ComputePipeline>,
    dispatch: BodyDispatch,
}

fn lane_base(handle: ColumnHandle) -> Result<u32, crate::gpu_pick::GpuPickError> {
    u32::try_from(handle.offset / 4).map_err(|_| crate::gpu_pick::GpuPickError::DeviceLimit {
        resource: "typed data pick column lane base",
        requested: handle.offset / 4,
        limit: u64::from(u32::MAX),
    })
}

enum DataPickTicketState {
    ReadyNone,
    Pending {
        /// The candidates' `MAP_READ` buffer, mapped once the pick completes.
        readback: GpuReadback,
        identities: Arc<[DataPickIdentity]>,
        gpu_reduced: bool,
        /// A resident pick's readback slot of the ring (#445), marked idle
        /// once read and unmapped.
        slot: Option<Arc<ReadbackSlot>>,
        /// A resident pick's prepared work, held until it is read.
        _prepared: Option<Arc<PreparedDataPick>>,
        _field_charges: Vec<FieldPickHold>,
        _scratch_charge: Option<GpuByteCharge>,
        _readback_charge: Option<GpuByteCharge>,
    },
}

pub(crate) struct RankedPickedData {
    pub(crate) picked: PickedData,
}

pub(crate) struct DataPickTicket {
    state: DataPickTicketState,
}

impl DataPickTicket {
    fn ready_none() -> Self {
        Self {
            state: DataPickTicketState::ReadyNone,
        }
    }

    async fn wait(&mut self) -> Result<(), crate::gpu_pick::GpuPickError> {
        if let DataPickTicketState::Pending { readback, .. } = &mut self.state {
            readback.wait().await?;
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn wait_blocking(&mut self) -> Result<(), crate::gpu_pick::GpuPickError> {
        if let DataPickTicketState::Pending { readback, .. } = &mut self.state {
            readback.wait_blocking()?;
        }
        Ok(())
    }

    fn poll_ready(&mut self) -> Result<bool, crate::gpu_pick::GpuPickError> {
        match &mut self.state {
            DataPickTicketState::Pending { readback, .. } => Ok(readback.poll_ready()?),
            _ => Ok(true),
        }
    }

    /// After a wait: decode the candidates, and release the ticket's
    /// buffers.
    fn finish(&mut self) -> Result<Option<RankedPickedData>, crate::gpu_pick::GpuPickError> {
        let DataPickTicketState::Pending {
            mut readback,
            identities,
            gpu_reduced,
            slot,
            _prepared,
            _field_charges,
            _scratch_charge,
            _readback_charge,
        } = std::mem::replace(&mut self.state, DataPickTicketState::ReadyNone)
        else {
            return Ok(None);
        };

        // The unmap is the ticket's last GPU call, so the read checks the
        // device health after it (#158).
        let result = readback.finish_checked(
            |mapped| -> Result<Option<RankedPickedData>, crate::gpu_pick::GpuPickError> {
                let records: &[DataPickCandidateGpu] = bytemuck::cast_slice(mapped);
                if gpu_reduced {
                    let [candidate] = records else {
                        return Err(crate::gpu_pick::GpuPickError::InvalidGpuResult);
                    };
                    if candidate.valid == 0 {
                        return Ok(None);
                    }
                    if candidate.valid != 1
                        || !candidate.distance_px.is_finite()
                        || candidate.distance_px < 0.0
                    {
                        return Err(crate::gpu_pick::GpuPickError::InvalidGpuResult);
                    }
                    let identity = identities
                        .get(candidate.paint_order as usize)
                        .ok_or(crate::gpu_pick::GpuPickError::InvalidGpuResult)?;
                    return decode_candidate(identity, *candidate).map(Some);
                }
                if records.len() != identities.len() {
                    return Err(crate::gpu_pick::GpuPickError::InvalidGpuResult);
                }

                let mut best: Option<(usize, DataPickCandidateGpu)> = None;
                for (index, candidate) in records.iter().copied().enumerate() {
                    if candidate.valid == 0 {
                        continue;
                    }
                    if candidate.valid != 1
                        || !candidate.distance_px.is_finite()
                        || candidate.distance_px < 0.0
                        || identities[index].paint_order != candidate.paint_order
                    {
                        return Err(crate::gpu_pick::GpuPickError::InvalidGpuResult);
                    }
                    let replace = best.as_ref().is_none_or(|(_, incumbent)| {
                        candidate.distance_px < incumbent.distance_px
                            || (candidate.distance_px == incumbent.distance_px
                                && candidate.paint_order > incumbent.paint_order)
                    });
                    if replace {
                        best = Some((index, candidate));
                    }
                }
                best.map(|(index, candidate)| decode_candidate(&identities[index], candidate))
                    .transpose()
            },
        )?;
        // The read unmapped the slot: the ring may copy into it again.
        if let Some(slot) = slot {
            slot.idle.store(true, std::sync::atomic::Ordering::Release);
        }
        drop((_prepared, _field_charges, _scratch_charge, _readback_charge));
        result
    }
}

fn decode_candidate(
    identity: &DataPickIdentity,
    candidate: DataPickCandidateGpu,
) -> Result<RankedPickedData, crate::gpu_pick::GpuPickError> {
    let source_id = identity.source_id.clone();
    let series_id = identity.series_id.clone();
    let distance_px = candidate.distance_px;
    let picked = match candidate.kind {
        0 => PickedData::Point {
            source_id,
            series_id,
            point_index: candidate.index0 as usize,
            distance_px,
        },
        DATA_PICK_KIND_HISTOGRAM_BIN => PickedData::HistogramBin {
            source_id,
            series_id,
            bin_index: candidate.index0 as usize,
            distance_px,
            half: match (identity.violin, candidate.index1) {
                (true, crate::data_render::VIOLIN_SIDE_LOW) => {
                    Some(crate::data_config::ViolinSide::Low)
                }
                (true, crate::data_render::VIOLIN_SIDE_HIGH) => {
                    Some(crate::data_config::ViolinSide::High)
                }
                _ => None,
            },
            extent_px: identity
                .extent
                .then(|| f32::from_bits(candidate.index1))
                .filter(|length| length.is_finite() && *length >= 0.0),
        },
        DATA_PICK_KIND_MATRIX_CELL => PickedData::MatrixCell {
            source_id,
            series_id,
            x_index: candidate.index0 as usize,
            y_index: candidate.index1 as usize,
            distance_px,
        },
        DATA_PICK_KIND_CONTOUR_LEVEL => PickedData::ContourLevel {
            source_id,
            series_id,
            level_index: candidate.index0 as usize,
            x_index: candidate.index1 as usize,
            y_index: candidate.index2 as usize,
            distance_px,
        },
        DATA_PICK_KIND_MESH_TRIANGLE => PickedData::MeshTriangle {
            source_id,
            series_id,
            triangle_index: candidate.index0 as usize,
            vertex_index: candidate.index1 as usize,
            distance_px,
        },
        DATA_PICK_KIND_VALUE_LABEL => PickedData::ValueLabel {
            source_id,
            series_id,
            category_index: candidate.index0 as usize,
            part: None,
            distance_px,
        },
        _ => return Err(crate::gpu_pick::GpuPickError::InvalidGpuResult),
    };
    Ok(RankedPickedData { picked })
}

impl fmt::Debug for DataPickTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataPickTicket").finish_non_exhaustive()
    }
}

/// Owned async ticket for a typed data pick.
///
/// Point/line picking keeps its mature streaming gate engine; histogram/field
/// picking uses the render shaders' compute entries. Resident queries retain
/// their existing scalar merge at resolution. Streamed queries reduce every
/// chunk and primitive family on the GPU and map only the final scalar.
pub struct GpuDataPickTicket {
    state: GpuDataPickTicketState,
}

enum GpuDataPickTicketState {
    Resident {
        point: crate::gpu_pick::GpuPickTicket,
        data: DataPickTicket,
        /// The best hit of the layer's DataLess energy-level series, picked
        /// on the CPU over their SSOT when the ticket was made (#203).
        cpu: Option<RankedPickedData>,
        /// The layer `data` picks (the active layer, or the subplot at the
        /// cursor).
        layer: crate::LayerRef,
        /// The typed picks of the group's other layers with body or fill
        /// series, encoded and not run (#381): the one whose layer has the
        /// ink owner runs when the owner is a body series there.
        deferred: Vec<(crate::LayerRef, EncodedDataPick)>,
        /// The point pick's answer once it has been read.
        point_result: Option<crate::PickResult>,
        /// The owner layer's typed pick, once it runs.
        follow: Option<DataPickTicket>,
    },
    Stream(DataPickTicket),
    /// A 3D chart's 1×1 ID render (`Renderer::pick_surface`).
    Surface(SurfacePickTicket),
}

/// Who a surface ID names: ID `k` is entry `k − 1`.
pub(crate) struct SurfacePickIdentity {
    pub(crate) source_id: Option<String>,
    pub(crate) series_id: String,
    /// The grid's constituent columns run along y: the ID's `(c, r)` is
    /// `(y, x)`.
    pub(crate) columns_are_y: bool,
}

/// The kind word a 3D point pick writes (`fs_3d_pick`, #132): the ID's
/// third word is the point's index.
pub(crate) const SURFACE_PICK_KIND_POINT: u32 = 1;
/// The kind word a triangle mesh surface pick writes (`surface_input_mesh.wgsl::surface_pick_ids`, #210):
/// the ID's `(c, r)` are the triangle and its vertex nearest the fragment.
pub(crate) const SURFACE_PICK_KIND_MESH_TRIANGLE: u32 = 2;

/// A pending surface ID pick: the 16 bytes the 1×1 render wrote, read back.
pub(crate) struct SurfacePickTicket {
    /// The 16 bytes the 1×1 render wrote, mapped once the render completes.
    pub(crate) readback: GpuReadback,
    pub(crate) identities: Vec<SurfacePickIdentity>,
    pub(crate) _charges: Vec<crate::gpu_memory::SharedCharge>,
}

impl SurfacePickTicket {
    /// After a wait: decode the ID the 1×1 render wrote.
    fn finish(&mut self) -> Result<Option<PickedData>, crate::gpu_pick::GpuPickError> {
        let [word, b, c, r] = self.readback.finish(|mapped| {
            let words: &[u32] = bytemuck::cast_slice(mapped);
            [words[0], words[1], words[2], words[3]]
        })?;
        // The top bit marks a floor contour: `b` its level, `(c, r)` its
        // cell in x / y order. Otherwise `(c, r)` is a surface sample.
        let floor = word & 0x8000_0000 != 0;
        let id = word & 0x7fff_ffff;
        if id == 0 {
            return Ok(None);
        }
        let identity = self
            .identities
            .get(id as usize - 1)
            .ok_or(crate::gpu_pick::GpuPickError::InvalidGpuResult)?;
        if floor {
            return Ok(Some(PickedData::ContourLevel {
                source_id: identity.source_id.clone(),
                series_id: identity.series_id.clone(),
                level_index: b as usize,
                x_index: c as usize,
                y_index: r as usize,
                distance_px: 0.0,
            }));
        }
        // A 3D scatter or line series' point (#132): the marker or the
        // segment end under the cursor, at distance zero as the surface's
        // sample is.
        if b == SURFACE_PICK_KIND_POINT {
            return Ok(Some(PickedData::Point {
                source_id: identity.source_id.clone(),
                series_id: identity.series_id.clone(),
                point_index: c as usize,
                distance_px: 0.0,
            }));
        }
        // A triangle mesh drawn as a surface (#210): the triangle under the
        // cursor and its vertex nearest the fragment, at distance zero.
        if b == SURFACE_PICK_KIND_MESH_TRIANGLE {
            return Ok(Some(PickedData::MeshTriangle {
                source_id: identity.source_id.clone(),
                series_id: identity.series_id.clone(),
                triangle_index: c as usize,
                vertex_index: r as usize,
                distance_px: 0.0,
            }));
        }
        let (x, y) = if identity.columns_are_y {
            (r, c)
        } else {
            (c, r)
        };
        Ok(Some(PickedData::MatrixCell {
            source_id: identity.source_id.clone(),
            series_id: identity.series_id.clone(),
            x_index: x as usize,
            y_index: y as usize,
            distance_px: 0.0,
        }))
    }
}

impl GpuDataPickTicket {
    /// A registered chart's typed data pick (#381): its nearest-datum
    /// pick with the ink hit, and its typed hits.
    pub(crate) fn new(
        point: crate::gpu_pick::GpuPickTicket,
        data: DataPickTicket,
        cpu: Option<RankedPickedData>,
        layer: crate::LayerRef,
        deferred: Vec<(crate::LayerRef, EncodedDataPick)>,
    ) -> Self {
        Self {
            state: GpuDataPickTicketState::Resident {
                point,
                data,
                cpu,
                layer,
                deferred,
                point_result: None,
                follow: None,
            },
        }
    }

    /// Once the point pick and the typed pick are read: run the typed pick
    /// of the ink owner's layer when the owner is a body or fill series on
    /// a layer other than the one `data` picked (#381). Only that layer's
    /// kernels run; the other encoded picks are dropped unrun.
    fn follow_owner(&mut self) -> Result<(), crate::gpu_pick::GpuPickError> {
        let GpuDataPickTicketState::Resident {
            point,
            layer,
            deferred,
            point_result,
            follow,
            ..
        } = &mut self.state
        else {
            return Ok(());
        };
        if point_result.is_some() {
            return Ok(());
        }
        let result = point.finish_full()?;
        let owner_layer = result
            .owner
            .as_ref()
            .filter(|owner| result.hit.is_none() && owner.layer != *layer)
            .map(|owner| owner.layer);
        if let Some(owner_layer) = owner_layer
            && let Some(at) = deferred.iter().position(|(at, _)| *at == owner_layer)
        {
            let (_, encoded) = deferred.swap_remove(at);
            *follow = Some(encoded.submit());
        }
        deferred.clear();
        *point_result = Some(result);
        Ok(())
    }

    /// A ticket for a 3D chart's ID pick.
    pub(crate) fn surface(ticket: SurfacePickTicket) -> Self {
        Self {
            state: GpuDataPickTicketState::Surface(ticket),
        }
    }

    /// Wait for the pick: the bundle's poll thread completes the wait on
    /// native (see [`crate::RendererDevice::new`]). The selected element
    /// ([`DataPickResult::picked`]); see [`Self::resolve_full`] for every
    /// answer.
    pub async fn resolve(self) -> Result<Option<PickedData>, crate::gpu_pick::GpuPickError> {
        Ok(self.resolve_full().await?.picked)
    }

    /// [`Self::resolve`], polling the device on this thread until the pick
    /// completes. A streamed series' pick fails with
    /// [`GpuPickError::Streamed`](crate::gpu_pick::GpuPickError::Streamed).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve_blocking(self) -> Result<Option<PickedData>, crate::gpu_pick::GpuPickError> {
        Ok(self.resolve_full_blocking()?.picked)
    }

    /// The pick if it has completed, or the ticket back, without blocking.
    /// A streamed series' pick fails with
    /// [`GpuPickError::Streamed`](crate::gpu_pick::GpuPickError::Streamed).
    pub fn try_resolve(
        self,
    ) -> Result<crate::TryResolve<Option<PickedData>, Self>, crate::gpu_pick::GpuPickError> {
        Ok(match self.try_resolve_full()? {
            crate::TryResolve::Ready(result) => crate::TryResolve::Ready(result.picked),
            crate::TryResolve::Pending(ticket) => crate::TryResolve::Pending(ticket),
        })
    }

    /// Wait for the pick: the selected element, the nearest-datum pick and
    /// the typed hit, each its own answer (#381).
    pub async fn resolve_full(mut self) -> Result<DataPickResult, crate::gpu_pick::GpuPickError> {
        match &mut self.state {
            GpuDataPickTicketState::Resident { point, data, .. } => {
                point.wait().await?;
                data.wait().await?;
                self.follow_owner()?;
                if let GpuDataPickTicketState::Resident {
                    follow: Some(follow),
                    ..
                } = &mut self.state
                {
                    follow.wait().await?;
                }
            }
            GpuDataPickTicketState::Stream(ticket) => ticket.wait().await?,
            GpuDataPickTicketState::Surface(ticket) => ticket.readback.wait().await?,
        }
        self.finish()
    }

    /// [`Self::resolve_full`], polling the device on this thread until the
    /// pick completes. A streamed series' pick fails with
    /// [`GpuPickError::Streamed`](crate::gpu_pick::GpuPickError::Streamed).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve_full_blocking(
        mut self,
    ) -> Result<DataPickResult, crate::gpu_pick::GpuPickError> {
        match &mut self.state {
            GpuDataPickTicketState::Resident { point, data, .. } => {
                point.wait_blocking()?;
                data.wait_blocking()?;
                self.follow_owner()?;
                if let GpuDataPickTicketState::Resident {
                    follow: Some(follow),
                    ..
                } = &mut self.state
                {
                    follow.wait_blocking()?;
                }
            }
            GpuDataPickTicketState::Stream(_) => {
                return Err(crate::gpu_pick::GpuPickError::Streamed);
            }
            GpuDataPickTicketState::Surface(ticket) => ticket.readback.wait_blocking()?,
        }
        self.finish()
    }

    /// [`Self::resolve_full`] if the pick has completed, or the ticket back,
    /// without blocking. A streamed series' pick fails with
    /// [`GpuPickError::Streamed`](crate::gpu_pick::GpuPickError::Streamed).
    pub fn try_resolve_full(
        mut self,
    ) -> Result<crate::TryResolve<DataPickResult, Self>, crate::gpu_pick::GpuPickError> {
        let ready = match &mut self.state {
            GpuDataPickTicketState::Resident {
                point,
                data,
                point_result,
                ..
            } => {
                // Both, so the first call hands both waits to the poll thread.
                let point_ready = point_result.is_some() || point.poll_ready()?;
                let ready = data.poll_ready()? && point_ready;
                if ready {
                    self.follow_owner()?;
                    match &mut self.state {
                        GpuDataPickTicketState::Resident {
                            follow: Some(follow),
                            ..
                        } => follow.poll_ready()?,
                        _ => true,
                    }
                } else {
                    false
                }
            }
            GpuDataPickTicketState::Stream(_) => {
                return Err(crate::gpu_pick::GpuPickError::Streamed);
            }
            GpuDataPickTicketState::Surface(ticket) => ticket.readback.poll_ready()?,
        };
        if !ready {
            return Ok(crate::TryResolve::Pending(self));
        }
        self.finish().map(crate::TryResolve::Ready)
    }

    /// After a wait: decode the pick. A resident layer answers its
    /// nearest-datum pick and its typed hit separately (#381), and the
    /// series that owns the drawn ink nearest to the cursor selects: its
    /// nearest datum for a series with point data, the typed hit for a body
    /// or fill series. With no ink in reach the typed hit answers alone.
    fn finish(&mut self) -> Result<DataPickResult, crate::gpu_pick::GpuPickError> {
        let alone = |picked: Option<PickedData>| DataPickResult {
            picked: picked.clone(),
            point: crate::PickResult::default(),
            data: picked,
        };
        if matches!(self.state, GpuDataPickTicketState::Resident { .. }) {
            self.follow_owner()?;
        }
        let (point, data, cpu, follow) = match &mut self.state {
            GpuDataPickTicketState::Surface(ticket) => return ticket.finish().map(alone),
            GpuDataPickTicketState::Stream(ticket) => {
                return ticket
                    .finish()
                    .map(|result| alone(result.map(|hit| hit.picked)));
            }
            GpuDataPickTicketState::Resident {
                point_result,
                data,
                cpu,
                follow,
                ..
            } => (
                point_result.take().unwrap_or_default(),
                data,
                cpu.take(),
                follow.take(),
            ),
        };
        let gpu = data.finish()?;
        // An energy-level hit (#203) is composited over the panel's GPU
        // data, so it wins a tie with any of it.
        let data = match (gpu, cpu) {
            (None, None) => None,
            (Some(hit), None) | (None, Some(hit)) => Some(hit.picked),
            (Some(gpu), Some(cpu)) => {
                if cpu.picked.distance_px() <= gpu.picked.distance_px() {
                    Some(cpu.picked)
                } else {
                    Some(gpu.picked)
                }
            }
        };
        // The series that owns the ink nearest to the cursor decides: a
        // point-datum owner selects its nearest datum, a body or fill owner
        // its typed hit. Without an owner the typed hit answers as before:
        // value labels and DataLess series are not in the ink masks.
        // A body owner on another layer than `data`'s selects the typed
        // hit of its own layer, picked for it alone.
        let owned = match follow {
            Some(mut follow) => follow.finish()?.map(|hit| hit.picked),
            None => None,
        };
        let picked = match (&point.hit, &point.owner) {
            (Some(hit), _) => Some(PickedData::from(hit.clone())),
            (None, Some(owner)) => owned
                .filter(|hit| hit.series_id() == owner.series_id)
                .or_else(|| {
                    data.clone()
                        .filter(|data| data.series_id() == owner.series_id)
                })
                .or_else(|| data.clone()),
            (None, None) => data.clone(),
        };
        Ok(DataPickResult {
            picked,
            point,
            data,
        })
    }
}

/// A resolved typed data pick (#381): the selected element and the two
/// answers it is chosen from, each complete on its own.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DataPickResult {
    /// The element the pick selects: the nearest datum (as
    /// [`PickedData::Point`]) when the series that owns the ink at the
    /// cursor (`point.owner`) has point data, the typed hit `data`
    /// otherwise.
    pub picked: Option<PickedData>,
    /// The nearest-datum pick ([`crate::Renderer::pick_chart`]): the hit
    /// when drawn ink is within reach, and every series' nearest datum.
    pub point: crate::PickResult,
    /// The typed hit under the cursor: a bin, body, bar, cell, contour
    /// level, value label or DataLess element, as the typed kernels and the
    /// CPU pick of DataLess series answer it.
    pub data: Option<PickedData>,
}

impl fmt::Debug for GpuDataPickTicket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuDataPickTicket").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    use crate::data_render;
    use wgpu::util::DeviceExt;

    fn bundle(device: wgpu::Device, queue: wgpu::Queue) -> Arc<DataPickPipelineBundle> {
        let bar = crate::gpu_compile::shader_module(
            &device,
            wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(crate::data_render::BAR_SHADER.source.into()),
            },
        );
        let field = crate::gpu_compile::shader_module(
            &device,
            wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(crate::data_render::FIELD_SHADER.source.into()),
            },
        );
        let vector_grid = data_render::vector::shader_module(
            &device,
            data_render::vector::VectorInput::Grid,
            crate::shader_assembly::ShaderSet::Full,
        );
        let transform = data_render::create_scatter_transform_bind_group_layout(&device);
        let field_bgl = data_render::create_field_data_bind_group_layout(&device);
        let style = data_render::create_per_point_style_map_bind_group_layout(&device);
        let vector_bgl = data_render::vector::create_vector_data_bind_group_layout(&device);
        let category_bgl = data_render::create_category_bar_data_bind_group_layout(&device);
        let parallel_bgl = data_render::create_parallel_data_bind_group_layout(&device);
        let line = crate::gpu_compile::shader_module(
            &device,
            wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(crate::data_render::LINE_SHADER.source.into()),
            },
        );
        DataPickPipelineBundle::new_observed(
            device.clone(),
            queue,
            Arc::new(GpuLedger::new()),
            crate::renderer::DeviceHealth::polled_for_test(&device),
            None,
            DataPickShaders {
                bar: &bar,
                field: &field,
                vector_grid: &vector_grid,
                line: &line,
                set: crate::shader_assembly::ShaderSet::Full,
            },
            &transform,
            &field_bgl,
            &style,
            &vector_bgl,
            &category_bgl,
            &parallel_bgl,
            &mut |_| {},
        )
        .unwrap()
    }

    fn config() -> crate::config::Config {
        let mut config = crate::default::default_config();
        config.chart_area = crate::layout::ChartArea(crate::layout::Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        });
        config.bottom_x.min = 1_000_000_000_000.0;
        config.bottom_x.max = 1_000_000_000_004.0;
        config.bottom_x.scale = crate::config::AxisScale::Linear;
        config.left_y.min = 0.0;
        config.left_y.max = 10.0;
        config.chart_title.top_margin = 0.0;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.out_margin = 0.0;
            axis.major_tick_length = 0.0;
        }
        config
    }

    #[test]
    fn stream_typed_reducer_keeps_point_and_paint_order_ties_on_gpu() {
        let Some((device, queue)) = data_render::shared_device() else {
            return;
        };
        let bundle = bundle(device.clone(), queue.clone());
        let mut kernels = DataPickKernels::default();
        let query = GpuDataPickQuery {
            transform: data_render::scatter_transform_from_config(&config()),
            chart_rect_px: [0.0, 0.0, 100.0, 100.0],
            data_area_px: None,
            canvas_position_px: [50.0; 2],
            max_distance_px: 5.0,
        };
        for point_order in [0u32, 1, 2] {
            for point_first in [false, true] {
                let mut stream = bundle
                    .begin_stream(
                        &mut kernels,
                        query,
                        (0..3)
                            .map(|order| (None, format!("series-{order}")))
                            .collect(),
                    )
                    .unwrap();
                let point = [
                    1u32,
                    point_order,
                    42,
                    1,
                    41,
                    0.0f32.to_bits(),
                    0.0f32.to_bits(),
                    0,
                ];
                let data = DataPickCandidateGpu {
                    valid: 1,
                    paint_order: 1,
                    kind: 1,
                    index0: 17,
                    index1: 0,
                    index2: 0,
                    distance_px: 0.0,
                    primitive_order: 17,
                };
                let point = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&point),
                    usage: wgpu::BufferUsages::STORAGE,
                });
                let data = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::bytes_of(&data),
                    usage: wgpu::BufferUsages::STORAGE,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                for is_point in [point_first, !point_first] {
                    stream.accumulate(
                        &mut encoder,
                        if is_point { &point } else { &data },
                        is_point,
                    );
                }
                queue.submit([encoder.finish()]);
                let actual = crate::test_gpu::block_on(stream.finish().unwrap().resolve())
                    .unwrap()
                    .unwrap();
                if point_order >= 1 {
                    assert!(matches!(
                        actual,
                        PickedData::Point {
                            point_index: 42,
                            ..
                        }
                    ));
                } else {
                    assert!(matches!(
                        actual,
                        PickedData::HistogramBin { bin_index: 17, .. }
                    ));
                }
                assert_eq!(actual.series_id(), format!("series-{}", point_order.max(1)));
            }
        }
    }
}
