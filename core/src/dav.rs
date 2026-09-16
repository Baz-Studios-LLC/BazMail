//! The WebDAV underneath contacts and calendars.
//!
//! CardDAV and CalDAV are the same protocol with different nouns: find the
//! user's principal, ask it where its collections live, list them, then fetch
//! their contents. Only the namespace and the name of the home-set differ. They
//! are written once here so the two cannot drift into disagreeing about how
//! discovery works — the part that is fiddly and identical.
//!
//! Discovery is three requests and each exists for a reason:
//!
//!   1. `current-user-principal` on the well-known path. The account's own URL
//!      is not derivable from its address; only the server knows it.
//!   2. the home-set on that principal, because collections do not have to live
//!      under the principal and on iCloud they do not.
//!   3. the collections themselves, filtered by resource type rather than by
//!      name — a calendar called "Contacts" is a calendar.
//!
//! Verified against both providers before any of it was written: iCloud answers
//! PROPFIND on `contacts.icloud.com` and `caldav.icloud.com` with 401, and
//! Fastmail redirects its well-known paths to `/dav/addressbooks` and
//! `/dav/calendars`.

use anyhow::{anyhow, Context, Result};
use reqwest::{Method, Url};

const DAV: &str = "DAV:";
const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";
const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
/// Apple's extension, and the only way to tell a collection changed without
/// asking for all of it.
const CALSERVER: &str = "http://calendarserver.org/ns/";

/// Which kind of collection we are looking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Contacts,
    Calendars,
}

impl Kind {
    fn namespace(self) -> &'static str {
        match self {
            Kind::Contacts => CARDDAV,
            Kind::Calendars => CALDAV,
        }
    }

    fn home_set(self) -> &'static str {
        match self {
            Kind::Contacts => "addressbook-home-set",
            Kind::Calendars => "calendar-home-set",
        }
    }

    /// The resource type that marks a collection as one of ours.
    fn resource_type(self) -> &'static str {
        match self {
            Kind::Contacts => "addressbook",
            Kind::Calendars => "calendar",
        }
    }

    fn well_known(self) -> &'static str {
        match self {
            Kind::Contacts => "/.well-known/carddav",
            Kind::Calendars => "/.well-known/caldav",
        }
    }
}

/// One address book or calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    /// Absolute URL. Relative hrefs are resolved on the way out, because a
    /// caller holding a path with no host cannot do anything with it.
    pub url: String,
    pub name: String,
    /// Changes whenever anything inside does. Held so a sync can ask "is there
    /// anything new" without downloading the collection to find out.
    pub tag: Option<String>,
}

pub struct DavClient {
    http: reqwest::Client,
    base: Url,
    username: String,
    password: String,
}

impl DavClient {
    pub fn new(http: reqwest::Client, base: &str, username: &str, password: &str) -> Result<Self> {
        Ok(Self {
            http,
            base: Url::parse(base).with_context(|| format!("{base} is not a URL"))?,
            username: username.to_string(),
            password: password.to_string(),
        })
    }

    /// One PROPFIND.
    ///
    /// `Depth` is explicit at every call site: the difference between asking
    /// about a collection and asking about everything in it is one header, and
    /// getting it wrong either returns nothing useful or fetches an entire
    /// address book by accident.
    async fn propfind(&self, url: &Url, depth: &str, body: &str) -> Result<String> {
        self.request(Method::from_bytes(b"PROPFIND")?, url, depth, body)
            .await
    }

    pub async fn report(&self, url: &str, depth: &str, body: &str) -> Result<String> {
        let url = self.base.join(url).with_context(|| format!("joining {url}"))?;
        self.request(Method::from_bytes(b"REPORT")?, &url, depth, body)
            .await
    }

