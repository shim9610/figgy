//! Interactive exact-streaming capacity demo.
//!
//! The selected logical data size is generated lazily through `ColumnSource`;
//! the demo never allocates a dataset-sized CPU buffer. Every point is drawn.
//!
//! Run:
//! `cargo run --release -p renderer --example streaming_capacity_demo --features egui_demo`
//! Add `-- --benchmark` to run the default 1 GiB virtual source once and print measured timings.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui_wgpu::{self, CallbackTrait};
use eframe::wgpu;
use renderer::layout::{ChartArea, Rect};
use renderer::{
    Chart, ChartId, ChartView, ColumnPairWriter, ColumnRangeWriteError, ColumnSource,
    ColumnUploadStats, DataRenderType, DataScatterStyleConfig, FiggyError, HiLoColumnSource,
    PreparedFrame, RegisteredChartDrawItem, Renderer, RendererDevice, ScatterShape, SeriesConfig,
    StreamBounds, StreamColumn, StreamColumnSource, StreamEncoding, StreamReplay,
    StreamSourceBinding, StreamStatistics, StreamingChartOptions, StreamingLimits,
    StreamingProgress,
};

const POOL_CAPACITY: u64 = 1024 * 1024;
const STEP_BUDGET: Duration = Duration::from_millis(8);
const CHUNK_CHOICES: [u64; 4] = [65_536, 262_144, 1_048_576, 4_194_304];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Precision {
    F32,
    F64HiLo,
}

impl Precision {
    fn label(self) -> &'static str {
        match self {
            Self::F32 => "f32 (4 bytes/value)",
            Self::F64HiLo => "f64 → hi/lo (8 bytes/value)",
        }
    }

    fn bytes_per_point(self) -> u64 {
        match self {
            Self::F32 => 8,
            Self::F64HiLo => 16,
        }
    }

    fn encoding(self) -> StreamEncoding {
        match self {
            Self::F32 => StreamEncoding::ScalarF32,
            Self::F64HiLo => StreamEncoding::HiLoF32,
        }
    }
}

#[derive(Clone, Copy)]
struct RunRequest {
    gib: f64,
    chunk_points: u64,
    precision: Precision,
}

impl RunRequest {
    fn point_count(self) -> u64 {
        let bytes = (self.gib * 1024.0 * 1024.0 * 1024.0).round().max(1.0) as u64;
        (bytes / self.precision.bytes_per_point()).clamp(2, u32::MAX as u64)
    }

    fn actual_bytes(self) -> u64 {
        self.point_count() * self.precision.bytes_per_point()
    }
}

struct VirtualColumn {
    len: usize,
    axis: VirtualAxis,
}

#[derive(Clone, Copy)]
enum VirtualAxis {
    X,
    Y,
}

impl VirtualColumn {
    #[inline]
    fn precise(&self, index: usize) -> f64 {
        let denominator = self.len.saturating_sub(1).max(1) as f64;
        match self.axis {
            VirtualAxis::X => index as f64 / denominator,
            VirtualAxis::Y => {
                // Twelve continuous triangle waves make submitted chunks visibly
                // accumulate from left to right without distributing points over
                // the entire plot area.
                let phase = ((index as u64 * 12) % self.len.saturating_sub(1).max(1) as u64)
                    as f64
                    / denominator;
                let triangle = 1.0 - (phase * 2.0 - 1.0).abs();
                0.1 + triangle * 0.8
            }
        }
    }

    #[inline]
    fn coarse(&self, index: usize) -> f32 {
        self.precise(index) as f32
    }
}

fn checked_range(
    total: usize,
    start: u64,
    len: usize,
) -> Result<std::ops::Range<usize>, ColumnRangeWriteError> {
    let start = usize::try_from(start).map_err(|_| ColumnRangeWriteError::InvalidRange)?;
    let end = start
        .checked_add(len)
        .filter(|end| *end <= total)
        .ok_or(ColumnRangeWriteError::InvalidRange)?;
    Ok(start..end)
}

impl ColumnSource for VirtualColumn {
    fn len(&self) -> usize { self.len }
    fn min(&self) -> f64 { 0.0 }
    fn max(&self) -> f64 { 1.0 }

