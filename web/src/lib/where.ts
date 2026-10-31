// Where the explorer and viewer work: the person's own files, or a grant they received. Gives the API base and the
// app's own URLs for a path there.
import { createContext } from "preact";
import { useContext } from "preact/hooks";
import { href } from "./router";
import type { ReceivedGrant } from "./types";

export type Where = {
  /** `/api`, or `/api/g/{id}`. */
  api: string;
  grant: ReceivedGrant | null;
  /** The app's address of a folder here. */
  files(path: string): string;
  /** The app's address of a file here. */
  view(path: string): string;
};

export const own: Where = {
  api: "/api",
  grant: null,
  files: (path) => href({ name: "files", path }),
  view: (path) => href({ name: "view", path }),
};

export function inGrant(g: ReceivedGrant): Where {
  return {
    api: `/api/g/${g.id}`,
    grant: g,
    files: (path) => href({ name: "grant", id: g.id, path, view: false }),
    view: (path) => href({ name: "grant", id: g.id, path, view: true }),
  };
}

export const WhereContext = createContext<Where>(own);

export function useWhere(): Where {
  return useContext(WhereContext);
}
