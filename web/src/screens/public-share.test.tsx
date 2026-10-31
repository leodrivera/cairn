import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { App } from "../app";
import { buttonNamed, byLabel, flush, json, render, setValue, stubFetch, waitFor, type Handler, passwordToggles } from "../test/dom";
import { config, file, folder } from "../test/fixtures";
import type { ShareInfo } from "../lib/types";

const ID = "k7Qw3nTzAbCdEfGhIjKl-_";
const now = () => Math.floor(Date.now() / 1000);

function info(extra: Partial<ShareInfo> = {}): ShareInfo {
  return {
    entry: file("trail-ridge.jpg", { mime: "image/jpeg" }),
    download: true,
    expiresAt: now() + 7 * 86_400 - 60,
    sharedBy: "Admin",
    ...extra,
  };
}

function visit(at: string, routes: Record<string, Handler>) {
  history.replaceState(null, "", at);
  const log = stubFetch({ "GET /api/config": () => json(config), ...routes });
  const el = render(<App />);
  return { el, log };
}

test("a share route skips the session", async () => {
  const { el, log } = visit(`/s/${ID}`, { [`GET /s/${ID}/info`]: () => json(info()) });
  await waitFor(() => assert.match(el.textContent!, /trail-ridge\.jpg/));
  assert.ok(!log.some((r) => new URL(r.url).pathname === "/api/me"));
  assert.equal(location.pathname, `/s/${ID}`);
});

test("gone state", async () => {
  const { el } = visit(`/s/${ID}`, {
    [`GET /s/${ID}/info`]: () => json({ error: "share_gone", message: "x" }, 404),
  });
  await waitFor(() => assert.match(el.textContent!, /This link no longer works/));
  assert.match(el.textContent!, /Ask the person who sent it for a new one\./);
  assert.ok(!el.querySelector("button"));
});

test("locked, then open with the password", async () => {
  let unlocked = false;
  const { el, log } = visit(`/s/${ID}`, {
    [`GET /s/${ID}/info`]: () => (unlocked ? json(info()) : json({ error: "share_locked", message: "x" }, 403)),
    [`POST /s/${ID}/unlock`]: () => {
      unlocked = true;
      return new Response(null, { status: 204 });
    },
  });
  await waitFor(() => assert.match(el.textContent!, /This link needs a password/));
  setValue(byLabel(el, "Password"), "s3cret-pass");
  await flush();
  buttonNamed(el, "Open").click();
  await waitFor(() => assert.match(el.textContent!, /trail-ridge\.jpg/));
  const post = log.find((r) => r.method === "POST")!;
  assert.deepEqual(await post.json(), { password: "s3cret-pass" });
  assert.equal(post.headers.get("x-cairn"), "1");
});

test("wrong password message", async () => {
  const { el } = visit(`/s/${ID}`, {
    [`GET /s/${ID}/info`]: () => json({ error: "share_locked", message: "x" }, 403),
    [`POST /s/${ID}/unlock`]: () => json({ error: "wrong_link_password", message: "x" }, 403),
  });
  await waitFor(() => assert.ok(byLabel(el, "Password")));
  setValue(byLabel(el, "Password"), "nope-nope");
  await flush();
  buttonNamed(el, "Open").click();
  await waitFor(() => assert.match(el.querySelector('[role="alert"]')!.textContent!, /That password is not right\./));
});

test("too many tries says how long to wait", async () => {
  const { el } = visit(`/s/${ID}`, {
    [`GET /s/${ID}/info`]: () => json({ error: "share_locked", message: "x" }, 403),
    [`POST /s/${ID}/unlock`]: () => json({ error: "too_many", message: "x" }, 429, { "retry-after": "600" }),
  });
  await waitFor(() => assert.ok(byLabel(el, "Password")));
  setValue(byLabel(el, "Password"), "nope-nope");
  await flush();
  buttonNamed(el, "Open").click();
  await waitFor(() => assert.match(el.querySelector('[role="alert"]')!.textContent!, /Wait 10 minutes/));
});

test("a file link shows the image inline with a download", async () => {
  const { el } = visit(`/s/${ID}`, { [`GET /s/${ID}/info`]: () => json(info()) });
  await waitFor(() => assert.ok(el.querySelector("img.public-image")));
  assert.equal(el.querySelector<HTMLImageElement>("img.public-image")!.getAttribute("src"), `/s/${ID}/raw`);
  const dl = [...el.querySelectorAll("a")].find((a) => a.textContent!.trim() === "Download");
  assert.ok(dl);
  assert.equal(dl!.getAttribute("href"), `/s/${ID}/raw?download=1`);
  assert.match(el.textContent!, /JPG · 4\.2 MB/);
});

