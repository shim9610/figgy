import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath, pathToFileURL } from "node:url";

const TEST_DIR = path.dirname(fileURLToPath(import.meta.url));
const FACADE_PATH = path.resolve(TEST_DIR, "..", "figgy-chart.js");

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

async function flushTasks() {
  await Promise.resolve();
  await new Promise((resolve) => setImmediate(resolve));
  await Promise.resolve();
}

class FakeEventTarget {
  constructor() {
    this.listeners = new Map();
  }

  addEventListener(type, listener) {
    const listeners = this.listeners.get(type) ?? [];
    listeners.push(listener);
    this.listeners.set(type, listeners);
  }

  dispatchEvent(event) {
    event.target ??= this;
    for (const listener of this.listeners.get(event.type) ?? []) {
      listener.call(this, event);
    }
    return true;
  }

  emit(type, fields = {}) {
    this.dispatchEvent({ type, ...fields });
  }
}

class FakeCanvas extends FakeEventTarget {
  constructor() {
    super();
    this.width = 300;
    this.height = 150;
    this.rect = { left: 0, top: 0, width: 640, height: 480 };
    this.capturedPointers = [];
  }

  getBoundingClientRect() {
    return { ...this.rect };
  }

  setPointerCapture(pointerId) {
    this.capturedPointers.push(pointerId);
  }
}

class FakeHTMLElement extends FakeEventTarget {
  constructor() {
    super();
    this.isConnected = false;
    this.clientWidth = 640;
    this.clientHeight = 480;
    this.rect = { left: 0, top: 0, width: 640, height: 480 };
    this.shadow = null;
  }

  attachShadow() {
    this.shadow = {
      children: [],
      append: (...children) => this.shadow.children.push(...children),
    };
    return this.shadow;
  }

  getBoundingClientRect() {
    return { ...this.rect };
  }

  setRect(width, height) {
    this.rect = { ...this.rect, width, height };
    this.clientWidth = width;
    this.clientHeight = height;
  }
}

class FakeCustomEvent {
  constructor(type, options = {}) {
    this.type = type;
    this.detail = options.detail;
    this.bubbles = options.bubbles ?? false;
    this.composed = options.composed ?? false;
    this.target = null;
  }
}

async function loadFacade({ initImpl, createImpl } = {}) {
  const state = {
    initCalls: 0,
    createCalls: [],
    observers: [],
    rafs: new Map(),
    nextRaf: 1,
    errors: [],
  };

  class FakeResizeObserver {
    constructor(callback) {
      this.callback = callback;
      this.target = null;
      this.disconnectCalls = 0;
      state.observers.push(this);
    }

    observe(target) {
      this.target = target;
    }

    disconnect() {
      this.target = null;
      this.disconnectCalls += 1;
    }
  }

  const registry = new Map();
  const document = {
    createElement(tag) {
      if (tag === "canvas") {
        return new FakeCanvas();
      }
      return { textContent: "" };
    },
  };
  const customElements = {
    define(name, constructor) {
      registry.set(name, constructor);
    },
    get(name) {
      return registry.get(name);
    },
  };
  const requestAnimationFrame = (callback) => {
    const id = state.nextRaf++;
    state.rafs.set(id, callback);
    return id;
  };
  const cancelAnimationFrame = (id) => {
    state.rafs.delete(id);
  };
  const quietConsole = {
    error: (...args) => state.errors.push(args),
    log() {},
    warn() {},
  };

  const context = vm.createContext({
    HTMLElement: FakeHTMLElement,
    document,
    customElements,
    CustomEvent: FakeCustomEvent,
    ResizeObserver: FakeResizeObserver,
    requestAnimationFrame,
    cancelAnimationFrame,
    window: { devicePixelRatio: 1 },
    DOMException,
    console: quietConsole,
    Uint8Array,
  });

  class RawFiggyChart {
    static create(canvas) {
      state.createCalls.push(canvas);
      return createImpl(canvas, state.createCalls.length - 1);
    }
  }

  const init = () => {
    state.initCalls += 1;
    return initImpl?.() ?? Promise.resolve();
  };
  const bindings = new vm.SyntheticModule(
    [
      "default",
      "AxisPreset",
      "ColorCycle",
      "FiggyChart",
      "color_cycle_css",
      "draw_style_modes",
      "draw_style_param_specs",
    ],
    function setBindings() {
      this.setExport("default", init);
      this.setExport("AxisPreset", Object.freeze({}));
      this.setExport("ColorCycle", Object.freeze({}));
      this.setExport("FiggyChart", RawFiggyChart);
      this.setExport("color_cycle_css", () => "");
      this.setExport("draw_style_modes", () => "[]");
      this.setExport("draw_style_param_specs", () => "[]");
    },
    { context, identifier: "figgy-test-bindings" },
  );

  const source = await readFile(FACADE_PATH, "utf8");
  const facade = new vm.SourceTextModule(source, {
    context,
    identifier: pathToFileURL(FACADE_PATH).href,
  });
  await facade.link((specifier) => {
    assert.equal(specifier, "./pkg/figgy.js");
    return bindings;
  });
  await facade.evaluate();

  return {
    Element: facade.namespace.FiggyChartElement,
    state,
  };
}

