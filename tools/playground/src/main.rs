//! figgy playground — every chart family in one interactive window.
//!
//! A throwaway test bench, not an API sample: winit owns the window and the
//! event loop, this file owns the wgpu surface, and egui (through
//! `egui-winit` + `egui-wgpu`, no eframe) draws the control panel over it.
//!
//! - Cartesian scenes (lines/areas/steps, scatter + error bars, histogram,
//!   heatmap + contours, log axes, twin Y axes, subplots, annotations, up to
//!   100 million resident points) are registered charts of one `Renderer`. The renderer paints
//!   them straight into the surface pass; egui draws on top.
//! - Bounded scenes (radial, radar) render into an
//!   offscreen texture that egui shows as an image.
//!
//! Pointer, on the chart:
//! - Left press: select the element (axis, labels, titles, legend, data
//!   area, colourbar, annotation) and drag it; the selected data area has
//!   eight resize handles. In "Pick data" mode a click picks the nearest
//!   point / bin / cell / contour (Shift adds to the selection).
//! - Wheel: zoom around the cursor (Cartesian; Shift = X only, Ctrl = Y only).
//! - Right or middle drag: pan (Cartesian).
//!
//! Run:
//! `cargo run --manifest-path tools/playground/Cargo.toml` (from the repo root)

mod big_data;
mod bounded;
mod cartesian;
/// The scene catalogue, shared with the renderer's decoration oracle.
#[path = "../../../crates/renderer/tests/support/playground_scenes.rs"]
mod scenes;
mod widgets;

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use renderer::data_render;
use renderer::{Renderer, RendererDevice};

use crate::bounded::BoundedScene;
use crate::cartesian::CartesianHost;
use crate::scenes::SceneKind;

const POOL_CAPACITY: u64 = 128 * 1024 * 1024;
const PANEL_WIDTH: f32 = 330.0;

/// GPU objects the host owns. figgy only borrows the device and queue.
struct Gpu {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    _adapter: wgpu::Adapter,
    _instance: wgpu::Instance,
}

impl Gpu {
    fn new(window: Arc<Window>) -> Result<Self, Box<dyn std::error::Error>> {
        let size = window.inner_size();
        let instance = data_render::create_instance();
        let surface = data_render::create_surface_for_window(&instance, window)?;
        let adapter = data_render::request_adapter_for_surface(&instance, &surface)?;
        eprintln!("[gpu] adapter: {:?}", adapter.get_info());
        let (device, queue) = request_device(&adapter)?;
        let limits = device.limits();
        eprintln!(
            "[gpu] max buffer {} MiB, max storage binding {} MiB",
            limits.max_buffer_size >> 20,
            limits.max_storage_buffer_binding_size >> 20
        );
        let device = Arc::new(device);
        let queue = Arc::new(queue);
        let config = data_render::try_configure_surface(
            &surface,
            &adapter,
            &device,
            size.width.max(1),
            size.height.max(1),
        )?;
        eprintln!("[gpu] surface format: {:?}", config.format);
        Ok(Self {
            surface,
            config,
            device,
            queue,
            _adapter: adapter,
            _instance: instance,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    fn renderer_device(&self) -> RendererDevice {
        RendererDevice::new(Arc::clone(&self.device), Arc::clone(&self.queue))
    }
}

/// A device with the adapter's own buffer limits. The WebGPU baseline caps one
/// buffer at 256 MiB and a storage binding at 128 MiB; the column pool is one
/// buffer, so 100 million resident points (1.6 GB of hi/lo pairs) need what
/// the GPU actually allows. Every other limit stays at the baseline.
fn request_device(
    adapter: &wgpu::Adapter,
) -> Result<(wgpu::Device, wgpu::Queue), wgpu::RequestDeviceError> {
    let supported = adapter.limits();
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("figgy playground device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits {
            max_buffer_size: supported.max_buffer_size,
            max_storage_buffer_binding_size: supported.max_storage_buffer_binding_size,
            ..wgpu::Limits::default()
        },
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    };
    pollster::block_on(adapter.request_device(&descriptor))
}

/// What the scene code needs from the host while egui runs.
pub struct HostCtx<'a> {
    pub device: &'a Arc<wgpu::Device>,
    pub queue: &'a Arc<wgpu::Queue>,
    pub egui_renderer: &'a mut egui_wgpu::Renderer,
    pub pixels_per_point: f32,
    pub status: &'a mut String,
}

/// Exactly one scene is live at a time.
enum Active {
    Cartesian(Box<CartesianHost>),
    Bounded(Box<dyn BoundedScene>),
}

struct Playground {
    /// One Cartesian renderer for every Cartesian scene; columns stay in its
    /// pool across scene switches.
    cartesian: Renderer,
    kind: SceneKind,
    active: Active,
    status: String,
}

impl Playground {
    fn new(gpu: &Gpu) -> Result<Self, Box<dyn std::error::Error>> {
        let mut cartesian =
            Renderer::try_new(gpu.renderer_device(), gpu.config.format, POOL_CAPACITY)?;
        // The large-data scene uploads up to 1.6 GB; the pool grows to fit, up
        // to the device's buffer limit.
        cartesian.set_pool_growth_policy(data_render::GrowthPolicy::OnAllocFailure);
        if let Err(e) = cartesian.enable_gpu_picking() {
            eprintln!("[pick] GPU picking unavailable: {e}");
        }
        let kind = SceneKind::Lines;
        let active = Active::Cartesian(Box::new(CartesianHost::new(&mut cartesian, kind)?));
        Ok(Self {
            cartesian,
            kind,
            active,
            status: String::new(),
        })
    }

