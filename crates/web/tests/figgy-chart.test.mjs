import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath, pathToFileURL } from "node:url";

const TEST_DIR = path.dirname(fileURLToPath(import.meta.url));
const FACADE_PATH = path.resolve(TEST_DIR, "..", "figgy-chart.js");
const RAW_STARTUP_STAGES = [
  ["window", "instance"],
  ["window", "surface"],
  ["window", "adapter"],
  ["window", "device"],
  ["window", "configure"],
  ["window", "figgy frame msaa target"],
  ["renderer", "capabilities"],
  ["renderer", "figgy column pool"],
  ["renderer", "shader modules"],
  ["renderer", "figgy fullscreen textured pipeline"],
  ["renderer", "identity"],
  ["web.create", "chart resources"],
  ["web.create", "first frame"],
];

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

async function waitForIdle(element) {
  for (let attempt = 0; attempt < 8; attempt += 1) {
    await flushTasks();
    if (!element.busy) return;
  }
  throw new Error("facade did not become idle");
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
    timers: new Map(),
    nextTimer: 1,
    now: 0,
    messages: [],
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
    hidden: false,
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
    performance: { now: () => state.now },
    setTimeout: (callback) => {
      const id = state.nextTimer++;
      state.timers.set(id, callback);
      return id;
    },
    clearTimeout: (id) => state.timers.delete(id),
    MessageChannel: class {
      constructor() {
        this.port1 = { onmessage: null, closed: false, close() { this.closed = true; } };
        this.port2 = {
          postMessage: () => state.messages.push(() => { if (!this.port1.closed) this.port1.onmessage?.(); }),
          close() {},
        };
      }
    },
    window: { devicePixelRatio: 1 },
    DOMException,
    console: quietConsole,
    Uint8Array,
    Float32Array,
    Float64Array,
  });

  class RawFiggyChart {
    static create(canvas) {
      state.createCalls.push(canvas);
      return createImpl(canvas, state.createCalls.length - 1);
    }
    static async create_with_progress(canvas, onEvent) {
      for (const [scope, stage] of RAW_STARTUP_STAGES) {
        onEvent({ scope, stage, phase: "started" });
        if (stage !== "first frame") {
          onEvent({ scope, stage, phase: "finished" });
        }
      }
      const kernel = await RawFiggyChart.create(canvas);
      onEvent({ scope: "web.create", stage: "first frame", phase: "finished" });
      return kernel;
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
    document,
    browserWindow: context.window,
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
    prewarmImpl: options.prewarmImpl ?? (() => Promise.resolve()),
    prewarmAllWithProgressImpl:
      options.prewarmAllWithProgressImpl ?? (() => Promise.resolve()),
    prewarmAllImpl: options.prewarmAllImpl ?? (() => Promise.resolve()),
    hitValue: undefined,
    pickImpl: () => Promise.resolve(undefined),
    pickDataImpl: () => Promise.resolve(undefined),
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
    first_frame_ready() {
      calls.push(["first_frame_ready"]);
      return options.firstFrameImpl?.() ?? Promise.resolve();
    },
    hit_test() {
      return this.hitValue;
    },
    pick_point(x, y, maxDistancePx) {
      calls.push(["pick", x, y, maxDistancePx]);
      return this.pickImpl(x, y, maxDistancePx);
    },
    pick_data(x, y, maxDistancePx) {
      calls.push(["pick_data", x, y, maxDistancePx]);
      return this.pickDataImpl(x, y, maxDistancePx);
    },
    auto_fit_all(padding) {
      calls.push(["auto_fit_all", padding]);
      return options.autoFitImpl?.(padding) ?? Promise.resolve();
    },
    ensure_extent_engine() {
      calls.push(["ensure_extent_engine"]);
      return options.ensureExtentImpl?.() ?? Promise.resolve();
    },
    prewarm_gpu_picking() {
      calls.push(["prewarm_gpu_picking"]);
      return this.prewarmImpl();
    },
    prewarm_all_with_progress(onEvent) {
      calls.push(["prewarm_all_with_progress", onEvent]);
      return this.prewarmAllWithProgressImpl(onEvent);
    },
    prewarm_all() {
      calls.push(["prewarm_all"]);
      return this.prewarmAllImpl();
    },
    request_stream_auto_fit() { return false; },
    stream_status() {
      return JSON.stringify({ status: "complete", revision: "1", job_id: "1",
        submitted_primitives: 0, total_primitives: 0, auto_fit_pending: false });
    },
    set_stream_chunk_budget(size) { calls.push(["set_stream_chunk_budget", size]); },
    streaming_gpu_ready() { return new Promise(() => {}); },
    stream_selection_request_ranges() { return { status: "complete", free() {} }; },
    suspend_stream_selection() { calls.push(["suspend_stream_selection"]); },
    cancel_streaming_and_wait() {
      calls.push(["cancel_streaming_and_wait"]);
      this.interrupt_render?.();
      return options.cancelImpl?.() ?? Promise.resolve();
    },
    configure_auto_residency(memoryBudgetBytes, workingSetLimitBytes) {
      calls.push(["configure_auto_residency", memoryBudgetBytes, workingSetLimitBytes]);
    },
    try_auto_resident_chart(ids, revisions, sources) {
      calls.push(["try_auto_resident_chart", ids, revisions, sources]);
      return { status: "streamed", reason: "policy_unset" };
    },
    demote_auto_resident_columns(ids, revisions, sources) {
      calls.push(["demote_auto_resident_columns", ids, revisions, sources]);
    },
  };
  return kernel;
}

async function completedReplayFixture({ readRange = null, typed = false, extraColumn = null } = {}) {
  const kernel = makeKernel("auxiliary-replay");
  kernel.register_streaming_columns = () => {};
  kernel.register_streaming_column_sources = () => {};
  kernel.request_auto_streaming_chart = () => ({ status: "complete" });
  const environment = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  const element = new environment.Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const values = new Float32Array([0, 1, 2, 3, 4, 5, 6, 7]);
  const columns = typed
    ? [{ id: "x", revision: 2, values }]
    : [{ id: "x", revision: 2, length: 8, encoding: "f32" }];
  if (extraColumn) columns.push(extraColumn);
  await element.render_chart({ columns, readRange, maxPrimitivesPerChunk: 4 }).done;
  let pending = true;
  const handle = { free: () => kernel.calls.push(["handle-free"]) };
  kernel.begin_stream_export = (...args) => {
    kernel.calls.push(["begin-export", ...args]);
    pending = true;
    return handle;
  };
  kernel.stream_operation_request_ranges = (actual) => {
    assert.equal(actual, handle);
    return pending ? {
      status: "ready", source_ids: ["x"], source_revisions: [2], source_lengths: [8],
      offsets: [3], lengths: [2], encodings: ["f32"],
      free: () => kernel.calls.push(["request-free"]),
    } : { status: "complete", free: () => kernel.calls.push(["request-free"]) };
  };
  kernel.stream_operation_submit_ranges = (actual, ids, revisions, lengths, offsets, chunks) => {
    assert.equal(actual, handle);
    kernel.calls.push(["submit-aux", ids, revisions, lengths, offsets, chunks]);
    pending = false;
    return { free: () => kernel.calls.push(["progress-free"]) };
  };
  kernel.finish_stream_export = () => Promise.resolve(new Uint8Array([137, 80, 78, 71]));
  kernel.set_stream_operation_chunk_budget = (_handle, count) => {
    kernel.calls.push(["aux-budget", count]);
  };
  kernel.cancel_stream_operation_and_wait = () => {
    kernel.calls.push(["cancel-aux"]);
    return Promise.resolve();
  };
  return { ...environment, kernel, element, values };
}

async function rangeStreamFixture() {
  const kernel = makeKernel("provider-view-cache");
  let revision = 1;
  let offset = 0;
  kernel.register_streaming_column_sources = (_ids, revisions) => { revision = revisions[0]; };
  kernel.replace_streaming_column_sources = (_ids, revisions) => { revision = revisions[0]; offset = 0; };
  kernel.request_auto_streaming_chart = () => ({
    status: "started", source_ids: ["x"], source_revisions: [revision],
  });
  kernel.auto_stream_chart_request_ranges = () => ({
    status: offset === 4 ? "complete" : "ready",
    source_ids: ["x"], source_revisions: [revision], source_lengths: [4],
    offsets: [offset], lengths: [2], encodings: ["f32"],
    submitted_primitives: offset, total_primitives: 4,
  });
  kernel.auto_stream_chart_submit_ranges = (ids, revisions, lengths, offsets, chunks) => {
    assert.equal(offsets[0], offset);
    assert.equal(chunks[0].length, 2);
    kernel.calls.push(["stream-submit", revisions[0], chunks[0]]);
    offset += 2;
    return { status: "submitted", submitted_primitives: offset, total_primitives: 4 };
  };
  const { Element, state } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const runFrame = async () => {
    const [id, callback] = [...state.rafs.entries()].at(-1);
    state.rafs.delete(id);
    callback(0);
    await flushTasks();
  };
  const request = (readRange, sourceRevision = 1) => element.render_chart({
    columns: [{ id: "x", revision: sourceRevision, length: 4, encoding: "f32" }],
    maxPrimitivesPerChunk: 2, readRange,
  });
  const finishFrames = async () => { for (let index = 0; index < 6; index += 1) await runFrame(); };
  return { kernel, element, request, runFrame, finishFrames };
}

test("range-provider uses bounded exact stream without full-column promotion", async () => {
  const fixture = await rangeStreamFixture();
  const reads = [];
  const job = fixture.request((request) => {
    reads.push(request);
    return new Float32Array(request.length).fill(request.offset);
  });
  await fixture.finishFrames();
  assert.equal((await job.done).status, "complete");
  assert.deepEqual(reads.map(({ offset, length }) => [offset, length]), [[0, 2], [2, 2]]);
  assert.equal(fixture.kernel.calls.filter(([name]) => name === "stream-submit").length, 2);
  assert.equal(fixture.kernel.calls.filter(([name]) => name === "try_auto_resident_chart").length, 0);
});