function connect(element) {
  element.isConnected = true;
  element.connectedCallback();
}

function disconnect(element) {
  element.isConnected = false;
  element.disconnectedCallback();
}

function makeKernel(name, options = {}) {
  const calls = [];
  const kernel = {
    name,
    calls,
    freeCalls: 0,
    exportImpl: options.exportImpl ?? (() => Promise.resolve(new Uint8Array([1]))),
    hitValue: undefined,
    pickImpl: () => Promise.resolve(undefined),
    free() {
      this.freeCalls += 1;
      calls.push(["free"]);
    },
    frame() {
      calls.push(["frame"]);
    },
    resize(width, height) {
      calls.push(["resize", width, height]);
      if (options.resizeError) {
        throw options.resizeError;
      }
    },
    on_press(x, y) {
      calls.push(["press", x, y]);
      return true;
    },
    on_move(dx, dy) {
      calls.push(["move", dx, dy]);
    },
    on_release() {
      calls.push(["release"]);
      options.onRelease?.();
      if (options.releaseError) {
        throw options.releaseError;
      }
    },
    has_selection() {
      calls.push(["has_selection"]);
      return true;
    },
    export_png(scale) {
      calls.push(["export", scale]);
      return this.exportImpl(scale);
    },
    hit_test() {
      return this.hitValue;
    },
    pick_point() {
      return this.pickImpl();
    },
  };
  return kernel;
}

test("disconnect during wasm init rejects only the old ready generation", async () => {
  const init = deferred();
  const create = deferred();
  const kernel = makeKernel("current");
  const { Element, state } = await loadFacade({
    initImpl: () => init.promise,
    createImpl: () => create.promise,
  });
  const element = new Element();
  let readyEvents = 0;
  element.addEventListener("figgy-ready", () => {
    readyEvents += 1;
  });

  const oldReady = element.ready;
  connect(element);
  await flushTasks();
  assert.equal(state.initCalls, 1);
  assert.equal(state.createCalls.length, 0);

  disconnect(element);
  await assert.rejects(oldReady, (error) => error?.name === "AbortError");
  const nextReady = element.ready;
  element.free();
  assert.equal(element.ready, nextReady, "repeated free must not strand another ready");

  connect(element);
  init.resolve();
  await flushTasks();
  assert.equal(state.initCalls, 1, "wasm init promise is shared");
  assert.equal(state.createCalls.length, 1, "only the current generation may create");

  create.resolve(kernel);
  await nextReady;
  assert.equal(element.kernel, kernel);
  assert.equal(readyEvents, 1);
  assert.equal(state.observers.filter((observer) => observer.target === element).length, 1);
  assert.equal(state.rafs.size, 1);

  disconnect(element);
  element.free();
  assert.equal(kernel.freeCalls, 1);
  assert.equal(state.rafs.size, 0);
});

