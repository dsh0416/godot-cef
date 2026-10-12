import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, readFileSync, writeFileSync } from "node:fs";
import { chmod, mkdir, mkdtemp, readFile, readlink, rm, stat, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { test, type TestContext } from "node:test";
import { assemble, cargoInvocation, configuration } from "./bundle.ts";
import { copy, root, run } from "./common.ts";
import { manifestForVariant, pack, validate } from "./pack.ts";
import { includes, linuxAssets, macTarget, macTargets, platforms, windowsAssets } from "./platforms.ts";
import { cefVersion, validateVersions } from "./versions.ts";

async function temporary(t: TestContext): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "gdcef build "));
  t.after(() => rm(path, { recursive: true, force: true }));
  return path;
}
async function file(path: string, data = "fixture"): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, data);
}
function lock(cef = "154.5.0+154.0.8", sys = cef): string {
  return `[[package]]\nname = "cef"\nversion = "${cef}"\n[[package]]\nname = "cef-dll-sys"\nversion = "${sys}"\n`;
}

test("CEF version parsing accepts TOML syntax and rejects ambiguous or incompatible pins", () => {
  assert.deepEqual(cefVersion(lock()), { core: "154.5.0", runtime: "154.0.8" });
  assert.deepEqual(cefVersion(lock().replaceAll('"', "'") + "# comment\n"), { core: "154.5.0", runtime: "154.0.8" });
  for (const contents of [lock() + lock(), lock("154.5.0"), lock("154.5.0+154.0.8", "154.5.0+154.0.9"),
    lock("0154.5.0+154.0.8"), lock("154.5.0+invalid"), lock("154.5.0+154.0.8+extra"), ""]) {
    assert.throws(() => cefVersion(contents));
  }
});

test("version validation checks package, exporter core and optional runtime", async t => {
  const base = await temporary(t);
  await file(join(base, "Cargo.lock"), lock());
  await file(join(base, "Cargo.toml"), '[workspace.package]\nversion = "2.1.0"\n');
  await file(join(base, "package.json"), '{"version":"2.1.0"}');
  for (const exporter of ["154.5.0", "154.5.0+154.0.8"]) {
    await file(join(base, "mise.toml"), `[tools]\n"cargo:export-cef-dir" = "${exporter}"\n`);
    await validateVersions(base);
  }
  await file(join(base, "mise.toml"), '[tools]\n"cargo:export-cef-dir" = "154.5.0+154.0.9"\n');
  await assert.rejects(validateVersions(base), /must match/);
  await file(join(base, "package.json"), '{"version":"2.0.0"}');
  await assert.rejects(validateVersions(base), /workspace version/);
});

test("build plans propagate target directories, coverage flags and per-architecture CEF", () => {
  const config = configuration({ release: true, targetDir: "/tmp/output with spaces" }, "darwin", "arm64", {});
  assert.deepEqual(config.targets, macTargets);
  for (const target of macTargets) {
    const result = cargoInvocation(config, target, {
      CEF_PATH_ARM64: "/tmp/cef arm", CEF_PATH_X64: "/tmp/cef x64", RUSTFLAGS: "-C instrument-coverage",
    });
    assert.equal(result.env.CEF_PATH, resolve(target === macTargets[0] ? "/tmp/cef arm" : "/tmp/cef x64"));
    assert.equal(result.env.RUSTFLAGS, "-C instrument-coverage");
    assert.deepEqual(result.args.slice(-5), ["--target", target, "--target-dir", config.targetDir, "--release"]);
  }
  const cross = configuration({ target: "aarch64-unknown-linux-gnu" }, "linux", "x64", { CARGO_TARGET_DIR: "coverage-build" });
  assert.equal(cross.targetDir, resolve("coverage-build"));
  assert.throws(() => configuration({ target: "x86_64-pc-windows-msvc" }, "linux", "x64"), /Unsupported/);
  assert.throws(() => configuration({}, "linux", "ppc64"), /architecture/);
  assert.throws(() => cargoInvocation(cross, cross.target, {}), /Missing CEF/);
});

test("Store manifest filtering removes only the excluded libraries and dependency dictionaries", () => {
  const source = '[libraries]\r\nwindows.arm64 = "arm"\r\nwindows.x86_64 = "x64"\r\n[dependencies]\r\nlinux.arm64 = {\r\n"a": "b"\r\n}\r\nmacos = {}\r\n';
  assert.equal(manifestForVariant(source, "full"), source);
  assert.equal(manifestForVariant(source, "store"), '[libraries]\r\nwindows.x86_64 = "x64"\r\n[dependencies]\r\nmacos = {}\r\n');
  assert.throws(() => manifestForVariant("linux.arm64 = {\n", "store"), /Unterminated/);
});

