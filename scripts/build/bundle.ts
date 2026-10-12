import { mkdir, readdir, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { assertSeparate, copy, requirePaths, root, run } from "./common.ts";
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

function cargoInvocation(config: BuildConfig, target: string): { args: string[]; env: NodeJS.ProcessEnv } {
  const { env } = process;
  const cefPath = config.target === macTarget
    ? env[target === macTargets[0] ? "CEF_PATH_ARM64" : "CEF_PATH_X64"]
    : env.CEF_PATH;
  if (!cefPath) throw new Error(`Missing CEF path for ${target}`);
  const args = ["build", "--locked", "--package", "gdcef", "--package", "gdcef_helper",
    "--target", target, "--target-dir", config.targetDir];
  if (config.profile === "release") args.push("--release");
  return { args, env: { ...env, CEF_PATH: resolve(cefPath) } };
}

export function build(config: BuildConfig): void {
  // Cargo owns Rust's incremental dependency graph. Both packages share each invocation.
  for (const target of config.targets) {
    const invocation = cargoInvocation(config, target);
    run("cargo", invocation.args, invocation.env);
  }
}

async function assembleNative(config: BuildConfig): Promise<void> {
  const { env } = process;
  const windows = config.target.endsWith("-pc-windows-msvc");
  const assets = windows ? windowsAssets : linuxAssets;
  const binaries = windows ? ["gdcef.dll", "gdcef_helper.exe"] : ["libgdcef.so", "gdcef_helper"];
  const source = join(config.targetDir, config.target, config.profile);
  if (!env.CEF_PATH) throw new Error("Set CEF_PATH to the matching CEF runtime");
  const cef = resolve(env.CEF_PATH);
  const output = join(root, "addons/godot_cef/bin", config.target);
  assertSeparate(output, [source, cef]);
  await requirePaths(source, binaries, []);
  await requirePaths(cef, assets, ["locales"]);
  await rm(output, { recursive: true, force: true });
  for (const name of binaries) await copy(join(source, name), join(output, name));
  for (const name of [...assets, "locales"]) await copy(join(cef, name), join(output, name));
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
  const deployment = join(root, "addons/godot_cef/bin", macTarget);
  assertSeparate(framework, [...sourceFrameworks, ...binaryDirs]);
  assertSeparate(deployment, [framework, ...sourceFrameworks, ...binaryDirs]);
  for (const dir of binaryDirs) await requirePaths(dir, ["gdcef_helper", "libgdcef.dylib"], []);
  for (const dir of sourceFrameworks) await requirePaths(dir, ["Chromium Embedded Framework"], ["Libraries", "Resources"]);
  await rm(framework, { recursive: true, force: true });
  await mkdir(join(framework, "Resources"), { recursive: true });
  const universalHelper = join(framework, "gdcef_helper_universal");
  run("lipo", ["-create", ...binaryDirs.map(dir => join(dir, "gdcef_helper")), "-output", universalHelper]);
  const app = await createApp(join(framework, "Helpers"), "Godot CEF", universalHelper, false);
  const frameworks = join(app, "Contents/Frameworks");
  for (let index = 0; index < sourceFrameworks.length; index++) {
    const destination = join(frameworks, `Chromium Embedded Framework (${index === 0 ? "ARM64" : "X86_64"}).framework`);
    await copy(sourceFrameworks[index], destination);
    for (const name of await readdir(join(destination, "Libraries"))) {
      if (name.endsWith(".dylib")) sign(join(destination, "Libraries", name));
    }
    sign(destination);
  }
  for (const suffix of [" (GPU)", " (Renderer)", " (Plugin)", " (Alerts)", ""]) {
    sign(await createApp(frameworks, `Godot CEF Helper${suffix}`, universalHelper, true));
  }
  sign(app);
  await rm(universalHelper);
  const library = join(framework, "libgdcef.dylib");
  run("lipo", ["-create", ...binaryDirs.map(dir => join(dir, "libgdcef.dylib")), "-output", library]);
  await writePlist(join(framework, "Resources/Info.plist"), {
    ...plistBase("libgdcef.dylib", true), CFBundleSupportedPlatforms: ["MacOSX"],
  });
  run("install_name_tool", ["-id", "@rpath/Godot CEF.framework/libgdcef.dylib", library]);
  sign(framework);
  await rm(deployment, { recursive: true, force: true });
  await copy(framework, join(deployment, "Godot CEF.framework"));
}

export async function assemble(config: BuildConfig): Promise<void> {
  if (config.target === macTarget) await assembleMac(config);
  else await assembleNative(config);
  console.error(`Bundled ${config.target}`);
}
