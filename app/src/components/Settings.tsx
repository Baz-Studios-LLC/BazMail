import { useState } from "react";
import { ACCOUNT_COLORS, api } from "../api";
import type { Account, Status } from "../types";
import { Panel } from "./Panel";

interface SettingsProps {
  status: Status | null;
  accounts: Account[];
  markReadDelay: number;
  onMarkReadDelayChange: (ms: number) => void;
  onAddAccount: () => void;
  /** Renews an account's credential in place, keeping its id and settings. */
  onReconnect: (account: Account) => void;
  /** Called after an account is removed so the shell can reload. */
  onChanged: () => void;
  onClose: () => void;
}

const DELAYS = [
  { ms: 0, label: "Immediately" },
  { ms: 500, label: "After ½ second" },
  { ms: 900, label: "After 1 second" },
  { ms: 2000, label: "After 2 seconds" },
  { ms: -1, label: "Never" },
];

export function Settings({
  status,
  accounts,
  markReadDelay,
  onMarkReadDelayChange,
  onAddAccount,
  onReconnect,
  onChanged,
  onClose,
}: SettingsProps) {
  // Removing an account is not undoable, so it takes two clicks rather than one.
  const [confirming, setConfirming] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function recolor(accountId: string, color: string) {
    setError(null);
    try {
      await api.setAccountColor(accountId, color);
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  }

  async function remove(accountId: string) {
    setBusy(true);
    setError(null);
    try {
      await api.deleteAccount(accountId);
      setConfirming(null);
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Panel title="Settings" onBack={onClose} backLabel="Back to mail">

      <section className="settings-section">
        <h3>Accounts</h3>
        {accounts.length === 0 && (
          <p className="setup-note">No accounts connected yet.</p>
        )}

        {accounts.map((account) => (
          <div className="account-row" key={account.id}>
            <span className="dot" style={{ background: account.color }} />
            <span className="account-main">
              <span className="account-label">{account.label}</span>
              <span className="account-identity">{account.identity}</span>
              {/* Provenance colour is the only thing separating two accounts
                  at a glance in a unified list, so a collision makes that list
                  unreadable — and two accounts really can end up the same. */}
              <span className="swatches account-swatches">
                {ACCOUNT_COLORS.map((c) => (
                  <button
                    key={c}
                    className={`swatch ${c === account.color ? "selected" : ""}`}
                    style={{ background: c }}
                    aria-label={`Use ${c} for ${account.label}`}
                    title={`Use ${c} for ${account.label}`}
                    onClick={() => void recolor(account.id, c)}
                  />
                ))}
              </span>
            </span>

            {confirming !== account.id && (
              <button
                className="btn-quiet"
                disabled={busy}
                title={
                  account.connection === "oauth"
                    ? "Approve BazMail again in your browser"
                    : account.connection === "imap"
                      ? "Enter the app password again"
                      : "Paste a new API token"
                }
                onClick={() => onReconnect(account)}
              >
                Reconnect
              </button>
            )}

            {confirming === account.id ? (
              <>
                <span className="account-warn">
                  Removes the stored credential too.
                </span>
                <button
                  className="btn-danger"
                  disabled={busy}
                  onClick={() => void remove(account.id)}
                >
                  Remove
                </button>
                <button className="btn-quiet" onClick={() => setConfirming(null)}>
                  Cancel
                </button>
              </>
            ) : (
              <button
                className="btn-quiet"
                onClick={() => setConfirming(account.id)}
              >
                Remove
              </button>
            )}
          </div>
        ))}

        <div className="field-row">
          <button className="btn-primary" onClick={onAddAccount}>
            Add account
          </button>
          <span className="setup-aside">
            Signing in with an address you already have replaces that account
            rather than adding a second one — which is how you reconnect a broken
            one.
          </span>
        </div>

        {error && <div className="field-error">{error}</div>}
      </section>

      <section className="settings-section">
        <h3>Reading</h3>
        <label className="field">
          <span className="field-label">Mark as read</span>
          <select
            className="field-input"
            value={markReadDelay}
            onChange={(e) => onMarkReadDelayChange(Number(e.target.value))}
          >
            {DELAYS.map((d) => (
              <option key={d.ms} value={d.ms}>
                {d.label}
              </option>
            ))}
          </select>
        </label>
        <p className="setup-aside">
          A delay matters because <kbd>j</kbd> and <kbd>k</kbd> open each message
          as they move. Marking immediately would mark everything you pass.
        </p>
      </section>

      <section className="settings-section">
        <h3>Storage</h3>
        <dl className="facts">
          <dt>Version</dt>
          <dd>{status?.version ?? "—"}</dd>

          <dt>Accounts file</dt>
          <dd className="path">{status?.configPath ?? "—"}</dd>

          <dt>Local mirror</dt>
          <dd className="path">{status?.storePath ?? "—"}</dd>

          <dt>Encrypted at rest</dt>
          <dd>
            {!status ? (
              "—"
            ) : status.storeProtection === "encrypted" ? (
              "Yes"
            ) : status.storeProtection === "volume" ? (
              <span>
                By the disk — FileVault encrypts the whole volume when it is
                switched on. BazMail neither manages nor verifies that, so this
                is worth confirming in System Settings rather than taking on
                trust.
              </span>
            ) : (
              <span className="fact-warn">
                No — every synced message is readable by anything that can read
                your disk. Windows refused the request on this machine.
              </span>
            )}
          </dd>

          <dt>Queued changes</dt>
          <dd>
            {status?.queuedMutations ?? 0}
            {(status?.queuedMutations ?? 0) > 0 && " waiting to reach the server"}
          </dd>
        </dl>
      </section>
    </Panel>
  );
}