test("resize callback generation changes stop stale create before it starts", async () => {
  const kernel = makeKernel("current");
  const { Element, state } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  let reconnectedReady;
  let resizeEvents = 0;
  element.addEventListener("figgy-resize", () => {
    resizeEvents += 1;
    if (resizeEvents === 1) {
      disconnect(element);
      reconnectedReady = element.ready;
      connect(element);
    }
  });

  connect(element);
  await flushTasks();
  assert.ok(reconnectedReady);
  await reconnectedReady;
  assert.equal(state.createCalls.length, 1, "stale generation must stop after resize callback");
  assert.equal(element.kernel, kernel);
  assert.equal(state.observers.filter((observer) => observer.target === element).length, 1);
  assert.equal(state.rafs.size, 1);
});

test("stale create success is freed and stale rejection cannot fail current ready", async () => {
  {
    const first = deferred();
    const second = deferred();
    const staleKernel = makeKernel("stale");
    const currentKernel = makeKernel("current");
    const creates = [first, second];
    const { Element } = await loadFacade({
      createImpl: () => creates.shift().promise,
    });
    const element = new Element();
    const oldReady = element.ready;
    connect(element);
    await flushTasks();
    disconnect(element);
    await assert.rejects(oldReady, (error) => error?.name === "AbortError");
    const currentReady = element.ready;
    connect(element);
    await flushTasks();

    first.resolve(staleKernel);
    await flushTasks();
    assert.equal(staleKernel.freeCalls, 1);
    assert.equal(element.busy, false);

    second.resolve(currentKernel);
    await currentReady;
    assert.equal(element.kernel, currentKernel);
  }

  {
    const first = deferred();
    const second = deferred();
    const currentKernel = makeKernel("current");
    const creates = [first, second];
    const { Element, state } = await loadFacade({
      createImpl: () => creates.shift().promise,
    });
    const element = new Element();
    let errorEvents = 0;
    element.addEventListener("figgy-error", () => {
      errorEvents += 1;
    });
    const oldReady = element.ready;
    connect(element);
    await flushTasks();
    disconnect(element);
    await assert.rejects(oldReady, (error) => error?.name === "AbortError");
    const currentReady = element.ready;
    connect(element);
    await flushTasks();

    second.resolve(currentKernel);
    await currentReady;
    const staleError = new Error("stale create failed");
    first.reject(staleError);
    await flushTasks();
    assert.equal(element.kernel, currentKernel);
    assert.equal(errorEvents, 0);
    assert.equal(state.errors.length, 0);
  }

  {
    const first = deferred();
    const second = deferred();
    const staleKernel = makeKernel("late-stale");
    const currentKernel = makeKernel("current-first");
    const creates = [first, second];
    const { Element } = await loadFacade({
      createImpl: () => creates.shift().promise,
    });
    const element = new Element();
    const oldReady = element.ready;
    connect(element);
    await flushTasks();
    disconnect(element);
    await assert.rejects(oldReady, (error) => error?.name === "AbortError");
    const currentReady = element.ready;
    connect(element);
    await flushTasks();

    second.resolve(currentKernel);
    await currentReady;
    first.resolve(staleKernel);
    await flushTasks();
    assert.equal(element.kernel, currentKernel);
    assert.equal(staleKernel.freeCalls, 1);
    assert.equal(currentKernel.freeCalls, 0);
  }
});

test("ready-event teardown publishes only the reconnected observer and rAF", async () => {
  const firstKernel = makeKernel("first-ready");
  const currentKernel = makeKernel("second-ready");
  const kernels = [firstKernel, currentKernel];
  const { Element, state } = await loadFacade({
    createImpl: () => Promise.resolve(kernels.shift()),
  });
  const element = new Element();
  let readyEvents = 0;
  let reconnectedReady;
  element.addEventListener("figgy-ready", () => {
    readyEvents += 1;
    if (readyEvents === 1) {
      disconnect(element);
      reconnectedReady = element.ready;
      connect(element);
    }
  });

  const firstReady = element.ready;
  connect(element);
  await firstReady;
  await flushTasks();
  assert.ok(reconnectedReady);
  await reconnectedReady;
  assert.equal(readyEvents, 2);
  assert.equal(firstKernel.freeCalls, 1);
  assert.equal(element.kernel, currentKernel);
  assert.equal(state.observers.filter((observer) => observer.target === element).length, 1);
  assert.equal(state.rafs.size, 1, "stale ready callback must not schedule its own rAF");
});

