// The one upload queue of the page: shared by every folder, so uploads keep going while the person moves around.
import { useEffect, useState } from "preact/hooks";
import { UploadQueue, xhrTransport, type Transport } from "../lib/uploads";

let queue: UploadQueue | null = null;
const uploaded = new Set<(dir: string, base: string) => void>();

function make(send: Transport): UploadQueue {
  const q = new UploadQueue(send, () => {});
  q.onDone = (dir, base) => {
    for (const fn of uploaded) fn(dir, base);
  };
  return q;
}

// Closing or reloading the tab would cut running uploads off, so the browser asks first.
if (typeof window !== "undefined") {
  window.addEventListener("beforeunload", (e) => {
    if (queue && queue.counts.active > 0) {
      e.preventDefault();
      e.returnValue = "";
    }
  });
}

export function uploads(): UploadQueue {
  queue ??= make(xhrTransport);
  return queue;
}

/** Tests: a fresh, empty queue over `send`. */
export function resetUploads(send: Transport): void {
  queue?.cancelAll();
  queue = make(send);
}

/** Calls `fn(dir, base)` when a batch of uploads into `dir` (in the place `base`) has settled with a file done. */
export function useUploaded(fn: (dir: string, base: string) => void): void {
  useEffect(() => {
    uploaded.add(fn);
    return () => {
      uploaded.delete(fn);
    };
  }, [fn]);
}

/** Re-renders the caller on every change to `q`. */
export function useQueue(q: UploadQueue): UploadQueue {
  const [, setTick] = useState(0);
  useEffect(() => q.subscribe(() => setTick((n) => n + 1)), [q]);
  return q;
}
