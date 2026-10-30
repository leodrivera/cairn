// A folder: toolbar, breadcrumbs, the listing, the selection bar and the item actions. Files dropped on it upload here.
import { useCallback, useEffect, useMemo, useRef, useState } from "preact/hooks";
import { api, ApiFailure, offline } from "../lib/api";
import { apiUrl, displaySegment, join, segmentOf } from "../lib/paths";
import { filterEntries } from "../lib/filter";
import {
  downloadUrl,
  failureOfItem,
  newFolder,
  remove,
  rename,
  startDownload,
  transfer,
  zipForm,
  ZIP_MAX,
  type Failed,
} from "../lib/actions";
import { addTree, entriesFromDrop, filesFromPicker, type TreeFile } from "../lib/uploads";
import { errorText, t, tn } from "../i18n/index";
import { Shell, useNarrow } from "../components/shell";
import { Breadcrumbs } from "../components/breadcrumbs";
import { FileList, opensInViewer } from "../components/file-list";
import { FileGrid } from "../components/file-grid";
import { Icon } from "../components/icons";
import { StatePanel } from "../components/states";
import { ContextMenu, entryMenu } from "../components/context-menu";
import { DeleteDialog, FolderPicker, NewFolderDialog, RenameDialog, folderLabel } from "../components/dialogs";
import { showToast } from "../components/toast";
import { ShareDialog } from "../components/share-dialog";
import { href, navigate } from "../lib/router";
import { Selection } from "../state/selection";
import { useSession, useUser } from "../state/session";
import { uploads, useUploaded } from "../state/uploads";
import type { Entry, Listing } from "../lib/types";

type DialogState =
  | { kind: "rename"; entry: Entry }
  | { kind: "newFolder" }
  | { kind: "move" | "copy"; items: Entry[] }
  | { kind: "delete"; items: Entry[] }
  | { kind: "share"; entry: Entry };

