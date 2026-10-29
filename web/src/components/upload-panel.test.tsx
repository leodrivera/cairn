import "../test/dom";
import { test } from "node:test";
import assert from "node:assert/strict";
import { UploadPanel } from "./upload-panel";
import { UploadQueue, type Transport } from "../lib/uploads";
import { render, waitFor } from "../test/dom";

test("shows counts, errors and retry", async () => {
  const answers = [
    { status: 201, body: "{}" },
    { status: 507, body: JSON.stringify({ error: "disk_full", message: "x" }) },
    { status: 201, body: "{}" },
  ];
  const urls: string[] = [];
  const send: Transport = async (url) => {
    urls.push(url);
    return answers.shift()!;
  };
  const q = new UploadQueue(send, () => {});
  const el = render(<UploadPanel queue={q} />);
  q.add("photos/2026", [new File(["a"], "cover.png"), new File(["b"], "site-backup.tar.gz")]);
  await waitFor(() => assert.match(el.textContent!, /Failed/));
  assert.match(el.textContent!, /The server's disk is full\. Free some space, then try again\./);
  assert.match(el.textContent!, /to photos\/2026/);
  const retry = [...el.querySelectorAll("button")].find((b) => b.textContent === "Retry")!;
  retry.click();
  await waitFor(() => assert.match(el.textContent!, /Uploaded 2 files to photos\/2026\./));
  assert.equal(urls.length, 3);
});

test("names the upload while it runs", async () => {
  const send: Transport = () => new Promise(() => {});
  const q = new UploadQueue(send, () => {});
  const el = render(<UploadPanel queue={q} />);
  q.add("", [new File(["a"], "a"), new File(["b"], "b"), new File(["c"], "c")]);
  await waitFor(() => assert.match(el.textContent!, /Uploading 1 of 3/));
  assert.match(el.textContent!, /to My files/);
  assert.equal(el.querySelectorAll("progress").length, 3);
  assert.match(el.textContent!, /Queued/);
});