test("Full and Store packages validate their exact platform sets and reject incomplete artifacts", async t => {
  const base = await temporary(t);
  const artifacts = join(base, "artifacts");
  for (const platform of platforms) {
    const directory = join(artifacts, `gdcef-${platform.target}`);
    for (const name of platform.files) await file(join(directory, name));
    for (const name of platform.dirs) await mkdir(join(directory, name), { recursive: true });
  }
  const output = join(base, "output");
  for (const variant of ["full", "store"] as const) {
    await pack(artifacts, output, undefined, variant);
    await validate(output, variant);
    for (const platform of platforms) {
      assert.equal(await stat(join(output, "bin", platform.target)).then(() => true, () => false), includes(variant, platform.target));
    }
  }
  await mkdir(join(output, "bin/aarch64-unknown-linux-gnu"));
  await assert.rejects(validate(output, "store"), /Excluded/);
  await rm(join(output, "bin/aarch64-unknown-linux-gnu"), { recursive: true });
  await rm(join(output, "bin/x86_64-unknown-linux-gnu/libcef.so"));
  await assert.rejects(validate(output), /Missing file/);
  await assert.rejects(pack(artifacts, artifacts), /overlap/);
  await assert.rejects(pack(join(base, "absent"), output), /No platform/);
  await rm(join(artifacts, "gdcef-x86_64-pc-windows-msvc/locales"), { recursive: true });
  await file(join(artifacts, "gdcef-x86_64-pc-windows-msvc/locales"));
  await assert.rejects(pack(artifacts, output), /Missing directory/);
});

test("copy preserves executable modes and framework-relative symlinks", { skip: process.platform === "win32" }, async t => {
  const base = await temporary(t);
  const source = join(base, "source");
  await file(join(source, "Versions/A/library"));
  await chmod(join(source, "Versions/A/library"), 0o755);
  await symlink("A", join(source, "Versions/Current"));
  await symlink("Versions/Current/library", join(source, "library"));
  const output = join(base, "copy");
  await copy(source, output);
  assert.equal(await readlink(join(output, "library")), "Versions/Current/library");
  assert.equal(await readlink(join(output, "Versions/Current")), "A");
  assert.equal((await stat(join(output, "library"))).mode & 0o777, 0o755);
});

for (const target of ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"]) {
  test(`assembly stages ${target} without rebuilding or mutating cached artifacts`, async t => {
    const base = await temporary(t);
    const windows = target.endsWith("msvc");
    const config = configuration({ target, targetDir: join(base, "target") }, windows ? "win32" : "linux", "x64", {});
    const cef = join(base, "cef");
    const source = join(config.targetDir, target, "debug");
    const binaries = windows ? ["gdcef.dll", "gdcef_helper.exe"] : ["libgdcef.so", "gdcef_helper"];
    for (const name of binaries) await file(join(source, name));
    for (const name of windows ? windowsAssets : linuxAssets) await file(join(cef, name));
    await file(join(cef, "locales/en-US.pak"));
    const addon = join(base, "addon");
    const calls: string[] = [];
    await assemble(config, {
      env: { CEF_PATH: cef }, addonBin: join(addon, "bin"),
      command: (command, args) => {
        calls.push(command);
        assert.equal(command, target.startsWith("aarch64") ? "aarch64-linux-gnu-strip" : "strip");
        assert.ok(args[1].startsWith(join(addon, "bin")));
        writeFileSync(args[1], "stripped");
      },
    });
    await validate(addon);
    assert.equal(calls.length, windows ? 0 : 3);
    assert.equal(await readFile(join(cef, windows ? "libcef.dll" : "libcef.so"), "utf8"), "fixture");
    assert.equal(await readFile(join(source, binaries[0]), "utf8"), "fixture");
  });
}

