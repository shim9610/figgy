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
  #started = false;
  #lifecycleGeneration = 0;
  #readyToken = null;
  #exportToken = null;
  #lastPoint = null;
  #dpr = 1;
  #pendingResize = null;
  #pendingRelease = null;

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
      const generation = ++this.#lifecycleGeneration;
      const readyToken = this.#readyToken;
      this.#connect(generation, readyToken).catch((error) => {
        if (this.#isCurrentConnection(generation, readyToken)) {
          this.#fail(error, readyToken);
        }
      });
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
    return this.#exportToken !== null;
  }

  #isCurrentConnection(generation, readyToken = this.#readyToken) {
    return this.#started
      && this.isConnected
      && this.#lifecycleGeneration === generation
      && this.#readyToken === readyToken;
  }

  #isCurrentKernel(token) {
    return this.#exportToken === token
      && this.#lifecycleGeneration === token.generation
      && this.#kernel === token.kernel;
  }

  async #connect(generation, readyToken) {
    await ensureWasm();
    if (!this.#isCurrentConnection(generation, readyToken)) {
      return;
    }
    this.#resizeCanvas(false);
    if (!this.#isCurrentConnection(generation, readyToken)) {
      return;
    }
    const kernel = await RawFiggyChart.create(this.#canvas);
    if (!this.#isCurrentConnection(generation, readyToken)) {
      kernel.free();
      return;
    }
    let resizeObserver;
    try {
      resizeObserver = new ResizeObserver(() => this.#resizeCanvas(true));
      resizeObserver.observe(this);
    } catch (error) {
      kernel.free();
      throw error;
    }
    if (!this.#isCurrentConnection(generation, readyToken)) {
      resizeObserver.disconnect();
      kernel.free();
      return;
    }
    this.#kernel = kernel;
    this.#resizeObserver = resizeObserver;
    this.#pendingResize = null;
    this.#pendingRelease = null;
    readyToken.state = "fulfilled";
    readyToken.resolve(this);
    dispatchFiggyEvent(this, "figgy-ready", { chart: this, kernel });
    if (this.#isCurrentConnection(generation, readyToken) && this.#kernel === kernel) {
      this.#startLoop();
    }
  }

  #resetReady() {
    const token = {
      state: "pending",
      resolve: null,
      reject: null,
      promise: null,
    };
    token.promise = new Promise((resolve, reject) => {
      token.resolve = resolve;
      token.reject = reject;
    });
    // A disconnect is a normal custom-element lifecycle event. Keep the
    // rejected Promise observable to callers without generating a page-level
    // unhandledrejection when nobody retained that connection's `ready`.
    token.promise.catch(() => {});
    this.#readyToken = token;
    this.ready = token.promise;
  }

  #fail(error, readyToken) {
    console.error("figgy-chart:", error);
    if (this.#readyToken === readyToken && readyToken.state === "pending") {
      readyToken.state = "rejected";
      readyToken.reject(error);
    }
    dispatchFiggyEvent(this, "figgy-error", { error });
  }

  #startLoop() {
    if (!this.#raf) {
      this.#raf = requestAnimationFrame(this.#tick);
    }
  }

  #tick = () => {
    this.#raf = requestAnimationFrame(this.#tick);
    if (!this.#kernel || this.busy) {
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
      if (this.busy) {
        this.#pendingResize = {
          token: this.#exportToken,
          width,
          height,
        };
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
      if (!this.#kernel || this.busy) {
        return;
      }
      this.#canvas.setPointerCapture(event.pointerId);
      const [x, y] = this.#eventPoint(event);
      this.#lastPoint = [x, y];
      const selected = this.#kernel.on_press(x, y);
      dispatchFiggyEvent(this, "figgy-select", { selected, x, y, originalEvent: event });
    });

    this.#canvas.addEventListener("pointermove", (event) => {
      if (!this.#kernel || !this.#lastPoint || this.busy) {
        return;
      }
      const [x, y] = this.#eventPoint(event);
      const [lastX, lastY] = this.#lastPoint;
      this.#kernel.on_move(x - lastX, y - lastY);
      this.#lastPoint = [x, y];
      dispatchFiggyEvent(this, "figgy-drag", { x, y, dx: x - lastX, dy: y - lastY });
    });

    const release = () => {
      if (!this.#kernel || !this.#lastPoint) {
        return;
      }
      this.#lastPoint = null;
      const token = this.#exportToken;
      if (token) {
        if (this.#isCurrentKernel(token) && !this.#pendingRelease) {
          this.#pendingRelease = { token };
        }
        return;
      }
      const kernel = this.#kernel;
      kernel.on_release();
      dispatchFiggyEvent(this, "figgy-release", { selected: kernel.has_selection() });
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
    if (this.busy) {
      throw new Error("figgy chart is busy");
    }
    return this.kernel;
  }

  resize() {
    this.#resizeCanvas(true);
  }

  frame() {
    if (!this.busy) {
      this.kernel.frame();
    }
  }

  async export_png(scale = 1.0) {
    if (this.busy) {
      throw new Error("figgy chart is busy");
    }
    const token = {
      generation: this.#lifecycleGeneration,
      kernel: this.kernel,
      disposal: "attached",
    };
    this.#exportToken = token;
    try {
      return await token.kernel.export_png(scale);
    } finally {
      this.#settleExport(token);
    }
  }

  #settleExport(token) {
    let cleanupError = null;
    if (this.#isCurrentKernel(token) && this.#pendingRelease?.token === token) {
      this.#pendingRelease = null;
      try {
        token.kernel.on_release();
        const selected = token.kernel.has_selection();
        dispatchFiggyEvent(this, "figgy-release", { selected });
      } catch (error) {
        cleanupError = error;
      }
    }

    if (this.#isCurrentKernel(token) && this.#pendingResize?.token === token) {
      const { width, height } = this.#pendingResize;
      this.#pendingResize = null;
      try {
        token.kernel.resize(width, height);
      } catch (error) {
        cleanupError ??= error;
      }
    }

    if (this.#exportToken === token) {
      this.#exportToken = null;
    }
    if (token.disposal === "deferred") {
      token.disposal = "freed";
      try {
        token.kernel.free();
      } catch (error) {
        cleanupError ??= error;
      }
    }
    if (cleanupError) {
      throw cleanupError;
    }
  }

  free() {
    const exportToken = this.#exportToken;
    const active = this.#started
      || this.#kernel !== null
      || this.#resizeObserver !== null
      || this.#raf !== 0
      || this.#exportToken !== null;
    if (!active) {
      return;
    }

    ++this.#lifecycleGeneration;
    this.#started = false;
    this.#lastPoint = null;
    this.#pendingResize = null;
    this.#pendingRelease = null;
    this.#exportToken = null;

    if (this.#raf) {
      cancelAnimationFrame(this.#raf);
      this.#raf = 0;
    }
    if (this.#resizeObserver) {
      this.#resizeObserver.disconnect();
      this.#resizeObserver = null;
    }
    const kernel = this.#kernel;
    this.#kernel = null;

    const readyToken = this.#readyToken;
    if (readyToken.state === "pending") {
      const error = new DOMException(
        "figgy chart connection ended before it became ready",
        "AbortError",
      );
      readyToken.state = "rejected";
      readyToken.reject(error);
    }
    this.#resetReady();

    if (kernel) {
      if (exportToken?.kernel === kernel && exportToken.disposal === "attached") {
        exportToken.disposal = "deferred";
      } else {
        kernel.free();
      }
    }
  }

  register_font(bytes) { return this.#kernelForCall().register_font(bytes); }
  register_column_f32(id, data) { return this.#kernelForCall().register_column_f32(id, data); }
  register_column_f64(id, data) { return this.#kernelForCall().register_column_f64(id, data); }
  update_register_column_f32(id, data) {
    return this.#kernelForCall().update_register_column_f32(id, data);
  }
  update_register_column_f64(id, data) {
    return this.#kernelForCall().update_register_column_f64(id, data);
  }
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
  hit_test(x, y) {
    const hit = this.#kernelForCall().hit_test(x, y);
    return hit === undefined ? null : hit;
  }
  pick_point(x, y, maxDistancePx) {
    return this.#kernelForCall().pick_point(x, y, maxDistancePx).then((hit) => (
      hit === undefined ? null : JSON.parse(hit)
    ));
  }
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
