import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { addTree, ensureFolder, entriesFromDrop, filesFromPicker, UploadQueue, type Transport } from "./uploads";
import { ApiFailure } from "./api";
import { json, stubFetch } from "../test/dom";

type Call = { url: string; file: Blob; progress(n: number): void; signal: AbortSignal; done(status: number, body?: string): void };

function fakeTransport() {
  const calls: Call[] = [];
  const send: Transport = (url, file, progress, signal) =>
    new Promise((resolve, reject) => {
      signal.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
      calls.push({ url, file, progress, signal, done: (status, body = "{}") => resolve({ status, body }) });
    });
  return { send, calls };
}

const tick = () => new Promise((r) => setTimeout(r, 0));
const f = (name: string, size = 10) => new File(["x".repeat(size)], name);

test("runs two at a time", async () => {
  const { send, calls } = fakeTransport();
  const q = new UploadQueue(send, () => {});
  q.add("photos", [f("a"), f("b"), f("c"), f("d"), f("e")]);
  await tick();
  assert.equal(calls.length, 2);
  assert.deepEqual(q.items.map((i) => i.state), ["sending", "sending", "queued", "queued", "queued"]);
  calls[0].done(201);
  await tick();
  assert.equal(calls.length, 3);
  assert.equal(q.items[0].state, "done");
});

test("progress updates", async () => {
  const { send, calls } = fakeTransport();
  let changes = 0;
  const q = new UploadQueue(send, () => changes++);
  q.add("", [f("a", 100)]);
  await tick();
  const before = changes;
  calls[0].progress(40);
  assert.equal(q.items[0].sent, 40);
  assert.ok(changes > before);
});

test("exists asks, then replace sends replace=1", async () => {
  const { send, calls } = fakeTransport();
  const q = new UploadQueue(send, () => {});
  q.add("photos", [f("cover.png")]);
  await tick();
  calls[0].done(409, JSON.stringify({ error: "exists", message: "Something with that name is already there." }));
  await tick();
  assert.equal(q.items[0].state, "exists");
  q.replace(q.items[0].id);
  await tick();
  assert.equal(calls[1].url, "/api/upload/photos/cover.png?replace=1");
  calls[1].done(200);
  await tick();
  assert.equal(q.items[0].state, "done");
});

test("skip leaves an existing name alone", async () => {
  const { send, calls } = fakeTransport();
  const q = new UploadQueue(send, () => {});
  q.add("", [f("a")]);
  await tick();
  calls[0].done(409, JSON.stringify({ error: "exists", message: "x" }));
  await tick();
  q.skip(q.items[0].id);
  assert.equal(q.items[0].state, "cancelled");
  assert.equal(calls.length, 1);
});

test("a failure, then retry from zero", async () => {
  const { send, calls } = fakeTransport();
  const q = new UploadQueue(send, () => {});
  q.add("", [f("a", 100)]);
  await tick();
  calls[0].progress(60);
  calls[0].done(507, JSON.stringify({ error: "disk_full", message: "The server's disk is full." }));
  await tick();
  assert.equal(q.items[0].state, "failed");
  assert.ok(q.items[0].error instanceof ApiFailure);
  assert.equal(q.items[0].error!.code, "disk_full");
  q.retry(q.items[0].id);
  assert.equal(q.items[0].sent, 0);
  await tick();
  assert.equal(calls.length, 2);
  assert.equal(calls[1].url, "/api/upload/a");
});

test("a network error fails as offline", async () => {
  const q = new UploadQueue(() => Promise.reject(new TypeError("network")), () => {});
  q.add("", [f("a")]);
  await tick();
  await tick();
  assert.equal(q.items[0].state, "failed");
  assert.equal(q.items[0].error!.code, "offline");
});

test("cancel aborts the signal", async () => {
  const { send, calls } = fakeTransport();
  const q = new UploadQueue(send, () => {});
  q.add("", [f("a"), f("b"), f("c")]);
  await tick();
  q.cancel(q.items[0].id);
  await tick();
  assert.equal(calls[0].signal.aborted, true);
  assert.equal(q.items[0].state, "cancelled");
  assert.equal(calls.length, 3, "the next one starts");
  q.cancelAll();
  await tick();
  assert.deepEqual(q.items.map((i) => i.state), ["cancelled", "cancelled", "cancelled"]);
});

test("unicode name url", async () => {
  const { send, calls } = fakeTransport();
  const q = new UploadQueue(send, () => {});
  q.add("photos", [f("Report #1 (50%) ✓ café.txt")]);
  await tick();
  assert.equal(calls[0].url, "/api/upload/photos/Report%20%231%20(50%25)%20%E2%9C%93%20caf%C3%A9.txt");
});

test("onDone fires once the batch for a folder settles", async () => {
  const { send, calls } = fakeTransport();
  const q = new UploadQueue(send, () => {});
  const done: string[] = [];
  q.onDone = (dir) => done.push(dir);
  q.add("photos", [f("a"), f("b")]);
  await tick();
  calls[0].done(201);
  await tick();
  assert.deepEqual(done, []);
  calls[1].done(201);
  await tick();
  assert.deepEqual(done, ["photos"]);
});

test("a folder tree creates parents first, each once", async () => {
  const { send, calls } = fakeTransport();
  const q = new UploadQueue(send, () => {});
  const made: string[] = [];
  await addTree(
    q,
    "photos",
    [
      { relPath: "a/b/c.txt", file: f("c.txt") },
      { relPath: "a/d.txt", file: f("d.txt") },
      { relPath: "a/empty/", file: null },
    ],
    async (p) => {
      assert.equal(calls.length, 0, "no upload before the folders");
      made.push(p);
    },
  );
  assert.deepEqual(made, ["photos/a", "photos/a/b", "photos/a/empty"]);
  await tick();
  assert.deepEqual(
    calls.map((c) => c.url),
    ["/api/upload/photos/a/b/c.txt", "/api/upload/photos/a/d.txt"],
  );
  assert.deepEqual(q.items.map((i) => i.label), ["a/b/c.txt", "a/d.txt"]);
});

