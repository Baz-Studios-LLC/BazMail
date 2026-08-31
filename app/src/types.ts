// Mirrors the serde shapes in bazmail-core. Kept hand-written rather than
// generated: the surface is small, and a codegen step for six types would cost
// more than it saves.

export interface Account {
  id: string;
  label: string;
  color: string;
  identity: string;
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
