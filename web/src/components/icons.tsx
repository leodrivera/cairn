// Stroke icons, 24 px grid. Decorative: the control around them carries the name.

export const paths = {
  alert: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18z M12 8v5 M12 16h.01",
  archive: "M4 4h16v5H4z M5 9v11h14V9 M10 13h4",
  back: "M15 6l-6 6 6 6",
  check: "M5 12l5 5 9-10",
  close: "M6 6l12 12 M18 6L6 18",
  code: "M8 8l-4 4 4 4 M16 8l4 4-4 4 M14 5l-4 14",
  copy: "M9 9h11v11H9z M5 15H4V4h11v1",
  download: "M12 4v12 M7 11l5 5 5-5 M4 20h16",
  file: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z M14 3v5h5",
  film: "M4 5h16v14H4z M4 9h16 M4 15h16 M8 5v14 M16 5v14",
  folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
  folderPlus: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z M12 10v6 M9 13h6",
  grid: "M4 4h7v7H4z M13 4h7v7h-7z M4 13h7v7H4z M13 13h7v7h-7z",
  home: "M4 11l8-7 8 7 M6 10v10h12V10",
  image: "M5 4h14a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1z M4 16l5-5 4 4 3-3 4 4 M9 8.5h.01",
  link: "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1 M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1",
  list: "M8 6h12 M8 12h12 M8 18h12 M4 6h.01 M4 12h.01 M4 18h.01",
  lock: "M6 11h12v9H6z M8 11V8a4 4 0 0 1 8 0v3",
  menu: "M4 7h16 M4 12h16 M4 17h16",
  more: "M5 12h.01 M12 12h.01 M19 12h.01",
  move: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z M9 13h6 M13 11l2 2-2 2",
  music: "M9 18V5l11-2v13 M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0z M20 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z",
  rename: "M4 20h4l10-10-4-4L4 16z M13 7l4 4",
  search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14z M20 20l-4-4",
  signOut: "M14 4h5v16h-5 M10 8l-4 4 4 4 M6 12h10",
  sliders: "M4 6h10 M18 6h2 M4 12h4 M12 12h8 M4 18h12 M14 4v4 M8 10v4 M16 16v4",
  trash: "M4 7h16 M9 7V4h6v3 M6 7l1 13h10l1-13 M10 11v5 M14 11v5",
  upload: "M12 16V4 M7 9l5-5 5 5 M4 20h16",
  user: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8z M4 20v-1a5 5 0 0 1 5-5h6a5 5 0 0 1 5 5v1",
  users:
    "M16 20v-1a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v1 M10 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z M20 20v-1a4 4 0 0 0-3-3.9 M16 4.2a3.5 3.5 0 0 1 0 6.6",
} as const;

export type IconName = keyof typeof paths;

export function Icon({ name, size = 18, class: cls }: { name: IconName; size?: number; class?: string }) {
  return (
    <svg
      class={cls ? `icon ${cls}` : "icon"}
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      stroke-width="1.75"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d={paths[name]} />
    </svg>
  );
}

export function Mark({ size = 30 }: { size?: number }) {
  return (
    <svg viewBox="0 0 48 48" width={size} height={size} aria-hidden="true" class="mark">
      <rect x="5" y="33" width="38" height="10" rx="5" fill="currentColor" />
      <rect x="10" y="20" width="26" height="10" rx="5" fill="currentColor" />
      <rect x="19" y="8" width="14" height="9" rx="4.5" class="accent" />
    </svg>
  );
}
