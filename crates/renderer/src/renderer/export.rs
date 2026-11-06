//! PNG and RGBA export of charts and documents: scale limits, the export
//! render path and the per-panel draw storage it uses.

use super::*;
use crate::gpu_readback::{GpuReadback, ReadbackError};

/// Lower bound for export scale. 1.0 is screen-equivalent (96 DPI baseline,
/// `scale = dpi / 96`); 0.25 corresponds to ~24 DPI (thumbnails).
pub const MIN_EXPORT_SCALE: f32 = 0.25;

/// Convert DPI to a `scale` value relative to the 96 DPI baseline.
pub fn dpi_to_scale(dpi: f32) -> f32 {
    clamp_export_scale(dpi / 96.0)
}

/// Raise `scale` to at least [`MIN_EXPORT_SCALE`] (NaN and values at or
/// below zero become it). There is no upper bound: the scaled target must fit
/// the device's texture limit (`max_texture_dimension_2d`), and an export
/// past it fails with [`FiggyError::GpuResourceLimit`] before anything is
/// allocated.
pub fn clamp_export_scale(scale: f32) -> f32 {
    if scale.is_nan() || scale <= 0.0 {
        return MIN_EXPORT_SCALE;
    }
    scale.max(MIN_EXPORT_SCALE)
}

/// In-memory RGBA8 image (straight alpha).
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, row-major, channel order R, G, B, A.
    pub rgba: Vec<u8>,
}

/// Encode `img` as PNG bytes in memory. Saving to disk is the caller's job.
///
/// The bytes grow fallibly: an output the host cannot allocate is
/// [`FiggyError::StateAllocationFailed`], not an abort.
pub fn encode_png(img: &RasterImage) -> Result<Vec<u8>> {
    let mut buf = FallibleBytes(Vec::new());
    {
        let mut enc = png::Encoder::new(&mut buf, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| png_error("png header", e))?;
        writer
            .write_image_data(&img.rgba)
            .map_err(|e| png_error("png write", e))?;
    }
    Ok(buf.0)
}

/// PNG output bytes whose growth is fallible: a failed reservation is an
/// out-of-memory I/O error, which the encoder returns.
struct FallibleBytes(Vec<u8>);

impl std::io::Write for FallibleBytes {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0
            .try_reserve(data.len())
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::OutOfMemory, error))?;
        self.0.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn png_error(stage: &str, error: png::EncodingError) -> FiggyError {
    match error {
        png::EncodingError::IoError(io) if io.kind() == std::io::ErrorKind::OutOfMemory => {
            FiggyError::StateAllocationFailed {
                resource: "png export bytes",
                reason: io.to_string(),
            }
        }
        other => FiggyError::RasterWrapFailed {
            reason: format!("{stage}: {other}"),
        },
    }
}

/// Run an export inside native out-of-memory, validation and internal error
/// scopes, popped once it is done. An error the export returned itself is
/// reported first; otherwise any captured GPU error fails the export, since
/// an image rendered through a failed resource is not one to save. `gpu`
/// turns a captured error into the caller's error type. Every export uses
/// it, the radial chart's included (#160).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn native_export_scoped<T, E>(
    device: &wgpu::Device,
    work: impl std::future::Future<Output = std::result::Result<T, E>>,
    gpu: impl FnOnce(FiggyError) -> E,
) -> std::result::Result<T, E> {
    // wgpu 30's scope guards pop themselves on drop, so a cancelled export
    // cannot leave a scope on the device's stack for the next caller.
    let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let rendered = work.await;
    // Popped in reverse push order, every one, before any is inspected.
    let internal = internal.pop().await;
    let validation = validation.pop().await;
    let out_of_memory = out_of_memory.pop().await;
    settle_export_scopes(rendered, [out_of_memory, validation, internal], gpu)
}

/// [`native_export_scoped`] for a blocking export: the same scopes around
/// `work`, popped at once.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn native_export_scoped_blocking<T, E>(
    device: &wgpu::Device,
    work: impl FnOnce() -> std::result::Result<T, E>,
    gpu: impl FnOnce(FiggyError) -> E,
) -> std::result::Result<T, E> {
    let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let rendered = work();
    // Popped in reverse push order, every one, before any is inspected.
    let internal = pop_scope_now(internal);
    let validation = pop_scope_now(validation);
    let out_of_memory = pop_scope_now(out_of_memory);
    let popped = match (out_of_memory, validation, internal) {
        (Ok(out_of_memory), Ok(validation), Ok(internal)) => [out_of_memory, validation, internal],
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
            drop(rendered);
            return Err(gpu(error));
        }
    };
    settle_export_scopes(rendered, popped, gpu)
}

/// An export's result after its scopes are popped: an error the export
/// returned itself first, then the first captured GPU error in
/// out-of-memory, validation, internal order.
#[cfg(not(target_arch = "wasm32"))]
fn settle_export_scopes<T, E>(
    rendered: std::result::Result<T, E>,
    popped: [Option<wgpu::Error>; 3],
    gpu: impl FnOnce(FiggyError) -> E,
) -> std::result::Result<T, E> {
    let rendered = rendered?;
    let [out_of_memory, validation, internal] = popped;
    match out_of_memory.or(validation).or(internal) {
        Some(error) => Err(gpu(gpu_error("figgy export", error))),
        None => Ok(rendered),
    }
}

/// Pop a native error scope without waiting (see
/// [`crate::gpu_readback::ready_now`]): a pop that is not ready is reported
/// as an internal error, never waited for.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pop_scope_now(scope: wgpu::ErrorScopeGuard) -> Result<Option<wgpu::Error>> {
    crate::gpu_readback::ready_now(scope.pop()).ok_or(FiggyError::internal(
        "a native wgpu error scope pops at once",
    ))
}