test("range-provider cancellation discards late data without another source read", async () => {
  const fixture = await rangeStreamFixture();
  const late = deferred();
  const job = fixture.request(() => late.promise);
  await fixture.runFrame();
  await job.cancel();
  late.resolve(new Float32Array(2));
  await fixture.finishFrames();
  assert.equal((await job.done).status, "cancelled");
  assert.equal(fixture.kernel.calls.filter(([name]) => name === "stream-submit").length, 0);
  assert.equal(fixture.kernel.calls.filter(([name]) => name === "cancel_streaming_and_wait").length, 1);
});
test("completed stream export reuses source views and picking delegates without replay", async () => {
  const { kernel, element, values } = await completedReplayFixture({ typed: true });
  assert.deepEqual(Array.from(await element.export_png(2)), [137, 80, 78, 71]);
  const callsBeforePick = kernel.calls.length;
  assert.equal(await element.pick_point(10, 20, 4), null);
  assert.equal(await element.pick_data(10, 20, 4), null);
  assert.deepEqual(kernel.calls.slice(callsBeforePick).map(([name]) => name),
    ["pick", "pick_data"], "stream picking may use the packed GPU cache but must not replay sources");
  const submissions = kernel.calls.filter(([name]) => name === "submit-aux");
  assert.equal(submissions.length, 1);
  for (const [, ids, revisions, lengths, offsets, chunks] of submissions) {
    assert.deepEqual([...ids], ["x"]);
    assert.deepEqual([...revisions], [2]);
    assert.deepEqual([...lengths], [8]);
    assert.deepEqual([...offsets], [3]);
    assert.deepEqual([...chunks[0]], [3, 4]);
    assert.equal(chunks[0].buffer, values.buffer, "the facade must borrow a view, not clone data");
  }
  assert.equal(kernel.calls.filter(([name]) => name === "cancel-aux").length, 0);
  assert.equal(kernel.calls.filter(([name]) => name === "handle-free").length, 1);
  assert.equal(kernel.calls.filter(([name]) => name === "request-free").length, 2);
  assert.equal(kernel.calls.filter(([name]) => name === "progress-free").length, 1);
  assert.equal(element.busy, false);
});

function runFacadeFrame(state) {
  const [id, callback] = [...state.rafs.entries()].at(-1);
  state.rafs.delete(id);
  callback(state.now);
}

function selectionRequest(index, freed) {
  return {
    status: "ready", index, source_ids: ["x"], source_revisions: [2], source_lengths: [8],
    offsets: [index], lengths: [1], encodings: ["f32"],
    same_selection_request(other) { return other.index === index; },
    free() { freed.push(index); },
  };
}

test("selection-only mutations do not replay a completed stream", async () => {
  const { kernel, element, state } = await completedReplayFixture({ typed: true });
  let replayRequests = 0;
  kernel.request_auto_streaming_chart = () => {
    replayRequests += 1;
    return { status: "complete" };
  };
  kernel.set_picked_points = () => {};
  kernel.set_picked_data = () => {};
  for (let index = 0; index < 10; index += 1) {
    element.set_picked_points(String(index));
    runFacadeFrame(state);
  }
  element.set_picked_data("null");
  runFacadeFrame(state);
  await flushTasks();
  assert.equal(replayRequests, 0);
  element.free();
});

test("selection replacement keeps old overlay until the newest bounded range resolves", async () => {
  const reads = [], freed = [], first = deferred(), newest = deferred();
  const { kernel, element, state } = await completedReplayFixture({ readRange(range) {
    reads.push(range);
    return range.offset === 3 ? first.promise : newest.promise;
  } });
  let desired = 3, committed = 1;
  kernel.set_picked_points = (json) => { desired = JSON.parse(json); };
  kernel.stream_selection_request_ranges = () => desired === committed
    ? { status: "complete", free() {} } : selectionRequest(desired, freed);
  kernel.stream_selection_submit_ranges = (request, ids, revisions, lengths, offsets, chunks) => {
    assert.equal(request.index, desired);
    assert.deepEqual([...chunks[0]], [desired]);
    committed = desired;
  };
  runFacadeFrame(state);
  await flushTasks();
  runFacadeFrame(state);
  assert.equal(reads.length, 1, "unchanged pending selection must not re-read");
  assert.equal(committed, 1);
  element.set_picked_points("5");
  runFacadeFrame(state);
  await flushTasks();
  assert.deepEqual(reads.map((r) => r.offset), [3, 5]);
  assert.equal(committed, 1, "old overlay stays visible while new source is delayed");
  first.resolve(new Float32Array([3]));
  await flushTasks();
  runFacadeFrame(state);
  assert.equal(committed, 1, "late superseded reply cannot replace the overlay");
  newest.resolve(new Float32Array([5]));
  await flushTasks();
  runFacadeFrame(state);
  assert.equal(committed, 5);
  assert.ok(freed.includes(3) && freed.includes(5));
  assert.equal(element.busy, false);
  element.free();
});

test("a superseded selection rejection cannot swallow a newer selection mutation", async () => {
  const old = deferred(), reads = [], freed = [];
  const { kernel, element, state } = await completedReplayFixture({ readRange(range) {
    reads.push(range.offset);
    return range.offset === 3 ? old.promise : new Float32Array([5]);
  } });
  let desired = 3, committed = 1;
  kernel.set_picked_points = (json) => { desired = JSON.parse(json); };
  kernel.stream_selection_request_ranges = () => desired === committed
    ? { status: "complete", free() {} } : selectionRequest(desired, freed);
  kernel.stream_selection_submit_ranges = (request) => { committed = request.index; };
  kernel.discard_stream_selection_request = () => assert.fail("superseded ticket must not abandon current selection");
  runFacadeFrame(state);
  old.reject(new Error("old selection failed"));
  await flushTasks();
  element.set_picked_points("5");
  runFacadeFrame(state);
  await flushTasks();
  runFacadeFrame(state);
  assert.deepEqual(reads, [3, 5]);
  assert.equal(committed, 5);
  element.free();
});

test("selection borrows typed-array ranges without copying", async () => {
  const { kernel, element, values, state } = await completedReplayFixture({ typed: true });
  const freed = [];
  let complete = false;
  kernel.stream_selection_request_ranges = () => complete
    ? { status: "complete", free() {} } : selectionRequest(4, freed);
  kernel.stream_selection_submit_ranges = (_request, _ids, _revisions, _lengths, offsets, chunks) => {
    assert.deepEqual([...offsets], [4]);
    assert.equal(chunks[0].buffer, values.buffer);
    assert.deepEqual([...chunks[0]], [4]);
    complete = true;
  };
  runFacadeFrame(state);
  await flushTasks();
  runFacadeFrame(state);
  assert.equal(complete, true);
  element.free();
  assert.ok(freed.includes(4));
});

test("disconnect releases an unresolved selected-row request and ignores its late reply", async () => {
  const pending = deferred(), freed = [];
  const { kernel, element, state } = await completedReplayFixture({ readRange() { return pending.promise; } });
  kernel.stream_selection_request_ranges = () => selectionRequest(3, freed);
  kernel.stream_selection_submit_ranges = () => assert.fail("disconnected selection must not submit");
  runFacadeFrame(state);
  element.free();
  assert.deepEqual(freed, [3]);
  pending.resolve(new Float32Array([3]));
  await flushTasks();
  assert.equal(state.timers.size, 0);
});

test("selection backpressure uses a bounded wake rather than spinning on an already-complete fence", async () => {
  const { kernel, element, state } = await completedReplayFixture({ typed: true });
  let requests = 0, fences = 0;
  kernel.stream_selection_request_ranges = () => { requests++; return { status: "backpressure", free() {} }; };
  kernel.streaming_gpu_ready = () => { fences++; return Promise.resolve(); };
  runFacadeFrame(state);
  await flushTasks();
  for (let i = 0; i < 5; i++) runFacadeFrame(state);
  assert.equal(requests, 1);
  assert.equal(fences, 1);
  assert.equal(state.timers.size, 1);
  element.free();
  assert.equal(state.timers.size, 0);
});

test("selection provider failure abandons only its ticket and leaves data execution available", async () => {
  const failure = new Error("selected range unavailable");
  const { kernel, element, state } = await completedReplayFixture({ readRange() { throw failure; } });
  const errors = [];
  element.addEventListener("figgy-error", (event) => errors.push(event));
  const freed = [];
  let abandoned = 0, requests = 0;
  kernel.stream_selection_request_ranges = () => { requests++; return selectionRequest(3, freed); };
  kernel.discard_stream_selection_request = (request) => { assert.equal(request.index, 3); abandoned++; };
  kernel.stream_selection_submit_ranges = () => assert.fail("failed input must not be submitted");
  runFacadeFrame(state);
  await flushTasks();
  runFacadeFrame(state);
  runFacadeFrame(state);
  assert.equal(abandoned, 1);
  assert.equal(requests, 1, "failed selection must not loop without a new mutation");
  assert.ok(errors.some((event) => event.type === "figgy-error"
    && event.detail.error === failure && event.detail.operation === "stream_selection"));
  assert.equal(kernel.calls.filter(([name]) => name === "cancel_streaming_and_wait").length, 0);
  assert.equal(element.busy, false);
  element.free();
});

test("exclusive operations suspend a selected-row reservation and resume it in a hidden tab", async () => {
  const oldRead = deferred(), warm = deferred(), freed = [];
  let reads = 0;
  const { kernel, element, state } = await completedReplayFixture({ readRange() {
    return ++reads === 1 ? oldRead.promise : new Float32Array([3]);
  } });
  let reserved = false, committed = false;
  kernel.stream_selection_request_ranges = () => {
    if (committed) return { status: "complete", free() {} };
    reserved = true;
    return selectionRequest(3, freed);
  };
  kernel.suspend_stream_selection = () => { reserved = false; };
  kernel.stream_selection_submit_ranges = () => { committed = true; reserved = false; };
  kernel.prewarmImpl = () => {
    assert.equal(reserved, false, "exclusive GPU work must not wait behind a held source ticket");
    return warm.promise;
  };
  runFacadeFrame(state);
  const operation = element.prewarm_gpu_picking();
  assert.equal(element.busy, true);
  oldRead.resolve(new Float32Array([3]));
  await flushTasks();
  for (let i = 0; i < 8 && state.messages.length; i++) state.messages.shift()();
  assert.equal(committed, false);
  warm.resolve();
  await operation;
  // No rAF callback is invoked after the exclusive operation settles.
  for (let i = 0; i < 12; i++) {
    if (state.messages.length) state.messages.shift()();
    await flushTasks();
  }
  assert.equal(committed, true);
  assert.equal(reads, 2);
  assert.equal(element.busy, false);
  element.free();
});