    fn switch(&mut self, kind: SceneKind, gpu: &Gpu, egui_renderer: &mut egui_wgpu::Renderer) {
        let next = if kind.is_bounded() {
            bounded::create(kind, gpu.renderer_device()).map(Active::Bounded)
        } else {
            CartesianHost::new(&mut self.cartesian, kind)
                .map(|c| Active::Cartesian(Box::new(c)))
                .map_err(|e| e.to_string())
        };
        match next {
            Ok(next) => {
                match std::mem::replace(&mut self.active, next) {
                    Active::Cartesian(old) => old.remove(&mut self.cartesian),
                    Active::Bounded(mut old) => old.release(egui_renderer),
                }
                self.kind = kind;
                self.status = format!("Scene: {}", kind.label());
            }
            Err(e) => self.status = format!("Scene {} failed: {e}", kind.label()),
        }
    }
}

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    playground: Option<Playground>,
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            gpu: None,
            egui_ctx: egui::Context::default(),
            egui_state: None,
            egui_renderer: None,
            playground: None,
        }
    }

    fn shutdown(&mut self) {
        if let Some(gpu) = &self.gpu {
            let _ = gpu.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
        }
        if let (Some(p), Some(r)) = (self.playground.as_mut(), self.egui_renderer.as_mut())
            && let Active::Bounded(scene) = &mut p.active
        {
            scene.release(r);
        }
        self.playground = None;
        self.egui_renderer = None;
        self.egui_state = None;
        self.gpu = None;
        self.window = None;
    }

    fn redraw(&mut self) {
        let (Some(window), Some(gpu), Some(state), Some(egui_renderer), Some(playground)) = (
            self.window.as_ref(),
            self.gpu.as_mut(),
            self.egui_state.as_mut(),
            self.egui_renderer.as_mut(),
            self.playground.as_mut(),
        ) else {
            return;
        };

        // ---- egui pass: controls + pointer handling ----
        let raw_input = state.take_egui_input(window);
        let mut switch_to = None;
        let ppp = self.egui_ctx.pixels_per_point();
        let mut full_output = self.egui_ctx.run_ui(raw_input, |ui| {
            let mut host = HostCtx {
                device: &gpu.device,
                queue: &gpu.queue,
                egui_renderer,
                pixels_per_point: ppp,
                status: &mut playground.status,
            };
            // Scene tabs: every chart family one click away.
            egui::Panel::top("scene_tabs").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong("Cartesian");
                    for k in SceneKind::ALL.iter().filter(|k| !k.is_bounded()) {
                        if ui
                            .selectable_label(playground.kind == *k, k.label())
                            .clicked()
                            && playground.kind != *k
                        {
                            switch_to = Some(*k);
                        }
                    }
                    ui.separator();
                    ui.strong("Bounded");
                    for k in SceneKind::ALL.iter().filter(|k| k.is_bounded()) {
                        if ui
                            .selectable_label(playground.kind == *k, k.label())
                            .clicked()
                            && playground.kind != *k
                        {
                            switch_to = Some(*k);
                        }
                    }
                });
            });
            egui::Panel::left("controls")
                .resizable(false)
                .exact_size(PANEL_WIDTH)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.heading(playground.kind.label());
                        ui.label(egui::RichText::new(playground.kind.hint()).weak());
                        if !host.status.is_empty() {
                            ui.label(egui::RichText::new(host.status.as_str()).monospace());
                        }
                        ui.separator();
                        match &mut playground.active {
                            Active::Cartesian(c) => {
                                c.controls(ui, &mut playground.cartesian, &mut host)
                            }
                            Active::Bounded(b) => b.controls(ui, &mut host),
                        }
                    });
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    let rect = ui.available_rect_before_wrap();
                    let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                    match &mut playground.active {
                        Active::Cartesian(c) => {
                            c.viewport(ui, rect, &response, &mut playground.cartesian, &mut host)
                        }
                        Active::Bounded(b) => b.viewport(ui, rect, &response, &mut host),
                    }
                });
        });
        state.handle_platform_output(window, full_output.platform_output);
        if let Some(kind) = switch_to {
            playground.switch(kind, gpu, egui_renderer);
            window.request_redraw();
        }

        // ---- figgy prepare: the Cartesian chart goes straight to the surface ----
        let frame_start = Instant::now();
        let prepared = match &mut playground.active {
            Active::Cartesian(c) => c.prepare(&mut playground.cartesian),
            Active::Bounded(_) => None,
        };
        let prepare_time = frame_start.elapsed();
        let timed = matches!(&playground.active, Active::Cartesian(c) if c.wants_frame_timing());

        // egui asserts that every texture delta is consumed: upload the new
        // ones now, free the old ones once this frame is submitted (or skipped).
        let mut textures = std::mem::take(&mut full_output.textures_delta);
        for (id, deltas) in textures.set.drain() {
            for delta in deltas {
                egui_renderer.update_texture(&gpu.device, &gpu.queue, id, &delta);
            }
        }
        let freed: Vec<egui::TextureId> = textures.free.drain().collect();

        // ---- frame ----
        let frame = match gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            other => {
                eprintln!("[surface] {other:?}; reconfiguring");
                let (w, h) = (gpu.config.width, gpu.config.height);
                gpu.resize(w, h);
                for id in &freed {
                    egui_renderer.free_texture(id);
                }
                window.request_redraw();
                return;
            }
        };
        let target = frame.texture.create_view(&Default::default());
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [gpu.config.width, gpu.config.height],
            pixels_per_point: full_output.pixels_per_point,
        };
        let primitives = self
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        let mut buffers = egui_renderer.update_buffers(
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &primitives,
            &screen,
        );
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("playground frame"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                })
                .forget_lifetime();
            if let Some(prepared) = &prepared
                && let Err(e) = playground.cartesian.paint_prepared(
                    &mut pass,
                    (gpu.config.width, gpu.config.height),
                    prepared,
                )
            {
                eprintln!("[paint] {e}");
                window.request_redraw();
            }
            egui_renderer.render(&mut pass, &primitives, &screen);
        }
        buffers.push(encoder.finish());
        gpu.queue.submit(buffers);
        if timed {
            // Wait for the GPU so the time covers the whole frame, not just
            // its recording.
            if let Err(e) = gpu.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            }) {
                eprintln!("[timing] poll: {e}");
            }
            if let Active::Cartesian(c) = &mut playground.active
                && c.frame_timed(
                    &mut playground.cartesian,
                    prepare_time,
                    frame_start.elapsed(),
                )
            {
                window.request_redraw();
            }
        }
        window.pre_present_notify();
        gpu.queue.present(frame);
        drop(prepared);
        playground.cartesian.end_gpu_frame();
        for id in &freed {
            egui_renderer.free_texture(id);
        }

        let repaint = full_output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay);
        if repaint.is_some_and(|d| d.is_zero()) {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("figgy playground (winit + egui)")
            .with_inner_size(LogicalSize::new(1500.0, 900.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("window"));
        let gpu = match Gpu::new(Arc::clone(&window)) {
            Ok(gpu) => gpu,
            Err(e) => {
                eprintln!("[init] {e}");
                event_loop.exit();
                return;
            }
        };
        self.egui_ctx.set_visuals(egui::Visuals::light());
        let state = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer =
            egui_wgpu::Renderer::new(&gpu.device, gpu.config.format, Default::default());
        let playground = match Playground::new(&gpu) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[init] {e}");
                event_loop.exit();
                return;
            }
        };
        self.egui_state = Some(state);
        self.egui_renderer = Some(egui_renderer);
        self.playground = Some(playground);
        self.gpu = Some(gpu);
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if let Some(state) = self.egui_state.as_mut() {
            let response = state.on_window_event(&window, &event);
            if response.repaint {
                window.request_redraw();
            }
        }
        match event {
            WindowEvent::CloseRequested => {
                self.shutdown();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn main() {
    if std::env::args().any(|a| a == "--selftest") {
        let instance = data_render::create_instance();
        let adapter = data_render::request_adapter(&instance).expect("adapter");
        let (d, q) = data_render::request_device(&adapter).expect("device");
        let gpu = RendererDevice::new(Arc::new(d), Arc::new(q));
        let mut r = Renderer::try_new(gpu, wgpu::TextureFormat::Bgra8Unorm, POOL_CAPACITY)
            .expect("renderer");
        match cartesian::selftest(&mut r) {
            Ok(()) => println!("selftest passed"),
            Err(e) => {
                eprintln!("selftest FAILED: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new();
    event_loop.run_app(&mut app).expect("run app");
}
