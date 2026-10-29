// Search is a filter of the open folder, in the browser: case and accents do not matter.
import type { Entry } from "./types";

export function fold(s: string): string {
  return s.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();
}

export function filterEntries(entries: Entry[], query: string): Entry[] {
  const q = fold(query.trim());
  return q === "" ? entries : entries.filter((e) => fold(e.name).includes(q));
}
