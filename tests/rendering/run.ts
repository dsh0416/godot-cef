// Run directly with the repository's Node 26; no npm dependencies are required.
import { spawn } from "node:child_process";
import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";

const root = fileURLToPath(new URL("../../", import.meta.url));
const fixture = fileURLToPath(new URL("./", import.meta.url));
const { values } = parseArgs({ options: {
  godot: { type: "string" }, addon: { type: "string" }, output: { type: "string" },
  driver: { type: "string", default: process.platform === "win32" ? "d3d12" : "vulkan" },
  class: { type: "string", default: "CefTexture,CefTexture2D" },
  mode: { type: "string", default: "software,accelerated" },
  "thread-model": { type: "string", default: "0,2" },
  "gpu-validation": { type: "boolean", default: false },
} });
if (!values.godot) throw new Error("Pass --godot <Godot executable>; use *_console.exe on Windows");
const godot = resolve(values.godot);
const addon = resolve(values.addon ?? join(root, "addons/godot_cef"));
const classes = select(values.class, ["CefTexture", "CefTexture2D"], "class");
const modes = select(values.mode, ["software", "accelerated"], "mode");
const models = select(values["thread-model"], ["0", "2"], "thread-model");
const gpuValidationRequested = values["gpu-validation"];
if (!["d3d12", "vulkan", "metal"].includes(values.driver)) throw new Error("Unknown --driver");
const runRoot = join(resolve(values.output ?? join(root, "target/rendering")), `${Date.now()}-${process.pid}`);
const project = join(runRoot, "project");
const results: Record<string, unknown>[] = [];
await mkdir(project, { recursive: true });
await cp(addon, join(project, "addons/godot_cef"), { recursive: true });
for (const name of ["main.tscn", "main.gd"]) await cp(join(fixture, name), join(project, name));
await mkdir(join(project, "fixture"));
await cp(join(fixture, "page.html"), join(project, "fixture/page.html"));
const projectTemplate = await readFile(join(fixture, "project.godot"), "utf8");
await writeFile(join(project, "project.godot"), projectTemplate);
console.log(`Graphical evidence: ${runRoot}`);
let fatal: string | null = null;
try {
  const imported = await invoke("import", [
    "--rendering-method", "mobile", "--rendering-driver", values.driver,
    "--position", "-10000,-10000", "--editor", "--import", "--quit",
  ], {}, 60000);
  if (imported.code !== 0 || imported.timedOut || hasEngineError(imported.log)) {
    throw new Error("Godot project import failed; see import/godot.log");
  }
  for (const model of models) for (const mode of modes) for (const className of classes) {
    const name = `${values.driver}-thread${model}-${mode}-${className}`;
    const output = join(runRoot, name);
    await writeFile(join(project, "project.godot"), projectTemplate.replace("threads/thread_model=0", `threads/thread_model=${model}`));
    const processResult = await invoke(name, [
      "--rendering-method", "mobile", "--rendering-driver", values.driver,
      "--position", "-10000,-10000", "--resolution", "640x360",
    ], {
      GDCEF_RENDER_CLASS: className, GDCEF_RENDER_MODE: mode, GDCEF_RENDER_OUTPUT: output,
    }, 105000);
    const reports = processResult.log.split(/\r?\n/)
      .filter(line => line.startsWith("GDCEF_RENDER_RESULT "));
    let report: Record<string, unknown> | null = null;
    try { if (reports.length === 1) report = JSON.parse(reports[0].slice("GDCEF_RENDER_RESULT ".length)); } catch {}
    const reasons: string[] = [];
    if (processResult.timedOut) reasons.push("Godot exceeded the outer process deadline");
    if (processResult.code !== 0) reasons.push(`Godot exit code ${processResult.code}`);
    if (hasEngineError(processResult.log)) reasons.push("Godot reported a script/native/validation error");
    if (!report || report.class !== className || report.passed !== true || !(Number(report.checks) > 0)
        || !Array.isArray(report.failures) || report.failures.length !== 0) reasons.push("Missing, invalid, or unsuccessful test report");
    if (report && (report.driver !== values.driver || Number(report.thread_model) !== Number(model))) reasons.push("Godot used a different graphics driver or thread model");
    const acceleratedObserved = processResult.log.includes(`[${className}] Creating browser in accelerated rendering mode`)
      && !/falling back to software rendering/i.test(processResult.log);
    if (mode === "accelerated" && !acceleratedObserved) reasons.push("Requested accelerated OSR was not confirmed (or fell back)");
    if (mode === "software" && !processResult.log.includes(`[${className}] Creating browser in software rendering mode`)) reasons.push("Software OSR startup was not confirmed");
    const result = { name, passed: reasons.length === 0, reasons, acceleratedObserved, gpuValidationRequested, ...processResult, log: undefined, report };
    await writeFile(join(output, "assessment.json"), JSON.stringify(result, null, 2));
    results.push(result);
    console.log(`${result.passed ? "PASS" : "FAIL"} ${name}${reasons.length ? ": " + reasons.join("; ") : ""}`);
  }
} catch (error) {
  fatal = String(error);
} finally {
  const passed = fatal === null && results.length === classes.length * modes.length * models.length
    && results.every(result => result.passed);
  await writeFile(join(runRoot, "summary.json"), JSON.stringify({ passed, fatal, driver: values.driver, gpuValidationRequested, results }, null, 2));
  console.log(`Results: ${join(runRoot, "summary.json")}`);
  if (!passed) process.exitCode = 1;
  if (fatal) console.error(fatal);
}

