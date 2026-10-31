// Share links: every active link (an admin sees everyone's), with copy and delete.
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import { ApiFailure, offline } from "../lib/api";
import { displayPath } from "../lib/paths";
import { copyText, deleteShare, expiryText, listShares, shareUrl, type Expiry } from "../lib/shares";
import { deleteGrant, listGiven, updateGrant } from "../lib/grants";
import { errorText, t, tn } from "../i18n/index";
import { Dialog } from "../components/dialogs";
import { Icon } from "../components/icons";
import { useNarrow } from "../components/shell";
import { StatePanel } from "../components/states";
import { showToast } from "../components/toast";
import { useUser } from "../state/session";
import type { GivenGrant, Share } from "../lib/types";

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
          <span class="muted mono">{displayPath(folderOf(s.path))}</span>
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
      {shares?.some((s) => s.url === null) && <p class="hint links-notice">{t("share.noPublicUrl")}</p>}
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
      <Grants />
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

const expiries: { value: string; days: Expiry; label: "share.in1" | "share.in7" | "share.in30" | "share.never" }[] = [
  { value: "1", days: 1, label: "share.in1" },
  { value: "7", days: 7, label: "share.in7" },
  { value: "30", days: 30, label: "share.in30" },
  { value: "never", days: null, label: "share.never" },
];

/** Grants given: who may open what, with access, expiry and Stop sharing. An admin sees everyone's. */
function Grants() {
  const me = useUser();
  const narrow = useNarrow();
  const [grants, setGrants] = useState<GivenGrant[] | null>(null);
  const [error, setError] = useState<ApiFailure | null>(null);
  const [revoking, setRevoking] = useState<GivenGrant | null>(null);

  const load = async () => {
    try {
      setGrants(await listGiven());
      setError(null);
    } catch (e) {
      setError(e instanceof ApiFailure ? e : offline());
    }
  };

  useEffect(() => {
    load();
  }, []);

  const change = async (g: GivenGrant, patch: { permission?: "read" | "write"; expiresDays?: Expiry }) => {
    try {
      const saved = await updateGrant(g.id, patch);
      setGrants((list) => list && list.map((x) => (x.id === g.id ? saved : x)));
      showToast(t("grants.changed", { name: g.recipient.name }));
    } catch (e) {
      showToast(errorText(e instanceof ApiFailure ? e : offline()));
      load();
    }
  };

  if (error) {
    return (
      <p class="error-line" role="alert">
        {errorText(error)}
      </p>
    );
  }
  if (!grants || grants.length === 0) return null;

  const cells = (g: GivenGrant) => {
    const expiry = expiryText(g.expiresAt);
    const vars = { name: g.recipient.name, item: g.name };
    return {
      item: (
        <span class="link-file">
          <span class="link-name">{g.name}</span>
          <span class="muted mono">{displayPath(g.path)}</span>
          {g.kind === null && <span class="muted">{t("grants.gone")}</span>}
          {g.owner.id !== me.id && <span class="muted">{t("links.sharedBy", { name: g.owner.name })}</span>}
        </span>
      ),
      person: (
        <span class="link-file">
          <span>{g.recipient.name}</span>
          <span class="muted">{g.recipient.email}</span>
        </span>
      ),
      access: (
        <select
          class="input"
          aria-label={t("grants.accessFor", vars)}
          value={g.permission}
          onChange={(e) => change(g, { permission: e.currentTarget.value as "read" | "write" })}
        >
          <option value="read">{t("people.canView")}</option>
          <option value="write">{t("people.canEdit")}</option>
        </select>
      ),
      expires: (
        <select
          class="input"
          aria-label={t("grants.endsFor", vars)}
          value=""
          onChange={(e) => {
            const pick = expiries.find((x) => x.value === e.currentTarget.value);
            if (pick) change(g, { expiresDays: pick.days });
          }}
        >
          <option value="">{t("grants.keep", { when: expiry.text })}</option>
          {expiries.map((x) => (
            <option value={x.value}>{t(x.label)}</option>
          ))}
        </select>
      ),
      actions: (
        <span class="row-actions">
          <button type="button" class="icon-button" aria-label={t("grants.revokeFor", vars)} onClick={() => setRevoking(g)}>
            <Icon name="trash" />
          </button>
        </span>
      ),
    };
  };

  return (
    <>
      <header class="settings-head grants-head">
        <h2>{t("grants.title")}</h2>
        <p class="muted">{t("grants.lead")}</p>
      </header>
      <div class="table-wrap grants-list">
        {narrow ? (
          <ul class="links-stack">
            {grants.map((g) => {
              const c = cells(g);
              return (
                <li key={g.id}>
                  <div class="links-stack-head">
                    {c.item}
                    {c.actions}
                  </div>
                  <div class="links-stack-meta">
                    {c.person}
                    {c.access}
                    {c.expires}
                  </div>
                </li>
              );
            })}
          </ul>
        ) : (
          <table class="users-table">
            <thead>
              <tr>
                <th scope="col">{t("grants.item")}</th>
                <th scope="col">{t("grants.person")}</th>
                <th scope="col">{t("grants.access")}</th>
                <th scope="col">{t("grants.expires")}</th>
                <th scope="col">
                  <span class="visually-hidden">{t("explorer.actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {grants.map((g) => {
                const c = cells(g);
                return (
                  <tr key={g.id}>
                    <td>{c.item}</td>
                    <td>{c.person}</td>
                    <td>{c.access}</td>
                    <td>{c.expires}</td>
                    <td>{c.actions}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
      {revoking && (
        <Revoke
          grant={revoking}
          onDone={() => {
            showToast(t("grants.revoked", { name: revoking.recipient.name, item: revoking.name }));
            load();
          }}
          onClose={() => setRevoking(null)}
        />
      )}
    </>
  );
}

function Revoke({ grant, onDone, onClose }: { grant: GivenGrant; onDone(): void; onClose(): void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const cancel = useRef<HTMLButtonElement>(null);
  useLayoutEffect(() => cancel.current?.focus(), []);
  const vars = { name: grant.recipient.name, item: grant.name };
  const confirm = async () => {
    setBusy(true);
    try {
      await deleteGrant(grant.id);
      onDone();
      onClose();
    } catch (e) {
      setError(errorText(e instanceof ApiFailure ? e : offline()));
      setBusy(false);
    }
  };
  return (
    <Dialog title={t("grants.revokeTitle", vars)} onClose={onClose}>
      <p class="dialog-body">{t("grants.revokeBody", vars)}</p>
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
          {t("grants.revoke")}
        </button>
      </div>
    </Dialog>
  );
}
