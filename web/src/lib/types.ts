// Shapes of what the server sends. Field names follow the JSON.

export type Lang = "en" | "pt-BR";

export type Perms = { read: boolean; write: boolean; share: boolean; admin: boolean };

export type User = {
  id: number;
  email: string;
  name: string;
  scope: string;
  permissions: Perms;
  lang: Lang | null;
  theme: "dark" | "light" | null;
  view: "list" | "gallery" | null;
  hasPassword: boolean;
};

export type Entry = {
  name: string;
  kind: "folder" | "file" | "link";
  size: number;
  /** Unix seconds. */
  modified: number;
  mime: string;
  outside?: true;
  /** Only for a name that is not UTF-8: the path segment to use for it. */
  rawName?: string;
};

export type Listing = { path: string; entries: Entry[]; space?: { used: number; total: number } };

export type Config = {
  authMethods: ("password" | "oauth" | "oidc")[];
  captcha: { provider: "recaptcha" | "turnstile" | "hcaptcha"; siteKey: string } | null;
  providers: { name: string; href: string }[];
  setupNeeded?: true;
  theme: "dark" | "light" | null;
  thumbnails: boolean;
  editMax: number;
};

export type Me = {
  user: User | null;
  /** After a provider sign-in for an e-mail with no account; sent once. */
  deniedEmail?: string;
  method?: string;
  security?: { captchaDisabled: boolean };
  /** Whether deletes go to the trash; missing on old servers means no. */
  trash?: { on: boolean };
};

/** One item in the trash, as the person who may restore it sees it. */
export type TrashEntry = {
  id: string;
  name: string;
  rawName?: string;
  kind: "file" | "folder";
  size: number;
  /** The folder it came from, as this person sees it. */
  from: string;
  deletedBy: { id: number; name: string } | null;
  deletedAt: number;
  expiresAt: number;
  /** False when it is reached through a grant: restore only. */
  canDelete: boolean;
};

/** A share link as its owner, or an admin, sees it. */
export type Share = {
  id: string;
  /** Built from the server's public address; null when it has none. */
  url: string | null;
  path: string;
  name: string;
  /** Unix seconds, or null for never. */
  expiresAt: number | null;
  hasPassword: boolean;
  download: boolean;
  hits: number;
  createdAt: number;
  owner: { id: number; name: string };
};

/** What the public page of a link gets. */
export type ShareInfo = { entry: Entry; download: boolean; expiresAt: number | null; sharedBy: string };

/** A grant as its recipient sees it: never the owner's path. */
export type ReceivedGrant = {
  id: string;
  name: string;
  kind: "file" | "folder";
  permission: "read" | "write";
  /** What it allows now: a write grant works as read while its owner cannot write there. */
  effective: "read" | "write";
  expiresAt: number | null;
  createdAt: number;
  owner: { id: number; name: string };
  recipient: { id: number; name: string; email: string };
};

/** A grant as its owner, or an admin, sees it. `kind` is null while the item is not where the grant points. */
export type GivenGrant = Omit<ReceivedGrant, "kind"> & { kind: "file" | "folder" | null; path: string };
