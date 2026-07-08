import init, {
  AxisPreset,
  ColorCycle,
  FiggyChart as RawFiggyChart,
  color_cycle_css,
  draw_style_modes,
  draw_style_param_specs,
} from "./pkg/figgy.js";

let wasmReady = null;

async function ensureWasm() {
  if (!wasmReady) {
    wasmReady = init();
  }
  await wasmReady;
}

function dispatchFiggyEvent(target, type, detail = {}) {
  target.dispatchEvent(new CustomEvent(type, { detail, bubbles: true, composed: true }));
}

export class FiggyChartElement extends HTMLElement {
  #canvas;
  #kernel = null;
  #resizeObserver = null;
  #raf = 0;
  #busy = false;
  #started = false;
  #lastPoint = null;
  #dpr = 1;
  #pendingResize = null;
  #readyResolve = null;
  #readyReject = null;

  constructor() {
    super();
    const shadow = this.attachShadow({ mode: "open" });
    const style = document.createElement("style");
    style.textContent = `
      :host {
        display: block;
        inline-size: 100%;
        block-size: 100%;
        min-inline-size: 1px;
        min-block-size: 1px;
      }
      canvas {
        display: block;
        inline-size: 100%;
        block-size: 100%;
        touch-action: none;
      }
    `;
    this.#canvas = document.createElement("canvas");
    shadow.append(style, this.#canvas);
    this.#resetReady();
    this.#installPointerHandlers();
  }

  connectedCallback() {
    if (!this.#started) {
      this.#started = true;
      this.#connect().catch((error) => this.#fail(error));
    }
  }

  disconnectedCallback() {
    this.free();
  }

  get canvas() {
    return this.#canvas;
  }

  get kernel() {
    if (!this.#kernel) {
      throw new Error("figgy chart is not ready yet; await element.ready first");
    }
    return this.#kernel;
  }

  get busy() {
    return this.#busy;
  }

  async #connect() {
    await ensureWasm();
    if (!this.isConnected) {
      return;
    }
    this.#resizeCanvas(false);
    const kernel = await RawFiggyChart.create(this.#canvas);
    if (!this.isConnected) {
      kernel.free();
      return;
    }
    this.#kernel = kernel;
    this.#resizeObserver = new ResizeObserver(() => this.#resizeCanvas(true));
    this.#resizeObserver.observe(this);
    this.#readyResolve(this);
    dispatchFiggyEvent(this, "figgy-ready", { chart: this, kernel: this.#kernel });
    this.#startLoop();
  }

  #resetReady() {
    this.ready = new Promise((resolve, reject) => {
      this.#readyResolve = resolve;
      this.#readyReject = reject;
    });
  }

  #fail(error) {
    console.error("figgy-chart:", error);
    this.#readyReject(error);
    dispatchFiggyEvent(this, "figgy-error", { error });
  }

  #startLoop() {
    if (!this.#raf) {
      this.#raf = requestAnimationFrame(this.#tick);
    }
  }

