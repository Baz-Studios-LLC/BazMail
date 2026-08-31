import { useState } from "react";
import { api } from "../api";
import type { Account } from "../types";
import { Panel } from "./Panel";

export interface Draft {
  accountId: string;
  to: string;
  cc: string;
  subject: string;
  text: string;
  inReplyTo?: string | null;
  references?: string[];
}

interface ComposeProps {
  accounts: Account[];
  draft: Draft;
  onClose: () => void;
  /** Called once the server has accepted it, so the shell can refresh Sent. */
  onSent: () => void;
}

/**
 * Splits a typed recipient list into addresses.
 *
 * Commas and semicolons both, because people paste from everywhere. A display
 * name is kept when the input is in `Name <addr>` form and dropped otherwise —
 * inventing one from the local part would put a name on the wire that the
 * recipient never chose.
 */
export function parseRecipients(input: string): { name: string | null; email: string }[] {
  return input
    .split(/[,;]/)
    .map((part) => part.trim())
    .filter(Boolean)
    .map((part) => {
      const angled = part.match(/^(.*?)\s*<([^>]+)>$/);
      if (angled) {
        const name = angled[1].trim().replace(/^["']|["']$/g, "");
        return { name: name || null, email: angled[2].trim() };
      }
      return { name: null, email: part };
    });
}

/** Enough to catch a typo, not enough to reject an address a server would take. */
function looksLikeAddress(email: string): boolean {
  return /^[^@\s]+@[^@\s.]+\.[^@\s]+$/.test(email);
}

export function Compose({ accounts, draft, onClose, onSent }: ComposeProps) {
  const [form, setForm] = useState<Draft>(draft);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const account = accounts.find((a) => a.id === form.accountId) ?? accounts[0];
  const recipients = parseRecipients(form.to);
  const bad = recipients.filter((r) => !looksLikeAddress(r.email));
  const canSend =
    recipients.length > 0 && bad.length === 0 && !sending && account != null;

  function set<K extends keyof Draft>(key: K, value: Draft[K]) {
    setForm((current) => ({ ...current, [key]: value }));
  }

  async function send() {
    if (!account) return;
    setSending(true);
    setError(null);
    try {
      await api.send({
        accountId: account.id,
        to: recipients,
        cc: parseRecipients(form.cc),
        bcc: [],
        subject: form.subject,
        text: form.text,
        inReplyTo: form.inReplyTo ?? null,
        references: form.references ?? [],
      });
      onSent();
    } catch (e) {
      setError(String(e));
    } finally {
      setSending(false);
    }
  }

  return (
    <Panel
      title={form.inReplyTo ? "Reply" : "New message"}
      onBack={onClose}
      backLabel="Discard and go back"
      actions={
        <button className="btn-primary" disabled={!canSend} onClick={() => void send()}>
          {sending ? "Sending…" : "Send"}
        </button>
      }
    >
      <section className="settings-section compose">
        {/* Only shown with more than one account: a picker with one option is
            a decision nobody has to make. */}
        {accounts.length > 1 && (
          <label className="field">
            <span className="field-label">From</span>
            <select
              className="field-input"
              value={form.accountId}
              onChange={(e) => set("accountId", e.target.value)}
            >
              {accounts.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.identity}
                </option>
              ))}
            </select>
          </label>
        )}

        <label className="field">
          <span className="field-label">To</span>
          <input
            className="field-input"
            value={form.to}
            autoComplete="off"
            spellCheck={false}
            placeholder="someone@example.com, another@example.com"
            onChange={(e) => set("to", e.target.value)}
          />
        </label>

        <label className="field">
          <span className="field-label">Cc</span>
          <input
            className="field-input"
            value={form.cc}
            autoComplete="off"
            spellCheck={false}
            onChange={(e) => set("cc", e.target.value)}
          />
        </label>

        <label className="field">
          <span className="field-label">Subject</span>
          <input
            className="field-input"
            value={form.subject}
            onChange={(e) => set("subject", e.target.value)}
          />
        </label>

        <textarea
          className="field-input compose-body"
          value={form.text}
          spellCheck
          placeholder="Write your message…"
          onChange={(e) => set("text", e.target.value)}
        />

        {bad.length > 0 && (
          <div className="setup-aside fact-warn">
            {bad.map((b) => b.email).join(", ")} does not look like an address.
          </div>
        )}


        <p className="setup-aside">
          Plain text for now. It goes out through {account?.identity ?? "your account"} and
          is filed in Sent once the server accepts it.
        </p>

        {error && <div className="field-error">{error}</div>}
      </section>
    </Panel>
  );
}
