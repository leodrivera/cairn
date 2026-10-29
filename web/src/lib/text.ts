// Text files in the browser: which ones open in the editor, how they are labelled, and where the caret is.
import { t } from "../i18n/index";

function extension(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

/** Text by MIME type, plus names without an extension (Makefile) and dotfiles (.env). */
export function isEditable(mime: string, name: string): boolean {
  if (mime.startsWith("text/")) return true;
  if (/^application\/(json|xml|javascript)$/.test(mime)) return true;
  return name.startsWith(".") || !name.includes(".");
}

const languages: Record<string, string> = {
  sh: "Shell",
  md: "Markdown",
  json: "JSON",
  rs: "Rust",
  py: "Python",
  js: "JavaScript",
  ts: "TypeScript",
  yaml: "YAML",
  yml: "YAML",
  toml: "TOML",
  css: "CSS",
  html: "HTML",
};

export function languageLabel(name: string): string {
  return languages[extension(name)] ?? t("editor.plain");
}

/** CRLF when the first line break is one; LF otherwise. */
export function lineEnding(text: string): "LF" | "CRLF" {
  const i = text.indexOf("\n");
  return i > 0 && text[i - 1] === "\r" ? "CRLF" : "LF";
}

/** 1-based line and column of a caret offset (UTF-16 units, as textareas count). */
export function caret(text: string, offset: number): { line: number; col: number } {
  let line = 1;
  let start = 0;
  for (let i = text.indexOf("\n"); i >= 0 && i < offset; i = text.indexOf("\n", i + 1)) {
    line++;
    start = i + 1;
  }
  return { line, col: offset - start + 1 };
}