test("downloads off: no download controls", async () => {
  const { el } = visit(`/s/${ID}`, { [`GET /s/${ID}/info`]: () => json(info({ download: false })) });
  await waitFor(() => assert.ok(el.querySelector("img.public-image")));
  assert.ok(![...el.querySelectorAll("a, button")].some((a) => /Download/.test(a.textContent!)));
});

test("a text file is shown in a pre", async () => {
  const { el } = visit(`/s/${ID}`, {
    [`GET /s/${ID}/info`]: () => json(info({ entry: file("notes.txt", { mime: "text/plain", size: 10 }) })),
    [`GET /s/${ID}/raw`]: () => new Response("some notes", { headers: { "content-type": "text/plain" } }),
  });
  await waitFor(() => assert.equal(el.querySelector("pre.public-text")?.textContent, "some notes"));
});

test("another kind of file has no preview", async () => {
  const { el } = visit(`/s/${ID}`, {
    [`GET /s/${ID}/info`]: () => json(info({ entry: file("site-backup.tar.gz", { mime: "application/gzip" }) })),
  });
  await waitFor(() => assert.match(el.textContent!, /No preview for this file\./));
});

const tree = {
  [`GET /s/${ID}/info`]: () => json(info({ entry: folder("photos") })),
  [`GET /s/${ID}/list`]: () =>
    json({ path: "", entries: [folder("sub"), file("notes.txt", { mime: "text/plain", size: 10 })] }),
  [`GET /s/${ID}/list/sub`]: () => json({ path: "sub", entries: [file("deep.txt", { mime: "text/plain", size: 4 })] }),
  [`GET /s/${ID}/raw/notes.txt`]: () => new Response("some notes"),
  [`GET /s/${ID}/info/notes.txt`]: () => json({ entry: file("notes.txt", { mime: "text/plain", size: 10 }) }),
};

test("folder rows and navigation", async () => {
  const { el } = visit(`/s/${ID}`, tree);
  await waitFor(() => assert.ok(el.querySelector(".public-list")));
  const zip = [...el.querySelectorAll("a")].find((a) => a.textContent!.trim() === "Download all");
  assert.equal(zip!.getAttribute("href"), `/s/${ID}/zip`);
  el.querySelector<HTMLAnchorElement>('.public-list a[href$="?path=sub"]')!.click();
  await waitFor(() => assert.match(el.textContent!, /deep\.txt/));
  assert.equal(location.search, "?path=sub");
  history.back();
  await waitFor(() => assert.match(el.textContent!, /notes\.txt/));
  history.replaceState(null, "", `/s/${ID}`);
  const { el: el2 } = visit(`/s/${ID}`, tree);
  await waitFor(() => assert.ok(el2.querySelector('.public-list a[href$="?file=notes.txt"]')));
  el2.querySelector<HTMLAnchorElement>('.public-list a[href$="?file=notes.txt"]')!.click();
  await waitFor(() => assert.equal(el2.querySelector("pre.public-text")?.textContent, "some notes"));
});

test("footer says who shared it and until when", async () => {
  const { el } = visit(`/s/${ID}`, { [`GET /s/${ID}/info`]: () => json(info()) });
  await waitFor(() => assert.match(el.textContent!, /Shared by Admin · expires in 7 days/));
  const { el: el2 } = visit(`/s/${ID}`, { [`GET /s/${ID}/info`]: () => json(info({ expiresAt: null })) });
  await waitFor(() => assert.match(el2.textContent!, /Shared by Admin/));
  assert.doesNotMatch(el2.textContent!, /expires/);
});

test("an odd name uses its raw segment in links", async () => {
  const raw = "\u0000Y2Fm6S50eHQ";
  const { el } = visit(`/s/${ID}`, {
    [`GET /s/${ID}/info`]: () => json(info({ entry: folder("photos") })),
    [`GET /s/${ID}/list`]: () => json({ path: "", entries: [file("caf�.txt", { rawName: raw })] }),
  });
  await waitFor(() => assert.ok(el.querySelector(".public-list a")));
  assert.match(el.querySelector(".public-list a")!.getAttribute("href")!, /\?file=%00Y2Fm6S50eHQ$/);
});

