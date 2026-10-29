// Runs the front-end tests: bundles each src/**/*.test.ts(x) with esbuild, then runs them with node --test.
// Usage: node test.mjs [filter] — only test files whose path contains the filter.
import { build } from "esbuild";
import { readdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

const out = ".test-out";
const filter = process.argv[2] ?? "";
rmSync(out, { recursive: true, force: true });

const entries = readdirSync("src", { recursive: true })
  .map((p) => join("src", p))
  .filter((p) => /\.test\.tsx?$/.test(p) && p.includes(filter));
if (entries.length === 0) {
  console.error(`no test files match "${filter}"`);
  process.exit(1);
}

await build({
  entryPoints: entries,
  bundle: true,
  format: "esm",
  platform: "node",
  target: "node22",
  jsx: "automatic",
  jsxImportSource: "preact",
  outdir: out,
  outbase: "src",
  outExtension: { ".js": ".mjs" },
  loader: { ".css": "empty", ".svg": "text" },
  packages: "external",
  logLevel: "warning",
});

const files = readdirSync(out, { recursive: true })
  .map((p) => join(out, p))
  .filter((p) => p.endsWith(".test.mjs"));
const run = spawnSync(process.execPath, ["--test", "--test-reporter=dot", ...files], { stdio: "inherit" });
process.exit(run.status ?? 1);
