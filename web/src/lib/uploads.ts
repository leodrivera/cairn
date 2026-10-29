// Uploads: a queue in the browser with progress, retry, replace and cancel; folders are created first.
import { api, ApiFailure, failureOf, offline } from "./api";
import { apiUrl, join, parent } from "./paths";

export type Transport = (
  url: string,
  file: Blob,
  onProgress: (sent: number) => void,
  signal: AbortSignal,
) => Promise<{ status: number; body: string }>;

/** PUT through XMLHttpRequest, the only browser API that reports upload progress. */
export const xhrTransport: Transport = (url, file, onProgress, signal) =>
  new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("PUT", url);
    xhr.setRequestHeader("X-Cairn", "1");
    xhr.upload.onprogress = (e) => onProgress(e.loaded);
    xhr.onload = () => resolve({ status: xhr.status, body: xhr.responseText });
    xhr.onerror = () => reject(new TypeError("network error"));
    xhr.onabort = () => reject(new DOMException("aborted", "AbortError"));
    signal.addEventListener("abort", () => xhr.abort(), { once: true });
    xhr.send(file);
  });

export type UploadState = "queued" | "sending" | "done" | "failed" | "cancelled" | "exists";

export type UploadItem = {
  id: number;
  /** The folder the person uploaded into. */
  root: string;
  /** The folder the file lands in (root, or a folder inside it for folder uploads). */
  dir: string;
  name: string;
  /** What the panel shows: the path relative to the drop. */
  label: string;
  size: number;
  sent: number;
  state: UploadState;
  error: ApiFailure | null;
};

type Job = { item: UploadItem; file: Blob; replace: boolean; abort: AbortController | null };

let nextId = 1;

export class UploadQueue {
  items: UploadItem[] = [];
  onDone: (dir: string) => void = () => {};
  private jobs = new Map<number, Job>();
  private listeners = new Set<() => void>();

  constructor(
    private send: Transport,
    private onChange: () => void,
    private concurrency = 2,
  ) {}

  /** Queues files into `dir`. `labels` (same order) name them in the panel; the file name by default. */
  add(dir: string, files: File[], into: string[] = [], labels: string[] = [], root = dir): void {
    files.forEach((file, i) => {
      const item: UploadItem = {
        id: nextId++,
        root,
        dir: into[i] ?? dir,
        name: file.name,
        label: labels[i] ?? file.name,
        size: file.size,
        sent: 0,
        state: "queued",
        error: null,
      };
      this.items.push(item);
      this.jobs.set(item.id, { item, file, replace: false, abort: null });
    });
    this.changed();
    this.pump();
  }

  /** An item that never started because its folder could not be made. */
  addFailed(root: string, dir: string, file: File, label: string, error: ApiFailure): void {
    this.items.push({
      id: nextId++,
      root,
      dir,
      name: file.name,
      label,
      size: file.size,
      sent: 0,
      state: "failed",
      error,
    });
    this.changed();
  }

  /** Calls `fn` after every change, besides the constructor's onChange. Returns the unsubscribe. */
  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  retry(id: number): void {
    this.requeue(id, false);
  }

  replace(id: number): void {
    this.requeue(id, true);
  }

  skip(id: number): void {
    const job = this.jobs.get(id);
    if (!job || job.item.state !== "exists") return;
    this.finish(job, "cancelled", null);
  }

  cancel(id: number): void {
    const job = this.jobs.get(id);
    if (!job) return;
    const s = job.item.state;
    if (s === "done" || s === "cancelled") return;
    job.abort?.abort();
    this.finish(job, "cancelled", null);
  }

  cancelAll(): void {
    for (const item of this.items) this.cancel(item.id);
  }

  clearFinished(): void {
    this.items = this.items.filter((i) => i.state === "queued" || i.state === "sending" || i.state === "exists");
    for (const id of [...this.jobs.keys()]) {
      if (!this.items.some((i) => i.id === id)) this.jobs.delete(id);
    }
    this.changed();
  }

  private requeue(id: number, replace: boolean): void {
    const job = this.jobs.get(id);
    if (!job || job.item.state === "sending" || job.item.state === "queued") return;
    job.replace = replace;
    job.item.sent = 0;
    job.item.error = null;
    job.item.state = "queued";
    this.changed();
    this.pump();
  }

  private changed(): void {
    this.onChange();
    for (const fn of this.listeners) fn();
  }

  private pump(): void {
    let running = this.items.filter((i) => i.state === "sending").length;
    for (const item of this.items) {
      if (running >= this.concurrency) break;
      if (item.state !== "queued") continue;
      const job = this.jobs.get(item.id);
      if (!job) continue;
      running++;
      this.start(job);
    }
  }

