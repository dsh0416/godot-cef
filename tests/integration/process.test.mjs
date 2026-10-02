import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { assessResult, junit, runProcess } from "./process.mjs";

const expected = { class: "CefTexture", case: "js_ipc" };
const report = { ...expected, passed: true, checks: 3, failures: [] };
const line = `GDCEF_ITEST_RESULT ${JSON.stringify(report)}\n`;
const success = { exitCode: 0, signal: null, timedOut: false, log: line };

test("success requires both a matching report and a clean process exit", () => {
  assert.equal(assessResult(success, expected).passed, true);
  for (const outcome of [
    { ...success, exitCode: 1 }, { ...success, exitCode: null, signal: "SIGSEGV" },
    { ...success, timedOut: true }, { ...success, outputOverflow: true },
    { ...success, log: "" }, { ...success, log: line + line },
    { ...success, log: "GDCEF_ITEST_RESULT invalid\n" },
    { ...success, log: line + "SCRIPT ERROR: example\n" },
    { ...success, log: line + "ERROR: [CefTexture2D] Failed to connect render frame\n" },
    { ...success, log: line + "thread 'main' panicked at foo.rs\n" },
  ]) assert.equal(assessResult(outcome, expected).passed, false);
  assert.equal(assessResult(success, { ...expected, class: "CefTexture2D" }).passed, false);
  assert.equal(assessResult(success).passed, false, "startup must not silently run tests");
  for (const patch of [{ passed: false }, { checks: 0 }, { failures: ["failure"] }, { failures: null }]) {
    const log = `GDCEF_ITEST_RESULT ${JSON.stringify({ ...report, ...patch })}\n`;
    assert.equal(assessResult({ ...success, log }, expected).passed, false);
  }
});

test("JUnit preserves failure evidence and escapes diagnostic text", () => {
  const xml = junit([{ ...expected, passed: false, durationMs: 1250, failures: ['<error a="b"> & failure'] }]);
  assert.match(xml, /tests="1" failures="1"/);
  assert.match(xml, /time="1.25"/);
  assert.match(xml, /&lt;error a=&quot;b&quot;&gt; &amp; failure/);
});

test("a process that reports success and then crashes still fails", async () => {
  const dir = await mkdtemp(join(tmpdir(), "gdcef-itest-crash-"));
  try {
    const outcome = await runProcess(process.execPath, ["-e", `console.log(${JSON.stringify(line.trim())}); process.exit(7);`], {
      cwd: dir, logPath: join(dir, "process.log"), timeoutMs: 5000,
    });
    assert.equal(outcome.exitCode, 7);
    assert.equal(assessResult(outcome, expected).passed, false);
    assert.match(await readFile(join(dir, "process.log"), "utf8"), /GDCEF_ITEST_RESULT/);
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test("outer deadline kills a hung process tree and records its partial output", async () => {
  const dir = await mkdtemp(join(tmpdir(), "gdcef-itest-timeout-"));
  try {
    const heartbeat = join(dir, "heartbeat");
    const descendant = `const fs = require('node:fs'); setInterval(() => fs.writeFileSync(${JSON.stringify(heartbeat)}, String(Date.now())), 25);`;
    const parent = `require('node:child_process').spawn(process.execPath, ['-e', ${JSON.stringify(descendant)}], { stdio: 'inherit' }); console.log('waiting'); setInterval(() => {}, 1000);`;
    const outcome = await runProcess(process.execPath, ["-e", parent], {
      cwd: dir, logPath: join(dir, "process.log"), timeoutMs: 1000,
    });
    assert.equal(outcome.timedOut, true);
    assert.equal(assessResult(outcome, expected).passed, false);
    assert.match(await readFile(join(dir, "process.log"), "utf8"), /waiting/);
    const finalHeartbeat = await readFile(heartbeat, "utf8");
    await new Promise((resolve) => setTimeout(resolve, 200));
    assert.equal(await readFile(heartbeat, "utf8"), finalHeartbeat, "CEF-like child must not survive timeout");
  } finally { await rm(dir, { recursive: true, force: true }); }
});
