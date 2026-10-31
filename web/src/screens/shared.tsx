// Shared with me: the grants this person received, and the place inside one.
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import { ApiFailure, offline } from "../lib/api";
import { deleteGrant, grantInfo, listReceived } from "../lib/grants";
import { expiryText } from "../lib/shares";
import { Link } from "../lib/router";
import { inGrant } from "../lib/where";
import { errorText, t } from "../i18n/index";
import { Dialog } from "../components/dialogs";
import { FileIcon } from "../components/file-icon";
import { Icon } from "../components/icons";
import { Shell, useNarrow } from "../components/shell";
import { StatePanel } from "../components/states";
import { showToast } from "../components/toast";
import { Explorer } from "./explorer";
import { Viewer } from "./viewer";
import type { ReceivedGrant } from "../lib/types";

export function SharedWithMe() {
  const narrow = useNarrow();
  const [grants, setGrants] = useState<ReceivedGrant[] | null>(null);
  const [error, setError] = useState<ApiFailure | null>(null);
  const [leaving, setLeaving] = useState<ReceivedGrant | null>(null);

  const load = async () => {
    try {
      setGrants(await listReceived());
      setError(null);
    } catch (e) {
      setError(e instanceof ApiFailure ? e : offline());
    }
  };

  useEffect(() => {
    load();
  }, []);

  const cells = (g: ReceivedGrant) => {
    const expiry = expiryText(g.expiresAt);
    return {
      name: (
        <span class="trash-name">
          <FileIcon entry={{ name: g.name, kind: g.kind, size: 0, modified: g.createdAt, mime: "" }} />
          <span class="link-file">
            <Link class="link-name" href={`/shared/${g.id}`}>
              {g.name}
            </Link>
            <span class="muted">{t("shared.by", { name: g.owner.name })}</span>
          </span>
        </span>
      ),
      access: <span class="chip">{g.effective === "write" ? t("people.canEdit") : t("people.canView")}</span>,
      expires: <span class={expiry.soon ? "soon" : undefined}>{expiry.text}</span>,
      actions: (
        <span class="row-actions">
          <button
            type="button"
            class="button quiet"
            aria-label={t("shared.leaveFor", { name: g.name })}
            onClick={() => setLeaving(g)}
          >
            {t("shared.leave")}
          </button>
        </span>
      ),
    };
  };

  return (
    <section class="settings-page">
      <header class="settings-head">
        <h1>{t("nav.sharedWithMe")}</h1>
        <p class="muted">{t("shared.lead")}</p>
      </header>
      {error && (
        <StatePanel icon="alert" tone="danger" title={t("nav.sharedWithMe")} body={errorText(error)}>
          <button type="button" class="button primary" onClick={load}>
            {t("state.tryAgain")}
          </button>
        </StatePanel>
      )}
      {grants?.length === 0 && <StatePanel icon="users" title={t("shared.empty")} body={t("shared.emptyBody")} />}
      {grants && grants.length > 0 && (
        <div class="table-wrap shared-list">
          {narrow ? (
            <ul class="links-stack">
              {grants.map((g) => {
                const c = cells(g);
                return (
                  <li key={g.id}>
                    <div class="links-stack-head">
                      {c.name}
                      {c.actions}
                    </div>
                    <div class="links-stack-meta">
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
                  <th scope="col">{t("shared.name")}</th>
                  <th scope="col">{t("shared.access")}</th>
                  <th scope="col">{t("shared.expires")}</th>
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
                      <td>{c.name}</td>
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
      )}
      {leaving && (
        <Leave
          grant={leaving}
          onLeft={() => {
            showToast(t("shared.left", { name: leaving.name }));
            load();
          }}
          onClose={() => setLeaving(null)}
        />
      )}
    </section>
  );
}

function Leave({ grant, onLeft, onClose }: { grant: ReceivedGrant; onLeft(): void; onClose(): void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const cancel = useRef<HTMLButtonElement>(null);
  useLayoutEffect(() => cancel.current?.focus(), []);
  const confirm = async () => {
    setBusy(true);
    try {
      await deleteGrant(grant.id);
      onLeft();
      onClose();
    } catch (e) {
      setError(errorText(e instanceof ApiFailure ? e : offline()));
      setBusy(false);
    }
  };
  return (
    <Dialog title={t("shared.leaveTitle", { name: grant.name })} onClose={onClose}>
      <p class="dialog-body">{t("shared.leaveBody", { owner: grant.owner.name })}</p>
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
          {t("shared.leave")}
        </button>
      </div>
    </Dialog>
  );
}

/** Inside a grant: its folder in the explorer, or its file (or a file in its folder) in the viewer. */
export function GrantPlace({ id, path, view }: { id: string; path: string; view: boolean }) {
  const [grant, setGrant] = useState<ReceivedGrant | null>(null);
  const [error, setError] = useState<ApiFailure | null>(null);

  useEffect(() => {
    let live = true;
    setGrant(null);
    setError(null);
    grantInfo(id).then(
      (g) => live && setGrant(g),
      (e) => live && setError(e instanceof ApiFailure ? e : offline()),
    );
    return () => {
      live = false;
    };
  }, [id]);

  if (error) {
    const gone = error.code === "grant_gone" || error.code === "not_found";
    return (
      <Shell>
        <StatePanel
          icon={gone ? "users" : "alert"}
          tone={gone ? undefined : "danger"}
          title={gone ? t("error.grant_gone") : t("nav.sharedWithMe")}
          body={gone ? t("shared.emptyBody") : errorText(error)}
        >
          <Link class="button" href="/shared">
            <Icon name="back" />
            {t("shared.back")}
          </Link>
        </StatePanel>
      </Shell>
    );
  }
  if (!grant) return <Shell>{null}</Shell>;
  const where = inGrant(grant);
  if (grant.kind === "file" || view) {
    return (
      <Shell>
        <Viewer path={grant.kind === "file" ? "" : path} where={where} />
      </Shell>
    );
  }
  return <Explorer path={path} where={where} />;
}
