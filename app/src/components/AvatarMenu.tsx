import { useEffect, useRef, useState } from "react";
import { SettingsIcon, MailIcon, PeopleIcon } from "./Icons";

interface AvatarMenuProps {
  initials: string;
  onSettings: () => void;
  onAddAccount: () => void;
  onSync: () => void;
  syncing: boolean;
}

/**
 * The account button at the foot of the rail, and the menu it opens.
 *
 * Opens on either click, because the avatar reads as a button and a
 * left-click that does nothing is a dead end — but right-click is where people
 * reach for a menu, so both work.
 */
export function AvatarMenu({
  initials,
  onSettings,
  onAddAccount,
  onSync,
  syncing,
}: AvatarMenuProps) {
  const [open, setOpen] = useState(false);
  const wrapper = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;

    const onPointerDown = (event: MouseEvent) => {
      if (!wrapper.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };

    // Capture phase: a click on something that stops propagation would otherwise
    // leave the menu stuck open behind whatever it opened.
    document.addEventListener("mousedown", onPointerDown, true);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointerDown, true);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  function choose(action: () => void) {
    setOpen(false);
    action();
  }

  return (
    <div className="avatar-menu-wrap" ref={wrapper}>
      {open && (
        <div className="avatar-menu" role="menu">
          <button
            className="avatar-menu-item"
            role="menuitem"
            disabled={syncing}
            onClick={() => choose(onSync)}
          >
            <MailIcon size={15} />
            {syncing ? "Checking…" : "Check for mail"}
          </button>
          <button
            className="avatar-menu-item"
            role="menuitem"
            onClick={() => choose(onAddAccount)}
          >
            <PeopleIcon size={15} />
            Add account
          </button>
          <div className="avatar-menu-rule" />
          <button
            className="avatar-menu-item"
            role="menuitem"
            onClick={() => choose(onSettings)}
          >
            <SettingsIcon size={15} />
            Settings
          </button>
        </div>
      )}

      <button
        className={`avatar-me ${open ? "open" : ""}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        onContextMenu={(e) => {
          e.preventDefault();
          setOpen((v) => !v);
        }}
      >
        {initials}
      </button>
    </div>
  );
}
