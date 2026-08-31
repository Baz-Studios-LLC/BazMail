/**
 * The providers BazMail knows about.
 *
 * `method` is not decoration — every one of these connects differently, and the
 * differences are big enough to be worth showing before someone picks. Fastmail
 * is JMAP with real OAuth. Gmail and Outlook are their own APIs, each behind a
 * verification process. iCloud has no OAuth for third-party mail at all and
 * needs an app-specific password over IMAP. Proton cannot be reached directly
 * from a mail client at all without their Bridge running locally.
 *
 * Showing that up front is more honest than a grid of logos that implies they
 * are interchangeable.
 */
export interface Provider {
  id: string;
  name: string;
  /** How this provider is reached. */
  method: string;
  available: boolean;
  /** What still stands in the way, shown when an unavailable one is picked. */
  note?: string;
  /** Set when the provider connects over IMAP, so the form can prefill. */
  imap?: { host: string; port: number };
}

export const PROVIDERS: Provider[] = [
  {
    id: "fastmail",
    name: "Fastmail",
    method: "JMAP · OAuth",
    available: true,
  },
  {
    id: "gmail",
    name: "Gmail / Google Workspace",
    method: "Gmail API · OAuth",
    available: false,
    note: "Needs the Gmail API adapter, plus Google OAuth verification and an annual security assessment before it can be used by anyone but the developer.",
  },
  {
    id: "outlook",
    name: "Outlook.com / Microsoft 365",
    method: "Microsoft Graph · OAuth",
    available: false,
    note: "Needs the Microsoft Graph adapter. App registration is considerably lighter than Google's.",
  },
  {
    id: "icloud",
    name: "iCloud",
    method: "IMAP · app password",
    available: true,
    imap: { host: "imap.mail.me.com", port: 993 },
  },
  {
    id: "yahoo",
    name: "Yahoo Mail",
    method: "IMAP · app password",
    available: false,
    note: "Needs the IMAP adapter and an app password from Yahoo account security.",
  },
  {
    id: "proton",
    name: "Proton Mail",
    method: "IMAP via Proton Bridge",
    available: false,
    note: "Proton is end-to-end encrypted, so no mail client can reach it directly. It requires their Bridge running locally, and server-side search cannot work because the server cannot read your mail.",
  },
  {
    id: "zoho",
    name: "Zoho Mail",
    method: "IMAP · OAuth",
    available: false,
    note: "Needs the IMAP adapter.",
  },
  {
    id: "aol",
    name: "AOL Mail",
    method: "IMAP · app password",
    available: false,
    note: "Needs the IMAP adapter and an app password.",
  },
  {
    id: "gmx",
    name: "GMX / Mail.com",
    method: "IMAP",
    available: false,
    note: "Needs the IMAP adapter. IMAP access must be enabled in GMX settings first.",
  },
  {
    id: "imap",
    name: "Other (IMAP / JMAP)",
    method: "Host, port, credentials",
    available: false,
    note: "Any other host, once the IMAP adapter exists. A server that speaks JMAP will work sooner, since that path is already built.",
  },
];
