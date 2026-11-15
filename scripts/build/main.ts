import { parseArgs } from "node:util";

async function main(): Promise<void> {
  const [command, ...args] = process.argv.slice(2);
  switch (command) {
    case "cef-version":
    case "validate-versions": {
      parseArgs({ args, options: {} });
      const { lockedCefVersion, validateVersions } = await import("./versions.ts");
      if (command === "cef-version") console.log((await lockedCefVersion()).runtime);
      else { await validateVersions(); console.error("Version validation complete"); }
      break;
    }
    case "build":
    case "assemble":
    case "bundle": {
      const { values } = parseArgs({ args, options: {
        release: { type: "boolean", short: "r" }, profile: { type: "string" },
        target: { type: "string" }, "target-dir": { type: "string" },
      } });
      if (values.profile && !["debug", "release"].includes(values.profile)) throw new Error("Profile must be debug or release");
      const { configuration, build, assemble } = await import("./bundle.ts");
      const config = configuration({
        release: values.release || values.profile === "release" || process.env.usage_release === "true",
        target: values.target ?? process.env.usage_target,
        targetDir: values["target-dir"] ?? process.env.usage_target_dir,
      });
      if (command !== "assemble") await build(config);
      if (command !== "build") await assemble(config);
      break;
    }
    case "pack": {
      const { values } = parseArgs({ args, options: {
        artifacts: { type: "string", short: "a" }, output: { type: "string", short: "o" },
        "addon-src": { type: "string" }, variant: { type: "string" },
      } });
      if (!values.artifacts || !values.output) throw new Error("pack requires --artifacts and --output");
      const { pack } = await import("./pack.ts");
      const { variant } = await import("./platforms.ts");
      await pack(values.artifacts, values.output, values["addon-src"], variant(values.variant));
      break;
    }
    case "platform-dir":
    case "stage-artifact": {
      const { values } = parseArgs({ args, options: { target: { type: "string" }, output: { type: "string" } } });
      if (!values.target) throw new Error(`${command} requires --target`);
      const { join, resolve } = await import("node:path");
      const { root, assertSeparate, copy } = await import("./common.ts");
      const { installManifest, platformDirectory } = await import("./install-manifest.ts");
      const { platforms } = await import("./platforms.ts");
      if (!platforms.some(platform => platform.target === values.target)) throw new Error("Unknown platform target");
      const source = join(root, "addons/godot_cef", platformDirectory(await installManifest(), values.target));
      if (command === "platform-dir") console.log(source);
      else {
        if (!values.output) throw new Error("stage-artifact requires --output");
        const output = resolve(values.output);
        assertSeparate(output, [source]);
        const { rm } = await import("node:fs/promises");
        await rm(output, { recursive: true, force: true });
        await copy(source, output);
      }
      break;
    }
    case "validate": {
      const { values } = parseArgs({ args, options: { addon: { type: "string" }, variant: { type: "string" } } });
      if (!values.addon) throw new Error("validate requires --addon");
      const { validate } = await import("./pack.ts");
      const { variant } = await import("./platforms.ts");
      await validate(values.addon, values.variant === undefined ? undefined : variant(values.variant));
      break;
    }
    default: throw new Error("Usage: node scripts/build/main.ts <cef-version|validate-versions|build|assemble|bundle|pack|validate|platform-dir|stage-artifact> [options]");
  }
}

try { await main(); }
catch (error) { console.error(error instanceof Error ? error.message : error); process.exitCode = 1; }
