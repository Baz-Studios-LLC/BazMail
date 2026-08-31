import { useCallback, useRef } from "react";

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
 * Uses pointer capture rather than window listeners: the pointer keeps
 * reporting to this element even when it outruns the drag — which it will,
 * since the clamp stops the layout at a limit the cursor is happy to pass.
 * Without capture the release would land on whatever is under the cursor and
 * the strip would stay stuck in a drag.
 */
export function Resizer({ width, onChange, onCommit }: ResizerProps) {
  const origin = useRef<{ x: number; width: number } | null>(null);

  const onPointerMove = useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      if (!origin.current) return;
      onChange(clampListWidth(origin.current.width + (event.clientX - origin.current.x)));
    },
    [onChange],
  );

  return (
    <div
      className="resizer"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize the message list"
      onPointerDown={(event) => {
        origin.current = { x: event.clientX, width };
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={onPointerMove}
      onPointerUp={(event) => {
        if (origin.current) onCommit(width);
        origin.current = null;
        event.currentTarget.releasePointerCapture(event.pointerId);
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
