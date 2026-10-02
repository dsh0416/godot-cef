import { spawn } from "node:child_process";
import { writeFile } from "node:fs/promises";

// A detached Unix process group contains Godot and its CEF subprocesses. On
// Windows taskkill /T also handles the official Godot console launcher's child.
async function terminateTree(child) {
  if (!child.pid) return;
  if (process.platform === "win32") {
    await new Promise((resolve) => {
      const killer = spawn("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
        stdio: "ignore", windowsHide: true,
      });
      killer.once("error", () => { child.kill(); resolve(); });
      killer.once("close", resolve);
    });
  } else {
    try { process.kill(-child.pid, "SIGKILL"); }
    catch (error) { if (error.code !== "ESRCH") throw error; }
  }
}

export async function runProcess(executable, args, { cwd, env = {}, logPath, timeoutMs }) {
  let log = "";
  let timedOut = false;
  let outputOverflow = false;
  let processError = null;
  const started = Date.now();
  const child = spawn(executable, args, {
    cwd, env: { ...process.env, ...env }, stdio: ["ignore", "pipe", "pipe"],
    detached: process.platform !== "win32", windowsHide: true,
  });
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  const append = (chunk) => {
    log += chunk.toString();
    // A runaway diagnostic must fail without exhausting the CI runner's RAM.
    if (log.length > 8 * 1024 * 1024) {
      outputOverflow = true;
      log = log.slice(-8 * 1024 * 1024);
    }
  };
  child.stdout.on("data", append);
  child.stderr.on("data", append);
  let cleanup = Promise.resolve();
  const timer = setTimeout(() => {
    timedOut = true;
    cleanup = terminateTree(child).catch((error) => { processError = String(error); child.kill(); });
  }, timeoutMs);
  // Terminate orphan helpers as soon as the engine exits, including when they
  // still hold inherited stdout open and would otherwise prevent close.
  child.once("exit", () => {
    if (process.platform !== "win32") {
      cleanup = terminateTree(child).catch((error) => { processError = String(error); });
    }
  });
  const [exitCode, signal] = await new Promise((resolve) => {
    child.once("error", (error) => { processError = String(error); });
    child.once("close", (code, reason) => resolve([code, reason]));
  });
  clearTimeout(timer);
  await cleanup;
  await writeFile(logPath, log);
  return { exitCode, signal, timedOut, processError, outputOverflow, durationMs: Date.now() - started, log };
}

export function assessResult(outcome, expected = null) {
  const failures = [];
  if (outcome.exitCode !== 0) failures.push(`Process exit code: ${outcome.exitCode}; signal: ${outcome.signal}`);
  if (outcome.timedOut) failures.push("Outer process deadline exceeded");
  if (outcome.processError) failures.push(outcome.processError);
  if (outcome.outputOverflow) failures.push("Process exceeded the log size limit");
  if (/^ERROR:|SCRIPT ERROR:|Parse Error:|panicked at|thread.*panicked/im.test(outcome.log)) {
    failures.push("Godot error, extension loading error, or Rust panic diagnostic in log");
  }
  const reports = outcome.log.split(/\r?\n/).filter((line) => line.startsWith("GDCEF_ITEST_RESULT "));
  let report = null;
  if (expected) {
    if (reports.length !== 1) failures.push(`Expected exactly one test report, received ${reports.length}`);
    if (reports.length === 1) {
      try { report = JSON.parse(reports[0].slice("GDCEF_ITEST_RESULT ".length)); }
      catch { failures.push("Malformed test report JSON"); }
    }
    if (!report || report.class !== expected.class || report.case !== expected.case
      || report.passed !== true || !Number.isInteger(report.checks) || report.checks <= 0
      || !Array.isArray(report.failures) || report.failures.length !== 0) {
      failures.push("Missing, mismatched, or failed test report");
    }
  } else if (reports.length) {
    failures.push("Test addon ran without explicit opt-in during startup smoke");
  }
  return { passed: failures.length === 0, failures, report };
}

function xml(value) {
  return String(value).replaceAll(/[^\x09\x0a\x0d\x20-\uFFFF]/g, "")
    .replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;").replaceAll("'", "&apos;");
}

export function junit(results) {
  const failures = results.filter((result) => !result.passed).length;
  return `<?xml version="1.0" encoding="UTF-8"?>\n<testsuite name="Godot CEF integration" tests="${results.length}" failures="${failures}">\n`
    + results.map((result) => `  <testcase classname="${xml(result.class)}" name="${xml(result.case)}" time="${(result.durationMs ?? 0) / 1000}">`
      + (result.passed ? "" : `<failure message="${xml(result.failures?.[0] ?? "Integration failure")}">${xml(JSON.stringify(result, null, 2))}</failure>`)
      + `</testcase>`).join("\n") + "\n</testsuite>\n";
}
