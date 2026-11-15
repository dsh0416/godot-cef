import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { assertSeparate, copy, exists, requirePaths, root, run } from "./common.ts";
import { embeddedArchive, helperArchive } from "./archive.ts";
import { installManifest, platformDirectory, writeAddonMetadata, writeReceipt, type InstallManifest, type InstalledPlatform } from "./install-manifest.ts";
import { linuxAssets, macTarget, macTargets, platforms, windowsAssets } from "./platforms.ts";

export interface BuildOptions { release?: boolean; target?: string; targetDir?: string }
export interface BuildConfig {
  target: string;
  targets: readonly string[];
  targetDir: string;
  profile: "debug" | "release";
}

export function configuration(options: BuildOptions): BuildConfig {
  const { platform: host, arch, env } = process;
  const architecture = arch === "x64" ? "x86_64" : arch === "arm64" ? "aarch64" : undefined;
  if (!architecture) throw new Error(`Unsupported host architecture: ${arch}`);
  let target = options.target;
  if (!target || target === "host") {
    target = host === "darwin" ? macTarget : `${architecture}-${host === "win32" ? "pc-windows-msvc" : "unknown-linux-gnu"}`;
  }
  const suffix = host === "win32" ? "-pc-windows-msvc" : "-unknown-linux-gnu";
  if (host === "darwin" ? target !== macTarget :
    !["win32", "linux"].includes(host) || !platforms.some(p => p.target === target && target.endsWith(suffix))) {
    throw new Error(`Unsupported target ${target} on ${host}`);
  }
  return {
    target, targets: target === macTarget ? macTargets : [target],
    targetDir: resolve(options.targetDir || env.CARGO_TARGET_DIR || join(root, "target")),
    profile: options.release ? "release" : "debug",
  };
}

function cargoInvocation(config: BuildConfig, target: string, packages: string[]): { args: string[]; env: NodeJS.ProcessEnv } {
  const { env } = process;
  const cefPath = config.target === macTarget
    ? env[target === macTargets[0] ? "CEF_PATH_ARM64" : "CEF_PATH_X64"]
    : env.CEF_PATH;
  if (!cefPath) throw new Error(`Missing CEF path for ${target}`);
  const args = ["build", "--locked", ...packages.flatMap(name => ["--package", name]),
    "--target", target, "--target-dir", config.targetDir];
  if (config.profile === "release") args.push("--release");
  return { args, env: { ...env, CEF_PATH: resolve(cefPath) } };
}

export async function build(config: BuildConfig): Promise<void> {
  // Build helpers before the core: its compressed payload is a compile-time input.
  for (const target of config.targets) {
    const invocation = cargoInvocation(config, target, ["gdcef_helper"]);
    run("cargo", invocation.args, invocation.env);
  }
  await prepareHelper(config);
  for (const target of config.targets) {
    const invocation = cargoInvocation(config, target, ["gdcef", "gdcef_bootstrap"]);
    run("cargo", invocation.args, { ...invocation.env, GDCEF_HELPER_ARCHIVE: helperPaths(config).archive });
  }
}

function helperPaths(config: BuildConfig) {
  const base = join(config.targetDir, "gdcef-helper", config.target, config.profile);
  return { base, payload: join(base, "payload"), archive: join(base, "helper.tar.gz"), index: join(base, "files.json") };
}

async function prepareHelper(config: BuildConfig): Promise<void> {
  const paths = helperPaths(config);
  const sources = config.targets.map(target => join(config.targetDir, target, config.profile));
  assertSeparate(paths.payload, sources);
  await rm(paths.payload, { recursive: true, force: true });
  await mkdir(paths.payload, { recursive: true });
  if (config.target === macTarget) {
    const helper = join(paths.base, "gdcef_helper_universal");
    run("lipo", ["-create", ...sources.map(dir => join(dir, "gdcef_helper")), "-output", helper]);
    const app = await createApp(join(paths.payload, "Godot CEF.framework/Helpers"), "Godot CEF", helper, false);
    // The parent app is signed only after CEF is installed. Its seal would otherwise change.
    for (const suffix of [" (GPU)", " (Renderer)", " (Plugin)", " (Alerts)", ""]) {
      sign(await createApp(join(app, "Contents/Frameworks"), `Godot CEF Helper${suffix}`, helper, true));
    }
    await mkdir(join(paths.payload, "Godot CEF.framework/Resources"), { recursive: true });
    await writePlist(join(paths.payload, "Godot CEF.framework/Resources/Info.plist"), {
      ...plistBase("libgdcef.dylib", true), CFBundleSupportedPlatforms: ["MacOSX"],
    });
  } else {
    const helper = config.target.endsWith("-pc-windows-msvc") ? "gdcef_helper.exe" : "gdcef_helper";
    await copy(join(sources[0], helper), join(paths.payload, helper));
  }
  await writeFile(paths.index, JSON.stringify(await helperArchive(paths.payload, paths.archive)));
}

