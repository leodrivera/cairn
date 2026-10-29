// The icon and tint for an entry, from its kind and MIME type only.
import { Icon, type IconName } from "./icons";
import type { Entry } from "../lib/types";

export function iconFor(e: Entry): { name: IconName; tone: "accent" | "info" | "muted" } {
  if (e.kind === "folder") return { name: "folder", tone: "accent" };
  if (e.outside) return { name: "link", tone: "muted" };
  const m = e.mime;
  if (m.startsWith("image/")) return { name: "image", tone: "info" };
  if (m.startsWith("video/")) return { name: "film", tone: "muted" };
  if (m.startsWith("audio/")) return { name: "music", tone: "muted" };
  if (/zip|gzip|x-tar|x-7z|compressed/.test(m)) return { name: "archive", tone: "muted" };
  if (/json|javascript|xml|x-sh|x-python|x-rust|typescript|toml|yaml|css|html/.test(m)) return { name: "code", tone: "muted" };
  return { name: "file", tone: "muted" };
}

export function FileIcon({ entry, size = 20 }: { entry: Entry; size?: number }) {
  const { name, tone } = iconFor(entry);
  return (
    <span class={`file-icon tone-${tone === "info" ? "info" : tone}`}>
      <Icon name={name} size={size} />
    </span>
  );
}
