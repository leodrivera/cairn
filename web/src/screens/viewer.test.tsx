import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { App } from "../app";
import { flush, json, render, waitFor } from "../test/dom";
import { admin, file, listing, reader, startAt } from "../test/fixtures";

const script = file("backup.sh", { mime: "text/x-sh", size: 5 });

function textarea(el: HTMLElement): HTMLTextAreaElement {
  const t = el.querySelector<HTMLTextAreaElement>("textarea");
  assert.ok(t, "no textarea");
  return t!;
}

function button(el: HTMLElement, text: string): HTMLButtonElement | undefined {
  return [...el.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.trim() === text);
}

async function edit(el: HTMLElement, value: string) {
  const t = textarea(el);
  t.value = value;
  t.dispatchEvent(new Event("input", { bubbles: true }));
  await flush();
}

function save(el: HTMLElement) {
  textarea(el).dispatchEvent(new KeyboardEvent("keydown", { key: "s", ctrlKey: true, bubbles: true, cancelable: true }));
}

const modified = new Date(2026, 9, 24, 8, 0).toUTCString();

/** A raw file as the server sends it: the whole body, or one byte for `Range: bytes=0-0` (the viewer's probe). */
function raw(body: BodyInit, etag = '"5-1-0"', type = "text/plain", size?: number) {
  return async (req: Request) => {
    const bytes = new Uint8Array(await new Response(body).arrayBuffer());
    const total = size ?? bytes.length;
    const headers = { ETag: etag, "content-type": type, "last-modified": modified };
    if (req.headers.get("range") === "bytes=0-0") {
      if (total === 0) return new Response(null, { status: 416, headers: { ...headers, "content-range": "bytes */0" } });
      return new Response(bytes.slice(0, 1), { status: 206, headers: { ...headers, "content-range": `bytes 0-0/${total}` } });
    }
    return new Response(bytes, { headers });
  };
}

const full = (r: Request) => r.url.includes("/api/raw/") && !r.headers.get("range");

test("loads, then saves with If-Match and keeps the new ETag", async () => {
  const puts: { ifMatch: string | null; type: string | null; body: string }[] = [];
  let n = 1;
  const log = startAt("/view/scripts/backup.sh", admin, {
    "GET /api/raw/scripts/backup.sh": raw("hello"),
    "PUT /api/text/scripts/backup.sh": async (req) => {
      puts.push({ ifMatch: req.headers.get("if-match"), type: req.headers.get("content-type"), body: await req.text() });
      const etag = `"6-${++n}-0"`;
      return new Response(JSON.stringify({ etag }), { headers: { ETag: etag, "content-type": "application/json" } });
    },
  });
  // happy-dom's Request drops `cache`, so read it from the call itself.
  const caches: (string | undefined)[] = [];
  const stub = globalThis.fetch;
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    if (String(input).includes("/api/raw/")) caches.push(init?.cache);
    return stub(input, init);
  }) as typeof fetch;
  const el = render(<App />);
  await waitFor(() => assert.equal(textarea(el).value, "hello"));
  assert.ok(log.some((r) => r.url.includes("/api/raw/")));
  assert.deepEqual(caches, ["no-store", "no-store"]);
  assert.match(el.textContent!, /backup\.sh/);
  assert.match(el.textContent!, /scripts · 5 B · edited Oct 24/);
  assert.match(el.textContent!, /Shell · UTF-8 · LF/);
  assert.match(el.textContent!, /All changes saved/);
  assert.equal(textarea(el).getAttribute("spellcheck"), "false");
  assert.equal(textarea(el).getAttribute("wrap"), "off");
  await edit(el, "hello!");
  assert.match(el.textContent!, /Unsaved changes/);
  save(el);
  await waitFor(() => assert.match(el.textContent!, /All changes saved/));
  await edit(el, "hello!!");
  save(el);
  await waitFor(() => assert.equal(puts.length, 2));
  assert.deepEqual(puts[0], { ifMatch: '"5-1-0"', type: "text/plain; charset=utf-8", body: "hello!" });
  assert.equal(puts[1].ifMatch, '"6-2-0"');
  assert.equal(puts[1].body, "hello!!");
});