async function assembleNative(config: BuildConfig): Promise<void> {
  const { env } = process;
  const windows = config.target.endsWith("-pc-windows-msvc");
  const assets = windows ? windowsAssets : linuxAssets;
  const binaries = windows ? ["gdcef.dll", "gdcef_bootstrap.dll"] : ["libgdcef.so", "libgdcef_bootstrap.so"];
  const source = join(config.targetDir, config.target, config.profile);
  if (!env.CEF_PATH) throw new Error("Set CEF_PATH to the matching CEF runtime");
  const cef = resolve(env.CEF_PATH);
  const output = join(root, "addons/godot_cef", platformDirectory(await installManifest(), config.target));
  assertSeparate(output, [source, cef]);
  await requirePaths(source, binaries, []);
  await requirePaths(cef, assets, ["locales"]);
  await rm(output, { recursive: true, force: true });
  for (const name of binaries) await copy(join(source, name), join(output, name));
  await copy(helperPaths(config).payload, output);
  for (const name of [...assets, "locales"]) await copy(join(cef, name), join(output, name));
  if (windows && await exists(join(cef, "dxil.dll"))) await copy(join(cef, "dxil.dll"), join(output, "dxil.dll"));
  // Strip only the staged runtime, never Cargo or the shared CEF cache (coverage reuses both).
  if (!windows) {
    const strip = config.target.startsWith("aarch64") ? "aarch64-linux-gnu-strip" : "strip";
    for (const name of ["libcef.so", "libvk_swiftshader.so", "libvulkan.so.1"]) {
      run(strip, ["--strip-debug", join(output, name)]);
    }
  }
}

function plistBase(executable: string, framework: boolean): Record<string, unknown> {
  return {
    CFBundleDevelopmentRegion: "en", CFBundleExecutable: executable,
    CFBundleIdentifier: framework ? "me.delton.gdcef.libgdcef" : "me.delton.gdcef.helper",
    CFBundleInfoDictionaryVersion: "6.0", CFBundleName: "gdcef",
    CFBundlePackageType: framework ? "FMWK" : "APPL", CFBundleSignature: "????",
    CFBundleVersion: "1.0.0", CFBundleShortVersionString: "1.0",
    LSEnvironment: { MallocNanoZone: "0" }, LSFileQuarantineEnabled: true,
    LSMinimumSystemVersion: "11.0",
  };
}

async function writePlist(path: string, value: Record<string, unknown>): Promise<void> {
  await writeFile(path, JSON.stringify(value));
  run("plutil", ["-convert", "xml1", path]);
}

function sign(path: string): void { run("codesign", ["--force", "--sign", "-", path]); }

async function createApp(parent: string, name: string, executable: string, helper: boolean): Promise<string> {
  const app = join(parent, `${name}.app`);
  for (const dir of ["MacOS", "Resources", "Frameworks"]) await mkdir(join(app, "Contents", dir), { recursive: true });
  await copy(executable, join(app, "Contents/MacOS", name));
  await writePlist(join(app, "Contents/Info.plist"), {
    ...plistBase(name, false), CFBundleDisplayName: name,
    ...(helper ? { LSUIElement: "1" } : {}),
    NSBluetoothAlwaysUsageDescription: name, NSSupportsAutomaticGraphicsSwitching: true,
    NSWebBrowserPublicKeyCredentialUsageDescription: name,
    NSCameraUsageDescription: name, NSMicrophoneUsageDescription: name,
  });
  return app;
}

