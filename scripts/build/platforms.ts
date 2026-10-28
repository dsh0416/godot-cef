export type Variant = "full" | "store";
export interface Platform {
  target: string;
  files: readonly string[];
  dirs: readonly string[];
}
export const macTarget = "universal-apple-darwin";
export const macTargets = ["aarch64-apple-darwin", "x86_64-apple-darwin"] as const;
export const platforms: readonly Platform[] = [
  { target: macTarget, files: [], dirs: ["Godot CEF.framework"] },
  ...["x86_64", "aarch64"].map(arch => ({
    target: `${arch}-pc-windows-msvc`, files: ["gdcef.dll", "gdcef_helper.exe", "libcef.dll"], dirs: ["locales"],
  })),
  ...["x86_64", "aarch64"].map(arch => ({
    target: `${arch}-unknown-linux-gnu`, files: ["libgdcef.so", "gdcef_helper", "libcef.so"], dirs: ["locales"],
  })),
];
export const windowsAssets = [
  "libcef.dll", "chrome_elf.dll", "d3dcompiler_47.dll", "dxcompiler.dll",
  "vk_swiftshader.dll", "vk_swiftshader_icd.json", "vulkan-1.dll", "icudtl.dat", "resources.pak",
  "chrome_100_percent.pak", "chrome_200_percent.pak", "v8_context_snapshot.bin", "bootstrap.exe", "bootstrapc.exe",
];
export const linuxAssets = [
  "libcef.so", "libvk_swiftshader.so", "libvulkan.so.1", "vk_swiftshader_icd.json",
  "icudtl.dat", "resources.pak", "chrome_100_percent.pak", "chrome_200_percent.pak",
  "v8_context_snapshot.bin", "chrome-sandbox",
];
export function includes(variant: Variant, target: string): boolean {
  return variant === "full" || !["aarch64-pc-windows-msvc", "aarch64-unknown-linux-gnu"].includes(target);
}
export function variant(value = "full"): Variant {
  if (value !== "full" && value !== "store") throw new Error(`Unsupported variant: ${value}`);
  return value;
}