/// What an export renders: one panel or the whole layered document.
///
/// The panel export of a layered document draws its base panel only (with
/// the group data area); for a document without overlays or subplots the two
/// produce byte-identical images. Vector exports (SVG, PDF) render the whole
/// document. A radial chart has one panel, so both scopes are the same there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ExportScope {
    Panel,
    #[default]
    Document,
}

/// How an export renders: its [`ExportScope`], its `scale` and its
/// background.
///
/// - `scale` multiplies the logical size; fonts, line widths and margins
///   scale with it. It is raised to at least [`MIN_EXPORT_SCALE`]; past the
///   device's texture limit the export fails with
///   [`FiggyError::GpuResourceLimit`].
/// - `clear` is the background, fully transparent by default. A vector
///   export draws it as a rectangle under everything.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportOptions {
    pub scope: ExportScope,
    pub scale: f32,
    pub clear: crate::color::Color,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            scope: ExportScope::Document,
            scale: 1.0,
            clear: crate::color::Color::from_rgba(0.0, 0.0, 0.0, 0.0),
        }
    }
}

impl ExportOptions {
    /// One panel at `scale`, on a transparent background.
    pub fn panel(scale: f32) -> Self {
        Self {
            scope: ExportScope::Panel,
            scale,
            ..Self::default()
        }
    }

    /// The whole document at `scale`, on a transparent background.
    pub fn document(scale: f32) -> Self {
        Self {
            scope: ExportScope::Document,
            scale,
            ..Self::default()
        }
    }

    /// These options over a background of `clear`.
    pub fn with_clear(self, clear: crate::color::Color) -> Self {
        Self { clear, ..self }
    }
}

/// The chart an export renders: a chart the caller holds, or one registered
/// with the renderer.
#[derive(Clone, Copy)]
pub enum ExportSource<'a> {
    /// A chart and its series as the caller holds them.
    Chart {
        chart: &'a Chart,
        series: &'a [SeriesConfig],
    },
    /// A chart registered with [`Renderer::register_chart`]: its stored
    /// config and series, as they are serialized (not its display placement).
    Registered(ChartId),
}

impl<'a> ExportSource<'a> {
    /// [`Self::Chart`].
    pub fn chart(chart: &'a Chart, series: &'a [SeriesConfig]) -> Self {
        Self::Chart { chart, series }
    }
}

/// Owned storage for the internal panels of a prepare or export call: the
/// expanded configs and each panel's slice of the series, so the per-panel
/// `ChartDrawItem`s can borrow from one place. Expansion copies configuration
/// only; the series entries are the caller's references.
///
/// Only layered items are materialized here; a single-layer item is drawn
/// through the caller's own references, so a frame without overlays
/// allocates nothing for expansion.
pub(super) struct PanelDrawStorage<'a> {
    pub(super) configs: Vec<Config>,
    pub(super) series: Vec<Vec<Series<'a>>>,
    pub(super) views: Vec<&'a ChartView>,
    /// Panels per layered source item, in source order.
    pub(super) counts: Vec<usize>,
}

impl<'a> PanelDrawStorage<'a> {
    pub(super) fn expand(items: &[ChartDrawItem<'a>]) -> Result<Self> {
        let mut configs = Vec::new();
        let mut series = Vec::new();
        let mut views = Vec::new();
        let mut counts = Vec::new();
        for item in items {
            if !is_layered(item.chart_config) {
                continue;
            }
            let panels = expand_for_render(item.chart_config)?;
            let panel_views = layer_views(item.view, panels.len())?;
            let mut partition: Vec<Vec<Series<'a>>> = vec![Vec::new(); panels.len()];
            for entry in item.series {
                let index =
                    series_panel_index(item.chart_config, panels.len(), &entry.config.series_id);
                partition[index].push(*entry);
            }
            counts.push(panels.len());
            configs.extend(panels);
            series.extend(partition);
            views.extend(panel_views);
        }
        Ok(Self {
            configs,
            series,
            views,
            counts,
        })
    }

    /// The per-panel draw items of `source` (the items `expand` was given),
    /// in source order: a single-layer item as it is, a layered item as its
    /// expanded panels.
    pub(super) fn items<'s>(
        &'s self,
        source: &'s [ChartDrawItem<'a>],
    ) -> Result<Vec<ChartDrawItem<'s>>> {
        let mut out = Vec::with_capacity(source.len() + self.configs.len());
        let mut counts = self.counts.iter();
        let mut next = 0;
        for item in source {
            if !is_layered(item.chart_config) {
                out.push(ChartDrawItem {
                    view: item.view,
                    chart_config: item.chart_config,
                    series: item.series,
                });
                continue;
            }
            let count = *counts
                .next()
                .ok_or(FiggyError::internal("one count per layered item"))?;
            for index in next..next + count {
                out.push(ChartDrawItem {
                    view: self.views[index],
                    chart_config: &self.configs[index],
                    series: &self.series[index],
                });
            }
            next += count;
        }
        Ok(out)
    }
}

impl PanelDrawStorage<'_> {
    /// Panels per source item, in source order (1 for a single-layer item).
    pub(super) fn group_sizes(&self, source: &[ChartDrawItem<'_>]) -> Result<Vec<usize>> {
        let mut counts = self.counts.iter();
        source
            .iter()
            .map(|item| {
                if !is_layered(item.chart_config) {
                    Ok(1)
                } else {
                    counts
                        .next()
                        .copied()
                        .ok_or(FiggyError::internal("one count per layered item"))
                }
            })
            .collect()
    }
}

