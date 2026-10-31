// The trash: what was deleted from places the person may write to, with Restore and a permanent delete.
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import { ApiFailure, offline } from "../lib/api";
import { displayPath } from "../lib/paths";
import { formatSize, formatWhen } from "../lib/format";
import { deleteForever, emptyTrash, listTrash, restore } from "../lib/trash";
import { errorText, t, tn } from "../i18n/index";
import { Dialog } from "../components/dialogs";
import { FileIcon } from "../components/file-icon";
import { Icon } from "../components/icons";
import { useNarrow } from "../components/shell";
import { StatePanel } from "../components/states";
import { showToast } from "../components/toast";
import type { TrashEntry } from "../lib/types";

type Asking = { kind: "one"; entry: TrashEntry } | { kind: "empty"; count: number };

export function Trash() {
  const narrow = useNarrow();
  const [entries, setEntries] = useState<TrashEntry[] | null>(null);
  const [error, setError] = useState<ApiFailure | null>(null);
  const [asking, setAsking] = useState<Asking | null>(null);
  const [now] = useState(() => new Date());

  const load = async () => {
    try {
      setEntries(await listTrash());
      setError(null);
    } catch (e) {
      setError(e instanceof ApiFailure ? e : offline());
    }
  };

  useEffect(() => {
    load();
  }, []);

  const failed = (e: unknown) => showToast(errorText(e instanceof ApiFailure ? e : offline()));

  const doRestore = async (entry: TrashEntry) => {
    try {
      const r = await restore([entry.id]);
      if (r.failed.length) {
        showToast(errorText(new ApiFailure(0, r.failed[0].error, r.failed[0].message), { name: entry.name }));
      } else {
        showToast(tn("trash.restored", r.restored.length));
      }
      load();
    } catch (e) {
      failed(e);
    }
  };

  const deletable = entries?.filter((e) => e.canDelete) ?? [];

  const cells = (e: TrashEntry) => ({
    name: (
      <span class="trash-name">
        <FileIcon entry={{ name: e.name, kind: e.kind, size: e.size, modified: e.deletedAt, mime: "" }} />
        <span class="link-name">{e.name}</span>
      </span>
    ),
    from: <span class="muted mono">{displayPath(e.from)}</span>,
    by: <span class="muted">{e.deletedBy?.name ?? t("trash.nobody")}</span>,
    on: <span class="muted">{formatWhen(e.deletedAt, now)}</span>,
    size: <span class="muted">{formatSize(e.size)}</span>,
    actions: (
      <span class="row-actions">
        <button type="button" class="button quiet" aria-label={t("trash.restoreFor", { name: e.name })} onClick={() => doRestore(e)}>
          {t("trash.restore")}
        </button>
        {e.canDelete && (
          <button
            type="button"
            class="icon-button"
            aria-label={t("trash.deleteFor", { name: e.name })}
            onClick={() => setAsking({ kind: "one", entry: e })}
          >
            <Icon name="trash" />
          </button>
        )}
      </span>
    ),
  });

  return (
    <section class="settings-page">
      <header class="settings-head trash-head">
        <div>
          <h1>{t("nav.trash")}</h1>
          <p class="muted">{t("trash.lead")}</p>
        </div>
        {deletable.length > 0 && (
          <button type="button" class="button danger-text" onClick={() => setAsking({ kind: "empty", count: deletable.length })}>
            {t("trash.emptyTrash")}
          </button>
        )}
      </header>
      {error && (
        <StatePanel icon="alert" tone="danger" title={t("nav.trash")} body={errorText(error)}>
          <button type="button" class="button primary" onClick={load}>
            {t("state.tryAgain")}
          </button>
        </StatePanel>
      )}
      {entries?.length === 0 && <StatePanel icon="trash" title={t("trash.empty")} body={t("trash.emptyBody")} />}
      {entries && entries.length > 0 && (
        <div class="table-wrap trash-list">
          {narrow ? (
            <ul class="links-stack">
              {entries.map((e) => {
                const c = cells(e);
                return (
                  <li key={e.id}>
                    <div class="links-stack-head">
                      {c.name}
                      {c.actions}
                    </div>
                    <div class="links-stack-meta">
                      {c.from}
                      {c.by}
                      {c.on}
                      {c.size}
                    </div>
                  </li>
                );
              })}
            </ul>
          ) : (
            <table class="users-table">
              <thead>
                <tr>
                  <th scope="col">{t("trash.name")}</th>
                  <th scope="col">{t("trash.from")}</th>
                  <th scope="col">{t("trash.by")}</th>
                  <th scope="col">{t("trash.on")}</th>
                  <th scope="col">{t("trash.size")}</th>
                  <th scope="col">
                    <span class="visually-hidden">{t("explorer.actions")}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {entries.map((e) => {
                  const c = cells(e);
                  return (
                    <tr key={e.id}>
                      <td>{c.name}</td>
                      <td>{c.from}</td>
                      <td>{c.by}</td>
                      <td>{c.on}</td>
                      <td>{c.size}</td>
                      <td>{c.actions}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
      )}
      {asking && (
        <Forever
          asking={asking}
          onDone={(n) => {
            showToast(tn("trash.deleted", n));
            load();
          }}
          onClose={() => setAsking(null)}
        />
      )}
    </section>
  );
}

function Forever({ asking, onDone, onClose }: { asking: Asking; onDone(n: number): void; onClose(): void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const cancel = useRef<HTMLButtonElement>(null);
  useLayoutEffect(() => cancel.current?.focus(), []);
  const confirm = async () => {
    setBusy(true);
    try {
      const n =
        asking.kind === "one" ? (await deleteForever([asking.entry.id])).deleted : (await emptyTrash()).deleted;
      onDone(n);
      onClose();
    } catch (e) {
      setError(errorText(e instanceof ApiFailure ? e : offline()));
      setBusy(false);
    }
  };
  const title = asking.kind === "one" ? t("trash.foreverTitle", { name: asking.entry.name }) : t("trash.emptyTitle");
  const body = asking.kind === "one" ? t("trash.foreverBody") : tn("trash.emptyConfirm", asking.count);
  return (
    <Dialog title={title} onClose={onClose}>
      <p class="dialog-body">{body}</p>
      {error && (
        <p class="error-line" role="alert">
          {error}
        </p>
      )}
      <div class="dialog-buttons">
        <button type="button" class="button" ref={cancel} onClick={onClose}>
          {t("dialog.cancel")}
        </button>
        <button type="button" class="button danger" disabled={busy} onClick={confirm}>
          {asking.kind === "one" ? t("dialog.deleteForever") : t("trash.emptyTrash")}
        </button>
      </div>
    </Dialog>
  );
}
