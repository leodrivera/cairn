// Builds the front-end into dist/, which the Rust binary embeds. Hashed names under assets/ are cached for a year.
import { build } from "esbuild";
import { copyFileSync, readFileSync, rmSync, writeFileSync } from "node:fs";

const dist = "dist";
rmSync(dist, { recursive: true, force: true });

const result = await build({
  entryPoints: ["src/main.tsx"],
  bundle: true,
  minify: true,
  format: "esm",
  target: "es2022",
  jsx: "automatic",
  jsxImportSource: "preact",
  outdir: dist,
  entryNames: "assets/app-[hash]",
  metafile: true,
  logLevel: "warning",
});

const outputs = Object.keys(result.metafile.outputs).map((p) => p.slice(dist.length + 1));
const pick = (ext) => {
  const found = outputs.filter((p) => p.endsWith(ext));
  if (found.length !== 1) throw new Error(`expected one ${ext} output, got ${found.join(", ") || "none"}`);
  return found[0];
};

const html = readFileSync("index.html", "utf8").replace("{{js}}", pick(".js")).replace("{{css}}", pick(".css"));
writeFileSync(`${dist}/index.html`, html);

for (const name of ["favicon.svg", "favicon.ico", "apple-touch-icon.png", "icon-192.png", "icon-512.png"]) {
  copyFileSync(`../brand/${name}`, `${dist}/${name}`);
}
