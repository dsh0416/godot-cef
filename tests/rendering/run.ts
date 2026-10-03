// Run directly with the repository's Node 26; no npm dependencies are required.
import { spawn } from "node:child_process";
import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
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
  scenario: { type: "string", default: "lifecycle" },
} });
if (!values.godot) throw new Error("Pass --godot <Godot executable>; use *_console.exe on Windows");
const godot = resolve(values.godot);
const addon = resolve(values.addon ?? join(root, "addons/godot_cef"));
const classes = select(values.class, ["CefTexture", "CefTexture2D"], "class");
const modes = select(values.mode, ["software", "accelerated"], "mode");
const models = select(values["thread-model"], ["0", "2"], "thread-model");
const gpuValidationRequested = values["gpu-validation"];
if (!["lifecycle", "pacing", "popup"].includes(values.scenario)) throw new Error("--scenario must be lifecycle, pacing, or popup");
const pacing = values.scenario === "pacing";
const popup = values.scenario === "popup";
if (!["d3d12", "vulkan", "metal"].includes(values.driver)) throw new Error("Unknown --driver");
const runRoot = join(resolve(values.output ?? join(root, "target/rendering")), `${Date.now()}-${process.pid}`);
const project = join(runRoot, "project");
const results: Record<string, unknown>[] = [];
const browserPacing = new Map<string, Record<string, unknown>[]>();
const page = await readFile(join(fixture, "page.html"));
const server = pacing || popup ? createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", "http://127.0.0.1");
  if (popup && request.method === "GET" && url.pathname === "/redirect") {
    response.writeHead(302, { Location: `${fixtureOrigin}/popup.html${url.search}`, "Cache-Control": "no-store" });
    response.end();
    return;
  }
  if (popup && request.method === "GET" && url.pathname === "/popup.html") {
    response.writeHead(200, { "Content-Type": "text/html", "Cache-Control": "no-store" });
    response.end(await readFile(join(fixture, "popup.html")));
    return;
  }
  if (request.method === "GET" && url.pathname === "/page.html") {
    response.writeHead(200, { "Content-Type": "text/html", "Cache-Control": "no-store" });
    response.end(page);
    return;
  }
  if (request.method === "GET" && url.pathname === "/pacing") {
    const reports = browserPacing.get(url.searchParams.get("run") ?? "") ?? [];
    response.writeHead(reports.length ? 200 : 204, { "Content-Type": "application/json", "Cache-Control": "no-store" });
    response.end(reports.length ? JSON.stringify(reports) : undefined);
    return;
  }
  if (request.method === "POST" && url.pathname === "/pacing") {
    try {
      let body = "";
      for await (const chunk of request) {
        body += chunk.toString();
        if (body.length > 65536) throw new Error("Oversized pacing report");
      }
      const key = url.searchParams.get("run") ?? "";
      const reports = browserPacing.get(key) ?? [];
      reports.push(JSON.parse(body));
      browserPacing.set(key, reports);
      response.writeHead(204);
      response.end();
    } catch {
      response.writeHead(400);
      response.end();
    }
    return;
  }
  response.writeHead(404);
  response.end();
}) : null;
if (server) await new Promise<void>((resolve, reject) => {
  server.once("error", reject);
  server.listen(0, "127.0.0.1", resolve);
});
server?.unref();
const address = server?.address();
const fixtureOrigin = address && typeof address !== "string" ? `http://127.0.0.1:${address.port}` : "";
await mkdir(project, { recursive: true });
await cp(addon, join(project, "addons/godot_cef"), { recursive: true });
for (const name of ["main.tscn", "main.gd"]) await cp(join(fixture, name), join(project, name));
if (popup) {
  await cp(join(fixture, "popup.gd"), join(project, "popup.gd"));
  const scene = await readFile(join(project, "main.tscn"), "utf8");
  await writeFile(join(project, "main.tscn"), scene.replace("res://main.gd", "res://popup.gd"));
}
await mkdir(join(project, "fixture"));
await cp(join(fixture, "page.html"), join(project, "fixture/page.html"));
const projectTemplate = (await readFile(join(fixture, "project.godot"), "utf8"))
  .replace("window/vsync/vsync_mode=0", `window/vsync/vsync_mode=${pacing ? 1 : 0}`)
  .replaceAll("=640", pacing ? "=1280" : "=640")
  .replaceAll("=360", pacing ? "=800" : "=360");
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
      "--position", "-10000,-10000", "--resolution", pacing ? "1280x800" : "640x360",
    ], {
      GDCEF_RENDER_CLASS: className, GDCEF_RENDER_MODE: mode, GDCEF_RENDER_OUTPUT: output,
      GDCEF_RENDER_SCENARIO: values.scenario,
      GDCEF_RENDER_URL: popup ? `${fixtureOrigin.replace("127.0.0.1", "localhost")}/redirect?run=${encodeURIComponent(name)}`
        : pacing ? `${fixtureOrigin}/page.html?run=${encodeURIComponent(name)}` : "res://fixture/page.html",
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
    const pacingReports = browserPacing.get(name) ?? [];
    const pacingAssessment = pacing ? assessPacing(report, pacingReports, reasons) : null;
    const result = { name, scenario: values.scenario, passed: reasons.length === 0, reasons, acceleratedObserved, gpuValidationRequested, pacing: pacingAssessment, ...processResult, log: undefined, report };
    await writeFile(join(output, "assessment.json"), JSON.stringify(result, null, 2));
    results.push(result);
    console.log(`${result.passed ? "PASS" : "FAIL"} ${name}${reasons.length ? ": " + reasons.join("; ") : ""}`);
  }
} catch (error) {
  fatal = String(error);
} finally {
  if (server) await new Promise<void>(resolve => server.close(() => resolve()));
  const passed = fatal === null && results.length === classes.length * modes.length * models.length
    && results.every(result => result.passed);
  await writeFile(join(runRoot, "summary.json"), JSON.stringify({ passed, fatal, scenario: values.scenario, driver: values.driver, gpuValidationRequested, results }, null, 2));
  console.log(`Results: ${join(runRoot, "summary.json")}`);
  if (!passed) process.exitCode = 1;
  if (fatal) console.error(fatal);
}