    fn write_f32_le_into(&self, _dst: &mut [u8]) {
        panic!("streaming demo must not request a full-column buffer")
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        _dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        panic!("streaming demo must not request a full-column writer")
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut dst: ColumnPairWriter<'_>,
    ) -> Result<Option<StreamBounds>, ColumnRangeWriteError> {
        let range = checked_range(self.len, start, dst.len())?;
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        let mut min_positive = f32::INFINITY;
        for (local, index) in range.enumerate() {
            let value = self.coarse(index);
            dst.write_pair(local, value, 0.0);
            min = min.min(value);
            max = max.max(value);
            if value > 0.0 {
                min_positive = min_positive.min(value);
            }
        }
        Ok((dst.len() != 0).then_some(StreamBounds {
            min: f64::from(min),
            max: f64::from(max),
            min_positive: min_positive.is_finite().then(|| f64::from(min_positive)),
        }))
    }
}

impl HiLoColumnSource for VirtualColumn {
    fn len(&self) -> usize { self.len }
    fn min(&self) -> f64 { 0.0 }
    fn max(&self) -> f64 { 1.0 }

    fn write_f32_pair_le_into(&self, _dst: &mut [u8]) {
        panic!("streaming demo must not request a full-column buffer")
    }

    fn write_f32_pair_le_into_with_stats(
        &self,
        _dst: ColumnPairWriter<'_>,
    ) -> ColumnUploadStats {
        panic!("streaming demo must not request a full-column writer")
    }

    fn write_f32_pair_range_into_with_stats(
        &self,
        start: u64,
        mut dst: ColumnPairWriter<'_>,
    ) -> Result<Option<StreamBounds>, ColumnRangeWriteError> {
        let range = checked_range(self.len, start, dst.len())?;
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut min_positive = f64::INFINITY;
        for (local, index) in range.enumerate() {
            let (hi, lo) = renderer::data::split_f64_to_f32_pair(self.precise(index));
            dst.write_pair(local, hi, lo);
            let value = hi as f64 + lo as f64;
            min = min.min(value);
            max = max.max(value);
            if value > 0.0 {
                min_positive = min_positive.min(value);
            }
        }
        Ok((dst.len() != 0).then_some(StreamBounds {
            min,
            max,
            min_positive: min_positive.is_finite().then_some(min_positive),
        }))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Running,
    Draining,
    Done,
    Cancelled,
    Failed,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Running => "Rendering chunks",
            Self::Draining => "Waiting for GPU completion",
            Self::Done => "Complete",
            Self::Cancelled => "Cancelled",
            Self::Failed => "Failed",
        }
    }

    fn active(self) -> bool {
        matches!(self, Self::Running | Self::Draining)
    }
}

#[derive(Clone)]
struct Snapshot {
    phase: Phase,
    points: u64,
    bytes: u64,
    submitted: u64,
    first_submit_ms: Option<f64>,
    all_submitted_ms: Option<f64>,
    gpu_complete_ms: Option<f64>,
    submitted_chunks: u64,
    peak_in_flight_chunks: usize,
    peak_reserved_gpu_bytes: u64,
    renderer_accounted_gpu_peak_bytes: u64,
    mean_frame_interval_ms: Option<f64>,
    p95_frame_interval_ms: Option<f64>,
    max_frame_interval_ms: Option<f64>,
    error: Option<String>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            points: 0,
            bytes: 0,
            submitted: 0,
            first_submit_ms: None,
            all_submitted_ms: None,
            gpu_complete_ms: None,
            submitted_chunks: 0,
            peak_in_flight_chunks: 0,
            peak_reserved_gpu_bytes: 0,
            renderer_accounted_gpu_peak_bytes: 0,
            mean_frame_interval_ms: None,
            p95_frame_interval_ms: None,
            max_frame_interval_ms: None,
            error: None,
        }
    }
}

