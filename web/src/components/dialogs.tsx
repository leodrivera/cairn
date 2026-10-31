// Dialogs for item actions: rename, new folder, the folder picker for move and copy, and delete.
import type { ComponentChildren } from "preact";
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import { api, ApiFailure, offline } from "../lib/api";
import { apiUrl, join, parent, segmentOf } from "../lib/paths";
import { errorText, lang, t, tn } from "../i18n/index";
import { Icon } from "./icons";
import type { Entry, Listing } from "../lib/types";
import { useWhere } from "../lib/where";

let nextId = 1;

/** "My files" for the scope root, else the path. */
/** A folder as people read it; the top is "My files", or `root` (a granted folder's name). */
export function folderLabel(dir: string, root?: string): string {
  return dir === "" ? (root ?? t("nav.myFiles")) : dir;
}

type DialogProps = { title: string; onClose(): void; children: ComponentChildren; wide?: boolean };

/** A modal box. Escape closes; Tab stays inside; focus goes back where it was. */
export function Dialog({ title, onClose, children, wide }: DialogProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [id] = useState(() => `dialog-${nextId++}`);
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null;
    // Everything beside the dialog, at each level up to <body>, is inert while it is open, so screen-reader
    // cursors stay inside too. Only what this dialog marked is unmarked.
    const marked: Element[] = [];
    for (let node: Element | null = ref.current!.parentElement; node && node !== document.body; node = node.parentElement) {
      for (const sibling of node.parentElement?.children ?? []) {
        if (sibling !== node && !sibling.hasAttribute("inert")) {
          sibling.setAttribute("inert", "");
          marked.push(sibling);
        }
      }
    }
    return () => {
      for (const el of marked) el.removeAttribute("inert");
      before?.focus?.();
    };
  }, []);
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === "Tab") {
      const all = [
        ...ref.current!.querySelectorAll<HTMLElement>(
          'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
        ),
      ];
      if (!all.length) return;
      const first = all[0];
      const last = all[all.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    }
  };
  return (
    <div class="dialog-backdrop" onKeyDown={onKey}>
      <div ref={ref} class={wide ? "dialog wide" : "dialog"} role="dialog" aria-modal="true" aria-labelledby={id}>
        <h2 id={id} class="dialog-title">
          {title}
        </h2>
        {children}
      </div>
    </div>
  );
}

function failureText(e: unknown): string {
  return errorText(e instanceof ApiFailure ? e : offline());
}

type NameProps = {
  title: string;
  label: string;
  submit: string;
  initial: string;
  select: [number, number];
  onSave(name: string): Promise<void>;
  onClose(): void;
};

function NameDialog({ title, label, submit, initial, select, onSave, onClose }: NameProps) {
  const input = useRef<HTMLInputElement>(null);
  const [value, setValue] = useState(initial);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [id] = useState(() => `name-${nextId++}`);

  useLayoutEffect(() => {
    const el = input.current!;
    el.focus();
    el.setSelectionRange(select[0], select[1]);
  }, []);

  const onSubmit = async (e: Event) => {
    e.preventDefault();
    if (busy) return;
    const name = value.trim();
    if (name === "" || name.includes("/")) {
      setError(t("dialog.nameRule"));
      return;
    }
    setBusy(true);
    setError("");
    try {
      await onSave(name);
      onClose();
    } catch (err) {
      setError(failureText(err));
      setBusy(false);
    }
  };

  return (
    <Dialog title={title} onClose={onClose}>
      <form class="dialog-form" onSubmit={onSubmit} noValidate>
        <label class="field-label" for={id}>
          {label}
        </label>
        <input
          ref={input}
          id={id}
          class="input"
          value={value}
          autocomplete="off"
          spellcheck={false}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? `${id}-error` : undefined}
          onInput={(e) => setValue(e.currentTarget.value)}
        />
        {error && (
          <p id={`${id}-error`} class="error-line" role="alert">
            {error}
          </p>
        )}
        <div class="dialog-buttons">
          <button type="button" class="button" onClick={onClose}>
            {t("dialog.cancel")}
          </button>
          <button type="submit" class="button primary" disabled={busy}>
            {submit}
          </button>
        </div>
      </form>
    </Dialog>
  );
}

export function RenameDialog({
  entry,
  onRename,
  onClose,
}: {
  entry: Entry;
  onRename(name: string): Promise<void>;
  onClose(): void;
}) {
  const dot = entry.name.lastIndexOf(".");
  const end = entry.kind !== "folder" && dot > 0 ? dot : entry.name.length;
  return (
    <NameDialog
      title={t("menu.rename")}
      label={t("dialog.newName")}
      submit={t("menu.rename")}
      initial={entry.name}
      select={[0, end]}
      onSave={(name) => (name === entry.name ? Promise.resolve() : onRename(name))}
      onClose={onClose}
    />
  );
}

export function NewFolderDialog({ onCreate, onClose }: { onCreate(name: string): Promise<void>; onClose(): void }) {
  return (
    <NameDialog
      title={t("explorer.newFolder")}
      label={t("dialog.folderName")}
      submit={t("dialog.create")}
      initial=""
      select={[0, 0]}
      onSave={onCreate}
      onClose={onClose}
    />
  );
}

export function NewTextFileDialog({ onCreate, onClose }: { onCreate(name: string): Promise<void>; onClose(): void }) {
  return (
    <NameDialog
      title={t("explorer.newTextFile")}
      label={t("dialog.fileName")}
      submit={t("dialog.create")}
      initial="untitled.txt"
      select={[0, "untitled".length]}
      onSave={onCreate}
      onClose={onClose}
    />
  );
}

