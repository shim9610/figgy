//! Monotonic-x line series (#271) in the browser: the four calls that
//! take a group's source (`set_monotonic`, `append_monotonic`,
//! `resize_monotonic` and `fit_layout`), `needs_source`,
//! `monotonic_groups` and the exports that take the groups' sources.
//!
//! A host passes a group's samples as typed arrays of one of the six
//! element types (`Float64Array`, `Float32Array`, `Int32Array`,
//! `Int16Array`, `Uint16Array`, `Uint8Array`): an `Array` of them, one per
//! channel, or one interleaved, sample-major array with a `stride` (and the
//! frame columns its channels draw, every column by default). The arrays
//! are borrowed for the call: the renderer asks for the samples it reads
//! (the visible slice and one on each side, or the new samples), and only
//! those are copied into wasm memory, a bounded chunk at a time. Nothing is
//! kept after the call (M1, M3).

use super::*;
use renderer::monotonic::{MonotonicElement, MonotonicRun, MonotonicSource};
use renderer::renderer::MonotonicGroupRef;
use renderer::renderer::MonotonicRange;
use renderer::{MonotonicAppend, MonotonicSourceRef, MonotonicUpdate};
use wasm_bindgen::JsCast;

/// Samples per channel copied from the host's arrays into wasm memory per
/// step of a lend: the transfer chunk, not a limit on the samples a call
/// takes (rule 10).
const CHUNK: usize = 1 << 15;

/// A typed array element type and its JavaScript array.
trait JsElement: MonotonicElement + Default {
    type Array: JsCast + Clone;
    fn length(array: &Self::Array) -> u32;
    /// Copies `array[begin..end]` into `dst` (of that length).
    fn copy(array: &Self::Array, begin: u32, end: u32, dst: &mut [Self]);
}

macro_rules! js_element {
    ($($t:ty => $array:ty),*) => {$(
        impl JsElement for $t {
            type Array = $array;
            fn length(array: &$array) -> u32 {
                array.length()
            }
            fn copy(array: &$array, begin: u32, end: u32, dst: &mut [$t]) {
                array.subarray(begin, end).copy_to(dst);
            }
        }
    )*};
}

js_element!(
    f64 => js_sys::Float64Array,
    f32 => js_sys::Float32Array,
    i32 => js_sys::Int32Array,
    i16 => js_sys::Int16Array,
    u16 => js_sys::Uint16Array,
    u8 => js_sys::Uint8Array
);

/// How the host's arrays hold the channels.
enum Layout<A> {
    /// One array per channel.
    Channels(Vec<A>),
    /// One sample-major array: frame `i` is `values[i * stride ..]`, and
    /// channel `n` reads column `columns[n]`.
    Interleaved {
        values: A,
        stride: usize,
        columns: Vec<usize>,
    },
}

/// A browser host's samples for one call, read from its typed arrays,
/// with their grid.
struct JsSource<T: JsElement> {
    start: f64,
    interval: f64,
    layout: Layout<T::Array>,
    len: usize,
    truncated: bool,
}

fn layout_error(reason: &str) -> JsValue {
    js_err(format!("monotonic data: {reason}"))
}

