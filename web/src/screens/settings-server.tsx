// Server: the environment, read-only, and the two settings kept in the database (default theme, thumbnails).
import { useEffect, useState } from "preact/hooks";
import { api, ApiFailure, offline } from "../lib/api";
import { formatSize } from "../lib/format";
import { errorText, t, tn, type Key } from "../i18n/index";
import { StatePanel } from "../components/states";
import { showToast } from "../components/toast";
import { useSession } from "../state/session";

type ServerInfo = {
  root: string;
  publicUrl: string | null;
  addr: string;
  authMethods: string[];
  captcha: "none" | "recaptcha" | "turnstile" | "hcaptcha";
  uploadMax: number;
  editMax: number;
  threads: number;
  trustedProxies: string[];
  log: string;
  trash?: { days: number; fromEnv: boolean; size: number; taken: string[] };
};

const captchaNames = { recaptcha: "reCAPTCHA v2", turnstile: "Turnstile", hcaptcha: "hCaptcha" };

function rows(s: ServerInfo): [string, string, string][] {
  return [
    [t("server.root"), s.root, "CAIRN_ROOT"],
    [t("server.publicUrl"), s.publicUrl ?? t("server.notSet"), "CAIRN_PUBLIC_URL"],
    [t("server.addr"), s.addr, "CAIRN_ADDR"],
    [t("server.methods"), s.authMethods.map((m) => t(`method.${m}` as Key)).join(", "), "CAIRN_AUTH_METHODS"],
    [t("server.captcha"), s.captcha === "none" ? t("server.off") : captchaNames[s.captcha], "CAIRN_CAPTCHA"],
    [t("server.uploadMax"), s.uploadMax > 0 ? formatSize(s.uploadMax) : t("server.noLimit"), "CAIRN_UPLOAD_MAX"],
    [t("server.editMax"), formatSize(s.editMax), "CAIRN_EDIT_MAX"],
    [t("server.threads"), String(s.threads), "CAIRN_THREADS"],
    [t("server.proxies"), s.trustedProxies.length ? s.trustedProxies.join(", ") : t("server.none"), "CAIRN_TRUSTED_PROXIES"],
    [t("server.log"), s.log, "CAIRN_LOG"],
    ...(s.trash?.fromEnv
      ? [[t("server.trash"), tn("server.trashDaysValue", s.trash.days), "CAIRN_TRASH_DAYS"] as [string, string, string]]
      : []),
  ];
}

