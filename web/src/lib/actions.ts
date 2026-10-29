// What people do to files, as API calls. Paths use each entry's segment (rawName when the name is not UTF-8).
import { apiUrl, join, segmentOf } from "./paths";
import type { Entry } from "./types";

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