/// What [`Renderer::prepare_export`] builds for one export: its panels'
/// prepared draws and the target they render into, with every resource
/// those draws read kept alive until the export ends.
pub(crate) struct ExportFrame {
    /// The target size in physical pixels.
    pub(crate) size: (u32, u32),
    /// The exported area's size in logical pixels.
    pub(crate) logical: (u32, u32),
    /// The export scale `chart`'s pixel values carry.
    pub(crate) scale: f32,
    /// The scaled document, placed at the target's origin.
    pub(crate) chart: Chart,
    /// The expanded panels of a layered document; empty for a single-layer
    /// one, whose only panel is `chart`'s config.
    panel_configs: Vec<Config>,
    /// One prepared draw per panel, in panel order.
    pub(super) items: Vec<PreparedItem>,
    target: TrackedTexture,
    msaa: Option<MsaaTarget>,
    _view: ChartView,
    _styles: Vec<ChartStyle>,
    _pipelines: TargetPipelines,
    #[cfg(target_arch = "wasm32")]
    browser_scopes: Option<BrowserErrorScopes>,
}

impl ExportFrame {
    /// Every panel's config in the scaled document's frame, in panel order:
    /// the order of [`Self::paint_export`]'s panel indices.
    pub(crate) fn panels(&self) -> &[Config] {
        if self.panel_configs.is_empty() {
            std::slice::from_ref(self.chart.config())
        } else {
            &self.panel_configs
        }
    }
}

impl Renderer {
    /// Render `source` offscreen and read it back as straight-alpha RGBA8
    /// (#160): the panel or the document by `options.scope`, at
    /// `options.scale × ` its logical size, over `options.clear`.
    ///
    /// A series with a streamed source is rejected. On native the bundle's
    /// poll thread completes the readback (see [`RendererDevice::new`]);
    /// `Self::export_rgba_blocking` waits on the caller's thread instead. `&mut self` for the prepare phase
    /// (arc prefixes); the export's styled pipelines live in a per-call set.
    /// Encoding is the caller's: [`encode_png`] or [`Self::export_png`].
    pub async fn export_rgba(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
    ) -> Result<RasterImage> {
        match source {
            ExportSource::Chart { chart, series } => {
                self.export_rgba_with_clear_scoped(
                    chart,
                    series,
                    options.scale,
                    options.clear,
                    options.scope,
                )
                .await
            }
            ExportSource::Registered(id) => {
                let (chart, series) = self.registered_export_state(id)?;
                self.export_rgba_with_clear_scoped(
                    &chart,
                    &series,
                    options.scale,
                    options.clear,
                    options.scope,
                )
                .await
            }
        }
    }

    /// [`Self::export_rgba`], encoded as PNG bytes in memory. Saving them is
    /// the caller's.
    pub async fn export_png(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
    ) -> Result<Vec<u8>> {
        let image = self.export_rgba(source, options).await?;
        encode_png(&image)
    }

    /// `source`'s whole document as an SVG document at `options.scale ×`
    /// resolution (#48). The decorations are vector content from the
    /// placement the PNG export draws with: axes, ticks, tick labels, grid,
    /// titles, legend, colourbar, polar chrome and the annotation layer,
    /// glyphs written as outlines. The data layer is drawn by the GPU, so
    /// each panel group's data (fills, bodies, picked highlights, value
    /// labels) is embedded as a PNG image of its data area, the same pixels
    /// as the PNG export; the colourbar ramp and the Milkyway backdrop are
    /// images too, and the Milkyway glow is omitted. The `viewBox` is in
    /// physical pixels and the size the logical size in points
    /// (1 px = 0.75 pt). `options.clear` is a rectangle under everything.
    ///
    /// A vector export renders the whole document: `ExportScope::Panel` is
    /// refused with [`FiggyError::InvalidConfig`]. A series with a streamed
    /// source is rejected, as for the PNG export.
    pub async fn export_svg(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
    ) -> Result<String> {
        self.export_vector::<crate::vector_export::SvgOut>(source, options)
            .await
    }

    /// [`Self::export_svg`] as a one-page PDF: the page is the logical size
    /// in points, each image a deflated RGB image with a soft mask. Glyphs
    /// are outlines, so the text is not searchable.
    pub async fn export_pdf(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
    ) -> Result<Vec<u8>> {
        self.export_vector::<crate::vector_export::PdfOut>(source, options)
            .await
    }