test("stream auxiliary provider receives only bounded ranges and waits on GPU backpressure", async () => {
  const reads = [];
  const { kernel, element } = await completedReplayFixture({
    readRange(request) { reads.push(request); return new Float32Array([3, 4]); },
  });
  const gpu = deferred();
  let initial = true;
  const request = kernel.stream_operation_request_ranges;
  kernel.stream_operation_request_ranges = (handle) => {
    if (initial) { initial = false; return { status: "backpressure" }; }
    return request(handle);
  };
  kernel.streaming_gpu_ready = () => gpu.promise;
  const exported = element.export_png(2);
  await flushTasks();
  assert.equal(reads.length, 0);
  assert.equal(element.busy, true);
  gpu.resolve();
  await exported;
  assert.deepEqual(JSON.parse(JSON.stringify(reads)), [
    { id: "x", revision: 2, offset: 3, length: 2, encoding: "f32" },
  ]);
});

test("free interrupts an unresolved stream export provider and drains before disposing", async () => {
  const read = deferred();
  const drained = deferred();
  const { kernel, element } = await completedReplayFixture({ readRange: () => read.promise });
  kernel.cancel_stream_operation_and_wait = () => {
    kernel.calls.push(["cancel-aux"]);
    return drained.promise;
  };
  const exported = element.export_png();
  const rejected = assert.rejects(exported, (error) => error.name === "AbortError");
  await flushTasks();
  disconnect(element);
  await flushTasks();
  assert.equal(kernel.calls.filter(([name]) => name === "cancel-aux").length, 1);
  assert.equal(kernel.freeCalls, 0);
  drained.resolve();
  await rejected;
  assert.equal(kernel.freeCalls, 1);
  read.resolve(new Float32Array([3, 4]));
  await flushTasks();
  assert.equal(kernel.calls.filter(([name]) => name === "submit-aux").length, 0);
});

test("invalid stream export input is rejected and its reservation is drained", async () => {
  const { kernel, element } = await completedReplayFixture({
    readRange: () => new Float64Array([3, 4]),
  });
  await assert.rejects(element.export_png(), /Float32Array/);
  assert.equal(kernel.calls.filter(([name]) => name === "submit-aux").length, 0);
  assert.equal(kernel.calls.filter(([name]) => name === "cancel-aux").length, 1);
  assert.equal(element.busy, false);
});

test("stream auxiliary cleanup errors do not replace the original provider failure", async () => {
  const primary = new Error("provider failed");
  const cleanup = new Error("drain failed");
  const { kernel, element } = await completedReplayFixture({
    readRange: () => Promise.reject(primary),
  });
  const cleanupEvents = [];
  element.addEventListener("figgy-error", (event) => cleanupEvents.push(event.detail));
  kernel.cancel_stream_operation_and_wait = () => Promise.reject(cleanup);
  await assert.rejects(element.export_png(), (error) => error === primary);
  assert.equal(cleanupEvents.length, 1);
  assert.equal(cleanupEvents[0].error, cleanup);
  assert.equal(kernel.calls.filter(([name]) => name === "handle-free").length, 1);
  assert.equal(element.busy, false);
});

test("queued resize cleanup preserves a stream auxiliary provider failure", async () => {
  const read = deferred();
  const primary = new Error("provider failed before queued resize");
  const cleanup = new Error("queued resize failed");
  const { kernel, element } = await completedReplayFixture({ readRange: () => read.promise });
  const cleanupEvents = [];
  element.addEventListener("figgy-error", (event) => cleanupEvents.push(event.detail));
  kernel.resize = (width, height) => {
    kernel.calls.push(["resize", width, height]);
    throw cleanup;
  };
  const exported = element.export_png();
  const rejected = assert.rejects(exported, (error) => error === primary);
  await flushTasks();
  element.setRect(800, 600);
  element.resize();
  assert.equal(kernel.calls.filter(([name]) => name === "resize").length, 0);
  read.reject(primary);
  await rejected;
  assert.equal(cleanupEvents.length, 1);
  assert.equal(cleanupEvents[0].error, cleanup);
  assert.equal(cleanupEvents[0].operation, "operation_cleanup");
  assert.equal(kernel.calls.filter(([name]) => name === "resize").length, 1);
  assert.equal(kernel.calls.filter(([name]) => name === "cancel-aux").length, 1);
  assert.equal(kernel.calls.filter(([name]) => name === "handle-free").length, 1);
  assert.equal(element.busy, false);
});

test("synchronous provider throw still observes earlier pending range rejections", async () => {
  const pending = deferred();
  const primary = new Error("second provider throws synchronously");
  const { kernel, element } = await completedReplayFixture({
    extraColumn: { id: "y", revision: 2, length: 8, encoding: "f32" },
    readRange({ id }) {
      if (id === "x") return pending.promise;
      throw primary;
    },
  });
  kernel.stream_operation_request_ranges = () => ({
    status: "ready", source_ids: ["x", "y"], source_revisions: [2, 2], source_lengths: [8, 8],
    offsets: [3, 3], lengths: [2, 2], encodings: ["f32", "f32"],
  });
  await assert.rejects(element.export_png(), (error) => error === primary);
  pending.reject(new Error("first provider rejects later"));
  await flushTasks();
  assert.equal(element.busy, false);
  assert.equal(kernel.calls.filter(([name]) => name === "cancel-aux").length, 1);
});

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
  await waitForIdle(element);
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
  await waitForIdle(element);
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
    assert.equal(element.busy, true, "the current connection owns the operation token");

    second.resolve(currentKernel);
    await currentReady;
    await waitForIdle(element);
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
    await waitForIdle(element);
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
    await waitForIdle(element);
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
  await waitForIdle(element);
  assert.equal(readyEvents, 2);
  assert.equal(firstKernel.freeCalls, 1);
  assert.equal(element.kernel, currentKernel);
  assert.equal(state.observers.filter((observer) => observer.target === element).length, 1);
  assert.equal(state.rafs.size, 1, "stale ready callback must not schedule its own rAF");
});

test("facade relays the exact raw startup contract as bubbling composed events", async () => {
  const order = [];
  const kernel = makeKernel("progress-contract", {
    prewarmImpl: () => {
      order.push({ kind: "prewarm" });
      return Promise.resolve();
    },
  });
  const { Element } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  element.addEventListener("figgy-init-progress", (event) => {
    order.push({
      kind: "progress",
      detail: { ...event.detail },
      bubbles: event.bubbles,
      composed: event.composed,
    });
  });
  element.addEventListener("figgy-ready", () => {
    order.push({ kind: "ready" });
  });

  connect(element);
  await element.ready;
  await waitForIdle(element);

  const progress = order.filter(({ kind }) => kind === "progress");
  const expected = RAW_STARTUP_STAGES.flatMap(([scope, stage]) => [
    { scope, stage, phase: "started" },
    { scope, stage, phase: "finished" },
  ]);
  assert.deepEqual(progress.map(({ detail }) => detail), expected);
  assert.ok(progress.every(({ bubbles, composed }) => bubbles && composed));

  const firstFrameFinished = order.findIndex(({ kind, detail }) => (
    kind === "progress"
      && detail.stage === "first frame"
      && detail.phase === "finished"
  ));
  const ready = order.findIndex(({ kind }) => kind === "ready");
  const prewarm = order.findIndex(({ kind }) => kind === "prewarm");
  assert.ok(firstFrameFinished < ready, "first-frame finished must precede ready");
  assert.ok(ready < prewarm, "background prewarm must follow ready publication");
});

test("first-frame completion and ready publish before recoverable background prewarm", async () => {
  const firstPrewarm = deferred();
  const order = [];
  let prewarmAttempts = 0;
  let nestedExport;
  let element;
  const kernel = makeKernel("background-prewarm", {
    prewarmImpl: () => {
      prewarmAttempts += 1;
      order.push(`prewarm-${prewarmAttempts}`);
      return prewarmAttempts === 1 ? firstPrewarm.promise : Promise.resolve();
    },
    onRelease: () => {
      nestedExport = element.export_png();
      nestedExport.catch(() => {});
    },
  });
  const { Element, state } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  element = new Element();
  const errors = [];
  element.addEventListener("figgy-init-progress", ({ detail }) => {
    if (detail.stage === "first frame" && detail.phase === "finished") {
      order.push("first-frame-finished");
    }
  });
  element.addEventListener("figgy-ready", () => {
    order.push("ready-event");
    element.canvas.emit("pointerdown", { pointerId: 7, clientX: 10, clientY: 20 });
  });
  element.addEventListener("figgy-error", ({ detail }) => errors.push(detail));

  connect(element);
  await element.ready;
  order.push("ready-promise");
  await flushTasks();

  assert.deepEqual(order, [
    "first-frame-finished",
    "ready-event",
    "prewarm-1",
    "ready-promise",
  ]);
  assert.equal(element.busy, true);
  assert.equal(kernel.calls.filter(([name]) => name === "press").length, 1);

  for (const callback of [...state.rafs.values()]) callback(0);
  assert.equal(kernel.calls.filter(([name]) => name === "frame").length, 0);
  assert.throws(() => element.get_config(), /busy/);

  element.setRect(700, 500);
  element.resize();
  element.setRect(900, 700);
  element.resize();
  element.canvas.emit("pointerup");
  element.canvas.emit("pointercancel");
  assert.equal(kernel.calls.filter(([name]) => name === "release").length, 0);
  assert.equal(kernel.calls.filter(([name]) => name === "resize").length, 0);

  const prewarmError = new Error("synthetic picker prewarm failure");
  firstPrewarm.reject(prewarmError);
  await flushTasks();
  await assert.rejects(nestedExport, /busy/);

  assert.equal(element.busy, false);
  assert.equal(await element.ready, element, "ready stays fulfilled after picker failure");
  assert.equal(errors.length, 1);
  assert.equal(errors[0].error, prewarmError);
  assert.equal(errors[0].operation, "prewarm_gpu_picking");
  assert.equal(errors[0].recoverable, true);
  assert.deepEqual(kernel.calls.filter(([name]) => name === "resize"), [
    ["resize", 900, 700],
  ]);
  assert.equal(kernel.calls.filter(([name]) => name === "release").length, 1);

  await element.prewarm_gpu_picking();
  await element.prewarm_gpu_picking();
  assert.equal(prewarmAttempts, 3, "retries delegate to renderer-owned idempotence");

  const frameCount = kernel.calls.filter(([name]) => name === "frame").length;
  for (const callback of [...state.rafs.values()]) callback(1);
  assert.ok(kernel.calls.filter(([name]) => name === "frame").length > frameCount);
});