type PickerProps = {
  op: "move" | "copy";
  dir: string;
  items: Entry[];
  onPick(dest: string): Promise<void>;
  onClose(): void;
};

/** Browses folders from the scope root. The current folder and anything inside a chosen folder cannot be picked. */
export function FolderPicker({ op, dir, items, onPick, onClose }: PickerProps) {
  const where = useWhere();
  const [at, setAt] = useState("");
  const [folders, setFolders] = useState<Entry[] | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const cancel = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let live = true;
    setFolders(null);
    setError("");
    api<Listing>("GET", apiUrl("files", at, undefined, where.api)).then(
      (l) => live && setFolders(l.entries.filter((e) => e.kind === "folder" && !e.outside)),
      (e) => live && setError(failureText(e)),
    );
    return () => {
      live = false;
    };
  }, [at]);

  useLayoutEffect(() => cancel.current?.focus(), []);

  const chosen = items.filter((e) => e.kind === "folder").map((e) => join(dir, segmentOf(e)));
  const blocked = at === dir || chosen.some((c) => at === c || at.startsWith(`${c}/`));

  const pick = async () => {
    setBusy(true);
    try {
      await onPick(at);
      onClose();
    } catch (e) {
      setError(failureText(e));
      setBusy(false);
    }
  };

  return (
    <Dialog title={tn(op === "move" ? "dialog.moveTitle" : "dialog.copyTitle", items.length)} onClose={onClose} wide>
      <div class="picker-head">
        <button
          type="button"
          class="icon-button"
          aria-label={t("dialog.up")}
          disabled={at === ""}
          onClick={() => setAt(parent(at))}
        >
          <Icon name="back" />
        </button>
        <span class="mono picker-path">{folderLabel(at, where.grant?.name)}</span>
      </div>
      <ul class="picker-list" aria-busy={folders === null}>
        {folders?.map((f) => (
          <li key={segmentOf(f)}>
            <button type="button" class="picker-item" onClick={() => setAt(join(at, segmentOf(f)))}>
              <Icon name="folder" />
              <span>{f.name}</span>
            </button>
          </li>
        ))}
        {folders?.length === 0 && <li class="picker-empty muted">{t("dialog.noFolders")}</li>}
      </ul>
      {error && (
        <p class="error-line" role="alert">
          {error}
        </p>
      )}
      <div class="dialog-buttons">
        <button ref={cancel} type="button" class="button" onClick={onClose}>
          {t("dialog.cancel")}
        </button>
        <button type="button" class="button primary" disabled={blocked || busy} onClick={pick}>
          {t(op === "move" ? "dialog.moveHere" : "dialog.copyHere")}
        </button>
      </div>
    </Dialog>
  );
}

/** Up to three names, then "and {k} more". */
export function nameList(items: Entry[]): string {
  const names = items.slice(0, 3).map((e) => e.name);
  if (items.length > 3) names.push(tn("dialog.more", items.length - 3));
  return new Intl.ListFormat(lang(), { type: "conjunction" }).format(names);
}

/**
 * `trash`: moves to the trash, with a permanent delete behind a second question. `permanent`: the trash is off.
 * `grant`: inside a grant, only the trash. `instead`: the trash refused these items (`reason` says why); offers a
 * permanent delete.
 */
export type DeleteMode = "trash" | "permanent" | "grant" | "instead";

type DeleteProps = {
  dir: string;
  items: Entry[];
  mode: DeleteMode;
  reason?: string;
  onConfirm(permanent: boolean): Promise<void>;
  onClose(): void;
};

export function DeleteDialog({ dir, items, mode, reason, onConfirm, onClose }: DeleteProps) {
  const cancel = useRef<HTMLButtonElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [forever, setForever] = useState(mode === "permanent" || mode === "instead");
  useLayoutEffect(() => cancel.current?.focus(), [forever]);
  const confirm = async (permanent: boolean) => {
    setBusy(true);
    try {
      await onConfirm(permanent);
      onClose();
    } catch (e) {
      setError(failureText(e));
      setBusy(false);
    }
  };
  const names = nameList(items);
  let title: string;
  let body: string;
  if (mode === "permanent") {
    title = tn("dialog.deleteTitle", items.length);
    body = `${t("dialog.deleteBody", { names, folder: folderLabel(dir) })} ${t("dialog.trashOff")}`;
  } else if (mode === "instead") {
    title = tn("dialog.foreverTitle", items.length);
    body = t("dialog.insteadBody", { names });
  } else if (forever) {
    title = tn("dialog.foreverTitle", items.length);
    body = t("dialog.foreverBody", { names });
  } else {
    title = tn("dialog.trashTitle", items.length);
    body = t("dialog.trashBody", { names });
  }
  return (
    <Dialog title={title} onClose={onClose}>
      {reason && <p class="dialog-body">{reason}</p>}
      <p class="dialog-body">{body}</p>
      {error && (
        <p class="error-line" role="alert">
          {error}
        </p>
      )}
      <div class="dialog-buttons">
        <button ref={cancel} type="button" class="button" onClick={onClose}>
          {t("dialog.cancel")}
        </button>
        {forever ? (
          <button type="button" class="button danger" disabled={busy} onClick={() => confirm(true)}>
            {mode === "permanent" ? t("explorer.delete") : t("dialog.deleteForever")}
          </button>
        ) : (
          <>
            {mode === "trash" && (
              <button type="button" class="button quiet danger-text" disabled={busy} onClick={() => setForever(true)}>
                {t("dialog.deleteForever")}
              </button>
            )}
            <button type="button" class="button primary" disabled={busy} onClick={() => confirm(false)}>
              {t("dialog.moveToTrash")}
            </button>
          </>
        )}
      </div>
    </Dialog>
  );
}
