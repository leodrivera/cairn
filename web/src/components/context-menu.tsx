// The actions menu of an item: right click, its More button, or Shift+F10. Arrows move, Enter runs, Escape closes.
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import { t } from "../i18n/index";

export type MenuItem = { label: string; key?: string; danger?: boolean; run(): void } | "separator";

type Handlers = { open(): void; download(): void; rename(): void; move(): void; copy(): void; remove(): void };

/** The items, in order. Write actions only for people who may write. */
export function entryMenu(canWrite: boolean, h: Handlers): MenuItem[] {
  const items: MenuItem[] = [
    { label: t("menu.open"), key: "Enter", run: h.open },
    { label: t("explorer.download"), run: h.download },
  ];
  if (!canWrite) return items;
  return [
    ...items,
    "separator",
    { label: t("menu.rename"), key: "F2", run: h.rename },
    { label: t("menu.move"), run: h.move },
    { label: t("menu.copy"), run: h.copy },
    "separator",
    { label: t("explorer.delete"), key: "Del", danger: true, run: h.remove },
  ];
}

type Props = { at: { x: number; y: number }; items: MenuItem[]; label?: string; onClose(): void };

export function ContextMenu({ at, items, label, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const [opener] = useState(() => document.activeElement as HTMLElement | null);
  const [pos, setPos] = useState(at);

  const buttons = () => [...(ref.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? [])];

  const close = () => {
    opener?.focus?.();
    onClose();
  };

  useLayoutEffect(() => {
    const el = ref.current!;
    const r = el.getBoundingClientRect();
    setPos({
      x: Math.max(8, Math.min(at.x, innerWidth - r.width - 8)),
      y: Math.max(8, Math.min(at.y, innerHeight - r.height - 8)),
    });
    buttons()[0]?.focus();
  }, [at.x, at.y]);

  useEffect(() => {
    const away = (e: Event) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    document.addEventListener("pointerdown", away);
    addEventListener("blur", onClose);
    addEventListener("resize", onClose);
    return () => {
      document.removeEventListener("pointerdown", away);
      removeEventListener("blur", onClose);
      removeEventListener("resize", onClose);
    };
  }, [onClose]);

  const onKey = (e: KeyboardEvent) => {
    const list = buttons();
    const i = list.indexOf(document.activeElement as HTMLButtonElement);
    const move = (to: number) => list[(to + list.length) % list.length]?.focus();
    if (e.key === "ArrowDown") move(i + 1);
    else if (e.key === "ArrowUp") move(i < 0 ? -1 : i - 1);
    else if (e.key === "Home") move(0);
    else if (e.key === "End") move(-1);
    else if (e.key === "Escape" || e.key === "Tab") close();
    else if (e.key === "Enter" || e.key === " ") {
      if (i >= 0) list[i].click();
    } else return;
    e.preventDefault();
    e.stopPropagation();
  };

  return (
    <div
      ref={ref}
      class="menu context-menu"
      role="menu"
      aria-label={label}
      style={{ left: `${pos.x}px`, top: `${pos.y}px` }}
      onKeyDown={onKey}
    >
      {items.map((item, i) =>
        item === "separator" ? (
          <div key={i} class="menu-separator" role="separator" />
        ) : (
          <button
            key={i}
            type="button"
            role="menuitem"
            tabIndex={-1}
            class={item.danger ? "menu-item danger-text" : "menu-item"}
            onClick={() => {
              close();
              item.run();
            }}
          >
            <span>{item.label}</span>
            {item.key && <kbd class="menu-key">{item.key}</kbd>}
          </button>
        ),
      )}
    </div>
  );
}
