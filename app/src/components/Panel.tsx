import type { ReactNode } from "react";
import { useEffect } from "react";
import { BackIcon } from "./Icons";

/**
 * A page: anything that takes over the reading pane.
 *
 * The page owns its own chrome — the header bar, where the back control sits,
 * how the body scrolls. Screens supply content and header props, and nothing
 * else. That division is the whole point: when Settings and sign-in each laid
 * out their own header, the two drifted and the back arrow ended up in a
 * different place in each. A page is defined here, once, and every screen
 * inherits it whether or not its author thought about layout.
 *
 * `layout` is the only structural choice a screen gets, because there really
 * are two shapes: a long form that scrolls from the top, and a short one that
 * sits in the middle of the pane.
 */
interface PanelProps {
  title?: string;
  /** Pops one level: within the page first, out of it only at the top. */
  onBack?: () => void;
  backLabel?: string;
  /** Right-hand end of the header bar. */
  actions?: ReactNode;
  layout?: "scroll" | "center";
  children: ReactNode;
}

export function Panel({
  title,
  onBack,
  backLabel = "Back",
  actions,
  layout = "scroll",
  children,
}: PanelProps) {
  useEffect(() => {
    if (!onBack) return;
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      // Escape belongs to the field while you are typing in one.
      if (target && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName)) return;
      if (event.key === "Escape") {
        event.preventDefault();
        onBack();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onBack]);

  return (
    <div className="panel">
      {/* No bottom border, deliberately: the command bar above already draws
          one, and a second rule a few pixels below reads as a mistake rather
          than as structure. */}
      <header className="panel-header">
        {onBack && (
          <button
            className="panel-back"
            onClick={onBack}
            aria-label={backLabel}
            title={backLabel}
          >
            <BackIcon size={17} />
          </button>
        )}
        {title && <h2 className="panel-title">{title}</h2>}
        <div className="panel-header-gap" />
        {actions}
      </header>

      <div className={`panel-body ${layout}`}>{children}</div>
    </div>
  );
}