/** The name of the item a batch failure is about, from its path. */
function failedName(f: Failed): string {
  const path = f.from ?? f.path ?? "";
  return displaySegment(path.slice(path.lastIndexOf("/") + 1));
}

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
  const [tools, setTools] = useState(false);
  const narrow = useNarrow();
  const [itemMenu, setItemMenu] = useState<{ entry: Entry; at: { x: number; y: number } } | null>(null);
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const pickFiles = useRef<HTMLInputElement>(null);
  const pickFolder = useRef<HTMLInputElement>(null);

  // Only the latest request may fill the screen: a slow answer for the folder just left must not land under this one,
  // or actions would join this path with that folder's names.
  const latest = useRef(0);
  const load = async () => {
    const n = ++latest.current;
    try {
      const l = await api<Listing>("GET", apiUrl("files", path));
      if (n !== latest.current) return;
      setListing(l);
      setError(null);
    } catch (e) {
      if (n !== latest.current) return;
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

  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e: KeyboardEvent) => {
    if (dialog || itemMenu) return;
    if (e.key === "Escape" && (menu || tools)) {
      setMenu(false);
      setTools(false);
      return;
    }
    // While typing, keys belong to the field (Escape clears a search, Delete deletes a letter).
    const target = e.target as HTMLElement | null;
    if (target?.closest?.("input, textarea, select, [contenteditable]")) return;
    if (e.key === "Escape" && selection.size > 0) {
      selection.clear();
      touch();
      return;
    }
    if (!canWrite || chosen.length === 0) return;
    if (e.key === "F2" && chosen.length === 1) {
      e.preventDefault();
      setDialog({ kind: "rename", entry: chosen[0] });
    } else if (e.key === "Delete") {
      e.preventDefault();
      setDialog({ kind: "delete", items: chosen });
    }
  };

  // An open Upload or More menu closes on a press anywhere outside it.
  useEffect(() => {
    if (!menu && !tools) return;
    const away = (e: Event) => {
      if (!(e.target as HTMLElement | null)?.closest?.(".menu-anchor")) {
        setMenu(false);
        setTools(false);
      }
    };
    document.addEventListener("pointerdown", away);
    return () => document.removeEventListener("pointerdown", away);
  }, [menu, tools]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => keys.current(e);
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

  const clearSelection = () => {
    selection.clear();
    touch();
  };

  /** A long press on a phone selects the item; from then on taps on checkboxes add more. */
  const pressSelect = (e: Entry) => {
    selection.toggle(segmentOf(e));
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

  const downloadItems = (items: Entry[]) => {
    const url = downloadUrl(path, items);
    if (url) startDownload(url);
    else if (items.length > ZIP_MAX) showToast(t("invalid.zipItems"));
    else zipForm(path, items);
  };
  const download = () => downloadItems(chosen);

  /** After a change: the selection is stale, the listing is reloaded, and a toast says what happened. */
  const changed = (text: string) => {
    selection.clear();
    touch();
    load();
    showToast(text);
  };

  const open = (e: Entry) => {
    if (e.outside) return;
    const to = join(path, segmentOf(e));
    if (e.kind === "folder") navigate(href({ name: "files", path: to }));
    else if (opensInViewer(e)) navigate(href({ name: "view", path: to }));
    else downloadItems([e]);
  };

  /** The menu acts on the whole selection when the item is part of one. */
  const targets = (e: Entry) => (selection.has(segmentOf(e)) && chosen.length > 1 ? chosen : [e]);

  const doRename = async (e: Entry, name: string) => {
    await rename(path, e, name);
    changed(t("files.renamed", { name }));
  };

  const doTransfer = async (op: "move" | "copy", items: Entry[], dest: string) => {
    const r = await transfer(op, path, items, dest);
    if (r.failed.length === 0) {
      changed(tn(op === "move" ? "files.moved" : "files.copied", r.done, { dest: folderLabel(dest) }));
    } else {
      const f = r.failed[0];
      changed(
        tn(op === "move" ? "files.movedSome" : "files.copiedSome", items.length, {
          done: r.done,
          name: failedName(f),
          error: errorText(failureOfItem(f)),
        }),
      );
    }
  };

  const doDelete = async (items: Entry[]) => {
    const r = await remove(path, items);
    if (r.failed.length === 0) {
      changed(tn("files.deleted", r.deleted));
    } else {
      const f = r.failed[0];
      changed(
        tn("files.deletedSome", items.length, {
          done: r.deleted,
          name: failedName(f),
          error: errorText(failureOfItem(f)),
        }),
      );
    }
  };

  const doNewFolder = async (name: string) => {
    const entry = await newFolder(path, name);
    changed(t("files.created", { name: entry.name }));
  };

  const closeMenu = useCallback(() => setItemMenu(null), []);
  const closeDialog = () => setDialog(null);

  const openMenu = (entry: Entry, at: { x: number; y: number }) => setItemMenu({ entry, at });

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
          onMore={openMenu}
          onLongPress={narrow ? pressSelect : undefined}
        />
      ) : (
        <FileList
          dir={path}
          entries={shown}
          selection={selection}
          onSelect={select}
          onMore={openMenu}
          onLongPress={narrow ? pressSelect : undefined}
        />
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
        {!narrow && (
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
        )}
        {canWrite && (
          <>
            {!narrow && (
              <button
                type="button"
                class="button"
                aria-label={t("explorer.newFolder")}
                onClick={() => setDialog({ kind: "newFolder" })}
              >
                <Icon name="folderPlus" />
                <span class="label">{t("explorer.newFolder")}</span>
              </button>
            )}
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
        {narrow && (
          <div class="menu-anchor">
            <button
              type="button"
              class="icon-button"
              aria-label={t("explorer.moreTools")}
              aria-haspopup="true"
              aria-expanded={tools}
              onClick={() => setTools(!tools)}
            >
              <Icon name="more" />
            </button>
            {tools && (
              <div class="menu" role="group" aria-label={t("explorer.moreTools")}>
                {canWrite && (
                  <button
                    type="button"
                    class="menu-item"
                    onClick={() => {
                      setTools(false);
                      setDialog({ kind: "newFolder" });
                    }}
                  >
                    <Icon name="folderPlus" />
                    {t("explorer.newFolder")}
                  </button>
                )}
                {(["list", "gallery"] as const).map((v) => (
                  <button
                    key={v}
                    type="button"
                    class="menu-item"
                    aria-pressed={view === v}
                    onClick={() => {
                      setTools(false);
                      switchView(v);
                    }}
                  >
                    <Icon name={v === "list" ? "list" : "grid"} />
                    {t(v === "list" ? "explorer.listView" : "explorer.galleryView")}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
      <div class="crumb-row">
        {narrow && selection.size > 0 ? (
          <div class="select-head" role="toolbar" aria-label={t("explorer.selectionTools")}>
            <button type="button" class="icon-button" aria-label={t("explorer.clearSelection")} onClick={clearSelection}>
              <Icon name="close" />
            </button>
            <span class="count">{tn("explorer.selected", selection.size)}</span>
            <button
              type="button"
              class="button quiet"
              onClick={() => {
                selection.all(order);
                touch();
              }}
            >
              {t("explorer.selectAll")}
            </button>
          </div>
        ) : (
          <Breadcrumbs path={path} />
        )}
        {!narrow && selection.size > 0 && (
          <div class="selection-bar" role="toolbar" aria-label={t("explorer.selectionTools")}>
            <span class="count">{tn("explorer.selected", selection.size)}</span>
            <button type="button" class="button quiet" onClick={download}>
              {t("explorer.download")}
            </button>
            {canWrite && (
              <>
                <button type="button" class="button quiet" onClick={() => setDialog({ kind: "move", items: chosen })}>
                  {t("explorer.move")}
                </button>
                <button type="button" class="button quiet" onClick={() => setDialog({ kind: "copy", items: chosen })}>
                  {t("explorer.copy")}
                </button>
                <button
                  type="button"
                  class="button quiet danger-text"
                  onClick={() => setDialog({ kind: "delete", items: chosen })}
                >
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
      {narrow && selection.size > 0 && (
        <div class="bottom-bar" role="toolbar" aria-label={t("explorer.selectionTools")}>
          <button type="button" class="bar-button" onClick={download}>
            <Icon name="download" />
            <span>{t("explorer.download")}</span>
          </button>
          {canWrite && (
            <>
              <button type="button" class="bar-button" onClick={() => setDialog({ kind: "move", items: chosen })}>
                <Icon name="move" />
                <span>{t("explorer.move")}</span>
              </button>
              <button
                type="button"
                class="bar-button danger-text"
                onClick={() => setDialog({ kind: "delete", items: chosen })}
              >
                <Icon name="trash" />
                <span>{t("explorer.delete")}</span>
              </button>
            </>
          )}
        </div>
      )}
      {itemMenu && (
        <ContextMenu
          at={itemMenu.at}
          label={t("menu.label", { name: itemMenu.entry.name })}
          items={entryMenu(canWrite, user.permissions.share, {
            open: () => open(itemMenu.entry),
            download: () => downloadItems([itemMenu.entry]),
            share: () => setDialog({ kind: "share", entry: itemMenu.entry }),
            rename: () => setDialog({ kind: "rename", entry: itemMenu.entry }),
            move: () => setDialog({ kind: "move", items: targets(itemMenu.entry) }),
            copy: () => setDialog({ kind: "copy", items: targets(itemMenu.entry) }),
            remove: () => setDialog({ kind: "delete", items: targets(itemMenu.entry) }),
          })}
          onClose={closeMenu}
        />
      )}
      {dialog?.kind === "rename" && (
        <RenameDialog entry={dialog.entry} onRename={(n) => doRename(dialog.entry, n)} onClose={closeDialog} />
      )}
      {dialog?.kind === "share" && (
        <ShareDialog
          entryPath={join(path, segmentOf(dialog.entry))}
          kind={dialog.entry.kind === "folder" ? "folder" : "file"}
          onClose={closeDialog}
        />
      )}
      {dialog?.kind === "newFolder" && <NewFolderDialog onCreate={doNewFolder} onClose={closeDialog} />}
      {(dialog?.kind === "move" || dialog?.kind === "copy") && (
        <FolderPicker
          op={dialog.kind}
          dir={path}
          items={dialog.items}
          onPick={(dest) => doTransfer(dialog.kind as "move" | "copy", dialog.items, dest)}
          onClose={closeDialog}
        />
      )}
      {dialog?.kind === "delete" && (
        <DeleteDialog dir={path} items={dialog.items} onConfirm={() => doDelete(dialog.items)} onClose={closeDialog} />
      )}
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
