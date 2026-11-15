import { readFile, writeFile } from "node:fs/promises";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import type { EmbeddedArchive } from "./archive.ts";
import { root } from "./common.ts";
import { macTarget, platforms } from "./platforms.ts";
import { lockedCefVersion } from "./versions.ts";

export interface InstalledPlatform {
  target: string;
  directory: string;
  library: string;
  bootstrap: string;
  helper: EmbeddedArchive;
  files: string[];
  dirs: string[];
}
export interface InstallManifest {
  schema_version: 1;
  plugin_version: string;
  cef_version: string;
  platforms: InstalledPlatform[];
}
export async function installManifest(): Promise<InstallManifest> {
  const { version } = JSON.parse(await readFile(join(root, "package.json"), "utf8"));
  return { schema_version: 1, plugin_version: version, cef_version: (await lockedCefVersion()).runtime, platforms: [] };
}
export function platformDirectory(manifest: InstallManifest, target: string): string {
  return `bin/${target}/${manifest.plugin_version}-cef-${manifest.cef_version}`;
}
export function coreDescriptor(manifest: InstallManifest): string {
  const libraries: string[] = [];
  for (const record of manifest.platforms) {
    const platform = platforms.find(p => p.target === record.target)!;
    const library = record.target === macTarget ? record.library.slice(0, -"/libgdcef.dylib".length) : record.library;
    libraries.push(`${platform.feature} = ${JSON.stringify(library)}`);
  }
  const dependencies: string[] = [];
  for (const record of manifest.platforms) {
    // Godot exports a macOS framework with all nested resources automatically.
    if (record.target === macTarget) {
      if (record.dirs.includes("cef-licenses")) dependencies.push(`macos = { ${JSON.stringify(`${record.directory}/cef-licenses`)} : "" }`, "");
      continue;
    }
    const platform = platforms.find(p => p.target === record.target)!;
    const files = [...new Set([platform.library, ...record.files, ...record.dirs])];
    dependencies.push(`${platform.feature} = {`, ...files.map(path => `  ${JSON.stringify(`${record.directory}/${path}`)} : "",`), "}", "");
  }
  return readFileSync(join(root, "addons/godot_cef/godot_cef.gdextension.in"), "utf8").replaceAll("\r\n", "\n")
    .replace("{{libraries}}", libraries.join("\n")).replace("{{dependencies}}", dependencies.join("\n"));
}
export function bootstrapDescriptor(manifest: InstallManifest): string {
  return ["[configuration]", 'entry_symbol = "gdcef_bootstrap_init"', "compatibility_minimum = 4.6", "reloadable = false", "", "[libraries]",
    ...manifest.platforms.map(record => `${platforms.find(p => p.target === record.target)!.feature} = ${JSON.stringify(record.bootstrap)}`), ""].join("\n");
}
export async function writeAddonMetadata(addon: string, manifest: InstallManifest, installed: boolean): Promise<void> {
  await writeFile(join(addon, "runtime-install.json"), JSON.stringify(manifest, null, 2) + "\n");
  await writeFile(join(addon, "godot_cef_bootstrap.gdextension"), bootstrapDescriptor(manifest));
  // The repository keeps a reference template; assembled/packed distributions receive exact paths.
  if (resolve(addon) !== resolve(root, "addons/godot_cef")) await writeFile(join(addon, "godot_cef.gdextension.in"), coreDescriptor(manifest));
  if (installed) await writeFile(join(addon, "godot_cef.gdextension"), coreDescriptor(manifest));
}
export async function writeReceipt(directory: string, manifest: InstallManifest, record: InstalledPlatform): Promise<void> {
  await writeFile(join(directory, ".runtime-installed.json"), JSON.stringify({
    schema_version: 1, plugin_version: manifest.plugin_version, cef_version: manifest.cef_version, helper_sha256: record.helper.sha256,
  }, null, 2) + "\n");
}
