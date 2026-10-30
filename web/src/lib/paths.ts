// Paths in the server's form: segments joined by "/", the root is "". A name that is not UTF-8 is a segment that
// starts with NUL followed by its bytes in base64url (the entry's rawName). Display names never build URLs.
import type { Entry } from "./types";

export function segmentOf(e: Entry): string {
  return e.rawName ?? e.name;
}

export function segments(path: string): string[] {
  return path === "" ? [] : path.split("/");
}

export function join(dir: string, segment: string): string {
  return dir === "" ? segment : `${dir}/${segment}`;
}

export function parent(path: string): string {
  const i = path.lastIndexOf("/");
  return i < 0 ? "" : path.slice(0, i);
}

/** A segment as people read it: raw bytes decoded with U+FFFD for what is not UTF-8. */
export function displaySegment(segment: string): string {
  if (!segment.startsWith("\u0000")) return segment;
  const b64 = segment.slice(1).replace(/-/g, "+").replace(/_/g, "/");
  try {
    const bin = atob(b64 + "=".repeat((4 - (b64.length % 4)) % 4));
    const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
    return new TextDecoder("utf-8").decode(bytes);
  } catch {
    return "�";
  }
}

/** A whole path as people read it, each segment through `displaySegment`. */
export function displayPath(path: string): string {
  return path.split("/").map(displaySegment).join("/");
}

export function encodePath(path: string): string {
  return segments(path).map(encodeURIComponent).join("/");
}

type Kind = "files" | "raw" | "zip" | "upload" | "folders" | "text";

export function apiUrl(kind: Kind, path: string, query?: Record<string, string | string[]>): string {
  let url = path === "" ? `/api/${kind}` : `/api/${kind}/${encodePath(path)}`;
  if (query) {
    const parts = Object.entries(query).flatMap(([k, v]) =>
      (Array.isArray(v) ? v : [v]).map((one) => `${encodeURIComponent(k)}=${encodeURIComponent(one)}`),
    );
    if (parts.length) url += `?${parts.join("&")}`;
  }
  return url;
}