    async fn request(&self, method: Method, url: &Url, depth: &str, body: &str) -> Result<String> {
        let response = self
            .http
            .request(method, url.clone())
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body.to_string())
            .send()
            .await
            .with_context(|| format!("talking to {url}"))?;

        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            // Worth its own message: for iCloud this is almost always an Apple
            // ID password where an app-specific one is needed, and the generic
            // "401" sends people to check the username instead.
            return Err(anyhow!(
                "{url} rejected the credentials — for iCloud this must be an app-specific password"
            ));
        }
        if !status.is_success() && status != reqwest::StatusCode::MULTI_STATUS {
            return Err(anyhow!("{url} answered {status}"));
        }
        Ok(text)
    }

    /// Finds the address books or calendars on this account.
    pub async fn discover(&self, kind: Kind) -> Result<Vec<Collection>> {
        let principal = self.principal(kind).await?;
        let home = self.home_set(&principal, kind).await?;
        self.collections(&home, kind).await
    }

    /// Finds the account's own URL.
    ///
    /// The well-known path is asked first because that is what it is for, and
    /// the root second because iCloud does not answer it: it returns an empty
    /// current-user-principal in a propstat marked 404, which is a polite way
    /// of saying the property is not here. Its root carries it instead.
    async fn principal(&self, kind: Kind) -> Result<Url> {
        let body = r#"<d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;

        let mut last: Option<anyhow::Error> = None;
        for path in [kind.well_known(), "/"] {
            let url = match self.base.join(path) {
                Ok(url) => url,
                Err(_) => continue,
            };
            match self.propfind(&url, "0", body).await {
                Ok(xml) => match first_href(&xml, DAV, "current-user-principal") {
                    Some(href) => return self.absolute(&href),
                    None => continue,
                },
                Err(e) => last = Some(e),
            }
        }

        Err(last.unwrap_or_else(|| {
            anyhow!("the server did not say where this account keeps its principal")
        }))
    }
    async fn home_set(&self, principal: &Url, kind: Kind) -> Result<Url> {
        let body = format!(
            r#"<d:propfind xmlns:d="DAV:" xmlns:x="{ns}"><d:prop><x:{home}/></d:prop></d:propfind>"#,
            ns = kind.namespace(),
            home = kind.home_set(),
        );
        let xml = self.propfind(principal, "0", &body).await?;

        let href = first_href(&xml, kind.namespace(), kind.home_set()).ok_or_else(|| {
            anyhow!(
                "the account has no {} — the provider may not offer them",
                kind.home_set()
            )
        })?;
        self.absolute(&href)
    }

    async fn collections(&self, home: &Url, kind: Kind) -> Result<Vec<Collection>> {
        // Depth 1: the collections inside the home, not the home itself and not
        // every card in every one of them.
        let body = format!(
            r#"<d:propfind xmlns:d="DAV:" xmlns:cs="{cs}"><d:prop><d:resourcetype/><d:displayname/><cs:getctag/><d:sync-token/></d:prop></d:propfind>"#,
            cs = CALSERVER,
        );
        let xml = self.propfind(home, "1", &body).await?;
        let doc = roxmltree::Document::parse(&xml).context("reading the collection list")?;

        let mut found = Vec::new();
        for response in doc
            .descendants()
            .filter(|n| n.has_tag_name((DAV, "response")))
        {
            // By resource type, never by name: a calendar called "Contacts" is
            // still a calendar, and a server is free to name things anything.
            let is_ours = response
                .descendants()
                .any(|n| n.has_tag_name((kind.namespace(), kind.resource_type())));
            if !is_ours {
                continue;
            }

            let Some(href) = child_text(response, DAV, "href") else {
                continue;
            };
            let url = self.absolute(&href)?;
            let name = response
                .descendants()
                .find(|n| n.has_tag_name((DAV, "displayname")))
                .and_then(|n| n.text())
                .map(str::to_string)
                .filter(|s| !s.trim().is_empty())
                // A collection with no display name still has to be called
                // something, and its last path segment is what the provider's
                // own interface usually shows.
                .unwrap_or_else(|| last_segment(&url));

            let tag = response
                .descendants()
                .find(|n| {
                    n.has_tag_name((CALSERVER, "getctag")) || n.has_tag_name((DAV, "sync-token"))
                })
                .and_then(|n| n.text())
                .map(str::to_string);

            found.push(Collection {
                url: url.to_string(),
                name,
                tag,
            });
        }
        Ok(found)
    }

    /// Resolves an href that may be a path, against this account's host.
    fn absolute(&self, href: &str) -> Result<Url> {
        self.base
            .join(href.trim())
            .with_context(|| format!("resolving {href}"))
    }
}

