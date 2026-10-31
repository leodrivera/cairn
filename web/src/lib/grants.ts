// Grants: sharing with other Cairn users, as API calls.
import { api } from "./api";
import type { GivenGrant, ReceivedGrant } from "./types";
import type { Expiry } from "./shares";

export type GrantResult =
  | { email: string; status: "added"; grant: GivenGrant }
  | { email: string; status: "no_account" | "self"; message: string };

export async function createGrants(
  path: string,
  emails: string[],
  permission: "read" | "write",
  expiresDays: Expiry,
): Promise<GrantResult[]> {
  return (await api<{ results: GrantResult[] }>("POST", "/api/grants", { path, emails, permission, expiresDays })).results;
}

export async function listGiven(): Promise<GivenGrant[]> {
  return (await api<{ grants: GivenGrant[] }>("GET", "/api/grants")).grants;
}

export async function listReceived(): Promise<ReceivedGrant[]> {
  return (await api<{ grants: ReceivedGrant[] }>("GET", "/api/grants/received")).grants;
}

export async function grantInfo(id: string): Promise<ReceivedGrant> {
  return (await api<{ grant: ReceivedGrant }>("GET", `/api/g/${encodeURIComponent(id)}`)).grant;
}

export async function updateGrant(
  id: string,
  patch: { permission?: "read" | "write"; expiresDays?: Expiry },
): Promise<GivenGrant> {
  return (await api<{ grant: GivenGrant }>("PATCH", `/api/grants/${encodeURIComponent(id)}`, patch)).grant;
}

export async function deleteGrant(id: string): Promise<void> {
  await api<void>("DELETE", `/api/grants/${encodeURIComponent(id)}`);
}

/** E-mails as typed: split on commas, spaces and new lines, blanks dropped. */
export function splitEmails(text: string): string[] {
  return text.split(/[\s,;]+/).filter(Boolean);
}
