// The public side of a share link: everything under /s/<id>, with no session.
import { api } from "./api";
import { encodePath } from "./paths";
import type { Entry, ShareInfo } from "./types";

export function shareInfo(id: string): Promise<ShareInfo> {
  return api<ShareInfo>("GET", `/s/${id}/info`);
}

export function unlock(id: string, password: string): Promise<void> {
  return api<void>("POST", `/s/${id}/unlock`, { password });
}

export function shareList(id: string, path: string): Promise<{ path: string; entries: Entry[] }> {
  return api("GET", path === "" ? `/s/${id}/list` : `/s/${id}/list/${encodePath(path)}`);
}

export function rawUrl(id: string, path: string, download = false): string {
  const url = path === "" ? `/s/${id}/raw` : `/s/${id}/raw/${encodePath(path)}`;
  return download ? `${url}?download=1` : url;
}

export function zipUrl(id: string, path: string): string {
  return path === "" ? `/s/${id}/zip` : `/s/${id}/zip/${encodePath(path)}`;
}
