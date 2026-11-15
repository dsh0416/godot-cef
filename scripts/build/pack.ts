import { createHash } from "node:crypto";
import { mkdir, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { assertSeparate, contains, copy, exists, requirePaths, root } from "./common.ts";
import { bootstrapDescriptor, coreDescriptor, installManifest, platformDirectory, writeAddonMetadata, type InstallManifest, type InstalledPlatform } from "./install-manifest.ts";
import { macTarget, platforms, type Variant } from "./platforms.ts";

function nativeFiles(record: InstalledPlatform): string[] {
  const platform = platforms.find(item => item.target === record.target)!;
  return [platform.library, platform.bootstrap, ...(record.target === macTarget ? ["Godot CEF.framework/Resources/Info.plist"] : [])];
}

function checkRecord(manifest: InstallManifest, record: InstalledPlatform): void {
  const platform = platforms.find(item => item.target === record.target);
  if (!platform || record.directory !== platformDirectory(manifest, record.target)
      || record.library !== `${record.directory}/${platform.library}`
      || record.bootstrap !== `${record.directory}/${platform.bootstrap}`) throw new Error(`Invalid platform layout: ${record.target}`);
  for (const path of [...record.files, ...record.dirs]) {
    if (!path || path.includes("\\") || path.startsWith("/") || path.split("/").some(part => part === ".." || part === ".")
      || !contains(record.directory, join(record.directory, path))) throw new Error(`Unsafe runtime path: ${path}`);
  }
  if (!Number.isSafeInteger(record.helper.offset) || record.helper.offset < 0 || !Number.isSafeInteger(record.helper.length)
      || record.helper.length <= 0 || !/^[a-f0-9]{64}$/.test(record.helper.sha256)) throw new Error("Invalid embedded helper metadata");
}

async function verifyPayload(library: string, record: InstalledPlatform): Promise<void> {
  const binary = await readFile(library);
  const { offset, length, sha256 } = record.helper;
  if (offset + length > binary.length || createHash("sha256").update(binary.subarray(offset, offset + length)).digest("hex") !== sha256) {
    throw new Error(`Embedded helper archive does not match ${record.target}`);
  }
}

async function verifyReceipt(directory: string, manifest: InstallManifest, record: InstalledPlatform): Promise<void> {
  const receipt = JSON.parse(await readFile(join(directory, ".runtime-installed.json"), "utf8"));
  if (receipt.schema_version !== 1 || receipt.plugin_version !== manifest.plugin_version || receipt.cef_version !== manifest.cef_version
      || receipt.helper_sha256 !== record.helper.sha256 || receipt.signing_pending === true) throw new Error(`Incomplete or mismatched runtime receipt: ${record.target}`);
}

async function verifyExecutableModes(directory: string, record: InstalledPlatform): Promise<void> {
  if (process.platform === "win32" || record.target.endsWith("-pc-windows-msvc")) return;
  for (const path of record.files) {
    if (path === "gdcef_helper" || path === "chrome-sandbox" || path.includes("/Contents/MacOS/")) {
      if ((await stat(join(directory, path))).mode & 0o111) continue;
      throw new Error(`Native executable lost its execute permission: ${path}`);
    }
  }
}

export async function pack(artifacts: string, output: string, addon = join(root, "addons/godot_cef"), variant: Variant = "full"): Promise<void> {
  artifacts = resolve(artifacts); output = resolve(output); addon = resolve(addon);
  assertSeparate(output, [artifacts, addon]);
  const manifest = await installManifest();
  const selected: { source: string; record: InstalledPlatform }[] = [];
  for (const platform of platforms) {
    const source = join(artifacts, `gdcef-${platform.target}`);
    if (await exists(source)) {
      const metadata = JSON.parse(await readFile(join(source, "runtime-platform.json"), "utf8")) as InstallManifest;
      if (metadata.schema_version !== 1 || metadata.plugin_version !== manifest.plugin_version || metadata.cef_version !== manifest.cef_version
          || metadata.platforms.length !== 1 || metadata.platforms[0].target !== platform.target) throw new Error(`Artifact version mismatch: ${platform.target}`);
      const record = metadata.platforms[0];
      checkRecord(manifest, record);
      await requirePaths(source, [...nativeFiles(record), ...record.files], record.dirs);
      await verifyPayload(join(source, platform.library), record);
      await verifyReceipt(source, manifest, record);
      await verifyExecutableModes(source, record);
      selected.push({ record, source });
      manifest.platforms.push(record);
    }
  }
  if (!selected.length) throw new Error("No platform artifacts found");
  const license = await readFile(join(root, "LICENSE"));
  await rm(output, { recursive: true, force: true });
  await mkdir(join(output, "bin"), { recursive: true });
  await writeFile(join(output, "LICENSE"), license);
  if (await exists(join(addon, "icons"))) {
    for (const entry of await readdir(join(addon, "icons"), { withFileTypes: true })) {
      if (entry.isFile() && /\.(png|svg)$/.test(entry.name)) {
        await copy(join(addon, "icons", entry.name), join(output, "icons", entry.name));
      }
    }
  }
  for (const { record, source } of selected) {
    const destination = join(output, record.directory);
    if (variant === "full") await copy(source, destination);
    else for (const file of nativeFiles(record)) await copy(join(source, file), join(destination, file));
  }
  await writeAddonMetadata(output, manifest, variant === "full");
  console.error(`Packed ${selected.length} platform(s) into ${output}`);
}

export async function validate(addon: string, variant?: Variant): Promise<void> {
  await requirePaths(addon, ["runtime-install.json", "godot_cef_bootstrap.gdextension", "godot_cef.gdextension.in", ...(variant ? ["LICENSE"] : [])], ["bin"]);
  const manifest = JSON.parse(await readFile(join(addon, "runtime-install.json"), "utf8")) as InstallManifest;
  if (manifest.schema_version !== 1 || !manifest.platforms.length) throw new Error("Invalid runtime install manifest");
  const targets = manifest.platforms.map(item => item.target);
  if (new Set(targets).size !== targets.length || (variant && platforms.some(platform => !targets.includes(platform.target)))) throw new Error("Missing or duplicate platform artifacts");
  const installed = variant ? variant === "full" : await exists(join(addon, "godot_cef.gdextension"));
  if ((await exists(join(addon, "godot_cef.gdextension"))) !== installed) throw new Error("Store must not contain an active core GDExtension");
  if (await readFile(join(addon, "godot_cef_bootstrap.gdextension"), "utf8") !== bootstrapDescriptor(manifest)
      || (resolve(addon) !== resolve(root, "addons/godot_cef") && await readFile(join(addon, "godot_cef.gdextension.in"), "utf8") !== coreDescriptor(manifest))) throw new Error("GDExtension descriptors do not match native libraries");
  if (installed && await readFile(join(addon, "godot_cef.gdextension"), "utf8") !== coreDescriptor(manifest)) throw new Error("Full addon core descriptor mismatch");
  const allowed = new Set<string>();
  for (const record of manifest.platforms) {
    checkRecord(manifest, record);
    const directory = join(addon, record.directory);
    await requirePaths(directory, installed ? [...nativeFiles(record), ...record.files, ".runtime-installed.json"] : nativeFiles(record), installed ? record.dirs : []);
    await verifyPayload(join(addon, record.library), record);
    if (installed) {
      await verifyReceipt(directory, manifest, record);
      await verifyExecutableModes(directory, record);
    }
    for (const file of nativeFiles(record)) allowed.add(`${record.directory}/${file}`);
  }
  if (!installed) {
    for (const path of ["LICENSE", "runtime-install.json", "godot_cef_bootstrap.gdextension", "godot_cef.gdextension.in"]) allowed.add(path);
    async function checkTree(relative: string): Promise<void> {
      for (const entry of await readdir(join(addon, relative), { withFileTypes: true })) {
        const path = relative ? `${relative}/${entry.name}` : entry.name;
        if (entry.isSymbolicLink()) throw new Error(`Store native tree contains a symlink: ${path}`);
        if (entry.isDirectory()) await checkTree(path);
        else if (!allowed.has(path) && !/^icons\/[^/]+\.(png|svg)$/.test(path)) throw new Error(`Unexpected Store file (runtime/executables must be downloaded): ${path}`);
      }
    }
    await checkTree("");
  }
  console.error(`Validated ${manifest.platforms.length} platform(s), ${installed ? "offline" : "installer"} distribution`);
}