async function assembleMac(config: BuildConfig): Promise<void> {
  const { env } = process;
  const output = join(config.targetDir, config.profile);
  const framework = join(output, "Godot CEF.framework");
  const cefDirs = [env.CEF_PATH_ARM64, env.CEF_PATH_X64];
  if (cefDirs.some(path => !path)) throw new Error("Set CEF_PATH_ARM64 and CEF_PATH_X64");
  const sourceFrameworks = cefDirs.map(path => join(resolve(path!), "Chromium Embedded Framework.framework"));
  const binaryDirs = macTargets.map(target => join(config.targetDir, target, config.profile));
  const deployment = join(root, "addons/godot_cef", platformDirectory(await installManifest(), macTarget));
  assertSeparate(framework, [...sourceFrameworks, ...binaryDirs]);
  assertSeparate(deployment, [framework, ...sourceFrameworks, ...binaryDirs]);
  for (const dir of binaryDirs) await requirePaths(dir, ["libgdcef_bootstrap.dylib", "libgdcef.dylib"], []);
  for (const dir of sourceFrameworks) await requirePaths(dir, ["Chromium Embedded Framework"], ["Libraries", "Resources"]);
  await rm(framework, { recursive: true, force: true });
  await copy(join(helperPaths(config).payload, "Godot CEF.framework"), framework);
  await mkdir(join(framework, "Resources"), { recursive: true });
  const app = join(framework, "Helpers/Godot CEF.app");
  const frameworks = join(app, "Contents/Frameworks");
  for (let index = 0; index < sourceFrameworks.length; index++) {
    const destination = join(frameworks, `Chromium Embedded Framework (${index === 0 ? "ARM64" : "X86_64"}).framework`);
    await copy(sourceFrameworks[index], destination);
    for (const name of await readdir(join(destination, "Libraries"))) {
      if (name.endsWith(".dylib")) sign(join(destination, "Libraries", name));
    }
    sign(destination);
  }
  sign(app);
  const library = join(framework, "libgdcef.dylib");
  run("lipo", ["-create", ...binaryDirs.map(dir => join(dir, "libgdcef.dylib")), "-output", library]);
  await writePlist(join(framework, "Resources/Info.plist"), {
    ...plistBase("libgdcef.dylib", true), CFBundleSupportedPlatforms: ["MacOSX"],
  });
  run("install_name_tool", ["-id", "@rpath/Godot CEF.framework/libgdcef.dylib", library]);
  sign(framework);
  await rm(deployment, { recursive: true, force: true });
  await copy(framework, join(deployment, "Godot CEF.framework"));
  const bootstrap = join(deployment, "libgdcef_bootstrap.dylib");
  run("lipo", ["-create", ...binaryDirs.map(dir => join(dir, "libgdcef_bootstrap.dylib")), "-output", bootstrap]);
  sign(bootstrap);
}

export async function assemble(config: BuildConfig): Promise<void> {
  await requirePaths(helperPaths(config).base, ["helper.tar.gz", "files.json"], ["payload"]);
  if (config.target === macTarget) await assembleMac(config);
  else await assembleNative(config);
  const addon = join(root, "addons/godot_cef");
  const manifest = await installManifest();
  const platform = platforms.find(item => item.target === config.target)!;
  const directory = platformDirectory(manifest, config.target);
  // Preserve the upstream notices alongside the runtime and include them in exports.
  for (const target of config.targets) {
    const cefPath = config.target === macTarget ? process.env[target === macTargets[0] ? "CEF_PATH_ARM64" : "CEF_PATH_X64"] : process.env.CEF_PATH;
    if (!cefPath) continue;
    for (const name of await readdir(cefPath)) {
      if (/^(LICENSE(?:\.txt)?|CREDITS(?:\.html)?)$/i.test(name)) await copy(join(cefPath, name), join(addon, directory, "cef-licenses", target, name));
    }
  }
  const helperFiles = JSON.parse(await readFile(helperPaths(config).index, "utf8")) as string[];
  const record: InstalledPlatform = {
    target: platform.target, directory, library: `${directory}/${platform.library}`, bootstrap: `${directory}/${platform.bootstrap}`,
    helper: await embeddedArchive(join(addon, directory, platform.library), helperPaths(config).archive),
    files: [...new Set([platform.library, ...platform.files, ...helperFiles,
      ...(platform.target.endsWith("-pc-windows-msvc") && await exists(join(addon, directory, "dxil.dll")) ? ["dxil.dll"] : [])])],
    dirs: [...platform.dirs, ...(await exists(join(addon, directory, "cef-licenses")) ? ["cef-licenses"] : [])],
  };
  // CI uploads this platform directory; pack combines these records without loading native code.
  await writeFile(join(addon, directory, "runtime-platform.json"), JSON.stringify({ ...manifest, platforms: [record] }, null, 2) + "\n");
  await writeReceipt(join(addon, directory), manifest, record);
  const previousPath = join(addon, "runtime-install.json");
  if (await exists(previousPath)) {
    const previous = JSON.parse(await readFile(previousPath, "utf8")) as InstallManifest;
    if (previous.plugin_version === manifest.plugin_version && previous.cef_version === manifest.cef_version) {
      manifest.platforms.push(...previous.platforms.filter(item => item.target !== record.target));
    }
  }
  manifest.platforms.push(record);
  await writeAddonMetadata(addon, manifest, true);
  console.error(`Bundled ${config.target}`);
}