struct DemoState {
    renderer: Renderer,
    chart: Chart,
    chart_id: ChartId,
    view: ChartView,
    x: VirtualColumn,
    y: VirtualColumn,
    revision: u64,
    pending_start: Option<RunRequest>,
    last_request: Option<RunRequest>,
    started: Option<Instant>,
    first_paint_encoded_us: AtomicU64,
    last_prepare_at: Option<Instant>,
    frame_intervals_ms: Vec<f64>,
    surface_size: (u32, u32),
    snapshot: Snapshot,
    prepared: Option<PreparedFrame>,
}

impl DemoState {
    fn metadata(id: &str, len: u64, revision: u64, encoding: StreamEncoding) -> StreamColumn {
        StreamColumn {
            id: id.into(),
            len,
            encoding,
            replay: StreamReplay::RandomAccess,
            revision,
            statistics: StreamStatistics::Unknown,
        }
    }

    fn cancel(&mut self, phase: Phase) {
        if self.snapshot.phase.active() || self.snapshot.phase == Phase::Done {
            let _ = self.renderer.cancel_streaming_chart(self.chart_id);
        }
        self.pending_start = None;
        self.started = None;
        self.snapshot.phase = phase;
        self.prepared = None;
    }

    fn fail(&mut self, error: FiggyError) {
        self.snapshot.error = Some(error.to_string());
        self.cancel(Phase::Failed);
    }

    fn start(&mut self, request: RunRequest, surface_size: (u32, u32)) -> renderer::Result<()> {
        self.cancel(Phase::Idle);
        self.revision = self.revision.checked_add(1).ok_or(FiggyError::CounterExhausted {
            counter: "streaming demo revision",
        })?;
        let points = request.point_count();
        self.x.len = points as usize;
        self.y.len = points as usize;
        self.renderer.replace_streamed_columns(vec![
            Self::metadata("stream-x", points, self.revision, request.precision.encoding()),
            Self::metadata("stream-y", points, self.revision, request.precision.encoding()),
        ])?;
        self.renderer.begin_streaming_chart(
            self.chart_id,
            &self.view,
            StreamingChartOptions {
                size: surface_size,
                clear_color: renderer::Color::new(0.035, 0.045, 0.065, 1.0),
                max_primitives_per_chunk: request.chunk_points,
            },
        )?;
        self.surface_size = surface_size;
        self.last_request = Some(request);
        self.started = Some(Instant::now());
        self.first_paint_encoded_us.store(0, Ordering::Relaxed);
        self.last_prepare_at = None;
        self.frame_intervals_ms.clear();
        self.snapshot = Snapshot {
            phase: Phase::Running,
            points,
            bytes: request.actual_bytes(),
            ..Snapshot::default()
        };
        Ok(())
    }

    fn step_once(&mut self) -> renderer::Result<StreamingProgress> {
        let revision = self.revision;
        let progress = match self.last_request.expect("active run has request").precision {
            Precision::F32 => {
                let sources = [
                    StreamSourceBinding {
                        id: "stream-x",
                        revision,
                        source: StreamColumnSource::Scalar(&self.x),
                    },
                    StreamSourceBinding {
                        id: "stream-y",
                        revision,
                        source: StreamColumnSource::Scalar(&self.y),
                    },
                ];
                self.renderer.stream_chart_step(self.chart_id, &self.view, &sources)?
            }
            Precision::F64HiLo => {
                let sources = [
                    StreamSourceBinding {
                        id: "stream-x",
                        revision,
                        source: StreamColumnSource::HiLo(&self.x),
                    },
                    StreamSourceBinding {
                        id: "stream-y",
                        revision,
                        source: StreamColumnSource::HiLo(&self.y),
                    },
                ];
                self.renderer.stream_chart_step(self.chart_id, &self.view, &sources)?
            }
        };
        Ok(progress)
    }

