import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, type TestContext } from "node:test";
import { validateVersions } from "./versions.ts";

const revision = "bacdb41c17f30f4eca4a6f747586fbf7f280ab90";
const cef = "156.5.0+156.0.28";
const sourceError = /Source export-cef-dir must pin an upstream commit and select the exporter crate/;

function sourceConfig(
  tool = `{ version = "rev:${revision}", crate = "export-cef-dir" }`,
  alias: string | null = "cargo:tauri-apps/cef-rs",
): string {
  return `[tools]\nexport-cef-dir = ${tool}\n`
    + (alias === null ? "" : `\n[tool_alias]\nexport-cef-dir = "${alias}"\n`);
}

async function fixture(
  t: TestContext,
  mise: string,
  options: { packageVersion?: string; cefVersion?: string; sysVersion?: string } = {},
): Promise<string> {
  const base = await mkdtemp(join(tmpdir(), "godot-cef-versions-"));
  t.after(() => rm(base, { recursive: true, force: true }));
  await Promise.all([
    writeFile(join(base, "Cargo.toml"), '[workspace.package]\nversion = "2.1.3"\n'),
    writeFile(join(base, "package.json"), JSON.stringify({ version: options.packageVersion ?? "2.1.3" })),
    writeFile(join(base, "mise.toml"), mise),
    writeFile(join(base, "Cargo.lock"), [
      "version = 4",
      '[[package]]\nname = "cef"',
      `version = "${options.cefVersion ?? cef}"`,
      '[[package]]\nname = "cef-dll-sys"',
      `version = "${options.sysVersion ?? cef}"`,
    ].join("\n")),
  ]);
  return base;
}

for (const version of ["156.5.0", cef]) {
  test(`accepts registry exporter ${version}`, async t => {
    const base = await fixture(t, `[tools]\n"cargo:export-cef-dir" = "${version}"\n`);
    await validateVersions(base);
  });
}

test("accepts the pinned upstream exporter source and crate", async t => {
  await validateVersions(await fixture(t, sourceConfig()));
});

for (const version of ["156.4.0", "156.5.0+156.0.26"]) {
  test(`rejects registry exporter version mismatch ${version}`, async t => {
    const base = await fixture(t, `[tools]\n"cargo:export-cef-dir" = "${version}"\n`);
    await assert.rejects(validateVersions(base), /cargo:export-cef-dir must match 156\.5\.0\+156\.0\.28/);
  });
}

for (const [name, alias] of [
  ["missing", null],
  ["different repository", "cargo:someone/cef-rs"],
  ["registry backend", "cargo:export-cef-dir"],
] as const) {
  test(`rejects source exporter with ${name} alias`, async t => {
    await assert.rejects(validateVersions(await fixture(t, sourceConfig(undefined, alias))), sourceError);
  });
}

for (const [name, tool] of [
  ["missing crate", `{ version = "rev:${revision}" }`],
  ["wrong crate", `{ version = "rev:${revision}", crate = "download-cef" }`],
  ["plain string", `"rev:${revision}"`],
  ["missing revision", '{ crate = "export-cef-dir" }'],
  ["numeric revision", '{ version = 156, crate = "export-cef-dir" }'],
] as const) {
  test(`rejects source exporter with ${name}`, async t => {
    await assert.rejects(validateVersions(await fixture(t, sourceConfig(tool))), sourceError);
  });
}

for (const version of ["latest", "branch:main", "tag:download-cef-v3.0.1", "rev:bacdb41", `rev:${"g".repeat(40)}`]) {
  test(`rejects unpinned or invalid source revision ${version}`, async t => {
    const mise = sourceConfig(`{ version = "${version}", crate = "export-cef-dir" }`);
    await assert.rejects(validateVersions(await fixture(t, mise)), sourceError);
  });
}

for (const [name, mise] of [
  ["registry", '[tools]\n"cargo:export-cef-dir" = "156.5.0"\n'],
  ["source", sourceConfig()],
] as const) {
  test(`${name} exporter still rejects different cef and cef-dll-sys versions`, async t => {
    const base = await fixture(t, mise, { sysVersion: "156.4.0+156.0.26" });
    await assert.rejects(validateVersions(base), /cef and cef-dll-sys versions must match/);
  });

  test(`${name} exporter still rejects missing CEF runtime metadata`, async t => {
    const base = await fixture(t, mise, { cefVersion: "156.5.0", sysVersion: "156.5.0" });
    await assert.rejects(validateVersions(base), /CEF version must contain numeric core\+runtime versions/);
  });

  test(`${name} exporter still rejects a workspace package version mismatch`, async t => {
    const base = await fixture(t, mise, { packageVersion: "2.1.2" });
    await assert.rejects(validateVersions(base), /package.json version must match Cargo.toml workspace version/);
  });
}
