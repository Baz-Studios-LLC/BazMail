import { invoke } from "@tauri-apps/api/core";
import type {
  Account,
  ArchiveOutcome,
  EmailBody,
  Envelope,
  Mailbox,
  Status,
  SyncOutcome,
  VerifiedAccount,
} from "./types";

export const api = {
  status: () => invoke<Status>("status"),
  syncAll: (limit = 100) => invoke<SyncOutcome[]>("sync_all", { limit }),
  unifiedInbox: (limit = 100) => invoke<Envelope[]>("unified_inbox", { limit }),

  /**
   * One mailbox out of the mirror. Not the same as filtering the unified
   * inbox — that narrows to inbox-role mailboxes, so filtering it by Spam or
   * Archive returns nothing however much has been synced.
   */
  mailboxEnvelopes: (mailboxId: string, limit = 200) =>
    invoke<Envelope[]>("mailbox_envelopes", { mailboxId, limit }),

  /**
   * Fetches one mailbox from the server. The periodic sync only pulls inboxes,
   * so every other folder is fetched the first time it is opened.
   */
  syncMailbox: (accountId: string, mailboxId: string, limit = 200) =>
    invoke<number>("sync_mailbox", { accountId, mailboxId, limit }),
  mailboxes: (accountId: string) => invoke<Mailbox[]>("mailboxes", { accountId }),
  body: (accountId: string, emailId: string) =>
    invoke<EmailBody>("body", { accountId, emailId }),

  /**
   * Sends a message. Resolves only once the server has accepted it — sending
   * is the one mutation that is not queued, because a retry that duplicates
   * mail is worse than one that fails where you can see it.
   */
  send: (message: {
    accountId: string;
    to: { name: string | null; email: string }[];
    cc: { name: string | null; email: string }[];
    bcc: { name: string | null; email: string }[];
    subject: string;
    text: string;
    inReplyTo: string | null;
    references: string[];
  }) => invoke<void>("send", { message }),

  markRead: (accountId: string, emailId: string, read: boolean) =>
    invoke<void>("mark_read", { accountId, emailId, read }),

  archive: (accountId: string, emailId: string) =>
    invoke<ArchiveOutcome>("archive", { accountId, emailId }),
  unarchive: (accountId: string, emailId: string) =>
    invoke<ArchiveOutcome>("unarchive", { accountId, emailId }),

  /** Moves a message to Trash, out of every mailbox it is currently in. */
  trash: (accountId: string, emailId: string) =>
    invoke<ArchiveOutcome>("trash", { accountId, emailId }),

  setFlagged: (accountId: string, emailId: string, flagged: boolean) =>
    invoke<void>("set_flagged", { accountId, emailId, flagged }),

  setAccountSignature: (accountId: string, signature: string) =>
    invoke<void>("set_account_signature", { accountId, signature }),
  flushOutbox: () => invoke<number>("flush_outbox"),
  /**
   * The whole OAuth sign-in. Opens the system browser and resolves once the
   * user has approved and the redirect has come back — or rejects on timeout.
   */
  connectFastmail: (color: string) => invoke<Account>("connect_fastmail", { color }),

  /**
   * Connects an IMAP account. Verified by logging in and listing mailboxes
   * before anything is written, so a saved account always works.
   */
  connectImap: (account: {
    host: string;
    port: number;
    username: string;
    password: string;
    label: string;
    color: string;
  }) => invoke<Account>("connect_imap", account),

  /** Checks a token against JMAP without saving anything. */
  verifyToken: (sessionUrl: string, token: string) =>
    invoke<VerifiedAccount>("verify_token", { sessionUrl, token }),

  /** Verifies again, then stores the token in the OS credential manager. */
  saveAccount: (account: {
    id: string;
    label: string;
    color: string;
    identity: string;
    sessionUrl: string;
    token: string;
  }) => invoke<void>("save_account", account),

  deleteAccount: (accountId: string) =>
    invoke<void>("delete_account", { accountId }),

  setAccountColor: (accountId: string, color: string) =>
    invoke<void>("set_account_color", { accountId, color }),

  moveAccount: (accountId: string, up: boolean) =>
    invoke<void>("move_account", { accountId, up }),
};

/** Fastmail's JMAP discovery endpoint. */
export const FASTMAIL_SESSION_URL = "https://api.fastmail.com/jmap/session";

/** Provenance colours, handed out in order as accounts are added. */
export const ACCOUNT_COLORS = [
  "#e0a94a",
  "#57c07d",
  "#5aa9f0",
  "#a98ae0",
  "#4fc4c0",
];

/**
 * The hosts a message would reach out to if its images were allowed.
 *
 * Advisory only. The CSP is what actually blocks anything, and it fails closed:
 * a host this scan misses stays blocked rather than slipping through. That
 * separation is deliberate, because scanning a stranger's HTML with a pattern
 * is approximate by nature and should never be load-bearing.
 *
 * Naming the hosts is the honest version of "images are blocked". Seeing
 * `track.klaviyo.com` in the list tells you something that a generic warning
 * never could.
 */
export function remoteImageHosts(body: EmailBody): string[] {
  if (!body.html) return [];

  const hosts = new Set<string>();
  // src="…" covers <img>; url(…) covers CSS backgrounds, which the CSP governs
  // under img-src too.
  const pattern = /src\s*=\s*["']([^"']+)["']|url\(\s*["']?([^"')]+)["']?\s*\)/gi;

  for (const match of body.html.matchAll(pattern)) {
    const raw = (match[1] ?? match[2] ?? "").trim();
    if (!/^https?:\/\//i.test(raw)) continue;
    try {
      hosts.add(new URL(raw).hostname);
    } catch {
      // A malformed URL is not worth reporting; the CSP blocks it regardless.
    }
  }

  return [...hosts].sort();
}

export function sandboxDocument(
  body: EmailBody,
  { loadRemoteImages = false }: { loadRemoteImages?: boolean } = {},
): string {
  // Only img-src ever moves. default-src 'none' keeps remote fonts, stylesheets
  // and connections blocked whatever the user decides about pictures, and
  // scripts have no source at all in either case.
  const imgSrc = loadRemoteImages ? "data: https:" : "data:";
  const csp =
    `default-src 'none'; style-src 'unsafe-inline'; img-src ${imgSrc}; media-src 'none'; frame-src 'none'`;

  const content = body.html
    ? body.html
    : `<pre style="white-space:pre-wrap;font:14px/1.68 -apple-system,'Segoe UI Variable Text','Segoe UI',system-ui,sans-serif">${escapeHtml(
        body.text ?? "",
      )}</pre>`;

  // A target-only <base> makes every link in the message open out of the frame
  // without rewriting a single anchor. Parsing a stranger's HTML to add
  // attributes is exactly the kind of thing that grows an injection bug; one
  // declarative line in our own head cannot. It carries no href, so relative
  // URLs still resolve as they did.
  return `<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<base target="_blank">
<style>
  :root { color-scheme: dark; }
  body {
    margin: 0; padding: 0;
    background: transparent;
    color: #c9cfd6;
    font: 14px/1.68 -apple-system, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif;
  }
  a { color: #5aa9f0; }
  img { max-width: 100%; height: auto; }
  table { max-width: 100%; }
  blockquote {
    margin: 12px 0; padding-left: 12px;
    border-left: 2px solid rgba(255,255,255,0.12); color: #8b939c;
  }
</style></head><body>${content}</body></html>`;
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}