    fn advance(&mut self) {
        let Some(started) = self.started else { return; };
        let deadline = Instant::now() + STEP_BUDGET;
        if self.snapshot.phase == Phase::Running {
            loop {
                let result = self.step_once();
                self.sample_gpu_usage();
                match result {
                    Ok(StreamingProgress::Submitted {
                        submitted_primitives,
                        ..
                    }) => {
                        self.snapshot.submitted = submitted_primitives;
                        self.snapshot.submitted_chunks += 1;
                        self.snapshot.first_submit_ms.get_or_insert_with(|| {
                            started.elapsed().as_secs_f64() * 1000.0
                        });
                    }
                    Ok(StreamingProgress::Backpressure {
                        submitted_primitives,
                        ..
                    }) => {
                        self.snapshot.submitted = submitted_primitives;
                        break;
                    }
                    Ok(StreamingProgress::AllSubmitted { total_primitives }) => {
                        self.snapshot.submitted = total_primitives;
                        self.snapshot.all_submitted_ms =
                            Some(started.elapsed().as_secs_f64() * 1000.0);
                        self.snapshot.phase = Phase::Draining;
                        break;
                    }
                    Err(error) => {
                        self.fail(error);
                        return;
                    }
                }
                if Instant::now() >= deadline {
                    break;
                }
            }
        }
        if self.snapshot.phase == Phase::Draining {
            let result = self.step_once();
            self.sample_gpu_usage();
            if let Err(error) = result {
                self.fail(error);
                return;
            }
            if self.renderer.streaming_usage().in_flight_chunks == 0 {
                self.snapshot.gpu_complete_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
                self.snapshot.phase = Phase::Done;
                self.finish_frame_intervals();
            }
        }
    }

    fn sample_gpu_usage(&mut self) {
        let usage = self.renderer.streaming_usage();
        self.snapshot.peak_in_flight_chunks = self
            .snapshot.peak_in_flight_chunks
            .max(usage.in_flight_chunks);
        self.snapshot.peak_reserved_gpu_bytes = self
            .snapshot.peak_reserved_gpu_bytes
            .max(usage.reserved_gpu_bytes);
        self.snapshot.renderer_accounted_gpu_peak_bytes = self
            .renderer
            .gpu_memory_usage()
            .peak_bytes();
    }

    fn record_prepare_interval(&mut self) {
        if !self.snapshot.phase.active() {
            return;
        }
        let now = Instant::now();
        if let Some(previous) = self.last_prepare_at.replace(now) {
            self.frame_intervals_ms
                .push(now.duration_since(previous).as_secs_f64() * 1000.0);
        }
    }

    fn finish_frame_intervals(&mut self) {
        if self.frame_intervals_ms.is_empty() {
            return;
        }
        self.frame_intervals_ms.sort_by(f64::total_cmp);
        let count = self.frame_intervals_ms.len();
        self.snapshot.mean_frame_interval_ms = Some(
            self.frame_intervals_ms.iter().sum::<f64>() / count as f64,
        );
        self.snapshot.p95_frame_interval_ms =
            Some(self.frame_intervals_ms[((count as f64 * 0.95).ceil() as usize).saturating_sub(1)]);
        self.snapshot.max_frame_interval_ms = self.frame_intervals_ms.last().copied();
    }

    fn resize(&mut self, panel_rect: Rect, surface_size: (u32, u32)) -> renderer::Result<()> {
        let changed = self.view.panel_rect() != panel_rect || self.surface_size != surface_size;
        if !changed {
            return Ok(());
        }
        let restart = self
            .pending_start
            .take()
            .or_else(|| self.snapshot.phase.active().then_some(self.last_request).flatten());
        self.cancel(Phase::Cancelled);
        self.chart.config_mut().chart_area = ChartArea(panel_rect);
        self.renderer
            .set_chart_config(self.chart_id, self.chart.config().clone())?;
        self.renderer.refresh_axis(&mut self.view, &self.chart, panel_rect)?;
        self.surface_size = surface_size;
        if let Some(request) = restart {
            self.pending_start = Some(request);
        }
        Ok(())
    }

    fn prepare_frame(&mut self) {
        self.prepared = match self.renderer.prepare_registered(&[RegisteredChartDrawItem {
            chart_id: self.chart_id,
            view: &self.view,
        }]) {
            Ok(prepared) => Some(prepared),
            Err(error) => {
                self.snapshot.error = Some(error.to_string());
                None
            }
        };
        self.sample_gpu_usage();
    }

    fn shutdown(&mut self) {
        let _ = self.renderer.cancel_streaming_chart(self.chart_id);
        self.renderer.wait_idle();
        self.prepared = None;
    }
}

