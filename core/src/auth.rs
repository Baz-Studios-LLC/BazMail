//! Reading the provider's authentication verdict for a message.
//!
//! We do not evaluate SPF, DKIM or DMARC ourselves. The receiving provider
//! already did, at the only point where it is possible — the moment the message
//! arrived, with the connecting IP in hand. By the time a client fetches a
//! message that information is gone, so re-deriving it here is not merely
//! wasteful, it is not possible.
//!
//! What we do instead is read the `Authentication-Results` header the provider
//! stamped, and this rests on one assumption worth stating plainly: **the
//! topmost header is ours.** A sender can put whatever they like in a message,
//! including a forged `Authentication-Results` claiming DMARC passed. Providers
//! are required to strip or rename such headers at ingress and add their own on
//! top. Every mail client makes this assumption; it is sound exactly as far as
//! the provider is trustworthy, which is already a precondition for reading
//! mail through them at all.
//!
//! The verdict matters because it decides whether "always allow images from
//! apple.com" is a sensible offer or a trap. On a DMARC-enforced domain nobody
//! else can put that domain in a From header, so the allowance means what the
//! user thinks it means. Without that check, the user's own allowlist becomes
//! the attack: spoof a domain they trusted, and their beacons load.

/// What the provider concluded about where a message came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authentication {
    /// DMARC passed, so the `From` domain is genuinely that domain.
    Verified { domain: String },
    /// The provider checked and the message did not pass.
    Failed,
    /// No usable verdict: an older message, a provider that stamps nothing, or
    /// a header we could not read.
    Unknown,
}

impl Authentication {
    /// The domain a message is proven to come from, if any.
    ///
    /// `Failed` and `Unknown` deliberately collapse to `None` here: the caller
    /// treats them identically, because in both cases there is nothing we can
    /// promise the user about who sent this.
    pub fn verified_domain(&self) -> Option<&str> {
        match self {
            Authentication::Verified { domain } => Some(domain),
            _ => None,
        }
    }
}

/// Parses the topmost `Authentication-Results` header.
///
/// Takes the header value only. Where several are present, pass the first —
/// see the module note on why the top one is the one that counts.
pub fn parse(header: &str) -> Authentication {
    let cleaned = strip_comments(header);

    // authserv-id ; method=result ; method=result ...
    let mut dmarc: Option<&str> = None;
    let mut from_domain: Option<String> = None;

    for part in cleaned.split(';').skip(1) {
        for token in part.split_whitespace() {
            let Some((key, value)) = token.split_once('=') else {
                continue;
            };
            match key.trim().to_ascii_lowercase().as_str() {
                "dmarc" => dmarc = Some(value.trim()),
                // header.from is what DMARC actually asserts about, which is
                // why we key on it rather than on the envelope sender: it is
                // the domain the reader sees.
                "header.from" => {
                    let domain = value.trim().trim_matches('"').trim_start_matches('@');
                    if !domain.is_empty() {
                        from_domain = Some(domain.to_ascii_lowercase());
                    }
                }
                _ => {}
            }
        }
    }

    match (dmarc, from_domain) {
        (Some(result), Some(domain)) if result.eq_ignore_ascii_case("pass") => {
            Authentication::Verified { domain }
        }
        // A pass with no domain to attach it to tells us nothing actionable.
        (Some(result), _) if result.eq_ignore_ascii_case("pass") => Authentication::Unknown,
        (Some(_), _) => Authentication::Failed,
        _ => Authentication::Unknown,
    }
}

/// Removes RFC 5322 comments, which may nest and routinely contain `;` and `=`.
///
/// `dmarc=pass (p=REJECT sp=REJECT dis=NONE) header.from=apple.com` would
/// otherwise parse `p=REJECT` as a method result.
fn strip_comments(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut depth = 0usize;

    for ch in value.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_dmarc_pass() {
        let got = parse("mx.example.net; dmarc=pass header.from=apple.com; spf=pass");
        assert_eq!(
            got,
            Authentication::Verified {
                domain: "apple.com".into()
            }
        );
    }

    #[test]
    fn policy_comments_do_not_become_results() {
        // The parenthesised policy carries p=REJECT, which is not a method.
        let got = parse("mx1.example.net; dmarc=pass (p=REJECT sp=REJECT dis=NONE) header.from=Apple.com");
        assert_eq!(
            got.verified_domain(),
            Some("apple.com"),
            "domain should be lowercased and the comment ignored"
        );
    }

    #[test]
    fn a_failure_is_not_a_verification() {
        let got = parse("mx.example.net; dmarc=fail header.from=paypa1.com");
        assert_eq!(got, Authentication::Failed);
        assert_eq!(got.verified_domain(), None);
    }

    #[test]
    fn spf_and_dkim_alone_prove_nothing_about_the_from_domain() {
        // Both can pass for a domain the reader never sees. Only DMARC ties the
        // result to the visible From, so this must not verify.
        let got = parse("mx.example.net; spf=pass smtp.mailfrom=bounces.sender.net; dkim=pass header.d=sender.net");
        assert_eq!(got, Authentication::Unknown);
    }

    #[test]
    fn an_absent_or_unreadable_header_is_unknown() {
        assert_eq!(parse(""), Authentication::Unknown);
        assert_eq!(parse("mx.example.net; none"), Authentication::Unknown);
    }

    #[test]
    fn a_pass_without_a_from_domain_is_not_actionable() {
        assert_eq!(parse("mx.example.net; dmarc=pass"), Authentication::Unknown);
    }
}
