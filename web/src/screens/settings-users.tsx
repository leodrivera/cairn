// Users: everyone with an account, their scope and permissions; new, edit and delete.
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import { api, ApiFailure, offline } from "../lib/api";
import { errorText, t } from "../i18n/index";
import { Dialog } from "../components/dialogs";
import { Icon } from "../components/icons";
import { initials } from "../components/shell";
import { StatePanel } from "../components/states";
import { showToast } from "../components/toast";
import { UserForm } from "../components/user-form";
import { useSession } from "../state/session";
import type { Perms, User } from "../lib/types";

const permKeys: (keyof Perms)[] = ["read", "write", "share", "admin"];

export function Users() {
  const { session } = useSession();
  const sso = session!.config.authMethods.some((m) => m === "oauth" || m === "oidc");
  const [users, setUsers] = useState<User[] | null>(null);
  const [error, setError] = useState<ApiFailure | null>(null);
  const [editing, setEditing] = useState<User | "new" | null>(null);
  const [deleting, setDeleting] = useState<User | null>(null);

  const load = async () => {
    try {
      setUsers((await api<{ users: User[] }>("GET", "/api/users")).users);
      setError(null);
    } catch (e) {
      setError(e instanceof ApiFailure ? e : offline());
    }
  };

  useEffect(() => {
    load();
  }, []);

  return (
    <section class="settings-page">
      <header class="settings-head row">
        <div>
          <h1>{t("settings.users")}</h1>
          <p class="muted">{t("users.lead")}</p>
        </div>
        <button type="button" class="button primary" onClick={() => setEditing("new")}>
          <Icon name="plus" />
          {t("users.new")}
        </button>
      </header>
      {error && (
        <StatePanel icon="alert" tone="danger" title={t("settings.users")} body={errorText(error)}>
          <button type="button" class="button primary" onClick={load}>
            {t("state.tryAgain")}
          </button>
        </StatePanel>
      )}
      {users && (
        <div class="table-wrap">
          <table class="users-table">
            <thead>
              <tr>
                <th scope="col">{t("users.user")}</th>
                <th scope="col">{t("users.scope")}</th>
                <th scope="col">{t("users.permissions")}</th>
                <th scope="col">
                  <span class="visually-hidden">{t("explorer.actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {users.map((u) => (
                <tr key={u.id}>
                  <td>
                    <span class="user-cell">
                      <span class="avatar" aria-hidden="true">
                        {initials(u.name)}
                      </span>
                      <span class="account-text">
                        <span class="account-name">{u.name}</span>
                        <span class="account-role mono">{u.email}</span>
                      </span>
                    </span>
                  </td>
                  <td class="mono">{u.scope}</td>
                  <td>
                    <span class="chips">
                      {permKeys.map((k) => (
                        <span key={k} class={u.permissions[k] ? "chip on" : "chip"}>
                          {t(`perm.${k}`)}
                        </span>
                      ))}
                    </span>
                  </td>
                  <td class="row-actions">
                    <button
                      type="button"
                      class="icon-button"
                      aria-label={t("users.edit", { name: u.name })}
                      onClick={() => setEditing(u)}
                    >
                      <Icon name="edit" />
                    </button>
                    <button
                      type="button"
                      class="icon-button"
                      aria-label={t("users.delete", { name: u.name })}
                      onClick={() => setDeleting(u)}
                    >
                      <Icon name="trash" />
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
      {editing && (
        <UserForm
          user={editing === "new" ? undefined : editing}
          sso={sso}
          onSaved={(u) => {
            const name = u?.name ?? "";
            showToast(editing === "new" ? t("users.created", { name }) : t("users.updated", { name }));
            load();
          }}
          onChanged={load}
          onClose={() => setEditing(null)}
        />
      )}
      {deleting && (
        <DeleteUser
          user={deleting}
          onDeleted={() => {
            showToast(t("users.deleted", { name: deleting.name }));
            load();
          }}
          onClose={() => setDeleting(null)}
        />
      )}
    </section>
  );
}

function DeleteUser({ user, onDeleted, onClose }: { user: User; onDeleted(): void; onClose(): void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const cancel = useRef<HTMLButtonElement>(null);
  useLayoutEffect(() => cancel.current?.focus(), []);
  const confirm = async () => {
    setBusy(true);
    try {
      await api("DELETE", `/api/users/${user.id}`);
      onDeleted();
      onClose();
    } catch (e) {
      setError(errorText(e instanceof ApiFailure ? e : offline()));
      setBusy(false);
    }
  };
  return (
    <Dialog title={t("users.deleteTitle", { name: user.name })} onClose={onClose}>
      <p class="dialog-body">{t("users.deleteBody")}</p>
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
          {t("explorer.delete")}
        </button>
      </div>
    </Dialog>
  );
}
