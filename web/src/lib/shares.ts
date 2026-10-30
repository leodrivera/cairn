// Share links: the management API and the address of a link.
import { api } from "./api";
import { t } from "../i18n/index";
import type { Share } from "./types";

export type Expiry = 1 | 7 | 30 | null;
export type ShareOptions = { expiresDays: Expiry; password: string | null; download: boolean };

export async function createShare(path: string, opts: ShareOptions): Promise<Share> {
  return (await api<{ share: Share }>("POST", "/api/shares", { path, ...opts })).share;
}

export async function listShares(): Promise<Share[]> {
  return (await api<{ shares: Share[] }>("GET", "/api/shares")).shares;
}

export async function updateShare(id: string, patch: Partial<ShareOptions>): Promise<Share> {
  return (await api<{ share: Share }>("PATCH", `/api/shares/${encodeURIComponent(id)}`, patch)).share;
}

export async function deleteShare(id: string): Promise<void> {
  await api<void>("DELETE", `/api/shares/${encodeURIComponent(id)}`);
}

/** The server's public address when it has one, else this page's. */
export function shareUrl(s: Share): string {
  return s.url ?? `${location.origin}/s/${s.id}`;
}

/** False when the browser refuses or has no clipboard. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

/** "Today", "Tomorrow", "In 5 days" or "Never", and whether it ends within two days. */
export function expiryText(expiresAt: number | null, now = Date.now() / 1000): { text: string; soon: boolean } {
  if (expiresAt === null) return { text: t("links.never"), soon: false };
  const left = expiresAt - now;
  const soon = left < 2 * 86_400;
  if (left < 86_400) return { text: t("links.today"), soon };
  if (left < 2 * 86_400) return { text: t("links.tomorrow"), soon };
  return { text: t("links.inDays", { n: Math.ceil(left / 86_400) }), soon };
}
