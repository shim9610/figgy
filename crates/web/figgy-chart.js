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

function streamArrayEncoding(values) {
  if (values instanceof Float32Array) return "f32";
  if (values instanceof Float64Array) return "f64";
  return null;
}

function validateStreamRangeArray(values, encoding, length, id) {
  const actualEncoding = streamArrayEncoding(values);
  if (actualEncoding !== encoding) {
    throw new TypeError(
      `readRange for ${id} must return a ${encoding === "f32" ? "Float32Array" : "Float64Array"}`,
    );
  }
  if (values.length !== length) {
    throw new RangeError(
      `readRange for ${id} returned ${values.length} values; renderer requested ${length}`,
    );
  }
  return values;
}

class FiggyRenderJob {
  #state;
  #cancel;

  constructor(state, cancel) {
    this.#state = state;
    this.#cancel = cancel;
    this.done = state.done;
    this.finished = state.done;
  }

  get status() { return this.#state.status; }
  get executionId() { return this.#state.executionId; }
  get submittedPrimitives() { return this.#state.submittedPrimitives; }
  get totalPrimitives() { return this.#state.totalPrimitives; }
  get revision() { return this.#state.revision ?? null; }
  get autoFitPending() { return this.#state.autoFitPending ?? false; }
  cancel() { return this.#cancel(); }
}

export class FiggyChartElement extends HTMLElement {
  #canvas;
  #kernel = null;
  #resizeObserver = null;
  #raf = 0;
  #started = false;
  #lifecycleGeneration = 0;
  #readyToken = null;
  #operationToken = null;
  #lastPoint = null;
  #dpr = 1;
  #pendingResize = null;
  #pendingRelease = null;
  #pendingPrewarm = null;
  #streamSources = new Map();
  #streamExecution = null;
  #streamReplay = null;
  #nextStreamExecution = 1;
  #lastStreamState = null;
  #streamWake = 0;
  #streamChannel = null;
  #streamTaskPending = false;
  #streamRefreshRequested = false;
  #streamSelection = null;
  #selectionDirty = false;

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
    if (this.busy) {
      throw new Error("figgy chart is busy");
    }
    if (!this.#kernel) {
      throw new Error("figgy chart is not ready yet; await element.ready first");
    }
    return this.#kernel;
  }

  get busy() {
    return this.#operationToken !== null;
  }

  #isCurrentConnection(generation, readyToken = this.#readyToken) {
    return this.#started
      && this.isConnected
      && this.#lifecycleGeneration === generation
      && this.#readyToken === readyToken;
  }

  #isCurrentOperation(token) {
    return this.#operationToken === token
      && this.#lifecycleGeneration === token.generation
      && (token.kind === "connect" || this.#kernel === token.kernel);
  }

  async #connect(generation, readyToken) {
    const token = this.#beginOperation("connect", null, generation);
    let kernel = null;
    let resizeObserver = null;
    let published = false;
    try {
      await ensureWasm();
      if (!this.#isCurrentConnection(generation, readyToken)
          || !this.#isCurrentOperation(token)) {
        return;
      }
      this.#resizeCanvas(false);
      if (!this.#isCurrentConnection(generation, readyToken)
          || !this.#isCurrentOperation(token)) {
        return;
      }
      kernel = await RawFiggyChart.create_with_progress(this.#canvas, (event) => {
        if (this.#isCurrentConnection(generation, readyToken)
            && this.#isCurrentOperation(token)) {
          dispatchFiggyEvent(this, "figgy-init-progress", event);
        }
      });
      token.kernel = kernel;
      if (!this.#isCurrentConnection(generation, readyToken)
          || !this.#isCurrentOperation(token)) {
        return;
      }
      resizeObserver = new ResizeObserver(() => this.#resizeCanvas(true));
      resizeObserver.observe(this);
      if (!this.#isCurrentConnection(generation, readyToken)
          || !this.#isCurrentOperation(token)) {
        return;
      }
      this.#kernel = kernel;
      this.#resizeObserver = resizeObserver;
      this.#pendingResize = null;
      this.#pendingRelease = null;
      published = true;
    } finally {
      if (!published && resizeObserver) {
        resizeObserver.disconnect();
      }
      if (!published && kernel && token.disposal === "attached") {
        token.disposal = "freed";
        kernel.free();
      }
      this.#settleOperation(token);
    }

    if (!published || !this.#isCurrentConnection(generation, readyToken)
        || this.#kernel !== kernel) {
      return;
    }
    readyToken.state = "fulfilled";
    readyToken.resolve(this);
    dispatchFiggyEvent(this, "figgy-ready", { chart: this, kernel });
    if (this.#isCurrentConnection(generation, readyToken) && this.#kernel === kernel) {
      this.#startLoop();
      this.#queueBackgroundPrewarm(generation, kernel);
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
    this.#advanceFrame();
  };

  #advanceFrame() {
    if (!this.#kernel || this.busy) {
      return;
    }
    if ((window.devicePixelRatio || 1) !== this.#dpr) {
      this.#resizeCanvas(true);
    }
    try {
      if (this.#streamRefreshRequested && !this.#streamExecution) {
        this.#streamRefreshRequested = false;
        this.#replayCompletedStreamAfterResize();
      }
      const deadline = performance.now() + (this.#streamExecution?.maxFrameTimeMs ?? 8);
      for (let step = 0; step < 32; step += 1) {
        const execution = this.#streamExecution;
        if (!execution) break;
        const before = execution.state.submittedPrimitives;
        const started = performance.now();
        const more = this.#pumpStreamingExecution();
        const elapsed = performance.now() - started;
        const submitted = execution.state.submittedPrimitives - before;
        if (submitted > 0) {
          const estimate = Math.floor(submitted * execution.maxFrameTimeMs * 0.5 / Math.max(0.05, elapsed));
          execution.chunkPrimitives = Math.max(1, Math.min(
            execution.maxPrimitivesPerChunk, execution.chunkPrimitives * 2, estimate,
          ));
        }
        if (more && (step === 31 || performance.now() >= deadline) && !this.busy) {
          this.#queueStreamTask();
        }
        if (!more || performance.now() >= deadline || this.busy) break;
      }
      if (!this.busy) this.#pumpStreamSelection();
      if (!this.busy) this.#kernel?.frame();
    } catch (error) {
      if (this.#streamExecution && !this.#streamExecution.settled) {
        this.#failStreamingExecution(this.#streamExecution, error);
      }
      console.error("figgy frame:", error);
      dispatchFiggyEvent(this, "figgy-error", { error });
    }
  }

  #scheduleStreamWake() {
    if (this.#streamWake || !this.#streamExecution) return;
    this.#streamWake = setTimeout(() => {
      this.#streamWake = 0;
      if (document.hidden) this.#advanceFrame();
      this.#scheduleStreamWake();
    }, document.hidden ? 16 : 100);
  }

  #queueStreamTask() {
    if (this.#streamTaskPending || (!this.#streamExecution && !this.#streamRefreshRequested
        && !this.#selectionDirty && !this.#streamSelection)) return;
    if (!this.#streamChannel) {
      this.#streamChannel = new MessageChannel();
      this.#streamChannel.port1.onmessage = () => {
        this.#streamTaskPending = false;
        if (this.#streamExecution || this.#streamRefreshRequested
            || this.#selectionDirty || this.#streamSelection) this.#advanceFrame();
      };
    }
    this.#streamTaskPending = true;
    this.#streamChannel.port2.postMessage(null);
  }

  #wakeOnGpuCompletion(execution) {
    if (execution.gpuWakePending || this.busy || execution.settled) return;
    execution.gpuWakePending = true;
    execution.kernel.streaming_gpu_ready().then(() => {
      execution.gpuWakePending = false;
      if (this.#streamExecution === execution && !execution.settled) this.#queueStreamTask();
    }, (error) => {
      execution.gpuWakePending = false;
      if (this.#streamExecution === execution && !execution.settled) {
        this.#failStreamingExecution(execution, error);
      }
    });
  }

  #notifyStreamingMutation() {
    this.#selectionDirty = true;
    if (this.#streamExecution) {
      this.#streamExecution.reconcileRequested = true;
    } else if (this.#streamReplay) {
      this.#streamRefreshRequested = true;
    }
    this.#queueStreamTask();
  }

  #mutateChart(method, ...args) {
    const result = this.#kernelForCall()[method](...args);
    this.#notifyStreamingMutation();
    return result;
  }

  #mutateSelection(method, json) {
    const result = this.#kernelForCall()[method](json);
    if (this.#streamReplay || this.#streamExecution || this.#streamSelection) {
      this.#selectionDirty = true;
      this.#queueStreamTask();
    }
    return result;
  }