impl<T: JsElement> JsSource<T> {
    /// `data`, at `start + i × interval`, as a source of element type `T`;
    /// `None` when its arrays hold another type.
    fn new(
        (start, interval): (f64, f64),
        data: &JsValue,
        stride: Option<u32>,
        columns: Option<&[u32]>,
    ) -> Result<Option<Self>, JsValue> {
        if let Some(values) = data.dyn_ref::<T::Array>() {
            let stride = stride
                .filter(|&stride| stride > 0)
                .ok_or_else(|| layout_error("an interleaved array needs a positive stride"))?
                as usize;
            let columns: Vec<usize> = match columns {
                Some(columns) => columns.iter().map(|&c| c as usize).collect(),
                None => (0..stride).collect(),
            };
            if let Some(&column) = columns.iter().find(|&&c| c >= stride) {
                return Err(layout_error(&format!(
                    "column {column} lies outside a frame of {stride} values"
                )));
            }
            let len = T::length(values) as usize / stride;
            return Ok(Some(Self {
                start,
                interval,
                layout: Layout::Interleaved {
                    values: values.clone(),
                    stride,
                    columns,
                },
                len,
                truncated: false,
            }));
        }
        let Some(list) = data.dyn_ref::<js_sys::Array>() else {
            return Ok(None);
        };
        let first = list.get(0);
        if !first.is_instance_of::<T::Array>() {
            return Ok(None);
        }
        if stride.is_some() || columns.is_some() {
            return Err(layout_error(
                "an array of channels takes no stride or columns",
            ));
        }
        let mut arrays = Vec::new();
        arrays
            .try_reserve_exact(list.length() as usize)
            .map_err(|_| layout_error("the channel list could not be allocated"))?;
        for item in list.iter() {
            let array = item
                .dyn_into::<T::Array>()
                .map_err(|_| layout_error("every channel is a typed array of one element type"))?;
            arrays.push(array);
        }
        let lengths = || arrays.iter().map(|a| T::length(a) as usize);
        let len = lengths().min().unwrap_or(0);
        let truncated = lengths().any(|l| l != len);
        Ok(Some(Self {
            start,
            interval,
            layout: Layout::Channels(arrays),
            len,
            truncated,
        }))
    }
}

impl<T: JsElement> MonotonicSource for JsSource<T> {
    type Element = T;

    fn start(&self) -> f64 {
        self.start
    }

    fn interval(&self) -> f64 {
        self.interval
    }

    fn len(&self) -> usize {
        self.len
    }

    fn channel_count(&self) -> usize {
        match &self.layout {
            Layout::Channels(arrays) => arrays.len(),
            Layout::Interleaved { columns, .. } => columns.len(),
        }
    }

    fn truncated(&self) -> bool {
        self.truncated
    }

    fn lend(&self, range: std::ops::Range<usize>, visit: &mut dyn FnMut(MonotonicRun<'_, T>)) {
        let end = range.end.min(self.len);
        let mut at = range.start;
        if at >= end {
            return;
        }
        let index = |i: usize| u32::try_from(i).ok();
        match &self.layout {
            Layout::Channels(arrays) => {
                let width = CHUNK.min(end - at);
                // One chunk per channel; a source that cannot be copied
                // lends nothing, and the call reduces what it was lent.
                let mut buffer = Vec::new();
                if buffer.try_reserve_exact(width * arrays.len()).is_err() {
                    return;
                }
                buffer.resize(width * arrays.len(), T::default());
                while at < end {
                    let n = width.min(end - at);
                    let (Some(from), Some(to)) = (index(at), index(at + n)) else {
                        return;
                    };
                    for (c, array) in arrays.iter().enumerate() {
                        T::copy(array, from, to, &mut buffer[c * width..c * width + n]);
                    }
                    let slices: Vec<&[T]> = (0..arrays.len())
                        .map(|c| &buffer[c * width..c * width + n])
                        .collect();
                    if let Some(run) = MonotonicRun::slices(at, n, &slices, 0) {
                        visit(run);
                    }
                    at += n;
                }
            }
            Layout::Interleaved {
                values,
                stride,
                columns,
            } => {
                let frames = CHUNK.min(end - at);
                let mut buffer = Vec::new();
                if buffer.try_reserve_exact(frames * stride).is_err() {
                    return;
                }
                buffer.resize(frames * stride, T::default());
                while at < end {
                    let n = frames.min(end - at);
                    let (Some(from), Some(to)) = (index(at * stride), index((at + n) * stride))
                    else {
                        return;
                    };
                    let chunk = &mut buffer[..n * stride];
                    T::copy(values, from, to, chunk);
                    if let Some(run) = MonotonicRun::interleaved(at, n, chunk, *stride, columns, 0)
                    {
                        visit(run);
                    }
                    at += n;
                }
            }
        }
    }
}

/// A source of any element type, owned for one call.
enum AnySource {
    F64(JsSource<f64>),
    F32(JsSource<f32>),
    I32(JsSource<i32>),
    I16(JsSource<i16>),
    U16(JsSource<u16>),
    U8(JsSource<u8>),
}

impl AnySource {
    /// `data`, at `start + i × interval` (`grid`), as a source of
    /// whichever element type its arrays hold.
    fn new(
        grid: (f64, f64),
        data: &JsValue,
        stride: Option<u32>,
        columns: Option<&[u32]>,
    ) -> Result<Self, JsValue> {
        macro_rules! try_type {
            ($t:ty, $variant:ident) => {
                if let Some(source) = JsSource::<$t>::new(grid, data, stride, columns)? {
                    return Ok(Self::$variant(source));
                }
            };
        }
        try_type!(f64, F64);
        try_type!(f32, F32);
        try_type!(i32, I32);
        try_type!(i16, I16);
        try_type!(u16, U16);
        try_type!(u8, U8);
        Err(layout_error(
            "pass a Float64Array, Float32Array, Int32Array, Int16Array, Uint16Array or \
             Uint8Array with a stride, or an array of them, one per channel",
        ))
    }

