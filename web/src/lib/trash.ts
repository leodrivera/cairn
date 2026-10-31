// The trash, as API calls.
import { api } from "./api";
import type { Failed } from "./actions";
import type { TrashEntry } from "./types";

export async function listTrash(): Promise<TrashEntry[]> {
  return (await api<{ entries: TrashEntry[] }>("GET", "/api/trash")).entries;
}

export function restore(ids: string[]): Promise<{ restored: { id: string; path: string; name: string }[]; failed: Failed[] }> {
  return api("POST", "/api/trash/restore", { ids });
}

export function deleteForever(ids: string[]): Promise<{ deleted: number; failed: Failed[] }> {
  return api("DELETE", "/api/trash", { ids });
}

export function emptyTrash(): Promise<{ deleted: number }> {
  return api("POST", "/api/trash/empty");
}