test("macOS assembly preserves helper layout, plist metadata and inside-out signing order", async t => {
  const base = await temporary(t);
  const config = configuration({ targetDir: join(base, "target"), release: true }, "darwin", "arm64", {});
  for (const target of macTargets) {
    for (const name of ["gdcef_helper", "libgdcef.dylib"]) await file(join(config.targetDir, target, "release", name));
  }
  const env = { CEF_PATH_ARM64: join(base, "arm"), CEF_PATH_X64: join(base, "x64") };
  for (const cef of Object.values(env)) {
    const framework = join(cef, "Chromium Embedded Framework.framework");
    await file(join(framework, "Chromium Embedded Framework"));
    await file(join(framework, "Libraries/libEGL.dylib"));
    await file(join(framework, "Resources/icudtl.dat"));
  }
  const signed: string[] = [];
  let installNameSet = false;
  await assemble(config, { env, addonBin: join(base, "addon/bin"), command: (command, args) => {
    if (command === "lipo") copyFileSync(args[1], args[4]);
    else if (command === "plutil") {
      // Native plutil is exercised on macOS CI; other hosts inspect the JSON input.
      if (process.platform === "darwin") run(command, args);
      else JSON.parse(readFileSync(args[2], "utf8"));
    } else if (command === "install_name_tool") {
      assert.equal(args[1], "@rpath/Godot CEF.framework/libgdcef.dylib");
      installNameSet = true;
    } else if (command === "codesign") {
      if (args[3].endsWith("Godot CEF.framework")) assert.ok(installNameSet);
      signed.push(args[3]);
    } else assert.fail(`Unexpected command: ${command}`);
  } });
  assert.equal(signed.length, 11);
  assert.ok(signed[0].endsWith(join("Libraries", "libEGL.dylib")));
  assert.ok(signed[1].endsWith("(ARM64).framework"));
  assert.ok(signed[9].endsWith("Godot CEF.app"));
  assert.ok(signed[10].endsWith("Godot CEF.framework"));
  const framework = join(base, "addon/bin", macTarget, "Godot CEF.framework");
  const infoPath = join(framework, "Resources/Info.plist");
  const info = process.platform === "darwin"
    ? JSON.parse(spawnSync("plutil", ["-convert", "json", "-o", "-", infoPath], { encoding: "utf8" }).stdout)
    : JSON.parse(await readFile(infoPath, "utf8"));
  assert.equal(info.CFBundleExecutable, "libgdcef.dylib");
  assert.deepEqual(info.CFBundleSupportedPlatforms, ["MacOSX"]);
  for (const suffix of ["", " (GPU)", " (Renderer)", " (Plugin)", " (Alerts)"]) {
    const name = `Godot CEF Helper${suffix}`;
    await stat(join(framework, "Helpers/Godot CEF.app/Contents/Frameworks", `${name}.app`, "Contents/MacOS", name));
  }
  await validate(join(base, "addon"));
});

test("CLI emits only the runtime version on stdout and propagates command failures", () => {
  const cli = join(root, "scripts/build/main.ts");
  const env = { ...process.env };
  delete env.FORCE_COLOR;
  const result = spawnSync(process.execPath, [cli, "cef-version"], { cwd: tmpdir(), encoding: "utf8", env });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^\d+\.\d+\.\d+\r?\n$/);
  assert.equal(result.stderr, "");
  assert.equal(spawnSync(process.execPath, [cli, "pack", "--variant", "bad"]).status, 1);
  assert.throws(() => run(process.execPath, ["-e", "process.exit(7)"]), /failed \(7\)/);
});

test("mise bundle forwards flags and paths to build and then assemble", async t => {
  const base = await temporary(t);
  const config = await readFile(join(root, "mise.toml"), "utf8");
  await file(join(base, "mise.toml"), `[env]\n_.path = [${JSON.stringify(dirname(process.execPath))}]\n` + config.slice(config.indexOf("[tasks.build]"), config.indexOf("[tasks.pack]")));
  await file(join(base, "empty.toml"), "");
  await copy(join(root, "scripts/build/main.ts"), join(base, "scripts/build/main.ts"));
  await file(join(base, "scripts/build/package.json"), '{"type":"module"}');
  // Probe the actual CLI and mise graph without invoking compilers or modifying an addon.
  await file(join(base, "scripts/build/bundle.ts"), `
import { appendFileSync } from 'node:fs';
export const configuration = options => options;
export const build = options => appendFileSync(process.env.PROBE_LOG, JSON.stringify({ command: 'build', options }) + '\\n');
export const assemble = options => appendFileSync(process.env.PROBE_LOG, JSON.stringify({ command: 'assemble', options }) + '\\n');
`);
  const log = join(base, "calls.jsonl");
  const env = {
    ...process.env, MISE_TRUSTED_CONFIG_PATHS: base, MISE_AUTO_INSTALL: "0",
    MISE_GLOBAL_CONFIG_FILE: join(base, "empty.toml"), MISE_SYSTEM_CONFIG_FILE: join(base, "empty.toml"),
    PROBE_LOG: log,
  };
  for (const { release, target, targetDir } of [
    { release: false, target: "host", targetDir: "" },
    { release: true, target: "host", targetDir: "" },
    { release: true, target: "aarch64-unknown-linux-gnu", targetDir: "" },
    { release: false, target: "aarch64-unknown-linux-gnu", targetDir: join(base, "output with spaces") },
    { release: true, target: "aarch64-unknown-linux-gnu", targetDir: join(base, "output with spaces") },
  ]) {
    await writeFile(log, "");
    const args = ["run", "bundle", ...(release ? ["--release"] : []),
      ...(target === "host" ? [] : ["--target", target]), ...(targetDir ? ["--target-dir", targetDir] : [])];
    const result = spawnSync("mise", args, { cwd: base, env, encoding: "utf8" });
    assert.equal(result.status, 0, result.error?.message ?? result.stderr);
    const calls = (await readFile(log, "utf8")).trim().split("\n").map(line => JSON.parse(line));
    assert.deepEqual(calls, ["build", "assemble"].map(command => ({
      command, options: { release, target, targetDir },
    })));
  }
});