struct DemoCallback {
    panel_rect: Rect,
}

impl CallbackTrait for DemoCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(state) = resources.get_mut::<DemoState>() else {
            return Vec::new();
        };
        let surface_size = (screen.size_in_pixels[0], screen.size_in_pixels[1]);
        if let Err(error) = state.resize(self.panel_rect, surface_size) {
            state.fail(error);
            return Vec::new();
        }
        if let Some(request) = state.pending_start.take()
            && let Err(error) = state.start(request, surface_size)
        {
            state.fail(error);
            return Vec::new();
        }
        state.record_prepare_interval();
        state.advance();
        if state.snapshot.phase != Phase::Idle && state.snapshot.phase != Phase::Cancelled {
            state.prepare_frame();
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(state) = resources.get::<DemoState>() else { return; };
        let Some(prepared) = state.prepared.as_ref() else { return; };
        let result = state.renderer.paint_prepared(
            pass,
            (info.screen_size_px[0], info.screen_size_px[1]),
            prepared,
        );
        if result.is_ok() && state.snapshot.submitted > 0 {
            if let Some(started) = state.started {
                let micros = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
                let _ = state.first_paint_encoded_us.compare_exchange(
                    0,
                    micros.max(1),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                );
            }
        }
    }
}

struct DemoApp {
    initialized: bool,
    data_gib: f64,
    precision: Precision,
    chunk_index: usize,
    render_state: Option<egui_wgpu::RenderState>,
    init_error: Option<String>,
    benchmark: bool,
    benchmark_reported: bool,
}

impl Default for DemoApp {
    fn default() -> Self {
        Self {
            initialized: false,
            data_gib: 1.0,
            precision: Precision::F64HiLo,
            chunk_index: 2,
            render_state: None,
            init_error: None,
            benchmark: false,
            benchmark_reported: false,
        }
    }
}

impl DemoApp {
    fn initialize(&mut self, render_state: &egui_wgpu::RenderState) -> renderer::Result<()> {
        let mut renderer = Renderer::try_new(
            RendererDevice::new(
                Arc::new(render_state.device.clone()),
                Arc::new(render_state.queue.clone()),
            ),
            render_state.target_format,
            POOL_CAPACITY,
        )?;
        renderer.configure_streaming(StreamingLimits {
            max_active_charts: 1,
            max_in_flight_chunks: 2,
            max_columns_per_chunk: 7,
            max_chunk_input_bytes: 128 * 1024 * 1024,
            max_in_flight_gpu_bytes: 512 * 1024 * 1024,
        })?;
        renderer.register_streamed_columns(vec![
            DemoState::metadata("stream-x", 2, 1, StreamEncoding::HiLoF32),
            DemoState::metadata("stream-y", 2, 1, StreamEncoding::HiLoF32),
        ])?;
        let placeholder = Rect { x: 0, y: 100, width: 800, height: 500 };
        let mut config = renderer::default::default_config();
        config.chart_area = ChartArea(placeholder);
        let foreground = renderer::Color::new(0.92, 0.92, 0.94, 1.0);
        config.chart_title.text.color = foreground;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.line_color = foreground;
            axis.label_style.color = foreground;
            axis.title_option.text.color = foreground;
        }
        config.grid.major_x_color = renderer::Color::new(0.28, 0.30, 0.35, 1.0);
        config.grid.major_y_color = renderer::Color::new(0.28, 0.30, 0.35, 1.0);
        config.grid.minor_x_color = renderer::Color::new(0.18, 0.20, 0.24, 1.0);
        config.grid.minor_y_color = renderer::Color::new(0.18, 0.20, 0.24, 1.0);
        let mut chart = Chart::new(config)
            .with_title("Exact streaming: every point is drawn")
            .with_x_title("Normalized X")
            .with_y_title("Virtual signal");
        chart.set_x_range(0.0, 1.0);
        chart.set_y_range(0.0, 1.0);
        let series = SeriesConfig {
            series_id: "stream-points".into(),
            source_id: None,
            label: None,
            x_column: "stream-x".into(),
            y_column: "stream-y".into(),
            render_type: DataRenderType::Scatter {
                scatter: DataScatterStyleConfig {
                    point_color: renderer::Color::new(0.15, 0.65, 1.0, 0.35),
                    point_shape: ScatterShape::SquareFilled,
                    point_size: 1.0,
                    point_style_table: None,
                    point_style_index_column: None,
                    point_style_overrides: None,
                },
            },
        };
        let chart_id = renderer.register_chart(chart.config().clone(), vec![series])?;
        let view = renderer.create_chart_view(&chart, placeholder)?;
        render_state.renderer.write().callback_resources.insert(DemoState {
            renderer,
            chart,
            chart_id,
            view,
            x: VirtualColumn { len: 2, axis: VirtualAxis::X },
            y: VirtualColumn { len: 2, axis: VirtualAxis::Y },
            revision: 1,
            pending_start: None,
            last_request: None,
            started: None,
            first_paint_encoded_us: AtomicU64::new(0),
            last_prepare_at: None,
            frame_intervals_ms: Vec::new(),
            surface_size: (0, 0),
            snapshot: Snapshot::default(),
            prepared: None,
        });
        Ok(())
    }

    fn cleanup(&mut self) {
        let Some(render_state) = self.render_state.take() else { return; };
        if let Some(mut state) = render_state
            .renderer
            .write()
            .callback_resources
            .remove::<DemoState>()
        {
            state.shutdown();
        }
    }
}