test("busy pointer release is deferred once and settled before resize", async () => {
  const firstExport = deferred();
  const secondExport = deferred();
  const exports = [firstExport, secondExport];
  let element;
  const kernel = makeKernel("active", {
    exportImpl: () => exports.shift().promise,
    onRelease: () => element.canvas.emit("pointercancel"),
  });
  const { Element } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  element = new Element();
  const order = [];
  let nestedExport;
  element.addEventListener("figgy-release", () => {
    order.push("event");
    nestedExport ??= element.export_png();
  });
  connect(element);
  await element.ready;

  element.canvas.emit("pointerdown", { pointerId: 1, clientX: 10, clientY: 20 });
  const exported = element.export_png(2);
  assert.equal(element.busy, true);
  element.canvas.emit("pointerdown", { pointerId: 99, clientX: 30, clientY: 40 });
  element.canvas.emit("pointermove", { pointerId: 99, clientX: 50, clientY: 60 });
  assert.equal(kernel.calls.filter(([name]) => name === "press").length, 1);
  assert.equal(kernel.calls.filter(([name]) => name === "move").length, 0);
  element.canvas.emit("pointerup");
  element.canvas.emit("pointercancel");
  assert.equal(kernel.calls.filter(([name]) => name === "release").length, 0);
  assert.equal(kernel.calls.filter(([name]) => name === "has_selection").length, 0);

  element.setRect(800, 600);
  element.resize();
  firstExport.resolve(new Uint8Array([2]));
  assert.deepEqual(Array.from(await exported), [2]);
  await assert.rejects(nestedExport, /busy/);
  assert.equal(element.busy, false);
  assert.equal(kernel.calls.filter(([name]) => name === "release").length, 1);
  assert.equal(kernel.calls.filter(([name]) => name === "has_selection").length, 1);
  assert.equal(kernel.calls.filter(([name]) => name === "resize").length, 1);
  assert.deepEqual(order, ["event"]);

  element.canvas.emit("pointerup");
  assert.equal(kernel.calls.filter(([name]) => name === "release").length, 1);

  element.canvas.emit("pointerdown", { pointerId: 2, clientX: 5, clientY: 6 });
  const rejected = element.export_png(1);
  element.canvas.emit("pointercancel");
  const exportError = new Error("export failed");
  secondExport.reject(exportError);
  await assert.rejects(rejected, (error) => error === exportError);
  assert.equal(element.busy, false);
  assert.equal(kernel.calls.filter(([name]) => name === "release").length, 2);
});

