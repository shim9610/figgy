// Run actual wasm-bindgen browser tests with Playwright, without ChromeDriver.
// Input: cargo test --no-run --message-format=json output for selected WASM tests.
const {chromium} = require('playwright');
const {spawn} = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');

(async () => {
  const [input, output = 'target/ci-results/wasm-browser.json'] = process.argv.slice(2);
  if (!input) throw Error('Usage: node ci/wasm_browser_test.cjs cargo-build.jsonl [report.json]');
  const artifacts = fs.readFileSync(input, 'utf8').trim().split('\n').map(JSON.parse)
    .filter(v => v.reason === 'compiler-artifact' && v.profile.test && v.executable?.endsWith('.wasm'));
  assert.ok(artifacts.length > 0, 'No WASM test binaries in Cargo output');
  const results = [];
  fs.mkdirSync(path.dirname(output), {recursive: true});
  for (const artifact of artifacts) {
    let browser;
    let runnerOutput = '';
    const runner = spawn(process.env.WASM_BINDGEN_TEST_RUNNER || 'wasm-bindgen-test-runner', [artifact.executable], {
      env: {...process.env, NO_HEADLESS: '1', WASM_BINDGEN_TEST_ADDRESS: '127.0.0.1:0', WASM_BINDGEN_TEST_TIMEOUT: '180'},
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    const exited = new Promise(resolve => runner.once('exit', resolve));
    try {
      const url = await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(Error('WASM test server startup timed out')), 60000);
        const consume = bytes => {
          runnerOutput += bytes.toString();
          const match = runnerOutput.match(/Interactive browsers tests are now available at (http:\/\/[^\s]+)/);
          if (match) { clearTimeout(timer); resolve(match[1]); }
        };
        runner.stdout.on('data', consume);
        runner.stderr.on('data', consume);
        runner.once('error', error => { clearTimeout(timer); reject(error); });
        runner.once('exit', code => { clearTimeout(timer); reject(Error(`Test runner exited ${code}: ${runnerOutput}`)); });
      });
      browser = await chromium.launch({headless: true, executablePath: process.env.BROWSER_EXECUTABLE_PATH || undefined, args: ['--enable-unsafe-webgpu']});
      const page = await browser.newPage();
      const errors = [];
      page.on('pageerror', error => errors.push(error.message));
      await page.goto(url);
      await page.waitForFunction(() => /test result: (ok|FAILED)/.test(document.querySelector('#output')?.textContent || ''), {}, {timeout: 240000});
      const text = await page.locator('#output').innerText();
      console.log(text);
      assert.deepEqual(errors, []);
      const match = text.match(/test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored/);
      assert.ok(match, 'Browser test failed');
      assert.ok(+match[1] > 0);
      assert.equal(+match[2], 0);
      assert.equal(+match[3], 0);
      results.push({test: artifact.target.name, browser: browser.version(), passed: +match[1], failed: +match[2], ignored: +match[3], output: text});
    } catch (error) {
      results.push({test: artifact.target.name, error: String(error), runner_output: runnerOutput});
      throw error;
    } finally {
      await browser?.close();
      if (runner.exitCode === null && runner.pid) { runner.kill('SIGTERM'); await exited; }
      fs.writeFileSync(output, JSON.stringify(results, null, 2) + '\n');
    }
  }
})().catch(error => {console.error(error); process.exitCode = 1;});
