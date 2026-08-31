import { useState } from "react";
import { ACCOUNT_COLORS, api, FASTMAIL_SESSION_URL } from "../api";
import type { Account, VerifiedAccount } from "../types";
import { wordmarkFor } from "../theme";
import { ProviderPicker } from "./ProviderPicker";
import { Panel } from "./Panel";
import { PROVIDERS, type Provider } from "../providers";

interface SignInProps {
  /** How many accounts already exist, so the next colour is a new one. */
  existingCount: number;
  /**
   * Colours already in use. Counting accounts was not enough: replacing an
   * account leaves the count unchanged, so two accounts could be handed the
   * same colour — and provenance colour is the only thing telling them apart
   * in a unified list.
   */
  usedColors?: string[];
  /** Set when renewing an existing account rather than adding a new one. */
  reconnecting?: Account | null;
  onConnected: () => void;
  onCancel?: () => void;
}

/**
 * Two steps on purpose.
 *
 * The token is checked against JMAP *before* anything is written, so the app can
 * say who it belongs to and a mistyped token never becomes a saved-but-broken
 * account. Only after that does it reach the OS credential store — the token is
 * never written to config.json, and never leaves this machine.
 */
export function SignIn({
  existingCount,
  usedColors = [],
  reconnecting = null,
  onConnected,
  onCancel,
}: SignInProps) {
  const [token, setToken] = useState("");
  const [verified, setVerified] = useState<VerifiedAccount | null>(null);
  const [label, setLabel] = useState("");
  const [identity, setIdentity] = useState("");
  const [color, setColor] = useState(
    // Keep an account's own colour when reconnecting; otherwise take the first
    // one nobody is using, falling back to the rotation once they run out.
    () =>
      reconnecting?.color ??
      ACCOUNT_COLORS.find((c) => !usedColors.includes(c)) ??
      ACCOUNT_COLORS[existingCount % ACCOUNT_COLORS.length],
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // OAuth is the way in. The token form stays available because it is the only
  // option if the browser round trip cannot complete — a locked-down machine, or
  // a headless run — but it is not what anyone should meet first.
  const [showTokenForm, setShowTokenForm] = useState(false);
  // Which host was chosen. Null means the picker is showing.
  const [provider, setProvider] = useState<Provider | null>(() => {
    // Reconnecting already knows the host, so skip the picker and go straight
    // to the form that asks for what we genuinely cannot keep: the password.
    if (reconnecting?.connection !== "imap") return null;
    const domain = reconnecting.identity.split("@")[1] ?? "";
    const stem = domain.split(".")[0];
    return PROVIDERS.find((p) => p.imap && p.id === stem) ?? null;
  });
  const [imapUser, setImapUser] = useState(reconnecting?.identity ?? "");
  const [imapPassword, setImapPassword] = useState("");

  async function connectImap() {
    if (!provider?.imap) return;
    setBusy(true);
    setError(null);
    try {
      await api.connectImap({
        host: provider.imap.host,
        port: provider.imap.port,
        username: imapUser.trim(),
        password: imapPassword.trim(),
        label: "",
        color,
      });
      setImapPassword("");
      onConnected();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function connect() {
    setBusy(true);
    setError(null);
    try {
      await api.connectFastmail(color);
      onConnected();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function verify() {
    setBusy(true);
    setError(null);
    try {
      const result = await api.verifyToken(FASTMAIL_SESSION_URL, token.trim());
      setVerified(result);
      setLabel(result.suggestedLabel);
      setIdentity(result.username);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function save() {
    if (!verified) return;
    setBusy(true);
    setError(null);
    try {
      await api.saveAccount({
        id: verified.suggestedId,
        label: label.trim() || verified.suggestedLabel,
        color,
        identity: identity.trim() || verified.username,
        sessionUrl: FASTMAIL_SESSION_URL,
        token: token.trim(),
      });
      setToken("");
      onConnected();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  // One level at a time: a confirmation goes back to the form, a form back to
  // the picker, and only the picker itself leaves the panel.
  function goBack() {
    if (verified) setVerified(null);
    else if (showTokenForm) setShowTokenForm(false);
    else if (provider) setProvider(null);
    else onCancel?.();
  }

  return (
    <Panel
      layout="center"
      onBack={verified || showTokenForm || provider || onCancel ? goBack : undefined}
    >
      <div className="setup">
        <img className="setup-mark" src={wordmarkFor()} alt="BazMail" draggable={false} />
        <h2>
          {reconnecting
            ? `Reconnect ${reconnecting.label}`
            : "Connect an email account"}
        </h2>

        {!verified && !showTokenForm && !provider ? (
          <>
            <p className="setup-note">
              Who hosts your mail? Every provider connects differently, so this
              also tells you what each one needs.
            </p>

            <ProviderPicker
              onPick={(picked) => {
                setError(null);
                setProvider(picked);
                // Only the OAuth providers can go straight to the browser; the
                // IMAP ones need credentials typed first.
                if (picked.available && !picked.imap) void connect();
              }}
            />

            <div className="field-row">
              <button className="link-quiet" onClick={() => setShowTokenForm(true)}>
                Use an API token instead
              </button>
            </div>
          </>
        ) : !verified && !showTokenForm && provider ? (
          <>
            {provider.available && provider.imap ? (
              <>
                <p className="setup-note">
                  Apple offers no OAuth for third-party mail, so iCloud needs an{" "}
                  <strong>app-specific password</strong> — not your Apple ID
                  password. Generate one at <code>appleid.apple.com</code> under
                  Sign-In and Security, with two-factor authentication switched on.
                </p>

                <label className="field">
                  <span className="field-label">iCloud address</span>
                  <input
                    className="field-input"
                    value={imapUser}
                    autoComplete="off"
                    spellCheck={false}
                    placeholder="you@icloud.com"
                    onChange={(e) => setImapUser(e.target.value)}
                  />
                </label>

                <label className="field">
                  <span className="field-label">App-specific password</span>
                  <input
                    className="field-input"
                    type="password"
                    value={imapPassword}
                    autoComplete="off"
                    spellCheck={false}
                    placeholder="xxxx-xxxx-xxxx-xxxx"
                    onChange={(e) => setImapPassword(e.target.value)}
                    onKeyDown={(e) => {
                      if (
                        e.key === "Enter" &&
                        imapUser.trim() &&
                        imapPassword.trim() &&
                        !busy
                      ) {
                        void connectImap();
                      }
                    }}
                  />
                </label>

                <div className="field-row">
                  <button
                    className="btn-primary"
                    onClick={() => void connectImap()}
                    disabled={busy || !imapUser.trim() || !imapPassword.trim()}
                  >
                    {busy ? "Connecting…" : "Connect iCloud"}
                  </button>
                  <button className="btn-quiet" onClick={() => setProvider(null)}>
                    Back
                  </button>
                </div>
              </>
            ) : provider.available ? (
              <>
                <p className="setup-note">
                  Approve BazMail in the browser window that just opened. It asks
                  for mail only — not contacts or calendars — and you can revoke
                  it from {provider.name}'s own settings at any time.
                </p>
                <div className="field-row">
                  <button
                    className="btn-primary"
                    onClick={() => void connect()}
                    disabled={busy}
                  >
                    {busy ? "Waiting for your browser…" : "Try again"}
                  </button>
                  <button className="btn-quiet" onClick={() => setProvider(null)}>
                    Back
                  </button>
                </div>
              </>
            ) : (
              <>
                <p className="setup-note">
                  <strong>{provider.name}</strong> isn't connected yet — it needs{" "}
                  {provider.method.toLowerCase()}.
                </p>
                <p className="setup-note setup-aside">{provider.note}</p>
                <div className="field-row">
                  <button className="btn-quiet" onClick={() => setProvider(null)}>
                    Pick another
                  </button>
                </div>
              </>
            )}
          </>
        ) : !verified ? (
          <>
            <p className="setup-note">
              In Fastmail, go to <strong>Settings → Privacy &amp; Security →
              Integrations</strong> and create an API token with read and write
              access to Mail. Paste it below — it goes straight to the Windows
              Credential Manager and never touches config.json.
            </p>

            <label className="field">
              <span className="field-label">API token</span>
              <input
                className="field-input"
                type="password"
                value={token}
                autoComplete="off"
                spellCheck={false}
                placeholder="fmu1-…"
                onChange={(e) => setToken(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && token.trim() && !busy) void verify();
                }}
              />
            </label>

            <div className="field-row">
              <button
                className="btn-primary"
                onClick={() => void verify()}
                disabled={busy || token.trim().length === 0}
              >
                {busy ? "Checking…" : "Connect"}
              </button>
              <button className="btn-quiet" onClick={() => setShowTokenForm(false)}>
                Back
              </button>
            </div>
          </>
        ) : (
          <>
            <p className="setup-note">
              Connected as <strong>{verified.username}</strong>
              {verified.accountName && ` · ${verified.accountName}`}. Nothing is
              saved until you confirm.
            </p>

            <label className="field">
              <span className="field-label">Sidebar name</span>
              <input
                className="field-input"
                value={label}
                onChange={(e) => setLabel(e.target.value)}
              />
            </label>

            <label className="field">
              <span className="field-label">Send from</span>
              <input
                className="field-input"
                value={identity}
                onChange={(e) => setIdentity(e.target.value)}
              />
            </label>

            <div className="field">
              <span className="field-label">Colour</span>
              <div className="swatches">
                {ACCOUNT_COLORS.map((c) => (
                  <button
                    key={c}
                    className={`swatch ${c === color ? "selected" : ""}`}
                    style={{ background: c }}
                    aria-label={`Use ${c}`}
                    onClick={() => setColor(c)}
                  />
                ))}
              </div>
            </div>

            <div className="field-row">
              <button
                className="btn-primary"
                onClick={() => void save()}
                disabled={busy}
              >
                {busy ? "Saving…" : "Save and sync"}
              </button>
              <button
                className="btn-quiet"
                onClick={() => {
                  setVerified(null);
                  setError(null);
                }}
              >
                Back
              </button>
            </div>
          </>
        )}

        {error && <div className="field-error">{error}</div>}
      </div>
    </Panel>
  );
}
