// Client routes on the History API. No library: a handful of URLs.
import { h, type ComponentChildren } from "preact";
import { useEffect, useState } from "preact/hooks";
import { encodePath } from "./paths";

/** Why a provider sign-in came back to the sign-in page. */
export type LoginError = "sso" | "no-account";

export type SettingsPage = "profile" | "users" | "server" | "links";

export type Route =
  | { name: "files"; path: string }
  | { name: "view"; path: string }
  | { name: "login"; next: string | null; error?: LoginError }
  | { name: "settings"; page: SettingsPage }
  /** A share link's public page; `path` is beneath the shared folder, `file` when it names a file. */
  | { name: "share"; id: string; path: string; file: boolean }
  | { name: "missing" };

const pages: SettingsPage[] = ["profile", "users", "server", "links"];

/** Decodes the path after a prefix; null when a segment is empty inside, malformed, or hides a "/". */
function decodePath(rest: string): string | null {
  const trimmed = rest.replace(/\/$/, "");
  if (trimmed === "") return "";
  const out: string[] = [];
  for (const raw of trimmed.split("/")) {
    if (raw === "") return null;
    let seg: string;
    try {
      seg = decodeURIComponent(raw);
    } catch {
      return null;
    }
    if (seg.includes("/")) return null;
    out.push(seg);
  }
  return out.join("/");
}

/** A place on this site to return to after signing in; anything else is dropped. */
function localNext(next: string | null): string | null {
  return next && next.startsWith("/") && !next.startsWith("//") && !next.startsWith("/\\") ? next : null;
}

export function parseRoute(pathname: string, search: string): Route {
  if (pathname === "/" || pathname === "/files") return { name: "files", path: "" };
  if (pathname.startsWith("/files/")) {
    const path = decodePath(pathname.slice(7));
    return path === null ? { name: "missing" } : { name: "files", path };
  }
  if (pathname.startsWith("/view/")) {
    const path = decodePath(pathname.slice(6));
    return path ? { name: "view", path } : { name: "missing" };
  }
  if (pathname === "/login") {
    const q = new URLSearchParams(search);
    const route: Route = { name: "login", next: localNext(q.get("next")) };
    const error = q.get("error");
    if (error === "sso" || error === "no-account") route.error = error;
    return route;
  }
  if (pathname === "/settings" || pathname === "/settings/") return { name: "settings", page: "profile" };
  const share = /^\/s\/([A-Za-z0-9_-]{22})$/.exec(pathname);
  if (share) {
    const q = new URLSearchParams(search);
    const file = q.get("file");
    return { name: "share", id: share[1], path: file ?? q.get("path") ?? "", file: file !== null };
  }
  if (pathname.startsWith("/settings/")) {
    const page = pathname.slice(10).replace(/\/$/, "") as SettingsPage;
    return pages.includes(page) ? { name: "settings", page } : { name: "missing" };
  }
  return { name: "missing" };
}

export function href(r: Route): string {
  switch (r.name) {
    case "files":
      return `/files/${encodePath(r.path)}`;
    case "view":
      return `/view/${encodePath(r.path)}`;
    case "login":
      return r.next ? `/login?next=${encodeURIComponent(r.next)}` : "/login";
    case "settings":
      return `/settings/${r.page}`;
    case "share":
      return r.path === "" ? `/s/${r.id}` : `/s/${r.id}?${r.file ? "file" : "path"}=${encodeURIComponent(r.path)}`;
    case "missing":
      return "/";
  }
}

const listeners = new Set<() => void>();

/** Asked before leaving; returns false to stay (the editor with unsaved text uses it). */
let guard: (() => boolean) | null = null;

export function setLeaveGuard(g: (() => boolean) | null): void {
  guard = g;
}

/** Asks the leave guard, if any; true when it is fine to leave. Clears the guard once the person agreed. */
export function mayLeave(): boolean {
  if (guard && !guard()) return false;
  guard = null;
  return true;
}

/** The address the app shows, so a Back the guard refuses can be put back. */
let shown = typeof location === "undefined" ? "/" : location.pathname + location.search;

export function navigate(to: string, opts: { replace?: boolean } = {}): boolean {
  if (guard && !guard()) return false;
  if (opts.replace) history.replaceState(null, "", to);
  else history.pushState(null, "", to);
  shown = location.pathname + location.search;
  listeners.forEach((l) => l());
  return true;
}

/** Back and Forward ask the guard too. The browser has already moved, so staying pushes the shown address again. */
function onPop(): void {
  if (guard && !guard()) {
    history.pushState(null, "", shown);
    return;
  }
  shown = location.pathname + location.search;
  listeners.forEach((l) => l());
}

if (typeof window !== "undefined") window.addEventListener("popstate", onPop);

export function currentRoute(): Route {
  return parseRoute(location.pathname, location.search);
}

export function useRoute(): Route {
  const [route, setRoute] = useState(currentRoute);
  useEffect(() => {
    const update = () => setRoute(currentRoute());
    listeners.add(update);
    return () => {
      listeners.delete(update);
    };
  }, []);
  return route;
}

type LinkProps = {
  href: string;
  children?: ComponentChildren;
  class?: string;
  "aria-current"?: "page";
  "aria-label"?: string;
};

/** An <a> that navigates in the app on a plain left click and lets other clicks through. */
export function Link(props: LinkProps) {
  const onClick = (e: MouseEvent) => {
    if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    e.preventDefault();
    navigate(props.href);
  };
  return h("a", { ...props, onClick }, props.children);
}
