// Runs the nearest-datum pick kernel (#381 prototype) in headless Chromium
// and times one pick per size: the kernel over every datum, the merge of the
// per-workgroup K nearest and the readback of the K nearest. Measurement only.
//
// node ci/pick_nearest_browser.cjs target/ci-results/pick_nearest_browser.json report.json
//
// The input comes from the renderer's `export_nearest_browser_inputs` test
// (`cargo test -p figgy-renderer --release --lib export_nearest_browser_inputs -- --ignored`):
// the basic module's source and, per size, the query parameters. The pool is
// filled on the GPU (`FILL` below): x pairs from word 0, y pairs from word
// 2 × points, spread over the plot or all within one pixel of the cursor.
// Requires Playwright on Node's module search path; see ci/README.md.
const fs = require('node:fs');
const http = require('node:http');
const {chromium} = require('playwright');

const FILL = `
struct Fill { points: u32, all_inside: u32, _a: u32, _b: u32 };
@group(0) @binding(0) var<storage, read_write> pool: array<u32>;
@group(0) @binding(1) var<uniform> fill: Fill;
fn hash(i: u32, seed: u32) -> f32 {
  var x = (i * 0x9E3779B9u) ^ (seed * 0x85EBCA6Bu);
  x = x ^ (x >> 16u); x = x * 0x7FEB352Du; x = x ^ (x >> 15u); x = x * 0x846CA68Bu; x = x ^ (x >> 16u);
  return f32(x >> 8u) / 16777216.0;
}
@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(num_workgroups) n: vec3<u32>) {
  let i = id.x + id.y * n.x * 256u;
  if i >= fill.points { return; }
  for (var axis = 0u; axis < 2u; axis = axis + 1u) {
    let unit = hash(i, axis + 1u);
    let v = select(unit * 1000.0, 500.0 + (unit - 0.5) * 0.8, fill.all_inside != 0u);
    let word = axis * 2u * fill.points + 2u * i;
    pool[word] = bitcast<u32>(v);
    pool[word + 1u] = 0u;
  }
}`;

