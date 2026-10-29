// The text editor: a plain textarea with a line-number gutter and a status bar. No highlighting, no library.
import { useLayoutEffect, useMemo, useRef, useState } from "preact/hooks";
import { caret, languageLabel, lineEnding } from "../lib/text";
import { t } from "../i18n/index";

export type SaveState = "saved" | "unsaved" | "saving";

type Props = {
  name: string;
  text: string;
  readOnly: boolean;
  /** Shown in the status bar; null for a file that cannot be saved. */
  save: SaveState | null;
  onInput(text: string): void;
};

export function Editor({ name, text, readOnly, save, onInput }: Props) {
  const area = useRef<HTMLTextAreaElement>(null);
  const gutter = useRef<HTMLPreElement>(null);
  const [offset, setOffset] = useState(0);
  const lines = useMemo(() => {
    let n = 1;
    for (let i = text.indexOf("\n"); i >= 0; i = text.indexOf("\n", i + 1)) n++;
    return Array.from({ length: n }, (_, i) => i + 1).join("\n");
  }, [text]);
  const pos = caret(text, Math.min(offset, text.length));
  // As an attribute: a property set from JSX does not always reach the markup, and "false" as a string is truthy.
  useLayoutEffect(() => area.current?.setAttribute("spellcheck", "false"), []);
  const track = () => setOffset(area.current?.selectionStart ?? 0);

  return (
    <div class="editor">
      <div class="editor-body">
        <pre ref={gutter} class="editor-gutter" aria-hidden="true">
          {lines}
        </pre>
        <textarea
          ref={area}
          class="editor-text"
          aria-label={t("editor.label", { name })}
          value={text}
          readOnly={readOnly}
          autocomplete="off"
          autocapitalize="off"
          wrap="off"
          onInput={(e) => {
            onInput(e.currentTarget.value);
            track();
          }}
          onScroll={(e) => {
            if (gutter.current) gutter.current.scrollTop = e.currentTarget.scrollTop;
          }}
          onKeyUp={track}
          onClick={track}
          onSelect={track}
        />
      </div>
      <div class="editor-status mono">
        <span>{t("editor.status", { language: languageLabel(name), eol: lineEnding(text) })}</span>
        <span>{t("editor.position", { l: pos.line, c: pos.col })}</span>
        {save && (
          <span class={save === "saved" ? "accent" : ""} aria-live="polite">
            {save === "saved" ? t("editor.saved") : save === "saving" ? t("editor.saving") : t("editor.unsaved")}
          </span>
        )}
      </div>
    </div>
  );
}