  #releaseSelectionRequest() {
    const selection = this.#streamSelection;
    this.#streamSelection = null;
    if (selection) {
      if (selection.timer) clearTimeout(selection.timer);
      selection.request?.free?.();
      selection.request = null;
      selection.chunks = null;
      selection.sources = null;
    }
  }

  #pumpStreamSelection() {
    if (this.busy) return;
    const replay = this.#streamReplay;
    const kernel = this.#kernel;
    if (!replay || !kernel) {
      this.#releaseSelectionRequest();
      this.#selectionDirty = false;
      return;
    }
    let selection = this.#streamSelection;
    if (selection && (selection.kernel !== kernel || selection.replay !== replay)) {
      this.#releaseSelectionRequest();
      selection = null;
      this.#selectionDirty = true;
    }
    try {
      if (selection?.error && !this.#selectionDirty) throw selection.error;
      if (this.#selectionDirty || (selection && !selection.request && !selection.waitingGpu)) {
        const request = kernel.stream_selection_request_ranges();
        const status = request.status;
        this.#selectionDirty = false;
        if (status === "complete" || status === "failed") {
          request.free?.();
          this.#releaseSelectionRequest();
          return;
        }
        if (status === "backpressure") {
          request.free?.();
          this.#releaseSelectionRequest();
          selection = this.#streamSelection = { kernel, replay, request: null, waitingGpu: true };
          kernel.streaming_gpu_ready().then(() => {
            if (this.#streamSelection !== selection) return;
            // A held source request can occupy the shared slot without GPU
            // work; an already-resolved fence must not create a task spin.
            selection.timer = setTimeout(() => {
              if (this.#streamSelection === selection) {
                selection.waitingGpu = false;
                this.#queueStreamTask();
              }
            }, 16);
          }, (error) => {
            if (this.#streamSelection === selection) {
              selection.error = error;
              this.#queueStreamTask();
            }
          });
          return;
        }
        if (status !== "ready") {
          request.free?.();
          throw new Error(`renderer returned unexpected selection state: ${status}`);
        }
        if (selection?.request && request.same_selection_request(selection.request)) {
          request.free?.();
        } else {
          this.#releaseSelectionRequest();
          selection = this.#streamSelection = {
            kernel, replay, request, chunks: null, error: null,
            ids: request.source_ids, revisions: request.source_revisions,
            sourceLengths: request.source_lengths, offsets: request.offsets,
            lengths: request.lengths, encodings: request.encodings,
            sources: new Map(replay.columns.map((source) => [source.id, source])),
            started: performance.now(),
          };
          if (replay.stallTimeoutMs > 0) {
            selection.timer = setTimeout(() => {
              if (this.#streamSelection === selection) {
                selection.error = new Error(`stream selection stalled for ${replay.stallTimeoutMs} ms`);
                this.#queueStreamTask();
              }
            }, replay.stallTimeoutMs);
          }
          const { ids, revisions, sourceLengths, offsets, lengths, encodings } = selection;
          if ([revisions, sourceLengths, offsets, lengths, encodings].some((v) => v.length !== ids.length)) {
            throw new Error("renderer returned mismatched selection range arrays");
          }
          const reads = ids.map(async (id, index) => {
            const source = selection.sources.get(id);
            const sourceLength = source?.values?.length ?? source?.length;
            const encoding = source?.values ? streamArrayEncoding(source.values) : source?.encoding;
            const offset = offsets[index], length = lengths[index];
            if (!source || source.revision !== revisions[index]
                || sourceLength !== sourceLengths[index] || encoding !== encodings[index]) {
              throw new Error(`selection source ${id} does not match its render revision`);
            }
            if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(length)
                || offset < 0 || length <= 0 || offset + length > sourceLength) {
              throw new Error(`selection range for ${id} exceeds its source`);
            }
            return source.values ? source.values.subarray(offset, offset + length)
              : replay.readRange({ id, revision: source.revision, offset, length, encoding });
          });
          Promise.all(reads).then((chunks) => {
            if (this.#streamSelection !== selection) return;
            selection.chunks = chunks.map((chunk, index) => validateStreamRangeArray(
              chunk, encodings[index], lengths[index], ids[index],
            ));
          }).catch((error) => {
            if (this.#streamSelection === selection) selection.error = error;
          }).finally(() => {
            if (this.#streamSelection === selection) this.#queueStreamTask();
          });
          return;
        }
      }
      if (!selection) return;
      if (selection.error) throw selection.error;
      if (selection.chunks) {
        kernel.stream_selection_submit_ranges(selection.request, selection.ids, selection.revisions,
          selection.sourceLengths, selection.offsets, selection.chunks);
        this.#releaseSelectionRequest();
        this.#selectionDirty = true;
        this.#queueStreamTask();
      } else if (selection.request && replay.stallTimeoutMs > 0
          && performance.now() - selection.started > replay.stallTimeoutMs) {
        throw new Error(`stream selection stalled for ${replay.stallTimeoutMs} ms`);
      }
    } catch (error) {
      if (selection?.request) {
        try { kernel.discard_stream_selection_request(selection.request); }
        catch (cleanupError) {
          dispatchFiggyEvent(this, "figgy-error", { error: cleanupError, operation: "selection_cleanup" });
        }
      }
      this.#releaseSelectionRequest();
      this.#selectionDirty = false;
      dispatchFiggyEvent(this, "figgy-error", { error, operation: "stream_selection", recoverable: true });
    }
  }

  #emitStreamingProgress(execution) {
    const progress = {
      executionId: execution.state.executionId,
      revision: execution.state.revision ?? null,
      status: execution.state.status,
      submittedPrimitives: execution.state.submittedPrimitives,
      totalPrimitives: execution.state.totalPrimitives,
    };
    try {
      execution.onProgress?.(progress);
    } catch (error) {
      dispatchFiggyEvent(this, "figgy-error", {
        error,
        operation: "stream_progress_callback",
        recoverable: true,
      });
    }
    dispatchFiggyEvent(this, "figgy-stream-progress", progress);
  }

  #settleStreamingExecution(execution, status, error = null) {
    if (execution.settled) {
      return;
    }
    if (status === "complete") {
      execution.state.autoFitPending = JSON.parse(execution.kernel.stream_status()).auto_fit_pending;
    }
    execution.settled = true;
    execution.state.status = status;
    this.#lastStreamState = execution.state;
    execution.signal?.removeEventListener("abort", execution.abortListener);
    if (status === "failed"
        && this.#streamReplay?.token === execution.replayToken) {
      this.#streamReplay = execution.previousReplay;
    }
    if (status === "resident" && this.#streamReplay?.token === execution.replayToken) {
      this.#streamReplay = null;
      this.#streamRefreshRequested = false;
      this.#releaseSelectionRequest();
      this.#selectionDirty = false;
    }
    execution.previousReplay = null;
    if (this.#streamExecution === execution) {
      this.#streamExecution = null;
    }
    const result = {
      status,
      submittedPrimitives: execution.state.submittedPrimitives,
      totalPrimitives: execution.state.totalPrimitives,
    };
    this.#emitStreamingProgress(execution);
    if (error) {
      execution.reject(error);
      dispatchFiggyEvent(this, "figgy-error", {
        error,
        operation: "render_streaming_chart",
      });
    } else {
      execution.resolve(result);
    }
    execution.sources = [];
    execution.rangeSources = null;
    execution.rangeResolved = null;
    execution.rangeError = null;
    execution.rangePending = false;
    execution.readRange = null;
    execution.signal = null;
    execution.abortListener = null;
    execution.onProgress = null;
    execution.kernel = null;
    if (!this.#streamExecution && this.#streamWake) {
      clearTimeout(this.#streamWake);
      this.#streamWake = 0;
    }
    if (!this.#streamExecution && !this.#streamRefreshRequested && this.#streamChannel) {
      this.#streamChannel.port1.close();
      this.#streamChannel.port2.close();
      this.#streamChannel = null;
      this.#streamTaskPending = false;
    }
  }

  #completedStreamingJob(onProgress, replayToken = this.#streamReplay?.token) {
    if (!Number.isSafeInteger(this.#nextStreamExecution)) {
      throw new Error("render execution counter exhausted");
    }
    const current = JSON.parse(this.#kernel.stream_status());
    const result = {
      status: "complete",
      submittedPrimitives: current.submitted_primitives,
      totalPrimitives: current.total_primitives,
    };
    const state = {
      executionId: this.#nextStreamExecution++,
      revision: current.revision,
      autoFitPending: current.auto_fit_pending,
      ...result,
      done: Promise.resolve(result),
    };
    this.#lastStreamState = state;
    try {
      onProgress?.(result);
    } catch (error) {
      dispatchFiggyEvent(this, "figgy-error", {
        error,
        operation: "stream_progress_callback",
        recoverable: true,
      });
    }
    dispatchFiggyEvent(this, "figgy-stream-progress", result);
    const execution = { state, settled: true, replayToken, kernel: this.#kernel };
    return new FiggyRenderJob(state, () => this.#cancelStreamingExecution(execution));
  }

  #createStreamReplay(columns, maxPrimitivesPerChunk, onProgress, readRange,
    maxFrameTimeMs, stallTimeoutMs) {
    return {
      token: {},
      columns: columns.map((column) => {
        if (!column || typeof column !== "object") return column;
        const snapshot = {
          id: column.id,
          revision: column.revision,
        };
        if ("values" in column) snapshot.values = column.values;
        if ("length" in column) snapshot.length = column.length;
        if ("encoding" in column) snapshot.encoding = column.encoding;
        return snapshot;
      }),
      maxPrimitivesPerChunk,
      maxFrameTimeMs,
      stallTimeoutMs,
      onProgress,
      readRange,
    };
  }

  #renderWithStreamReplay(replay, render) {
    const previous = this.#streamReplay;
    replay.previousReplay = previous;
    this.#streamReplay = replay;
    this.#selectionDirty = true;
    try {
      const job = render();
      delete replay.previousReplay;
      if (job.status === "resident") {
        if (this.#streamReplay === replay) this.#streamReplay = null;
      } else if (job.status === "failed") {
        if (this.#streamReplay === replay) this.#streamReplay = previous;
      } else if (this.#streamReplay === replay
          && this.#streamExecution?.handle === job) {
        this.#streamExecution.replayToken = replay.token;
        this.#streamExecution.previousReplay = previous;
      }
      return job;
    } catch (error) {
      delete replay.previousReplay;
      if (this.#streamReplay === replay) this.#streamReplay = previous;
      throw error;
    }
  }

  #clearStreamReplay(execution) {
    if (this.#streamReplay?.token === execution.replayToken) {
      for (const column of this.#streamReplay.columns) {
        const source = this.#streamSources.get(column.id);
        if (source?.revision === column.revision) {
          this.#streamSources.set(column.id, {
            revision: source.revision,
            length: source.length ?? source.values?.length,
            encoding: source.encoding ?? streamArrayEncoding(source.values),
            values: null,
          });
        }
      }
      this.#streamReplay = null;
      this.#streamRefreshRequested = false;
      this.#releaseSelectionRequest();
      this.#selectionDirty = false;
    }
    execution.previousReplay = null;
  }

  #cancelStreamingExecution(execution) {
    if (execution.cancelPromise) return execution.cancelPromise;
    const replayMatches = this.#streamReplay?.token === execution.replayToken;
    if (replayMatches && this.#streamExecution && this.#streamExecution !== execution) {
      return this.#cancelStreamingExecution(this.#streamExecution);
    }
    if (this.#streamExecution !== execution && !replayMatches) {
      return Promise.resolve({ status: execution.state.status });
    }
    this.#clearStreamReplay(execution);
    execution.rangeGeneration = (execution.rangeGeneration ?? 0) + 1;
    execution.rangeResolved = null;
    execution.rangePending = false;
    execution.cancelPromise = new Promise((resolve, reject) => {
      execution.cancelResolve = resolve;
      execution.cancelReject = reject;
    });
    execution.cancelPromise.catch(() => {});
    if (execution.settled) {
      execution.kernel = this.#kernel;
      execution.cleanupOnly = true;
      this.#streamExecution = execution;
    } else {
      execution.state.status = "cancel_requested";
      this.#emitStreamingProgress(execution);
    }
    this.#drainStreamingCancellation(execution);
    return execution.cancelPromise;
  }

  #drainStreamingCancellation(execution) {
    if (execution.draining || this.busy || !this.#kernel) return;
    execution.draining = true;
    const kernel = this.#kernel;
    this.#runKernelOperation("stream-cancel", (current) => current.cancel_streaming_and_wait())
      .then(() => {
        if (execution.cleanupOnly) {
          if (this.#streamExecution === execution) this.#streamExecution = null;
          execution.kernel = null;
        } else if (!execution.settled) {
          this.#settleStreamingExecution(execution,
            execution.terminalError ? "failed" : "cancelled", execution.terminalError);
        }
        execution.cancelResolve({ status: "cancelled" });
      }, (error) => {
        if (execution.cleanupOnly) {
          if (this.#streamExecution === execution) this.#streamExecution = null;
          execution.kernel = null;
        } else if (!execution.settled) {
          this.#settleStreamingExecution(execution, "failed", error);
        }
        execution.cancelReject(error);
      }).finally(() => {
        if (this.#kernel === kernel && !this.#streamExecution && this.#streamWake) {
          clearTimeout(this.#streamWake);
          this.#streamWake = 0;
        }
        if (!this.#streamExecution && !this.#streamRefreshRequested && this.#streamChannel) {
          this.#streamChannel.port1.close();
          this.#streamChannel.port2.close();
          this.#streamChannel = null;
          this.#streamTaskPending = false;
        }
      });
  }

  #replayCompletedStreamAfterResize() {
    const replay = this.#streamReplay;
    if (!replay || this.#streamExecution || !this.#kernel || this.busy) {
      return;
    }
    try {
      const render = replay.readRange !== null
          || replay.columns.some((column) => !column?.values)
        ? () => this.#renderRangeStreamingChart({
          columns: replay.columns,
          maxPrimitivesPerChunk: replay.maxPrimitivesPerChunk,
          maxFrameTimeMs: replay.maxFrameTimeMs,
          stallTimeoutMs: replay.stallTimeoutMs,
          signal: null,
          onProgress: replay.onProgress,
          readRange: replay.readRange,
        })
        : () => this.#renderTypedArrayStreamingChart({
          columns: replay.columns,
          maxPrimitivesPerChunk: replay.maxPrimitivesPerChunk,
          maxFrameTimeMs: replay.maxFrameTimeMs,
          stallTimeoutMs: replay.stallTimeoutMs,
          signal: null,
          onProgress: replay.onProgress,
        });
      const job = this.#renderWithStreamReplay(replay, render);
      job.done.catch(() => {
        // The execution path already emits figgy-error with the original error.
      });
      return job;
    } catch (error) {
      dispatchFiggyEvent(this, "figgy-error", {
        error,
        operation: "stream_resize_replay",
      });
    }
  }

  #handleStreamingResize() {
    const execution = this.#streamExecution;
    if (!execution) {
      this.#replayCompletedStreamAfterResize();
      return;
    }
    if (execution.state.status === "cancel_requested") {
      return;
    }
    if (execution.rangeSources) {
      execution.rangeGeneration += 1;
      execution.rangePending = false;
      execution.rangeResolved = null;
      execution.rangeError = null;
      execution.restartForResize = true;
    }
    this.#queueStreamTask();
  }

  #bindStreamingSources(execution, request) {
    if (request.status !== "started") {
      return;
    }
    const ids = request.source_ids;
    const revisions = request.source_revisions;
    if (ids.length !== revisions.length) {
      throw new Error("renderer returned mismatched streaming source identities");
    }
    const sources = [];
    for (let index = 0; index < ids.length; index += 1) {
      const source = this.#streamSources.get(ids[index]);
      if (!source || source.revision !== revisions[index]) {
        throw new Error(
          `stream source ${ids[index]} revision ${revisions[index]} is not registered in this chart`,
        );
      }
      sources.push(source.values);
    }
    execution.ids = [...ids];
    execution.revisions = [...revisions];
    execution.sources = sources;
    execution.state.submittedPrimitives = 0;
    execution.state.totalPrimitives = 0;
    execution.state.revision = request.revision ?? null;
  }

  #bindRangeStreamingSources(execution, request) {
    if (request.status !== "started") {
      return;
    }
    const ids = request.source_ids;
    const revisions = request.source_revisions;
    if (ids.length !== revisions.length) {
      throw new Error("renderer returned mismatched streaming source identities");
    }
    for (let index = 0; index < ids.length; index += 1) {
      const source = execution.rangeSources.get(ids[index]);
      if (!source || source.revision !== revisions[index]) {
        throw new Error(
          `stream source ${ids[index]} revision ${revisions[index]} is not bound to this request`,
        );
      }
    }
    execution.ids = [...ids];
    execution.revisions = [...revisions];
    execution.state.submittedPrimitives = 0;
    execution.state.totalPrimitives = 0;
    execution.state.revision = request.revision ?? null;
  }

  #completeRangeStreamingExecution(execution) {
    for (const id of execution.ids) {
      const source = execution.rangeSources.get(id);
      this.#streamSources.set(id, {
        revision: source.revision,
        length: source.length,
        encoding: source.encoding,
        values: source.values ?? null,
      });
    }
    this.#settleStreamingExecution(execution, "complete");
  }

  #failStreamingExecution(execution, error) {
    execution.terminalError = error;
    this.#cancelStreamingExecution(execution);
  }

  #updateStreamProgress(execution, progress) {
    const before = execution.state.submittedPrimitives;
    if (progress.submitted_primitives !== undefined) {
      execution.state.submittedPrimitives = progress.submitted_primitives;
      execution.state.totalPrimitives = progress.total_primitives;
    }
    if (progress.revision !== undefined) execution.state.revision = progress.revision;
    // Scan/replay preparation submits useful GPU work before any primitive is
    // drawable. Backpressure and status-only polling are not forward progress.
    if (before !== execution.state.submittedPrimitives || progress.status === "submitted") {
      execution.lastProgressTime = performance.now();
    }
  }

  #applyStreamChunkBudget(execution) {
    if (execution.appliedChunkPrimitives !== execution.chunkPrimitives) {
      execution.kernel.set_stream_chunk_budget(execution.chunkPrimitives);
      execution.appliedChunkPrimitives = execution.chunkPrimitives;
    }
  }

  #restartRangeStreamingExecution(execution) {
    const request = execution.kernel.request_auto_streaming_chart(
      execution.maxPrimitivesPerChunk,
    );
    execution.restartForResize = false;
    execution.reconcileRequested = false;
    if (request.status === "complete") {
      this.#updateStreamProgress(execution, request);
      this.#completeRangeStreamingExecution(execution);
      return false;
    }
    if (request.status === "started") {
      execution.appliedChunkPrimitives = null;
      execution.rangeGeneration += 1;
      execution.rangePending = false;
      execution.rangeResolved = null;
      execution.rangeError = null;
      execution.lastProgressTime = performance.now();
      this.#bindRangeStreamingSources(execution, request);
      execution.state.status = "running";
      this.#emitStreamingProgress(execution);
      return true;
    }
    if (request.status === "active") {
      return true;
    }
    throw new Error(`renderer returned unexpected stream state: ${request.status}`);
  }

  #pumpRangeStreamingExecution(execution) {
    if (execution.state.status === "cancel_requested") {
      this.#drainStreamingCancellation(execution);
      return false;
    }
    if ((execution.restartForResize || execution.reconcileRequested)
        && !this.#restartRangeStreamingExecution(execution)) {
      return false;
    }
    if (this.busy || this.#streamExecution !== execution || execution.settled
        || execution.state.status === "cancel_requested") {
      return false;
    }
    this.#applyStreamChunkBudget(execution);
    if (execution.rangeError) {
      const error = execution.rangeError;
      execution.rangeError = null;
      throw error;
    }
    if (execution.rangeResolved) {
      const resolved = execution.rangeResolved;
      execution.rangeResolved = null;
      const args = [resolved.ids, resolved.revisions, resolved.sourceLengths,
        resolved.offsets, resolved.chunks];
      const progress = execution.kernel.auto_stream_chart_submit_ranges(...args);
      execution.state.status = progress.status === "all_submitted"
        ? "waiting_gpu"
        : "running";
      this.#updateStreamProgress(execution, progress);
      this.#emitStreamingProgress(execution);
      if (progress.status !== "submitted") this.#wakeOnGpuCompletion(execution);
      return progress.status === "submitted";
    }
    if (execution.rangePending) {
      return false;
    }

    const request = execution.kernel.auto_stream_chart_request_ranges();
    this.#updateStreamProgress(execution, request);
    if (request.status === "complete") {
      this.#completeRangeStreamingExecution(execution);
      return false;
    }
    if (request.status === "backpressure" || request.status === "all_submitted") {
      execution.state.status = request.status === "all_submitted"
        ? "waiting_gpu"
        : "running";
      this.#emitStreamingProgress(execution);
      this.#wakeOnGpuCompletion(execution);
      return false;
    }
    if (request.status !== "ready") {
      throw new Error(`renderer returned unexpected range state: ${request.status}`);
    }

    const ids = request.source_ids;
    const revisions = request.source_revisions;
    const sourceLengths = request.source_lengths;
    const offsets = request.offsets;
    const lengths = request.lengths;
    const encodings = request.encodings;
    const count = ids.length;
    if (revisions.length !== count
        || sourceLengths.length !== count
        || offsets.length !== count
        || lengths.length !== count
        || encodings.length !== count) {
      throw new Error("renderer returned mismatched range arrays");
    }

    const generation = ++execution.rangeGeneration;
    execution.lastProgressTime = performance.now();
    execution.rangePending = true;
    const reads = ids.map(async (id, index) => {
      if (execution.settled || this.#streamExecution !== execution
          || execution.state.status === "cancel_requested") {
        throw new DOMException("stream range request cancelled", "AbortError");
      }
      const source = execution.rangeSources.get(id);
      if (!source
          || source.revision !== revisions[index]
          || source.length !== sourceLengths[index]
          || source.encoding !== encodings[index]) {
        throw new Error(`range request source ${id} no longer matches its captured revision`);
      }
      const offset = offsets[index];
      const length = lengths[index];
      if (source.values) {
        return source.values.subarray(offset, offset + length);
      }
      return execution.readRange({
        id,
        revision: revisions[index],
        offset,
        length,
        encoding: encodings[index],
      });
    });
    Promise.all(reads.map((read) => Promise.resolve(read))).then((chunks) => {
      if (execution.settled
          || this.#streamExecution !== execution
          || execution.rangeGeneration !== generation
          || execution.state.status === "cancel_requested") {
        return;
      }
      const validated = chunks.map((chunk, index) => validateStreamRangeArray(
        chunk,
        encodings[index],
        lengths[index],
        ids[index],
      ));
      execution.rangeResolved = {
        ids,
        revisions,
        sourceLengths,
        offsets,
        chunks: validated,
      };
      execution.lastProgressTime = performance.now();
    }).catch((error) => {
      if (execution.settled
          || this.#streamExecution !== execution
          || execution.rangeGeneration !== generation) {
        return;
      }
      execution.rangeError = error;
    }).finally(() => {
      if (execution.rangeGeneration === generation) {
        execution.rangePending = false;
        if (execution.rangeResolved || execution.rangeError) this.#queueStreamTask();
      }
    });
    return false;
  }

  #pumpStreamingExecution() {
    const execution = this.#streamExecution;
    if (this.busy || !execution || execution.settled || execution.kernel !== this.#kernel) {
      return false;
    }
    try {
      if (execution.stallTimeoutMs > 0
          && performance.now() - execution.lastProgressTime > execution.stallTimeoutMs) {
        throw new Error(`streaming stalled for ${execution.stallTimeoutMs} ms`);
      }
      if (execution.rangeSources) {
        return this.#pumpRangeStreamingExecution(execution);
      }
      if (execution.state.status === "cancel_requested") {
        this.#drainStreamingCancellation(execution);
        return false;
      }

      const request = this.#kernel.request_auto_streaming_chart(
        execution.maxPrimitivesPerChunk,
      );
      this.#bindStreamingSources(execution, request);
      if (request.status === "started") execution.appliedChunkPrimitives = null;
      if (request.status === "complete") {
        this.#updateStreamProgress(execution, request);
        this.#settleStreamingExecution(execution, "complete");
        return false;
      }
      if (execution.ids.length === 0) {
        throw new Error("automatic streaming execution has no pinned source set");
      }

      this.#applyStreamChunkBudget(execution);

      const progress = this.#kernel.auto_stream_chart_step(
        execution.ids,
        execution.revisions,
        execution.sources,
      );
      if (progress.status === "complete") {
        this.#updateStreamProgress(execution, progress);
        this.#settleStreamingExecution(execution, "complete");
        return false;
      }
      execution.state.status = progress.status === "all_submitted"
        ? "waiting_gpu"
        : "running";
      this.#updateStreamProgress(execution, progress);
      this.#emitStreamingProgress(execution);
      if (progress.status !== "submitted") this.#wakeOnGpuCompletion(execution);
      return progress.status === "submitted";
    } catch (error) {
      this.#failStreamingExecution(execution, error);
      return false;
    }
  }

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
          token: this.#operationToken,
          width,
          height,
        };
      } else {
        this.#kernel.resize(width, height);
        this.#pendingResize = null;
        this.#handleStreamingResize();
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
      this.#notifyStreamingMutation();
      dispatchFiggyEvent(this, "figgy-select", { selected, x, y, originalEvent: event });
    });

    this.#canvas.addEventListener("pointermove", (event) => {
      if (!this.#kernel || !this.#lastPoint || this.busy) {
        return;
      }
      const [x, y] = this.#eventPoint(event);
      const [lastX, lastY] = this.#lastPoint;
      this.#kernel.on_move(x - lastX, y - lastY);
      this.#notifyStreamingMutation();
      this.#lastPoint = [x, y];
      dispatchFiggyEvent(this, "figgy-drag", { x, y, dx: x - lastX, dy: y - lastY });
    });

    const release = () => {
      if (!this.#kernel || !this.#lastPoint) {
        return;
      }
      this.#lastPoint = null;
      const token = this.#operationToken;
      if (token) {
        if (this.#isCurrentOperation(token)) {
          this.#pendingRelease = { token };
        }
        return;
      }
      const kernel = this.#kernel;
      kernel.on_release();
      this.#notifyStreamingMutation();
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

  #beginOperation(kind, kernel = this.kernel, generation = this.#lifecycleGeneration) {
    if (this.busy) {
      throw new Error("figgy chart is busy");
    }
    if (this.#streamSelection && kernel === this.#kernel) {
      kernel.suspend_stream_selection();
      this.#releaseSelectionRequest();
      this.#selectionDirty = true;
    }
    const token = {
      kind,
      generation,
      kernel,
      disposal: "attached",
    };
    this.#operationToken = token;
    return token;
  }

  async #runKernelOperation(kind, operation) {
    const token = this.#beginOperation(kind);
    let failed = false;
    try {
      return await operation(token.kernel, token);
    } catch (error) {
      failed = true;
      throw error;
    } finally {
      try { this.#settleOperation(token); }
      catch (error) {
        if (!failed) throw error;
        dispatchFiggyEvent(this, "figgy-error", { error, operation: "operation_cleanup" });
      }
    }
  }

  #queueBackgroundPrewarm(generation, kernel) {
    this.#pendingPrewarm = { generation, kernel };
    Promise.resolve().then(() => this.#drainBackgroundPrewarm());
  }

  #drainBackgroundPrewarm() {
    const pending = this.#pendingPrewarm;
    if (!pending) {
      return;
    }
    if (!this.#started
        || !this.isConnected
        || this.#lifecycleGeneration !== pending.generation
        || this.#kernel !== pending.kernel) {
      this.#pendingPrewarm = null;
      return;
    }
    if (this.busy) {
      return;
    }
    this.#pendingPrewarm = null;
    this.#runKernelOperation(
      "picker-prewarm",
      (kernel) => kernel.prewarm_gpu_picking(),
    ).catch((error) => {
      if (this.#started
          && this.isConnected
          && this.#lifecycleGeneration === pending.generation
          && this.#kernel === pending.kernel) {
        console.error("figgy picker prewarm:", error);
        dispatchFiggyEvent(this, "figgy-error", {
          error,
          operation: "prewarm_gpu_picking",
          recoverable: true,
        });
      }
    });
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
    if (this.#streamReplay) return this.#replayStreamExport(scale);
    return this.#runKernelOperation("export", (kernel) => kernel.export_png(scale));
  }

  async #waitStreamOperation(token, pending, timeoutMs) {
    let timer = 0;
    try {
      const waits = [pending, token.cancelled];
      if (timeoutMs > 0) {
        waits.push(new Promise((_, reject) => {
          timer = setTimeout(() => reject(new Error(
            `stream ${token.kind} stalled for ${timeoutMs} ms`,
          )), timeoutMs);
        }));
      }
      return await Promise.race(waits);
    } finally {
      if (timer) clearTimeout(timer);
    }
  }

  async #yieldStreamOperation(token, timeoutMs) {
    let channel;
    let timer;
    try {
      const nextTask = new Promise((resolve) => {
        if (typeof MessageChannel === "function") {
          channel = new MessageChannel();
          channel.port1.onmessage = resolve;
          channel.port2.postMessage(null);
        } else {
          timer = setTimeout(resolve, 0);
        }
      });
      await this.#waitStreamOperation(token, nextTask, timeoutMs);
    } finally {
      channel?.port1.close();
      channel?.port2.close();
      if (timer) clearTimeout(timer);
    }
  }

  async #replayStreamExport(scale) {
    if (this.#streamExecution) {
      throw new Error("stream export requires a completed render revision");
    }
    const replay = this.#streamReplay;
    return this.#runKernelOperation("export", async (kernel, token) => {
      const sources = new Map(replay.columns.map((column) => [column.id, column]));
      token.cancelled = new Promise((_, reject) => { token.cancel = reject; });
      token.cancelled.catch(() => {});
      const timeoutMs = replay.stallTimeoutMs;
      let handle = null;
      let finished = false;
      let failed = false;
      try {
        handle = await kernel.begin_stream_export(scale, replay.maxPrimitivesPerChunk);
        if (!this.#isCurrentOperation(token)) {
          throw new DOMException("chart disconnected during stream operation", "AbortError");
        }
        let chunkPrimitives = Math.min(replay.maxPrimitivesPerChunk, 4096);
        kernel.set_stream_operation_chunk_budget(handle, chunkPrimitives);
        let quantumStart = performance.now();
        let steps = 0;
        while (true) {
          if (!this.#isCurrentOperation(token)) {
            throw new DOMException("chart disconnected during stream operation", "AbortError");
          }
          const request = kernel.stream_operation_request_ranges(handle);
          let ids, revisions, sourceLengths, offsets, lengths, encodings, status, submittedBefore;
          try {
            status = request.status;
            if (status === "ready") {
              ids = request.source_ids;
              revisions = request.source_revisions;
              sourceLengths = request.source_lengths;
              offsets = request.offsets;
              lengths = request.lengths;
              encodings = request.encodings;
              submittedBefore = request.submitted_primitives;
            }
          } finally {
            request.free?.();
          }
          if (status === "complete") break;
          if (status === "backpressure" || status === "all_submitted") {
            await this.#waitStreamOperation(token, kernel.streaming_gpu_ready(), timeoutMs);
          } else if (status === "ready") {
            if ([revisions, sourceLengths, offsets, lengths, encodings]
                .some((values) => values.length !== ids.length)) {
              throw new Error("renderer returned mismatched operation range arrays");
            }
            const reads = ids.map(async (id, index) => {
              if (!this.#isCurrentOperation(token)) {
                throw new DOMException("chart disconnected during stream operation", "AbortError");
              }
              const source = sources.get(id);
              const sourceLength = source?.values?.length ?? source?.length;
              const encoding = source?.values
                ? streamArrayEncoding(source.values) : source?.encoding;
              if (!source || source.revision !== revisions[index]
                  || sourceLength !== sourceLengths[index] || encoding !== encodings[index]) {
                throw new Error(`operation source ${id} does not match the completed revision`);
              }
              const offset = offsets[index];
              const length = lengths[index];
              if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(length)
                  || offset < 0 || length < 0 || offset + length > sourceLength) {
                throw new Error(`operation range for ${id} exceeds its source`);
              }
              if (source.values) return source.values.subarray(offset, offset + length);
              return replay.readRange({ id, revision: source.revision, offset, length, encoding });
            });
            const chunks = await this.#waitStreamOperation(token, Promise.all(reads), timeoutMs);
            if (!this.#isCurrentOperation(token)) {
              throw new DOMException("chart disconnected during stream operation", "AbortError");
            }
            for (let index = 0; index < chunks.length; index += 1) {
              validateStreamRangeArray(chunks[index], encodings[index], lengths[index], ids[index]);
            }
            const submitStarted = performance.now();
            const progress = kernel.stream_operation_submit_ranges(
              handle, ids, revisions, sourceLengths, offsets, chunks,
            );
            let submitted;
            try { submitted = progress.submitted_primitives - submittedBefore; }
            finally { progress.free?.(); }
            if (submitted > 0) {
              const elapsed = performance.now() - submitStarted;
              const estimate = Math.floor(submitted * replay.maxFrameTimeMs * 0.5 / Math.max(0.05, elapsed));
              chunkPrimitives = Math.max(1, Math.min(
                replay.maxPrimitivesPerChunk, chunkPrimitives * 2, estimate,
              ));
              kernel.set_stream_operation_chunk_budget(handle, chunkPrimitives);
            }
          } else {
            throw new Error(`renderer returned unexpected operation state: ${status}`);
          }
          if (++steps >= 32 || performance.now() - quantumStart >= replay.maxFrameTimeMs) {
            await this.#yieldStreamOperation(token, timeoutMs);
            quantumStart = performance.now();
            steps = 0;
          }
        }
        const result = await kernel.finish_stream_export(handle);
        finished = true;
        return result;
      } catch (error) {
        failed = true;
        throw error;
      } finally {
        token.cancel = null;
        if (handle) {
          let cleanupError = null;
          try {
            if (!finished) await kernel.cancel_stream_operation_and_wait(handle);
          } catch (error) {
            cleanupError = error;
          }
          try { handle.free?.(); }
          catch (error) { cleanupError ??= error; }
          if (cleanupError) {
            if (!failed) throw cleanupError;
            dispatchFiggyEvent(this, "figgy-error", {
              error: cleanupError, operation: "stream_operation_cleanup",
            });
          }
        }
      }
    });
  }

  async first_frame_ready() {
    await this.#runKernelOperation("first-frame", (kernel) => kernel.first_frame_ready());
  }

  warm_up() {
    return this.first_frame_ready();
  }

  async ensure_extent_engine() {
    await this.#runKernelOperation(
      "extent-prewarm",
      (kernel) => kernel.ensure_extent_engine(),
    );
  }

  async prewarm_gpu_picking() {
    await this.#runKernelOperation(
      "picker-prewarm",
      (kernel) => kernel.prewarm_gpu_picking(),
    );
  }

  async prewarm_all_with_progress(onEvent) {
    await this.#runKernelOperation(
      "prewarm-all",
      (kernel) => kernel.prewarm_all_with_progress(onEvent),
    );
  }

  async prewarm_all() {
    await this.#runKernelOperation(
      "prewarm-all",
      (kernel) => kernel.prewarm_all(),
    );
  }

  #settleOperation(token) {
    let cleanupError = null;
    let resized = false;
    if (this.#isCurrentOperation(token) && this.#pendingRelease?.token === token) {
      this.#pendingRelease = null;
      try {
        token.kernel.on_release();
        const selected = token.kernel.has_selection();
        dispatchFiggyEvent(this, "figgy-release", { selected });
      } catch (error) {
        cleanupError = error;
      }
    }

    if (this.#isCurrentOperation(token) && this.#pendingResize?.token === token) {
      const { width, height } = this.#pendingResize;
      this.#pendingResize = null;
      try {
        token.kernel.resize(width, height);
        resized = true;
      } catch (error) {
        cleanupError ??= error;
      }
    }

    if (this.#operationToken === token) {
      this.#operationToken = null;
    }
    if (token.disposal === "deferred") {
      token.disposal = "freed";
      if (token.kernel) {
        try {
          token.kernel.free();
        } catch (error) {
          cleanupError ??= error;
        }
      }
    }
    if (this.#pendingPrewarm) {
      Promise.resolve().then(() => this.#drainBackgroundPrewarm());
    }
    if (resized && !cleanupError && this.#kernel === token.kernel) {
      this.#handleStreamingResize();
    }
    if (this.#streamExecution || this.#streamSelection || this.#selectionDirty) this.#queueStreamTask();
    const execution = this.#streamExecution;
    if (execution?.cancelPromise && !execution.draining) {
      this.#drainStreamingCancellation(execution);
    }
    if (cleanupError) {
      throw cleanupError;
    }
  }

  free() {
    const operationToken = this.#operationToken;
    const active = this.#started
      || this.#kernel !== null
      || this.#resizeObserver !== null
      || this.#raf !== 0
      || this.#operationToken !== null;
    if (!active) {
      return;
    }

    const kernel = this.#kernel;
    this.#releaseSelectionRequest();
    this.#selectionDirty = false;
    this.#kernel = null;
    ++this.#lifecycleGeneration;
    operationToken?.cancel?.(new DOMException("chart disconnected during stream operation", "AbortError"));
    if (this.#streamExecution) {
      const execution = this.#streamExecution;
      this.#settleStreamingExecution(this.#streamExecution, "cancelled");
      if (!execution.draining && execution.cancelReject) {
        execution.cancelReject(new DOMException("chart freed before cancellation drain", "AbortError"));
      }
    }
    this.#streamReplay = null;
    this.#streamSources.clear();
    this.#lastStreamState = null;
    this.#streamRefreshRequested = false;
    if (this.#streamWake) {
      clearTimeout(this.#streamWake);
      this.#streamWake = 0;
    }
    this.#streamChannel?.port1.close();
    this.#streamChannel?.port2.close();
    this.#streamChannel = null;
    this.#streamTaskPending = false;
    this.#started = false;
    this.#lastPoint = null;
    this.#pendingResize = null;
    this.#pendingRelease = null;
    this.#pendingPrewarm = null;
    this.#operationToken = null;
    if (operationToken?.disposal === "attached") {
      operationToken.disposal = "deferred";
    }

    if (this.#raf) {
      cancelAnimationFrame(this.#raf);
      this.#raf = 0;
    }
    if (this.#resizeObserver) {
      this.#resizeObserver.disconnect();
      this.#resizeObserver = null;
    }
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
      if (operationToken?.kernel !== kernel) {
        kernel.free();
      }
    }
  }

  register_font(bytes) { return this.#kernelForCall().register_font(bytes); }
  configure_streaming(maxActiveCharts, maxInFlightChunks, maxColumnsPerChunk,
    maxChunkInputBytes, maxInFlightGpuBytes) {
    return this.#kernelForCall().configure_streaming(
      maxActiveCharts,
      maxInFlightChunks,
      maxColumnsPerChunk,
      maxChunkInputBytes,
      maxInFlightGpuBytes,
    );
  }
  configure_auto_residency(memoryBudgetBytes, workingSetLimitBytes) {
    return this.#kernelForCall().configure_auto_residency(
      memoryBudgetBytes,
      workingSetLimitBytes,
    );
  }
  register_streaming_columns(ids, revisions, sources) {
    const result = this.#kernelForCall().register_streaming_columns(ids, revisions, sources);
    for (let index = 0; index < ids.length; index += 1) {
      this.#streamSources.set(ids[index], {
        revision: revisions[index],
        values: sources[index],
      });
    }
    return result;
  }
  replace_streaming_columns(ids, revisions, sources) {
    const result = this.#kernelForCall().replace_streaming_columns(ids, revisions, sources);
    for (let index = 0; index < ids.length; index += 1) {
      this.#streamSources.set(ids[index], {
        revision: revisions[index],
        values: sources[index],
      });
    }
    return result;
  }
  demote_auto_resident_columns(ids, revisions, sources) {
    const result = this.#kernelForCall().demote_auto_resident_columns(ids, revisions, sources);
    for (let index = 0; index < ids.length; index += 1) {
      this.#streamSources.set(ids[index], {
        revision: revisions[index],
        values: sources[index],
      });
    }
    return result;
  }
  #renderRangeStreamingChart({
    columns,
    maxPrimitivesPerChunk,
    maxFrameTimeMs,
    stallTimeoutMs,
    signal,
    onProgress,
    readRange,
  }) {
    const kernel = this.#kernelForCall();
    const ids = [];
    const revisions = [];
    const lengths = [];
    const encodings = [];
    const rangeSources = new Map();
    const seen = new Set();
    let registeredCount = 0;
    let changedCount = 0;
    for (const column of columns) {
      if (!column || typeof column.id !== "string" || column.id.length === 0) {
        throw new TypeError("each streaming column needs a non-empty id");
      }
      if (seen.has(column.id)) {
        throw new TypeError(`streaming column ${column.id} appears more than once`);
      }
      if (!Number.isSafeInteger(column.revision) || column.revision < 0) {
        throw new RangeError(`streaming column ${column.id} has an invalid revision`);
      }
      const valuesEncoding = column.values ? streamArrayEncoding(column.values) : null;
      if (column.values && !valuesEncoding) {
        throw new TypeError(
          `streaming column ${column.id} values must be a Float32Array or Float64Array`,
        );
      }
      const encoding = valuesEncoding ?? column.encoding;
      if (encoding !== "f32" && encoding !== "f64") {
        throw new TypeError(`streaming column ${column.id} needs encoding 'f32' or 'f64'`);
      }
      const length = column.values?.length ?? column.length;
      if (!Number.isSafeInteger(length) || length < 0) {
        throw new RangeError(`streaming column ${column.id} has an invalid length`);
      }
      if (!column.values && typeof readRange !== "function") {
        throw new TypeError("readRange must be a function when a column has no typed array");
      }
      seen.add(column.id);
      ids.push(column.id);
      revisions.push(column.revision);
      lengths.push(length);
      encodings.push(encoding);
      rangeSources.set(column.id, {
        revision: column.revision,
        length,
        encoding,
        values: column.values ?? null,
      });
      const current = this.#streamSources.get(column.id);
      if (current) {
        registeredCount += 1;
        if (current.revision === column.revision
            && ((current.length ?? current.values?.length) !== length
              || (current.encoding ?? streamArrayEncoding(current.values)) !== encoding)) {
          throw new Error(
            `streaming column ${column.id} changed metadata without a new revision`,
          );
        }
        if (current.revision !== column.revision) {
          changedCount += 1;
        }
      }
    }

    if (!Number.isSafeInteger(this.#nextStreamExecution)) {
      throw new Error("stream execution counter exhausted");
    }
    if (registeredCount !== 0 && registeredCount !== columns.length) {
      throw new Error(
        "one render request cannot mix new and registered range-provider columns",
      );
    }
    if (registeredCount === 0) {
      kernel.register_streaming_column_sources(ids, revisions, lengths, encodings);
    } else if (changedCount > 0) {
      const changedIds = [];
      const changedRevisions = [];
      const changedLengths = [];
      const changedEncodings = [];
      for (let index = 0; index < ids.length; index += 1) {
        if (this.#streamSources.get(ids[index]).revision === revisions[index]) continue;
        changedIds.push(ids[index]);
        changedRevisions.push(revisions[index]);
        changedLengths.push(lengths[index]);
        changedEncodings.push(encodings[index]);
      }
      kernel.replace_streaming_column_sources(
        changedIds,
        changedRevisions,
        changedLengths,
        changedEncodings,
      );
    }
    for (const [id, source] of rangeSources) {
      this.#streamSources.set(id, { ...source });
    }
    const request = kernel.request_auto_streaming_chart(maxPrimitivesPerChunk);
    if (request.status === "active" && this.#streamExecution) {
      return this.#streamExecution.handle;
    }
    if (request.status === "complete") {
      if (this.#streamExecution) {
        const handle = this.#streamExecution.handle;
        this.#updateStreamProgress(this.#streamExecution, request);
        this.#settleStreamingExecution(this.#streamExecution, "complete");
        return handle;
      }
      return this.#completedStreamingJob(onProgress);
    }
    if (request.status !== "started") {
      throw new Error(`renderer returned unexpected automatic stream state: ${request.status}`);
    }
    if (!Number.isSafeInteger(this.#nextStreamExecution)) {
      kernel.interrupt_render();
      throw new Error("stream execution counter exhausted");
    }
    if (this.#streamExecution) {
      this.#settleStreamingExecution(this.#streamExecution, "superseded");
    }

    let resolve;
    let reject;
    const done = new Promise((yes, no) => {
      resolve = yes;
      reject = no;
    });
    const state = {
      executionId: this.#nextStreamExecution++,
      status: "running",
      submittedPrimitives: 0,
      totalPrimitives: 0,
      done,
    };
    const execution = {
      kernel,
      state,
      resolve,
      reject,
      settled: false,
      ids: [],
      revisions: [],
      sources: [],
      rangeSources,
      readRange,
      rangePending: false,
      rangeGeneration: 0,
      rangeResolved: null,
      rangeError: null,
      restartForResize: false,
      maxPrimitivesPerChunk,
      maxFrameTimeMs,
      stallTimeoutMs,
      chunkPrimitives: Math.min(maxPrimitivesPerChunk, 4096),
      lastProgressTime: performance.now(),
      signal,
      abortListener: null,
      onProgress,
      handle: null,
      replayToken: this.#streamReplay?.token,
      previousReplay: this.#streamReplay?.previousReplay ?? null,
    };
    execution.handle = new FiggyRenderJob(state, () => this.#cancelStreamingExecution(execution));
    this.#streamExecution = execution;
    this.#scheduleStreamWake();
    this.#queueStreamTask();
    try {
      this.#bindRangeStreamingSources(execution, request);
    } catch (error) {
      try {
        kernel.interrupt_render();
      } catch {
        // Preserve the source identity error.
      }
      this.#settleStreamingExecution(execution, "failed", error);
      return execution.handle;
    }
    if (signal) {
      execution.abortListener = () => execution.handle.cancel();
      signal.addEventListener("abort", execution.abortListener, { once: true });
      if (signal.aborted) execution.abortListener();
    }
    this.#emitStreamingProgress(execution);
    return execution.handle;
  }
  render_streaming_chart({
    columns,
    maxPrimitivesPerChunk = 262144,
    maxFrameTimeMs = 8,
    stallTimeoutMs = 30000,
    signal = null,
    onProgress = null,
    readRange = null,
  } = {}) {
    this.#kernelForCall();
    if (!Array.isArray(columns) || columns.length === 0) {
      throw new TypeError("columns must be a non-empty array");
    }
    if (!Number.isSafeInteger(maxPrimitivesPerChunk) || maxPrimitivesPerChunk <= 0) {
      throw new RangeError("maxPrimitivesPerChunk must be a positive safe integer");
    }
    if (!Number.isFinite(maxFrameTimeMs) || maxFrameTimeMs <= 0) {
      throw new RangeError("maxFrameTimeMs must be finite and positive");
    }
    if (!Number.isFinite(stallTimeoutMs) || stallTimeoutMs < 0) {
      throw new RangeError("stallTimeoutMs must be finite and non-negative (0 disables it)");
    }
    if (onProgress !== null && typeof onProgress !== "function") {
      throw new TypeError("onProgress must be a function");
    }
    if (signal !== null
        && (typeof signal.addEventListener !== "function"
          || typeof signal.removeEventListener !== "function")) {
      throw new TypeError("signal must be an AbortSignal");
    }
    if (readRange !== null && typeof readRange !== "function") {
      throw new TypeError("readRange must be a function");
    }
    const replay = this.#createStreamReplay(
      columns,
      maxPrimitivesPerChunk,
      onProgress,
      readRange,
      maxFrameTimeMs,
      stallTimeoutMs,
    );
    if (readRange !== null || columns.some((column) => !column?.values)) {
      return this.#renderWithStreamReplay(
        replay,
        () => this.#renderRangeStreamingChart({
          columns,
          maxPrimitivesPerChunk,
          maxFrameTimeMs,
          stallTimeoutMs,
          signal,
          onProgress,
          readRange,
        }),
      );
    }

    return this.#renderWithStreamReplay(
      replay,
      () => this.#renderTypedArrayStreamingChart({
        columns,
        maxPrimitivesPerChunk,
        maxFrameTimeMs,
        stallTimeoutMs,
        signal,
        onProgress,
      }),
    );
  }

  #renderTypedArrayStreamingChart({
    columns,
    maxPrimitivesPerChunk,
    maxFrameTimeMs,
    stallTimeoutMs,
    signal,
    onProgress,
  }) {
    const kernel = this.#kernelForCall();

    const ids = [];
    const revisions = [];
    const sources = [];
    const seen = new Set();
    let existing = 0;
    for (const column of columns) {
      if (!column || typeof column.id !== "string" || column.id.length === 0) {
        throw new TypeError("each streaming column needs a non-empty id");
      }
      if (seen.has(column.id)) {
        throw new TypeError(`streaming column ${column.id} appears more than once`);
      }
      if (!Number.isSafeInteger(column.revision) || column.revision < 0) {
        throw new RangeError(`streaming column ${column.id} has an invalid revision`);
      }
      if (!column.values || !Number.isSafeInteger(column.values.length)) {
        throw new TypeError(`streaming column ${column.id} needs a typed-array source`);
      }
      seen.add(column.id);
      ids.push(column.id);
      revisions.push(column.revision);
      sources.push(column.values);
      existing += this.#streamSources.has(column.id) ? 1 : 0;
    }
    if (existing !== 0 && existing !== columns.length) {
      throw new Error(
        "one render request cannot mix newly registered and replacement streaming columns",
      );
    }

    if (existing === 0) {
      this.register_streaming_columns(ids, revisions, sources);
    } else {
      const changedIds = [];
      const changedRevisions = [];
      const changedSources = [];
      for (let index = 0; index < ids.length; index += 1) {
        const current = this.#streamSources.get(ids[index]);
        if (current.revision === revisions[index]) {
          if (current.values && current.values !== sources[index]) {
            throw new Error(
              `streaming column ${ids[index]} changed values without a new revision`,
            );
          }
          if (!current.values) {
            if (current.length !== sources[index].length
                || current.encoding !== streamArrayEncoding(sources[index])) {
              throw new Error(`streaming column ${ids[index]} changed metadata without a new revision`);
            }
            this.#streamSources.set(ids[index], { ...current, values: sources[index] });
          }
          continue;
        }
        changedIds.push(ids[index]);
        changedRevisions.push(revisions[index]);
        changedSources.push(sources[index]);
      }
      if (changedIds.length > 0) {
        this.replace_streaming_columns(changedIds, changedRevisions, changedSources);
      }
    }

    const request = kernel.request_auto_streaming_chart(maxPrimitivesPerChunk);
    if (request.status === "active" && this.#streamExecution) {
      return this.#streamExecution.handle;
    }
    if (request.status === "complete" && this.#streamExecution) {
      const handle = this.#streamExecution.handle;
      this.#updateStreamProgress(this.#streamExecution, request);
      this.#settleStreamingExecution(this.#streamExecution, "complete");
      return handle;
    }
    if (request.status === "complete") {
      return this.#completedStreamingJob(onProgress);
    }
    if (request.status !== "started") {
      throw new Error(`renderer returned unexpected automatic stream state: ${request.status}`);
    }
    if (!Number.isSafeInteger(this.#nextStreamExecution)) {
      kernel.interrupt_render();
      throw new Error("stream execution counter exhausted");
    }

    if (this.#streamExecution) {
      this.#settleStreamingExecution(this.#streamExecution, "superseded");
    }
    let resolve;
    let reject;
    const done = new Promise((yes, no) => {
      resolve = yes;
      reject = no;
    });
    const state = {
      executionId: this.#nextStreamExecution++,
      status: "running",
      submittedPrimitives: 0,
      totalPrimitives: 0,
      done,
    };
    const execution = {
      kernel,
      state,
      resolve,
      reject,
      settled: false,
      ids: [],
      revisions: [],
      sources: [],
      maxPrimitivesPerChunk,
      maxFrameTimeMs,
      stallTimeoutMs,
      chunkPrimitives: Math.min(maxPrimitivesPerChunk, 4096),
      lastProgressTime: performance.now(),
      signal,
      abortListener: null,
      onProgress,
      handle: null,
      replayToken: this.#streamReplay?.token,
      previousReplay: this.#streamReplay?.previousReplay ?? null,
    };
    execution.handle = new FiggyRenderJob(state, () => this.#cancelStreamingExecution(execution));
    this.#streamExecution = execution;
    this.#scheduleStreamWake();
    this.#queueStreamTask();
    try {
      this.#bindStreamingSources(execution, request);
    } catch (error) {
      try {
        kernel.interrupt_render();
      } catch {
        // Preserve the original missing-source contract failure.
      }
      this.#settleStreamingExecution(execution, "failed", error);
      return execution.handle;
    }
    if (signal) {
      execution.abortListener = () => execution.handle.cancel();
      signal.addEventListener("abort", execution.abortListener, { once: true });
      if (signal.aborted) {
        execution.abortListener();
      }
    }
    this.#emitStreamingProgress(execution);
    return execution.handle;
  }
  render_chart(options = {}) {
    return this.render_streaming_chart(options);
  }
  stream_status() {
    const state = this.#streamExecution?.state ?? this.#lastStreamState;
    const renderer = JSON.parse(this.#kernelForCall().stream_status());
    return {
      ...renderer,
      execution_id: state?.executionId ?? null,
      execution_status: state?.status ?? null,
    };
  }
  inspect_column_admission(metadata) {
    if (!Array.isArray(metadata)) throw new TypeError("metadata must be an array");
    const lengths = metadata.map((column) => {
      if (!column || !Number.isSafeInteger(column.length) || column.length < 0) {
        throw new TypeError("each column metadata entry needs a non-negative safe integer length");
      }
      return column.length;
    });
    return JSON.parse(this.#kernelForCall().inspect_column_admission(lengths));
  }
  streaming_capabilities() {
    return JSON.parse(this.#kernelForCall().streaming_capabilities());
  }
  set_pool_auto_growth(enabled) {
    if (typeof enabled !== "boolean") throw new TypeError("enabled must be a boolean");
    return this.#kernelForCall().set_pool_auto_growth(enabled);
  }
  gpu_memory_status() {
    return JSON.parse(this.#kernelForCall().gpu_memory_status());
  }
  async release_unused_gpu_memory() {
    if (this.#streamExecution) {
      throw new Error("finish or cancel the active stream before GPU pool cleanup");
    }
    const status = await this.#runKernelOperation(
      "gpu-memory-cleanup",
      (kernel) => kernel.release_unused_gpu_memory(),
    );
    return JSON.parse(status);
  }
  request_auto_streaming_chart(maxPrimitivesPerChunk) {
    return this.#kernelForCall().request_auto_streaming_chart(maxPrimitivesPerChunk);
  }
  auto_stream_chart_step(ids, revisions, sources) {
    return this.#kernelForCall().auto_stream_chart_step(ids, revisions, sources);
  }
  register_streaming_column_sources(ids, revisions, lengths, encodings) {
    const result = this.#kernelForCall().register_streaming_column_sources(
      ids, revisions, lengths, encodings,
    );
    for (let index = 0; index < ids.length; index += 1) {
      this.#streamSources.set(ids[index], {
        revision: revisions[index], length: lengths[index], encoding: encodings[index], values: null,
      });
    }
    return result;
  }
  replace_streaming_column_sources(ids, revisions, lengths, encodings) {
    const result = this.#kernelForCall().replace_streaming_column_sources(
      ids, revisions, lengths, encodings,
    );
    for (let index = 0; index < ids.length; index += 1) {
      this.#streamSources.set(ids[index], {
        revision: revisions[index], length: lengths[index], encoding: encodings[index], values: null,
      });
    }
    this.#notifyStreamingMutation();
    return result;
  }
  request_resident_stream_handoff(ids, revisions, lengths, encodings,
    maxPrimitivesPerChunk) {
    return this.#kernelForCall().request_resident_stream_handoff(
      ids, revisions, lengths, encodings, maxPrimitivesPerChunk,
    );
  }
  auto_stream_chart_request_ranges() {
    return this.#kernelForCall().auto_stream_chart_request_ranges();
  }
  auto_stream_chart_submit_ranges(ids, revisions, sourceLengths, offsets, sources) {
    return this.#kernelForCall().auto_stream_chart_submit_ranges(
      ids, revisions, sourceLengths, offsets, sources,
    );
  }
  interrupt_render() { return this.#kernelForCall().interrupt_render(); }
  begin_streaming(maxPrimitivesPerChunk) {
    return this.#kernelForCall().begin_streaming(maxPrimitivesPerChunk);
  }
  stream_step(ids, revisions, sources) {
    return this.#kernelForCall().stream_step(ids, revisions, sources);
  }
  cancel_streaming() { return this.#kernelForCall().cancel_streaming(); }
  streaming_usage() { return this.#kernelForCall().streaming_usage(); }
  register_column_f32(id, data) { return this.#kernelForCall().register_column_f32(id, data); }
  register_column_f64(id, data) { return this.#kernelForCall().register_column_f64(id, data); }
  // Matrix batch: `data` is one flat buffer of ids.length x valuesPerColumn
  // values, in id order. One upload for the whole batch, all-or-nothing.
  register_columns_f32(ids, data, valuesPerColumn) {
    return this.#kernelForCall().register_columns_f32(ids, data, valuesPerColumn);
  }
  register_columns_f64(ids, data, valuesPerColumn) {
    return this.#kernelForCall().register_columns_f64(ids, data, valuesPerColumn);
  }
  update_register_column_f32(id, data) {
    return this.#mutateChart("update_register_column_f32", id, data);
  }
  update_register_column_f64(id, data) {
    return this.#mutateChart("update_register_column_f64", id, data);
  }
  remove_column(id) { return this.#mutateChart("remove_column", id); }
  add_line_series(seriesId, xColumn, yColumn, lineWidth, label) {
    return this.#mutateChart("add_line_series", seriesId, xColumn, yColumn, lineWidth, label);
  }
  set_series_label(seriesId, label) { return this.#mutateChart("set_series_label", seriesId, label); }
  remove_series(seriesId) { return this.#mutateChart("remove_series", seriesId); }
  auto_fit_x(column, padding) { return this.#mutateChart("auto_fit_x", column, padding); }
  auto_fit_y(column, padding) { return this.#mutateChart("auto_fit_y", column, padding); }
  auto_fit_colorbar(padding) { return this.#mutateChart("auto_fit_colorbar", padding); }
  async auto_fit_all(padding) {
    const kernel = this.#kernelForCall();
    if (kernel.request_stream_auto_fit(padding)) {
      let execution = this.#streamExecution;
      let job;
      if (execution) {
        if (execution.cancelPromise) throw new DOMException("stream is cancelling", "AbortError");
        execution.reconcileRequested = true;
        this.#queueStreamTask();
        job = execution.handle;
      } else if (this.#streamReplay) {
        job = this.#replayCompletedStreamAfterResize();
      } else {
        const columns = [...this.#streamSources].map(([id, source]) => ({ id, ...source }));
        if (!columns.length || columns.some((column) => !column.values)) {
          throw new Error("stream auto-fit needs replayable sources; call render_chart with readRange first");
        }
        job = this.render_chart({ columns });
      }
      if (!job) throw new Error("stream auto-fit could not start its replay");
      const result = await job.done;
      if (result.status !== "complete" && result.status !== "resident") {
        throw new DOMException(`stream auto-fit ${result.status}`, "AbortError");
      }
      if (this.#kernel !== kernel) throw new DOMException("chart was freed during auto-fit", "AbortError");
      if (job.autoFitPending) {
        throw new Error("stream completed without committing the requested auto-fit");
      }
      return;
    }
    await this.#runKernelOperation(
      "auto-fit",
      (kernel) => kernel.auto_fit_all(padding),
    );
  }
  set_contour_nice_levels(seriesId, targetCount, useColormapColors) {
    return this.#mutateChart("set_contour_nice_levels",
      seriesId,
      targetCount,
      useColormapColors,
    );
  }
  series_draw_info(seriesId) {
    return JSON.parse(this.#kernelForCall().series_draw_info(seriesId));
  }
  set_title(text) { return this.#mutateChart("set_title", text); }
  set_x_title(text) { return this.#mutateChart("set_x_title", text); }
  set_y_title(text) { return this.#mutateChart("set_y_title", text); }
  set_colorbar_title(text) { return this.#mutateChart("set_colorbar_title", text); }
  set_colorbar_axis(json) { return this.#mutateChart("set_colorbar_axis", json); }
  apply_axis_preset(preset) { return this.#mutateChart("apply_axis_preset", preset); }
  apply_color_cycle(cycle) { return this.#mutateChart("apply_color_cycle", cycle); }
  get_config() { return this.#kernelForCall().get_config(); }
  set_config(json) { return this.#mutateChart("set_config", json); }
  get_series() { return this.#kernelForCall().get_series(); }
  set_series(json) { return this.#mutateChart("set_series", json); }
  reset_legend_from_series_labels() { return this.#mutateChart("reset_legend_from_series_labels"); }
  hit_test(x, y) {
    const hit = this.#kernelForCall().hit_test(x, y);
    return hit === undefined ? null : hit;
  }
  async pick_point(x, y, maxDistancePx) {
    // The renderer only picks a completed chart-local packed view. A streamed
    // image without that cache returns no hit; it never replays its source.
    const hit = await this.#runKernelOperation(
      "pick",
      (kernel) => kernel.pick_point(x, y, maxDistancePx),
    );
    return hit === undefined ? null : JSON.parse(hit);
  }
  async pick_data(x, y, maxDistancePx) {
    const hit = await this.#runKernelOperation(
      "pick",
      (kernel) => kernel.pick_data(x, y, maxDistancePx),
    );
    return hit === undefined ? null : JSON.parse(hit);
  }
  next_view_point_index(sourceId, seriesId, current, forward) {
    const index = this.#kernelForCall().next_view_point_index(sourceId ?? undefined, seriesId, current, forward);
    return index === undefined ? null : index;
  }
  set_picked_points(json) { return this.#mutateSelection("set_picked_points", json); }
  set_picked_data(json) { return this.#mutateSelection("set_picked_data", json); }
  set_clear_color(r, g, b, a) { return this.#mutateChart("set_clear_color", r, g, b, a); }
  load_demo() { return this.#kernelForCall().load_demo(); }
  on_press(x, y) { return this.#mutateChart("on_press", x, y); }
  on_move(dx, dy) { return this.#mutateChart("on_move", dx, dy); }
  on_release() { return this.#mutateChart("on_release"); }
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