(async () => {
  const [input, output] = process.argv.slice(2);
  if (!input || !output) throw Error('Usage: node ci/pick_nearest_browser.cjs inputs.json report.json');
  const inputs = JSON.parse(fs.readFileSync(input, 'utf8'));
  const server = http.createServer((_, res) => {res.writeHead(200, {'Content-Type': 'text/html'}); res.end('<!doctype html><title>figgy nearest pick</title>');});
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({headless: true, executablePath: process.env.BROWSER_EXECUTABLE_PATH || undefined, args: ['--enable-unsafe-webgpu']});
  const report = {browser: browser.version(), rows: []};
  try {
    const page = await browser.newPage();
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    page.on('console', msg => console.log('[page]', msg.text()));
    report.adapter = await page.evaluate(async ({source, fillSource}) => {
      const adapter = await navigator.gpu.requestAdapter();
      const limits = adapter.limits;
      window.device = await adapter.requestDevice({requiredLimits: {
        maxStorageBufferBindingSize: limits.maxStorageBufferBindingSize,
        maxBufferSize: limits.maxBufferSize,
        maxStorageBuffersPerShaderStage: limits.maxStorageBuffersPerShaderStage,
      }});
      const storage = (binding, type) => ({binding, visibility: GPUShaderStage.COMPUTE, buffer: {type}});
      const groups = [
        [storage(0, 'read-only-storage'), storage(1, 'storage'), storage(2, 'storage')],
        [{binding: 0, visibility: GPUShaderStage.COMPUTE, buffer: {type: 'uniform'}}, storage(1, 'read-only-storage'), storage(2, 'read-only-storage'), storage(3, 'storage')],
        [storage(0, 'read-only-storage'), storage(1, 'read-only-storage')],
      ].map(entries => device.createBindGroupLayout({entries}));
      window.layouts = groups;
      const start = performance.now();
      const module = device.createShaderModule({code: source});
      window.pipeline = await device.createComputePipelineAsync({
        layout: device.createPipelineLayout({bindGroupLayouts: groups}),
        compute: {module, entryPoint: 'pick_nearest_points'},
      });
      window.merge = await device.createComputePipelineAsync({layout: 'auto', compute: {module, entryPoint: 'pick_nearest_merge'}});
      window.compileMs = performance.now() - start;
      window.fill = await device.createComputePipelineAsync({layout: 'auto', compute: {module: device.createShaderModule({code: fillSource}), entryPoint: 'main'}});
      return {architecture: adapter.info.architecture, maxStorageBufferBindingSize: limits.maxStorageBufferBindingSize, maxBufferSize: limits.maxBufferSize, compileMs: window.compileMs};
    }, {source: inputs.source, fillSource: FILL});
    console.log(JSON.stringify(report.adapter));
    for (const c of inputs.cases) {
      for (const allInside of [false, true]) {
        const row = await page.evaluate(async ({c, allInside, candidateBytes, k}) => {
          const poolBytes = c.points * 16;
          if (poolBytes > device.limits.maxStorageBufferBindingSize) {
            return {points: c.points, allInside, refused: `pool of ${poolBytes} B exceeds the ${device.limits.maxStorageBufferBindingSize} B storage binding`};
          }
          const S = GPUBufferUsage;
          const buffer = (size, usage) => device.createBuffer({size: Math.max(size, 16), usage});
          const pool = buffer(poolBytes, S.STORAGE);
          const fillParams = buffer(16, S.UNIFORM | S.COPY_DST);
          device.queue.writeBuffer(fillParams, 0, new Uint32Array([c.points, allInside ? 1 : 0, 0, 0]));
          const fillGroups = Math.ceil(c.points / 256);
          const fx = Math.min(fillGroups, device.limits.maxComputeWorkgroupsPerDimension);
          let encoder = device.createCommandEncoder();
          let pass = encoder.beginComputePass();
          pass.setPipeline(fill);
          pass.setBindGroup(0, device.createBindGroup({layout: fill.getBindGroupLayout(0), entries: [{binding: 0, resource: {buffer: pool}}, {binding: 1, resource: {buffer: fillParams}}]}));
          pass.dispatchWorkgroups(fx, Math.ceil(fillGroups / fx));
          pass.end();
          device.queue.submit([encoder.finish()]);
          await device.queue.onSubmittedWorkDone();
          const params = buffer(c.params.length * 4, S.UNIFORM | S.COPY_DST);
          device.queue.writeBuffer(params, 0, new Uint32Array(c.params));
          const candidatesBytes = c.groups * k * candidateBytes;
          const candidates = buffer(candidatesBytes, S.STORAGE);
          const result = buffer(k * candidateBytes, S.STORAGE | S.COPY_SRC);
          const readback = buffer(k * candidateBytes, S.MAP_READ | S.COPY_DST);
          const small = () => buffer(64, S.STORAGE);
          const g0 = device.createBindGroup({layout: layouts[0], entries: [{binding: 0, resource: {buffer: pool}}, {binding: 1, resource: {buffer: small()}}, {binding: 2, resource: {buffer: candidates}}]});
          const g1 = device.createBindGroup({layout: layouts[1], entries: [{binding: 0, resource: {buffer: params}}, {binding: 1, resource: {buffer: small()}}, {binding: 2, resource: {buffer: small()}}, {binding: 3, resource: {buffer: small()}}]});
          const gm = device.createBindGroup({layout: merge.getBindGroupLayout(0), entries: [{binding: 2, resource: {buffer: result}}, {binding: 3, resource: {buffer: candidates}}]});
          const g2 = device.createBindGroup({layout: layouts[2], entries: [{binding: 0, resource: {buffer: small()}}, {binding: 1, resource: {buffer: small()}}]});
          const pick = async () => {
            const start = performance.now();
            const encoder = device.createCommandEncoder();
            const pass = encoder.beginComputePass();
            pass.setPipeline(pipeline);
            pass.setBindGroup(0, g0); pass.setBindGroup(1, g1); pass.setBindGroup(2, g2);
            pass.dispatchWorkgroups(c.dispatch[0], c.dispatch[1]);
            pass.end();
            const mergePass = encoder.beginComputePass();
            mergePass.setPipeline(merge);
            mergePass.setBindGroup(0, gm);
            mergePass.dispatchWorkgroups(1);
            mergePass.end();
            encoder.copyBufferToBuffer(result, 0, readback, 0, k * candidateBytes);
            device.queue.submit([encoder.finish()]);
            await readback.mapAsync(GPUMapMode.READ);
            const words = new Uint32Array(readback.getMappedRange().slice(0));
            readback.unmap();
            const floats = new Float32Array(words.buffer);
            const nearest = [];
            for (let j = 0; j < k; j++) if (words[j * 8] === 1) nearest.push({index: words[j * 8 + 2], d2: floats[j * 8 + 5]});
            return {ms: performance.now() - start, best: nearest[0], returned: nearest.length};
          };
          const first = await pick();
          const times = [];
          let best;
          for (let k = 0; k < 7; k++) { const r = await pick(); times.push(r.ms); best = r.best; }
          times.sort((a, b) => a - b);
          for (const b of [pool, fillParams, params, candidates, result, readback]) b.destroy();
          return {points: c.points, allInside, firstMs: first.ms, medianMs: times[3], hoverPerSecond: 1000 / times[3], best, returned: first.returned};
        }, {c, allInside, candidateBytes: inputs.candidate_bytes, k: inputs.k});
        console.log(JSON.stringify(row));
        report.rows.push(row);
        fs.writeFileSync(output, JSON.stringify(report, null, 2));
      }
    }
  } finally {
    await browser.close();
    await new Promise(resolve => server.close(resolve));
    fs.writeFileSync(output, JSON.stringify(report, null, 2));
  }
})().catch(error => {console.error(error); process.exitCode = 1;});
