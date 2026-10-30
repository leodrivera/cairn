// The folder as a windowed gallery. Thumbnails, when the setting is on, are the original images, loaded lazily.
import { useRef } from "preact/hooks";
import { apiUrl, join, segmentOf } from "../lib/paths";
import { formatSize } from "../lib/format";
import { gridRange, useScroller } from "../lib/windowing";
import { t } from "../i18n/index";
import { below, EntryName, menuKey, toggleClick, useLongPress } from "./file-list";
import { FileIcon } from "./file-icon";
import { Icon } from "./icons";
import type { Entry } from "../lib/types";
import type { Selection } from "../state/selection";

export const TILE_WIDTH = 168;
export const TILE_HEIGHT = 200;
const GAP = 16;

type Props = {
  dir: string;
  entries: Entry[];
  selection: Selection;
  thumbnails: boolean;
  onSelect(e: Entry, ev: MouseEvent): void;
  onMore?(e: Entry, at: { x: number; y: number }): void;
  onLongPress?(e: Entry): void;
};

export function FileGrid({ dir, entries, selection, thumbnails, onSelect, onMore, onLongPress }: Props) {
  const pressOf = useLongPress(onLongPress);
  const scroller = useRef<HTMLDivElement>(null);
  const { top, height, width } = useScroller(scroller);
  const columns = Math.max(2, Math.floor(width / TILE_WIDTH));
  const rowHeight = TILE_HEIGHT + GAP;
  const { start, end } = gridRange(top, height, rowHeight, entries.length, columns);
  const rows = Math.ceil(entries.length / columns);
  const padTop = (start / columns) * rowHeight;
  const padBottom = Math.max(0, (rows - Math.ceil(end / columns)) * rowHeight);
  return (
    <div class="grid-scroll" ref={scroller}>
      <div style={{ height: `${padTop}px` }} aria-hidden="true" />
      <ul class="grid" style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }} aria-label={t("explorer.table")}>
        {entries.slice(start, end).map((e) => {
          const seg = segmentOf(e);
          const on = selection.has(seg);
          const { pressed, ...press } = pressOf(e);
          const image = thumbnails && !e.outside && e.kind !== "folder" && e.mime.startsWith("image/");
          return (
            <li
              key={seg}
              {...press}
              onClickCapture={toggleClick(e, onSelect, press.onClickCapture)}
              class={on ? "tile on" : "tile"}
              onContextMenu={(ev) => {
                if (!onMore) return;
                ev.preventDefault();
                if (pressed?.()) return;
                onMore(e, { x: ev.clientX, y: ev.clientY });
              }}
              onKeyDown={(ev) => {
                if (!onMore || !menuKey(ev)) return;
                ev.preventDefault();
                onMore(e, below(ev.currentTarget as HTMLElement));
              }}
            >
              <div class="thumb">
                {image ? (
                  <img src={apiUrl("raw", join(dir, seg))} loading="lazy" decoding="async" alt="" />
                ) : (
                  <FileIcon entry={e} size={40} />
                )}
                <button
                  type="button"
                  class="check-button tile-check"
                  aria-pressed={on}
                  aria-label={t("explorer.select", { name: e.name })}
                  onClick={(ev) => onSelect(e, ev)}
                >
                  <span class="box">{on && <Icon name="check" size={14} />}</span>
                </button>
              </div>
              <div class="tile-foot">
                <EntryName dir={dir} entry={e} />
                {e.kind !== "folder" && !e.outside && <span class="mono muted size">{formatSize(e.size)}</span>}
                {onMore && (
                  <button
                    type="button"
                    class="icon-button tile-more"
                    aria-label={t("explorer.more", { name: e.name })}
                    aria-haspopup="menu"
                    onClick={(ev) => onMore(e, below(ev.currentTarget as HTMLElement))}
                  >
                    <Icon name="more" />
                  </button>
                )}
              </div>
            </li>
          );
        })}
      </ul>
      <div style={{ height: `${padBottom}px` }} aria-hidden="true" />
    </div>
  );
}
