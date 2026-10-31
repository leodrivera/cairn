// Uploads: a queue in the browser with progress, retry, replace and cancel; folders are created first.
import { api, ApiFailure, failureOf, offline, probe, reportSignedOut } from "./api";
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
  /** The API base of the place: `/api`, or `/api/g/{id}` inside a grant. */
  base: string;
  size: number;
  sent: number;
  state: UploadState;
  error: ApiFailure | null;
};

type Job = { item: UploadItem; file: Blob; replace: boolean; abort: AbortController | null };

let nextId = 1;

/** The same folder path in two places (own files, a grant) is two batches. */
const rootKey = (i: UploadItem) => `${i.base}\u0000${i.root}`;

const isActive = (s: UploadState) => s === "queued" || s === "sending" || s === "exists";

/** Counts the panel reads on every render, kept as items change state instead of scanning the list. */
export type UploadCounts = { total: number; cancelled: number; done: number; failed: number; active: number };

/**
 * Every state change goes through `move`, which keeps the counters, so a batch of 20 000 files costs linear time:
 * the waiting line is a FIFO with a moving head, and a root's batch is settled when its active count reaches zero.
 */
export class UploadQueue {
  items: UploadItem[] = [];
  onDone: (dir: string, base: string) => void = () => {};
  counts: UploadCounts = { total: 0, cancelled: 0, done: 0, failed: 0, active: 0 };
  private jobs = new Map<number, Job>();
  private listeners = new Set<() => void>();
  private waiting: number[] = [];
  private head = 0;
  private running = 0;
  private activeByRoot = new Map<string, number>();
  private doneByRoot = new Map<string, number>();
  private progressTimer: ReturnType<typeof setTimeout> | null = null;
  private progressPending = false;
  private quiet = false;

  constructor(
    private send: Transport,
    private onChange: () => void,
    private concurrency = 2,
  ) {}

  /** Calls `fn` after every change, besides the constructor's onChange. Returns the unsubscribe. */
  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Queues files into `dir`. `labels` (same order) name them in the panel; the file name by default. */
  add(dir: string, files: File[], into: string[] = [], labels: string[] = [], root = dir, base = "/api"): void {
    files.forEach((file, i) => {
      const item = this.push(root, into[i] ?? dir, file, labels[i] ?? file.name, "queued", null, base);
      this.jobs.set(item.id, { item, file, replace: false, abort: null });
      this.waiting.push(item.id);
    });
    this.changed();
    this.pump();
  }