export function ServerSettings() {
  const { session, reload } = useSession();
  const config = session!.config;
  const warn = session!.me.security?.captchaDisabled === true;
  const [info, setInfo] = useState<ServerInfo | null>(null);
  const [error, setError] = useState<ApiFailure | null>(null);
  const saved = { theme: config.theme, thumbnails: config.thumbnails };
  const [form, setForm] = useState(saved);
  const [busy, setBusy] = useState(false);
  const [saveError, setSaveError] = useState("");
  /** The trash days as typed; sent only when they differ from the server's. */
  const [days, setDays] = useState("");

  useEffect(() => {
    api<ServerInfo>("GET", "/api/server").then(
      (i) => {
        setInfo(i);
        if (i.trash) setDays(String(i.trash.days));
      },
      (e) => setError(e instanceof ApiFailure ? e : offline()),
    );
  }, []);
  const trash = info?.trash;
  const daysValue = Number(days);
  const daysOk = /^\d+$/.test(days) && daysValue <= 3650;

  const save = async (e: Event) => {
    e.preventDefault();
    if (trash && !trash.fromEnv && !daysOk) {
      setSaveError(t("server.trashDaysHint"));
      return;
    }
    setBusy(true);
    setSaveError("");
    const changedDays = trash && !trash.fromEnv && daysValue !== trash.days;
    try {
      await api("PUT", "/api/settings", changedDays ? { ...form, trashDays: daysValue } : form);
      if (changedDays) setInfo({ ...info!, trash: { ...trash!, days: daysValue } });
      await reload();
      showToast(t("server.saved"));
    } catch (err) {
      setSaveError(errorText(err instanceof ApiFailure ? err : offline()));
    }
    setBusy(false);
  };

  const [before, after] = t("server.captchaWarning", { name: "\u0000" }).split("\u0000");

  return (
    <section class="settings-page">
      <header class="settings-head">
        <h1>{t("settings.server")}</h1>
        <p class="muted">{t("server.note")}</p>
      </header>
      {warn && (
        <p class="notice warning" role="note">
          {before}
          <code>CAIRN_CAPTCHA</code>
          {after}
        </p>
      )}
      {error && <StatePanel icon="alert" tone="danger" title={t("settings.server")} body={errorText(error)} />}
      {info && (
        <div class="settings-card">
          <p class="env-badge muted small">{t("server.env")}</p>
          <dl class="env-rows">
            {rows(info).map(([label, value, env]) => (
              <div key={env} class="env-row">
                <dt>{label}</dt>
                <dd class="mono">{value}</dd>
                <dd class="mono muted env-name">{env}</dd>
              </div>
            ))}
          </dl>
        </div>
      )}
      <form class="settings-card" onSubmit={save} noValidate>
        <h2>{t("server.appearance")}</h2>
        <div class="field">
          <label class="field-label" for="server-theme">
            {t("server.theme")}
          </label>
          <select
            id="server-theme"
            class="input"
            value={form.theme ?? ""}
            onChange={(e) => {
              const theme = (e.currentTarget.value || null) as typeof form.theme;
              setForm((f) => ({ ...f, theme }));
            }}
          >
            <option value="">{t("server.themeDevice")}</option>
            <option value="dark">{t("theme.dark")}</option>
            <option value="light">{t("theme.light")}</option>
          </select>
        </div>
        <div class="field switch-field">
          <input
            id="server-thumbnails"
            type="checkbox"
            role="switch"
            class="switch"
            checked={form.thumbnails}
            aria-describedby="server-thumbnails-hint"
            onChange={() => setForm((f) => ({ ...f, thumbnails: !f.thumbnails }))}
          />
          <label class="field-label" for="server-thumbnails">
            {t("server.thumbnails")}
          </label>
          <p id="server-thumbnails-hint" class="hint">
            {t("server.thumbnailsHint")}
          </p>
        </div>
        {trash && (
          <>
            <h2>{t("server.trash")}</h2>
            {trash.taken.map((folder) => (
              <p key={folder} class="notice warning" role="note">
                {t("server.trashTaken", { folder: folder === "/" ? "" : folder })}
              </p>
            ))}
            {!trash.fromEnv && (
              <div class="field">
                <label class="field-label" for="server-trash-days">
                  {t("server.trashDays")}
                </label>
                <input
                  id="server-trash-days"
                  class="input input-narrow"
                  inputMode="numeric"
                  value={days}
                  aria-describedby="server-trash-days-hint"
                  aria-invalid={daysOk ? undefined : true}
                  onInput={(e) => setDays(e.currentTarget.value.trim())}
                />
                <p id="server-trash-days-hint" class="hint">
                  {t("server.trashDaysHint")}
                </p>
              </div>
            )}
            <p class="muted">{t("server.trashSize", { size: formatSize(trash.size) })}</p>
          </>
        )}
        {saveError && (
          <p class="error-line" role="alert">
            {saveError}
          </p>
        )}
        <div class="settings-buttons">
          <button type="submit" class="button primary" disabled={busy}>
            {t("settings.save")}
          </button>
          <button
            type="button"
            class="button"
            onClick={() => {
              setForm(saved);
              if (trash) setDays(String(trash.days));
            }}
          >
            {t("settings.discard")}
          </button>
        </div>
      </form>
    </section>
  );
}
