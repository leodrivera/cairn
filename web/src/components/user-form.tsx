// The new user and edit user form. Write and Share need Read, so ticking them ticks Read and locks it.
import { useLayoutEffect, useRef, useState } from "preact/hooks";
import { api, ApiFailure, offline } from "../lib/api";
import { passwordOk } from "../lib/password";
import { errorText, t } from "../i18n/index";
import { showToast } from "./toast";
import { Dialog } from "./dialogs";
import type { Perms, User } from "../lib/types";
import { PasswordInput } from "./password-input";

type Props = {
  user?: User;
  sso: boolean;
  onSaved(user: User | null): void;
  /** After a change that keeps the form open (Remove password). */
  onChanged?(): void;
  onClose(): void;
};

const permKeys = ["read", "write", "share", "admin"] as const;

export function UserForm({ user, sso, onSaved, onChanged, onClose }: Props) {
  const [hasPassword, setHasPassword] = useState(user?.hasPassword ?? false);
  const [name, setName] = useState(user?.name ?? "");
  const [email, setEmail] = useState(user?.email ?? "");
  const [password, setPassword] = useState("");
  const [scope, setScope] = useState(user?.scope ?? "/");
  const [perms, setPerms] = useState<Perms>(
    user?.permissions ?? { read: true, write: false, share: false, admin: false },
  );
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const id = user ? `user-${user.id}` : "user-new";
  const first = useRef<HTMLInputElement>(null);
  useLayoutEffect(() => first.current?.focus(), []);

  const toggle = (k: keyof Perms) =>
    setPerms((p) => {
      const next = { ...p, [k]: !p[k] };
      if (next.write || next.share) next.read = true;
      return next;
    });

  const run = async (work: () => Promise<User | null>) => {
    setBusy(true);
    setError("");
    try {
      onSaved(await work());
      onClose();
    } catch (e) {
      setError(errorText(e instanceof ApiFailure ? e : offline()));
      setBusy(false);
    }
  };

  const submit = (e: Event) => {
    e.preventDefault();
    if (password && !passwordOk(password)) {
      setError(t("error.weak_password"));
      return;
    }
    run(async () => {
      if (!user) {
        const body: Record<string, unknown> = { name: name.trim(), email: email.trim() };
        if (password) body.password = password;
        body.scope = scope.trim();
        body.permissions = perms;
        return (await api<{ user: User }>("POST", "/api/users", body)).user;
      }
      const patch: Record<string, unknown> = {};
      if (name.trim() !== user.name) patch.name = name.trim();
      if (email.trim() !== user.email) patch.email = email.trim();
      if (scope.trim() !== user.scope) patch.scope = scope.trim();
      if (permKeys.some((k) => perms[k] !== user.permissions[k])) patch.permissions = perms;
      let saved = user;
      if (Object.keys(patch).length) saved = (await api<{ user: User }>("PATCH", `/api/users/${user.id}`, patch)).user;
      if (password) {
        await api("PUT", `/api/users/${user.id}/password`, { password });
        saved = { ...saved, hasPassword: true };
      }
      return saved;
    });
  };

  /** Acts at once and keeps the form open, so other edits in it are not lost. */
  const removePassword = async () => {
    setBusy(true);
    setError("");
    try {
      await api("DELETE", `/api/users/${user!.id}/password`);
      setHasPassword(false);
      showToast(t("users.passwordRemoved", { name: user!.name }));
      onChanged?.();
    } catch (e) {
      setError(errorText(e instanceof ApiFailure ? e : offline()));
    }
    setBusy(false);
  };

  const permLabels: Record<keyof Perms, string> = {
    read: t("form.read"),
    write: t("form.write"),
    share: t("form.share"),
    admin: t("form.admin"),
  };

  return (
    <Dialog title={user ? t("form.editUser", { name: user.name }) : t("form.newUser")} onClose={onClose} wide>
      <form class="dialog-form user-form" onSubmit={submit} noValidate>
        <div class="field">
          <label class="field-label" for={`${id}-name`}>
            {t("form.name")}
          </label>
          <input ref={first} id={`${id}-name`} class="input" value={name} onInput={(e) => setName(e.currentTarget.value)} />
        </div>
        <div class="field">
          <label class="field-label" for={`${id}-email`}>
            {t("form.email")}
          </label>
          <input
            id={`${id}-email`}
            class="input"
            type="email"
            autocomplete="off"
            value={email}
            aria-describedby={`${id}-email-hint`}
            onInput={(e) => setEmail(e.currentTarget.value)}
          />
          <p id={`${id}-email-hint`} class="hint">
            {t("form.emailHint")}
          </p>
        </div>
        <div class="field">
          <label class="field-label" for={`${id}-password`}>
            {user ? t("form.newPassword") : t("form.password")}
          </label>
          <PasswordInput
            id={`${id}-password`}
            autocomplete="new-password"
            value={password}
            describedBy={`${id}-password-hint`}
            onInput={setPassword}
          />
          <p id={`${id}-password-hint`} class="hint">
            {t("form.passwordHint")} {user ? t("form.newPasswordHint") : sso ? t("form.passwordSso") : ""}
          </p>
          {user && sso && hasPassword && (
            <button type="button" class="button quiet danger-text" disabled={busy} onClick={removePassword}>
              {t("form.removePassword")}
            </button>
          )}
        </div>
        <div class="field">
          <label class="field-label" for={`${id}-scope`}>
            {t("form.scope")}
          </label>
          <input
            id={`${id}-scope`}
            class="input mono"
            value={scope}
            spellcheck={false}
            aria-describedby={`${id}-scope-hint`}
            onInput={(e) => setScope(e.currentTarget.value)}
          />
          <p id={`${id}-scope-hint`} class="hint">
            {t("form.scopeHint")}
          </p>
        </div>
        <fieldset class="field perms">
          <legend class="field-label">{t("form.permissions")}</legend>
          {permKeys.map((k) => (
            <label key={k} class="check">
              <input
                type="checkbox"
                checked={perms[k]}
                disabled={k === "read" && (perms.write || perms.share)}
                onChange={() => toggle(k)}
              />
              <span>{permLabels[k]}</span>
            </label>
          ))}
        </fieldset>
        {error && (
          <p class="error-line" role="alert">
            {error}
          </p>
        )}
        <div class="dialog-buttons">
          <button type="button" class="button" onClick={onClose}>
            {t("dialog.cancel")}
          </button>
          <button type="submit" class="button primary" disabled={busy}>
            {user ? t("settings.save") : t("form.create")}
          </button>
        </div>
      </form>
    </Dialog>
  );
}