    fn as_ref(&self) -> MonotonicSourceRef<'_> {
        match self {
            Self::F64(s) => MonotonicSourceRef::new(s),
            Self::F32(s) => MonotonicSourceRef::new(s),
            Self::I32(s) => MonotonicSourceRef::new(s),
            Self::I16(s) => MonotonicSourceRef::new(s),
            Self::U16(s) => MonotonicSourceRef::new(s),
            Self::U8(s) => MonotonicSourceRef::new(s),
        }
    }
}

/// Runs `$call` with `$source` bound to the typed source in `$any`.
macro_rules! with_source {
    ($any:expr, $source:ident => $call:expr) => {
        match &$any {
            AnySource::F64($source) => $call,
            AnySource::F32($source) => $call,
            AnySource::I32($source) => $call,
            AnySource::I16($source) => $call,
            AnySource::U16($source) => $call,
            AnySource::U8($source) => $call,
        }
    };
}

fn update_json(update: &MonotonicUpdate) -> Result<String, JsValue> {
    serde_json::to_string(&serde_json::json!({
        "len": update.len,
        "truncated": update.truncated,
        "dropped": update.dropped,
        "x_range": [update.x_range.0, update.x_range.1],
        "pixel_offset": update.pixel_offset,
    }))
    .map_err(js_err)
}

/// One group of the `sources` a call takes: `{ group, start, interval,
/// data, stride?, columns? }`.
struct LentGroup {
    group: String,
    source: AnySource,
}

fn field(object: &JsValue, name: &str) -> Result<JsValue, JsValue> {
    js_sys::Reflect::get(object, &JsValue::from_str(name))
}

/// `entry`, `{ start, interval, data, stride?, columns? }`, as group
/// `group`'s source.
fn source_group(entry: &JsValue, group: String) -> Result<LentGroup, JsValue> {
    let number = |name: &str| -> Result<f64, JsValue> {
        field(entry, name)?
            .as_f64()
            .ok_or_else(|| layout_error(&format!("a monotonic source needs a numeric {name}")))
    };
    let stride = field(entry, "stride")?.as_f64().map(|s| s as u32);
    let columns_value = field(entry, "columns")?;
    let columns: Option<Vec<u32>> = if columns_value.is_undefined() || columns_value.is_null() {
        None
    } else {
        Some(js_sys::Uint32Array::new(&columns_value).to_vec())
    };
    let grid = (number("start")?, number("interval")?);
    let source = AnySource::new(grid, &field(entry, "data")?, stride, columns.as_deref())?;
    Ok(LentGroup { group, source })
}