  /** An item that never started because its folder could not be made. */
  addFailed(root: string, dir: string, file: File, label: string, error: ApiFailure, base = "/api"): void {
    this.push(root, dir, file, label, "failed", error, base);
    this.changed();
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

  /** One notification for the whole batch, not one per item. */
  cancelAll(): void {
    this.quiet = true;
    try {
      for (const item of this.items) this.cancel(item.id);
    } finally {
      this.quiet = false;
    }
    this.changed();
  }

  clearFinished(): void {
    this.items = this.items.filter((i) => isActive(i.state));
    for (const [id, job] of this.jobs) if (!isActive(job.item.state)) this.jobs.delete(id);
    this.counts = { total: this.items.length, cancelled: 0, done: 0, failed: 0, active: this.items.length };
    this.doneByRoot.clear();
    this.changed();
  }

  private push(
    root: string,
    dir: string,
    file: File,
    label: string,
    state: UploadState,
    error: ApiFailure | null,
    base = "/api",
  ) {
    const item: UploadItem = {
      id: nextId++,
      root,
      dir,
      name: file.name,
      label,
      base,
      size: file.size,
      sent: 0,
      state: "cancelled",
      error,
    };
    this.items.push(item);
    this.counts.total++;
    this.counts.cancelled++;
    this.move(item, state);
    return item;
  }

  /** The one place an item changes state; keeps every counter right. */
  private move(item: UploadItem, to: UploadState): void {
    const from = item.state;
    if (from === to) return;
    const c = this.counts;
    if (from === "cancelled") c.cancelled--;
    if (from === "done") {
      c.done--;
      this.doneByRoot.set(rootKey(item), (this.doneByRoot.get(rootKey(item)) ?? 1) - 1);
    }
    if (from === "failed") c.failed--;
    if (from === "sending") this.running--;
    if (to === "cancelled") c.cancelled++;
    if (to === "done") {
      c.done++;
      this.doneByRoot.set(rootKey(item), (this.doneByRoot.get(rootKey(item)) ?? 0) + 1);
    }
    if (to === "failed") c.failed++;
    if (to === "sending") this.running++;
    const was = isActive(from);
    const is = isActive(to);
    if (was !== is) {
      const d = is ? 1 : -1;
      c.active += d;
      this.activeByRoot.set(rootKey(item), (this.activeByRoot.get(rootKey(item)) ?? 0) + d);
    }
    item.state = to;
  }

  private requeue(id: number, replace: boolean): void {
    const job = this.jobs.get(id);
    if (!job || job.item.state === "sending" || job.item.state === "queued") return;
    job.replace = replace;
    job.item.sent = 0;
    job.item.error = null;
    this.move(job.item, "queued");
    this.waiting.push(id);
    this.changed();
    this.pump();
  }

  private changed(): void {
    if (this.quiet) return;
    this.onChange();
    for (const fn of this.listeners) fn();
  }

  /** Progress events come many times a second per file: announce the first at once, then at most every 100 ms. */
  private progressed(): void {
    if (this.progressTimer) {
      this.progressPending = true;
      return;
    }
    this.changed();
    this.progressTimer = setTimeout(() => {
      this.progressTimer = null;
      if (this.progressPending) {
        this.progressPending = false;
        this.changed();
      }
    }, 100);
  }

  private pump(): void {
    while (this.running < this.concurrency && this.head < this.waiting.length) {
      const job = this.jobs.get(this.waiting[this.head++]);
      if (job && job.item.state === "queued") this.start(job);
    }
    if (this.head > 1024 && this.head * 2 > this.waiting.length) {
      this.waiting = this.waiting.slice(this.head);
      this.head = 0;
    }
  }

  private async start(job: Job): Promise<void> {
    const { item } = job;
    this.move(item, "sending");
    const abort = new AbortController();
    job.abort = abort;
    this.changed();
    const url = apiUrl("upload", join(item.dir, item.name), job.replace ? { replace: "1" } : undefined, item.base);
    try {
      const res = await this.send(
        url,
        job.file,
        (sent) => {
          item.sent = sent;
          this.progressed();
        },
        abort.signal,
      );
      if (abort.signal.aborted) return;
      if (res.status >= 200 && res.status < 300) {
        item.sent = item.size;
        this.finish(job, "done", null);
      } else {
        const f = await failureOf(new Response(res.body, { status: res.status }));
        if (f.code === "signed_out") reportSignedOut();
        this.finish(job, f.code === "exists" && !job.replace ? "exists" : "failed", f);
      }
    } catch {
      if (abort.signal.aborted) return;
      this.finish(job, "failed", offline());
    }
  }

  private finish(job: Job, state: UploadState, error: ApiFailure | null): void {
    const wasSending = job.item.state === "sending";
    this.move(job.item, state);
    job.item.error = error;
    job.abort = null;
    this.changed();
    if (wasSending) this.pump();
    this.settle(job.item);
  }

  /** Tells the explorer to reload a folder once nothing for it is still on its way. */
  private settle(item: UploadItem): void {
    const key = rootKey(item);
    if ((this.activeByRoot.get(key) ?? 0) > 0) return;
    if ((this.doneByRoot.get(key) ?? 0) > 0) this.onDone(item.root, item.base);
  }
}

/**
 * Creates a folder, or accepts one that is already there; a file in the way fails with not_a_folder. What is there
 * is asked with a one-byte read, never a listing: the folder may hold 50 000 entries.
 */
export async function ensureFolder(path: string, base = "/api"): Promise<void> {
  try {
    await api("POST", apiUrl("folders", path, undefined, base));
    return;
  } catch (e) {
    if (!(e instanceof ApiFailure) || e.code !== "exists") throw e;
  }
  try {
    await probe(apiUrl("raw", path, undefined, base));
  } catch (e) {
    if (e instanceof ApiFailure && e.code === "is_a_folder") return;
    throw e;
  }
  throw new ApiFailure(409, "not_a_folder", "");
}

export type TreeFile = { relPath: string; file: File | null };

/** Uploads a dropped or picked folder tree into `dir`: every folder first, parents before children, then files. */
export async function addTree(
  queue: UploadQueue,
  dir: string,
  files: TreeFile[],
  mkdir?: (path: string) => Promise<void>,
  base = "/api",
): Promise<void> {
  const make = mkdir ?? ((path: string) => ensureFolder(path, base));
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
      await make(join(dir, rel));
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
    if (blocked) queue.addFailed(dir, into, file, relPath, blocked[1], base);
    else ok.push({ file, into, label: relPath });
  }
  if (ok.length) {
    queue.add(
      dir,
      ok.map((o) => o.file),
      ok.map((o) => o.into),
      ok.map((o) => o.label),
      dir,
      base,
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
