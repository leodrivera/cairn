// Profile: who is signed in, the display name, the password, and preferences (theme, view, language).
import { useState } from "preact/hooks";
import { api, ApiFailure, offline } from "../lib/api";
import { passwordOk } from "../lib/password";
import { errorText, t, type Key } from "../i18n/index";
import { showToast } from "../components/toast";
import { useSession, useUser } from "../state/session";
import type { Lang, User } from "../lib/types";

function failure(e: unknown): string {
  return errorText(e instanceof ApiFailure ? e : offline());
}

type Prefs = { name: string; theme: User["theme"]; view: User["view"]; lang: Lang | null };

export function Profile() {
  const user = useUser();
  const { session, setUser } = useSession();
  const method = session?.me.method ?? "password";
  const start: Prefs = { name: user.name, theme: user.theme, view: user.view, lang: user.lang };
  const [prefs, setPrefs] = useState(start);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const save = async (e: Event) => {
    e.preventDefault();
    const changed: Partial<Prefs> = {};
    for (const k of Object.keys(prefs) as (keyof Prefs)[]) {
      const v = k === "name" ? prefs.name.trim() : prefs[k];
      if (v !== start[k]) (changed as Record<string, unknown>)[k] = v;
    }
    if (Object.keys(changed).length === 0) return;
    setBusy(true);
    setError("");
    try {
      const res = await api<{ user: User }>("PATCH", "/api/me", changed);
      setUser(res.user);
      showToast(t("settings.saved"));
    } catch (err) {
      setError(failure(err));
    }
    setBusy(false);
  };

  const signedIn =
    method === "password"
      ? t("profile.signedInPassword", { email: user.email })
      : t("profile.signedInWith", { email: user.email, method: t(`method.${method}` as Key) });

  return (
    <section class="settings-page">
      <header class="settings-head">
        <h1>{t("settings.profile")}</h1>
        <p class="muted">{signedIn}</p>
      </header>
      <form class="settings-card" onSubmit={save} noValidate>
        <div class="field">
          <label class="field-label" for="profile-name">
            {t("profile.name")}
          </label>
          <input
            id="profile-name"
            class="input"
            value={prefs.name}
            autocomplete="name"
            onInput={(e) => {
              const name = e.currentTarget.value;
              setPrefs((p) => ({ ...p, name }));
            }}
          />
        </div>
        <h2>{t("profile.preferences")}</h2>
        <div class="field">
          <label class="field-label" for="profile-theme">
            {t("profile.theme")}
          </label>
          <select
            id="profile-theme"
            class="input"
            value={prefs.theme ?? ""}
            onChange={(e) => {
              const theme = (e.currentTarget.value || null) as User["theme"];
              setPrefs((p) => ({ ...p, theme }));
            }}
          >
            <option value="">{t("profile.themeServer")}</option>
            <option value="dark">{t("theme.dark")}</option>
            <option value="light">{t("theme.light")}</option>
          </select>
        </div>
        <div class="field">
          <label class="field-label" for="profile-view">
            {t("profile.view")}
          </label>
          <select
            id="profile-view"
            class="input"
            value={prefs.view ?? "list"}
            onChange={(e) => {
              const view = e.currentTarget.value as User["view"];
              setPrefs((p) => ({ ...p, view }));
            }}
          >
            <option value="list">{t("profile.viewList")}</option>
            <option value="gallery">{t("profile.viewGallery")}</option>
          </select>
        </div>
        <div class="field">
          <label class="field-label" for="profile-lang">
            {t("profile.lang")}
          </label>
          <select
            id="profile-lang"
            class="input"
            value={prefs.lang ?? ""}
            onChange={(e) => {
              const lang = (e.currentTarget.value || null) as Lang | null;
              setPrefs((p) => ({ ...p, lang }));
            }}
          >
            {start.lang === null && <option value="">{t("profile.langBrowser")}</option>}
            <option value="en">English</option>
            <option value="pt-BR">Português (Brasil)</option>
          </select>
        </div>
        {error && (
          <p class="error-line" role="alert">
            {error}
          </p>
        )}
        <div class="settings-buttons">
          <button type="submit" class="button primary" disabled={busy}>
            {t("settings.save")}
          </button>
        </div>
      </form>
      {user.hasPassword && <ChangePassword />}
    </section>
  );
}

function ChangePassword() {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [repeat, setRepeat] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async (e: Event) => {
    e.preventDefault();
    if (!passwordOk(next)) return setError(t("error.weak_password"));
    if (next !== repeat) return setError(t("profile.mismatch"));
    setBusy(true);
    setError("");
    try {
      await api("PUT", "/api/me/password", { current, next });
      setCurrent("");
      setNext("");
      setRepeat("");
      showToast(t("profile.passwordChanged"));
    } catch (err) {
      setError(failure(err));
    }
    setBusy(false);
  };

  const field = (id: string, label: string, value: string, set: (v: string) => void, auto: string) => (
    <div class="field">
      <label class="field-label" for={id}>
        {label}
      </label>
      <input
        id={id}
        class="input"
        type="password"
        autocomplete={auto}
        value={value}
        onInput={(e) => set(e.currentTarget.value)}
      />
    </div>
  );

  return (
    <form class="settings-card" onSubmit={submit} noValidate>
      <h2>{t("profile.password")}</h2>
      {field("profile-current", t("profile.current"), current, setCurrent, "current-password")}
      {field("profile-new", t("profile.new"), next, setNext, "new-password")}
      {field("profile-repeat", t("profile.repeat"), repeat, setRepeat, "new-password")}
      <p class="muted small">{t("profile.passwordNote")}</p>
      {error && (
        <p class="error-line" role="alert">
          {error}
        </p>
      )}
      <div class="settings-buttons">
        <button type="submit" class="button" disabled={busy}>
          {t("profile.password")}
        </button>
      </div>
    </form>
  );
}