test("a conflict keeps the text; Reload asks first", async () => {
  let raws = 0;
  startAt("/view/scripts/backup.sh", admin, {
    "GET /api/raw/scripts/backup.sh": (req) => {
      if (!req.headers.get("range")) raws++;
      return raw(raws <= 1 ? "hello" : "server text", `"9-${raws}-0"`)(req);
    },
    "PUT /api/text/scripts/backup.sh": () =>
      json({ error: "changed", message: "This file changed on the server since you opened it." }, 412),
  });
  const asked: string[] = [];
  let answer = false;
  globalThis.confirm = (m?: string) => {
    asked.push(String(m));
    return answer;
  };
  const el = render(<App />);
  await waitFor(() => assert.equal(textarea(el).value, "hello"));
  await edit(el, "mine");
  save(el);
  await waitFor(() => assert.match(el.textContent!, /This file changed on the server since you opened it\./));
  assert.equal(textarea(el).value, "mine");
  assert.ok(el.querySelector('a[download="backup.sh"][href^="blob:"]') || button(el, "Download my version"));
  button(el, "Reload")!.click();
  await flush();
  assert.equal(asked.length, 1);
  assert.equal(raws, 1);
  assert.equal(textarea(el).value, "mine");
  answer = true;
  button(el, "Reload")!.click();
  await waitFor(() => assert.equal(textarea(el).value, "server text"));
  assert.equal(raws, 2);
  assert.ok(!el.textContent!.includes("changed on the server"));
});

test("leaving with changes asks", async () => {
  startAt("/view/scripts/backup.sh", admin, {
    "GET /api/raw/scripts/backup.sh": raw("hello"),
  });
  const asked: string[] = [];
  globalThis.confirm = (m?: string) => {
    asked.push(String(m));
    return false;
  };
  const el = render(<App />);
  await waitFor(() => assert.equal(textarea(el).value, "hello"));
  await edit(el, "changed");
  const back = el.querySelector<HTMLAnchorElement>('a[href="/files/scripts"]')!;
  back.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
  await flush();
  assert.deepEqual(asked, ["Leave without saving? Your changes will be lost."]);
  assert.equal(location.pathname, "/view/scripts/backup.sh");
  const ev = new Event("beforeunload", { cancelable: true });
  dispatchEvent(ev);
  assert.ok(ev.defaultPrevented);
});

test("too large offers a download", async () => {
  const big = file("huge.log", { mime: "text/plain", size: 3 * 1024 * 1024 });
  const log = startAt("/view/huge.log", admin, {
    "GET /api/raw/huge.log": raw("x", '"1-1-0"', "text/plain", big.size),
  });
  const el = render(<App />);
  await waitFor(() =>
    assert.match(el.textContent!, /This file is too large to edit here \(3 MB\)\. Download it instead\./),
  );
  assert.ok(el.querySelector('a[href="/api/raw/huge.log?download=1"]'));
  assert.ok(!log.some(full));
  assert.ok(!log.some((r) => r.url.includes("/api/files")));
  assert.ok(!el.querySelector("textarea"));
});

test("an image shows an img", async () => {
  startAt("/view/photos/cover.png", admin, {
    "GET /api/raw/photos/cover.png": raw("png", '"3-1-0"', "image/png"),
  });
  const el = render(<App />);
  await waitFor(() => assert.ok(el.querySelector("img.viewer-image")));
  assert.equal(el.querySelector("img.viewer-image")!.getAttribute("src"), "/api/raw/photos/cover.png");
  assert.ok(!button(el, "Save"));
});

test("other files have no preview", async () => {
  startAt("/view/site-backup.tar.gz", admin, {
    "GET /api/raw/site-backup.tar.gz": raw("gz", '"2-1-0"', "application/gzip"),
  });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /No preview for this kind of file\./));
  assert.ok(el.querySelector('a[href="/api/raw/site-backup.tar.gz?download=1"]'));
});

test("a reader has no Save and cannot type", async () => {
  startAt("/view/scripts/backup.sh", reader, {
    "GET /api/raw/scripts/backup.sh": raw("hello"),
  });
  const el = render(<App />);
  await waitFor(() => assert.equal(textarea(el).value, "hello"));
  assert.ok(!button(el, "Save"));
  assert.ok(textarea(el).readOnly);
});

test("a body that is not UTF-8 is read-only", async () => {
  startAt("/view/notes.txt", admin, {
    "GET /api/raw/notes.txt": raw(new Uint8Array([0x63, 0xe9, 0x0a])),
  });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /This file is not text Cairn can edit\./));
  assert.ok(textarea(el).readOnly);
  assert.ok(!button(el, "Save"));
});

test("a file that is gone says so", async () => {
  startAt("/view/missing.txt", admin, {
    "GET /api/raw/missing.txt": () => json({ error: "not_found", message: "No such file or folder." }, 404),
  });
  const el = render(<App />);
  await waitFor(() => assert.match(el.textContent!, /No such file or folder\./));
});

