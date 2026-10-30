// Share one file or folder: choose expiry, password and downloads, create the link, then copy, change or delete it.
import { useState } from "preact/hooks";
import { ApiFailure, offline } from "../lib/api";
import { displayPath, displaySegment, segments } from "../lib/paths";
import { copyText, createShare, deleteShare, shareUrl, updateShare, type Expiry, type ShareOptions } from "../lib/shares";
import { errorText, t } from "../i18n/index";
import { Dialog } from "./dialogs";
import { showToast } from "./toast";
import type { Share } from "../lib/types";

type Props = { entryPath: string; kind: "file" | "folder"; onClose(): void };

const choices: { value: string; days: Expiry; label: "share.in1" | "share.in7" | "share.in30" | "share.never" }[] = [
  { value: "1", days: 1, label: "share.in1" },
  { value: "7", days: 7, label: "share.in7" },
  { value: "30", days: 30, label: "share.in30" },
  { value: "never", days: null, label: "share.never" },
];

export function ShareDialog({ entryPath, kind, onClose }: Props) {
  const name = displaySegment(segments(entryPath).at(-1) ?? "");
  const [share, setShare] = useState<Share | null>(null);
  // The expiry the link was made with: Done sends it again only when the choice changed.
  const [savedDays, setSavedDays] = useState<Expiry>(7);
  const [days, setDays] = useState<Expiry>(7);
  const [locked, setLocked] = useState(false);
  const [password, setPassword] = useState("");
  const [download, setDownload] = useState(true);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  // A new password of at least 8 characters is needed, unless the link already has one and the field is left empty.
  const keeping = password === "" && !!share?.hasPassword;
  const needsPassword = locked && !keeping && [...password].length < 8;

  const run = async (work: () => Promise<void>) => {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await work();
    } catch (e) {
      setError(errorText(e instanceof ApiFailure ? e : offline()));
    }
    setBusy(false);
  };

  const create = () =>
    run(async () => {
      const s = await createShare(entryPath, { expiresDays: days, password: locked ? password : null, download });
      setShare(s);
      setSavedDays(days);
      setPassword("");
    });

  const done = () =>
    run(async () => {
      const s = share!;
      const patch: Partial<ShareOptions> = {};
      if (days !== savedDays) patch.expiresDays = days;
      if (download !== s.download) patch.download = download;
      if (!locked && s.hasPassword) patch.password = null;
      if (locked && password !== "") patch.password = password;
      if (Object.keys(patch).length) await updateShare(s.id, patch);
      onClose();
    });

  const remove = () =>
    run(async () => {
      await deleteShare(share!.id);
      onClose();
      showToast(t("share.deleted", { name }));
    });

  const copy = async () => {
    showToast((await copyText(shareUrl(share!))) ? t("share.copied") : t("share.copyFailed"));
  };

  return (
    <Dialog title={t("share.title")} onClose={onClose}>
      <form
        class="dialog-form share-form"
        noValidate
        onSubmit={(e) => {
          e.preventDefault();
          if (share) done();
          else if (!needsPassword) create();
        }}
      >
        <p class="mono share-path">{displayPath(entryPath)}</p>
        {share && (
          <div class="field">
            <label class="field-label" for="share-link">
              {t(kind === "folder" ? "share.leadFolder" : "share.leadFile")}
            </label>
            <div class="share-link">
              <input id="share-link" class="input mono" readOnly value={shareUrl(share)} onFocus={(e) => e.currentTarget.select()} />
              <button type="button" class="button" onClick={copy}>
                {t("share.copy")}
              </button>
            </div>
          </div>
        )}
        <div class="field">
          <label class="field-label" for="share-expires">
            {t("share.expires")}
          </label>
          <select
            id="share-expires"
            class="input"
            value={choices.find((c) => c.days === days)!.value}
            onChange={(e) => setDays(choices.find((c) => c.value === e.currentTarget.value)!.days)}
          >
            {choices.map((c) => (
              <option value={c.value}>{t(c.label)}</option>
            ))}
          </select>
        </div>
        <div class="field switch-field">
          <input
            id="share-locked"
            type="checkbox"
            role="switch"
            class="switch"
            checked={locked}
            onChange={() => setLocked((v) => !v)}
          />
          <label class="field-label" for="share-locked">
            {t("share.password")}
          </label>
        </div>
        {locked && (
          <div class="field">
            <label class="field-label" for="share-password">
              {t("share.passwordField")}
            </label>
            <input
              id="share-password"
              class="input"
              type="password"
              autocomplete="new-password"
              value={password}
              maxLength={256}
              aria-describedby="share-password-hint"
              onInput={(e) => setPassword(e.currentTarget.value)}
            />
            <p id="share-password-hint" class="hint">
              {t("share.passwordRule")}
              {share?.hasPassword && ` ${t("share.passwordKeep")}`}
            </p>
          </div>
        )}
        <div class="field switch-field">
          <input
            id="share-download"
            type="checkbox"
            role="switch"
            class="switch"
            checked={download}
            aria-describedby="share-note"
            onChange={() => setDownload((v) => !v)}
          />
          <label class="field-label" for="share-download">
            {t("share.download")}
          </label>
          <p id="share-note" class="hint">
            {t("share.note")}
          </p>
        </div>
        {error && (
          <p class="error-line" role="alert">
            {error}
          </p>
        )}
        <div class="dialog-buttons">
          {share ? (
            <>
              <button type="button" class="button danger share-delete" disabled={busy} onClick={remove}>
                {t("share.delete")}
              </button>
              <button type="submit" class="button primary" disabled={busy || needsPassword}>
                {t("share.done")}
              </button>
            </>
          ) : (
            <>
              <button type="button" class="button" onClick={onClose}>
                {t("dialog.cancel")}
              </button>
              <button type="submit" class="button primary" disabled={busy || needsPassword}>
                {t("share.create")}
              </button>
            </>
          )}
        </div>
      </form>
    </Dialog>
  );
}
