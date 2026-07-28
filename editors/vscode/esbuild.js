// Bundles the extension into a single file.
//
// This is not only an optimization: `vsce` enumerates dependencies with
// `npm list`, which cannot read pnpm's symlinked store, and upstream closed
// that request as not planned (microsoft/vscode-vsce#421). Bundling removes
// the question — `vsce package --no-dependencies` then ships one file.

const esbuild = require("esbuild");

const production = process.argv.includes("--production");
const watch = process.argv.includes("--watch");

/** @type {import("esbuild").BuildOptions} */
const options = {
  entryPoints: ["src/extension.ts"],
  bundle: true,
  format: "cjs",
  platform: "node",
  outfile: "dist/extension.js",
  // Supplied by the editor at runtime, never bundled.
  external: ["vscode"],
  minify: production,
  sourcemap: !production,
  sourcesContent: false,
  logLevel: "warning",
};

async function main() {
  if (watch) {
    const ctx = await esbuild.context(options);
    await ctx.watch();
    return;
  }
  await esbuild.build(options);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