test("release cleanup cannot strand busy and listener teardown suppresses stale resize", async () => {
  {
    const operation = deferred();
    const releaseError = new Error("release cleanup failed");
    const kernel = makeKernel("throws", {
      exportImpl: () => operation.promise,
      releaseError,
    });
    const { Element } = await loadFacade({
      createImpl: () => Promise.resolve(kernel),
    });
    const element = new Element();
    connect(element);
    await element.ready;
    element.canvas.emit("pointerdown", { pointerId: 1, clientX: 1, clientY: 1 });
    const exported = element.export_png();
    element.canvas.emit("pointerup");
    element.setRect(700, 500);
    element.resize();
    operation.resolve(new Uint8Array([1]));
    await assert.rejects(exported, (error) => error === releaseError);
    assert.equal(element.busy, false);
    assert.equal(kernel.calls.filter(([name]) => name === "resize").length, 1);
  }

  {
    const operation = deferred();
    const kernel = makeKernel("reconnected-by-listener-old", {
      exportImpl: () => operation.promise,
    });
    const currentKernel = makeKernel("reconnected-by-listener-current");
    const kernels = [kernel, currentKernel];
    const { Element } = await loadFacade({
      createImpl: () => Promise.resolve(kernels.shift()),
    });
    const element = new Element();
    connect(element);
    await element.ready;
    let reconnectedReady;
    element.addEventListener("figgy-release", () => {
      disconnect(element);
      reconnectedReady = element.ready;
      connect(element);
    });
    element.canvas.emit("pointerdown", { pointerId: 1, clientX: 1, clientY: 1 });
    const exported = element.export_png();
    element.canvas.emit("pointerup");
    element.setRect(900, 700);
    element.resize();
    operation.resolve(new Uint8Array([3]));
    assert.deepEqual(Array.from(await exported), [3]);
    await flushTasks();
    await reconnectedReady;
    assert.equal(element.busy, false);
    assert.equal(kernel.freeCalls, 1);
    assert.equal(kernel.calls.filter(([name]) => name === "resize").length, 0);
    assert.equal(element.kernel, currentKernel);
    assert.equal(currentKernel.calls.filter(([name]) => name === "resize").length, 0);
  }

  {
    const operation = deferred();
    const resizeError = new Error("resize cleanup failed");
    const kernel = makeKernel("resize-throws", {
      exportImpl: () => operation.promise,
      resizeError,
    });
    const { Element } = await loadFacade({
      createImpl: () => Promise.resolve(kernel),
    });
    const element = new Element();
    connect(element);
    await element.ready;
    const exported = element.export_png();
    element.setRect(750, 550);
    element.resize();
    operation.resolve(new Uint8Array([4]));
    await assert.rejects(exported, (error) => error === resizeError);
    assert.equal(element.busy, false);
    assert.equal(kernel.calls.filter(([name]) => name === "resize").length, 1);
  }
});

test("stale export settle cannot clear a reconnected kernel export token", async () => {
  const oldExport = deferred();
  const newExport = deferred();
  const oldKernel = makeKernel("old", { exportImpl: () => oldExport.promise });
  const newKernel = makeKernel("new", { exportImpl: () => newExport.promise });
  const kernels = [oldKernel, newKernel];
  const { Element } = await loadFacade({
    createImpl: () => Promise.resolve(kernels.shift()),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  const oldPromise = element.export_png();

  disconnect(element);
  const newReady = element.ready;
  connect(element);
  await newReady;
  const newPromise = element.export_png();
  assert.equal(element.busy, true);

  oldExport.resolve(new Uint8Array([1]));
  await oldPromise;
  assert.equal(element.busy, true, "old finally must not release the new export");

  newExport.resolve(new Uint8Array([2]));
  await newPromise;
  assert.equal(element.busy, false);
});

test("facade normalizes hit and pick results without changing rejection reasons", async () => {
  const kernel = makeKernel("contract");
  const { Element } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;

  kernel.hitValue = undefined;
  assert.equal(element.hit_test(1, 2), null);
  kernel.hitValue = "legend";
  assert.equal(element.hit_test(1, 2), "legend");

  kernel.pickImpl = () => Promise.resolve(undefined);
  assert.equal(await element.pick_point(1, 2, 3), null);
  kernel.pickImpl = () => Promise.resolve(JSON.stringify({
    source_id: null,
    series_id: "series-a",
    point_index: 4,
    data_x: 1.25,
    data_y: -2,
    distance_px: 0.5,
  }));
  assert.deepEqual(
    JSON.parse(JSON.stringify(await element.pick_point(1, 2, 3))),
    {
      source_id: null,
      series_id: "series-a",
      point_index: 4,
      data_x: 1.25,
      data_y: -2,
      distance_px: 0.5,
    },
  );
  kernel.pickImpl = () => Promise.resolve(JSON.stringify({
    source_id: "source-a",
    series_id: "series-a",
    point_index: 5,
    data_x: 2,
    data_y: 3,
    distance_px: 1,
  }));
  assert.equal((await element.pick_point(1, 2, 3)).source_id, "source-a");

  kernel.pickImpl = () => Promise.resolve("{not json");
  await assert.rejects(
    element.pick_point(1, 2, 3),
    (error) => error?.name === "SyntaxError",
  );

  const rawError = new Error("raw pick failed");
  kernel.pickImpl = () => Promise.reject(rawError);
  await assert.rejects(element.pick_point(1, 2, 3), (error) => error === rawError);
});
