// The signed-in frame: sidebar with the brand, places, disk use (admins) and the account; the screen beside it;
// the upload panel above both.
import type { ComponentChildren } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import wordmark from "../../../brand/cairn-wordmark.svg";
import { api } from "../lib/api";
import { formatSize } from "../lib/format";
import { Link, navigate, useRoute } from "../lib/router";
import { t } from "../i18n/index";
import { useSession, useUser } from "../state/session";
import { uploads } from "../state/uploads";
import { Icon, Mark, type IconName } from "./icons";
import { Toast } from "./toast";
import { UploadPanel } from "./upload-panel";

export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters = words.length > 1 ? words[0][0] + words[1][0] : (words[0] ?? "?").slice(0, 2);
  return letters.toLocaleUpperCase();
}

const NARROW = "(max-width: 720px)";

/** True on a phone-sized screen; follows rotation and resizes. Decides structure only; looks stay in CSS. */
export function useNarrow(): boolean {
  const [narrow, setNarrow] = useState(() => matchMedia(NARROW).matches);
  useEffect(() => {
    const m = matchMedia(NARROW);
    const update = () => setNarrow(m.matches);
    update();
    m.addEventListener("change", update);
    return () => m.removeEventListener("change", update);
  }, []);
  return narrow;
}

type Props = {
  space?: { used: number; total: number };
  class?: string;
  onDragOver?: (e: DragEvent) => void;
  onDrop?: (e: DragEvent) => void;
  /** Replaces the places list (the settings pages bring their own). */
  nav?: ComponentChildren;
  children?: ComponentChildren;
};

export function Shell({ space, class: cls, onDragOver, onDrop, nav: ownNav, children }: Props) {
  const { setUser } = useSession();
  const user = useUser();
  const route = useRoute();
  const narrow = useNarrow();
  const [open, setOpen] = useState(false);
  const menuButton = useRef<HTMLButtonElement>(null);
  const routeKey = JSON.stringify(route);

  useEffect(() => setOpen(false), [routeKey, narrow]);

  const sheet = useRef<HTMLElement>(null);

  const closeSheet = () => {
    setOpen(false);
    menuButton.current?.focus();
  };

  // Open: focus moves into the sheet, and Escape anywhere closes it.
  useEffect(() => {
    if (!open) return;
    sheet.current?.querySelector<HTMLElement>(".places a")?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeSheet();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);
  const place = route.name === "files" || route.name === "view" ? "files" : route.name === "settings" ? route.page : "";

  const signOut = async () => {
    const res = await api<{ next: string | null }>("POST", "/api/auth/logout").catch(() => ({ next: null }));
    if (res.next) {
      location.assign(res.next);
      return;
    }
    navigate("/login", { replace: true });
    setUser(null);
  };

  const nav: Place[] = [
    { href: "/files/", label: t("nav.myFiles"), icon: "home", on: place === "files" },
    { href: "/settings/links", label: t("nav.sharedLinks"), icon: "link", on: place === "links" },
  ];

  const sidebar = (
      <nav ref={sheet} class={narrow ? "sidebar sheet" : "sidebar"} aria-label={t("nav.primary")}>
        {narrow && (
          <button type="button" class="icon-button sheet-close" aria-label={t("shell.closeMenu")} onClick={closeSheet}>
            <Icon name="close" />
          </button>
        )}
        <Link href="/files/" class="brand">
          <Mark />
          <span class="wordmark" aria-label="Cairn" dangerouslySetInnerHTML={{ __html: wordmark }} />
        </Link>
        {ownNav ?? <Places items={nav} />}
        <div class="sidebar-foot">
          {user.permissions.admin && space && space.total > 0 && (
            <div class="space">
              <div class="space-bar" aria-hidden="true">
                <div style={{ width: `${Math.min(100, Math.round((space.used / space.total) * 100))}%` }} />
              </div>
              <span>{t("shell.space", { used: formatSize(space.used), total: formatSize(space.total) })}</span>
            </div>
          )}
          <div class="account">
            <Link href="/settings/profile" class="account-link" aria-label={t("shell.settings")}>
              <span class="avatar" aria-hidden="true">
                {initials(user.name)}
              </span>
              <span class="account-text">
                <span class="account-name">{user.name}</span>
                <span class="account-role">{user.permissions.admin ? t("shell.admin") : t("shell.user")}</span>
              </span>
            </Link>
            <button type="button" class="icon-button" aria-label={t("shell.signOut")} onClick={signOut}>
              <Icon name="signOut" />
            </button>
          </div>
        </div>
      </nav>
  );

  return (
    <div class={narrow ? "shell narrow" : "shell"}>
      {narrow && (
        <header class="topbar">
          <button
            ref={menuButton}
            type="button"
            class="icon-button"
            aria-label={t("shell.openMenu")}
            aria-expanded={open}
            onClick={() => setOpen(true)}
          >
            <Icon name="menu" />
          </button>
          <Link href="/files/" class="brand">
            <Mark />
            <span class="wordmark" aria-label="Cairn" dangerouslySetInnerHTML={{ __html: wordmark }} />
          </Link>
        </header>
      )}
      {(!narrow || open) && sidebar}
      {narrow && open && <div class="sheet-backdrop" onClick={closeSheet} aria-hidden="true" />}
      <main class={cls ? `screen ${cls}` : "screen"} onDragOver={onDragOver} onDrop={onDrop}>
        {children}
      </main>
      <UploadPanel queue={uploads()} />
      <Toast />
    </div>
  );
}

type Place = { href: string; label: string; icon: IconName; on: boolean };

export function Places({ items, class: cls = "places" }: { items: Place[]; class?: string }) {
  return (
    <ul class={cls}>
      {items.map((n) => (
        <li key={n.href}>
          <Link href={n.href} class={n.on ? "place on" : "place"} aria-current={n.on ? "page" : undefined}>
            <Icon name={n.icon} />
            <span>{n.label}</span>
          </Link>
        </li>
      ))}
    </ul>
  );
}