test("all async facade borrows share the operation gate", async () => {
  const firstFrame = deferred();
  const extent = deferred();
  const pick = deferred();
  const explicitPrewarm = deferred();
  let prewarmCalls = 0;
  const kernel = makeKernel("async-gate", {
    firstFrameImpl: () => firstFrame.promise,
    ensureExtentImpl: () => extent.promise,
    prewarmImpl: () => {
      prewarmCalls += 1;
      return prewarmCalls === 1 ? Promise.resolve() : explicitPrewarm.promise;
    },
  });
  kernel.pickImpl = () => pick.promise;
  kernel.pickDataImpl = () => pick.promise;
  const { Element, state } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  await flushTasks();

  const cases = [
    [() => element.first_frame_ready(), firstFrame],
    [() => element.ensure_extent_engine(), extent],
    [() => element.pick_point(1, 2, 3), pick],
    [() => element.pick_data(1, 2, 3), pick],
    [() => element.prewarm_gpu_picking(), explicitPrewarm],
  ];
  for (const [start, operation] of cases) {
    const frameCount = kernel.calls.filter(([name]) => name === "frame").length;
    const pending = start();
    assert.equal(element.busy, true);
    assert.throws(() => element.set_title("blocked"), /busy/);
    for (const callback of [...state.rafs.values()]) callback(0);
    assert.equal(kernel.calls.filter(([name]) => name === "frame").length, frameCount);
    operation.resolve(undefined);
    await pending;
    assert.equal(element.busy, false);
  }
});

test("stale background prewarm settles and frees only its disconnected generation", async () => {
  const oldPrewarm = deferred();
  const newPrewarm = deferred();
  const oldKernel = makeKernel("old-prewarm", {
    prewarmImpl: () => oldPrewarm.promise,
  });
  const newKernel = makeKernel("new-prewarm", {
    prewarmImpl: () => newPrewarm.promise,
  });
  const kernels = [oldKernel, newKernel];
  const { Element } = await loadFacade({
    createImpl: () => Promise.resolve(kernels.shift()),
  });
  const element = new Element();
  let errorEvents = 0;
  element.addEventListener("figgy-error", () => {
    errorEvents += 1;
  });

  connect(element);
  await element.ready;
  await flushTasks();
  assert.equal(element.busy, true);
  disconnect(element);
  assert.equal(oldKernel.freeCalls, 0);

  const newReady = element.ready;
  connect(element);
  await newReady;
  await flushTasks();
  assert.equal(element.busy, true);
  assert.throws(() => element.kernel, /busy/);

  oldPrewarm.reject(new Error("stale prewarm failed"));
  await flushTasks();
  assert.equal(oldKernel.freeCalls, 1);
  assert.equal(newKernel.freeCalls, 0);
  assert.equal(element.busy, true, "stale settle cannot release the new operation");
  assert.equal(errorEvents, 0, "stale failure is not reported on the new generation");

  newPrewarm.resolve();
  await flushTasks();
  assert.equal(element.busy, false);
  assert.equal(element.kernel, newKernel);
  disconnect(element);
  assert.equal(newKernel.freeCalls, 1);
});

test("auto_fit_all holds busy so rAF does not call frame", async () => {
  const fit = deferred();
  const kernel = makeKernel("fit", {
    autoFitImpl: () => fit.promise,
  });
  const { Element, state } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);

  const pending = element.auto_fit_all(0.05);
  assert.equal(element.busy, true);
  for (const callback of [...state.rafs.values()]) {
    callback(0);
  }
  await flushTasks();
  assert.deepEqual(
    kernel.calls.filter(([name]) => name === "frame"),
    [],
    "frame must not run while auto_fit_all is pending",
  );
  await assert.rejects(element.auto_fit_all(0.1), /busy/);
  fit.resolve();
  await pending;
  assert.equal(element.busy, false);
  assert.deepEqual(kernel.calls.filter(([name]) => name === "auto_fit_all"), [
    ["auto_fit_all", 0.05],
  ]);
});

test("new synchronous facade APIs forward values and preserve raw failures", async () => {
  const kernel = makeKernel("sync-forwarding");
  const fitResult = { fitted: true };
  const contourError = new Error("raw contour failure");
  kernel.auto_fit_colorbar = (padding) => {
    kernel.calls.push(["auto_fit_colorbar", padding]);
    return fitResult;
  };
  kernel.set_contour_nice_levels = (seriesId, targetCount, useColormapColors) => {
    kernel.calls.push([
      "set_contour_nice_levels",
      seriesId,
      targetCount,
      useColormapColors,
    ]);
    if (seriesId === "broken") throw contourError;
    return 7;
  };
  kernel.series_draw_info = (seriesId) => {
    kernel.calls.push(["series_draw_info", seriesId]);
    return JSON.stringify({
      drawn_count: 12,
      cols: 4,
      rows: 3,
      truncated: false,
    });
  };
  kernel.set_picked_data = (json) => {
    kernel.calls.push(["set_picked_data", json]);
    return "picked-data-updated";
  };
  kernel.set_colorbar_axis = (json) => {
    kernel.calls.push(["set_colorbar_axis", json]);
    return "colorbar-axis-updated";
  };
  kernel.set_colorbar_title = (title) => {
    kernel.calls.push(["set_colorbar_title", title]);
    return "colorbar-title-updated";
  };
  const { Element } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);

  assert.equal(element.auto_fit_colorbar(0.125), fitResult);
  assert.equal(element.set_contour_nice_levels("contour", 7, true), 7);
  assert.deepEqual(
    JSON.parse(JSON.stringify(element.series_draw_info("contour"))),
    { drawn_count: 12, cols: 4, rows: 3, truncated: false },
  );
  assert.equal(element.set_picked_data("null"), "picked-data-updated");
  assert.equal(element.set_colorbar_axis('{"tick":"Both"}'), "colorbar-axis-updated");
  assert.equal(element.set_colorbar_title("intensity"), "colorbar-title-updated");
  assert.deepEqual(kernel.calls.filter(([name]) => (
    name === "auto_fit_colorbar"
      || name === "set_contour_nice_levels"
      || name === "series_draw_info"
      || name === "set_picked_data"
      || name === "set_colorbar_axis"
      || name === "set_colorbar_title"
  )), [
    ["auto_fit_colorbar", 0.125],
    ["set_contour_nice_levels", "contour", 7, true],
    ["series_draw_info", "contour"],
    ["set_picked_data", "null"],
    ["set_colorbar_axis", '{"tick":"Both"}'],
    ["set_colorbar_title", "intensity"],
  ]);
  assert.throws(
    () => element.set_contour_nice_levels("broken", 5, false),
    (error) => error === contourError,
  );
  kernel.series_draw_info = () => "{not json";
  assert.throws(
    () => element.series_draw_info("contour"),
    (error) => error?.name === "SyntaxError",
  );
});

test("automatic streaming facade forwards the exact host bindings", async () => {
  const kernel = makeKernel("auto-stream-forwarding");
  const requestResult = { status: "started" };
  const stepResult = { status: "submitted" };
  const revisions = new Float64Array([7, 7]);
  const sources = [new Float32Array([1, 2]), new Float32Array([3, 4])];
  kernel.request_auto_streaming_chart = (chunkSize) => {
    kernel.calls.push(["request_auto_streaming_chart", chunkSize]);
    return requestResult;
  };
  kernel.auto_stream_chart_step = (ids, suppliedRevisions, suppliedSources) => {
    kernel.calls.push([
      "auto_stream_chart_step",
      ids,
      suppliedRevisions,
      suppliedSources,
    ]);
    return stepResult;
  };
  kernel.interrupt_render = () => {
    kernel.calls.push(["interrupt_render"]);
    return "stream_cancel_queued";
  };
  const { Element } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);

  assert.equal(element.request_auto_streaming_chart(4096), requestResult);
  assert.equal(
    element.auto_stream_chart_step(["x", "y"], revisions, sources),
    stepResult,
  );
  assert.equal(element.interrupt_render(), "stream_cancel_queued");
  assert.deepEqual(kernel.calls.slice(-3), [
    ["request_auto_streaming_chart", 4096],
    ["auto_stream_chart_step", ["x", "y"], revisions, sources],
    ["interrupt_render"],
  ]);
});

test("high-level streaming job owns pumping, progress, and completion", async () => {
  const kernel = makeKernel("owned-stream-executor");
  const x = new Float32Array([0, 1, 2, 3]);
  const y = new Float64Array([4, 5, 6, 7]);
  let requestCount = 0;
  const steps = [
    { status: "submitted", submitted_primitives: 2, total_primitives: 4 },
    { status: "all_submitted", submitted_primitives: 4, total_primitives: 4 },
  ];
  kernel.register_streaming_columns = (ids, revisions, sources) => {
    kernel.calls.push(["register_streaming_columns", ids, revisions, sources]);
  };
  kernel.request_auto_streaming_chart = (chunkSize) => {
    kernel.calls.push(["request_auto_streaming_chart", chunkSize]);
    requestCount += 1;
    if (requestCount === 1) {
      return {
        status: "started",
        source_ids: ["x", "y"],
        source_revisions: [1, 1],
      };
    }
    return requestCount <= 3
      ? { status: "active", source_ids: [], source_revisions: [] }
      : { status: "complete", source_ids: [], source_revisions: [] };
  };
  kernel.auto_stream_chart_step = (ids, revisions, sources) => {
    kernel.calls.push(["auto_stream_chart_step", ids, revisions, sources]);
    return steps.shift();
  };
  const { Element, state } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);

  const progress = [];
  const job = element.render_streaming_chart({
    columns: [
      { id: "x", revision: 1, values: x },
      { id: "y", revision: 1, values: y },
    ],
    maxPrimitivesPerChunk: 2,
    onProgress: (event) => progress.push(event.status),
  });
  assert.equal(element.busy, false, "streaming must not block config and series calls");
  assert.equal(job.status, "running");

  for (let frame = 0; frame < 2; frame += 1) {
    const [id, callback] = [...state.rafs.entries()].at(-1);
    state.rafs.delete(id);
    callback(frame);
  }
  assert.deepEqual(JSON.parse(JSON.stringify(await job.done)), {
    status: "complete",
    submittedPrimitives: 4,
    totalPrimitives: 4,
  });
  assert.equal(job.status, "complete");
  assert.deepEqual(progress, ["running", "running", "waiting_gpu", "complete"]);
  const registration = kernel.calls.find(([name]) => name === "register_streaming_columns");
  assert.deepEqual(JSON.parse(JSON.stringify(registration.slice(0, 3))), [
    "register_streaming_columns",
    ["x", "y"],
    [1, 1],
  ]);
  assert.equal(registration[3][0], x);
  assert.equal(registration[3][1], y);
  const supplied = kernel.calls.filter(([name]) => name === "auto_stream_chart_step");
  assert.equal(supplied.length, 2);
  for (const [, ids, revisions, sources] of supplied) {
    assert.deepEqual(JSON.parse(JSON.stringify([ids, revisions])), [["x", "y"], [1, 1]]);
    assert.equal(sources[0], x);
    assert.equal(sources[1], y);
  }

  const repeated = element.render_streaming_chart({
    columns: [
      { id: "x", revision: 1, values: x },
      { id: "y", revision: 1, values: y },
    ],
    maxPrimitivesPerChunk: 2,
  });
  assert.deepEqual(JSON.parse(JSON.stringify(await repeated.done)), {
    status: "complete",
    submittedPrimitives: 0,
    totalPrimitives: 0,
  });
  assert.equal(kernel.calls.filter(([name]) => name === "auto_stream_chart_step").length, 2);

  const requestsBeforeResize = requestCount;
  element.setRect(800, 600);
  element.resize();
  assert.equal(requestCount, requestsBeforeResize + 1);
  await repeated.cancel();
  element.setRect(900, 700);
  element.resize();
  assert.equal(
    requestCount,
    requestsBeforeResize + 1,
    "cancelling a completed job must release its retained source references",
  );
});

