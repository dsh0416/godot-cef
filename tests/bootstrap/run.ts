/** Real-editor regression: cold Store addon -> offline install -> fresh load. */
import { spawnSync } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, readdir, symlink, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { parseArgs } from "node:util";
import assert from "node:assert/strict";
import { pack, validate } from "../../scripts/build/pack.ts";
import type { InstallManifest } from "../../scripts/build/install-manifest.ts";

const { values } = parseArgs({ options: {
  addon: { type: "string" }, godot: { type: "string" },
  "cef-source": { type: "string" }, "cef-source-x64": { type: "string" },
  output: { type: "string", default: "target/bootstrap-tests" },
} });
if (!values.addon || !values.godot || !values["cef-source"]) {
  throw new Error("Required: --addon <assembled addon> --godot <executable> --cef-source <extracted CEF>; macOS also --cef-source-x64");
}
const addon = resolve(values.addon);
const godot = resolve(values.godot);
const manifest: InstallManifest = JSON.parse(await readFile(join(addon, "runtime-install.json"), "utf8"));
const arch = process.arch === "arm64" ? "aarch64" : "x86_64";
const target = process.platform === "darwin" ? "universal-apple-darwin" :
  `${arch}-${process.platform === "win32" ? "pc-windows-msvc" : "unknown-linux-gnu"}`;
const platform = manifest.platforms.find(platform => platform.target === target);
assert(platform, `assembled addon must include host ${target}`);
await mkdir(resolve(values.output!), { recursive: true });
const output = await mkdtemp(join(resolve(values.output!), "run-"));
const artifacts = join(output, "artifacts");
await cp(join(addon, platform.directory), join(artifacts, `gdcef-${target}`), { recursive: true, verbatimSymlinks: true });
const project = join(output, "project");
const store = join(project, "addons/godot_cef");
await pack(artifacts, store, addon, "store", { allowPartial: true });
await validate(store);
assert(!(await readdir(store)).includes("godot_cef.gdextension"), "cold Store addon must not load core");
await writeFile(join(project, "project.godot"), '[application]\nconfig/name="Godot CEF Setup Test"\n[rendering]\nrenderer/rendering_method="gl_compatibility"\n');
let source = resolve(values["cef-source"]);
if (process.platform === "darwin") {
  assert(values["cef-source-x64"], "macOS test needs both CEF architectures");
  source = join(output, "cef-sources");
  await mkdir(source);
  await symlink(resolve(values["cef-source"]), join(source, "aarch64-apple-darwin"), "dir");
  await symlink(resolve(values["cef-source-x64"]), join(source, "x86_64-apple-darwin"), "dir");
}

const evidence: { name: string; code: number | null; seconds: number }[] = [];
async function run(name: string, args: string[], expected: number, marker?: string) {
  const start = Date.now();
  const env = { ...process.env };
  delete env.CEF_PATH;
  delete env.CEF_PATH_ARM64;
  delete env.CEF_PATH_X64;
  const result = spawnSync(godot, ["--headless", "--editor", "--path", project, ...args], {
    encoding: "utf8", windowsHide: true, timeout: 180_000, maxBuffer: 8 * 1024 * 1024, env,
  });
  const log = `${result.stdout ?? ""}${result.stderr ?? ""}`;
  await writeFile(join(output, `${name}.log`), log);
  evidence.push({ name, code: result.status, seconds: (Date.now() - start) / 1000 });
  await writeFile(join(output, "summary.json"), JSON.stringify({ output, target, evidence }, null, 2));
  if (result.error) throw result.error;
  assert.equal(result.status, expected, `${name}: ${log}`);
  if (marker) assert(log.includes(marker), `${name}: missing ${marker}: ${log}`);
  if (expected === 0) {
    assert(!/SCRIPT ERROR:|ERROR:|panicked at|entry point .* not found/.test(log), `${name}: unexpected engine error: ${log}`);
  }
  console.log(`${name}: passed (${evidence.at(-1)!.seconds.toFixed(1)}s)`);
}