/// Whether a propstat reports success.
///
/// A multistatus answers each property separately, and a server may return
/// the element you asked for inside a propstat that says 404. Searching the
/// whole response ignores that and treats an absent property as present —
/// which is how iCloud's well-known path looked like a parse failure rather
/// than a clear 'not here'.
fn propstat_succeeded(propstat: roxmltree::Node<'_, '_>) -> bool {
    let Some(status) = propstat
        .children()
        .find(|n| n.has_tag_name((DAV, "status")))
        .and_then(|n| n.text())
    else {
        // No status at all: take it at face value rather than discarding a
        // property a lenient server did include.
        return true;
    };
    status
        .split_whitespace()
        .find_map(|word| word.parse::<u16>().ok())
        .is_some_and(|code| (200..300).contains(&code))
}

/// The href inside the named property, ignoring properties the server said
/// it does not have.
fn first_href(xml: &str, namespace: &str, property: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    doc.descendants()
        .filter(|n| n.has_tag_name((DAV, "propstat")))
        .filter(|propstat| propstat_succeeded(*propstat))
        .flat_map(|propstat| propstat.descendants())
        .find(|n| n.has_tag_name((namespace, property)))
        .and_then(|found| found.descendants().find(|n| n.has_tag_name((DAV, "href"))))
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}
/// A direct child's text, rather than any descendant's.
///
/// Matters for `href`: a response carries its own, and so does every property
/// inside it, so a descendant search finds whichever came first.
fn child_text<'a>(
    node: roxmltree::Node<'a, 'a>,
    namespace: &str,
    name: &str,
) -> Option<String> {
    node.children()
        .find(|n| n.has_tag_name((namespace, name)))
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
}