    /// [`Self::export_rgba`], polling the device on this thread until the
    /// readback completes. Works on a bundle built with
    /// `RendererDevice::blocking` as with [`RendererDevice::new`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_rgba_blocking(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
    ) -> Result<RasterImage> {
        let (chart, series);
        let (chart, series) = match source {
            ExportSource::Chart { chart, series } => (chart, series),
            ExportSource::Registered(id) => {
                (chart, series) = self.registered_export_state(id)?;
                (&chart, series.as_slice())
            }
        };
        self.export_begin(series)?;
        let device = self.device.clone();
        let result = native_export_scoped_blocking(
            &device,
            || {
                let mut frame = self.prepare_export(chart, series, options.scale, options.scope)?;
                let panels = 0..frame.items.len();
                self.paint_export_blocking(&mut frame, panels, true, options.clear)
            },
            std::convert::identity,
        );
        self.export_end(result)
    }

    /// [`Self::export_png`], polling the device on this thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_png_blocking(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
    ) -> Result<Vec<u8>> {
        let image = self.export_rgba_blocking(source, options)?;
        encode_png(&image)
    }

    /// [`Self::export_svg`], polling the device on this thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_svg_blocking(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
    ) -> Result<String> {
        self.export_vector_blocking::<crate::vector_export::SvgOut>(source, options)
    }

    /// [`Self::export_pdf`], polling the device on this thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn export_pdf_blocking(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
    ) -> Result<Vec<u8>> {
        self.export_vector_blocking::<crate::vector_export::PdfOut>(source, options)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn export_vector_blocking<F: crate::vector_export::VectorFile>(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
    ) -> Result<F::Output> {
        check_vector_scope(options)?;
        let (chart, series);
        let (chart, series) = match source {
            ExportSource::Chart { chart, series } => (chart, series),
            ExportSource::Registered(id) => {
                (chart, series) = self.registered_export_state(id)?;
                (&chart, series.as_slice())
            }
        };
        self.export_begin(series)?;
        let device = self.device.clone();
        let result = native_export_scoped_blocking(
            &device,
            || {
                let mut frame =
                    self.prepare_export(chart, series, options.scale, ExportScope::Document)?;
                crate::vector_export::document_file_blocking::<F>(self, &mut frame, options.clear)
            },
            std::convert::identity,
        );
        self.export_end(result)
    }

    /// The checks every export makes before it renders.
    fn export_begin(&self, series: &[SeriesConfig]) -> Result<()> {
        self.health.check()?;
        for item in series {
            self.require_resident_series(item)?;
        }
        Ok(())
    }

    /// Every export submission and readback has completed and the export's
    /// transient owners are dropped: end the frame, and fail the export on a
    /// GPU error that reached the uncaptured-error handler meanwhile (a
    /// browser scope resolves asynchronously).
    fn export_end<T>(&mut self, result: Result<T>) -> Result<T> {
        self.end_gpu_frame();
        let value = result?;
        self.health.check()?;
        Ok(value)
    }

    async fn export_vector<F: crate::vector_export::VectorFile>(
        &mut self,
        source: ExportSource<'_>,
        options: ExportOptions,
    ) -> Result<F::Output> {
        check_vector_scope(options)?;
        match source {
            ExportSource::Chart { chart, series } => {
                self.export_vector_scoped::<F>(chart, series, options.scale, options.clear)
                    .await
            }
            ExportSource::Registered(id) => {
                let (chart, series) = self.registered_export_state(id)?;
                self.export_vector_scoped::<F>(&chart, &series, options.scale, options.clear)
                    .await
            }
        }
    }

    /// A registered chart's stored config, as a [`Chart`], and its series,
    /// for an export. Configuration only is copied (rule 2): the export
    /// prepares through `&mut self`, so it cannot borrow the registry.
    fn registered_export_state(&self, id: ChartId) -> Result<(Chart, Vec<SeriesConfig>)> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        Ok((Chart::new(state.config.clone()), state.series.clone()))
    }

    /// The vector export in format `F`, with the PNG export's checks, device
    /// error scopes and end-of-frame report
    /// ([`Self::export_rgba_with_clear_scoped`]).
    async fn export_vector_scoped<F: crate::vector_export::VectorFile>(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<F::Output> {
        self.export_begin(series)?;
        #[cfg(target_arch = "wasm32")]
        let result = self
            .export_vector_inner::<F>(chart, series, scale, clear)
            .await;

        #[cfg(not(target_arch = "wasm32"))]
        let result = {
            let device = self.device.clone();
            native_export_scoped(
                &device,
                self.export_vector_inner::<F>(chart, series, scale, clear),
                std::convert::identity,
            )
            .await
        };
        self.export_end(result)
    }

    async fn export_vector_inner<F: crate::vector_export::VectorFile>(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
    ) -> Result<F::Output> {
        let mut frame = self.prepare_export(chart, series, scale, ExportScope::Document)?;
        crate::vector_export::document_file::<F>(self, &mut frame, clear).await
    }

    pub(super) async fn export_rgba_with_clear_scoped(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
        scope: ExportScope,
    ) -> Result<RasterImage> {
        self.export_begin(series)?;
        #[cfg(target_arch = "wasm32")]
        let result = self
            .export_rgba_with_clear_inner(chart, series, scale, clear, scope)
            .await;

        #[cfg(not(target_arch = "wasm32"))]
        let result = {
            let device = self.device.clone();
            native_export_scoped(
                &device,
                self.export_rgba_with_clear_inner(chart, series, scale, clear, scope),
                std::convert::identity,
            )
            .await
        };
        self.export_end(result)
    }

    pub(super) async fn export_rgba_with_clear_inner(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        clear: crate::color::Color,
        scope: ExportScope,
    ) -> Result<RasterImage> {
        let mut frame = self.prepare_export(chart, series, scale, scope)?;
        let panels = 0..frame.items.len();
        self.paint_export(&mut frame, panels, true, clear).await
    }

    /// The first half of an export: validate the series, scale and place
    /// the document at the origin of a `scale ×` target, build its views,
    /// styles and pipelines, and resolve every panel's prepared draw. Runs
    /// once per export; [`Self::paint_export`] renders from what it returns.
    /// Synchronous, so the browser error scopes it pushes are still open
    /// when the first paint pops them.
    pub(crate) fn prepare_export(
        &mut self,
        chart: &Chart,
        series: &[SeriesConfig],
        scale: f32,
        scope: ExportScope,
    ) -> Result<ExportFrame> {
        #[cfg(target_arch = "wasm32")]
        let browser_scopes = BrowserErrorScopes::push(&self.device, "figgy export target")?;

        // The rules an export of a held chart has always checked, read from
        // the model (#160). The full series rules run per panel where the
        // chart is registered; a document's subplots and line offsets are
        // the expansion's.
        for config in series {
            validate_contour_level_count(config.series_id.as_str(), &config.render_type)?;
            validate_line_fill(&self.pool, series.iter(), config)?;
            chart
                .config()
                .validate_series_on_polar(config)
                .map_err(|reason| FiggyError::InvalidSeriesConfig {
                    series_id: config.series_id.clone(),
                    reason: reason.to_string(),
                })?;
        }
        chart.config().validate_radar(series).map_err(|reason| {
            FiggyError::InvalidSeriesConfig {
                series_id: series
                    .iter()
                    .find(|s| s.render_type.radar().is_some())
                    .map_or_else(String::new, |s| s.series_id.clone()),
                reason: reason.to_string(),
            }
        })?;
        chart
            .config()
            .validate_view3d_series(series)
            .map_err(|reason| FiggyError::InvalidSeriesConfig {
                series_id: series
                    .first()
                    .map_or_else(String::new, |s| s.series_id.clone()),
                reason: reason.into(),
            })?;
        let scale = clamp_export_scale(scale);
        // A document export covers every area; a panel export the parent's.
        let orig = match scope {
            ExportScope::Document => chart.config().document_area(),
            ExportScope::Panel => chart.config().chart_area.0,
        };
        let w = ((orig.width as f32) * scale).round().max(1.0) as u32;
        let h = ((orig.height as f32) * scale).round().max(1.0) as u32;
        validate_texture_extent(self.caps, "figgy export target dimension", w, h)?;
        let export_format = wgpu::TextureFormat::Rgba8Unorm;
        let mut export_sample_count = preferred_msaa_sample_count(self.caps, export_format);
        validate_target_sample_count(self.caps, export_format, export_sample_count)?;
        let export_features = export_format.guaranteed_format_features(self.caps.features);
        if !export_features
            .allowed_usages
            .contains(wgpu::TextureUsages::COPY_SRC)
        {
            return Err(FiggyError::UnsupportedSurfaceFormat {
                format: export_format,
                reason: "figgy export target is not guaranteed to support COPY_SRC".into(),
            });
        }

        // 1) Scaled config placed at the origin of a (0,0,w,h) target → temp
        // chart. Without subplots the chart area is the whole target. A panel
        // export places the parent's area at the origin.
        let target = Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        };
        let scaled_config = match scope {
            ExportScope::Document => place_document(chart.config(), scale, target),
            ExportScope::Panel => {
                let mut panel = chart.config().scaled(scale);
                let parent = panel.chart_area.0;
                panel.translate_areas(-i64::from(parent.x), -i64::from(parent.y));
                panel.chart_area = crate::layout::ChartArea(target);
                panel
            }
        };
        let panel_rect = scaled_config.chart_area.0;
        let scaled_chart = Chart::new(scaled_config);

        // 2) Temp ChartView with scaled axis textures, at the export scale.
        let view = self.create_chart_view_at(&scaled_chart, panel_rect, scale)?;

        // 3) Series styles — line_width also scales (extracted from render_type).
        let scaled_styles: Vec<ChartStyle> = series
            .iter()
            .map(|cfg| self.create_style_for_series_scaled_inner(cfg, scale))
            .collect::<Result<_>>()?;
        let series_objs: Vec<Series<'_>> = series
            .iter()
            .zip(scaled_styles.iter())
            .map(|(cfg, st)| Series {
                config: cfg,
                style: st,
            })
            .collect();
        let document = [ChartDrawItem {
            view: &view,
            chart_config: scaled_chart.config(),
            series: &series_objs,
        }];
        // One expansion path with `prepare`: the layered document becomes one
        // item per panel. A panel export keeps the base panel only.
        let storage = PanelDrawStorage::expand(&document)?;
        let mut items = storage.items(&document)?;
        let mut group_sizes = storage.group_sizes(&document)?;
        if scope == ExportScope::Panel {
            items.truncate(1);
            group_sizes = vec![1];
        }
        let items = items.as_slice();
        let mut contour_spacings = Vec::with_capacity(items.len());
        for item in items {
            let mut per_series = Vec::with_capacity(item.series.len());
            for series in item.series {
                let contour_enabled = effective_series_primitives(
                    &item.chart_config.draw_style,
                    &series.config.render_type,
                )
                .contour;
                per_series.push(preflight_contour_label_spacing(
                    series.config.series_id.as_str(),
                    &series.config.render_type,
                    contour_enabled,
                    scale,
                )?);
            }
            contour_spacings.push(per_series);
        }

        // 3.5) Per-item prepare phase (transform uniform write + arc-prefix
        // dispatch) — submitted before the render pass below, so queue order
        // sequences them.
        // 4) Offscreen target.
        let target_desc = wgpu::TextureDescriptor {
            label: Some("figgy export target"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: export_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        // Charged for the length of the export, so a concurrent pool growth
        // sees the export's bytes in its budget rather than after the fact.
        let target_tex = TrackedTexture::new(
            &self.gpu_ledger,
            GpuResourceKind::ExportTarget,
            create_texture_checked(&self.device, &target_desc, "figgy export target")?,
        );
        let msaa_export_target = match create_msaa_target(
            &self.device,
            &self.gpu_ledger,
            self.caps,
            "figgy export msaa target",
            w,
            h,
            export_format,
            export_sample_count,
        ) {
            Ok(target) => target,
            Err(_) if export_sample_count > 1 => {
                export_sample_count = 1;
                None
            }
            Err(e) => return Err(e),
        };
        // Per-call set against the export format. The same prepare-phase
        // ensure as `paint` fills the styled cache, so styled variants
        // compile only when this export actually draws with them.
        let mut export_target_pipelines = create_target_pipelines(
            &self.device,
            &self.texture_bgl,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            export_format,
            export_sample_count,
        );
        self.ensure_contour_label_pipelines(items);
        export_target_pipelines.ensure_precise_variants_for_items(
            &self.device,
            &self.transform_bgl,
            &self.style_bgl,
            &self.per_point_style_map_bgl,
            &self.point_value_map_bgl,
            [&self.surface_bgl, &self.view3d_value_map_bgl],
            &self.data_selection_bgl,
            &self.field_bgl,
            &self.vector_bgl,
            &self.star_data_bgl,
            &self.category_bar_data_bgl,
            &self.parallel_bgl,
            self.contour_label_pipelines.as_ref(),
            export_format,
            items,
        );
        export_target_pipelines.ensure_surface_pipelines(
            &self.device,
            &self.transform_bgl,
            &self.style_bgl,
            &self.field_bgl,
            &self.surface_bgl,
            export_format,
            items,
        );
        export_target_pipelines.ensure_ternary_region_pipelines(
            &self.device,
            &self.transform_bgl,
            &self.region_bgl,
            export_format,
            items,
        );
        export_target_pipelines.ensure_styles_for_items(
            &self.device,
            &self.queue,
            &self.transform_bgl,
            &self.style_bgl,
            &self.star_data_bgl,
            export_format,
            items,
        );
        // Export renders immediately rather than handing out a token, so the
        // receipts have nothing to be held for.
        let _ = self.ensure_field_scratches(items)?;
        self.ensure_value_map_scratches(items, scale)?;
        let _ = self.ensure_vector_scratches(items, scale)?;
        let (prepared_contours, _) =
            self.ensure_contour_scratches(items, scale, &contour_spacings)?;
        let _ = self.ensure_mesh_scratches(items, scale)?;
        let prepared_arcs = self.prepare_arc_items(items)?;
        let (mut prepared_items, _) = self.resolve_prepared_items(
            items,
            &prepared_arcs,
            &prepared_contours,
            &export_target_pipelines,
        )?;
        lift_annotation_overlays(&mut prepared_items, &group_sizes);
        self.attach_export_composites(
            items,
            &mut prepared_items,
            export_format,
            export_sample_count,
        );
        let PanelDrawStorage {
            configs: mut panel_configs,
            ..
        } = storage;
        // A panel export draws the base panel only: its panels are its one
        // item's, so no pass indexes `items` past the end (#152).
        if scope == ExportScope::Panel {
            panel_configs.truncate(1);
        }
        Ok(ExportFrame {
            size: (w, h),
            logical: (orig.width, orig.height),
            scale,
            chart: scaled_chart,
            panel_configs,
            items: prepared_items,
            target: target_tex,
            msaa: msaa_export_target,
            _view: view,
            _styles: scaled_styles,
            _pipelines: export_target_pipelines,
            #[cfg(target_arch = "wasm32")]
            browser_scopes: Some(browser_scopes),
        })
    }

    /// Give every 3D panel of an export its composite: the panel-sized
    /// colour and depth-stencil targets in the export's format and sample
    /// count, the surfaces drawn into them with the depth test, blitted in
    /// the data slot as the screen blits its resident composite. Transient:
    /// charged for the export and dropped with it. Flat panels draw
    /// directly, as before.
    fn attach_export_composites(
        &self,
        inputs: &[ChartDrawItem<'_>],
        prepared: &mut [PreparedItem],
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) {
        let max = self.caps.max_texture_dimension_2d;
        let mut encoder: Option<wgpu::CommandEncoder> = None;
        for (input, item) in inputs.iter().zip(prepared.iter_mut()) {
            let panel = item.view.panel_rect;
            let (w, h) = (panel.width, panel.height);
            let needs_attachment = input.chart_config.view_3d().is_some()
                || super::prepare::panel_stencil(input.chart_config, input.series);
            if !needs_attachment || item.series.is_empty() || w == 0 || h == 0 || w > max || h > max
            {
                continue;
            }
            let (Some(data_x), Some(data_y)) = (
                item.data_area.x.checked_sub(panel.x),
                item.data_area.y.checked_sub(panel.y),
            ) else {
                continue;
            };
            if data_x >= w || data_y >= h {
                continue;
            }
            let target = |samples: u32, usage: wgpu::TextureUsages| {
                TrackedTexture::new(
                    &self.gpu_ledger,
                    GpuResourceKind::ExportTarget,
                    self.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("figgy export panel composite"),
                        size: wgpu::Extent3d {
                            width: w,
                            height: h,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: samples,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage,
                        view_formats: &[],
                    }),
                )
            };
            let texture = target(
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            );
            let msaa = (sample_count > 1)
                .then(|| target(sample_count, wgpu::TextureUsages::RENDER_ATTACHMENT));
            let depth = super::prepare::create_composite_depth(
                &self.device,
                &self.gpu_ledger,
                (w, h),
                sample_count,
            );
            let view = texture.create_view(&Default::default());
            let msaa_view = msaa
                .as_ref()
                .map(|texture| texture.create_view(&Default::default()));
            let depth_view = depth.create_view(&Default::default());
            let series_list: Vec<_> = item.series.iter().map(PreparedSeries::layers).collect();
            let own = item.own_piece_layers();
            let encoder = encoder.get_or_insert_with(|| {
                self.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("figgy export panel composite"),
                    })
            });
            super::prepare::draw_panel_composite(
                encoder,
                super::prepare::PanelCompositeTargets {
                    color: msaa_view.as_ref().unwrap_or(&view),
                    resolve: msaa_view.as_ref().map(|_| &view),
                    depth: Some(&depth_view),
                },
                (w, h),
                &item
                    .data_pieces
                    .relative_to(panel.x, panel.y)
                    .unwrap_or_else(|| {
                        super::composite::DataPieces::whole(Rect {
                            x: data_x,
                            y: data_y,
                            width: item.data_area.width,
                            height: item.data_area.height,
                        })
                    }),
                super::composite::PieceSeries::of(&series_list, &own),
            );
            let bind_group = data_render::create_texture_bind_group(
                &self.device,
                &self.texture_bgl,
                &view,
                &self.sampler,
            );
            item.resident_display = Some(super::frame::ResidentDisplayPacket {
                view: 0,
                serial: 0,
                bind_group,
                _charge: texture.shared_charge(),
            });
            // The multisampled and depth targets are done once the encoder
            // is submitted; the resolved one lives as long as the packet.
            drop((msaa, depth));
        }
        if let Some(encoder) = encoder {
            self.queue.submit([encoder.finish()]);
        }
    }

    /// The second half of an export: render `panels` of `frame` (a range of
    /// its panel indices, all of them for a full export) into its target,
    /// cleared to `clear`, and read the target back as straight-alpha RGBA.
    /// With `chrome` false only the data slot of each panel is drawn: the
    /// fills, bodies, picked highlights and value labels, without the grid,
    /// decoration and annotation textures. The PNG export paints every panel
    /// with the chrome; the vector export paints each panel group's data.
    pub(crate) async fn paint_export(
        &self,
        frame: &mut ExportFrame,
        panels: std::ops::Range<usize>,
        chrome: bool,
        clear: crate::color::Color,
    ) -> Result<RasterImage> {
        let mut job = self.paint_export_draw(frame, panels, chrome, clear)?;
        #[cfg(target_arch = "wasm32")]
        if let Some(scopes) = job.draw_scopes.take() {
            scopes.request_all().finish().await?;
        }
        while let Some(mut chunk) = job.next_chunk(self)? {
            #[cfg(target_arch = "wasm32")]
            if let Some(scopes) = chunk.scopes.take() {
                scopes.request_all().finish().await?;
            }
            // On native the bundle's poll thread completes the map; in the
            // browser the device polls itself.
            chunk.readback.wait().await.map_err(export_readback_error)?;
            job.consume(chunk)?;
        }
        Ok(job.into_image())
    }

    /// [`Self::paint_export`], polling the device on this thread for each
    /// chunk's map.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn paint_export_blocking(
        &self,
        frame: &mut ExportFrame,
        panels: std::ops::Range<usize>,
        chrome: bool,
        clear: crate::color::Color,
    ) -> Result<RasterImage> {
        let mut job = self.paint_export_draw(frame, panels, chrome, clear)?;
        while let Some(mut chunk) = job.next_chunk(self)? {
            chunk
                .readback
                .wait_blocking()
                .map_err(export_readback_error)?;
            job.consume(chunk)?;
        }
        Ok(job.into_image())
    }

    /// The draw half of [`Self::paint_export`]: render the panels into the
    /// target and allocate the readback, returning the job that reads the
    /// target back chunk by chunk. Everything but the waits is here or in
    /// [`ExportReadback`], so the waiting drivers share it.
    fn paint_export_draw(
        &self,
        frame: &mut ExportFrame,
        panels: std::ops::Range<usize>,
        chrome: bool,
        clear: crate::color::Color,
    ) -> Result<ExportReadback> {
        #[cfg(target_arch = "wasm32")]
        let browser_scopes = match frame.browser_scopes.take() {
            Some(scopes) => scopes,
            None => BrowserErrorScopes::push(&self.device, "figgy export target")?,
        };
        let (w, h) = frame.size;
        let target_tex = &frame.target;
        let target_view = target_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let (render_view, resolve_target, store) = match frame.msaa.as_ref() {
            Some(msaa) => (&msaa.view, Some(&target_view), wgpu::StoreOp::Discard),
            None => (&target_view, None, wgpu::StoreOp::Store),
        };
        let prepared_items = &frame.items[panels];

        // 5) Readback buffer. Allocate at most the hardware limit and read rows
        // sequentially if the full image would exceed it.
        let unpadded_bpr_u64 = u64::from(w) * 4;
        let align = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let padded_bpr_u64 = unpadded_bpr_u64.div_ceil(align) * align;
        if padded_bpr_u64 > u64::from(u32::MAX) {
            return Err(FiggyError::GpuResourceLimit {
                resource: "figgy export bytes_per_row",
                requested: padded_bpr_u64,
                limit: u64::from(u32::MAX),
            });
        }
        let unpadded_bpr = unpadded_bpr_u64 as u32;
        let padded_bpr = padded_bpr_u64 as u32;
        let max_rows = (self.caps.max_buffer_size / padded_bpr_u64).min(u64::from(h));
        if max_rows == 0 {
            return Err(FiggyError::GpuResourceLimit {
                resource: "figgy export readback row",
                requested: padded_bpr_u64,
                limit: self.caps.max_buffer_size,
            });
        }
        let mut rows_per_chunk = max_rows.min(u64::from(u32::MAX)) as u32;
        let readback = loop {
            let size = padded_bpr_u64 * u64::from(rows_per_chunk);
            validate_buffer_size(self.caps, "figgy export readback buffer", size)?;
            let desc = wgpu::BufferDescriptor {
                label: Some("figgy export readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            };
            match create_buffer_checked(&self.device, &desc, "figgy export readback buffer") {
                Ok(buffer) => {
                    break TrackedBuffer::new(&self.gpu_ledger, GpuResourceKind::Readback, buffer);
                }
                Err(e) if rows_per_chunk > 1 => {
                    rows_per_chunk = (rows_per_chunk / 2).max(1);
                    let _ = e;
                }
                Err(e) => return Err(e),
            }
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy export encoder"),
            });

        // 6) Render pass — configured clear + a single paint.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("figgy export pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: render_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear.r as f64,
                            g: clear.g as f64,
                            b: clear.b as f64,
                            a: clear.a as f64,
                        }),
                        store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.paint_prepared_items(&mut pass, (w, h), prepared_items, chrome)?;
        }
        self.queue.submit(std::iter::once(encoder.finish()));

        // 7) Texture → readback buffer in row chunks.
        let rgba_len = u64::from(w)
            .checked_mul(u64::from(h))
            .and_then(|px| px.checked_mul(4))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or(FiggyError::GpuResourceLimit {
                resource: "figgy export rgba output",
                requested: u64::from(w).saturating_mul(u64::from(h)).saturating_mul(4),
                limit: usize::MAX as u64,
            })?;
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(rgba_len)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "figgy export rgba output",
                reason: error.to_string(),
            })?;
        rgba.resize(rgba_len, 0);
        Ok(ExportReadback {
            target: wgpu::Texture::clone(target_tex),
            readback,
            size: (w, h),
            unpadded_bpr,
            padded_bpr,
            rows_per_chunk,
            rgba,
            y0: 0,
            #[cfg(target_arch = "wasm32")]
            draw_scopes: Some(browser_scopes),
        })
    }
}

