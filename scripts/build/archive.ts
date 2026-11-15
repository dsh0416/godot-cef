import { createHash } from "node:crypto";
import { lstat, readFile, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { gzipSync } from "node:zlib";

export interface EmbeddedArchive { offset: number; length: number; sha256: string }

function octal(header: Buffer, offset: number, length: number, value: number): void {
  const encoded = value.toString(8).padStart(length - 1, "0");
  if (encoded.length >= length) throw new Error("Helper archive exceeds USTAR numeric limits");
  header.write(encoded, offset, length - 1, "ascii");
}

/** Small deterministic USTAR writer. Helper payloads intentionally contain no symlinks. */
export async function helperArchive(directory: string, output: string): Promise<string[]> {
  const blocks: Buffer[] = [];
  const files: string[] = [];
  async function visit(relative = ""): Promise<void> {
    for (const name of (await readdir(join(directory, relative))).sort()) {
      const path = relative ? `${relative}/${name}` : name;
      const info = await lstat(join(directory, path));
      if (info.isSymbolicLink() || (!info.isDirectory() && !info.isFile())) {
        throw new Error(`Unsupported helper archive entry: ${path}`);
      }
      const header = Buffer.alloc(512);
      const archivePath = path + (info.isDirectory() ? "/" : "");
      const split = archivePath.length > 100 ? archivePath.lastIndexOf("/", archivePath.length - 2) : -1;
      const filename = split < 0 ? archivePath : archivePath.slice(split + 1);
      const prefix = split < 0 ? "" : archivePath.slice(0, split);
      if (Buffer.byteLength(filename) > 100 || Buffer.byteLength(prefix) > 155) {
        throw new Error(`Helper archive path exceeds USTAR limits: ${path}`);
      }
      header.write(filename, 0, 100, "utf8");
      // Windows stat does not retain an executable bit; native helper executables are explicit.
      const executable = path.endsWith(".exe") || path === "gdcef_helper" || path.includes("/MacOS/");
      octal(header, 100, 8, info.isDirectory() || executable ? 0o755 : 0o644);
      octal(header, 108, 8, 0); octal(header, 116, 8, 0);
      octal(header, 124, 12, info.isDirectory() ? 0 : info.size);
      octal(header, 136, 12, 0);
      header.fill(0x20, 148, 156);
      header[156] = info.isDirectory() ? 0x35 : 0x30;
      header.write("ustar\0", 257, 6, "ascii"); header.write("00", 263, 2, "ascii");
      header.write(prefix, 345, 155, "utf8");
      const checksum = header.reduce((total, byte) => total + byte, 0);
      header.write(checksum.toString(8).padStart(6, "0"), 148, 6, "ascii");
      header[154] = 0; header[155] = 0x20;
      blocks.push(header);
      if (info.isDirectory()) await visit(path);
      else {
        const contents = await readFile(join(directory, path));
        blocks.push(contents, Buffer.alloc((512 - contents.length % 512) % 512));
        files.push(path);
      }
    }
  }
  await visit();
  blocks.push(Buffer.alloc(1024));
  await writeFile(output, gzipSync(Buffer.concat(blocks), { level: 9 }));
  return files;
}

export function locateArchive(library: Buffer, archive: Buffer): EmbeddedArchive {
  const offset = library.indexOf(archive);
  if (archive.length === 0 || offset < 0) throw new Error("Native core has no matching embedded helper archive; rebuild with GDCEF_HELPER_ARCHIVE");
  // Universal macOS libraries contain the same archive in both slices; the first is sufficient.
  return { offset, length: archive.length, sha256: createHash("sha256").update(archive).digest("hex") };
}

export async function embeddedArchive(library: string, archive: string): Promise<EmbeddedArchive> {
  return locateArchive(await readFile(library), await readFile(archive));
}