test("range-provider streaming fetches only renderer-selected chunks and owns completion", async () => {
  const kernel = makeKernel("range-provider-executor");
  const requested = [];
  let rangeStep = 0;
  let renderRequests = 0;
  kernel.register_streaming_column_sources = (ids, revisions, lengths, encodings) => {
    kernel.calls.push([
      "register_streaming_column_sources",
      ids,
      revisions,
      lengths,
      encodings,
    ]);
  };
  kernel.request_auto_streaming_chart = () => {
    renderRequests += 1;
    return renderRequests === 1 ? {
      status: "started",
      source_ids: ["x", "y"],
      source_revisions: [1, 1],
    } : {
      status: "complete",
      source_ids: [],
      source_revisions: [],
    };
  };
  kernel.auto_stream_chart_request_ranges = () => {
    rangeStep += 1;
    if (rangeStep === 1) {
      return {
        status: "ready",
        source_ids: ["x", "y"],
        source_revisions: [1, 1],
        source_lengths: [8, 8],
        offsets: [2, 2],
        lengths: [3, 3],
        encodings: ["f32", "f64"],
        submitted_primitives: 2,
        total_primitives: 7,
      };
    }
    if (rangeStep === 2) {
      return {
        status: "all_submitted",
        source_ids: [],
        source_revisions: [],
        source_lengths: [],
        offsets: [],
        lengths: [],
        encodings: [],
        submitted_primitives: 7,
        total_primitives: 7,
      };
    }
    return {
      status: "complete",
      source_ids: [],
      source_revisions: [],
      source_lengths: [],
      offsets: [],
      lengths: [],
      encodings: [],
      submitted_primitives: 0,
      total_primitives: 0,
    };
  };
  kernel.auto_stream_chart_submit_ranges = (
    ids, revisions, sourceLengths, offsets, sources,
  ) => {
    kernel.calls.push([
      "auto_stream_chart_submit_ranges",
      ids,
      revisions,
      sourceLengths,
      offsets,
      sources,
    ]);
    return { status: "submitted", submitted_primitives: 7, total_primitives: 7 };
  };
  const { Element, state } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);

  const job = element.render_chart({
    columns: [
      { id: "x", revision: 1, length: 8, encoding: "f32" },
      { id: "y", revision: 1, length: 8, encoding: "f64" },
    ],
    maxPrimitivesPerChunk: 2,
    readRange(request) {
      requested.push(request);
      return request.encoding === "f32"
        ? new Float32Array(request.length).fill(3)
        : Promise.resolve(new Float64Array(request.length).fill(4));
    },
  });
  const runFrame = (time) => {
    const [id, callback] = [...state.rafs.entries()].at(-1);
    state.rafs.delete(id);
    callback(time);
  };
  runFrame(0);
  await flushTasks();
  runFrame(1);
  runFrame(2);

  assert.deepEqual(JSON.parse(JSON.stringify(await job.done)), {
    status: "complete",
    submittedPrimitives: 0,
    totalPrimitives: 0,
  });
  assert.deepEqual(JSON.parse(JSON.stringify(requested)), [
    { id: "x", revision: 1, offset: 2, length: 3, encoding: "f32" },
    { id: "y", revision: 1, offset: 2, length: 3, encoding: "f64" },
  ]);
  const submit = kernel.calls.find(([name]) => name === "auto_stream_chart_submit_ranges");
  assert.deepEqual(JSON.parse(JSON.stringify(submit.slice(0, 5))), [
    "auto_stream_chart_submit_ranges",
    ["x", "y"],
    [1, 1],
    [8, 8],
    [2, 2],
  ]);
  assert.equal(submit[5][0] instanceof Float32Array, true);
  assert.equal(submit[5][1] instanceof Float64Array, true);

  const repeated = element.render_chart({
    columns: [
      { id: "x", revision: 1, length: 8, encoding: "f32" },
      { id: "y", revision: 1, length: 8, encoding: "f64" },
    ],
    maxPrimitivesPerChunk: 2,
    readRange() {
      throw new Error("completed identical input must not be read again");
    },
  });
  assert.deepEqual(JSON.parse(JSON.stringify(await repeated.done)), {
    status: "complete",
    submittedPrimitives: 0,
    totalPrimitives: 0,
  });
  assert.equal(renderRequests, 2);
  assert.equal(requested.length, 2);
});

for (const dprOnly of [false, true]) {
test(`completed range-provider stream replays exact source ranges after ${dprOnly ? "DPR change" : "resize"}`, async () => {
  const kernel = makeKernel("range-provider-resize-replay");
  let renderRequests = 0;
  let rangeReady = false;
  let reads = 0;
  let submits = 0;
  kernel.register_streaming_column_sources = () => {};
  kernel.request_auto_streaming_chart = () => {
    renderRequests += 1;
    rangeReady = true;
    return {
      status: "started",
      source_ids: ["x"],
      source_revisions: [1],
    };
  };
  kernel.auto_stream_chart_request_ranges = () => {
    if (!rangeReady) {
      return {
        status: "complete",
        source_ids: [], source_revisions: [], source_lengths: [],
        offsets: [], lengths: [], encodings: [],
        submitted_primitives: 0, total_primitives: 0,
      };
    }
    return {
      status: "ready",
      source_ids: ["x"],
      source_revisions: [1],
      source_lengths: [8],
      offsets: [2],
      lengths: [2],
      encodings: ["f32"],
      submitted_primitives: 0,
      total_primitives: 7,
    };
  };
  kernel.auto_stream_chart_submit_ranges = () => {
    submits += 1;
    rangeReady = false;
    return { status: "submitted", submitted_primitives: 7, total_primitives: 7 };
  };
  const { Element, state, browserWindow } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const runFrame = (time) => {
    const [id, callback] = [...state.rafs.entries()].at(-1);
    state.rafs.delete(id);
    callback(time);
  };

  const job = element.render_chart({
    columns: [{ id: "x", revision: 1, length: 8, encoding: "f32" }],
    readRange(request) {
      reads += 1;
      return new Float32Array(request.length).fill(reads);
    },
  });
  runFrame(0);
  await flushTasks();
  runFrame(1);
  await job.done;
  assert.equal(renderRequests, 1);
  assert.equal(reads, 1);
  assert.equal(submits, 1);

  if (dprOnly) {
    browserWindow.devicePixelRatio = 2;
    runFrame(2);
  } else {
    element.setRect(800, 600);
    element.resize();
  }
  assert.deepEqual(kernel.calls.filter(([name]) => name === "resize").at(-1), [
    "resize", dprOnly ? 1280 : 800, dprOnly ? 960 : 600,
  ]);
  assert.equal(renderRequests, 2, "resize must start an exact replay without host pumping");
  if (!dprOnly) runFrame(2);
  await flushTasks();
  runFrame(3);
  await flushTasks();

  assert.equal(reads, 2, "the retained provider must supply the resized target");
  assert.equal(submits, 2);
});

test(`active range-provider ${dprOnly ? "DPR change" : "resize"} restarts the same job and drops the old read`, async () => {
  const kernel = makeKernel("range-provider-active-resize");
  const oldRead = deferred();
  let renderRequests = 0;
  let ready = false;
  let reads = 0;
  const submitted = [];
  kernel.register_streaming_column_sources = () => {};
  kernel.request_auto_streaming_chart = () => {
    renderRequests += 1;
    ready = true;
    return {
      status: "started",
      source_ids: ["x"],
      source_revisions: [1],
    };
  };
  kernel.auto_stream_chart_request_ranges = () => {
    if (!ready) {
      return {
        status: "complete",
        source_ids: [], source_revisions: [], source_lengths: [],
        offsets: [], lengths: [], encodings: [],
        submitted_primitives: 0, total_primitives: 0,
      };
    }
    return {
      status: "ready",
      source_ids: ["x"],
      source_revisions: [1],
      source_lengths: [8],
      offsets: [0],
      lengths: [2],
      encodings: ["f32"],
      submitted_primitives: 0,
      total_primitives: 7,
    };
  };
  kernel.auto_stream_chart_submit_ranges = (_ids, _revisions, _lengths, _offsets, chunks) => {
    submitted.push([...chunks[0]]);
    ready = false;
    return { status: "submitted", submitted_primitives: 7, total_primitives: 7 };
  };
  const { Element, state, browserWindow } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const runFrame = (time) => {
    const [id, callback] = [...state.rafs.entries()].at(-1);
    state.rafs.delete(id);
    callback(time);
  };

  const job = element.render_chart({
    columns: [{ id: "x", revision: 1, length: 8, encoding: "f32" }],
    readRange() {
      reads += 1;
      return reads === 1 ? oldRead.promise : new Float32Array([2, 2]);
    },
  });
  const executionId = job.executionId;
  runFrame(0);
  assert.equal(reads, 1);

  if (dprOnly) {
    browserWindow.devicePixelRatio = 2;
  } else {
    element.setRect(800, 600);
    element.resize();
  }
  runFrame(1);
  assert.deepEqual(kernel.calls.filter(([name]) => name === "resize").at(-1), [
    "resize", dprOnly ? 1280 : 800, dprOnly ? 960 : 600,
  ]);
  await flushTasks();
  runFrame(2);
  await job.done;

  oldRead.resolve(new Float32Array([1, 1]));
  await flushTasks();
  assert.equal(job.executionId, executionId);
  assert.equal(renderRequests, 2);
  assert.deepEqual(submitted, [[2, 2]], "late data for the old target must be discarded");
});
}

