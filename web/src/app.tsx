// The app: loads the session, keeps people who are signed out on the sign-in page, and picks the screen.
import type { ComponentChildren } from "preact";
import { useEffect, useState } from "preact/hooks";
import { ApiFailure, offline, onSignedOut } from "./lib/api";
import { href, navigate, useRoute, type Route } from "./lib/router";
import { errorText, t } from "./i18n/index";
import { Shell } from "./components/shell";
import { Login } from "./screens/login";
import { Explorer } from "./screens/explorer";
import { Viewer } from "./screens/viewer";
import { StatePanel } from "./components/states";
import { applyLang, applyTheme, loadSession, SessionContext, type Session } from "./state/session";
import type { User } from "./lib/types";

/** Screens by route; filled in as they are built. */
function screenFor(route: Route, user: User): ComponentChildren {
  if (route.name === "missing") {
    return (
      <StatePanel icon="alert" title={t("state.missing.title")} body={t("state.missing.body")}>
        <a class="button" href="/files/" onClick={go("/files/")}>
          {t("state.backToFiles")}
        </a>
      </StatePanel>
    );
  }
  if (route.name === "view") return <Viewer path={route.path} />;
  if (route.name === "settings" && !user.permissions.admin && (route.page === "users" || route.page === "server")) {
    return null;
  }
  return null;
}

function go(to: string) {
  return (e: MouseEvent) => {
    e.preventDefault();
    navigate(to);
  };
}

export function App() {
  const [session, setSession] = useState<Session | null>(null);
  const [error, setError] = useState<ApiFailure | null>(null);
  const [slow, setSlow] = useState(false);
  const route = useRoute();

  const reload = async () => {
    setError(null);
    try {
      const s = await loadSession();
      applyTheme(s.me.user, s.config);
      applyLang(s.me.user);
      setSession(s);
    } catch (e) {
      setError(e instanceof ApiFailure ? e : offline());
    }
  };

  const setUser = (user: User | null) =>
    setSession((s) => {
      if (!s) return s;
      applyTheme(user, s.config);
      applyLang(user);
      return { ...s, me: user ? { ...s.me, user } : { user: null } };
    });

  useEffect(() => {
    reload();
    onSignedOut(() => setUser(null));
    const timer = setTimeout(() => setSlow(true), 300);
    return () => clearTimeout(timer);
  }, []);

  const user = session?.me.user ?? null;
  useEffect(() => {
    if (!session) return;
    if (!user && route.name !== "login") {
      navigate(href({ name: "login", next: location.pathname + location.search }), { replace: true });
    } else if (user && route.name === "login") {
      navigate(route.next ?? "/files/", { replace: true });
    }
  }, [session, user, route]);

  let body: ComponentChildren;
  if (error) {
    const off = error.code === "offline";
    body = (
      <main class="screen center">
        <StatePanel
          icon="alert"
          tone="danger"
          title={off ? t("state.offline.title") : t("state.failed.title")}
          body={off ? t("state.offline.body") : errorText(error)}
        >
          <button type="button" class="button primary" onClick={reload}>
            {t("state.tryAgain")}
          </button>
        </StatePanel>
      </main>
    );
  } else if (!session) {
    body = <main class="screen center">{slow && <p class="muted">{t("app.loading")}</p>}</main>;
  } else if (!user && route.name === "login") {
    body = <Login />;
  } else if (!user || route.name === "login") {
    body = <main class="screen center" />;
  } else if (route.name === "files") {
    body = <Explorer path={route.path} />;
  } else {
    body = <Shell>{screenFor(route, user)}</Shell>;
  }

  return (
    <SessionContext.Provider value={{ session, error, reload, setUser }}>{body}</SessionContext.Provider>
  );
}