function assessPacing(report: Record<string, unknown> | null, browserReports: Record<string, unknown>[], reasons: string[]) {
  const engine = report?.pacing as Record<string, unknown> | undefined;
  const browser = browserReports.length === 1 ? browserReports[0] : null;
  const refresh = Number(engine?.display_refresh_hz);
  const baseline = Math.min(60, refresh > 0 ? refresh : 60);
  const drawFps = Number(engine?.fps);
  const browserFps = Number(browser?.fps);
  const tailFps = Number(browser?.tail_fps);
  const ratio = browserFps / drawFps;
  if (Number(engine?.vsync_mode) !== 1) reasons.push("Pacing scenario did not enable VSync");
  if (!(Number(engine?.elapsed_ms) >= 19900) || !(Number(browser?.elapsed_ms) >= 20000)
      || !Array.isArray(browser?.bins) || browser.bins.length < 19) reasons.push("Missing or incomplete pacing samples");
  if (!(drawFps >= baseline * 0.8)) reasons.push(`Godot draw rate ${drawFps.toFixed(2)} is below 80% of ${baseline.toFixed(2)} Hz baseline`);
  if (!(browserFps >= baseline * 0.8) || !(tailFps >= baseline * 0.8) || !(ratio >= 0.8)) {
    reasons.push(`CEF rAF pacing regressed: ${browserFps.toFixed(2)} FPS, tail ${tailFps.toFixed(2)} FPS, browser/Godot ${ratio.toFixed(3)}`);
  }
  return { baseline_hz: baseline, minimum_ratio: 0.8, browser_to_godot_ratio: ratio, browser_reports: browserReports, engine };
}

function select(value: string, allowed: string[], name: string): string[] {
  const selected = value.split(",");
  if (!selected.length || selected.some(item => !allowed.includes(item)) || new Set(selected).size !== selected.length) {
    throw new Error(`--${name} must select unique values from ${allowed.join(",")}`);
  }
  return selected;
}

function hasEngineError(log: string): boolean {
  return /^(?:SCRIPT ERROR:|ERROR:)|panicked at|D3D12 device removed|D3D12[^\r\n]*\b(?:ERROR|CORRUPTION)\b|Validation Error:|VUID-|WARNING:.*RIDs?\b.*leaked/mi.test(log);
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