test("the browser Back button asks too, and stays when told to", async () => {
  startAt("/files/scripts", admin, {
    "GET /api/files/scripts": listing("scripts", [script]),
    "GET /api/raw/scripts/backup.sh": raw("hello"),
  });
  const asked: string[] = [];
  let answer = false;
  globalThis.confirm = (m?: string) => {
    asked.push(String(m));
    return answer;
  };
  const el = render(<App />);
  await waitFor(() => assert.ok(el.querySelector('a[href="/view/scripts/backup.sh"]')));
  el.querySelector('a[href="/view/scripts/backup.sh"]')!.dispatchEvent(
    new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }),
  );
  await waitFor(() => assert.equal(textarea(el).value, "hello"));
  await edit(el, "changed");
  history.back();
  await waitFor(() => assert.deepEqual(asked, ["Leave without saving? Your changes will be lost."]));
  await flush(10);
  assert.equal(location.pathname, "/view/scripts/backup.sh");
  assert.equal(textarea(el).value, "changed");
  answer = true;
  history.back();
  await waitFor(() => assert.equal(location.pathname, "/files/scripts"));
  await waitFor(() => assert.ok(!el.querySelector("textarea")));
  assert.equal(asked.length, 2);
});

test("final: Sign out with unsaved text asks first and stays signed in on Stay", async () => {
  const log = startAt("/view/scripts/backup.sh", admin, {
    "GET /api/raw/scripts/backup.sh": raw("hello"),
    "POST /api/auth/logout": () => json({ next: null }),
  });
  globalThis.confirm = () => false;
  const el = render(<App />);
  await waitFor(() => assert.equal(textarea(el).value, "hello"));
  await edit(el, "changed");
  el.querySelector<HTMLButtonElement>('button[aria-label="Sign out"]')!.click();
  await flush(5);
  assert.ok(!log.some((r) => r.url.endsWith("/api/auth/logout")));
  assert.equal(location.pathname, "/view/scripts/backup.sh");
  assert.equal(textarea(el).value, "changed");
});

test("final: any failed save offers Download my version", async () => {
  startAt("/view/scripts/backup.sh", admin, {
    "GET /api/raw/scripts/backup.sh": raw("hello"),
    "PUT /api/text/scripts/backup.sh": () => json({ error: "signed_out", message: "Sign in first." }, 403),
  });
  const el = render(<App />);
  await waitFor(() => assert.equal(textarea(el).value, "hello"));
  await edit(el, "mine");
  save(el);
  await waitFor(() => assert.match(el.textContent!, /Sign in first\./));
  assert.ok(button(el, "Download my version"));
  assert.equal(textarea(el).value, "mine");
});

test("minor: the viewer reads one byte, not the folder's listing", async () => {
  const log = startAt("/view/scripts/backup.sh", admin, { "GET /api/raw/scripts/backup.sh": raw("hello", '"5-1-0"', "text/x-sh") });
  const el = render(<App />);
  await waitFor(() => assert.equal(textarea(el).value, "hello"));
  assert.match(el.textContent!, /scripts · 5 B · edited Oct 24/);
  assert.ok(!log.some((r) => r.url.includes("/api/files")));
});

test("minor: a slow file that lands after moving on is dropped", async () => {
  let releaseA: (r: Response) => void = () => {};
  startAt("/view/a.txt", admin, {
    "GET /api/raw/a.txt": (req) =>
      req.headers.get("range") ? raw("A", '"1-1-0"')(req) : new Promise<Response>((r) => (releaseA = r)),
    "GET /api/raw/b.txt": raw("text of b", '"9-1-0"'),
  });
  const el = render(<App />);
  await flush(5);
  const { navigate } = await import("../lib/router");
  navigate("/view/b.txt");
  await waitFor(() => assert.equal(textarea(el).value, "text of b"));
  releaseA(new Response("text of a", { headers: { ETag: '"9-1-0"' } }));
  await flush(10);
  assert.equal(textarea(el).value, "text of b");
});

test("minor: Ctrl+S is left to the browser where nothing can be saved", async () => {
  startAt("/view/photos/cover.png", admin, { "GET /api/raw/photos/cover.png": raw("png", '"3-1-0"', "image/png") });
  const el = render(<App />);
  await waitFor(() => assert.ok(el.querySelector("img.viewer-image")));
  const ev = new KeyboardEvent("keydown", { key: "s", ctrlKey: true, bubbles: true, cancelable: true });
  document.body.dispatchEvent(ev);
  assert.ok(!ev.defaultPrevented);
});