/// The groups of a call's `sources`, an array of `{ group, start,
/// interval, data, stride?, columns? }`.
fn lent_groups(sources: &js_sys::Array) -> Result<Vec<LentGroup>, JsValue> {
    let mut groups = Vec::new();
    groups
        .try_reserve_exact(sources.length() as usize)
        .map_err(|_| layout_error("the monotonic sources could not be allocated"))?;
    for entry in sources.iter() {
        let group = field(&entry, "group")?
            .as_string()
            .ok_or_else(|| layout_error("a monotonic source names its group"))?;
        groups.push(source_group(&entry, group)?);
    }
    Ok(groups)
}

/// `value`, `{ x, y, width, height }` in document pixels, as a plot rect;
/// `undefined` or `null` for none.
fn plot_rect(value: &JsValue) -> Result<Option<renderer::layout::Rect>, JsValue> {
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    let number = |name: &str| -> Result<u32, JsValue> {
        field(value, name)?
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0 && *v <= f64::from(u32::MAX))
            .map(|v| v as u32)
            .ok_or_else(|| layout_error(&format!("a plot rect needs a non-negative {name}")))
    };
    Ok(Some(renderer::layout::Rect {
        x: number("x")?,
        y: number("y")?,
        width: number("width")?,
        height: number("height")?,
    }))
}

/// `range`, `[min, max]` or `"autoscale"`, as the range `set_monotonic`
/// sets.
fn monotonic_range(range: &JsValue) -> Result<MonotonicRange, JsValue> {
    if range.as_string().as_deref() == Some("autoscale") {
        return Ok(MonotonicRange::Autoscale);
    }
    let pair = range
        .dyn_ref::<js_sys::Array>()
        .filter(|pair| pair.length() == 2)
        .and_then(|pair| Some((pair.get(0).as_f64()?, pair.get(1).as_f64()?)));
    match pair {
        Some((min, max)) => Ok(MonotonicRange::X { min, max }),
        None => Err(layout_error("the range is [x_min, x_max] or \"autoscale\"")),
    }
}

impl FiggyChart {
    /// Whether the kernel shows a monotonic chart (#271).
    pub(super) fn has_monotonic(&self) -> Result<bool, JsValue> {
        Ok(self.monotonic.is_some())
    }

    /// The monotonic chart the kernel shows; refused when it shows a
    /// general one (`set_monotonic_document` first).
    pub(super) fn monotonic_id(&self) -> Result<renderer::MonotonicChartId, JsValue> {
        self.monotonic.ok_or_else(|| {
            js_err("monotonic calls need a monotonic chart: call set_monotonic_document first")
        })
    }

    /// The calls of the monotonic chart the kernel shows.
    fn monotonic_chart(&mut self) -> Result<renderer::MonotonicChart<'_>, JsValue> {
        let id = self.monotonic_id()?;
        self.renderer.monotonic_chart(id).map_err(js_err)
    }

    /// Refuses a streamed call while the kernel shows a monotonic chart: a
    /// monotonic series is fed by its own calls, never streamed (M5).
    pub(super) fn refuse_streaming(&self) -> Result<(), JsValue> {
        match self.monotonic {
            Some(_) => Err(js_err(
                "a monotonic chart is not streamed: its series are fed by set_monotonic",
            )),
            None => Ok(()),
        }
    }
}

#[wasm_bindgen]
impl FiggyChart {
    /// The full call of monotonic group `group` (#271): every series of
    /// the chart that names the group is reduced in full from `data`, its
    /// samples at `x = start + i × interval`. `data` is an `Array` of typed
    /// arrays, one per channel (leave `stride` and `columns` undefined), or
    /// one interleaved typed array with `stride` values per frame, its
    /// channels the frame `columns` (every column by default). Any of
    /// `Float64Array`, `Float32Array`, `Int32Array`, `Int16Array`,
    /// `Uint16Array` and `Uint8Array`. `range` sets the x range of the
    /// group's axes, the only call that does: `[x_min, x_max]`, or
    /// `"autoscale"` to fit the x range to the samples' span and each y axis
    /// the group draws on to the values every monotonic series on that axis
    /// keeps, this group's and any other's (the general auto-fit refuses the
    /// chart). Returns `{ len, truncated, dropped, x_range, pixel_offset }`
    /// as JSON.
    /// JS: `chart.set_monotonic("daq", 0, 1e-3, [ch0, ch1], "autoscale")`
    pub fn set_monotonic(
        &mut self,
        group: &str,
        start: f64,
        interval: f64,
        data: JsValue,
        range: JsValue,
        stride: Option<u32>,
        columns: Option<Vec<u32>>,
    ) -> Result<String, JsValue> {
        let range = monotonic_range(&range)?;
        let source = AnySource::new((start, interval), &data, stride, columns.as_deref())?;
        let mut chart = self.monotonic_chart()?;
        let update =
            with_source!(source, s => chart.set_monotonic(group, s, range)).map_err(js_err)?;
        self.sync_hitmap()?;
        update_json(&update)
    }

