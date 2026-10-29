// Sign in: provider buttons, then the password form with an optional CAPTCHA.
import { useEffect, useRef, useState } from "preact/hooks";
import wordmark from "../../../brand/cairn-wordmark.svg";
import { api, ApiFailure, offline } from "../lib/api";
import { effectiveTheme, mountCaptcha } from "../lib/captcha";
import { errorText, t } from "../i18n/index";
import { Mark } from "../components/icons";
import { useSession } from "../state/session";
import type { User } from "../lib/types";

export function Login() {
  const { session, setUser } = useSession();
  const config = session!.config;
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [keep, setKeep] = useState(false);
  const [token, setToken] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const captchaBox = useRef<HTMLDivElement>(null);
  const captcha = useRef<{ reset(): void } | null>(null);
  const passwordInput = useRef<HTMLInputElement>(null);

  const withPassword = config.authMethods.includes("password");
  const providers = config.providers;

  useEffect(() => {
    if (!withPassword || !config.captcha || !captchaBox.current) return;
    const { provider, siteKey } = config.captcha;
    mountCaptcha(captchaBox.current, provider, siteKey, effectiveTheme(), setToken)
      .then((c) => (captcha.current = c))
      .catch(() => setError(t("error.captcha_unreachable")));
  }, []);

  const submit = async (e: Event) => {
    e.preventDefault();
    if (sending) return;
    setSending(true);
    setError(null);
    try {
      const body: Record<string, unknown> = { email, password, keep };
      if (config.captcha) body.captchaToken = token;
      const res = await api<{ user: User }>("POST", "/api/auth/login", body);
      setUser(res.user);
    } catch (err) {
      setError(errorText(err instanceof ApiFailure ? err : offline()));
      setPassword("");
      captcha.current?.reset();
      passwordInput.current?.focus();
    } finally {
      setSending(false);
    }
  };

  const [setupBefore, setupAfter] = t("login.setup").split("{cmd}");

  return (
    <div class="login">
      <section class="login-intro">
        <div class="brand">
          <Mark size={36} />
          <span class="wordmark wordmark-large" aria-label="Cairn" dangerouslySetInnerHTML={{ __html: wordmark }} />
        </div>
        <div class="login-pebbles" aria-hidden="true">
          <span />
          <span />
          <span />
          <span />
        </div>
        <div class="login-words">
          <p class="login-tagline">{t("login.tagline")}</p>
          <p class="muted">{t("login.lead")}</p>
        </div>
      </section>
      <section class="login-panel">
        <form class="login-form" onSubmit={submit} noValidate>
          <div class="login-head">
            <h1>{t("login.title")}</h1>
            <span class="mono muted">{location.host}</span>
          </div>
          {config.setupNeeded && (
            <p class="notice">
              {setupBefore}
              <code>cairn user add</code>
              {setupAfter}
            </p>
          )}
          {error && (
            <div class="login-error" role="alert">
              {error}
            </div>
          )}
          {providers.length > 0 && (
            <div class="providers">
              {providers.map((p) => (
                <a key={p.href} class="button provider" href={p.href}>
                  {t("login.continueWith", { name: p.name })}
                </a>
              ))}
            </div>
          )}
          {providers.length > 0 && withPassword && (
            <div class="divider" role="separator">
              <span>{t("login.or")}</span>
            </div>
          )}
          {withPassword && (
            <>
              <div class="field">
                <label for="login-email">{t("login.email")}</label>
                <input
                  id="login-email"
                  class="input input-large"
                  type="email"
                  autocomplete="username"
                  required
                  value={email}
                  onInput={(e) => setEmail(e.currentTarget.value)}
                />
              </div>
              <div class="field">
                <label for="login-password">{t("login.password")}</label>
                <input
                  id="login-password"
                  ref={passwordInput}
                  class="input input-large"
                  type="password"
                  autocomplete="current-password"
                  required
                  value={password}
                  onInput={(e) => setPassword(e.currentTarget.value)}
                />
              </div>
              {config.captcha && <div class="captcha" ref={captchaBox} aria-label={t("login.captcha")} />}
              <label class="check">
                <input id="login-keep" type="checkbox" checked={keep} onChange={(e) => setKeep(e.currentTarget.checked)} />
                <span>{t("login.keep")}</span>
              </label>
              <button
                type="submit"
                class="button primary input-large"
                disabled={sending || (config.captcha !== null && !token)}
              >
                {t("login.submit")}
              </button>
              <p class="muted small">{t("login.forgot")}</p>
            </>
          )}
          {!withPassword && providers.length === 0 && <p class="muted">{t("login.none")}</p>}
        </form>
      </section>
    </div>
  );
}
