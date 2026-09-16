import { useEffect, useLayoutEffect, useRef, useState } from "react";

/**
 * One item. `danger` is for actions that destroy something, `disabled` for ones
 * that do not apply right now — kept as a state rather than an omission so the
 * menu does not change shape between messages, which is how you end up clicking
 * Delete when you meant Archive.
 */
export interface MenuItem {
  label: string;
  onSelect: () => void;
  danger?: boolean;
  disabled?: boolean;
  /** Drawn as a rule above this item. */
  separated?: boolean;
}

interface ContextMenuProps {
  /** Where the pointer was, in client coordinates. */
  at: { x: number; y: number };
  items: MenuItem[];
  onClose: () => void;
}

/** Keeps the menu on screen when opened near an edge. */
const MARGIN = 8;

/**
 * A menu at a point, for right-clicks.
 *
 * Written once. There were already three hand-rolled menus in this app by the
 * time a fourth was wanted, each with its own dismissal rules and its own
 * idea of padding — which is the ordinary way a set of menus stops looking
 * like one app.
 */
export function ContextMenu({ at, items, onClose }: ContextMenuProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState(at);

  // Measured after paint, because the correction needs the menu's real size.
  // Placing it first and adjusting is invisible; guessing a height is not.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const box = el.getBoundingClientRect();
    setPos({
      x: Math.min(at.x, window.innerWidth - box.width - MARGIN),
      y: Math.min(at.y, window.innerHeight - box.height - MARGIN),
    });
  }, [at.x, at.y]);

  // Every way out. A menu that survives a scroll or an alt-tab ends up floating
  // over a list it no longer refers to, still pointing at a message that has
  // moved.
  useEffect(() => {
    const dismiss = () => onClose();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("pointerdown", dismiss);
    window.addEventListener("blur", dismiss);
    window.addEventListener("resize", dismiss);
    // Capture: a scroll inside the list does not bubble to the window.
    window.addEventListener("scroll", dismiss, true);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("pointerdown", dismiss);
      window.removeEventListener("blur", dismiss);
      window.removeEventListener("resize", dismiss);
      window.removeEventListener("scroll", dismiss, true);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      className="context-menu"
      role="menu"
      style={{ left: pos.x, top: pos.y }}
      // The dismissal above listens on the window, so a click inside would
      // close the menu before the item's own handler ran.
      onPointerDown={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.preventDefault()}
    >
      {items.map((item, i) => (
        <button
          key={`${item.label}-${i}`}
          className={`context-menu-item ${item.danger ? "danger" : ""} ${
            item.separated ? "separated" : ""
          }`}
          role="menuitem"
          disabled={item.disabled}
          onClick={() => {
            onClose();
            item.onSelect();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}
