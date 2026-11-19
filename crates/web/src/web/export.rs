//! PNG export of resident and streamed charts, and the SVG and PDF export
//! of resident charts.

use super::*;

#[wasm_bindgen]
impl FiggyChart {
    /// Export the document as PNG bytes at `scale ×` resolution: the base
    /// layer and every overlay layer composited into one image, exactly
    /// as the canvas shows them. A document without overlays exports the
    /// same bytes as the per-panel renderer export.
    /// JS: `const png = await chart.export_png(2.0);`
    /// Holds a mutable borrow of this WASM object until the Promise settles.
    /// wasm-bindgen checks borrows; it does not queue concurrent calls.
    /// While awaiting export, the host must prevent every other call on the
    /// same object, including frame(), getters, edits, resize, picking and free().
    /// Overlapping access can throw "recursive use of an object". Release the
    /// host's per-instance operation gate in finally on success or rejection.
    /// The `<figgy-chart>` facade supplies that gate and rejects competing
    /// application calls; raw FiggyChart callers must manage it themselves.
    /// Automatic contour placement validates `spacing_px * scale` after the
    /// export scale is clamped. Explicit placement does not multiply spacing.
    /// Scale-product overflow or an atlas beyond the adapter texture limit
    /// rejects the export before contour GPU state changes.
    ///
    /// A layout waiting for its fit (`layout_state: "needs_fit"`) is fitted
    /// first and the fit is stored, as `frame` does (#357);
    /// `export_png_as_stored` exports the document exactly as stored.
    pub async fn export_png(&mut self, scale: f32) -> Result<js_sys::Uint8Array, JsValue> {
        self.export_png_with(renderer::ExportOptions::document(scale))
            .await
    }

    /// `export_png` of the document exactly as stored (#357): a layout
    /// waiting for its fit is exported with the margins it holds and stays
    /// `"needs_fit"`. The same borrow and gate rules as `export_png`.
    /// JS: `const png = await chart.export_png_as_stored(2.0);`
    pub async fn export_png_as_stored(
        &mut self,
        scale: f32,
    ) -> Result<js_sys::Uint8Array, JsValue> {
        self.export_png_with(renderer::ExportOptions::document(scale).as_stored())
            .await
    }

    /// Export the document as an SVG document at `scale ×` resolution: the
    /// decorations, annotations and text as vector content (glyphs as
    /// outlines) over each panel group's data embedded as a PNG image, with
    /// the clear colour as its background.
    /// JS: `const svg = await chart.export_svg(2.0);`
    /// The same borrow and gate rules as [`Self::export_png`]. A chart with a
    /// streamed source is rejected; the `<figgy-chart>` facade rejects it
    /// before calling.
    /// A layout waiting for its fit is fitted first, as for `export_png`.
    pub async fn export_svg(&mut self, scale: f32) -> Result<String, JsValue> {
        self.export_svg_with(renderer::ExportOptions::document(scale))
            .await
    }

    /// `export_svg` of the document exactly as stored (#357), as
    /// `export_png_as_stored` exports it.
    pub async fn export_svg_as_stored(&mut self, scale: f32) -> Result<String, JsValue> {
        self.export_svg_with(renderer::ExportOptions::document(scale).as_stored())
            .await
    }

    /// [`Self::export_svg`] as one-page PDF bytes.
    /// JS: `const pdf = await chart.export_pdf(2.0);`
    pub async fn export_pdf(&mut self, scale: f32) -> Result<js_sys::Uint8Array, JsValue> {
        self.export_pdf_with(renderer::ExportOptions::document(scale))
            .await
    }

    /// `export_pdf` of the document exactly as stored (#357), as
    /// `export_png_as_stored` exports it.
    pub async fn export_pdf_as_stored(
        &mut self,
        scale: f32,
    ) -> Result<js_sys::Uint8Array, JsValue> {
        self.export_pdf_with(renderer::ExportOptions::document(scale).as_stored())
            .await
    }

    /// Begin an independent replay of the completed stream. The facade
    /// supplies only renderer-requested ranges between these calls.
    pub fn begin_stream_export(
        &mut self,
        scale: f32,
        max_primitives_per_chunk: f64,
    ) -> Result<StreamingOperationHandle, JsValue> {
        self.refuse_streaming()?;
        let max_chunk = js_safe_u64(max_primitives_per_chunk, "max_primitives_per_chunk")?;
        let operation = self
            .renderer
            .begin_stream_export(self.chart_id, scale, self.clear_color, max_chunk)
            .map_err(js_err)?;
        Ok(StreamingOperationHandle {
            operation,
            revision: self
                .renderer
                .stream_status(self.chart_id)
                .map_err(js_err)?
                .revision
                .to_string(),
            counts: Cell::new((0.0, 0.0)),
        })
    }

    pub async fn finish_stream_export(
        &mut self,
        handle: &StreamingOperationHandle,
    ) -> Result<js_sys::Uint8Array, JsValue> {
        let image = self
            .renderer
            .finish_stream_export(handle.operation)
            .await
            .map_err(js_err)?;
        let bytes = renderer::encode_png(&image).map_err(js_err)?;
        Ok(js_sys::Uint8Array::from(bytes.as_slice()))
    }
}

impl FiggyChart {
    /// The PNG export of the shown chart with `options` over the clear
    /// colour.
    async fn export_png_with(
        &mut self,
        options: renderer::ExportOptions,
    ) -> Result<js_sys::Uint8Array, JsValue> {
        let source = self.export_source();
        let bytes = self
            .renderer
            .export_png(source, options.with_clear(self.clear_color))
            .await
            .map_err(js_err)?;
        Ok(js_sys::Uint8Array::from(bytes.as_slice()))
    }

    /// The SVG export of the shown chart with `options` over the clear
    /// colour.
    async fn export_svg_with(
        &mut self,
        options: renderer::ExportOptions,
    ) -> Result<String, JsValue> {
        let source = self.export_source();
        self.renderer
            .export_svg(source, options.with_clear(self.clear_color))
            .await
            .map_err(js_err)
    }

    /// The PDF export of the shown chart with `options` over the clear
    /// colour.
    async fn export_pdf_with(
        &mut self,
        options: renderer::ExportOptions,
    ) -> Result<js_sys::Uint8Array, JsValue> {
        let source = self.export_source();
        let bytes = self
            .renderer
            .export_pdf(source, options.with_clear(self.clear_color))
            .await
            .map_err(js_err)?;
        Ok(js_sys::Uint8Array::from(bytes.as_slice()))
    }

    /// The shown chart as an export source at the screen's mapping: a
    /// monotonic chart with no sources lent (#271).
    fn export_source(&self) -> renderer::ExportSource<'static> {
        match self.monotonic {
            Some(chart) => renderer::ExportSource::Monotonic {
                chart,
                monotonic: &[],
            },
            None => renderer::ExportSource::Registered(self.chart_id),
        }
    }
}
