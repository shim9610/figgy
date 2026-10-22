//! PNG export of resident and streamed charts.

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
    /// The example `<figgy-chart>` facade supplies that gate and rejects competing
    /// application calls; raw FiggyChart callers must manage it themselves.
    /// Automatic contour placement validates `spacing_px * scale` after the
    /// export scale is clamped. Explicit placement does not multiply spacing.
    /// Scale-product overflow or an atlas beyond the adapter texture limit
    /// rejects the export before contour GPU state changes.
    pub async fn export_png(&mut self, scale: f32) -> Result<js_sys::Uint8Array, JsValue> {
        let export_chart = Chart::new(self.chart_config().clone());
        let series = self.chart_series().to_vec();
        let bytes = self
            .renderer
            .export_document_png_bytes_with_clear_async(
                &export_chart,
                &series,
                scale,
                self.clear_color,
            )
            .await
            .map_err(js_err)?;
        Ok(js_sys::Uint8Array::from(bytes.as_slice()))
    }

    /// Begin an independent replay of the completed stream. The facade
    /// supplies only renderer-requested ranges between these calls.
    pub fn begin_stream_export(
        &mut self,
        scale: f32,
        max_primitives_per_chunk: f64,
    ) -> Result<StreamingOperationHandle, JsValue> {
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