fn last_segment(url: &Url) -> String {
    url.path_segments()
        .and_then(|s| s.filter(|p| !p.is_empty()).next_back())
        .unwrap_or("Untitled")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like iCloud's: namespace prefixes that are not the obvious ones,
    /// and an href on the response as well as inside the property.
    const PRINCIPAL: &str = r#"<?xml version="1.0"?>
<multistatus xmlns="DAV:">
  <response>
    <href>/1234567/principal/</href>
    <propstat>
      <prop><current-user-principal><href>/1234567/principal/</href></current-user-principal></prop>
      <status>HTTP/1.1 200 OK</status>
    </propstat>
  </response>
</multistatus>"#;

    const COLLECTIONS: &str = r#"<?xml version="1.0"?>
<multistatus xmlns="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav" xmlns:cs="http://calendarserver.org/ns/">
  <response>
    <href>/1234567/carddavhome/</href>
    <propstat><prop><resourcetype><collection/></resourcetype></prop></propstat>
  </response>
  <response>
    <href>/1234567/carddavhome/card/</href>
    <propstat>
      <prop>
        <resourcetype><collection/><card:addressbook/></resourcetype>
        <displayname>Contacts</displayname>
        <cs:getctag>HK-1</cs:getctag>
      </prop>
    </propstat>
  </response>
</multistatus>"#;

    fn client() -> DavClient {
        DavClient::new(
            reqwest::Client::new(),
            "https://contacts.icloud.com",
            "someone@icloud.com",
            "app-specific",
        )
        .unwrap()
    }

    /// Discovery against the real iCloud server.
    ///
    /// Ignored, because it needs an account and reaches the network. It reads
    /// the app-specific password already in the OS credential store, so no
    /// secret is typed, passed on a command line, or printed here.
    ///
    /// Run with: cargo test --manifest-path core/Cargo.toml -- --ignored --nocapture
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "reaches iCloud with the stored credential"]
    async fn discovers_what_icloud_actually_has() {
        let password = crate::secrets::load_token("icloud")
            .expect("reading the credential store")
            .expect("no iCloud credential stored");

        for (label, host, kind) in [
            ("contacts", "https://contacts.icloud.com", Kind::Contacts),
            ("calendars", "https://caldav.icloud.com", Kind::Calendars),
        ] {
            let client = DavClient::new(
                reqwest::Client::new(),
                host,
                "brettbazaar@icloud.com",
                &password,
            )
            .expect("client");

            match client.discover(kind).await {
                Ok(found) => {
                    println!("{label}: {} collection(s)", found.len());
                    for c in &found {
                        println!("   {:<28} {}", c.name, c.url);
                    }
                    assert!(!found.is_empty(), "{label}: discovery found nothing");
                }
                Err(e) => panic!("{label}: {e:#}"),
            }
        }
    }

    #[test]
    fn finds_the_principal_href() {
        assert_eq!(
            first_href(PRINCIPAL, DAV, "current-user-principal").as_deref(),
            Some("/1234567/principal/")
        );
    }

    #[test]
    fn a_relative_href_is_resolved_against_the_host() {
        // Servers answer with paths. A caller handed one cannot fetch it.
        let url = client().absolute("/1234567/principal/").unwrap();
        assert_eq!(url.as_str(), "https://contacts.icloud.com/1234567/principal/");
    }

    #[test]
    fn only_real_address_books_are_returned() {
        // The home itself comes back in a Depth 1 listing and is a collection
        // without being an address book. Keeping it would have us fetch cards
        // from a container that holds none.
        let doc = roxmltree::Document::parse(COLLECTIONS).unwrap();
        let books: Vec<_> = doc
            .descendants()
            .filter(|n| n.has_tag_name((DAV, "response")))
            .filter(|r| {
                r.descendants()
                    .any(|n| n.has_tag_name((CARDDAV, "addressbook")))
            })
            .collect();
        assert_eq!(books.len(), 1, "the home is not an address book");
        assert_eq!(
            child_text(books[0], DAV, "href").as_deref(),
            Some("/1234567/carddavhome/card/")
        );
    }

    #[test]
    fn the_response_href_wins_over_one_nested_in_a_property() {
        // A response carries its own href and so can any property inside it.
        // Searching descendants finds whichever came first, which is how a
        // collection ends up pointed at a principal.
        let xml = r#"<multistatus xmlns="DAV:"><response>
            <href>/right/</href>
            <propstat><prop><owner><href>/wrong/</href></owner></prop></propstat>
        </response></multistatus>"#;
        let doc = roxmltree::Document::parse(xml).unwrap();
        let response = doc
            .descendants()
            .find(|n| n.has_tag_name((DAV, "response")))
            .unwrap();
        assert_eq!(child_text(response, DAV, "href").as_deref(), Some("/right/"));
    }

    #[test]
    fn a_nameless_collection_is_named_for_its_path() {
        let url = Url::parse("https://contacts.icloud.com/123/carddavhome/card/").unwrap();
        assert_eq!(last_segment(&url), "card");
    }

    #[test]
    fn contacts_and_calendars_differ_only_in_their_nouns() {
        // The whole reason this module is shared.
        assert_eq!(Kind::Contacts.home_set(), "addressbook-home-set");
        assert_eq!(Kind::Calendars.home_set(), "calendar-home-set");
        assert_eq!(Kind::Calendars.namespace(), CALDAV);
        assert_eq!(Kind::Calendars.well_known(), "/.well-known/caldav");
    }
}