test("pt-BR public page", async () => {
  const had = Object.getOwnPropertyDescriptor(navigator, "languages");
  Object.defineProperty(navigator, "languages", { value: ["pt-BR"], configurable: true });
  try {
    const { el } = visit(`/s/${ID}`, {
      [`GET /s/${ID}/info`]: () => json({ error: "share_locked", message: "x" }, 403),
    });
    await waitFor(() => assert.match(el.textContent!, /Este link pede senha/));
  } finally {
    if (had) Object.defineProperty(navigator, "languages", had);
    else delete (navigator as { languages?: unknown }).languages;
    const { setLang } = await import("../i18n/index");
    setLang("en");
  }
});

test("a large shared folder renders only the rows on screen", async () => {
  const many = Array.from({ length: 5000 }, (_, i) => file(`photo-${i}.jpg`, { mime: "image/jpeg" }));
  const { el } = visit(`/s/${ID}`, {
    [`GET /s/${ID}/info`]: () => json(info({ entry: folder("photos") })),
    [`GET /s/${ID}/list`]: () => json({ path: "", entries: many }),
  });
  await waitFor(() => assert.ok(el.querySelector(".public-list li")));
  const rows = el.querySelectorAll(".public-list li").length;
  assert.ok(rows > 0 && rows < 100, `${rows} rows in the DOM`);
});

test("a file inside a folder asks only for itself", async () => {
  const { el, log } = visit(`/s/${ID}?file=sub%2Fdeep.txt`, {
    [`GET /s/${ID}/info`]: () => json(info({ entry: folder("photos") })),
    [`GET /s/${ID}/info/sub/deep.txt`]: () => json({ entry: file("deep.txt", { mime: "text/plain", size: 4 }) }),
    [`GET /s/${ID}/raw/sub/deep.txt`]: () => new Response("deep"),
  });
  await waitFor(() => assert.equal(el.querySelector("pre.public-text")?.textContent, "deep"));
  assert.ok(!log.some((r) => new URL(r.url).pathname.includes("/list")));
});

test("an unlock that ran out while reading asks for the password again", async () => {
  const { el } = visit(`/s/${ID}`, {
    [`GET /s/${ID}/info`]: () => json(info({ entry: file("notes.txt", { mime: "text/plain", size: 10 }) })),
    [`GET /s/${ID}/raw`]: () => json({ error: "share_locked", message: "x" }, 403),
  });
  await waitFor(() => assert.match(el.textContent!, /This link needs a password/));
  const { el: el2 } = visit(`/s/${ID}`, {
    [`GET /s/${ID}/info`]: () => json(info({ entry: folder("photos") })),
    [`GET /s/${ID}/list`]: () => json({ error: "share_locked", message: "x" }, 403),
  });
  await waitFor(() => assert.match(el2.textContent!, /This link needs a password/));
});

test("a preview that fails says so", async () => {
  const { el } = visit(`/s/${ID}`, {
    [`GET /s/${ID}/info`]: () => json(info({ entry: file("notes.txt", { mime: "text/plain", size: 10 }) })),
    [`GET /s/${ID}/raw`]: () => json({ error: "internal", message: "x" }, 500),
  });
  await waitFor(() => assert.match(el.querySelector('[role="alert"]')?.textContent ?? "", /Something went wrong/));
  assert.ok(!el.querySelector("pre.public-text"));
});

test("the page has a heading and a tab title", async () => {
  const { el } = visit(`/s/${ID}`, tree);
  await waitFor(() => assert.equal(el.querySelector("h1")?.textContent, "photos"));
  assert.equal(document.title, "photos");
  const { el: el2 } = visit(`/s/${ID}`, { [`GET /s/${ID}/info`]: () => json(info()) });
  await waitFor(() => assert.equal(el2.querySelector("h1")?.textContent, "trail-ridge.jpg"));
  assert.equal(document.title, "trail-ridge.jpg");
});

test("password_field_has_toggle", async () => {
  const { el } = visit(`/s/${ID}`, {
    [`GET /s/${ID}/info`]: () => json({ error: "share_locked", message: "x" }, 403),
  });
  await waitFor(() => assert.match(el.textContent!, /This link needs a password/));
  assert.equal(passwordToggles(el), 1);
});
