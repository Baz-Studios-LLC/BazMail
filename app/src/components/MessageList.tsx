import { useEffect, useRef } from "react";
import type { Account, Envelope } from "../types";
import { decodeEntities, displayName, formatWhen, initials } from "../types";
import { FilterIcon, PaperclipIcon } from "./Icons";

interface MessageListProps {
  title: string;
  envelopes: Envelope[];
  accounts: Account[];
  /** The message the reader is showing, and the one j/k moves from. */
  selectedId: string | null;
  /** Everything ticked. Usually just the focused one. */
  selectedIds: Set<string>;
  onSelect: (envelope: Envelope, mode: "replace" | "toggle" | "range") => void;
  onContextMenu: (envelope: Envelope, at: { x: number; y: number }) => void;
  unreadOnly: boolean;
  onToggleUnreadOnly: () => void;
}

export function MessageList({
  title,
  envelopes,
  accounts,
  selectedId,
  selectedIds,
  onSelect,
  onContextMenu,
  unreadOnly,
  onToggleUnreadOnly,
}: MessageListProps) {
  const colors = Object.fromEntries(accounts.map((a) => [a.id, a.color]));
  const selectedRef = useRef<HTMLButtonElement | null>(null);

  // Keyboard navigation moves the selection; the list has to follow it, or j/k
  // silently walks off the bottom of the viewport.
  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" });
  }, [selectedId]);

  const unread = envelopes.filter((e) => e.isUnread).length;
  const accountCount = new Set(envelopes.map((e) => e.accountId)).size;

  return (
    <section className="list">
      <header className="pane-header">
        <div style={{ flexGrow: 1, display: "flex", flexDirection: "column", gap: 1 }}>
          <div className="pane-title">{title}</div>
          <div className="pane-sub">
            {unreadOnly ? `${unread} unread · filtered` : `${unread} unread`}
            {accountCount > 1 && ` · ${accountCount} accounts`}
          </div>
        </div>

        {/* The block above grows, so this sits hard right. */}
        <button
          className={`pane-filter ${unreadOnly ? "on" : ""}`}
          onClick={onToggleUnreadOnly}
          aria-pressed={unreadOnly}
          title={unreadOnly ? "Showing unread only" : "Show unread only"}
          aria-label={unreadOnly ? "Showing unread only" : "Show unread only"}
        >
          <FilterIcon size={15} />
        </button>
      </header>

      <div className="rows">
        {envelopes.map((envelope) => {
          const focused = envelope.id === selectedId;
          const selected = selectedIds.has(envelope.id);
          const ring = colors[envelope.accountId] ?? "transparent";
          return (
            <button
              key={`${envelope.accountId}:${envelope.id}`}
              ref={focused ? selectedRef : undefined}
              className={`row ${selected ? "selected" : ""} ${
                focused ? "focused" : ""
              } ${envelope.isUnread ? "unread" : ""}`}
              onClick={(event) =>
                onSelect(
                  envelope,
                  event.shiftKey
                    ? "range"
                    : event.ctrlKey || event.metaKey
                      ? "toggle"
                      : "replace",
                )
              }
              // Right-clicking a message that is not in the selection acts on
              // that message alone, which is what every list does: the
              // alternative silently applies an action to rows the pointer is
              // nowhere near.
              onContextMenu={(event) => {
                event.preventDefault();
                onContextMenu(envelope, { x: event.clientX, y: event.clientY });
              }}
            >
              <span
                className="row-unread-dot"
                style={{
                  background: envelope.isUnread ? "var(--gold)" : "transparent",
                }}
              />
              <span className="avatar" style={{ boxShadow: `0 0 0 1.5px ${ring}` }}>
                {initials(envelope.from)}
              </span>
              <span className="row-main">
                <span className="row-top">
                  <span className="row-sender">{displayName(envelope.from)}</span>
                  {envelope.hasAttachment && (
                    <PaperclipIcon size={12} className="row-clip" />
                  )}
                  <span className="row-when">{formatWhen(envelope.receivedAt)}</span>
                </span>
                <span className="row-subject">
                  {decodeEntities(envelope.subject) || "(no subject)"}
                </span>
                <span className="row-preview">{decodeEntities(envelope.preview)}</span>
              </span>
            </button>
          );
        })}

        {envelopes.length === 0 && (
          <div className="empty" style={{ padding: 28 }}>
            Nothing here.
          </div>
        )}
      </div>
    </section>
  );
}
