/** One address book card, as held locally. */
export interface Contact {
  url: string;
  uid: string;
  etag: string | null;
  /** The card exactly as the server sent it. Editing rewrites these bytes in
   *  place rather than regenerating them, so fields this app does not model
   *  survive a change made here. */
  raw: string;
  displayName: string;
  search: string;
  detailsJson: string;
  /** null, 'created', 'modified' or 'deleted' — not yet accepted by the server. */
  pending: string | null;
}

export interface ContactDetail {
  value: string;
  label: string;
  preferred?: boolean;
}

export interface ContactDetails {
  emails: ContactDetail[];
  phones: ContactDetail[];
  organisation: string | null;
}

export interface ContactSync {
  accountId: string;
  stored: number;
  error: string | null;
}

/** Reads the details a contact carries, tolerating anything unexpected.
 *
 * The field is JSON built by the engine, so this should not fail — but a
 * contact list that throws on one malformed row shows nothing at all, which
 * is a poor trade for strictness about a field used only for display. */
export function contactDetails(contact: Contact): ContactDetails {
  try {
    const parsed = JSON.parse(contact.detailsJson || '{}');
    return {
      emails: Array.isArray(parsed.emails) ? parsed.emails : [],
      phones: Array.isArray(parsed.phones) ? parsed.phones : [],
      organisation: parsed.organisation ?? null,
    };
  } catch {
    return { emails: [], phones: [], organisation: null };
  }
}

// Mirrors the serde shapes in bazmail-core. Kept hand-written rather than
// generated: the surface is small, and a codegen step for six types would cost
// more than it saves.

export interface Account {
  id: string;
  label: string;
  color: string;
  identity: string;
  /**
   * How the account authenticates, which decides what reconnecting costs. An
   * OAuth grant is renewed in the browser with one click; IMAP needs its app
   * password typed again, because it was never kept anywhere readable.
   */
  connection: "oauth" | "imap" | "token";
  /** Appended to new messages from this account. */
  signature: string | null;
}

export interface Mailbox {
  id: string;
  accountId: string;
  name: string;
  role: string | null;
  totalEmails: number;
  unreadEmails: number;
  sortOrder: number;
}

export interface EmailAddress {
  name: string | null;
  email: string;
}

export interface Envelope {
  id: string;
  accountId: string;
  threadId: string;
  mailboxIds: string[];
  from: EmailAddress[];
  to: EmailAddress[];
  subject: string;
  preview: string;
  receivedAt: string;
  isUnread: boolean;
  isFlagged: boolean;
  hasAttachment: boolean;
  /**
   * The domain this message is proven to come from, when the provider's DMARC
   * check passed. Null covers both "failed" and "no verdict" — neither can
   * support offering to trust a sender.
   */
  verifiedDomain: string | null;
  /** RFC 5322 Message-ID. Carried so a reply threads rather than starting a
   *  new conversation. */
  messageId: string | null;
  references: string[];
}

export interface EmailBody {
  id: string;
  html: string | null;
  text: string | null;
}

export interface SyncOutcome {
  accountId: string;
  ok: boolean;
  envelopes: number;
  mailboxes: number;
  error: string | null;
}

/** What a token turned out to belong to, returned by verify before saving. */
export interface VerifiedAccount {
  username: string;
  accountName: string;
  suggestedId: string;
  suggestedLabel: string;
}

/** Result of a move: whether it reached the server or is still queued. */
export interface ArchiveOutcome {
  mutationId: number;
  accountId: string;
  emailId: string;
  synced: number;
  queued: number;
}

export interface Status {
  configured: boolean;
  configPath: string;
  accounts: Account[];
  storePath: string;
  storeProtection: "encrypted" | "refused" | "volume";
  version: string;
  queuedMutations: number;
}

/** How an account authenticates, worked out from its shape. */
export type AccountMethod = "OAuth" | "IMAP" | "API token";

/**
 * Decodes HTML entities that senders put in plain-text headers.
 *
 * A display name is not HTML, but plenty of senders emit one anyway —
 * "Claudia&#039;s Cucina" is a real example — and showing the raw entity looks
 * broken. Decoded by hand rather than through the DOM: routing untrusted header
 * text through innerHTML to unescape it would be an XSS foothold for the sake of
 * an apostrophe.
 */
export function decodeEntities(value: string): string {
  if (!value.includes("&")) return value;

  const named: Record<string, string> = {
    amp: "&",
    lt: "<",
    gt: ">",
    quot: '"',
    apos: "'",
    nbsp: " ",
  };

  return value.replace(/&(#x[0-9a-f]+|#[0-9]+|[a-z]+);/gi, (match, body: string) => {
    const token = body.toLowerCase();
    if (token.startsWith("#x")) {
      const code = parseInt(token.slice(2), 16);
      return Number.isFinite(code) ? String.fromCodePoint(code) : match;
    }
    if (token.startsWith("#")) {
      const code = parseInt(token.slice(1), 10);
      return Number.isFinite(code) ? String.fromCodePoint(code) : match;
    }
    return named[token] ?? match;
  });
}

/** Display name for a list row: a real name if we have one, else the local part. */
export function displayName(addresses: EmailAddress[]): string {
  const first = addresses[0];
  if (!first) return "Unknown sender";
  if (first.name && first.name.trim()) return decodeEntities(first.name);
  return first.email.split("@")[0] || first.email;
}

export function initials(addresses: EmailAddress[]): string {
  const name = displayName(addresses);
  const parts = name.split(/[\s.]+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

/** Time for today, "Yesterday", weekday within the week, date beyond that. */
export function formatWhen(iso: string): string {
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return "";

  const now = new Date();
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const days = Math.floor((startOfToday.getTime() - then.getTime()) / 86_400_000);

  if (then >= startOfToday) {
    return then.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  }
  if (days < 1) return "Yesterday";
  if (days < 6) return then.toLocaleDateString(undefined, { weekday: "short" });
  return then.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}
