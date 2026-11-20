// Actual renderer descriptors are exported by the Rust compilation coverage test.
// Requires Playwright on Node's module search path; see ci/README.md.
// node ci/shader_compile.cjs manifest.json report.json
// node ci/shader_compile.cjs --pick-variants manifest.json pick_variants.csv [--update] [--missing]
//   Compiles every row of the exact pick variant table (#363); see
//   ci/shader-compilation.md.
const fs = require('node:fs');
const http = require('node:http');
const {chromium} = require('playwright');

if (process.argv[2] === '--pick-variants') {
  // After the module body, which defines the table helpers below.
  setImmediate(() => pickVariants(process.argv.slice(3)).catch(error => {console.error(error); process.exitCode = 1;}));
} else (async () => {
  const [input, output] = process.argv.slice(2);
  if (!input || !output) throw Error('Usage: node ci/shader_compile.cjs manifest.json report.json');
  const manifest = JSON.parse(fs.readFileSync(input, 'utf8'));
  if (manifest.schema_version !== 1 || !manifest.pipelines.length) throw Error('Invalid/empty compilation capture');
  const limit = Number(process.env.FIGGY_SHADER_COMPILE_MAX_MS || 30000);
  const timeout = Number(process.env.FIGGY_SHADER_COMPILE_TIMEOUT_MS || 90000);
  if (!(limit > 0 && Number.isFinite(limit) && timeout >= limit)) throw Error('Invalid compilation time limits');
  const report = {schema_version: 1, browser: null, adapter: null, limit_ms: limit, measurements: [], errors: []};
  const server = http.createServer((_, res) => {res.writeHead(200, {'Content-Type': 'text/html'}); res.end('<!doctype html><title>figgy shader compilation</title>');});
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  let browser;
  const record = row => {
    report.measurements.push(row);
    console.log(JSON.stringify(row));
    fs.writeFileSync(output, JSON.stringify(report, null, 2));
  };
  try {
    // Driver selection is explicit. This script never forces SwiftShader on a
    // hardware machine. GPU-less CI may supply its existing browser wrapper.
    browser = await chromium.launch({headless: true, executablePath: process.env.BROWSER_EXECUTABLE_PATH || undefined, args: ['--enable-unsafe-webgpu', ...(process.env.BROWSER_ARGS || '').split(/\s+/).filter(Boolean)]});
    report.browser = browser.version();
    const page = await browser.newPage();
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.exposeFunction('recordCompile', record);
    report.adapter = await page.evaluate(async () => {
      const adapter = await navigator.gpu?.requestAdapter();
      if (!adapter) throw Error('Required WebGPU adapter unavailable');
      window.device = await adapter.requestDevice({requiredLimits:{maxStorageBuffersPerShaderStage:adapter.limits.maxStorageBuffersPerShaderStage}});
      window.device.lost.then(info => { window.gpuLost = `${info.reason}: ${info.message}`; });
      window.modules = [];
      window.kept = [];
      return {vendor:adapter.info.vendor, architecture:adapter.info.architecture, device:adapter.info.device, description:adapter.info.description};
    });
    async function bounded(task, label) {
      let timer;
      try {return await Promise.race([task, new Promise((_, reject) => {timer = setTimeout(() => reject(Error(`Compilation timed out: ${label}`)), timeout);})]);}
      finally {clearTimeout(timer);}
    }
    for (let index = 0; index < manifest.sources.length; index++) {
      await bounded(page.evaluate(async ({source, index}) => {
        const start = performance.now();
        const module = device.createShaderModule(source);
        const info = await module.getCompilationInfo();
        const errors = info.messages.filter(m => m.type === 'error');
        if (errors.length) throw Error(errors.map(m => m.message).join('\n'));
        modules[index] = module;
        await recordCompile({kind:'module', source:index, label:source.label, ms:performance.now()-start});
      }, {source:manifest.sources[index], index}), manifest.sources[index].label);
    }
    for (let index = 0; index < manifest.pipelines.length; index++) {
      console.log(JSON.stringify({kind:'start',pipeline:index,label:manifest.pipelines[index].descriptor.label}));
      await bounded(page.evaluate(async ({pipeline, index}) => {
        const desc = structuredClone(pipeline.descriptor);
        if (desc.layout !== 'auto') desc.layout = device.createPipelineLayout({bindGroupLayouts:desc.layout.map(entries => entries === null ? null : device.createBindGroupLayout({entries}))});
        for (const key of ['compute','vertex','fragment']) if (desc[key]) desc[key].module = modules[desc[key].module];
        const create = pipeline.kind === 'compute' ? device.createComputePipelineAsync.bind(device) : device.createRenderPipelineAsync.bind(device);
        let start = performance.now();
        kept.push(await create(desc));
        await recordCompile({kind:pipeline.kind,pipeline:index,label:desc.label,ms:performance.now()-start});
        start = performance.now();
        kept.push(await create(desc));
        await recordCompile({kind:'repeat',pipeline:index,label:desc.label,ms:performance.now()-start});
        if (window.gpuLost) throw Error(window.gpuLost);
      }, {pipeline:manifest.pipelines[index], index}), manifest.pipelines[index].descriptor.label);
    }
    for (const row of report.measurements) if (row.ms > limit) report.errors.push(`Compilation exceeded ${limit}ms: ${row.label} (${row.ms.toFixed(1)}ms)`);
    if (report.measurements.filter(r => r.kind === 'compute' || r.kind === 'render').length !== manifest.pipelines.length) throw Error('Incomplete pipeline coverage');
  } catch (error) { report.errors.push(String(error)); }
  finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
    fs.writeFileSync(output, JSON.stringify(report, null, 2));
  }
  console.log(JSON.stringify({kind:'summary',modules:manifest.sources.length,pipelines:manifest.pipelines.length,errors:report.errors}));
  if (report.errors.length) process.exitCode = 1;
})().catch(error => {console.error(error);process.exitCode = 1;});

