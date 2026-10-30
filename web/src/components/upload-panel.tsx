// Upload progress: one row per file, Retry on failures, Replace or Skip when the name is taken.
// Rows have one fixed height and only the visible ones render, so a drop of 10 000 files stays light.
import { useRef, useState } from "preact/hooks";
import { errorText, t, tn } from "../i18n/index";
import type { UploadItem, UploadQueue } from "../lib/uploads";
import { useWindow } from "../lib/windowing";
import { useQueue } from "../state/uploads";
import { Icon } from "./icons";

export const UPLOAD_ROW = 64;

function folderName(dir: string): string {
  return dir === "" ? t("nav.myFiles") : dir;
}

function status(i: UploadItem): string {
  switch (i.state) {
    case "queued":
      return t("upload.queued");
    case "sending":
      return `${i.size > 0 ? Math.floor((i.sent / i.size) * 100) : 0}%`;
    case "done":
      return t("upload.done");
    case "failed":
      return t("upload.failed");
    case "cancelled":
      return t("upload.cancelled");
    case "exists":
      return t("upload.exists");
  }
}

function Row({ item: i, queue }: { item: UploadItem; queue: UploadQueue }) {
  const note = i.state === "failed" && i.error ? errorText(i.error) : i.state === "exists" ? t("upload.existsBody") : "";
  return (
    <li class={`upload-row is-${i.state}`} style={{ height: `${UPLOAD_ROW}px` }}>
      <div class="upload-line">
        <span class="upload-name" title={i.label}>
          {i.label}
        </span>
        <span class="upload-status mono">{status(i)}</span>
        {(i.state === "queued" || i.state === "sending") && (
          <button
            type="button"
            class="icon-button"
            aria-label={t("upload.cancelOne", { name: i.label })}
            onClick={() => queue.cancel(i.id)}
          >
            <Icon name="close" />
          </button>
        )}
        {i.state === "failed" && (
          <button type="button" class="button quiet" onClick={() => queue.retry(i.id)}>
            {t("upload.retry")}
          </button>
        )}
        {i.state === "exists" && (
          <>
            <button type="button" class="button quiet" onClick={() => queue.replace(i.id)}>
              {t("upload.replace")}
            </button>
            <button type="button" class="button quiet" onClick={() => queue.skip(i.id)}>
              {t("upload.skip")}
            </button>
          </>
        )}
      </div>
      {note ? (
        <span class="upload-error" title={note}>
          {note}
        </span>
      ) : (
        <progress max={i.size || 1} value={i.state === "done" ? i.size || 1 : i.sent} aria-label={i.label} />
      )}
    </li>
  );
}

export function UploadPanel({ queue }: { queue: UploadQueue }) {
  useQueue(queue);
  const [open, setOpen] = useState(true);
  const scroller = useRef<HTMLUListElement>(null);
  const items = queue.items;
  const { start, end, padTop, padBottom } = useWindow(scroller, items.length, UPLOAD_ROW);
  if (items.length === 0) return null;

  const c = queue.counts;
  const counted = c.total - c.cancelled;
  const busy = c.active > 0;
  const folder = folderName(items[items.length - 1].root);
  const title = busy
    ? t("upload.title", { i: Math.min(counted, c.done + c.failed + 1), n: counted })
    : c.failed > 0
      ? tn("upload.someFailed", c.failed, { total: counted })
      : tn("upload.finished", c.done, { folder });

  return (
    <section class="upload-panel" aria-label={t("upload.panel")}>
      <header class="upload-head">
        <h2 aria-live="polite">{title}</h2>
        <div class="upload-head-tools">
          <button
            type="button"
            class="icon-button"
            aria-label={open ? t("upload.hide") : t("upload.show")}
            aria-expanded={open}
            onClick={() => setOpen(!open)}
          >
            <Icon name={open ? "chevronDown" : "chevronUp"} />
          </button>
          {!busy && (
            <button type="button" class="icon-button" aria-label={t("upload.close")} onClick={() => queue.clearFinished()}>
              <Icon name="close" />
            </button>
          )}
        </div>
      </header>
      {open && (
        <ul class="upload-rows" ref={scroller}>
          <li style={{ height: `${padTop}px` }} aria-hidden="true" />
          {items.slice(start, end).map((i) => (
            <Row key={i.id} item={i} queue={queue} />
          ))}
          <li style={{ height: `${padBottom}px` }} aria-hidden="true" />
        </ul>
      )}
      <footer class="upload-foot">
        <span class="mono muted">{t("upload.to", { folder })}</span>
        {busy && (
          <button type="button" class="button quiet danger-text" onClick={() => queue.cancelAll()}>
            {t("upload.cancelAll")}
          </button>
        )}
      </footer>
    </section>
  );
}