test("a folder name that is a file fails the items under it", async () => {
  const { send, calls } = fakeTransport();
  const q = new UploadQueue(send, () => {});
  await addTree(
    q,
    "",
    [
      { relPath: "a/x.txt", file: f("x.txt") },
      { relPath: "b/y.txt", file: f("y.txt") },
    ],
    async (p) => {
      if (p === "a") throw new ApiFailure(400, "not_a_folder", "That is a file, not a folder.");
    },
  );
  await tick();
  assert.equal(q.items[0].state, "failed");
  assert.equal(q.items[0].error!.code, "not_a_folder");
  assert.deepEqual(calls.map((c) => c.url), ["/api/upload/b/y.txt"]);
});

test("ensureFolder reuses an existing folder", async () => {
  stubFetch({
    "POST /api/folders/photos/a": () => json({ error: "exists", message: "x" }, 409),
    "GET /api/raw/photos/a": () => json({ error: "is_a_folder", message: "That is a folder, not a file." }, 400),
  });
  await ensureFolder("photos/a");
});

test("ensureFolder refuses a file in the way", async () => {
  stubFetch({
    "POST /api/folders/a": () => json({ error: "exists", message: "x" }, 409),
    "GET /api/raw/a": () => new Response("x", { status: 206, headers: { "content-range": "bytes 0-0/9" } }),
  });
  await assert.rejects(ensureFolder("a"), (e: ApiFailure) => e.code === "not_a_folder");
});

test("drop entries are read until an empty batch", async () => {
  const fileEntry = (name: string) => ({
    isFile: true,
    isDirectory: false,
    name,
    file: (ok: (f: File) => void) => ok(f(name)),
  });
  const batches = [[fileEntry("1.jpg"), fileEntry("2.jpg")], [fileEntry("3.jpg")], []];
  const dir = {
    isFile: false,
    isDirectory: true,
    name: "album",
    createReader: () => ({ readEntries: (ok: (e: unknown[]) => void) => ok(batches.shift() ?? []) }),
  };
  const emptyDir = {
    isFile: false,
    isDirectory: true,
    name: "empty",
    createReader: () => ({ readEntries: (ok: (e: unknown[]) => void) => ok([]) }),
  };
  const got = await entriesFromDrop([dir, emptyDir, fileEntry("loose.txt")] as unknown as FileSystemEntry[]);
  assert.deepEqual(
    got.map((g) => [g.relPath, g.file?.name ?? null]),
    [
      ["album/1.jpg", "1.jpg"],
      ["album/2.jpg", "2.jpg"],
      ["album/3.jpg", "3.jpg"],
      ["empty/", null],
      ["loose.txt", "loose.txt"],
    ],
  );
});

test("the folder picker uses the relative path", () => {
  const a = f("c.txt");
  Object.defineProperty(a, "webkitRelativePath", { value: "trip/day1/c.txt" });
  const b = f("plain.txt");
  assert.deepEqual(
    filesFromPicker([a, b]).map((x) => x.relPath),
    ["trip/day1/c.txt", "plain.txt"],
  );
});

test("final: progress notifies at most once per 100 ms, and the last value lands", async () => {
  const { send, calls } = fakeTransport();
  const q = new UploadQueue(send, () => {});
  q.add("", [f("a", 1000)]);
  await tick();
  let seen = 0;
  q.subscribe(() => seen++);
  for (let i = 1; i <= 100; i++) calls[0].progress(i * 10);
  assert.ok(seen <= 1, `${seen} notifications for 100 progress events`);
  assert.equal(q.items[0].sent, 1000);
  await new Promise((r) => setTimeout(r, 150));
  assert.ok(seen >= 2, "the last progress was never announced");
});

test("final: 20 000 quick uploads finish in linear time", async () => {
  const send: Transport = async () => ({ status: 201, body: "{}" });
  const q = new UploadQueue(send, () => {}, 2);
  const files = Array.from({ length: 20_000 }, (_, i) => f(`f-${i}`, 1));
  const start = Date.now();
  let done = false;
  q.onDone = () => (done = true);
  q.add("", files);
  while (!done) await tick();
  const ms = Date.now() - start;
  assert.ok(ms < 4000, `${ms} ms`);
  assert.ok(q.items.every((i) => i.state === "done"));
});

test("minor: ensureFolder checks with one byte, never a listing", async () => {
  const log = stubFetch({
    "POST /api/folders/photos/a": () => json({ error: "exists", message: "x" }, 409),
    "GET /api/raw/photos/a": () => json({ error: "is_a_folder", message: "That is a folder, not a file." }, 400),
  });
  await ensureFolder("photos/a");
  const probe = log.find((r) => r.method === "GET")!;
  assert.equal(probe.headers.get("range"), "bytes=0-0");
});

test("minor: a signed-out answer ends the session for the whole app", async () => {
  const { onSignedOut } = await import("./api");
  let ended = 0;
  onSignedOut(() => ended++);
  const send: Transport = async () => ({ status: 403, body: JSON.stringify({ error: "signed_out", message: "Sign in first." }) });
  const q = new UploadQueue(send, () => {});
  q.add("", [f("a")]);
  for (let i = 0; i < 5; i++) await tick();
  assert.equal(q.items[0].state, "failed");
  assert.equal(ended, 1);
});