// ---- Exact pick variant table (#363) ----
// Reads crates/renderer/tests/support/pick_variants.csv, finds each row's
// assembled exact module in the capture by its hash (the table's
// `module_hash` lines), and compiles the row's `pick_exact_candidates`
// pipeline with createComputePipelineAsync, the capture's explicit layout and
// the row's override constants, on a fresh device per row. Fails a row over
// its budget. --update writes the measured times and budgets back (budget =
// max(3 x measured, 2000 ms), rounded up to 100 ms); --missing measures only
// the rows without a time. FIGGY_SHADER_COMPILE_TIMEOUT_MS bounds one row.

// 64-bit FNV-1a over UTF-8 bytes, as the Rust table test and
// ci/test_pick_variants.py hash.
function fnv1a(text) {
  let hash = 0xcbf29ce484222325n;
  for (const byte of Buffer.from(text, 'utf8')) {
    hash ^= BigInt(byte);
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return hash.toString(16).padStart(16, '0');
}

const PICK_COLUMNS = 'module,set,bits,bits_names,shape,constants,families,example,cases,measured_ms,budget_ms';

function readPickTable(path) {
  const lines = fs.readFileSync(path, 'utf8').split('\n');
  const modules = new Map();
  const rows = [];
  lines.forEach((line, index) => {
    const hash = /^# module_hash=([^=]+)=([0-9a-f]{16})$/.exec(line);
    if (hash) modules.set(hash[1], hash[2]);
    if (line.startsWith('#') || line === PICK_COLUMNS || line === '') return;
    const fields = line.split(',');
    if (fields.length !== 11) throw Error(`${path}:${index + 1}: expected 11 fields`);
    rows.push({index, fields});
  });
  if (!lines.includes(PICK_COLUMNS)) throw Error(`${path}: no column header`);
  return {lines, modules, rows};
}

function budgetFor(ms) {
  return Math.ceil(Math.max(3 * ms, 2000) / 100) * 100;
}

async function pickVariants(args) {
  const update = args.includes('--update');
  const missingOnly = args.includes('--missing');
  const [manifestPath, tablePath] = args.filter(arg => !arg.startsWith('--'));
  if (!manifestPath || !tablePath) throw Error('Usage: node ci/shader_compile.cjs --pick-variants manifest.json pick_variants.csv [--update] [--missing]');
  const timeout = Number(process.env.FIGGY_SHADER_COMPILE_TIMEOUT_MS || 90000);
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  const table = readPickTable(tablePath);
  const byHash = new Map(manifest.sources.map(source => [fnv1a(source.code), source]));
  const sources = {};
  for (const [name, hash] of table.modules) {
    const source = byHash.get(hash);
    if (!source) throw Error(`The capture has no source for ${name} (${hash}): re-run the native capture (ci/README.md) on the shaders the table was generated from`);
    sources[name] = source.code;
  }
  const exact = manifest.pipelines.find(p => p.descriptor.label === 'figgy GPU pick exact candidate pipeline');
  if (!exact) throw Error('The capture has no exact pick pipeline');
  const layout = exact.descriptor.layout;
  const server = http.createServer((_, res) => {res.writeHead(200, {'Content-Type': 'text/html'}); res.end('<!doctype html><title>figgy pick variants</title>');});
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const errors = [];
  const todo = table.rows.filter(row => !missingOnly || row.fields[9] === '');
  let browser;
  let page;
  async function open() {
    if (browser) await browser.close().catch(() => {});
    browser = await chromium.launch({headless: true, executablePath: process.env.BROWSER_EXECUTABLE_PATH || undefined, args: ['--enable-unsafe-webgpu', ...(process.env.BROWSER_ARGS || '').split(/\s+/).filter(Boolean)]});
    page = await browser.newPage();
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.evaluate(({sources, layout}) => { window.pickSources = sources; window.pickLayout = layout; }, {sources, layout});
  }
  const save = () => fs.writeFileSync(tablePath, table.lines.join('\n'));
  try {
    for (let n = 0; n < todo.length; n++) {
      const row = todo[n];
      const [module, set, , , , constantsText] = row.fields;
      // A fresh browser every 100 rows keeps one GPU process from growing.
      if (!page || n % 100 === 0) await open();
      const constants = Object.fromEntries(constantsText.split(';').filter(Boolean).map(pair => {
        const [name, value] = pair.split('=');
        return [name, Number(value)];
      }));
      const label = `${module}/${set} ${row.fields[2]} ${row.fields[4]}`;
      let timer;
      let ms;
      try {
        ms = await Promise.race([
          page.evaluate(async ({key, constants}) => {
            const adapter = await navigator.gpu.requestAdapter();
            if (!adapter) throw Error('Required WebGPU adapter unavailable');
            const device = await adapter.requestDevice({requiredLimits: {maxStorageBuffersPerShaderStage: adapter.limits.maxStorageBuffersPerShaderStage}});
            try {
              const module = device.createShaderModule({label: key, code: window.pickSources[key]});
              const info = await module.getCompilationInfo();
              const failed = info.messages.filter(m => m.type === 'error');
              if (failed.length) throw Error(failed.map(m => m.message).join('\n'));
              const layout = device.createPipelineLayout({bindGroupLayouts: window.pickLayout.map(entries => entries === null ? null : device.createBindGroupLayout({entries}))});
              const start = performance.now();
              await device.createComputePipelineAsync({label: 'figgy GPU pick exact candidate pipeline', layout, compute: {module, entryPoint: 'pick_exact_candidates', constants}});
              return performance.now() - start;
            } finally {
              device.destroy();
            }
          }, {key: `${module}/${set}`, constants}),
          new Promise((_, reject) => {timer = setTimeout(() => reject(Error(`timed out after ${timeout} ms`)), timeout);}),
        ]);
      } catch (error) {
        errors.push(`${label}: ${String(error).split('\n')[0]}`);
        console.log(JSON.stringify({kind: 'pick-variant', row: n + 1, of: todo.length, label, error: String(error).split('\n')[0]}));
        page = null;
        continue;
      } finally {
        clearTimeout(timer);
      }
      const measured = Math.round(ms);
      const budget = Number(row.fields[10]);
      console.log(JSON.stringify({kind: 'pick-variant', row: n + 1, of: todo.length, label, ms: measured, budget: row.fields[10] || null}));
      if (update) {
        row.fields[9] = String(measured);
        row.fields[10] = String(budgetFor(measured));
        table.lines[row.index] = row.fields.join(',');
        save();
      } else if (!(budget > 0)) {
        errors.push(`${label}: no budget (measure with --update)`);
      } else if (measured > budget) {
        errors.push(`${label}: ${measured} ms exceeds its ${budget} ms budget`);
      }
    }
  } finally {
    if (browser) await browser.close().catch(() => {});
    await new Promise(resolve => server.close(resolve));
  }
  const times = table.rows.map(row => Number(row.fields[9])).filter(ms => ms > 0).sort((a, b) => a - b);
  const at = q => times.length ? times[Math.min(times.length - 1, Math.floor(q * times.length))] : null;
  console.log(JSON.stringify({kind: 'pick-variant-summary', rows: table.rows.length, measured: times.length, median: at(0.5), p90: at(0.9), max: times.at(-1) ?? null, errors}));
  if (errors.length) process.exitCode = 1;
}
