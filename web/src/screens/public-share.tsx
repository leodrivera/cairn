// The public page of a share link: no session, no sidebar. A password prompt, the file, or the folder's contents.
import { useEffect, useState } from "preact/hooks";
import { api, ApiFailure, offline, request } from "../lib/api";
import { rawUrl, shareInfo, shareList, unlock, zipUrl } from "../lib/public";
import { displaySegment, join, parent, segmentOf, segments } from "../lib/paths";
import { formatSize, formatWhen } from "../lib/format";
import { isEditable } from "../lib/text";
import { expiryText } from "../lib/shares";
import { href, Link } from "../lib/router";
import { errorText, lang, t } from "../i18n/index";
import { FileIcon } from "../components/file-icon";
import { StatePanel } from "../components/states";
import { applyLang, applyTheme } from "../state/session";
import type { Config, Entry, ShareInfo } from "../lib/types";

type Props = { id: string; path: string; file: boolean };

type State =
  | { kind: "loading" }
  | { kind: "gone" }
  | { kind: "locked" }
  | { kind: "failed"; error: ApiFailure }
  | { kind: "ready"; info: ShareInfo };

const EDIT_MAX = 2_097_152;

function failure(e: unknown): ApiFailure {
  return e instanceof ApiFailure ? e : offline();
}

export function PublicShare({ id, path, file }: Props) {
  // No user here: the browser's language, and the server's default theme once it is known.
  useState(() => applyLang(null));
  const [editMax, setEditMax] = useState(EDIT_MAX);
  const [state, setState] = useState<State>({ kind: "loading" });

  const load = async () => {
    try {
      setState({ kind: "ready", info: await shareInfo(id) });
    } catch (e) {
      const f = failure(e);
      if (f.code === "share_gone") setState({ kind: "gone" });
      else if (f.code === "share_locked") setState({ kind: "locked" });
      else setState({ kind: "failed", error: f });
    }
  };

  useEffect(() => {
    api<Config>("GET", "/api/config").then(
      (c) => {
        applyTheme(null, c);
        setEditMax(c.editMax);
      },
      () => {},
    );
  }, []);

  useEffect(() => {
    load();
  }, [id]);

  let body;
  if (state.kind === "gone") {
    body = <StatePanel icon="link" title={t("public.goneTitle")} body={t("public.goneBody")} />;
  } else if (state.kind === "locked") {
    body = <Unlock id={id} onOpen={load} />;
  } else if (state.kind === "failed") {
    body = (
      <StatePanel icon="alert" tone="danger" title={t("state.offline.title")} body={errorText(state.error)}>
        <button type="button" class="button primary" onClick={load}>
          {t("state.tryAgain")}
        </button>
      </StatePanel>
    );
  } else if (state.kind === "ready") {
    const { info } = state;
    const folder = info.entry.kind === "folder";
    body = (
      <>
        {!folder ? (
          <FileView id={id} path="" entry={info.entry} download={info.download} editMax={editMax} />
        ) : file ? (
          <FolderFile id={id} path={path} download={info.download} editMax={editMax} />
        ) : (
          <FolderView id={id} path={path} root={info.entry.name} download={info.download} />
        )}
        <Footer info={info} />
      </>
    );
  }
  return <main class="public">{body}</main>;
}

function Footer({ info }: { info: ShareInfo }) {
  const by = t("public.sharedBy", { name: info.sharedBy });
  const when = info.expiresAt === null ? "" : expiryText(info.expiresAt).text.toLocaleLowerCase(lang());
  return (
    <footer class="public-foot muted">
      <p>{when ? `${by} · ${t("public.expires", { when })}` : by}</p>
      <p class="public-brand">
        <img src="/favicon.svg" alt="" width={16} height={16} />
        {t("public.poweredBy")}
      </p>
    </footer>
  );
}

function Unlock({ id, onOpen }: { id: string; onOpen(): void }) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const submit = async (e: Event) => {
    e.preventDefault();
    if (busy || password === "") return;
    setBusy(true);
    setError("");
    try {
      await unlock(id, password);
      onOpen();
    } catch (err) {
      setError(errorText(failure(err)));
      setBusy(false);
    }
  };
  return (
    <form class="public-card public-lock" onSubmit={submit} noValidate>
      <h1>{t("public.lockedTitle")}</h1>
      <label class="field-label" for="public-password">
        {t("public.password")}
      </label>
      <input
        id="public-password"
        class="input"
        type="password"
        autocomplete="off"
        value={password}
        aria-describedby={error ? "public-password-error" : undefined}
        onInput={(e) => setPassword(e.currentTarget.value)}
      />
      {error && (
        <p id="public-password-error" class="error-line" role="alert">
          {error}
        </p>
      )}
      <button type="submit" class="button primary" disabled={busy || password === ""}>
        {t("public.open")}
      </button>
    </form>
  );
}