    /// The append call of monotonic group `group` (#271): `data` holds only
    /// the new samples of every channel, in either layout of
    /// `set_monotonic`. With `sliding`, the window keeps its length and the
    /// chart's x range moves by exactly the new samples' span; the host may
    /// drop every sample before the window. Returns the update as JSON.
    /// JS: `chart.append_monotonic("daq", [new0, new1], true)`
    pub fn append_monotonic(
        &mut self,
        group: &str,
        data: JsValue,
        sliding: bool,
        stride: Option<u32>,
        columns: Option<Vec<u32>>,
    ) -> Result<String, JsValue> {
        // The new samples continue the grid of the full call: an append
        // reads neither the source's start nor its interval.
        let source = AnySource::new((f64::NAN, f64::NAN), &data, stride, columns.as_deref())?;
        let mut chart = self.monotonic_chart()?;
        let options = MonotonicAppend { sliding };
        let update =
            with_source!(source, s => chart.append_monotonic(group, s, options)).map_err(js_err)?;
        if sliding {
            // The slide moved the x range in the document.
            self.sync_hitmap()?;
        }
        update_json(&update)
    }

    /// Shows a monotonic chart (#271): `config_json` and `series_json`, a
    /// document and series as `set_config` and `set_series` take them,
    /// registered as a monotonic chart, which alone may hold monotonic line
    /// series. Its x range and plot are written only by `set_monotonic`,
    /// `append_monotonic`, `resize_monotonic` and `fit_layout`; every other
    /// setter edits its document as on any chart and refuses a document
    /// that would move them. Its monotonic series draw nothing until their
    /// group's first `set_monotonic`. A previous monotonic chart is
    /// replaced; `clear_monotonic_document` shows the general chart again.
    pub fn set_monotonic_document(
        &mut self,
        config_json: &str,
        series_json: &str,
    ) -> Result<(), JsValue> {
        let mut config: renderer::Config = serde_json::from_str(config_json).map_err(js_err)?;
        let series: Vec<SeriesConfig> = serde_json::from_str(series_json).map_err(js_err)?;
        config.chart_area = self
            .renderer
            .chart_config(self.chart_id)
            .map_err(js_err)?
            .chart_area
            .clone();
        let id = self
            .renderer
            .register_monotonic_chart(config, series)
            .map_err(js_err)?;
        let area = self
            .renderer
            .monotonic_config(id)
            .map_err(js_err)?
            .chart_area
            .0;
        let shown = self
            .renderer
            .monotonic_chart(id)
            .and_then(|mut chart| chart.resize(area, None, Some(self.surface_size), &[]));
        if let Err(error) = shown {
            self.renderer
                .monotonic_chart(id)
                .and_then(|chart| chart.remove())
                .map_err(js_err)?;
            return Err(js_err(error));
        }
        if let Some(previous) = self.monotonic.replace(id) {
            self.renderer
                .monotonic_chart(previous)
                .and_then(|chart| chart.remove())
                .map_err(js_err)?;
        }
        self.set_selected_annotation(None);
        self.sync_hitmap()?;
        self.view_dirty = true;
        Ok(())
    }

