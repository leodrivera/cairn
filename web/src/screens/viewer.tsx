// One file: its header (name, folder, size, when), then the image, the text editor, or a way to download it.
import { useEffect, useRef, useState } from "preact/hooks";
import { ApiFailure, failureOf, offline, probe, request } from "../lib/api";
import { apiUrl, displaySegment, parent, segments } from "../lib/paths";
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
import { own, WhereContext, type Where } from "../lib/where";
import type { Entry } from "../lib/types";

type Text = { text: string; saved: string; readOnly: boolean };

/** What a one-byte read says about a file: size from Content-Range, type and date from their headers. */
function entryOf(segment: string, res: Response): Entry {
  const total = /\/(\d+)$/.exec(res.headers.get("content-range") ?? "")?.[1];
  const size = total !== undefined ? Number(total) : Number(res.headers.get("content-length") ?? 0);
  const when = Date.parse(res.headers.get("last-modified") ?? "");
  const mime = (res.headers.get("content-type") ?? "application/octet-stream").split(";")[0].trim();
  return {
    name: displaySegment(segment),
    kind: "file",
    size,
    modified: Number.isFinite(when) ? Math.floor(when / 1000) : 0,
    mime,
    ...(segment.startsWith("\u0000") ? { rawName: segment } : {}),
  };
}

export function Viewer({ path, where = own }: { path: string; where?: Where }) {
  return (
    <WhereContext.Provider value={where}>
      <File path={path} where={where} />
    </WhereContext.Provider>
  );
}

function File({ path, where }: { path: string; where: Where }) {
  const user = useUser();
  const { session } = useSession();
  const editMax = session!.config.editMax;
  const base = where.api;
  const grant = where.grant;
  // A grant of one file opens it as the path "": its name comes from the grant.
  const single = grant?.kind === "file";
  const dir = single ? "" : parent(path);
  const segment = single ? grant!.name : segments(path).at(-1)!;
  const canWrite = grant ? grant.effective === "write" : user.permissions.write;
  const root = grant?.name;
  const [entry, setEntry] = useState<Entry | null>(null);
  const [error, setError] = useState<ApiFailure | null>(null);
  const [doc, setDoc] = useState<Text | null>(null);
  const [saving, setSaving] = useState(false);
  const [conflict, setConflict] = useState(false);
  const [saveError, setSaveError] = useState<ApiFailure | null>(null);
  const etag = useRef("");
  // Only the latest load may fill the editor: a slow file just left must not land under this one.
  const latest = useRef(0);

  const kind = !entry
    ? null
    : entry.mime.startsWith("image/")
      ? "image"
      : isEditable(entry.mime, entry.name)
        ? entry.size > editMax
          ? "large"
          : "text"
        : "other";
  const canSave = kind === "text" && canWrite && !!doc && !doc.readOnly;
  const dirty = !!doc && doc.text !== doc.saved;

  const loadText = async () => {
    const n = ++latest.current;
    try {
      const res = await request(apiUrl("raw", path, undefined, base), { cache: "no-store" });
      const bytes = await res.arrayBuffer();
      if (n !== latest.current) return;
      etag.current = res.headers.get("ETag") ?? "";
      let text: string;
      let readOnly = !canWrite;
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
      if (n !== latest.current) return;
      setError(e instanceof ApiFailure ? e : offline());
    }
  };

  useEffect(() => {
    let live = true;
    latest.current++;
    setEntry(null);
    setError(null);
    setDoc(null);
    setConflict(false);
    setSaveError(null);
    probe(apiUrl("raw", path, undefined, base)).then(
      (res) => live && setEntry(entryOf(segment, res)),
      (e) => live && setError(e instanceof ApiFailure ? e : offline()),
    );
    return () => {
      live = false;
    };
  }, [path, base]);

  useEffect(() => {
    if (kind === "text") loadText();
  }, [kind, path]);

  const save = async () => {
    if (!canSave || saving || !dirty || !doc) return;
    const sent = doc.text;
    setSaving(true);
    setSaveError(null);
    try {
      const res = await fetch(apiUrl("text", path, undefined, base), {
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

  // Ctrl/Cmd+S saves an editable file; elsewhere the browser keeps its own Save.
  const keys = useRef<() => boolean>(() => false);
  keys.current = () => {
    if (!canSave) return false;
    save();
    return true;
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "s" && keys.current()) e.preventDefault();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (!dirty) return;
    setLeaveGuard(() => confirm(t("viewer.leave")));
    const onUnload = (e: BeforeUnloadEvent) => {
      e.preventDefault();
      e.returnValue = "";
    };
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

  const back = single ? href({ name: "shared" }) : where.files(dir);
  const download =
    entry && !single ? downloadUrl(dir, [entry], base)! : apiUrl("raw", path, { download: "1" }, base);
  const backLabel = single ? t("shared.back") : t("viewer.back", { folder: folderLabel(dir, root) });

  let body;
  if (error) {
    body = (
      <StatePanel icon="alert" tone="danger" title={segment} body={errorText(error)}>
        <Link class="button" href={back}>
          {backLabel}
        </Link>
      </StatePanel>
    );
  } else if (kind === "image") {
    body = (
      <div class="viewer-media">
        <img class="viewer-image" src={apiUrl("raw", path, undefined, base)} alt={entry!.name} decoding="async" />
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
        <Link class="icon-button" href={back} aria-label={backLabel}>
          <Icon name="back" />
        </Link>
        <div class="viewer-title">
          <h1>{entry?.name ?? segment}</h1>
          {entry && (
            <p class="mono muted">
              {t("viewer.meta", {
                folder: single ? t("shared.by", { name: grant!.owner.name }) : folderLabel(dir, root),
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