  private async start(job: Job): Promise<void> {
    const { item } = job;
    item.state = "sending";
    const abort = new AbortController();
    job.abort = abort;
    this.changed();
    const url = apiUrl("upload", join(item.dir, item.name), job.replace ? { replace: "1" } : undefined);
    try {
      const res = await this.send(
        url,
        job.file,
        (sent) => {
          item.sent = sent;
          this.changed();
        },
        abort.signal,
      );
      if (abort.signal.aborted) return;
      if (res.status >= 200 && res.status < 300) {
        item.sent = item.size;
        this.finish(job, "done", null);
      } else {
        const f = await failureOf(new Response(res.body, { status: res.status }));
        this.finish(job, f.code === "exists" && !job.replace ? "exists" : "failed", f);
      }
    } catch {
      if (abort.signal.aborted) return;
      this.finish(job, "failed", offline());
    }
  }

  private finish(job: Job, state: UploadState, error: ApiFailure | null): void {
    const wasActive = job.item.state === "sending";
    job.item.state = state;
    job.item.error = error;
    job.abort = null;
    this.changed();
    if (wasActive) this.pump();
    this.settle(job.item.root);
  }

  /** Tells the explorer to reload a folder once nothing for it is still on its way. */
  private settle(root: string): void {
    const batch = this.items.filter((i) => i.root === root);
    if (batch.some((i) => i.state === "queued" || i.state === "sending" || i.state === "exists")) return;
    if (batch.some((i) => i.state === "done")) this.onDone(root);
  }
}

/** Creates a folder, or accepts one that is already there; a file in the way fails with not_a_folder. */
export async function ensureFolder(path: string): Promise<void> {
  try {
    await api("POST", apiUrl("folders", path));
  } catch (e) {
    if (!(e instanceof ApiFailure) || e.code !== "exists") throw e;
    await api("GET", apiUrl("files", path));
  }
}

export type TreeFile = { relPath: string; file: File | null };

/** Uploads a dropped or picked folder tree into `dir`: every folder first, parents before children, then files. */
export async function addTree(
  queue: UploadQueue,
  dir: string,
  files: TreeFile[],
  mkdir: (path: string) => Promise<void> = ensureFolder,
): Promise<void> {
  const folders: string[] = [];
  const seen = new Set<string>();
  for (const { relPath, file } of files) {
    const parts = relPath.split("/").filter(Boolean);
    const depth = file ? parts.length - 1 : parts.length;
    for (let i = 1; i <= depth; i++) {
      const p = parts.slice(0, i).join("/");
      if (!seen.has(p)) {
        seen.add(p);
        folders.push(p);
      }
    }
  }
  const failed = new Map<string, ApiFailure>();
  for (const rel of folders) {
    const above = [...failed.keys()].find((f) => rel.startsWith(`${f}/`));
    if (above) continue;
    try {
      await mkdir(join(dir, rel));
    } catch (e) {
      failed.set(rel, e instanceof ApiFailure ? e : offline());
    }
  }
  const ok: { file: File; into: string; label: string }[] = [];
  for (const { relPath, file } of files) {
    if (!file) continue;
    const folder = parent(relPath);
    const into = folder === "" ? dir : join(dir, folder);
    const blocked = [...failed.entries()].find(([f]) => folder === f || folder.startsWith(`${f}/`));
    if (blocked) queue.addFailed(dir, into, file, relPath, blocked[1]);
    else ok.push({ file, into, label: relPath });
  }
  if (ok.length) {
    queue.add(
      dir,
      ok.map((o) => o.file),
      ok.map((o) => o.into),
      ok.map((o) => o.label),
    );
  }
}

/** Files from `<input type=file>`, with their path inside a picked folder when there is one. */
export function filesFromPicker(list: ArrayLike<File>): TreeFile[] {
  return Array.from(list, (file) => ({ relPath: file.webkitRelativePath || file.name, file }));
}

/** Walks dropped entries. Directory readers hand out entries in batches until an empty one. */
export async function entriesFromDrop(entries: FileSystemEntry[]): Promise<TreeFile[]> {
  const out: TreeFile[] = [];
  const walk = async (entry: FileSystemEntry, prefix: string): Promise<void> => {
    if (entry.isFile) {
      const file = await new Promise<File>((ok, fail) => (entry as FileSystemFileEntry).file(ok, fail));
      out.push({ relPath: prefix + entry.name, file });
      return;
    }
    if (!entry.isDirectory) return;
    const reader = (entry as FileSystemDirectoryEntry).createReader();
    const children: FileSystemEntry[] = [];
    for (;;) {
      const batch = await new Promise<FileSystemEntry[]>((ok, fail) => reader.readEntries(ok, fail));
      if (batch.length === 0) break;
      children.push(...batch);
    }
    if (children.length === 0) out.push({ relPath: `${prefix}${entry.name}/`, file: null });
    for (const child of children) await walk(child, `${prefix}${entry.name}/`);
  };
  for (const e of entries) await walk(e, "");
  return out;
}
