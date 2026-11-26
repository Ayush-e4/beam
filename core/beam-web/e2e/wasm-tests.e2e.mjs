// Runs a wasm-bindgen test module in headless Chromium: wasm-bindgen-test-runner serves the
// tests (some need a dedicated worker, for OPFS) and this script opens its page and reports
// what it prints. Usage (tools/web-engine-test.sh runs it):
//   node wasm-tests.e2e.mjs <test .wasm built for wasm32-unknown-unknown>
// CHROMIUM_PATH picks a local Chromium instead of the one Playwright installed.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { chromium } from 'playwright-core';

const [wasm] = process.argv.slice(2);
const ADDRESS = '127.0.0.1:18765';
const READY_MS = 20_000;
const RUN_MS = 180_000;

function withTimeout(promise, ms, what) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`timed out: ${what}`)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

function startRunner() {
  const runner = spawn('wasm-bindgen-test-runner', [wasm], {
    env: { ...process.env, NO_HEADLESS: '1', WASM_BINDGEN_TEST_ADDRESS: ADDRESS },
    stdio: ['ignore', 'pipe', 'inherit'],
  });
  const ready = new Promise((resolve, reject) => {
    createInterface({ input: runner.stdout }).on('line', (line) => {
      if (line.includes('are now available at')) resolve();
    });
    runner.on('exit', (code) => reject(new Error(`test runner exited (${code})`)));
  });
  return { runner, ready: withTimeout(ready, READY_MS, 'test runner start') };
}

const { runner, ready } = startRunner();
let browser;
try {
  await ready;
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined });
  const page = await browser.newPage();
  page.on('pageerror', (error) => console.error(`page error: ${error.message}`));
  await page.goto(`http://${ADDRESS}/`);
  const output = page.locator('#output');
  await output.filter({ hasText: /test result: (ok|FAILED)/ }).waitFor({ timeout: RUN_MS });
  const text = await output.innerText();
  console.log(text);
  if (!/test result: ok/.test(text)) process.exitCode = 1;
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
} finally {
  await browser?.close();
  runner.kill();
}
