// The session: server config and the signed-in user, loaded once and shared through context.
import { createContext } from "preact";
import { useContext } from "preact/hooks";
import { api, type ApiFailure } from "../lib/api";
import { pickLang, setLang } from "../i18n/index";
import type { Config, Me, User } from "../lib/types";

export type Session = { config: Config; me: Me };

export type SessionState = {
  session: Session | null;
  error: ApiFailure | null;
  reload(): Promise<void>;
  setUser(u: User | null): void;
};

export const SessionContext = createContext<SessionState>({
  session: null,
  error: null,
  reload: async () => {},
  setUser: () => {},
});

export function useSession(): SessionState {
  return useContext(SessionContext);
}

/** The signed-in user; only for screens that render inside the signed-in app. */
export function useUser(): User {
  const user = useSession().session?.me.user;
  if (!user) throw new Error("useUser outside the signed-in app");
  return user;
}

export async function loadSession(): Promise<Session> {
  const [config, me] = await Promise.all([api<Config>("GET", "/api/config"), api<Me>("GET", "/api/me")]);
  return { config, me };
}

/** The user's theme, else the server default, else no attribute: the device decides. */
export function applyTheme(user: User | null, config: Config): void {
  const theme = user?.theme ?? config.theme;
  if (theme) document.documentElement.dataset.theme = theme;
  else document.documentElement.removeAttribute("data-theme");
}

export function applyLang(user: User | null): void {
  const l = pickLang(user, navigator.languages ?? []);
  setLang(l);
  document.documentElement.lang = l;
}
