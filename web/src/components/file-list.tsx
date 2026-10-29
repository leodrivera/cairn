// The folder as a windowed table: select, name, size, modified, more.
import { useRef } from "preact/hooks";
import { Link, href } from "../lib/router";
import { join, segmentOf } from "../lib/paths";
import { downloadUrl } from "../lib/actions";
import { isEditable } from "../lib/text";
import { formatSize, formatWhen } from "../lib/format";
import { useWindow } from "../lib/windowing";
import { t } from "../i18n/index";
import { FileIcon } from "./file-icon";
import { Icon } from "./icons";
import type { Entry } from "../lib/types";
import type { Selection } from "../state/selection";

export const ROW_HEIGHT = 52;

/** Text and images open in the viewer; everything else downloads. */
export function opensInViewer(e: Entry): boolean {
  return e.mime.startsWith("image/") || isEditable(e.mime, e.name);
}

/** Shift+F10 or the context-menu key opens an item's menu from the keyboard. */
export function menuKey(e: KeyboardEvent): boolean {
  return (e.key === "F10" && e.shiftKey) || e.key === "ContextMenu";
}

/** Where a menu opened from an element starts: under its left edge. */
export function below(el: HTMLElement): { x: number; y: number } {
  const r = el.getBoundingClientRect();
  return { x: r.left, y: r.bottom };
}

type Props = {
  dir: string;
  entries: Entry[];
  selection: Selection;
  onSelect(e: Entry, ev: MouseEvent): void;
  onMore?(e: Entry, at: { x: number; y: number }): void;
};

export function EntryName({ dir, entry }: { dir: string; entry: Entry }) {
  const path = join(dir, segmentOf(entry));
  if (entry.outside) {
    return (
      <span class="name-cell">
        <span class="name">{entry.name}</span>
        <span class="outside">{t("explorer.outside")}</span>
      </span>
    );
  }
  if (entry.kind === "folder") {
    return (
      <Link class="name" href={href({ name: "files", path })}>
        {entry.name}
      </Link>
    );
  }
  if (opensInViewer(entry)) {
    return (
      <Link class="name" href={href({ name: "view", path })}>
        {entry.name}
      </Link>
    );
  }
  return (
    <a class="name" href={downloadUrl(dir, [entry])!} download>
      {entry.name}
    </a>
  );
}

export function FileList({ dir, entries, selection, onSelect, onMore }: Props) {
  const scroller = useRef<HTMLDivElement>(null);
  const { start, end, padTop, padBottom } = useWindow(scroller, entries.length, ROW_HEIGHT);
  const now = new Date();
  return (
    <div class="list" role="table" aria-label={t("explorer.table")} aria-rowcount={entries.length + 1}>
      <div class="list-head" role="rowgroup">
        <div class="list-row head" role="row">
          <span role="columnheader" class="cell-check">
            <span class="visually-hidden">{t("explorer.selectColumn")}</span>
          </span>
          <span role="columnheader">{t("explorer.name")}</span>
          <span role="columnheader" class="cell-size">
            {t("explorer.size")}
          </span>
          <span role="columnheader" class="cell-when">
            {t("explorer.modified")}
          </span>
          <span role="columnheader" class="cell-more">
            <span class="visually-hidden">{t("explorer.actions")}</span>
          </span>
        </div>
      </div>
      <div class="list-body" role="rowgroup" ref={scroller}>
        <div style={{ height: `${padTop}px` }} aria-hidden="true" />
        {entries.slice(start, end).map((e, i) => {
          const seg = segmentOf(e);
          const on = selection.has(seg);
          return (
            <div
              key={seg}
              class={on ? "list-row on" : "list-row"}
              role="row"
              aria-rowindex={start + i + 2}
              aria-selected={on}
              onContextMenu={(ev) => {
                if (!onMore) return;
                ev.preventDefault();
                onMore(e, { x: ev.clientX, y: ev.clientY });
              }}
              onKeyDown={(ev) => {
                if (!onMore || !menuKey(ev)) return;
                ev.preventDefault();
                onMore(e, below(ev.currentTarget as HTMLElement));
              }}
            >
              <span role="cell" class="cell-check">
                <button
                  type="button"
                  class="check-button"
                  aria-pressed={on}
                  aria-label={t("explorer.select", { name: e.name })}
                  onClick={(ev) => onSelect(e, ev)}
                >
                  <span class="box">{on && <Icon name="check" size={14} />}</span>
                </button>
              </span>
              <span role="cell" class="cell-name">
                <FileIcon entry={e} />
                <EntryName dir={dir} entry={e} />
              </span>
              <span role="cell" class="cell-size mono">
                {e.kind === "folder" || e.outside ? "—" : formatSize(e.size)}
              </span>
              <span role="cell" class="cell-when">
                {formatWhen(e.modified, now)}
              </span>
              <span role="cell" class="cell-more">
                {onMore && (
                  <button
                    type="button"
                    class="icon-button"
                    aria-label={t("explorer.more", { name: e.name })}
                    aria-haspopup="menu"
                    onClick={(ev) => onMore(e, below(ev.currentTarget as HTMLElement))}
                  >
                    <Icon name="more" />
                  </button>
                )}
              </span>
            </div>
          );
        })}
        <div style={{ height: `${padBottom}px` }} aria-hidden="true" />
      </div>
    </div>
  );
}
