import { useState } from "react";
import type { Account, Mailbox } from "../types";
import { MailIcon } from "./Icons";

export interface View {
  kind: "unified" | "mailbox";
  mailboxId?: string;
  accountId?: string;
  title: string;
}

interface SidebarProps {
  accounts: Account[];
  mailboxesByAccount: Record<string, Mailbox[]>;
  view: View;
  unreadTotal: number;
  onSelect: (view: View) => void;
  onAddAccount: () => void;
}

/** Mailboxes people actually navigate to, in the order they expect them. */
const ROLE_ORDER = ["inbox", "drafts", "sent", "archive", "junk", "trash"];

function sortMailboxes(mailboxes: Mailbox[]): Mailbox[] {
  return [...mailboxes].sort((a, b) => {
    const ai = a.role ? ROLE_ORDER.indexOf(a.role) : -1;
    const bi = b.role ? ROLE_ORDER.indexOf(b.role) : -1;
    if (ai !== bi) return (ai < 0 ? 99 : ai) - (bi < 0 ? 99 : bi);
    return a.name.localeCompare(b.name);
  });
}

/** Which accounts are folded away, remembered between runs. */
const COLLAPSED_KEY = "bazmail.collapsedAccounts";

function loadCollapsed(): Set<string> {
  try {
    const raw = localStorage.getItem(COLLAPSED_KEY);
    return new Set(raw ? (JSON.parse(raw) as string[]) : []);
  } catch {
    // A corrupt or unavailable store is not worth failing a render over.
    return new Set();
  }
}

export function Sidebar({
  accounts,
  mailboxesByAccount,
  view,
  unreadTotal,
  onSelect,
  onAddAccount,
}: SidebarProps) {
  const [collapsed, setCollapsed] = useState<Set<string>>(loadCollapsed);

  function toggle(accountId: string) {
    setCollapsed((current) => {
      const next = new Set(current);
      if (!next.delete(accountId)) next.add(accountId);
      try {
        localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...next]));
      } catch {
        // Not being able to remember it is a smaller problem than crashing.
      }
      return next;
    });
  }

  return (
    <nav className="sidebar">
      <div className="sidebar-group">
        <div className="sidebar-label">Unified</div>
        <button
          className={`sidebar-item ${view.kind === "unified" ? "active" : ""}`}
          onClick={() => onSelect({ kind: "unified", title: "Inbox" })}
        >
          <MailIcon size={16} />
          <span className="grow">Inbox</span>
          {unreadTotal > 0 && <span className="sidebar-count unread">{unreadTotal}</span>}
        </button>
      </div>

      {accounts.map((account) => {
        const mailboxes = sortMailboxes(mailboxesByAccount[account.id] ?? []);
        if (mailboxes.length === 0) return null;
        const isCollapsed = collapsed.has(account.id);
        // Unread still surfaces while folded — the point of collapsing is to
        // hide the folders, not to hide that something arrived.
        const accountUnread = mailboxes.reduce((n, m) => n + m.unreadEmails, 0);

        return (
          <div className="sidebar-group" key={account.id}>
            <button
              className="sidebar-label as-toggle"
              onClick={() => toggle(account.id)}
              aria-expanded={!isCollapsed}
            >
              <span className="dot" style={{ background: account.color }} />
              <span className="grow">{account.label}</span>
              {isCollapsed && accountUnread > 0 && (
                <span className="sidebar-count unread">{accountUnread}</span>
              )}
              <svg
                className={`chevron ${isCollapsed ? "closed" : ""}`}
                width="12"
                height="12"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="m6 9 6 6 6-6" />
              </svg>
            </button>
            {!isCollapsed &&
              mailboxes.map((mailbox) => {
              const active =
                view.kind === "mailbox" && view.mailboxId === mailbox.id;
              return (
                <button
                  key={mailbox.id}
                  className={`sidebar-item nested ${active ? "active" : ""}`}
                  onClick={() =>
                    onSelect({
                      kind: "mailbox",
                      mailboxId: mailbox.id,
                      accountId: account.id,
                      title: mailbox.name,
                    })
                  }
                >
                  <span className="grow">{mailbox.name}</span>
                  {mailbox.unreadEmails > 0 && (
                    <span className="sidebar-count unread">{mailbox.unreadEmails}</span>
                  )}
                </button>
                );
              })}
          </div>
        );
      })}

      {/* Adding an account is a once-ever act, and it already lives in the
          profile menu. It earns a permanent slot in the sidebar only while
          there is nothing else here — when it is the only thing left to do. */}
      {accounts.length === 0 && (
        <button className="sidebar-item add-account" onClick={onAddAccount}>
          <span className="grow">Add account…</span>
        </button>
      )}
    </nav>
  );
}
