// What people do to files, as API calls. Paths use each entry's segment (rawName when the name is not UTF-8).
import { api, ApiFailure } from "./api";
import { apiUrl, join, segmentOf } from "./paths";
import type { Entry } from "./types";

/** One item of a batch that did not go through, with the code and sentence a single request would get. */
export type Failed = { from?: string; path?: string; error: string; message: string };

export function failureOfItem(f: Failed): ApiFailure {
  return new ApiFailure(0, f.error, f.message);
}

export async function rename(dir: string, from: Entry, newName: string): Promise<void> {
  const r = await api<{ done: number; failed: Failed[] }>("POST", "/api/ops", {
    ops: [{ op: "rename", from: join(dir, segmentOf(from)), to: join(dir, newName) }],
  });
  if (r.failed.length) throw failureOfItem(r.failed[0]);
}

export function transfer(
  op: "move" | "copy",
  dir: string,
  items: Entry[],
  dest: string,
): Promise<{ done: number; failed: Failed[] }> {
  return api("POST", "/api/ops", {
    ops: items.map((e) => ({ op, from: join(dir, segmentOf(e)), to: join(dest, segmentOf(e)) })),
  });
}

export function remove(dir: string, items: Entry[]): Promise<{ deleted: number; failed: Failed[] }> {
  return api("DELETE", "/api/files", { paths: items.map((e) => join(dir, segmentOf(e))) });
}

export async function newFolder(dir: string, name: string): Promise<Entry> {
  const r = await api<{ entry: Entry }>("POST", apiUrl("folders", join(dir, name)));
  return r.entry;
}

/** A link that downloads one item: the file itself, or a folder as ZIP. Several items go through `zipForm`. */
export function downloadUrl(dir: string, items: Entry[]): string | null {
  if (items.length !== 1) return null;
  const e = items[0];
  const path = join(dir, segmentOf(e));
  return e.kind === "folder" ? apiUrl("zip", path) : apiUrl("raw", path, { download: "1" });
}

/** Downloads several items of `dir` as one ZIP. A plain form, so the browser saves the archive as it streams. */
export function zipForm(dir: string, items: Entry[]): void {
  const form = document.createElement("form");
  form.method = "post";
  form.action = apiUrl("zip", dir);
  form.hidden = true;
  for (const e of items) {
    const input = document.createElement("input");
    input.type = "hidden";
    input.name = "item";
    input.value = segmentOf(e);
    form.append(input);
  }
  document.body.append(form);
  form.submit();
  form.remove();
}
