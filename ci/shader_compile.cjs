// Actual renderer descriptors are exported by the Rust compilation coverage test.
// Requires Playwright on Node's module search path; see ci/README.md.
// node ci/shader_compile.cjs manifest.json report.json
const fs = require('node:fs');
const http = require('node:http');
const {chromium} = require('playwright');

(async () => {
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
    browser = await chromium.launch({headless: true, executablePath: process.env.BROWSER_EXECUTABLE_PATH || undefined, args: ['--enable-unsafe-webgpu']});
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
