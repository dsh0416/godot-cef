import assert from "node:assert/strict";
import { chmod, mkdtemp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { gunzipSync } from "node:zlib";
import { helperArchive, locateArchive } from "./archive.ts";
import { installManifest, platformDirectory, writeReceipt, type InstalledPlatform } from "./install-manifest.ts";
import { pack, validate } from "./pack.ts";
import { platforms } from "./platforms.ts";

async function fixture(targets = ["x86_64-pc-windows-msvc", "aarch64-unknown-linux-gnu"]) {
  const directory = await mkdtemp(join(tmpdir(), "gdcef-pack-"));
  const artifacts = join(directory, "artifacts");
  const addon = join(directory, "source-addon");
  await mkdir(join(addon, "icons"), { recursive: true });
  await writeFile(join(addon, "icons/webview.png"), "image");
  await writeFile(join(addon, "icons/webview.png.import"), "local editor import metadata");
  const manifest = await installManifest();
  for (const target of targets) {
    const platform = platforms.find(item => item.target === target)!;
    const source = join(artifacts, `gdcef-${target}`);
    await mkdir(source, { recursive: true });
    const payload = join(directory, target, "payload");
    const helper = target.includes("apple") ? "Godot CEF.framework/Helpers/Godot CEF.app/Contents/MacOS/Godot CEF" : target.includes("windows") ? "gdcef_helper.exe" : "gdcef_helper";
    await mkdir(payload, { recursive: true });
    await mkdir(dirname(join(payload, helper)), { recursive: true });
    await writeFile(join(payload, helper), "compiled helper");
    const archivePath = join(directory, target, "helper.tar.gz");
    await helperArchive(payload, archivePath);
    const archive = await readFile(archivePath);
    const binary = Buffer.concat([Buffer.from("native-library"), archive, Buffer.from("signature")]);
    const versioned = platformDirectory(manifest, target);
    const record: InstalledPlatform = {
      target, directory: versioned, library: `${versioned}/${platform.library}`, bootstrap: `${versioned}/${platform.bootstrap}`,
      helper: locateArchive(binary, archive), files: [platform.library, helper, ...platform.files], dirs: [...platform.dirs],
    };
    for (const name of record.files) {
      await mkdir(dirname(join(source, name)), { recursive: true });
      await writeFile(join(source, name), "runtime content");
      if (name === "gdcef_helper" || name === "chrome-sandbox" || name.includes("/Contents/MacOS/")) await chmod(join(source, name), 0o755);
    }
    for (const name of record.dirs) await mkdir(join(source, name), { recursive: true });
    await writeFile(join(source, platform.library), binary);
    await writeFile(join(source, platform.bootstrap), "CEF-free bootstrap");
    await writeReceipt(source, manifest, record);
    await writeFile(join(source, "runtime-platform.json"), JSON.stringify({ ...manifest, platforms: [record] }));
    manifest.platforms.push(record);
  }
  return { directory, artifacts, addon, manifest };
}

test("helper archive is deterministic, executable, and discoverable in universal binaries", async t => {
  const directory = await mkdtemp(join(tmpdir(), "gdcef-tar-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const payload = join(directory, "payload");
  await mkdir(payload);
  await writeFile(join(payload, "gdcef_helper"), "helper bytes");
  const first = join(directory, "first.gz");
  const second = join(directory, "second.gz");
  assert.deepEqual(await helperArchive(payload, first), ["gdcef_helper"]);
  await helperArchive(payload, second);
  const archive = await readFile(first);
  assert.deepEqual(archive, await readFile(second));
  const tar = gunzipSync(archive);
  assert.equal(tar.subarray(0, 100).toString().replaceAll("\0", ""), "gdcef_helper");
  assert.equal(parseInt(tar.subarray(100, 108).toString(), 8), 0o755);
  assert.equal(tar.subarray(512, 524).toString(), "helper bytes");
  assert.equal(locateArchive(Buffer.concat([Buffer.alloc(21), archive, archive]), archive).offset, 21);
  assert.throws(() => locateArchive(Buffer.from("unembedded"), archive), /no matching embedded/);
});

test("Store ships both native libraries and embedded helpers, while Full remains offline", async t => {
  const item = await fixture();
  t.after(() => rm(item.directory, { recursive: true, force: true }));
  for (const variant of ["store", "full"] as const) {
    const output = join(item.directory, variant);
    await pack(item.artifacts, output, item.addon, variant);
    await validate(output);
    assert.deepEqual(await readdir(join(output, "icons")), ["webview.png"]);
    const names = await readdir(output);
    assert.equal(names.includes("godot_cef.gdextension"), variant === "full");
    for (const record of item.manifest.platforms) {
      const files = await readdir(join(output, record.directory));
      assert.equal(files.includes("gdcef_helper.exe") || files.includes("gdcef_helper"), variant === "full");
      assert.equal(files.includes("libcef.dll") || files.includes("libcef.so"), variant === "full");
      assert.equal(files.some(file => file.includes("bootstrap") && !file.endsWith(".exe")), true);
    }
    const metadata = JSON.parse(await readFile(join(output, "runtime-install.json"), "utf8"));
    assert.equal(metadata.platforms.some((record: InstalledPlatform) => record.target === "aarch64-unknown-linux-gnu"), true);
  }
});

test("Store validation rejects leaked standalone files and corrupt embedded helpers", async t => {
  const item = await fixture(["x86_64-pc-windows-msvc"]);
  t.after(() => rm(item.directory, { recursive: true, force: true }));
  const output = join(item.directory, "store");
  await pack(item.artifacts, output, item.addon, "store");
  const leaked = join(output, "unexpected.exe");
  await writeFile(leaked, "executable");
  await assert.rejects(validate(output), /Unexpected Store file/);
  await rm(leaked);
  const record = item.manifest.platforms[0];
  const libraryPath = join(output, record.library);
  const binary = await readFile(libraryPath);
  binary[record.helper.offset] ^= 1;
  await writeFile(libraryPath, binary);
  await assert.rejects(validate(output), /Embedded helper archive does not match/);
});

test("macOS Store preserves the core framework shell without nested executable apps or CEF", async t => {
  const item = await fixture(["universal-apple-darwin"]);
  t.after(() => rm(item.directory, { recursive: true, force: true }));
  const output = join(item.directory, "store");
  await pack(item.artifacts, output, item.addon, "store");
  await validate(output);
  const record = item.manifest.platforms[0];
  assert.deepEqual((await readdir(join(output, record.directory))).sort(), ["Godot CEF.framework", "libgdcef_bootstrap.dylib"]);
  assert.deepEqual((await readdir(join(output, record.directory, "Godot CEF.framework"))).sort(), ["Resources", "libgdcef.dylib"]);
  const descriptor = await readFile(join(output, "godot_cef.gdextension.in"), "utf8");
  assert.match(descriptor, /macos = "bin\/universal-apple-darwin\/[^\n]+\/Godot CEF\.framework"/);
});

test("artifact version/path failures preserve an existing output", async t => {
  const item = await fixture(["x86_64-pc-windows-msvc"]);
  t.after(() => rm(item.directory, { recursive: true, force: true }));
  const output = join(item.directory, "store");
  await mkdir(output);
  await writeFile(join(output, "keep.txt"), "existing output");
  const metadataPath = join(item.artifacts, "gdcef-x86_64-pc-windows-msvc/runtime-platform.json");
  const metadata = JSON.parse(await readFile(metadataPath, "utf8"));
  metadata.cef_version = "0.0.0";
  await writeFile(metadataPath, JSON.stringify(metadata));
  await assert.rejects(pack(item.artifacts, output, item.addon, "store"), /Artifact version mismatch/);
  metadata.cef_version = item.manifest.cef_version;
  metadata.platforms[0].files.push("../../escape.exe");
  await writeFile(metadataPath, JSON.stringify(metadata));
  await assert.rejects(pack(item.artifacts, output, item.addon, "store"), /Unsafe runtime path/);
  assert.equal(await readFile(join(output, "keep.txt"), "utf8"), "existing output");
});

test("offline packaging rejects a helper whose executable mode was lost", { skip: process.platform === "win32" }, async t => {
  const item = await fixture(["x86_64-unknown-linux-gnu"]);
  t.after(() => rm(item.directory, { recursive: true, force: true }));
  await chmod(join(item.artifacts, "gdcef-x86_64-unknown-linux-gnu/gdcef_helper"), 0o644);
  await assert.rejects(pack(item.artifacts, join(item.directory, "full"), item.addon, "full"), /lost its execute permission/);
});
