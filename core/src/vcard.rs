//! Reading vCards.
//!
//! CardDAV servers hand back vCard, and the version depends on who you ask:
//! iCloud speaks 3.0, Fastmail will serve 4.0 if asked. The differences that
//! matter here are small — `TYPE=INTERNET` and a separate `PREF` parameter in
//! 3.0, `PREF=1` inside `TYPE` in 4.0 — so one reader handles both rather than
//! two readers disagreeing about which is which.
//!
//! Hand-written rather than taken from a crate, because the shape of the format
//! is small and its edges are where the bugs are: folded lines, escaped commas,
//! and Apple's habit of prefixing properties with a group (`item1.EMAIL`) so it
//! can hang a label off them. A parser that silently drops those returns a
//! contact with no address and looks like it worked.

use std::collections::HashMap;

// The line format itself is shared with iCalendar: same folding, same
// parameters, same escaping. Written once so the two cannot disagree.
use crate::contentline::{parse_property, split_escaped, unescape, unfold, Property};

/// One address book entry, reduced to what a mail client needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contact {
    /// The server's identifier for this card. Stable across edits, which is
    /// what makes an update an update rather than a second contact.
    pub uid: String,
    /// What to show. `FN` when present, otherwise assembled from `N`.
    pub name: String,
    pub emails: Vec<Detail>,
    pub phones: Vec<Detail>,
    pub organisation: Option<String>,
    /// Inline photo, as bytes and a media type. URLs are ignored for now: a
    /// remote fetch on behalf of a contact is a tracking beacon by another
    /// name, and deciding that deserves its own thought.
    pub photo: Option<Photo>,
    /// The card's revision, for noticing changes without comparing everything.
    pub revision: Option<String>,
}

/// An address or number, with whatever the card called it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    pub value: String,
    /// "home", "work", or a label the user typed. Lowercased; empty when the
    /// card said nothing.
    pub label: String,
    /// The one to use when only one can be used.
    pub preferred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Photo {
    pub media_type: String,
    pub data: Vec<u8>,
}

fn is_preferred(params: &HashMap<String, Vec<String>>) -> bool {
    // 3.0 says TYPE=PREF or a bare PREF; 4.0 says PREF=1.
    params
        .get("type")
        .is_some_and(|v| v.iter().any(|t| t.eq_ignore_ascii_case("pref")))
        || params.get("pref").is_some_and(|v| !v.is_empty())
}

/// The label a detail should carry.
///
/// INTERNET and other transport hints are not labels — they say how to reach an
/// address, not what it is for, and showing "internet" beside every address
/// tells the reader nothing.
fn label_of(params: &HashMap<String, Vec<String>>) -> String {
    params
        .get("type")
        .into_iter()
        .flatten()
        .map(|t| t.to_ascii_lowercase())
        .find(|t| !matches!(t.as_str(), "internet" | "pref" | "voice" | "other"))
        .unwrap_or_default()
}

