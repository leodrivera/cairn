// Sample session data for screen tests.
import type { Config, Entry, ReceivedGrant, User } from "../lib/types";
import { json, stubFetch, type Handler } from "./dom";

export const config: Config = {
  authMethods: ["password"],
  captcha: null,
  providers: [],
  theme: null,
  thumbnails: false,
  editMax: 2097152,
};

const all = { read: true, write: true, share: true, admin: true };

export const admin: User = {
  id: 1,
  email: "admin@example.com",
  name: "Admin",
  scope: "/",
  permissions: all,
  lang: null,
  theme: null,
  view: null,
  hasPassword: true,
};

export const reader: User = {
  ...admin,
  id: 2,
  email: "guest@example.com",
  name: "Guest",
  permissions: { read: true, write: false, share: false, admin: false },
};

/** Mid-2026, so sample dates read as "Oct 24". */
const when = Math.floor(new Date(2026, 9, 24, 8, 0).getTime() / 1000);

export function file(name: string, extra: Partial<Entry> = {}): Entry {
  return { name, kind: "file", size: 4_404_019, modified: when, mime: "application/octet-stream", ...extra };
}

export function folder(name: string, extra: Partial<Entry> = {}): Entry {
  return { name, kind: "folder", size: 0, modified: when, mime: "application/octet-stream", ...extra };
}

/** Puts the browser at `at` with a session for `user` and the given extra routes. Returns the request log. */
export function startAt(
  at: string,
  user: User | null,
  routes: Record<string, Handler> = {},
  cfg: Partial<Config> = {},
): Request[] {
  history.replaceState(null, "", at);
  return stubFetch({
    "GET /api/config": () => json({ ...config, ...cfg }),
    "GET /api/me": () => json({ user }),
    ...routes,
  });
}

export function listing(path: string, entries: Entry[], extra: object = {}): Handler {
  return () => json({ path, entries, ...extra });
}

/** A grant Bob received from Ana, as the recipient sees it. */
export function received(extra: Partial<ReceivedGrant> = {}): ReceivedGrant {
  return {
    id: "AbCdEfGhIjKlMnOpQrSt-_",
    name: "team",
    kind: "folder",
    permission: "write",
    effective: "write",
    expiresAt: null,
    createdAt: when,
    owner: { id: 7, name: "Ana" },
    recipient: { id: 2, name: "Guest", email: "guest@example.com" },
    ...extra,
  };
}