test("cancelled range-provider work discards late async data without submitting it", async () => {
  const kernel = makeKernel("range-provider-cancel");
  const pending = deferred();
  let cancelled = false;
  let submits = 0;
  let renderRequests = 0;
  kernel.register_streaming_column_sources = () => {};
  kernel.request_auto_streaming_chart = () => {
    renderRequests += 1;
    return {
      status: "started",
      source_ids: ["x"],
      source_revisions: [1],
    };
  };
  kernel.auto_stream_chart_request_ranges = () => {
    if (cancelled) throw new Error("automatic streaming chart has no active execution");
    return {
      status: "ready",
      source_ids: ["x"],
      source_revisions: [1],
      source_lengths: [8],
      offsets: [0],
      lengths: [2],
      encodings: ["f32"],
      submitted_primitives: 0,
      total_primitives: 7,
    };
  };
  kernel.auto_stream_chart_submit_ranges = () => {
    submits += 1;
    return { status: "submitted", submitted_primitives: 2, total_primitives: 7 };
  };
  kernel.interrupt_render = () => {
    cancelled = true;
    return "stream_cancel_queued";
  };
  const { Element, state } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);

  const job = element.render_chart({
    columns: [{ id: "x", revision: 1, length: 8, encoding: "f32" }],
    readRange: () => pending.promise,
  });
  let [id, callback] = [...state.rafs.entries()].at(-1);
  state.rafs.delete(id);
  callback(0);
  const cancellation = job.cancel();
  assert.equal(typeof cancellation.then, "function");
  [id, callback] = [...state.rafs.entries()].at(-1);
  state.rafs.delete(id);
  callback(1);
  pending.resolve(new Float32Array([1, 2]));
  await flushTasks();
  await cancellation;

  assert.equal(submits, 0);
  assert.equal(job.status, "cancelled");
  assert.deepEqual(JSON.parse(JSON.stringify(await job.done)), {
    status: "cancelled",
    submittedPrimitives: 0,
    totalPrimitives: 7,
  });
  element.setRect(800, 600);
  element.resize();
  assert.equal(renderRequests, 1, "cancel must release the replay provider reference");
});

test("range-provider type mismatch fails the job and never submits", async () => {
  const kernel = makeKernel("range-provider-invalid-type");
  let submits = 0;
  let renderRequests = 0;
  kernel.register_streaming_column_sources = () => {};
  kernel.request_auto_streaming_chart = () => {
    renderRequests += 1;
    return { status: "started", source_ids: ["x"], source_revisions: [1] };
  };
  kernel.auto_stream_chart_request_ranges = () => ({
    status: "ready",
    source_ids: ["x"],
    source_revisions: [1],
    source_lengths: [4],
    offsets: [0],
    lengths: [2],
    encodings: ["f32"],
    submitted_primitives: 0,
    total_primitives: 3,
  });
  kernel.auto_stream_chart_submit_ranges = () => { submits += 1; };
  kernel.interrupt_render = () => "stream_cancel_queued";
  const { Element, state } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);

  const job = element.render_chart({
    columns: [{ id: "x", revision: 1, length: 4, encoding: "f32" }],
    readRange: () => new Float64Array([1, 2]),
  });
  const [id, callback] = [...state.rafs.entries()].at(-1);
  state.rafs.delete(id);
  callback(0);
  await flushTasks();
  const [nextId, nextCallback] = [...state.rafs.entries()].at(-1);
  state.rafs.delete(nextId);
  nextCallback(1);
  await assert.rejects(job.done, /must return a Float32Array/);
  assert.equal(submits, 0);
  assert.equal(job.status, "failed");
  element.setRect(800, 600);
  element.resize();
  assert.equal(renderRequests, 1, "a failed provider must not become the resize replay source");
});

test("typed-array render never invokes whole-column automatic promotion", async () => {
  const kernel = makeKernel("packed-view-executor");
  kernel.register_streaming_columns = () => {};
  kernel.request_auto_streaming_chart = () => ({
    status: "started", source_ids: ["x", "y"], source_revisions: [1, 1],
  });
  kernel.auto_stream_chart_request_ranges = () => ({
    status: "backpressure", submitted_primitives: 0, total_primitives: 2,
  });
  const { Element } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const job = element.render_chart({
    columns: [
      { id: "x", revision: 1, values: new Float32Array([0, 1, 2]) },
      { id: "y", revision: 1, values: new Float32Array([2, 1, 0]) },
    ],
  });
  assert.equal(job.status, "running");
  assert.equal(kernel.calls.filter(([name]) => name === "try_auto_resident_chart").length, 0);
  await job.cancel();
});
test("high-level streaming cancellation settles without restarting", async () => {
  const kernel = makeKernel("owned-stream-cancel");
  const x = new Float32Array([0, 1]);
  kernel.register_streaming_columns = () => {};
  kernel.request_auto_streaming_chart = () => ({
    status: "started",
    source_ids: ["x"],
    source_revisions: [3],
  });
  kernel.interrupt_render = () => "stream_cancel_queued";
  kernel.auto_stream_chart_step = () => {
    throw new Error("automatic streaming chart has no active execution");
  };
  const { Element, state } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);

  const job = element.render_streaming_chart({
    columns: [{ id: "x", revision: 3, values: x }],
  });
  const cancellation = job.cancel();
  assert.equal(typeof cancellation.then, "function");
  assert.equal(job.status, "cancel_requested");
  const [id, callback] = [...state.rafs.entries()].at(-1);
  state.rafs.delete(id);
  callback(0);
  await cancellation;
  assert.deepEqual(JSON.parse(JSON.stringify(await job.done)), {
    status: "cancelled",
    submittedPrimitives: 0,
    totalPrimitives: 0,
  });
  assert.equal(job.status, "cancelled");
});

test("cancel Promise waits for GPU drain and is idempotent without rAF", async () => {
  const drain = deferred();
  const kernel = makeKernel("cancel-drain", { cancelImpl: () => drain.promise });
  kernel.register_streaming_columns = () => {};
  kernel.request_auto_streaming_chart = () => ({
    status: "started", revision: "5", source_ids: ["x"], source_revisions: [3],
  });
  const { Element } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const job = element.render_chart({ columns: [{ id: "x", revision: 3, values: new Float32Array(4) }] });
  assert.equal(job.finished, job.done);
  const cancelled = job.cancel();
  assert.equal(job.cancel(), cancelled);
  let finished = false;
  job.done.then(() => { finished = true; });
  await flushTasks();
  assert.equal(finished, false);
  assert.equal(element.busy, true);
  drain.resolve();
  assert.equal((await cancelled).status, "cancelled");
  assert.equal((await job.done).status, "cancelled");
  assert.equal(element.busy, false);
  assert.equal(kernel.calls.filter(([name]) => name === "cancel_streaming_and_wait").length, 1);
});

test("range configuration edits replace pending reads but decorations preserve them", async () => {
  const kernel = makeKernel("range-mutation");
  const firstRead = deferred();
  const secondRead = deferred();
  let requestCount = 0;
  let view = 1;
  let captured = 0;
  let readCount = 0;
  const submitted = [];
  kernel.register_streaming_column_sources = () => {};
  kernel.set_title = () => {};
  kernel.set_config = () => { view += 1; };
  kernel.request_auto_streaming_chart = () => {
    requestCount += 1;
    if (captured === view) return { status: "active" };
    captured = view;
    return { status: "started", revision: String(view), source_ids: ["x"], source_revisions: [1] };
  };
  kernel.auto_stream_chart_request_ranges = () => ({
    status: "ready", source_ids: ["x"], source_revisions: [1], source_lengths: [8],
    offsets: [0], lengths: [2], encodings: ["f32"], submitted_primitives: 0, total_primitives: 8,
  });
  kernel.auto_stream_chart_submit_ranges = (_a, _b, _c, _d, chunks) => {
    submitted.push([...chunks[0]]);
    return { status: "all_submitted", submitted_primitives: 8, total_primitives: 8 };
  };
  const { Element, state } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const frame = () => {
    const [id, callback] = [...state.rafs.entries()].at(-1);
    state.rafs.delete(id);
    callback();
  };
  const job = element.render_chart({
    columns: [{ id: "x", revision: 1, length: 8, encoding: "f32" }],
    readRange: () => (++readCount === 1 ? firstRead.promise : secondRead.promise),
  });
  frame();
  element.set_title("Decoration only");
  frame();
  assert.equal(readCount, 1, "a title change must not discard the pending range");
  element.set_config("new transform");
  frame();
  assert.equal(readCount, 2);
  assert.equal(job.revision, "2");
  firstRead.resolve(new Float32Array([1, 1]));
  secondRead.resolve(new Float32Array([2, 2]));
  await flushTasks();
  frame();
  assert.deepEqual(submitted, [[2, 2]]);
  assert.equal(requestCount, 3);
  await job.cancel();
});

