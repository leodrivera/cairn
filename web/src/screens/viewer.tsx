// One file: its header (name, folder, size, when), then the image, the text editor, or a way to download it.
import { useEffect, useRef, useState } from "preact/hooks";
import { api, ApiFailure, failureOf, offline, request } from "../lib/api";
import { apiUrl, parent, segments } from "../lib/paths";
import { downloadUrl } from "../lib/actions";
import { formatSize, formatWhen } from "../lib/format";
import { isEditable } from "../lib/text";
import { href, Link, setLeaveGuard } from "../lib/router";
import { errorText, t } from "../i18n/index";
import { Editor } from "../components/editor";
import { folderLabel } from "../components/dialogs";
import { Icon } from "../components/icons";
import { StatePanel } from "../components/states";
import { useSession, useUser } from "../state/session";
import type { Entry, Listing } from "../lib/types";

type Text = { text: string; saved: string; readOnly: boolean };

export function Viewer({ path }: { path: string }) {
  const user = useUser();
  const { session } = useSession();
  const editMax = session!.config.editMax;
  const dir = parent(path);
  const segment = segments(path).at(-1)!;
  const [entry, setEntry] = useState<Entry | null>(null);
  const [error, setError] = useState<ApiFailure | null>(null);
  const [doc, setDoc] = useState<Text | null>(null);
  const [saving, setSaving] = useState(false);
  const [conflict, setConflict] = useState(false);
  const [saveError, setSaveError] = useState<ApiFailure | null>(null);
  const etag = useRef("");

  const kind = !entry
    ? null
    : entry.mime.startsWith("image/")
      ? "image"
      : isEditable(entry.mime, entry.name)
        ? entry.size > editMax
          ? "large"
          : "text"
        : "other";
  const canSave = kind === "text" && user.permissions.write && !!doc && !doc.readOnly;
  const dirty = !!doc && doc.text !== doc.saved;

  const loadText = async () => {
    try {
      const res = await request(apiUrl("raw", path), { cache: "no-store" });
      etag.current = res.headers.get("ETag") ?? "";
      const bytes = await res.arrayBuffer();
      let text: string;
      let readOnly = !user.permissions.write;
      try {
        text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
      } catch {
        text = new TextDecoder("utf-8", { ignoreBOM: true }).decode(bytes);
        readOnly = true;
        setSaveError(new ApiFailure(0, "not_text", ""));
      }
      setDoc({ text, saved: text, readOnly });
      setConflict(false);
    } catch (e) {
      setError(e instanceof ApiFailure ? e : offline());
    }
  };

  useEffect(() => {
    let live = true;
    setEntry(null);
    setError(null);
    setDoc(null);
    setConflict(false);
    setSaveError(null);
    api<Listing>("GET", apiUrl("files", dir)).then(
      (l) => {
        if (!live) return;
        const found = l.entries.find((e) => (e.rawName ?? e.name) === segment);
        if (found && found.kind !== "folder") setEntry(found);
        else setError(new ApiFailure(404, "not_found", ""));
      },
      (e) => live && setError(e instanceof ApiFailure ? e : offline()),
    );
    return () => {
      live = false;
    };
  }, [path]);

  useEffect(() => {
    if (kind === "text") loadText();
  }, [kind, path]);

  const save = async () => {
    if (!canSave || saving || !dirty || !doc) return;
    const sent = doc.text;
    setSaving(true);
    setSaveError(null);
    try {
      const res = await fetch(apiUrl("text", path), {
        method: "PUT",
        headers: { "Content-Type": "text/plain; charset=utf-8", "If-Match": etag.current, "X-Cairn": "1" },
        body: sent,
        credentials: "same-origin",
      }).catch(() => {
        throw offline();
      });
      if (res.ok) {
        etag.current = res.headers.get("ETag") ?? ((await res.json()) as { etag: string }).etag;
        setDoc((d) => d && { ...d, saved: sent });
        setConflict(false);
      } else {
        const f = await failureOf(res);
        if (res.status === 412 || f.code === "changed") setConflict(true);
        else setSaveError(f);
      }
    } catch (e) {
      setSaveError(e instanceof ApiFailure ? e : offline());
    }
    setSaving(false);
  };

  const keys = useRef(save);
  keys.current = save;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "s") {
        e.preventDefault();
        keys.current();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (!dirty) return;
    setLeaveGuard(() => confirm(t("viewer.leave")));
    const onUnload = (e: BeforeUnloadEvent) => e.preventDefault();
    addEventListener("beforeunload", onUnload);
    return () => {
      setLeaveGuard(null);
      removeEventListener("beforeunload", onUnload);
    };
  }, [dirty]);

  const reload = () => {
    if (dirty && !confirm(t("viewer.reloadAsk"))) return;
    loadText();
  };

  const downloadMine = () => {
    if (!doc || !entry) return;
    const url = URL.createObjectURL(new Blob([doc.text], { type: "text/plain;charset=utf-8" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = entry.name;
    document.body.append(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 60_000);
  };

  const back = href({ name: "files", path: dir });
  const download = entry ? downloadUrl(dir, [entry])! : apiUrl("raw", path, { download: "1" });

  let body;
  if (error) {
    body = (
      <StatePanel icon="alert" tone="danger" title={segment} body={errorText(error)}>
        <Link class="button" href={back}>
          {t("viewer.back", { folder: folderLabel(dir) })}
        </Link>
      </StatePanel>
    );
  } else if (kind === "image") {
    body = (
      <div class="viewer-media">
        <img class="viewer-image" src={apiUrl("raw", path)} alt={entry!.name} decoding="async" />
      </div>
    );
  } else if (kind === "large" || kind === "other") {
    body = (
      <StatePanel
        icon="file"
        title={entry!.name}
        body={kind === "large" ? t("viewer.tooLarge", { size: formatSize(entry!.size) }) : t("viewer.noPreview")}
      >
        <a class="button primary" href={download} download>
          <Icon name="download" />
          {t("explorer.download")}
        </a>
      </StatePanel>
    );
  } else if (kind === "text" && doc) {
    body = (
      <>
        {conflict && (
          <div class="notice viewer-notice" role="alert">
            <span>{t("error.changed")}</span>
            <span class="viewer-notice-actions">
              <button type="button" class="button" onClick={reload}>
                {t("viewer.reload")}
              </button>
              <button type="button" class="button" onClick={downloadMine}>
                {t("viewer.downloadMine")}
              </button>
            </span>
          </div>
        )}
        {saveError && saveError.code === "not_text" && (
          <p class="error-line" role="alert">
            {errorText(saveError)}
          </p>
        )}
        {saveError && saveError.code !== "not_text" && (
          <div class="notice viewer-notice" role="alert">
            <span>{errorText(saveError)}</span>
            <button type="button" class="button" onClick={downloadMine}>
              {t("viewer.downloadMine")}
            </button>
          </div>
        )}
        <Editor
          name={entry!.name}
          text={doc.text}
          readOnly={doc.readOnly}
          save={canSave ? (saving ? "saving" : dirty ? "unsaved" : "saved") : null}
          onInput={(text) => setDoc((d) => d && { ...d, text })}
        />
      </>
    );
  }

  return (
    <div class="viewer">
      <header class="viewer-head">
        <Link class="icon-button" href={back} aria-label={t("viewer.back", { folder: folderLabel(dir) })}>
          <Icon name="back" />
        </Link>
        <div class="viewer-title">
          <h1>{entry?.name ?? segment}</h1>
          {entry && (
            <p class="mono muted">
              {t("viewer.meta", {
                folder: folderLabel(dir),
                size: formatSize(entry.size),
                when: formatWhen(entry.modified),
              })}
            </p>
          )}
        </div>
        {entry && (
          <div class="viewer-actions">
            <a class="button" href={download} download>
              <Icon name="download" />
              <span class="label">{t("explorer.download")}</span>
            </a>
            {canSave && (
              <button type="button" class="button primary" disabled={!dirty || saving} onClick={save}>
                {t("viewer.save")}
              </button>
            )}
          </div>
        )}
      </header>
      {body}
    </div>
  );
}
