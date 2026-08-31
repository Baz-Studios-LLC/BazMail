import { useState } from "react";
import { ACCOUNT_COLORS, api } from "../api";
import type { Account, Status } from "../types";
import { Panel } from "./Panel";
import { describeUpdate, type UpdateState } from "../useUpdater";

interface SettingsProps {
  status: Status | null;
  accounts: Account[];
  markReadDelay: number;
  onMarkReadDelayChange: (ms: number) => void;
  onAddAccount: () => void;
  /** Renews an account's credential in place, keeping its id and settings. */
  onReconnect: (account: Account) => void;
  updateState: UpdateState;
  onCheckForUpdates: () => void;
  /** Called after an account is removed so the shell can reload. */
  onChanged: () => void;
  onClose: () => void;
}

/**
 * Grouped by what a section *is*, not by when it was written.
 *
 * "Composing" is listed before it has anything in it and "About" swallows what
 * were separate Updates and Storage sections — both are facts about this
 * install rather than things you configure, and splitting them made you look
 * in two places for one answer.
 */
const TABS = [
  { id: "accounts", label: "Accounts" },
  { id: "reading", label: "Reading" },
  { id: "composing", label: "Composing" },
  { id: "privacy", label: "Privacy" },
  { id: "about", label: "About" },
] as const;

type TabId = (typeof TABS)[number]["id"];

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
  updateState,
  onCheckForUpdates,
  onChanged,
  onClose,
}: SettingsProps) {
  // Removing an account is not undoable, so it takes two clicks rather than one.
  const [tab, setTab] = useState<TabId>("accounts");
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
    <Panel
      title="Settings"
      onBack={onClose}
      backLabel="Back to mail"
      actions={
        <nav className="tabs" role="tablist">
          {TABS.map((t) => (
            <button
              key={t.id}
              role="tab"
              aria-selected={tab === t.id}
              className={`tab ${tab === t.id ? "on" : ""}`}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </nav>
      }
    >
      {/* No heading inside a tab — the tab is the heading, and repeating it
          just costs a line at the top of every panel. */}
      {tab === "accounts" && (
      <section className="settings-section">
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
      )}

      {tab === "reading" && (
      <section className="settings-section">
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
      )}

      {tab === "composing" && (
        <section className="settings-section">
          <p className="setup-note">
            Nothing here yet. Signatures, the default sending account and reply
            behaviour land with composing.
          </p>
        </section>
      )}

      {tab === "privacy" && (
        <section className="settings-section">
          <p className="setup-note">
            Remote images are blocked in every message, and each message offers
            to load them once. Nothing is remembered between messages yet —
            per-domain permissions will live here, and only for senders whose
            domain the provider has actually verified.
          </p>
        </section>
      )}

      {tab === "about" && (
      <>
      <section className="settings-section">
        <h3>Updates</h3>
        <div className="field-row">
          <button
            className="btn-quiet"
            disabled={
              updateState.kind === "checking" || updateState.kind === "downloading"
            }
            onClick={onCheckForUpdates}
          >
            Check now
          </button>
          <span
            className={
              updateState.kind === "error" ? "setup-aside fact-warn" : "setup-aside"
            }
          >
            {describeUpdate(updateState, status?.version ?? "")}
          </span>
        </div>
        <p className="setup-aside">
          Updates download in the background and apply on their own. This panel
          exists because silent should mean “does not interrupt you”, not “will
          not say what happened” — a failure nobody can see is a failure nobody
          can fix.
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
      </>
      )}
    </Panel>
  );
}
