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

function tokens(theme: "dark" | "light"): Record<string, string> {
  const css = readFileSync("../tokens.css", "utf8");
  const block = theme === "dark" ? /:root,\s*:root\[data-theme="dark"\]\s*\{([^}]*)\}/ : /:root\[data-theme="light"\]\s*\{([^}]*)\}/;
  const body = css.match(block)![1];
  return Object.fromEntries([...body.matchAll(/(--[\w-]+):\s*(#[0-9a-f]{6})/gi)].map((m) => [m[1], m[2]]));
}

function luminance(hex: string): number {
  const c = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  const l = c.map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2];
}

function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}

test("row_menu_icon_is_visible", () => {
  for (const theme of ["dark", "light"] as const) {
    const t = tokens(theme);
    for (const bg of ["--cairn-bg", "--cairn-surface", "--cairn-raised"]) {
      const c = contrast(t["--cairn-muted"], t[bg]);
      assert.ok(c >= 3, `${theme}: muted on ${bg} is ${c.toFixed(2)}:1`);
    }
  }
  const css = readFileSync("src/app.css", "utf8");
  for (const m of css.matchAll(/([^{}]*\.(?:cell-more|tile-more)[^{}]*)\{([^}]*)\}/g)) {
    if (/:hover|:focus/.test(m[1])) continue;
    assert.ok(!/opacity|visibility\s*:\s*hidden|display\s*:\s*none/.test(m[2]), `${m[1].trim()} hides the button`);
  }
});

test("dialog_buttons_wrap", () => {
  const css = readFileSync("src/app.css", "utf8");
  const rule = css.match(/\.dialog-buttons\s*\{([^}]*)\}/)![1];
  assert.match(rule, /flex-wrap:\s*wrap/);
});
