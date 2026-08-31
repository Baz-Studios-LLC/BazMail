import type { Account, EmailBody, Envelope } from "../types";
import { decodeEntities, displayName, formatWhen } from "../types";
import { sandboxDocument } from "../api";
import { MailIcon } from "./Icons";

interface ReaderProps {
  envelope: Envelope | null;
  body: EmailBody | null;
  loading: boolean;
  error: string | null;
  accounts: Account[];
}

export function Reader({ envelope, body, loading, error, accounts }: ReaderProps) {
  if (!envelope) {
    return (
      <div className="empty">
        <MailIcon size={26} />
        <div>Select a message</div>
      </div>
    );
  }

  const account = accounts.find((a) => a.id === envelope.accountId);

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

      <div className="body-frame">
        {loading && !body ? (
          <div className="empty">Loading…</div>
        ) : body ? (
          // sandbox="" denies scripts, forms and same-origin access; the CSP in
          // the document itself blocks remote images so tracking pixels never
          // fire. Both fences are deliberate — neither alone is enough.
          <iframe
            title="Message body"
            sandbox=""
            srcDoc={sandboxDocument(body)}
            referrerPolicy="no-referrer"
          />
        ) : (
          <div className="empty">No content</div>
        )}
      </div>
    </>
  );
}