impl Drop for DemoApp {
    fn drop(&mut self) {
        self.cleanup();
    }
}

impl eframe::App for DemoApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let Some(render_state) = frame.wgpu_render_state() else {
            ui.label("The wgpu render state is unavailable.");
            return;
        };
        self.render_state = Some(render_state.clone());
        if !self.initialized {
            match self.initialize(&render_state) {
                Ok(()) => self.initialized = true,
                Err(error) => self.init_error = Some(error.to_string()),
            }
        }
        if let Some(error) = self.init_error.as_deref() {
            ui.colored_label(egui::Color32::RED, error);
            return;
        }

        let request = RunRequest {
            gib: self.data_gib,
            chunk_points: CHUNK_CHOICES[self.chunk_index],
            precision: self.precision,
        };
        let mut start = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            ui.heading("figgy exact streaming capacity demo");
            ui.add(
                egui::Slider::new(&mut self.data_gib, 0.01..=16.0)
                    .logarithmic(true)
                    .text("logical GiB"),
            );
            egui::ComboBox::from_id_salt("precision")
                .selected_text(self.precision.label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.precision, Precision::F32, Precision::F32.label());
                    ui.selectable_value(
                        &mut self.precision,
                        Precision::F64HiLo,
                        Precision::F64HiLo.label(),
                    );
                });
            egui::ComboBox::from_id_salt("chunk")
                .selected_text(format!("{}K/chunk", CHUNK_CHOICES[self.chunk_index] / 1024))
                .show_ui(ui, |ui| {
                    for (index, chunk) in CHUNK_CHOICES.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.chunk_index,
                            index,
                            format!("{}K points", chunk / 1024),
                        );
                    }
                });
            start = ui.button("Start / Restart").clicked();
            cancel = ui.button("Cancel").clicked();
        });
        ui.label(format!(
            "{} points · {:.3} GiB logical input · exact draw, no LOD/downsampling",
            request.point_count(),
            request.actual_bytes() as f64 / 1024.0 / 1024.0 / 1024.0,
        ));

        let (snapshot, pending, first_paint_us) = {
            let mut guard = render_state.renderer.write();
            let state = guard.callback_resources.get_mut::<DemoState>().expect("demo state");
            if start {
                state.pending_start = Some(request);
            }
            if self.benchmark && state.snapshot.phase == Phase::Idle && state.pending_start.is_none() {
                state.pending_start = Some(request);
            }
            if cancel {
                state.cancel(Phase::Cancelled);
            }
            let first_paint_us = state.first_paint_encoded_us.load(Ordering::Relaxed);
            (state.snapshot.clone(), state.pending_start.is_some(), first_paint_us)
        };
        let progress = if snapshot.points == 0 {
            0.0
        } else {
            snapshot.submitted as f32 / snapshot.points as f32
        };
        ui.add(egui::ProgressBar::new(progress.clamp(0.0, 1.0)).show_percentage());
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Status: {}", snapshot.phase.label()));
            ui.label(format!("Submitted: {} / {} points", snapshot.submitted, snapshot.points));
            if snapshot.bytes != 0 {
                ui.label(format!(
                    "Active input: {:.3} GiB",
                    snapshot.bytes as f64 / 1024.0 / 1024.0 / 1024.0,
                ));
            }
            if let Some(ms) = snapshot.first_submit_ms {
                ui.label(format!("First chunk submitted: {ms:.2} ms"));
            }
            if first_paint_us != 0 {
                ui.label(format!(
                    "First paint encoded: {:.2} ms (not present time)",
                    first_paint_us as f64 / 1000.0,
                ));
            }
            if let Some(ms) = snapshot.all_submitted_ms {
                ui.label(format!("All chunks submitted: {ms:.2} ms"));
            }
            if let Some(ms) = snapshot.gpu_complete_ms {
                let throughput = snapshot.points as f64 / ms / 1000.0;
                ui.colored_label(
                    egui::Color32::LIGHT_GREEN,
                    format!("Full GPU completion: {ms:.2} ms · {throughput:.2} Mpoints/s"),
                );
            }
        });
        if let Some(error) = snapshot.error.as_deref() {
            ui.colored_label(egui::Color32::RED, error);
        }
        if self.benchmark
            && !self.benchmark_reported
            && matches!(snapshot.phase, Phase::Done | Phase::Failed)
        {
            self.benchmark_reported = true;
            eprintln!(
                "STREAMING_BENCHMARK phase={} points={} logical_input_bytes={} \
                 submitted_chunks={} first_submit_ms={:?} first_paint_encoded_ms={:?} \
                 all_submitted_ms={:?} gpu_complete_ms={:?} \
                 frame_interval_mean_ms={:?} frame_interval_p95_ms={:?} \
                 frame_interval_max_ms={:?} sampled_peak_in_flight_chunks={} \
                 sampled_peak_reserved_gpu_bytes={} renderer_accounted_gpu_peak_bytes={} \
                 gpu_copy_bytes=unmeasured input_delay_ms=unmeasured error={:?}",
                snapshot.phase.label(),
                snapshot.points,
                snapshot.bytes,
                snapshot.submitted_chunks,
                snapshot.first_submit_ms,
                (first_paint_us != 0).then_some(first_paint_us as f64 / 1000.0),
                snapshot.all_submitted_ms,
                snapshot.gpu_complete_ms,
                snapshot.mean_frame_interval_ms,
                snapshot.p95_frame_interval_ms,
                snapshot.max_frame_interval_ms,
                snapshot.peak_in_flight_chunks,
                snapshot.peak_reserved_gpu_bytes,
                snapshot.renderer_accounted_gpu_peak_bytes,
                snapshot.error,
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.separator();

        let pixels_per_point = ctx.pixels_per_point();
        let available = ui.available_size();
        let (rect, _) = ui.allocate_exact_size(available, egui::Sense::hover());
        let panel_rect = Rect {
            x: (rect.min.x * pixels_per_point).round().max(0.0) as u32,
            y: (rect.min.y * pixels_per_point).round().max(0.0) as u32,
            width: (rect.width() * pixels_per_point).max(1.0) as u32,
            height: (rect.height() * pixels_per_point).max(1.0) as u32,
        };
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            DemoCallback { panel_rect },
        ));
        if snapshot.phase.active() || start || pending {
            ctx.request_repaint();
        }
    }

    fn on_exit(&mut self) {
        self.cleanup();
    }
}

fn main() -> eframe::Result<()> {
    let benchmark = std::env::args().any(|arg| arg == "--benchmark");
    eframe::run_native(
        "figgy streaming capacity demo",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1400.0, 850.0])
                .with_title("figgy exact streaming capacity demo"),
            renderer: eframe::Renderer::Wgpu,
            ..Default::default()
        },
        Box::new(move |_| {
            let mut app = DemoApp::default();
            app.benchmark = benchmark;
            Ok(Box::new(app))
        }),
    )
}