  #tick = () => {
    this.#raf = requestAnimationFrame(this.#tick);
    if (!this.#kernel || this.#busy) {
      return;
    }
    if ((window.devicePixelRatio || 1) !== this.#dpr) {
      this.#resizeCanvas(true);
    }
    try {
      this.#kernel.frame();
    } catch (error) {
      console.error("figgy frame:", error);
      dispatchFiggyEvent(this, "figgy-error", { error });
    }
  };

  #resizeCanvas(notifyKernel) {
    const rect = this.getBoundingClientRect();
    const cssWidth = Math.max(1, Math.round(rect.width || this.clientWidth || 1));
    const cssHeight = Math.max(1, Math.round(rect.height || this.clientHeight || 1));
    const dpr = window.devicePixelRatio || 1;
    const width = Math.max(1, Math.round(cssWidth * dpr));
    const height = Math.max(1, Math.round(cssHeight * dpr));
    this.#dpr = dpr;

    if (this.#canvas.width === width && this.#canvas.height === height) {
      return;
    }

    this.#canvas.width = width;
    this.#canvas.height = height;
    if (notifyKernel && this.#kernel) {
      if (this.#busy) {
        this.#pendingResize = [width, height];
      } else {
        this.#kernel.resize(width, height);
        this.#pendingResize = null;
      }
    }
    dispatchFiggyEvent(this, "figgy-resize", {
      width,
      height,
      cssWidth,
      cssHeight,
      devicePixelRatio: dpr,
    });
  }

  #installPointerHandlers() {
    this.#canvas.addEventListener("pointerdown", (event) => {
      if (!this.#kernel || this.#busy) {
        return;
      }
      this.#canvas.setPointerCapture(event.pointerId);
      const [x, y] = this.#eventPoint(event);
      this.#lastPoint = [x, y];
      const selected = this.#kernel.on_press(x, y);
      dispatchFiggyEvent(this, "figgy-select", { selected, x, y, originalEvent: event });
    });

    this.#canvas.addEventListener("pointermove", (event) => {
      if (!this.#kernel || !this.#lastPoint || this.#busy) {
        return;
      }
      const [x, y] = this.#eventPoint(event);
      const [lastX, lastY] = this.#lastPoint;
      this.#kernel.on_move(x - lastX, y - lastY);
      this.#lastPoint = [x, y];
      dispatchFiggyEvent(this, "figgy-drag", { x, y, dx: x - lastX, dy: y - lastY });
    });

    const release = () => {
      if (!this.#kernel) {
        return;
      }
      this.#lastPoint = null;
      if (!this.#busy) {
        this.#kernel.on_release();
      }
      dispatchFiggyEvent(this, "figgy-release", { selected: this.#kernel.has_selection() });
    };
    this.#canvas.addEventListener("pointerup", release);
    this.#canvas.addEventListener("pointercancel", release);
  }

  #eventPoint(event) {
    const rect = this.#canvas.getBoundingClientRect();
    const sx = this.#canvas.width / Math.max(1, rect.width);
    const sy = this.#canvas.height / Math.max(1, rect.height);
    return [(event.clientX - rect.left) * sx, (event.clientY - rect.top) * sy];
  }

  #kernelForCall() {
    if (this.#busy) {
      throw new Error("figgy chart is busy");
    }
    return this.kernel;
  }

  resize() {
    this.#resizeCanvas(true);
  }

  frame() {
    if (!this.#busy) {
      this.kernel.frame();
    }
  }

  async export_png(scale = 1.0) {
    if (this.#busy) {
      throw new Error("figgy chart is busy");
    }
    this.#busy = true;
    try {
      return await this.kernel.export_png(scale);
    } finally {
      this.#busy = false;
      if (this.#pendingResize && this.#kernel) {
        const [width, height] = this.#pendingResize;
        this.#pendingResize = null;
        this.#kernel.resize(width, height);
      }
    }
  }

  free() {
    if (this.#raf) {
      cancelAnimationFrame(this.#raf);
      this.#raf = 0;
    }
    if (this.#resizeObserver) {
      this.#resizeObserver.disconnect();
      this.#resizeObserver = null;
    }
    if (this.#kernel) {
      this.#kernel.free();
      this.#kernel = null;
    }
    this.#busy = false;
    this.#started = false;
    this.#lastPoint = null;
    this.#pendingResize = null;
    this.#resetReady();
  }

  register_font(bytes) { return this.#kernelForCall().register_font(bytes); }
  set_column_f32(id, data) { return this.#kernelForCall().set_column_f32(id, data); }
  set_column_f64(id, data) { return this.#kernelForCall().set_column_f64(id, data); }
  remove_column(id) { return this.#kernelForCall().remove_column(id); }
  add_line_series(seriesId, xColumn, yColumn, lineWidth, label) {
    return this.#kernelForCall().add_line_series(seriesId, xColumn, yColumn, lineWidth, label);
  }
  set_series_label(seriesId, label) { return this.#kernelForCall().set_series_label(seriesId, label); }
  remove_series(seriesId) { return this.#kernelForCall().remove_series(seriesId); }
  auto_fit_x(column, padding) { return this.#kernelForCall().auto_fit_x(column, padding); }
  auto_fit_y(column, padding) { return this.#kernelForCall().auto_fit_y(column, padding); }
  auto_fit_all(padding) { return this.#kernelForCall().auto_fit_all(padding); }
  set_title(text) { return this.#kernelForCall().set_title(text); }
  set_x_title(text) { return this.#kernelForCall().set_x_title(text); }
  set_y_title(text) { return this.#kernelForCall().set_y_title(text); }
  apply_axis_preset(preset) { return this.#kernelForCall().apply_axis_preset(preset); }
  apply_color_cycle(cycle) { return this.#kernelForCall().apply_color_cycle(cycle); }
  get_config() { return this.#kernelForCall().get_config(); }
  set_config(json) { return this.#kernelForCall().set_config(json); }
  get_series() { return this.#kernelForCall().get_series(); }
  set_series(json) { return this.#kernelForCall().set_series(json); }
  reset_legend_from_series_labels() { return this.#kernelForCall().reset_legend_from_series_labels(); }
  hit_test(x, y) { return this.#kernelForCall().hit_test(x, y); }
  pick_point(x, y, maxDistancePx) { return this.#kernelForCall().pick_point(x, y, maxDistancePx); }
  set_picked_points(json) { return this.#kernelForCall().set_picked_points(json); }
  set_clear_color(r, g, b, a) { return this.#kernelForCall().set_clear_color(r, g, b, a); }
  load_demo() { return this.#kernelForCall().load_demo(); }
  on_press(x, y) { return this.#kernelForCall().on_press(x, y); }
  on_move(dx, dy) { return this.#kernelForCall().on_move(dx, dy); }
  on_release() { return this.#kernelForCall().on_release(); }
  has_selection() { return this.#kernelForCall().has_selection(); }
}

if (!customElements.get("figgy-chart")) {
  customElements.define("figgy-chart", FiggyChartElement);
}

export {
  AxisPreset,
  ColorCycle,
  RawFiggyChart,
  color_cycle_css,
  draw_style_modes,
  draw_style_param_specs,
};
