import { randomUUID } from "node:crypto";
import { access, copyFile, cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { startFixtureServer } from "./server.mjs";
import { assessResult, junit, runProcess } from "./process.mjs";

const fixture = dirname(fileURLToPath(import.meta.url));
const root = resolve(fixture, "../..");
const options = new Map();
for (let i = 2; i < process.argv.length; i += 2) {
  const flag = process.argv[i];
  const value = process.argv[i + 1];
  if (!["--godot", "--addon", "--test-addon", "--output", "--case"].includes(flag) || !value || options.has(flag)) {
    throw new Error("Usage: node tests/integration/run.mjs --godot <executable> --test-addon <library> [--addon <directory>] [--output <directory>] [--case CefTexture:js_ipc]");
  }
  options.set(flag, value);
}
if (!options.has("--godot") || !options.has("--test-addon")) throw new Error("--godot and --test-addon are required");
const godot = resolve(options.get("--godot"));
const addon = resolve(options.get("--addon") ?? join(root, "addons/godot_cef"));
const testAddon = resolve(options.get("--test-addon"));
const output = resolve(options.get("--output") ?? join(root, "target/integration"));
const runRoot = join(output, new Date().toISOString().replaceAll(/[:.]/g, "-") + "-" + randomUUID().slice(0, 8));
const project = join(runRoot, "project");
const scenarios = ["permission_grant_all", "permission_deny_one", "permission_timeout", "permission_navigation", "permission_unhandled_then_listen", "js_ipc", "lifecycle"];
const cases = ["CefTexture", "CefTexture2D"].flatMap((name) => scenarios.map((scenario) => ({ class: name, case: scenario })));
const selection = options.has("--case") ? cases.filter((item) => `${item.class}:${item.case}` === options.get("--case")) : cases;
if (!selection.length) throw new Error(`Unknown --case. Cases: ${cases.map((item) => `${item.class}:${item.case}`).join(", ")}`);
const platform = { linux: "linux", win32: "windows" }[process.platform];
const arch = { x64: "x86_64", arm64: "arm64" }[process.arch];
if (!platform || !arch) throw new Error("This integration runner supports Linux and Windows x64/ARM64 hosts");

await mkdir(runRoot, { recursive: true });
const results = [];
let server;
let fatalError = null;
try {
  await access(godot);
  await access(testAddon);
  await access(join(addon, "godot_cef.gdextension"));
  await access(join(addon, "bin"));
  await mkdir(project);
  for (const name of ["project.godot", "main.tscn"]) await copyFile(join(fixture, name), join(project, name));
  await cp(addon, join(project, "addons/godot_cef"), { recursive: true });
  const testDir = join(project, "addons/gdcef_itest");
  await mkdir(join(testDir, "bin"), { recursive: true });
  const library = platform === "windows" ? "gdcef_itest.dll" : "libgdcef_itest.so";
  await copyFile(testAddon, join(testDir, "bin", library));
  const descriptor = (await readFile(join(fixture, "gdcef_itest.gdextension.in"), "utf8"))
    .replace("@FEATURE@", `${platform}.${arch}`).replace("@LIBRARY@", library);
  await writeFile(join(testDir, "gdcef_itest.gdextension"), descriptor);

  const invoke = async (name, args, env, expected = null) => {
    const caseDir = join(runRoot, name);
    await mkdir(caseDir);
    const logPath = join(caseDir, "godot.log");
    const outcome = await runProcess(godot, args, { cwd: project, env, logPath, timeoutMs: 60000 });
    const { log, ...processResult } = outcome;
    const result = {
      class: expected?.class ?? "harness", case: expected?.case ?? name,
      ...processResult, ...assessResult(outcome, expected),
      browserEvents: expected ? server.events(env.GDCEF_ITEST_RUN) : [], log: logPath,
    };
    results.push(result);
    await writeFile(join(caseDir, "result.json"), JSON.stringify(result, null, 2));
    console.log(`${result.passed ? "PASS" : "FAIL"} ${result.class}:${result.case}${result.report ? ` (${result.report.checks} checks)` : ""}`);
    if (!result.passed) console.error(result.failures.join("\n"));
    return result;
  };
  // Explicitly unset opt-in even if the caller's environment enables tests.
  const disabled = { GDCEF_ITEST: "0" };
  const imported = await invoke("import", ["--headless", "--editor", "--import", "--path", project, "--quit"], disabled);
  if (!imported.passed) throw new Error("Godot import failed");
  const startup = await invoke("startup", ["--headless", "--path", project, "--quit-after", "3"], disabled);
  if (!startup.passed) throw new Error("Extension-only startup/exit smoke failed");
  server = await startFixtureServer();
  for (const item of selection) {
    const name = `${item.class}-${item.case}`;
    const id = `${name}-${randomUUID()}`;
    server.register(id);
    await invoke(name, ["--headless", "--path", project], {
      GDCEF_ITEST: "1", GDCEF_ITEST_CLASS: item.class, GDCEF_ITEST_CASE: item.case,
      GDCEF_ITEST_ORIGIN: server.origin, GDCEF_ITEST_RUN: id,
      GDCEF_ITEST_PROFILE: join(runRoot, name, "cef-profile"),
    }, item);
  }
} catch (error) {
  fatalError = String(error.stack ?? error);
  console.error(fatalError);
  results.push({ class: "harness", case: "setup", passed: false, failures: [fatalError] });
} finally {
  if (server) await server.close();
  const completedCases = results.filter((result) => result.class !== "harness").length;
  const summary = {
    godot, addon, testAddon, origin: server?.origin ?? null, project,
    godotHeadless: true, cefSoftwareRendering: true, fakeMediaDevices: true, fakeUi: false,
    requestedCases: selection.length, completedCases, fatalError, results,
    passed: !fatalError && completedCases === selection.length && results.every((result) => result.passed),
  };
  await writeFile(join(runRoot, "summary.json"), JSON.stringify(summary, null, 2));
  await writeFile(join(runRoot, "junit.xml"), junit(results));
  console.log(`Results: ${join(runRoot, "summary.json")}`);
  process.exitCode = summary.passed ? 0 : 1;
}
