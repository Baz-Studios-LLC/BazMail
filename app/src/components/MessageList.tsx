import { useEffect, useRef } from "react";
import type { Account, Envelope } from "../types";
import { decodeEntities, displayName, formatWhen, initials } from "../types";
import { PaperclipIcon } from "./Icons";

interface MessageListProps {
  title: string;
  envelopes: Envelope[];
  accounts: Account[];
  selectedId: string | null;
  onSelect: (envelope: Envelope) => void;
}

export function MessageList({
  title,
  envelopes,
  accounts,
  selectedId,
  onSelect,
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
            {unread} unread
            {accountCount > 1 && ` · ${accountCount} accounts`}
          </div>
        </div>
      </header>

      <div className="rows">
        {envelopes.map((envelope) => {
          const selected = envelope.id === selectedId;
          const ring = colors[envelope.accountId] ?? "transparent";
          return (
            <button
              key={`${envelope.accountId}:${envelope.id}`}
              ref={selected ? selectedRef : undefined}
              className={`row ${selected ? "selected" : ""} ${
                envelope.isUnread ? "unread" : ""
              }`}
              onClick={() => onSelect(envelope)}
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