/** "JPG" from the name, else the second half of the MIME type. */
function typeLabel(e: Entry): string {
  const dot = e.name.lastIndexOf(".");
  if (dot > 0) return e.name.slice(dot + 1).toUpperCase();
  return (e.mime.split("/")[1] ?? "").toUpperCase();
}

type FileProps = { id: string; path: string; entry: Entry; download: boolean; editMax: number };

function FileView({ id, path, entry, download, editMax }: FileProps) {
  const [text, setText] = useState<string | null>(null);
  const image = entry.mime.startsWith("image/");
  const textual = !image && isEditable(entry.mime, entry.name) && entry.size <= editMax;
  useEffect(() => {
    setText(null);
    if (!textual) return;
    let live = true;
    request(rawUrl(id, path), { cache: "no-store" })
      .then((res) => res.arrayBuffer())
      .then((b) => live && setText(new TextDecoder("utf-8").decode(b)))
      .catch(() => live && setText(""));
    return () => {
      live = false;
    };
  }, [id, path]);
  return (
    <article class="public-card">
      <header class="public-head">
        <div class="public-title">
          <h1>{entry.name}</h1>
          <p class="muted">
            {typeLabel(entry)} · {formatSize(entry.size)}
          </p>
        </div>
        {download && (
          <a class="button primary" href={rawUrl(id, path, true)}>
            {t("public.download")}
          </a>
        )}
      </header>
      {image && <img class="public-image" src={rawUrl(id, path)} alt={entry.name} />}
      {textual && text !== null && <pre class="public-text">{text}</pre>}
      {!image && !textual && <p class="muted public-none">{t("public.noPreview")}</p>}
    </article>
  );
}

/** A file inside a shared folder: its entry comes from its folder's listing. */
function FolderFile({ id, path, download, editMax }: Omit<FileProps, "entry">) {
  const [entry, setEntry] = useState<Entry | null>(null);
  const [error, setError] = useState<ApiFailure | null>(null);
  useEffect(() => {
    setEntry(null);
    const name = segments(path).at(-1) ?? "";
    shareList(id, parent(path)).then(
      (l) => {
        const found = l.entries.find((e) => segmentOf(e) === name);
        if (found) setEntry(found);
        else setError(new ApiFailure(404, "not_found", ""));
      },
      (e) => setError(failure(e)),
    );
  }, [id, path]);
  if (error) return <StatePanel icon="alert" tone="danger" title={t("state.gone.title")} body={errorText(error)} />;
  if (!entry) return null;
  return <FileView id={id} path={path} entry={entry} download={download} editMax={editMax} />;
}

function FolderView({ id, path, root, download }: { id: string; path: string; root: string; download: boolean }) {
  const [entries, setEntries] = useState<Entry[] | null>(null);
  const [error, setError] = useState<ApiFailure | null>(null);
  useEffect(() => {
    setEntries(null);
    setError(null);
    shareList(id, path).then(
      (l) => setEntries(l.entries),
      (e) => setError(failure(e)),
    );
  }, [id, path]);
  const crumbs = segments(path);
  const to = (p: string, file = false) => href({ name: "share", id, path: p, file });
  return (
    <article class="public-card">
      <header class="public-head">
        <nav class="public-crumbs" aria-label={root}>
          <Link href={to("")}>{root}</Link>
          {crumbs.map((seg, i) => (
            <>
              <span aria-hidden="true">/</span>
              <Link href={to(crumbs.slice(0, i + 1).join("/"))}>{displaySegment(seg)}</Link>
            </>
          ))}
        </nav>
        {download && (
          <a class="button primary" href={zipUrl(id, path)}>
            {t("public.downloadAll")}
          </a>
        )}
      </header>
      {error && <p class="error-line" role="alert">{errorText(error)}</p>}
      {entries?.length === 0 && <p class="muted public-none">{t("state.empty.title")}</p>}
      {entries && entries.length > 0 && (
        <ul class="public-list">
          {entries.map((e) => {
            const p = join(path, segmentOf(e));
            const isFolder = e.kind === "folder";
            return (
              <li key={p}>
                <Link href={to(p, !isFolder)}>
                  <FileIcon entry={e} />
                  <span class="public-name">{e.name}</span>
                </Link>
                <span class="muted">{isFolder ? "" : formatSize(e.size)}</span>
                <span class="muted">{formatWhen(e.modified)}</span>
              </li>
            );
          })}
        </ul>
      )}
    </article>
  );
}