    /// Removes the monotonic chart and shows the general one again (#271).
    pub fn clear_monotonic_document(&mut self) -> Result<(), JsValue> {
        if let Some(id) = self.monotonic.take() {
            self.renderer
                .monotonic_chart(id)
                .and_then(|chart| chart.remove())
                .map_err(js_err)?;
            self.set_selected_annotation(None);
            self.sync_hitmap()?;
            self.view_dirty = true;
        }
        Ok(())
    }

    /// Resizes the canvas surface to `width` × `height` device pixels on a
    /// chart with monotonic series (#271), and the plot to `plot_area`
    /// (`{ x, y, width, height }` in document pixels, as
    /// `take_plot_resize_request` proposes it; `undefined` keeps the
    /// margins): the resize call that takes the groups' sources,
    /// `[{ group, start, interval, data, stride?, columns? }]`, `data` as
    /// `set_monotonic` takes it. Each group whose series the resize maps to
    /// other device pixels is reduced again in this call. Atomic: without a
    /// source a moved group needs, the call is refused with
    /// `MonotonicSourceRequired` and changes nothing; `resize` refuses such
    /// a chart the same way.
    pub fn resize_monotonic(
        &mut self,
        width: u32,
        height: u32,
        sources: js_sys::Array,
        plot_area: JsValue,
    ) -> Result<(), JsValue> {
        let (w, h) = (width.max(1), height.max(1));
        let plot_area = plot_rect(&plot_area)?;
        let groups = lent_groups(&sources)?;
        let refs: Vec<MonotonicGroupRef<'_>> = groups.iter().map(group_ref).collect();
        let area = self.chart_config()?.chart_area.0;
        self.monotonic_chart()?
            .resize(area, plot_area, Some((w, h)), &refs)
            .map_err(js_err)?;
        self.renderer.resize(w, h).map_err(js_err)?;
        self.surface_size = (w, h);
        self.view_dirty = true;
        if plot_area.is_some() {
            self.sync_hitmap()?;
        }
        Ok(())
    }

    /// The plot rect the last plot-area resize gesture on a chart with
    /// monotonic series proposed, as JSON `{ x, y, width, height }` in
    /// document pixels, or `undefined` (#271). Read once: the element emits
    /// it as `figgy-plot-resize`, and the host applies it with
    /// `resize_monotonic(width, height, sources, plotArea)`.
    pub fn take_plot_resize_request(&mut self) -> Result<Option<String>, JsValue> {
        self.plot_resize_request
            .take()
            .map(|area| {
                serde_json::to_string(&serde_json::json!({
                    "x": area.x,
                    "y": area.y,
                    "width": area.width,
                    "height": area.height,
                }))
                .map_err(js_err)
            })
            .transpose()
    }

    /// Fits the chart's layout to its labels and titles and applies it
    /// (`Renderer::fit_layout`, #271). It can move the plot's edges, so it
    /// takes the sources of the chart's monotonic groups as
    /// `resize_monotonic` does (`[]` for a chart without them).
    pub fn fit_layout(&mut self, sources: js_sys::Array) -> Result<(), JsValue> {
        let groups = lent_groups(&sources)?;
        let refs: Vec<MonotonicGroupRef<'_>> = groups.iter().map(group_ref).collect();
        match self.monotonic {
            Some(_) => self.monotonic_chart()?.fit_layout(&refs).map_err(js_err)?,
            None => {
                let config = self.chart_config()?;
                let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
                let laid_out = renderer::layout::auto_layout(config, &measure);
                self.renderer
                    .set_chart_config(self.chart_id, laid_out)
                    .map_err(js_err)?;
            }
        }
        self.sync_hitmap()
    }

    /// The chart's monotonic groups, each once, as a JSON array of names
    /// (#271): the groups whose sources `resize_monotonic`, `fit_layout`
    /// and the monotonic exports take. Empty for a chart without them.
    pub fn monotonic_groups(&self) -> Result<String, JsValue> {
        let mut groups: Vec<&str> = Vec::new();
        for series in self.chart_series()? {
            if let Some(monotonic) = series.monotonic()
                && !groups.contains(&monotonic.group.as_str())
            {
                groups.push(&monotonic.group);
            }
        }
        serde_json::to_string(&groups).map_err(js_err)
    }

