import { useMemo, useState } from "react";
import { Panel } from "./Panel";
import { contactDetails, initials } from "../types";
import type { Contact, ContactSync } from "../types";

interface ContactsProps {
  contacts: Contact[];
  /** Fetches from every account that can; each reports separately. */
  onSync: () => void;
  syncing: boolean;
  /** What the last sync said, per account. Empty before one has run. */
  outcomes: ContactSync[];
  onClose: () => void;
}

export function Contacts({
  contacts,
  onSync,
  syncing,
  outcomes,
  onClose,
}: ContactsProps) {
  const [query, setQuery] = useState("");
  const [selectedUrl, setSelectedUrl] = useState<string | null>(null);

  // The search line was built once on the way in, so filtering is a substring
  // test rather than a walk over every field of every card on every keystroke.
  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return contacts;
    return contacts.filter((c) => c.search.includes(needle));
  }, [contacts, query]);

  const selected =
    shown.find((c) => c.url === selectedUrl) ?? shown[0] ?? null;

  // Only worth mentioning the ones that failed: an account that synced fine
  // needs no report, and listing every account every time turns the useful
  // case into something to read past.
  const failures = outcomes.filter((o) => o.error);

  return (
    <Panel
      title="Contacts"
      onBack={onClose}
      backLabel="Back to mail"
      actions={
        <button className="btn-quiet" onClick={onSync} disabled={syncing}>
          {syncing ? "Syncing…" : "Sync"}
        </button>
      }
    >
      <div className="contacts">
        <div className="contacts-list">
          <input
            className="contacts-search"
            placeholder="Search contacts"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            spellCheck={false}
          />

          {shown.map((contact) => {
            const details = contactDetails(contact);
            const first = details.emails[0]?.value ?? details.phones[0]?.value ?? "";
            return (
              <button
                key={contact.url}
                className={`contact-row ${
                  selected?.url === contact.url ? "selected" : ""
                }`}
                onClick={() => setSelectedUrl(contact.url)}
              >
                <span className="avatar">{initials([{ name: contact.displayName, email: first }])}</span>
                <span className="contact-row-main">
                  <span className="contact-row-name">
                    {contact.displayName || "(no name)"}
                  </span>
                  <span className="contact-row-sub">{first}</span>
                </span>
              </button>
            );
          })}

          {contacts.length === 0 && (
            <div className="empty" style={{ padding: 24 }}>
              No contacts yet. Sync to fetch them.
            </div>
          )}
          {contacts.length > 0 && shown.length === 0 && (
            <div className="empty" style={{ padding: 24 }}>
              Nothing matches “{query}”.
            </div>
          )}
        </div>

        <div className="contacts-detail">
          {selected ? (
            <ContactCard contact={selected} />
          ) : (
            <div className="empty">Select a contact</div>
          )}
        </div>
      </div>

      {failures.length > 0 && (
        <div className="contacts-problems">
          {failures.map((outcome) => (
            <div key={outcome.accountId} className="contacts-problem">
              <strong>{outcome.accountId}</strong> — {outcome.error}
            </div>
          ))}
        </div>
      )}
    </Panel>
  );
}

function ContactCard({ contact }: { contact: Contact }) {
  const details = contactDetails(contact);
  // Preferred first: a card may carry several addresses and the card itself
  // says which one its owner uses.
  const emails = [...details.emails].sort(
    (a, b) => Number(!!b.preferred) - Number(!!a.preferred),
  );

  return (
    <div className="contact-card">
      <div className="contact-card-head">
        <div className="contact-card-name">{contact.displayName || "(no name)"}</div>
        {details.organisation && (
          <div className="contact-card-org">{details.organisation}</div>
        )}
      </div>

      {emails.length > 0 && (
        <div className="contact-group">
          <div className="contact-group-title">Email</div>
          {emails.map((email) => (
            <div className="contact-line" key={email.value}>
              <a href={`mailto:${email.value}`}>{email.value}</a>
              {email.label && <span className="contact-tag">{email.label}</span>}
            </div>
          ))}
        </div>
      )}

      {details.phones.length > 0 && (
        <div className="contact-group">
          <div className="contact-group-title">Phone</div>
          {details.phones.map((phone) => (
            <div className="contact-line" key={phone.value}>
              <span>{phone.value}</span>
              {phone.label && <span className="contact-tag">{phone.label}</span>}
            </div>
          ))}
        </div>
      )}

      {emails.length === 0 && details.phones.length === 0 && (
        <div className="empty" style={{ padding: 0 }}>
          This card has no address or number on it.
        </div>
      )}
    </div>
  );
}
