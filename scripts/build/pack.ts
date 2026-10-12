import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { assertSeparate, copy, exists, requirePaths, root } from "./common.ts";
import { includes, platforms, type Variant } from "./platforms.ts";

export function manifestForVariant(manifest: string, variant: Variant): string {
  if (variant === "full") return manifest;
  let skipping = false;
  const lines = manifest.match(/[^\n]*\n|[^\n]+$/g) ?? [];
  const result = lines.filter(line => {
    const trimmed = line.trim();
    if (skipping) {
      if (trimmed === "}") skipping = false;
      return false;
    }
    const match = /^(windows|linux)\.arm64\s*=\s*(.*)$/.exec(trimmed);
    if (match) { skipping = match[2] === "{"; return false; }
    return true;
  }).join("");
  if (skipping) throw new Error("Unterminated ARM64 dependency dictionary in GDExtension manifest");
  return result;
}

export async function pack(artifacts: string, output: string, addon = join(root, "addons/godot_cef"), variant: Variant = "full"): Promise<void> {
  artifacts = resolve(artifacts); output = resolve(output); addon = resolve(addon);
  assertSeparate(output, [artifacts, addon]);
  const selected = [];
  for (const platform of platforms) {
    const source = join(artifacts, `gdcef-${platform.target}`);
    if (includes(variant, platform.target) && await exists(source)) {
      await requirePaths(source, platform.files, platform.dirs);
      selected.push({ platform, source });
    }
  }
  if (!selected.length) throw new Error("No platform artifacts found");
  const manifest = manifestForVariant(await readFile(join(addon, "godot_cef.gdextension"), "utf8"), variant);
  await rm(output, { recursive: true, force: true });
  await mkdir(join(output, "bin"), { recursive: true });
  await writeFile(join(output, "godot_cef.gdextension"), manifest);
  if (await exists(join(addon, "icons"))) await copy(join(addon, "icons"), join(output, "icons"));
  for (const { platform, source } of selected) await copy(source, join(output, "bin", platform.target));
  console.error(`Packed ${selected.length} platform(s) into ${output}`);
}

export async function validate(addon: string, variant?: Variant): Promise<void> {
  await requirePaths(addon, [], ["bin"]);
  const manifest = variant ? await readFile(join(addon, "godot_cef.gdextension"), "utf8") : "";
  let count = 0;
  for (const platform of platforms) {
    const directory = join(addon, "bin", platform.target);
    const present = await exists(directory);
    if (variant) {
      const required = includes(variant, platform.target);
      if (manifest.includes(platform.target) !== required) throw new Error(`Manifest does not match ${variant}: ${platform.target}`);
      if (present !== required) throw new Error(`${required ? "Missing required" : "Excluded"} target: ${platform.target}`);
    }
    if (present) {
      await requirePaths(directory, platform.files, platform.dirs);
      count++;
    }
  }
  if (!count) throw new Error("No platform directories found under addon bin/");
  console.error(`Validated ${count} platform(s)`);
}