/// The read half of an export: the target, copied into the readback buffer
/// and read back one chunk of rows at a time, written into the straight-
/// alpha RGBA image. The waits between a chunk's copy and its read are the
/// driver's ([`Renderer::paint_export`] or its `_blocking` twin); every
/// other step is here, once.
pub(crate) struct ExportReadback {
    target: wgpu::Texture,
    readback: TrackedBuffer,
    size: (u32, u32),
    unpadded_bpr: u32,
    padded_bpr: u32,
    rows_per_chunk: u32,
    rgba: Vec<u8>,
    /// The first row not yet read back.
    y0: u32,
    /// The draw's browser error scopes, finished before the first chunk.
    #[cfg(target_arch = "wasm32")]
    draw_scopes: Option<BrowserErrorScopes>,
}

/// One chunk's copy, submitted, with its map requested.
pub(crate) struct ExportChunk {
    readback: GpuReadback,
    rows: u32,
    /// The copy's browser error scopes, finished before the chunk's wait.
    #[cfg(target_arch = "wasm32")]
    scopes: Option<BrowserErrorScopes>,
}

impl ExportReadback {
    /// Copy the next chunk of rows into the readback buffer and request its
    /// map; `None` once every row has been read.
    fn next_chunk(&mut self, renderer: &Renderer) -> Result<Option<ExportChunk>> {
        let (w, h) = self.size;
        if self.y0 >= h {
            return Ok(None);
        }
        #[cfg(target_arch = "wasm32")]
        let scopes = BrowserErrorScopes::push(&renderer.device, "figgy export readback")?;

        let rows = self.rows_per_chunk.min(h - self.y0);
        let chunk_size = u64::from(self.padded_bpr) * u64::from(rows);
        let mut copy_encoder =
            renderer
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy export copy encoder"),
                });
        copy_encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: self.y0,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bpr),
                    rows_per_image: Some(rows),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: rows,
                depth_or_array_layers: 1,
            },
        );
        renderer
            .queue
            .submit(std::iter::once(copy_encoder.finish()));
        let readback = GpuReadback::map(
            &renderer.device,
            &renderer.health,
            &self.readback,
            Some(chunk_size),
        );
        Ok(Some(ExportChunk {
            readback,
            rows,
            #[cfg(target_arch = "wasm32")]
            scopes: Some(scopes),
        }))
    }

    /// After the chunk's wait: write its rows into the image, un-
    /// premultiplied, and move to the next chunk.
    fn consume(&mut self, mut chunk: ExportChunk) -> Result<()> {
        let w = self.size.0;
        let (y0, rows) = (self.y0, chunk.rows);
        let (unpadded_bpr, padded_bpr) = (self.unpadded_bpr, self.padded_bpr);
        let rgba = &mut self.rgba;
        let bgra = false;
        chunk
            .readback
            .finish(|mapped| {
                for local_y in 0..rows {
                    let src_off = (local_y * padded_bpr) as usize;
                    let dst_off = ((y0 + local_y) * unpadded_bpr) as usize;
                    let row = &mapped[src_off..src_off + unpadded_bpr as usize];
                    for i in 0..(w as usize) {
                        let p = i * 4;
                        let (b0, b1, b2, b3) = (row[p], row[p + 1], row[p + 2], row[p + 3]);
                        let (r, g, b, a) = if bgra {
                            (b2, b1, b0, b3)
                        } else {
                            (b0, b1, b2, b3)
                        };
                        let (or, og, ob) = if a == 0 || a == 255 {
                            (r, g, b)
                        } else {
                            let a_f = a as f32 / 255.0;
                            (
                                ((r as f32 / a_f).round().clamp(0.0, 255.0)) as u8,
                                ((g as f32 / a_f).round().clamp(0.0, 255.0)) as u8,
                                ((b as f32 / a_f).round().clamp(0.0, 255.0)) as u8,
                            )
                        };
                        rgba[dst_off + p] = or;
                        rgba[dst_off + p + 1] = og;
                        rgba[dst_off + p + 2] = ob;
                        rgba[dst_off + p + 3] = a;
                    }
                }
            })
            .map_err(export_readback_error)?;
        self.y0 += rows;
        Ok(())
    }

    fn into_image(self) -> RasterImage {
        RasterImage {
            width: self.size.0,
            height: self.size.1,
            rgba: self.rgba,
        }
    }
}

/// A vector export renders the whole document.
fn check_vector_scope(options: ExportOptions) -> Result<()> {
    if options.scope != ExportScope::Document {
        return Err(FiggyError::InvalidConfig {
            field: "ExportOptions.scope",
            reason: "a vector export renders the whole document",
        });
    }
    Ok(())
}

/// A readback failure as an export has always reported it.
fn export_readback_error(error: ReadbackError) -> FiggyError {
    let reason = match error {
        ReadbackError::Poll(error) | ReadbackError::Health(error) => return error,
        ReadbackError::Canceled(error) => format!("map_async sender dropped: {error}"),
        ReadbackError::Map(error) => format!("map_async: {error:?}"),
        ReadbackError::Range(error) => format!("get_mapped_range: {error:?}"),
    };
    FiggyError::GpuResourceAllocationFailed {
        resource: "figgy export readback mapping",
        reason,
    }
}