    /// The monotonic series that need their group's source before they
    /// can be drawn, as JSON `[{ series_id, group, change }]` (#271):
    /// series whose group was never supplied, which draw nothing until
    /// `set_monotonic` supplies it. A call that moves the mapping takes the
    /// source or is refused, so no other series is listed.
    pub fn needs_source(&self) -> Result<String, JsValue> {
        let needs = match self.monotonic {
            Some(id) => self.renderer.monotonic_needs_source(id).map_err(js_err)?,
            None => Vec::new(),
        };
        let list: Vec<serde_json::Value> = needs
            .iter()
            .map(|need| {
                serde_json::json!({
                    "series_id": need.series_id,
                    "group": need.group,
                    "change": format!("{:?}", need.change),
                })
            })
            .collect();
        serde_json::to_string(&list).map_err(js_err)
    }

    /// [`Self::export_png`] with the sources of the chart's monotonic
    /// groups (#271), for an export at another scale than the screen's
    /// (which maps the plot to other device pixels): `sources` is an array
    /// of `{ group, start, interval, data, stride?, columns? }`, `data` as
    /// `set_monotonic` takes it. Each group is reduced for the export alone.
    /// The same borrow and gate rules as `export_png`.
    pub async fn export_png_monotonic(
        &mut self,
        scale: f32,
        sources: js_sys::Array,
    ) -> Result<js_sys::Uint8Array, JsValue> {
        let chart = self.monotonic_id()?;
        let groups = lent_groups(&sources)?;
        let refs: Vec<MonotonicGroupRef<'_>> = groups.iter().map(group_ref).collect();
        let bytes = self
            .renderer
            .export_png(
                renderer::ExportSource::Monotonic {
                    chart,
                    monotonic: &refs,
                },
                renderer::ExportOptions::document(scale).with_clear(self.clear_color),
            )
            .await
            .map_err(js_err)?;
        Ok(js_sys::Uint8Array::from(bytes.as_slice()))
    }

    /// [`Self::export_svg`] with the sources of the chart's monotonic
    /// groups, as [`Self::export_png_monotonic`] takes them.
    pub async fn export_svg_monotonic(
        &mut self,
        scale: f32,
        sources: js_sys::Array,
    ) -> Result<String, JsValue> {
        let chart = self.monotonic_id()?;
        let groups = lent_groups(&sources)?;
        let refs: Vec<MonotonicGroupRef<'_>> = groups.iter().map(group_ref).collect();
        self.renderer
            .export_svg(
                renderer::ExportSource::Monotonic {
                    chart,
                    monotonic: &refs,
                },
                renderer::ExportOptions::document(scale).with_clear(self.clear_color),
            )
            .await
            .map_err(js_err)
    }

    /// [`Self::export_pdf`] with the sources of the chart's monotonic
    /// groups, as [`Self::export_png_monotonic`] takes them.
    pub async fn export_pdf_monotonic(
        &mut self,
        scale: f32,
        sources: js_sys::Array,
    ) -> Result<js_sys::Uint8Array, JsValue> {
        let chart = self.monotonic_id()?;
        let groups = lent_groups(&sources)?;
        let refs: Vec<MonotonicGroupRef<'_>> = groups.iter().map(group_ref).collect();
        let bytes = self
            .renderer
            .export_pdf(
                renderer::ExportSource::Monotonic {
                    chart,
                    monotonic: &refs,
                },
                renderer::ExportOptions::document(scale).with_clear(self.clear_color),
            )
            .await
            .map_err(js_err)?;
        Ok(js_sys::Uint8Array::from(bytes.as_slice()))
    }
}

fn group_ref(group: &LentGroup) -> MonotonicGroupRef<'_> {
    MonotonicGroupRef {
        group: &group.group,
        source: group.source.as_ref(),
    }
}