/// Reads one vCard.
///
/// Returns `None` only when there is nothing usable — no identifier and no way
/// to address the person. A card with a name and no address is still worth
/// keeping; a card with neither is not a contact.
pub fn parse(raw: &str) -> Option<Contact> {
    let unfolded = unfold(raw);
    let mut contact = Contact::default();
    let mut structured_name: Option<String> = None;
    // Apple attaches a custom label through a group: item1.EMAIL + item1.X-ABLabel.
    let mut group_labels: HashMap<String, String> = HashMap::new();
    let mut pending: Vec<(Option<String>, bool, String, String)> = Vec::new();

    for line in unfolded.lines() {
        let Some(property) = parse_property(line) else {
            continue;
        };
        let value = unescape(property.value).trim().to_string();

        match property.name.as_str() {
            "uid" if !value.is_empty() => contact.uid = value,
            "fn" if !value.is_empty() => contact.name = value,
            "n" if structured_name.is_none() => {
                // Family;Given;Middle;Prefix;Suffix — shown given-first, which
                // is how a person is addressed rather than how they are filed.
                // Split before unescaping, or a name holding an escaped
                // semicolon splits in the wrong place.
                let parts: Vec<String> = split_escaped(property.value, ';')
                    .iter()
                    .map(|p| unescape(p))
                    .collect();
                let given = parts.get(1).cloned().unwrap_or_default();
                let family = parts.first().cloned().unwrap_or_default();
                let joined = format!("{} {}", given.trim(), family.trim());
                let joined = joined.trim().to_string();
                if !joined.is_empty() {
                    structured_name = Some(joined);
                }
            }
            "org" if contact.organisation.is_none() && !value.is_empty() => {
                // Org;Department — the first field is the organisation.
                let first = split_escaped(property.value, ';')
                    .first()
                    .map(|p| unescape(p))
                    .unwrap_or_default();
                let first = first.trim().to_string();
                if !first.is_empty() {
                    contact.organisation = Some(first);
                }
            }
            "rev" if !value.is_empty() => contact.revision = Some(value),
            "x-ablabel" => {
                if let Some(group) = property.group.clone() {
                    if !value.is_empty() {
                        group_labels.insert(group, value.to_ascii_lowercase());
                    }
                }
            }
            "email" | "tel" if !value.is_empty() => pending.push((
                property.group.clone(),
                is_preferred(&property.params),
                label_of(&property.params),
                format!("{}\u{0}{}", property.name, value),
            )),
            "photo" if contact.photo.is_none() => {
                contact.photo = read_photo(&property, property.value);
            }
            _ => {}
        }
    }

    // Applied after the whole card is read: a group's label may appear either
    // side of the property it belongs to.
    for (group, preferred, label, tagged) in pending {
        let (kind, value) = tagged.split_once('\u{0}')?;
        let label = group
            .and_then(|g| group_labels.get(&g).cloned())
            .unwrap_or(label);
        let detail = Detail {
            value: value.to_string(),
            label,
            preferred,
        };
        if kind == "email" {
            contact.emails.push(detail);
        } else {
            contact.phones.push(detail);
        }
    }

    if contact.name.is_empty() {
        contact.name = structured_name
            .or_else(|| contact.emails.first().map(|e| e.value.clone()))
            .unwrap_or_default();
    }

    // Nothing to show and nothing to reach: not a contact.
    if contact.uid.is_empty() && contact.name.is_empty() && contact.emails.is_empty() {
        return None;
    }
    Some(contact)
}

fn read_photo(property: &Property<'_>, raw: &str) -> Option<Photo> {
    use base64::Engine;

    let encoding = property
        .params
        .get("encoding")
        .and_then(|v| v.first())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    // 4.0 inlines a data: URI; 3.0 uses ENCODING=b with a TYPE parameter.
    let (media_type, payload) = if let Some(rest) = raw.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        (
            meta.split(';').next().unwrap_or("image/jpeg").to_string(),
            data.to_string(),
        )
    } else if encoding == "b" || encoding == "base64" {
        let kind = property
            .params
            .get("type")
            .and_then(|v| v.first())
            .map(|t| t.to_ascii_lowercase())
            .unwrap_or_else(|| "jpeg".into());
        (format!("image/{kind}"), raw.to_string())
    } else {
        // A URL. Deliberately not followed — see the note on Contact::photo.
        return None;
    };

    let cleaned: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    let data = base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .ok()?;
    (!data.is_empty()).then_some(Photo { media_type, data })
}

