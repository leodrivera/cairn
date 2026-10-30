// Share links: every active link (an admin sees everyone's), with copy and delete.
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import { ApiFailure, offline } from "../lib/api";
import { copyText, deleteShare, expiryText, listShares, shareUrl } from "../lib/shares";
import { errorText, t, tn } from "../i18n/index";
import { Dialog } from "../components/dialogs";
import { Icon } from "../components/icons";
import { useNarrow } from "../components/shell";
import { StatePanel } from "../components/states";
import { showToast } from "../components/toast";
import { useUser } from "../state/session";
import type { Share } from "../lib/types";

/** The folder part of a link's path, `/` for the top. */
function folderOf(path: string): string {
  const cut = path.lastIndexOf("/");
  return cut <= 0 ? "/" : path.slice(0, cut);
}

export function Links() {
  const me = useUser();
  const narrow = useNarrow();
  const [shares, setShares] = useState<Share[] | null>(null);
  const [error, setError] = useState<ApiFailure | null>(null);
  const [deleting, setDeleting] = useState<Share | null>(null);

  const load = async () => {
    try {
      setShares(await listShares());
      setError(null);
    } catch (e) {
      setError(e instanceof ApiFailure ? e : offline());
    }
  };

  useEffect(() => {
    load();
  }, []);

  const copy = async (s: Share) => {
    showToast((await copyText(shareUrl(s))) ? t("share.copied") : t("share.copyFailed"));
  };

  const cells = (s: Share) => {
    const url = shareUrl(s);
    const expiry = expiryText(s.expiresAt);
    return {
      file: (
        <span class="link-file">
          <span class="link-name">{s.name}</span>
          <span class="muted mono">{folderOf(s.path)}</span>
          {s.owner.id !== me.id && <span class="muted">{t("links.sharedBy", { name: s.owner.name })}</span>}
        </span>
      ),
      link: (
        <a class="mono" href={url} target="_blank" rel="noopener noreferrer">
          /s/{s.id.slice(0, 8)}
        </a>
      ),
      expires: <span class={expiry.soon ? "soon" : undefined}>{expiry.text}</span>,
      protection: (
        <span class={s.hasPassword ? "chip on" : "chip"}>{s.hasPassword ? t("links.password") : t("links.open")}</span>
      ),
      opened: <span class="muted">{tn("links.times", s.hits)}</span>,
      actions: (
        <span class="row-actions">
          <button type="button" class="icon-button" aria-label={t("links.copyFor", { name: s.name })} onClick={() => copy(s)}>
            <Icon name="copy" />
          </button>
          <button
            type="button"
            class="icon-button"
            aria-label={t("links.deleteFor", { name: s.name })}
            onClick={() => setDeleting(s)}
          >
            <Icon name="trash" />
          </button>
        </span>
      ),
    };
  };

  return (
    <section class="settings-page">
      <header class="settings-head">
        <h1>{t("settings.links")}</h1>
        <p class="muted">{t("links.lead")}</p>
      </header>
      {error && (
        <StatePanel icon="alert" tone="danger" title={t("settings.links")} body={errorText(error)}>
          <button type="button" class="button primary" onClick={load}>
            {t("state.tryAgain")}
          </button>
        </StatePanel>
      )}
      {shares?.length === 0 && <StatePanel icon="link" title={t("links.empty")} body={t("links.emptyBody")} />}
      {shares && shares.length > 0 && (
        <div class="table-wrap links-list">
          {narrow ? (
            <ul class="links-stack">
              {shares.map((s) => {
                const c = cells(s);
                return (
                  <li key={s.id}>
                    <div class="links-stack-head">
                      {c.file}
                      {c.actions}
                    </div>
                    <div class="links-stack-meta">
                      {c.link}
                      {c.expires}
                      {c.protection}
                      {c.opened}
                    </div>
                  </li>
                );
              })}
            </ul>
          ) : (
            <table class="users-table">
              <thead>
                <tr>
                  <th scope="col">{t("links.file")}</th>
                  <th scope="col">{t("links.link")}</th>
                  <th scope="col">{t("links.expires")}</th>
                  <th scope="col">{t("links.protection")}</th>
                  <th scope="col">{t("links.opened")}</th>
                  <th scope="col">
                    <span class="visually-hidden">{t("explorer.actions")}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {shares.map((s) => {
                  const c = cells(s);
                  return (
                    <tr key={s.id}>
                      <td>{c.file}</td>
                      <td>{c.link}</td>
                      <td>{c.expires}</td>
                      <td>{c.protection}</td>
                      <td>{c.opened}</td>
                      <td>{c.actions}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          )}
        </div>
      )}
      {deleting && (
        <DeleteLink
          share={deleting}
          onDeleted={() => {
            showToast(t("share.deleted", { name: deleting.name }));
            load();
          }}
          onClose={() => setDeleting(null)}
        />
      )}
    </section>
  );
}

function DeleteLink({ share, onDeleted, onClose }: { share: Share; onDeleted(): void; onClose(): void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const cancel = useRef<HTMLButtonElement>(null);
  useLayoutEffect(() => cancel.current?.focus(), []);
  const confirm = async () => {
    setBusy(true);
    try {
      await deleteShare(share.id);
      onDeleted();
      onClose();
    } catch (e) {
      setError(errorText(e instanceof ApiFailure ? e : offline()));
      setBusy(false);
    }
  };
  return (
    <Dialog title={t("links.deleteTitle", { name: share.name })} onClose={onClose}>
      <p class="dialog-body">{t("links.deleteBody")}</p>
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
          {t("share.delete")}
        </button>
      </div>
    </Dialog>
  );
}
