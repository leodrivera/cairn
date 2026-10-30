// The visual rules, as a check over the sources: flat surfaces, system fonts, and no motion but an opacity fade.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const files = readdirSync("src", { recursive: true })
  .map((p) => join("src", String(p)))
  .filter((p) => /\.(css|tsx)$/.test(p));

const banned: [RegExp, string][] = [
  [/box-shadow/, "box-shadow"],
  [/text-shadow/, "text-shadow"],
  [/blur\(/, "blur("],
  [/gradient\(/, "gradient("],
  [/@font-face/, "@font-face"],
  [/fonts\.googleapis|fonts\.gstatic/, "a web font"],
];

test("no shadows, blur, gradients or web fonts", () => {
  const found: string[] = [];
  for (const f of files) {
    const text = readFileSync(f, "utf8");
    for (const [re, what] of banned) if (re.test(text)) found.push(`${f}: ${what}`);
  }
  assert.deepEqual(found, []);
});

test("transitions and animations move opacity only", () => {
  const found: string[] = [];
  for (const f of files) {
    const text = readFileSync(f, "utf8");
    for (const m of text.matchAll(/transition(?:-property)?\s*:\s*([^;}"]+)/g)) {
      const props = m[1].split(",").map((p) => p.trim().split(/\s+/)[0]);
      if (props.some((p) => p !== "opacity" && p !== "none")) found.push(`${f}: transition: ${m[1].trim()}`);
    }
    for (const m of text.matchAll(/@keyframes\s+[\w-]+\s*\{([\s\S]*?\})\s*\}/g)) {
      const props = [...m[1].matchAll(/([\w-]+)\s*:/g)].map((x) => x[1]);
      if (props.some((p) => p !== "opacity")) found.push(`${f}: keyframes move ${props.join(", ")}`);
    }
  }
  assert.deepEqual(found, []);
});

test("every animation stops under prefers-reduced-motion", () => {
  const css = files.filter((f) => f.endsWith(".css")).map((f) => readFileSync(f, "utf8")).join("\n");
  const animated = [...css.matchAll(/([^{}]+)\{[^{}]*animation\s*:(?!\s*none)[^;]+;/g)].map((m) => m[1].trim());
  const reduced = [...css.matchAll(/@media \(prefers-reduced-motion: reduce\)\s*\{([\s\S]*?)\n\}/g)].map((m) => m[1]).join("\n");
  for (const sel of animated) assert.ok(reduced.includes(sel), `${sel} keeps its animation under reduced motion`);
});

test("a name in a list row is a 44 px target on phones too", () => {
  // On phones the name sits over its date; its ::after covers the whole name cell, which fills the 52 px row.
  const css = readFileSync("src/app.css", "utf8");
  const cell = /\.narrow \.cell-name\s*\{([^}]*)\}/.exec(css)?.[1] ?? "";
  assert.match(cell, /position:\s*relative/);
  assert.match(cell, /align-self:\s*stretch/, "the cell is as tall as the row, not just its text");
  assert.match(css, /\.narrow \.cell-name \.name::after\s*\{[^}]*position:\s*absolute;[^}]*inset:\s*0/);
});
