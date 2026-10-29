// A folder: toolbar, breadcrumbs, the listing, and the selection bar. Files dropped on it upload here.
import { useCallback, useEffect, useMemo, useRef, useState } from "preact/hooks";
import { api, ApiFailure, offline } from "../lib/api";
import { apiUrl, segmentOf } from "../lib/paths";
import { filterEntries } from "../lib/filter";
import { downloadUrl, zipForm } from "../lib/actions";
import { addTree, entriesFromDrop, filesFromPicker, type TreeFile } from "../lib/uploads";
import { errorText, t, tn } from "../i18n/index";
import { Shell } from "../components/shell";
import { Breadcrumbs } from "../components/breadcrumbs";
import { FileList } from "../components/file-list";
import { FileGrid } from "../components/file-grid";
import { Icon } from "../components/icons";
import { StatePanel } from "../components/states";
import { Selection } from "../state/selection";
import { useSession, useUser } from "../state/session";
import { uploads, useUploaded } from "../state/uploads";
import type { Entry, Listing } from "../lib/types";

export function Explorer({ path }: { path: string }) {
  const user = useUser();
  const { session, setUser } = useSession();
  const view = user.view ?? "list";
  const canWrite = user.permissions.write;
  const [listing, setListing] = useState<Listing | null>(null);
  const [error, setError] = useState<ApiFailure | null>(null);
  const [query, setQuery] = useState("");
  const [selection] = useState(() => new Selection());
  const [, setTick] = useState(0);
  const touch = () => setTick((n) => n + 1);
  const [menu, setMenu] = useState(false);
  const pickFiles = useRef<HTMLInputElement>(null);
  const pickFolder = useRef<HTMLInputElement>(null);

  const load = async () => {
    try {
      const l = await api<Listing>("GET", apiUrl("files", path));
      setListing(l);
      setError(null);
    } catch (e) {
      setError(e instanceof ApiFailure ? e : offline());
    }
  };

  useEffect(() => {
    setListing(null);
    setError(null);
    setQuery("");
    selection.clear();
    load();
  }, [path]);

  useUploaded(
    useCallback(
      (dir: string) => {
        if (dir === path) load();
      },
      [path],
    ),
  );

  const upload = (files: TreeFile[]) => {
    if (files.length) addTree(uploads(), path, files);
  };

  const picked = (e: Event) => {
    const input = e.currentTarget as HTMLInputElement;
    setMenu(false);
    upload(filesFromPicker(input.files ?? []));
    input.value = "";
  };

  const onDragOver = (e: DragEvent) => {
    if (!canWrite || !e.dataTransfer?.types.includes("Files")) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
  };

  const onDrop = async (e: DragEvent) => {
    if (!canWrite || !e.dataTransfer) return;
    e.preventDefault();
    const dt = e.dataTransfer;
    const entries = [...dt.items]
      .filter((i) => i.kind === "file")
      .map((i) => i.webkitGetAsEntry?.())
      .filter((x): x is FileSystemEntry => !!x);
    upload(entries.length ? await entriesFromDrop(entries) : filesFromPicker(dt.files));
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && selection.size > 0) {
        selection.clear();
        touch();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  const shown = useMemo(() => filterEntries(listing?.entries ?? [], query), [listing, query]);
  const order = shown.map(segmentOf);
  const chosen = (listing?.entries ?? []).filter((e) => selection.has(segmentOf(e)));

  const select = (e: Entry, ev: MouseEvent) => {
    const seg = segmentOf(e);
    if (ev.shiftKey) selection.range(seg, order);
    else selection.toggle(seg);
    touch();
  };

  const switchView = async (next: "list" | "gallery") => {
    if (next === view) return;
    setUser({ ...user, view: next });
    try {
      const res = await api<{ user: typeof user }>("PATCH", "/api/me", { view: next });
      setUser(res.user);
    } catch {
      // The choice still applies on this page; it just is not remembered.
    }
  };

  const download = () => {
    const url = downloadUrl(path, chosen);
    if (url) location.assign(url);
    else zipForm(path, chosen);
  };

  let content;
  if (error) {
    content = <ListingError error={error} retry={load} />;
  } else if (!listing) {
    content = null;
  } else if (listing.entries.length === 0) {
    content = (
      <StatePanel icon="folder" tone="accent" title={t("state.empty.title")} body={t("state.empty.body")}>
        {canWrite && (
          <button type="button" class="button primary" onClick={() => pickFiles.current?.click()}>
            <Icon name="upload" />
            {t("explorer.upload")}
          </button>
        )}
      </StatePanel>
    );
  } else if (shown.length === 0) {
    content = (
      <StatePanel icon="search" title={t("state.noResults.title", { q: query.trim() })} body={t("state.noResults.body")}>
        <button type="button" class="button" onClick={() => setQuery("")}>
          {t("state.noResults.clear")}
        </button>
      </StatePanel>
    );
  } else {
    content =
      view === "gallery" ? (
        <FileGrid
          dir={path}
          entries={shown}
          selection={selection}
          thumbnails={session!.config.thumbnails}
          onSelect={select}
        />
      ) : (
        <FileList dir={path} entries={shown} selection={selection} onSelect={select} />
      );
  }

  return (
    <Shell space={listing?.space} class="files" onDragOver={onDragOver} onDrop={onDrop}>
      <div class="toolbar">
        <div class="search">
          <label for="explorer-search" class="visually-hidden">
            {t("explorer.search")}
          </label>
          <Icon name="search" class="search-icon" />
          <input
            id="explorer-search"
            class="input"
            type="search"
            placeholder={t("explorer.search")}
            value={query}
            onInput={(e) => setQuery(e.currentTarget.value)}
          />
        </div>
        <div class="segmented" role="group" aria-label={t("explorer.view")}>
          <button
            type="button"
            class="icon-button"
            aria-label={t("explorer.listView")}
            aria-pressed={view === "list"}
            onClick={() => switchView("list")}
          >
            <Icon name="list" />
          </button>
          <button
            type="button"
            class="icon-button"
            aria-label={t("explorer.galleryView")}
            aria-pressed={view === "gallery"}
            onClick={() => switchView("gallery")}
          >
            <Icon name="grid" />
          </button>
        </div>
        {canWrite && (
          <>
            <button type="button" class="button" aria-label={t("explorer.newFolder")}>
              <Icon name="folderPlus" />
              <span class="label">{t("explorer.newFolder")}</span>
            </button>
            <div class="menu-anchor">
              <button
                type="button"
                class="button primary"
                aria-haspopup="true"
                aria-expanded={menu}
                onClick={() => setMenu(!menu)}
              >
                <Icon name="upload" />
                {t("explorer.upload")}
              </button>
              {menu && (
                <div class="menu" role="group" aria-label={t("upload.choose")}>
                  <button type="button" class="menu-item" onClick={() => pickFiles.current?.click()}>
                    <Icon name="file" />
                    {t("upload.files")}
                  </button>
                  <button type="button" class="menu-item" onClick={() => pickFolder.current?.click()}>
                    <Icon name="folder" />
                    {t("upload.folder")}
                  </button>
                </div>
              )}
            </div>
            <input ref={pickFiles} data-pick="files" type="file" multiple hidden onChange={picked} />
            <input
              ref={pickFolder}
              data-pick="folder"
              type="file"
              multiple
              hidden
              {...{ webkitdirectory: "" }}
              onChange={picked}
            />
          </>
        )}
      </div>
      <div class="crumb-row">
        <Breadcrumbs path={path} />
        {selection.size > 0 && (
          <div class="selection-bar" role="toolbar" aria-label={t("explorer.selectionTools")}>
            <span class="count">{tn("explorer.selected", selection.size)}</span>
            <button type="button" class="button quiet" onClick={download}>
              {t("explorer.download")}
            </button>
            {canWrite && (
              <>
                <button type="button" class="button quiet">
                  {t("explorer.move")}
                </button>
                <button type="button" class="button quiet">
                  {t("explorer.copy")}
                </button>
                <button type="button" class="button quiet danger-text">
                  {t("explorer.delete")}
                </button>
              </>
            )}
            <button
              type="button"
              class="icon-button"
              aria-label={t("explorer.clearSelection")}
              onClick={() => {
                selection.clear();
                touch();
              }}
            >
              <Icon name="close" />
            </button>
          </div>
        )}
      </div>
      {content}
    </Shell>
  );
}

function ListingError({ error, retry }: { error: ApiFailure; retry(): void }) {
  const back = (
    <a class="button" href="/files/">
      {t("state.backToFiles")}
    </a>
  );
  if (error.code === "forbidden" || error.code === "outside") {
    return (
      <StatePanel icon="lock" tone="danger" title={t("state.forbidden.title")} body={t("state.forbidden.body")}>
        {back}
      </StatePanel>
    );
  }
  if (error.code === "not_found" || error.code === "not_a_folder") {
    return (
      <StatePanel icon="folder" title={t("state.gone.title")} body={t("state.gone.body")}>
        {back}
      </StatePanel>
    );
  }
  return (
    <StatePanel icon="alert" tone="danger" title={t("state.failed.listing")} body={errorText(error)}>
      <button type="button" class="button primary" onClick={retry}>
        {t("state.tryAgain")}
      </button>
    </StatePanel>
  );
}