test("adaptive chunk sizing yields at the budget and runs in a hidden document", async () => {
  const kernel = makeKernel("adaptive-stream");
  let count = 0;
  let registered = false;
  let chunk = 0;
  let submitted = 0;
  const budgets = [];
  const requests = [];
  kernel.register_streaming_columns = () => {};
  kernel.set_stream_chunk_budget = (value) => { chunk = value; budgets.push(value); };
  kernel.request_auto_streaming_chart = (maximum) => {
    requests.push(maximum);
    if (!registered) {
      registered = true;
      return { status: "started", revision: "7", source_ids: ["x"], source_revisions: [1] };
    }
    return count < 3 ? { status: "active" } : {
      status: "complete", revision: "7", submitted_primitives: submitted, total_primitives: submitted,
    };
  };
  const { Element, state, document } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  kernel.auto_stream_chart_step = () => {
    count += 1;
    submitted += chunk;
    state.now += 20;
    return { status: "submitted", submitted_primitives: submitted, total_primitives: 10000 };
  };
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const job = element.render_chart({
    columns: [{ id: "x", revision: 1, values: new Float32Array(10000) }],
    maxPrimitivesPerChunk: 8192, maxFrameTimeMs: 8,
  });
  document.hidden = true;
  for (let turn = 0; turn < 4; turn += 1) {
    const [id, callback] = [...state.timers.entries()].at(-1);
    state.timers.delete(id);
    callback();
    assert.ok(count <= turn + 1, "no second chunk once the measured budget is exhausted");
  }
  await job.done;
  assert.equal(budgets[0], 4096);
  assert.ok(budgets[1] < budgets[0]);
  assert.ok(requests.every((value) => value === 8192), "adaptive sizes must not change request identity");
  assert.equal(state.timers.size, 0);
});

test("stream auto-fit Promise waits for Config commit without blocking the pump", async () => {
  const kernel = makeKernel("stream-fit");
  const prewarm = deferred();
  let prewarmPromise;
  let prewarming = false;
  let requests = 0;
  let pendingFit = false;
  let step = 0;
  kernel.register_streaming_columns = () => {};
  kernel.request_auto_streaming_chart = () => {
    requests += 1;
    if (requests <= 2) return { status: "started", revision: String(requests), source_ids: ["x"], source_revisions: [1] };
    return { status: "active" };
  };
  kernel.request_stream_auto_fit = () => { pendingFit = true; return true; };
  kernel.auto_stream_chart_step = () => {
    step += 1;
    if (step < 2) return { status: "all_submitted", submitted_primitives: 4, total_primitives: 4 };
    pendingFit = false;
    return { status: "complete", revision: "3", submitted_primitives: 4, total_primitives: 4 };
  };
  kernel.stream_status = () => {
    if (prewarming) throw new Error("WASM borrow conflict");
    return JSON.stringify({ auto_fit_pending: pendingFit });
  };
  kernel.prewarm_all = () => { prewarming = true; return prewarm.promise; };
  const { Element, state } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const job = element.render_chart({
    columns: [{ id: "x", revision: 1, values: new Float32Array(4) }],
    onProgress: ({ status }) => {
      if (status === "complete") prewarmPromise = element.prewarm_all();
    },
  });
  state.messages.shift()();
  assert.equal(state.messages.length, 0);
  let resolved = false;
  const fit = element.auto_fit_all(0.05).then(() => { resolved = true; });
  await flushTasks();
  assert.equal(element.busy, false);
  assert.equal(resolved, false);
  assert.equal(state.messages.length, 1, "fit must wake an execution waiting for GPU completion");
  state.messages.shift()();
  await fit;
  assert.equal(job.status, "complete");
  assert.equal(job.revision, "3");
  assert.equal(pendingFit, false);
  assert.equal(element.busy, true, "fit must resolve without borrowing the kernel held by a callback");
  prewarm.resolve();
  await prewarmPromise;
});

test("same completed request keeps final counts and status queries have no commands", async () => {
  const kernel = makeKernel("read-only-status");
  const result = { status: "complete", revision: "9", job_id: "7", submitted_primitives: 100, total_primitives: 100 };
  let commandCount = 0;
  kernel.register_streaming_column_sources = () => {};
  kernel.stream_status = () => JSON.stringify(result);
  kernel.request_auto_streaming_chart = () => { commandCount += 1; return result; };
  const { Element } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const job = element.render_chart({ columns: [{ id: "x", revision: 1, length: 100, encoding: "f32" }], readRange: () => { throw new Error("must not read"); } });
  assert.equal((await job.done).submittedPrimitives, 100);
  assert.equal(job.totalPrimitives, 100);
  assert.equal(job.revision, "9");
  assert.equal(element.stream_status().execution_id, job.executionId);
  assert.equal(element.stream_status().job_id, "7");
  assert.equal(commandCount, 1);
  await job.cancel();
});

test("completed-job drain failure clears the executor so a new request can run", async () => {
  const kernel = makeKernel("completed-drain-error", { cancelImpl: () => Promise.reject(new Error("drain failed")) });
  kernel.register_streaming_column_sources = () => {};
  kernel.request_auto_streaming_chart = () => ({ status: "complete" });
  const { Element } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const options = { columns: [{ id: "x", revision: 1, length: 4, encoding: "f32" }], readRange: () => new Float32Array(4) };
  const completed = element.render_chart(options);
  await completed.done;
  await assert.rejects(completed.cancel(), /drain failed/);
  assert.equal(element.busy, false);
  const retried = element.render_chart(options);
  assert.equal((await retried.done).status, "complete");
  element.free();
});

test("free detaches the kernel before cancelled progress can reenter render", async () => {
  const kernel = makeKernel("free-reentry");
  kernel.register_streaming_columns = () => {};
  kernel.request_auto_streaming_chart = () => ({
    status: "started", source_ids: ["x"], source_revisions: [1],
  });
  const { Element, state } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const columns = [{ id: "x", revision: 1, values: new Float32Array(4) }];
  let callbacks = 0;
  let reentryError;
  const job = element.render_chart({ columns, onProgress(progress) {
    if (progress.status !== "cancelled") return;
    callbacks += 1;
    try { element.render_chart({ columns }); } catch (error) { reentryError = error; }
  } });
  element.free();
  assert.equal((await job.done).status, "cancelled");
  assert.equal(callbacks, 1);
  assert.match(reentryError?.message ?? "", /not ready/);
  assert.equal(kernel.freeCalls, 1);
  assert.equal(state.timers.size, 0);
  for (const message of state.messages.splice(0)) message();
  assert.equal(state.messages.length, 0);
});

test("completed typed-array replay does not pump across a reentrant async borrow", async () => {
  const prewarm = deferred();
  const kernel = makeKernel("replay-borrow");
  kernel.register_streaming_columns = () => {};
  kernel.set_title = () => {};
  let requests = 0;
  let borrow;
  let element;
  kernel.request_auto_streaming_chart = () => {
    assert.equal(element.busy, false, "every pump must recheck the WASM borrow gate");
    requests += 1;
    return requests === 2
      ? { status: "started", source_ids: ["x"], source_revisions: [1] }
      : { status: "complete" };
  };
  const { Element, state } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  kernel.prewarmAllImpl = () => prewarm.promise;
  const job = element.render_chart({
    columns: [{ id: "x", revision: 1, values: new Float32Array(4) }],
    onProgress(progress) {
      if (progress.status === "running") borrow = element.prewarm_all();
    },
  });
  await job.done;
  element.set_title("changed");
  state.messages.shift()();
  assert.equal(element.busy, true);
  assert.equal(requests, 2);
  prewarm.resolve();
  await borrow;
  state.messages.shift()();
  assert.equal(requests, 3);
  element.free();
});

test("background message tasks continue after the step quantum and wake on GPU completion", async () => {
  const completion = deferred();
  const kernel = makeKernel("background-quantum");
  kernel.register_streaming_columns = () => {};
  kernel.request_auto_streaming_chart = () => ({
    status: "started", source_ids: ["x"], source_revisions: [1],
  });
  let steps = 0;
  let fences = 0;
  let complete = false;
  kernel.auto_stream_chart_step = () => {
    steps += 1;
    return { status: complete ? "complete" : steps <= 40 ? "submitted" : "all_submitted",
      submitted_primitives: Math.min(steps, 40), total_primitives: 40 };
  };
  kernel.streaming_gpu_ready = () => { fences += 1; return completion.promise; };
  const { Element, state, document } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  document.hidden = true;
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const job = element.render_chart({
    columns: [{ id: "x", revision: 1, values: new Float32Array(41) }],
    maxPrimitivesPerChunk: 1,
  });
  state.messages.shift()();
  assert.equal(steps, 32);
  assert.equal(state.messages.length, 1, "quantum exhaustion must queue a task without rAF/timer");
  state.messages.shift()();
  assert.equal(steps, 41);
  assert.equal(fences, 1);
  assert.equal(state.messages.length, 0, "GPU backpressure must not busy-spin message tasks");
  element.setRect(800, 600);
  element.resize();
  assert.equal(state.messages.length, 1, "active TypedArray resize must wake without rAF");
  state.messages.shift()();
  assert.equal(state.messages.length, 0);
  assert.equal(fences, 1, "resize must not duplicate an outstanding GPU completion waiter");
  complete = true;
  completion.resolve();
  await flushTasks();
  assert.equal(state.messages.length, 1);
  state.messages.shift()();
  assert.equal((await job.done).status, "complete");
  assert.equal(state.messages.length, 0);
  assert.equal(state.timers.size, 0);
  element.free();
});

test("GPU preparation submissions do not falsely stall before the first drawable primitive", async () => {
  const kernel = makeKernel("arc-collect-progress");
  kernel.register_streaming_columns = () => {};
  let requests = 0;
  let steps = 0;
  kernel.request_auto_streaming_chart = () => ({
    status: ++requests === 1 ? "started" : "active",
    revision: "1", source_ids: ["x"], source_revisions: [1],
  });
  const { Element, state } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  kernel.auto_stream_chart_step = () => {
    state.now += 8;
    return ++steps <= 6
      ? { status: "submitted", submitted_primitives: 0, total_primitives: 4 }
      : { status: "complete", submitted_primitives: 4, total_primitives: 4 };
  };
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const job = element.render_chart({
    columns: [{ id: "x", revision: 1, values: new Float32Array(5) }],
    maxFrameTimeMs: 4, stallTimeoutMs: 10,
  });
  for (let step = 0; step < 7; step++) {
    assert.equal(state.messages.length, 1, "accepted GPU preparation must schedule further work");
    state.messages.shift()();
    if (step < 6) assert.equal(job.submittedPrimitives, 0);
  }
  assert.equal((await job.done).status, "complete");
  assert.equal(steps, 7);
  assert.equal(state.errors.length, 0);
  element.free();
});

