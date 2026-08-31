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
  mailboxes: (accountId: string) => invoke<Mailbox[]>("mailboxes", { accountId }),
  body: (accountId: string, emailId: string) =>
    invoke<EmailBody>("body", { accountId, emailId }),

  markRead: (accountId: string, emailId: string, read: boolean) =>
    invoke<void>("mark_read", { accountId, emailId, read }),

  archive: (accountId: string, emailId: string) =>
    invoke<ArchiveOutcome>("archive", { accountId, emailId }),
  unarchive: (accountId: string, emailId: string) =>
    invoke<ArchiveOutcome>("unarchive", { accountId, emailId }),
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

export function sandboxDocument(body: EmailBody): string {
  const csp =
    "default-src 'none'; style-src 'unsafe-inline'; img-src data:; media-src 'none'; frame-src 'none'";

  const content = body.html
    ? body.html
    : `<pre style="white-space:pre-wrap;font:14px/1.68 -apple-system,'Segoe UI Variable Text','Segoe UI',system-ui,sans-serif">${escapeHtml(
        body.text ?? "",
      )}</pre>`;

  return `<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
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
