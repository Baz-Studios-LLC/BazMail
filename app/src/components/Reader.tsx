import { useEffect, useState } from "react";
import type { Account, EmailBody, Envelope } from "../types";
import { decodeEntities, displayName, formatWhen } from "../types";
import { remoteImageHosts, sandboxDocument } from "../api";
import { ImageIcon, MailIcon } from "./Icons";

interface ReaderProps {
  envelope: Envelope | null;
  body: EmailBody | null;
  loading: boolean;
  error: string | null;
  accounts: Account[];
}

/** "a, b and 2 more" — the whole list is in the title attribute. */
function describeHosts(hosts: string[]): string {
  if (hosts.length <= 2) return hosts.join(" and ");
  return `${hosts[0]}, ${hosts[1]} and ${hosts.length - 2} more`;
}

export function Reader({ envelope, body, loading, error, accounts }: ReaderProps) {
  // Per message, and never remembered. An allow that outlives the message it
  // was granted for is a decision the user did not make.
  const [loadImages, setLoadImages] = useState(false);

  useEffect(() => {
    setLoadImages(false);
  }, [envelope?.id]);

  if (!envelope) {
    return (
      <div className="empty">
        <MailIcon size={26} />
        <div>Select a message</div>
      </div>
    );
  }

  const account = accounts.find((a) => a.id === envelope.accountId);
  const hosts = body ? remoteImageHosts(body) : [];

  return (
    <>
      <div className="thread-header">
        <div className="thread-subject">{decodeEntities(envelope.subject) || "(no subject)"}</div>
        <div className="thread-meta">
          {account && <span className="dot" style={{ background: account.color }} />}
          <span>{account?.identity ?? envelope.accountId}</span>
          <span className="sep">·</span>
          <span>{displayName(envelope.from)}</span>
          <span className="sep">·</span>
          <span>{formatWhen(envelope.receivedAt)}</span>
        </div>
      </div>

      {error && (
        <div className="banner">
          <span>{error}</span>
        </div>
      )}

      {/* Named hosts rather than a generic warning: seeing a tracking domain
          spelled out is the information that makes the choice a real one. */}
      {!loadImages && hosts.length > 0 && (
        <div className="images-bar">
          <ImageIcon size={16} className="images-bar-icon" />
          <div className="images-bar-text">
            <div className="images-bar-hosts" title={hosts.join("\n")}>
              Images blocked from {describeHosts(hosts)}
            </div>
            <div className="images-bar-note">
              Loading them tells the sender you opened this, and roughly where
              you are.
            </div>
          </div>
          <button className="btn-quiet" onClick={() => setLoadImages(true)}>
            Load images
          </button>
        </div>
      )}

      <div className="body-frame">
        {loading && !body ? (
          <div className="empty">Loading…</div>
        ) : body ? (
          // Scripts, forms and same-origin access stay denied; the CSP in the
          // document blocks remote images until asked, so tracking pixels never
          // fire on their own. Both fences are deliberate — neither alone is
          // enough.
          //
          // allow-popups is the single exception, and it grants less than it
          // sounds like. Without it the sandbox swallows link clicks before the
          // app ever hears about them, which is why links did nothing at all.
          // With it the click surfaces as a new-window request that Rust denies
          // outright and redirects to the system browser. No window is ever
          // created, and scripting is still off.
          <iframe
            title="Message body"
            sandbox="allow-popups"
            srcDoc={sandboxDocument(body, { loadRemoteImages: loadImages })}
            referrerPolicy="no-referrer"
          />
        ) : (
          <div className="empty">No content</div>
        )}
      </div>
    </>
  );
}
