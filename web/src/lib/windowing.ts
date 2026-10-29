// Windowed lists: only the rows on screen, plus a few either side, are in the DOM.
import type { RefObject } from "preact";
import { useEffect, useState } from "preact/hooks";

export function visibleRange(
  scrollTop: number,
  viewport: number,
  rowHeight: number,
  count: number,
  overscan = 8,
): { start: number; end: number } {
  if (count === 0) return { start: 0, end: 0 };
  const top = Math.max(0, scrollTop);
  const first = Math.floor(top / rowHeight);
  const start = Math.max(0, Math.min(count, first - overscan));
  const end = Math.min(count, Math.ceil((top + viewport) / rowHeight) + overscan);
  return { start, end: Math.max(start, end) };
}

/** Item indices of a grid with `columns` per row, by rows on screen plus `overscanRows` either side. */
export function gridRange(
  scrollTop: number,
  viewport: number,
  rowHeight: number,
  count: number,
  columns: number,
  overscanRows = 2,
): { start: number; end: number } {
  const rows = visibleRange(scrollTop, viewport, rowHeight, Math.ceil(count / columns), overscanRows);
  return { start: rows.start * columns, end: Math.min(count, rows.end * columns) };
}

/** Tracks the scroller's position and size; a scroller with no layout yet counts as one window. */
export function useScroller(ref: RefObject<HTMLElement | null>): { top: number; height: number; width: number } {
  const [box, setBox] = useState({ top: 0, height: 0, width: 0 });
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const update = () =>
      setBox({
        top: el.scrollTop,
        height: el.clientHeight || window.innerHeight,
        width: el.clientWidth || window.innerWidth,
      });
    update();
    el.addEventListener("scroll", update, { passive: true });
    const ro = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(update);
    ro?.observe(el);
    return () => {
      el.removeEventListener("scroll", update);
      ro?.disconnect();
    };
  }, [ref.current]);
  return box;
}

export function useWindow(ref: RefObject<HTMLElement | null>, count: number, rowHeight: number) {
  const { top, height } = useScroller(ref);
  const { start, end } = visibleRange(top, height, rowHeight, count);
  return { start, end, padTop: start * rowHeight, padBottom: (count - end) * rowHeight };
}
