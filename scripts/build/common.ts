import { spawnSync } from "node:child_process";
import { cp, mkdir, stat } from "node:fs/promises";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const root = fileURLToPath(new URL("../../", import.meta.url));

export function run(command: string, args: string[], env = process.env): void {
  console.error(`Running: ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, { cwd: root, env, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} failed (${result.signal ?? result.status})`);
  }
}

export async function exists(path: string): Promise<boolean> {
  try { await stat(path); return true; }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return false;
    throw error;
  }
}

export async function copy(source: string, destination: string): Promise<void> {
  await mkdir(dirname(destination), { recursive: true });
  // Keep framework-relative symlinks and executable modes when staging artifacts.
  await cp(source, destination, { recursive: true, dereference: false, verbatimSymlinks: true });
}

export async function requirePaths(base: string, files: readonly string[], dirs: readonly string[]): Promise<void> {
  for (const [names, directory] of [[files, false], [dirs, true]] as const) {
    for (const name of names) {
      const path = resolve(base, name);
      const info = await stat(path).catch((error: NodeJS.ErrnoException) => {
        if (error.code !== "ENOENT") throw error;
        return undefined;
      });
      if (!info || (directory ? !info.isDirectory() : !info.isFile())) {
        throw new Error(`Missing ${directory ? "directory" : "file"}: ${path}`);
      }
    }
  }
}

export function contains(parent: string, child: string): boolean {
  const path = relative(resolve(parent), resolve(child));
  return path === "" || (!isAbsolute(path) && path !== ".." && !path.startsWith(`..${process.platform === "win32" ? "\\" : "/"}`));
}

export function assertSeparate(output: string, inputs: string[]): void {
  if (inputs.some(input => contains(output, input) || contains(input, output))) {
    throw new Error(`Output must not overlap input directories: ${output}`);
  }
}