function select(value: string, allowed: string[], name: string): string[] {
  const selected = value.split(",");
  if (!selected.length || selected.some(item => !allowed.includes(item)) || new Set(selected).size !== selected.length) {
    throw new Error(`--${name} must select unique values from ${allowed.join(",")}`);
  }
  return selected;
}

function hasEngineError(log: string): boolean {
  return /^(?:SCRIPT ERROR:|ERROR:)|panicked at|D3D12 device removed|D3D12[^\r\n]*\b(?:ERROR|CORRUPTION)\b|Validation Error:|VUID-/mi.test(log);
}

async function invoke(name: string, args: string[], env: Record<string, string>, deadline: number) {
  const output = join(runRoot, name);
  await mkdir(output, { recursive: true });
  const command = ["--path", project, ...(gpuValidationRequested ? ["--gpu-validation"] : []), ...args];
  await writeFile(join(output, "command.json"), JSON.stringify({ executable: godot, arguments: command, env }, null, 2));
  return await new Promise<{ code: number | null; timedOut: boolean; log: string }>((done, reject) => {
    const child = spawn(godot, command, {
      cwd: project, env: { ...process.env, ...env }, windowsHide: true,
      detached: process.platform !== "win32", stdio: ["ignore", "pipe", "pipe"],
    });
    let log = "";
    let timedOut = false;
    child.stdout.on("data", chunk => { log += chunk.toString(); });
    child.stderr.on("data", chunk => { log += chunk.toString(); });
    const timer = setTimeout(() => {
      timedOut = true;
      if (!child.pid) return;
      if (process.platform === "win32") {
        const killer = spawn("taskkill", ["/PID", String(child.pid), "/T", "/F"], { windowsHide: true, stdio: "ignore" });
        killer.on("error", () => child.kill("SIGKILL"));
      } else {
        try { process.kill(-child.pid, "SIGKILL"); } catch { child.kill("SIGKILL"); }
      }
    }, deadline);
    child.on("error", error => { clearTimeout(timer); reject(error); });
    child.on("close", async code => {
      clearTimeout(timer);
      try {
        await writeFile(join(output, "godot.log"), log);
        done({ code, timedOut, log });
      } catch (error) { reject(error); }
    });
  });
}
