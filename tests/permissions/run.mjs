import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { access, copyFile, cp, mkdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { startFixtureServer } from "./server.mjs";

const fixture = dirname(fileURLToPath(import.meta.url));
const root = resolve(fixture, "../..");
const options = new Map();
for (let i = 2; i < process.argv.length; i += 2) {
  const flag = process.argv[i];
  const value = process.argv[i + 1];
  if (!["--godot", "--addon", "--output", "--case"].includes(flag) || !value) {
    throw new Error("Usage: node tests/permissions/run.mjs --godot <executable> [--addon <addon-directory>] [--output <directory>] [--case CefTexture:grant_all]");
  }
  options.set(flag, value);
}
if (!options.has("--godot")) throw new Error("--godot must name a Godot 4.5+ executable");

const godot = resolve(options.get("--godot"));
const addon = resolve(options.get("--addon") ?? join(root, "addons/godot_cef"));
const output = resolve(options.get("--output") ?? join(root, "target/permission-integration"));
const runRoot = join(output, new Date().toISOString().replaceAll(/[:.]/g, "-") + "-" + randomUUID().slice(0, 8));
const project = join(runRoot, "project");
const scenarios = ["grant_all", "deny_one", "timeout", "navigation", "unhandled_then_listen"];
const cases = ["CefTexture", "CefTexture2D"].flatMap((name) => scenarios.map((scenario) => [name, scenario]));
const selection = options.has("--case") ? cases.filter((parts) => parts.join(":") === options.get("--case")) : cases;
if (!selection.length) throw new Error(`--case must match CefTexture or CefTexture2D and one of: ${scenarios.join(", ")}`);

await access(godot);
await access(join(addon, "godot_cef.gdextension"));
await access(join(addon, "bin"));
await mkdir(project, { recursive: true });
for (const name of ["project.godot", "main.tscn", "main.gd"]) {
  await copyFile(join(fixture, name), join(project, name));
}
await cp(addon, join(project, "addons/godot_cef"), { recursive: true });

async function runGodot(args, env, logPath, timeoutMs) {
  let log = "";
  let timedOut = false;
  const child = spawn(godot, args, {
    cwd: project,
    env: { ...process.env, ...env },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  child.stdout.on("data", (chunk) => { log += chunk.toString(); });
  child.stderr.on("data", (chunk) => { log += chunk.toString(); });
  const timer = setTimeout(() => {
    timedOut = true;
    if (process.platform === "win32") {
      // The Godot console launcher can have an engine child. Kill only the
      // process tree this invocation started, including its CEF helpers.
      const cleanup = spawn("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
        stdio: "ignore", windowsHide: true,
      });
      cleanup.once("error", () => child.kill());
    } else {
      child.kill();
    }
  }, timeoutMs);
  let exitCode;
  let signal;
  try {
    [exitCode, signal] = await new Promise((resolveExit, reject) => {
      child.once("error", reject);
      child.once("close", (code, reason) => resolveExit([code, reason]));
    });
  } finally {
    clearTimeout(timer);
    await writeFile(logPath, log);
  }
  return { exitCode, signal, timedOut, log };
}

const server = await startFixtureServer();
const results = [];
try {
  const imported = await runGodot(
    ["--headless", "--editor", "--import", "--path", project, "--quit"],
    {}, join(runRoot, "import.log"), 60000,
  );
  if (imported.exitCode !== 0 || imported.timedOut || /SCRIPT ERROR:|Parse Error:|ERROR:.*GDExtension/i.test(imported.log)) {
    throw new Error(`Godot import failed; see ${join(runRoot, "import.log")}`);
  }
  // The real rendering loop drives CefTexture2D.frame_pre_draw. Only CEF OSR
  // uses software rendering; a headless/dummy Godot renderer is not exercised.
  for (const [name, scenario] of selection) {
    const id = `${name}-${scenario}-${randomUUID()}`;
    const caseDir = join(runRoot, `${name}-${scenario}`);
    await mkdir(caseDir);
    server.register(id);
    const outcome = await runGodot(
      ["--path", project, "--rendering-method", "gl_compatibility", "--audio-driver", "Dummy"],
      {
        PERMISSION_TEST_CLASS: name,
        PERMISSION_TEST_CASE: scenario,
        PERMISSION_TEST_ORIGIN: server.origin,
        PERMISSION_TEST_RUN: id,
        PERMISSION_TEST_PROFILE: join(caseDir, "cef-profile"),
      },
      join(caseDir, "godot.log"), 35000,
    );
    const line = outcome.log.split(/\r?\n/).find((entry) => entry.startsWith("PERMISSION_RESULT "));
    const report = line ? JSON.parse(line.slice("PERMISSION_RESULT ".length)) : null;
    const passed = outcome.exitCode === 0 && !outcome.timedOut && report?.passed === true
      && !/SCRIPT ERROR:|Parse Error:/i.test(outcome.log);
    const result = {
      class: name, case: scenario, passed, exitCode: outcome.exitCode,
      signal: outcome.signal, timedOut: outcome.timedOut,
      report, browserEvents: server.events(id), log: join(caseDir, "godot.log"),
    };
    results.push(result);
    await writeFile(join(caseDir, "result.json"), JSON.stringify(result, null, 2));
    console.log(`${passed ? "PASS" : "FAIL"} ${name}:${scenario}${report ? ` (${report.checks} checks)` : " (no result)"}`);
  }
} finally {
  await server.close();
  const summary = {
    godot, addon, origin: server.origin, project,
    fakeMediaDevices: true, fakeUi: false, cefSoftwareRendering: true,
    requestedCases: selection.length, results,
    passed: results.length === selection.length && results.every((result) => result.passed),
  };
  await writeFile(join(runRoot, "summary.json"), JSON.stringify(summary, null, 2));
  console.log(`Results: ${join(runRoot, "summary.json")}`);
}
process.exitCode = results.length === selection.length && results.every((result) => result.passed) ? 0 : 1;