await run("cold-import", ["--quit-after", "20"], 0);
await run("missing-preflight", ["--", "--godot-cef-check=host"], 1);
await run("invalid-source", ["--", "--godot-cef-install=host", `--godot-cef-source=${join(output, "missing-source")}`], 1);
assert(!(await readdir(store)).includes("godot_cef.gdextension"), "failed installation must not activate core");
await run("install", ["--", "--godot-cef-install=host", `--godot-cef-source=${source}`], 0, "GODOT_CEF_INSTALL_OK");
const descriptor = await readFile(join(store, "godot_cef.gdextension"), "utf8");
assert(descriptor.includes(platform.directory), "active descriptor must use versioned runtime");
assert(descriptor.includes('entry_symbol = "gdext_rust_init"'), "clean shutdown must publish the real browser entry");
assert((await readFile(join(project, ".godot/extension_list.cfg"), "utf8")).includes("res://addons/godot_cef/godot_cef.gdextension"), "the next startup must discover core before rendering starts");
await run("restart-preflight", ["--", "--godot-cef-check=host"], 0, "GODOT_CEF_CHECK_OK");
const receiptPath = join(store, platform.directory, ".runtime-installed.json");
const receipt = await readFile(receiptPath, "utf8");
await run("repeat-install", ["--", "--godot-cef-install=host", `--godot-cef-source=${source}`], 0, "GODOT_CEF_INSTALL_OK");
assert.equal(await readFile(receiptPath, "utf8"), receipt, "repeat installation must preserve receipt");
if (process.platform !== "darwin") {
  const { unlink } = await import("node:fs/promises");
  const helper = process.platform === "win32" ? "gdcef_helper.exe" : "gdcef_helper";
  await unlink(join(store, platform.directory, helper));
  await run("repair-missing-helper", ["--", "--godot-cef-install=host", `--godot-cef-source=${join(output, "missing-source")}`], 0, "GODOT_CEF_INSTALL_OK");
  assert((await readFile(join(store, platform.directory, helper))).length > 0, "offline repair must restore the embedded helper");
  assert.equal(await readFile(receiptPath, "utf8"), receipt, "helper repair must preserve the installed runtime receipt");
}
await run("unsupported-preflight", ["--", "--godot-cef-check=wasm32-unknown-unknown"], 1);

// Simulate an installed update while the active descriptor still loads the old
// core. Both libraries register CefTexture, so class existence cannot prove that
// the version selected by the new manifest is the one actually loaded.
const upgraded = structuredClone(manifest);
upgraded.plugin_version += "+upgrade-test";
const current = upgraded.platforms.find(item => item.target === target)!;
current.directory = `${platform.directory}-upgrade-test`;
current.library = platform.library.replace(platform.directory, current.directory);
current.bootstrap = platform.bootstrap.replace(platform.directory, current.directory);
await cp(join(store, platform.directory), join(store, current.directory), { recursive: true, verbatimSymlinks: true });
const nextReceipt = { ...JSON.parse(receipt), plugin_version: upgraded.plugin_version };
await writeFile(join(store, current.directory, ".runtime-installed.json"), JSON.stringify(nextReceipt));
await writeFile(join(store, "runtime-install.json"), JSON.stringify(upgraded));
await run("old-core-preflight", ["--", "--godot-cef-check=host"], 1);
await run("activate-upgrade", ["--", "--godot-cef-install=host", `--godot-cef-source=${source}`], 0, "GODOT_CEF_INSTALL_OK");
await run("upgrade-restart-preflight", ["--", "--godot-cef-check=host"], 0, "GODOT_CEF_CHECK_OK");
assert((await readdir(join(store, platform.directory))).length > 0, "upgrades must retain the previous runtime");
console.log(`Bootstrap smoke passed. Evidence: ${output}`);
