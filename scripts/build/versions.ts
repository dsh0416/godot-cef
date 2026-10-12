import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { parse } from "smol-toml";
import { root } from "./common.ts";

const numericVersion = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
export function cefVersion(lock: string): { core: string; runtime: string } {
  const packages = parse(lock).package;
  if (!Array.isArray(packages)) throw new Error("Cargo.lock has no package tables");
  const versions = ["cef", "cef-dll-sys"].map(name => {
    const matches = packages.filter(pkg => typeof pkg === "object" && pkg !== null && "name" in pkg && pkg.name === name);
    if (matches.length !== 1) throw new Error(`Cargo.lock must contain exactly one ${name} package`);
    const pkg = matches[0];
    if (typeof pkg !== "object" || pkg === null || !("version" in pkg) || typeof pkg.version !== "string") {
      throw new Error(`Missing ${name} version`);
    }
    return pkg.version;
  });
  if (versions[0] !== versions[1]) throw new Error("cef and cef-dll-sys versions must match");
  const parts = versions[0].split("+");
  if (parts.length !== 2 || !parts.every(part => numericVersion.test(part))) {
    throw new Error("CEF version must contain numeric core+runtime versions");
  }
  return { core: parts[0], runtime: parts[1] };
}

export async function lockedCefVersion(base = root) {
  return cefVersion(await readFile(join(base, "Cargo.lock"), "utf8"));
}

export async function validateVersions(base = root): Promise<void> {
  const cargo = parse(await readFile(join(base, "Cargo.toml"), "utf8")) as {
    workspace: { package: { version: string } };
  };
  const pkg = JSON.parse(await readFile(join(base, "package.json"), "utf8"));
  if (typeof pkg.version !== "string" || pkg.version !== cargo.workspace.package.version) {
    throw new Error("package.json version must match Cargo.toml workspace version");
  }
  const mise = parse(await readFile(join(base, "mise.toml"), "utf8")) as { tools: Record<string, unknown> };
  const exporter = mise.tools["cargo:export-cef-dir"];
  const { core, runtime } = await lockedCefVersion(base);
  if (exporter !== core && exporter !== `${core}+${runtime}`) {
    throw new Error(`mise.toml cargo:export-cef-dir must match ${core}+${runtime}`);
  }
}
