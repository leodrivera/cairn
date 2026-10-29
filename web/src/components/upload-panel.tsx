// Upload progress: one row per file, Retry on failures, Replace or Skip when the name is taken.
import { useState } from "preact/hooks";
import { errorText, t, tn } from "../i18n/index";
import type { UploadItem, UploadQueue } from "../lib/uploads";
import { useQueue } from "../state/uploads";
import { Icon } from "./icons";

const active = (i: UploadItem) => i.state === "queued" || i.state === "sending" || i.state === "exists";

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

export function UploadPanel({ queue }: { queue: UploadQueue }) {
  useQueue(queue);
  const [open, setOpen] = useState(true);
  const items = queue.items;
  if (items.length === 0) return null;

  const counted = items.filter((i) => i.state !== "cancelled");
  const done = counted.filter((i) => i.state === "done").length;
  const failed = counted.filter((i) => i.state === "failed").length;
  const busy = items.some(active);
  const folder = folderName(items[items.length - 1].root);
  const title = busy
    ? t("upload.title", { i: Math.min(counted.length, done + failed + 1), n: counted.length })
    : failed > 0
      ? tn("upload.someFailed", failed, { total: counted.length })
      : tn("upload.finished", done, { folder });

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
        <ul class="upload-rows">
          {items.map((i) => (
            <li key={i.id} class={`upload-row is-${i.state}`}>
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
              </div>
              <progress max={i.size || 1} value={i.state === "done" ? i.size || 1 : i.sent} aria-label={i.label} />
              {i.state === "failed" && i.error && (
                <div class="upload-line">
                  <span class="upload-error">{errorText(i.error)}</span>
                  <button type="button" class="button quiet" onClick={() => queue.retry(i.id)}>
                    {t("upload.retry")}
                  </button>
                </div>
              )}
              {i.state === "exists" && (
                <div class="upload-line">
                  <span class="upload-error">{t("upload.existsBody")}</span>
                  <span>
                    <button type="button" class="button quiet" onClick={() => queue.replace(i.id)}>
                      {t("upload.replace")}
                    </button>
                    <button type="button" class="button quiet" onClick={() => queue.skip(i.id)}>
                      {t("upload.skip")}
                    </button>
                  </span>
                </div>
              )}
            </li>
          ))}
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