test("backpressure polling does not refresh the streaming stall watchdog", async () => {
  const kernel = makeKernel("stalled-backpressure");
  kernel.register_streaming_columns = () => {};
  let requests = 0;
  kernel.request_auto_streaming_chart = () => ({
    status: ++requests === 1 ? "started" : "active",
    source_ids: ["x"], source_revisions: [1],
  });
  kernel.auto_stream_chart_step = () => ({
    status: "backpressure", submitted_primitives: 0, total_primitives: 4,
  });
  const { Element, state } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);
  const job = element.render_chart({
    columns: [{ id: "x", revision: 1, values: new Float32Array(5) }],
    stallTimeoutMs: 10,
  });
  const rejected = assert.rejects(job.done, /streaming stalled for 10 ms/);
  state.messages.shift()();
  for (const time of [6, 12]) {
    state.now = time;
    const frames = [...state.rafs.values()];
    state.rafs.clear();
    for (const callback of frames) callback(time);
  }
  await rejected;
  assert.equal(job.status, "failed");
  assert.equal(kernel.calls.filter(([name]) => name === "cancel_streaming_and_wait").length, 1);
  element.free();
});

test("full prewarm APIs share busy lifecycle and recover after resolve and reject", async () => {
  const progressPrewarm = deferred();
  const rejectedPrewarm = deferred();
  let plainCalls = 0;
  const kernel = makeKernel("full-prewarm", {
    prewarmAllWithProgressImpl: () => progressPrewarm.promise,
    prewarmAllImpl: () => {
      plainCalls += 1;
      return plainCalls === 1 ? rejectedPrewarm.promise : Promise.resolve();
    },
  });
  const { Element, state } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);

  const onProgress = () => {};
  const frameCount = kernel.calls.filter(([name]) => name === "frame").length;
  const withProgress = element.prewarm_all_with_progress(onProgress);
  assert.equal(element.busy, true);
  assert.equal(
    kernel.calls.find(([name]) => name === "prewarm_all_with_progress")[1],
    onProgress,
  );
  for (const callback of [...state.rafs.values()]) callback(0);
  assert.equal(kernel.calls.filter(([name]) => name === "frame").length, frameCount);
  await assert.rejects(element.prewarm_all(), /busy/);
  assert.equal(kernel.calls.filter(([name]) => name === "prewarm_all").length, 0);
  progressPrewarm.resolve();
  await withProgress;
  assert.equal(element.busy, false);

  const rawError = new Error("raw full prewarm failed");
  const rejected = element.prewarm_all();
  assert.equal(element.busy, true);
  rejectedPrewarm.reject(rawError);
  await assert.rejects(rejected, (error) => error === rawError);
  assert.equal(element.busy, false);

  await element.prewarm_all();
  assert.equal(element.busy, false);
  assert.equal(plainCalls, 2);
  assert.equal(kernel.calls.filter(([name]) => name === "prewarm_all").length, 2);
  element.frame();
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
  await waitForIdle(element);

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
    await waitForIdle(element);
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
    await waitForIdle(element);
    let reconnectedReady;
    element.addEventListener("figgy-release", () => {
      disconnect(element);
      assert.equal(kernel.freeCalls, 0, "exporting kernel must remain alive during settle");
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
    await waitForIdle(element);
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
    await waitForIdle(element);
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
  await waitForIdle(element);
  const oldPromise = element.export_png();

  disconnect(element);
  assert.equal(oldKernel.freeCalls, 0, "disconnect must defer disposal until export settles");
  const newReady = element.ready;
  connect(element);
  await newReady;
  await waitForIdle(element);
  const newPromise = element.export_png();
  assert.equal(element.busy, true);
  assert.equal(oldKernel.freeCalls, 0);
  assert.equal(newKernel.freeCalls, 0);

  oldExport.resolve(new Uint8Array([1]));
  await oldPromise;
  assert.equal(element.busy, true, "old finally must not release the new export");
  assert.equal(oldKernel.freeCalls, 1);
  assert.equal(newKernel.freeCalls, 0);

  newExport.resolve(new Uint8Array([2]));
  await newPromise;
  assert.equal(element.busy, false);
  assert.equal(oldKernel.freeCalls, 1);
  assert.equal(newKernel.freeCalls, 0);
});

test("disconnect defers exporting kernel disposal exactly once on resolve and reject", async () => {
  {
    const operation = deferred();
    const kernel = makeKernel("resolve-after-disconnect", {
      exportImpl: () => operation.promise,
    });
    const { Element, state } = await loadFacade({
      createImpl: () => Promise.resolve(kernel),
    });
    const element = new Element();
    connect(element);
    await element.ready;
    await waitForIdle(element);

    const exported = element.export_png();
    disconnect(element);
    element.free();

    assert.equal(kernel.freeCalls, 0);
    assert.equal(state.rafs.size, 0);
    assert.equal(state.observers.filter((observer) => observer.target === element).length, 0);
    assert.throws(() => element.kernel, /not ready/);

    operation.resolve(new Uint8Array([7]));
    assert.deepEqual(Array.from(await exported), [7]);
    assert.equal(kernel.freeCalls, 1);
    element.free();
    assert.equal(kernel.freeCalls, 1);
  }

  {
    const operation = deferred();
    const exportError = new Error("export rejected after disconnect");
    const kernel = makeKernel("reject-after-disconnect", {
      exportImpl: () => operation.promise,
    });
    const { Element } = await loadFacade({
      createImpl: () => Promise.resolve(kernel),
    });
    const element = new Element();
    connect(element);
    await element.ready;
    await waitForIdle(element);

    const exported = element.export_png();
    disconnect(element);
    assert.equal(kernel.freeCalls, 0);

    operation.reject(exportError);
    await assert.rejects(exported, (error) => error === exportError);
    assert.equal(kernel.freeCalls, 1);
  }
});

test("deferred exports from separate generations dispose independently out of order", async () => {
  const oldExport = deferred();
  const newExport = deferred();
  const oldKernel = makeKernel("old-deferred", { exportImpl: () => oldExport.promise });
  const newKernel = makeKernel("new-deferred", { exportImpl: () => newExport.promise });
  const kernels = [oldKernel, newKernel];
  const { Element } = await loadFacade({
    createImpl: () => Promise.resolve(kernels.shift()),
  });
  const element = new Element();

  connect(element);
  await element.ready;
  await waitForIdle(element);
  const oldPromise = element.export_png();
  disconnect(element);

  const newReady = element.ready;
  connect(element);
  await newReady;
  await waitForIdle(element);
  const newPromise = element.export_png();
  disconnect(element);

  assert.equal(oldKernel.freeCalls, 0);
  assert.equal(newKernel.freeCalls, 0);

  newExport.resolve(new Uint8Array([2]));
  assert.deepEqual(Array.from(await newPromise), [2]);
  assert.equal(oldKernel.freeCalls, 0);
  assert.equal(newKernel.freeCalls, 1);

  oldExport.resolve(new Uint8Array([1]));
  assert.deepEqual(Array.from(await oldPromise), [1]);
  assert.equal(oldKernel.freeCalls, 1);
  assert.equal(newKernel.freeCalls, 1);
});

test("facade normalizes hit and pick results without changing rejection reasons", async () => {
  const kernel = makeKernel("contract");
  const { Element } = await loadFacade({
    createImpl: () => Promise.resolve(kernel),
  });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);

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
    distance_px: 0.5,
  }));
  assert.deepEqual(
    JSON.parse(JSON.stringify(await element.pick_point(1, 2, 3))),
    {
      source_id: null,
      series_id: "series-a",
      point_index: 4,
      distance_px: 0.5,
    },
  );
  kernel.pickImpl = () => Promise.resolve(JSON.stringify({
    source_id: "source-a",
    series_id: "series-a",
    point_index: 5,
    distance_px: 1,
  }));
  assert.equal((await element.pick_point(1, 2, 3)).source_id, "source-a");

  kernel.pickDataImpl = () => Promise.resolve(undefined);
  assert.equal(await element.pick_data(1, 2, 3), null);
  kernel.next_view_point_index = (source, series, current, forward) => {
    assert.deepEqual([source, series, current, forward], ["source-a", "series-a", 5, true]);
    return 9;
  };
  assert.equal(element.next_view_point_index("source-a", "series-a", 5, true), 9);
  kernel.next_view_point_index = () => undefined;
  assert.equal(element.next_view_point_index(null, "series-a", 9, false), null);
  kernel.pickDataImpl = () => Promise.resolve(JSON.stringify({
    kind: "matrix_cell",
    source_id: "source-grid",
    series_id: "heatmap-a",
    x_index: 2,
    y_index: 3,
    distance_px: 0,
  }));
  assert.deepEqual(
    JSON.parse(JSON.stringify(await element.pick_data(1, 2, 3))),
    {
      kind: "matrix_cell",
      source_id: "source-grid",
      series_id: "heatmap-a",
      x_index: 2,
      y_index: 3,
      distance_px: 0,
    },
  );

  kernel.pickImpl = () => Promise.resolve("{not json");
  await assert.rejects(
    element.pick_point(1, 2, 3),
    (error) => error?.name === "SyntaxError",
  );

  const rawError = new Error("raw pick failed");
  kernel.pickImpl = () => Promise.reject(rawError);
  await assert.rejects(element.pick_point(1, 2, 3), (error) => error === rawError);
  kernel.pickDataImpl = () => Promise.reject(rawError);
  await assert.rejects(element.pick_data(1, 2, 3), (error) => error === rawError);
});

test("facade exposes pool growth, detailed memory status, and awaited cleanup", async () => {
  const kernel = makeKernel("memory");
  let growth = null;
  kernel.set_pool_auto_growth = (enabled) => { growth = enabled; };
  kernel.gpu_memory_status = () => JSON.stringify({ total_bytes: 32, pool: { capacity_bytes: 16 } });
  kernel.release_unused_gpu_memory = () => Promise.resolve(JSON.stringify({
    total_bytes: 16, pool: { capacity_bytes: 8 },
  }));
  const { Element } = await loadFacade({ createImpl: () => Promise.resolve(kernel) });
  const element = new Element();
  connect(element);
  await element.ready;
  await waitForIdle(element);

  assert.throws(() => element.set_pool_auto_growth(1), /enabled must be a boolean/);
  element.set_pool_auto_growth(true);
  assert.equal(growth, true);
  assert.equal(element.gpu_memory_status().pool.capacity_bytes, 16);
  const cleaned = await element.release_unused_gpu_memory();
  assert.equal(cleaned.total_bytes, 16);
  assert.equal(cleaned.pool.capacity_bytes, 8);
  assert.equal(element.busy, false);
});
