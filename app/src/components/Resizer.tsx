import { useEffect, useRef } from "react";

/** Narrow enough that the message list keeps two readable lines, wide enough
 *  that a long subject fits — past either end the layout stops being useful
 *  rather than becoming more configurable. */
export const LIST_MIN = 280;
export const LIST_MAX = 620;
export const LIST_DEFAULT = 366;

export function clampListWidth(width: number): number {
  return Math.min(LIST_MAX, Math.max(LIST_MIN, Math.round(width)));
}

interface ResizerProps {
  width: number;
  /** Fires continuously through the drag, so the layout tracks the pointer. */
  onChange: (width: number) => void;
  /** Fires once when the drag ends, for persisting the result. */
  onCommit: (width: number) => void;
}

/**
 * The grab strip between the message list and the reader.
 *
 * The drag is tracked on the window rather than on this element. Pointer
 * capture is the tidier-looking approach and it did not work here: the strip is
 * nine pixels wide, a drag leaves it within the first frame, and once the moves
 * stop arriving the column simply stops following the cursor — which reads as
 * "the divider highlights but will not drag".
 *
 * Window listeners have no such failure mode. They also keep working past the
 * clamp, so a cursor that runs well beyond the maximum still drags the column
 * back when it returns, instead of dropping the gesture at the limit.
 */
export function Resizer({ width, onChange, onCommit }: ResizerProps) {
  const drag = useRef<{ x: number; from: number; latest: number } | null>(null);

  useEffect(() => {
    const onMove = (event: PointerEvent) => {
      const active = drag.current;
      if (!active) return;
      // Held so the pointer cannot outrun the layout and lose the gesture.
      event.preventDefault();
      const next = clampListWidth(active.from + (event.clientX - active.x));
      active.latest = next;
      onChange(next);
    };

    const onEnd = () => {
      const active = drag.current;
      if (!active) return;
      drag.current = null;
      document.body.classList.remove("resizing");
      onCommit(active.latest);
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onEnd);
    // A cancelled pointer must still end the drag, or the body keeps the
    // resize cursor and every later click looks like it is about to drag.
    window.addEventListener("pointercancel", onEnd);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onEnd);
      window.removeEventListener("pointercancel", onEnd);
    };
  }, [onChange, onCommit]);

  return (
    <div
      className="resizer"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize the message list"
      onPointerDown={(event) => {
        event.preventDefault();
        drag.current = { x: event.clientX, from: width, latest: width };
        // Keeps the resize cursor while the pointer is anywhere on screen, and
        // stops the drag selecting text in the panes it passes over.
        document.body.classList.add("resizing");
      }}
      // Dragging a column to an awkward width is easy; getting back to the
      // original by eye is not.
      onDoubleClick={() => {
        onChange(LIST_DEFAULT);
        onCommit(LIST_DEFAULT);
      }}
    />
  );
}