/// Splits a response body holding several cards.
pub fn parse_many(raw: &str) -> Vec<Contact> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in unfold(raw).lines() {
        if line.eq_ignore_ascii_case("BEGIN:VCARD") {
            current.clear();
        }
        current.push_str(line);
        current.push('\n');
        if line.eq_ignore_ascii_case("END:VCARD") {
            if let Some(contact) = parse(&current) {
                out.push(contact);
            }
            current.clear();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What iCloud actually returns: 3.0, group prefixes, a custom label.
    const APPLE: &str = "BEGIN:VCARD\r\n\
VERSION:3.0\r\n\
UID:ABCD-1234\r\n\
N:Chen;Sarah;;;\r\n\
FN:Sarah Chen\r\n\
ORG:Loop Studios;Design\r\n\
item1.EMAIL;type=INTERNET;type=pref:sarah@loopstudios.example\r\n\
item1.X-ABLabel:Studio\r\n\
EMAIL;type=INTERNET;type=HOME:sarah@home.example\r\n\
TEL;type=CELL:+1 555 0100\r\n\
REV:2026-09-01T10:00:00Z\r\n\
END:VCARD\r\n";

    #[test]
    fn reads_a_card_from_icloud() {
        let c = parse(APPLE).expect("should parse");
        assert_eq!(c.uid, "ABCD-1234");
        assert_eq!(c.name, "Sarah Chen");
        assert_eq!(c.organisation.as_deref(), Some("Loop Studios"));
        assert_eq!(c.emails.len(), 2);
        assert_eq!(c.emails[0].value, "sarah@loopstudios.example");
        assert!(c.emails[0].preferred, "type=pref marks the one to use");
        assert_eq!(c.phones[0].value, "+1 555 0100");
        assert_eq!(c.revision.as_deref(), Some("2026-09-01T10:00:00Z"));
    }

    #[test]
    fn a_group_label_reaches_the_address_it_belongs_to() {
        // Apple hangs the label off a group rather than the property. Ignoring
        // groups gives every address the type from its TYPE parameter and
        // throws away the name the user actually chose.
        let c = parse(APPLE).unwrap();
        assert_eq!(c.emails[0].label, "studio");
        assert_eq!(c.emails[1].label, "home", "falls back to TYPE");
    }

    #[test]
    fn transport_hints_are_not_labels() {
        // Every address on an Apple card says INTERNET. Showing that beside
        // each one tells the reader nothing.
        let c = parse(APPLE).unwrap();
        assert!(c.emails.iter().all(|e| e.label != "internet"));
    }

    #[test]
    fn a_folded_line_is_rejoined() {
        // A long value is broken anywhere, continued with one space. Parsing
        // without rejoining truncates the address and it still looks like an
        // address.
        let raw = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Folded\r\nEMAIL:someone.with.a.very\r\n .long.address@example.com\r\nEND:VCARD\r\n";
        let c = parse(raw).unwrap();
        assert_eq!(c.emails[0].value, "someone.with.a.very.long.address@example.com");
    }

    #[test]
    fn escapes_come_back_as_text() {
        let raw = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Smith\\, Jane\r\nORG:A\\; B\r\nEND:VCARD\r\n";
        let c = parse(raw).unwrap();
        assert_eq!(c.name, "Smith, Jane");
        assert_eq!(c.organisation.as_deref(), Some("A; B"));
    }

    #[test]
    fn a_card_without_fn_is_still_named() {
        // N is given-first when shown, because that is how someone is
        // addressed rather than how they are filed.
        let raw = "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Okafor;Ada;;;\r\nEMAIL:ada@example.com\r\nEND:VCARD\r\n";
        assert_eq!(parse(raw).unwrap().name, "Ada Okafor");
    }

    #[test]
    fn a_nameless_card_falls_back_to_its_address() {
        let raw = "BEGIN:VCARD\r\nVERSION:3.0\r\nEMAIL:ghost@example.com\r\nEND:VCARD\r\n";
        assert_eq!(parse(raw).unwrap().name, "ghost@example.com");
    }

    #[test]
    fn a_card_with_nothing_to_show_is_not_a_contact() {
        let raw = "BEGIN:VCARD\r\nVERSION:3.0\r\nNOTE:just a note\r\nEND:VCARD\r\n";
        assert!(parse(raw).is_none());
    }

    #[test]
    fn version_four_marks_preference_differently() {
        // 4.0 puts PREF in its own parameter. Reading only 3.0's spelling
        // would silently pick the wrong address to send to.
        let raw = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Ada\r\nEMAIL;PREF=1:first@example.com\r\nEMAIL:second@example.com\r\nEND:VCARD\r\n";
        let c = parse(raw).unwrap();
        assert!(c.emails[0].preferred);
        assert!(!c.emails[1].preferred);
    }

    #[test]
    fn several_cards_in_one_body() {
        let raw = format!("{APPLE}{APPLE}");
        assert_eq!(parse_many(&raw).len(), 2);
    }

    #[test]
    fn an_inline_photo_is_decoded() {
        use base64::Engine;
        let png = [0x89u8, 0x50, 0x4e, 0x47];
        let encoded = base64::engine::general_purpose::STANDARD.encode(png);
        let raw = format!(
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Pic\r\nPHOTO;ENCODING=b;TYPE=PNG:{encoded}\r\nEND:VCARD\r\n"
        );
        let photo = parse(&raw).unwrap().photo.expect("a photo");
        assert_eq!(photo.media_type, "image/png");
        assert_eq!(photo.data, png);
    }

    #[test]
    fn a_photo_url_is_not_fetched() {
        // Following it would load a remote resource on the contact's behalf,
        // which is a tracking beacon wearing a friendlier name.
        let raw = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Linked\r\nPHOTO:https://example.com/p.jpg\r\nEND:VCARD\r\n";
        assert!(parse(raw).unwrap().photo.is_none());
    }
}
