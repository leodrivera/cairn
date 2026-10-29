import { test } from "node:test";
import assert from "node:assert/strict";
import { caret, isEditable, languageLabel, lineEnding } from "./text";

test("editable types", () => {
  assert.ok(isEditable("text/plain", "notes.txt"));
  assert.ok(isEditable("text/x-sh", "backup.sh"));
  assert.ok(isEditable("application/json", "data.json"));
  assert.ok(isEditable("application/xml", "feed.xml"));
  assert.ok(isEditable("application/javascript", "app.js"));
  assert.ok(isEditable("application/octet-stream", "Makefile"));
  assert.ok(isEditable("application/octet-stream", ".env"));
  assert.ok(!isEditable("image/png", "cover.png"));
  assert.ok(!isEditable("application/gzip", "site-backup.tar.gz"));
  assert.ok(!isEditable("application/octet-stream", "disk.img"));
});

test("labels", () => {
  const cases: [string, string][] = [
    ["backup.sh", "Shell"],
    ["README.md", "Markdown"],
    ["data.json", "JSON"],
    ["main.rs", "Rust"],
    ["tool.py", "Python"],
    ["app.js", "JavaScript"],
    ["app.ts", "TypeScript"],
    ["compose.yaml", "YAML"],
    ["ci.yml", "YAML"],
    ["Cargo.toml", "TOML"],
    ["site.css", "CSS"],
    ["index.html", "HTML"],
    ["NOTES.TXT", "Plain text"],
    ["Makefile", "Plain text"],
    ["Report #1 (50%) ✓ café.SH", "Shell"],
  ];
  for (const [name, label] of cases) assert.equal(languageLabel(name), label, name);
});

test("crlf detected", () => {
  assert.equal(lineEnding("a\nb\n"), "LF");
  assert.equal(lineEnding("a\r\nb\r\n"), "CRLF");
  assert.equal(lineEnding(""), "LF");
});

test("caret math", () => {
  const text = "ab\ncafé\n\nx";
  assert.deepEqual(caret(text, 0), { line: 1, col: 1 });
  assert.deepEqual(caret(text, 2), { line: 1, col: 3 });
  assert.deepEqual(caret(text, 3), { line: 2, col: 1 });
  assert.deepEqual(caret(text, 7), { line: 2, col: 5 });
  assert.deepEqual(caret(text, 9), { line: 4, col: 1 });
  assert.deepEqual(caret("a\r\nb", 3), { line: 2, col: 1 });
});
