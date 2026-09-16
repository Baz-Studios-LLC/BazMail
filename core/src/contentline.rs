//! The line format vCard and iCalendar share.
//!
//! RFC 6350 and RFC 5545 describe different things in the same syntax: lines of
//! `group.NAME;PARAM=value:content`, folded anywhere by a following space, with
//! commas and semicolons escaped inside values. Reading that is where the
//! awkward parts of both formats live, so it lives here once rather than twice.
//!
//! Two of the rules read the wrong way round and are worth naming, because
//! getting either backwards produces output that looks correct:
//!
//!   - Unfold before anything else. A long address or a base64 photo may be
//!     broken across lines at any point, and a parser that reads them line by
//!     line silently truncates.
//!   - Split on unescaped separators *before* unescaping. Unescaping first
//!     turns `\;` into a real semicolon, and the structural split then cuts the
//!     value there — which is how `ORG:A\; B` becomes `A`.

use std::collections::HashMap;

/// One property line, after unfolding.
pub struct Property<'a> {
    /// Lowercased, with any group prefix removed.
    pub name: String,
    /// Lowercased keys; values keep their case because labels are text.
    pub params: HashMap<String, Vec<String>>,
    /// The group this property belonged to, if any — Apple uses it to attach an
    /// `X-ABLabel` to an address, and without it the label belongs to nothing.
    pub group: Option<String>,
    /// Exactly as it appeared, still escaped.
    pub value: &'a str,
}

impl Property<'_> {
    /// The first value of a parameter, lowercased.
    pub fn param(&self, key: &str) -> Option<String> {
        self.params
            .get(key)
            .and_then(|v| v.first())
            .map(|v| v.to_ascii_lowercase())
    }

    /// Whether a parameter carries a value, compared without case.
    pub fn param_is(&self, key: &str, value: &str) -> bool {
        self.params
            .get(key)
            .is_some_and(|v| v.iter().any(|item| item.eq_ignore_ascii_case(value)))
    }
}

/// Rejoins continuation lines.
///
/// A line may be broken anywhere by starting the next one with a space or tab.
/// Parsing without rejoining splits a base64 photo into nonsense and truncates
/// any value long enough to wrap.
pub fn unfold(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for line in raw.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        match line.strip_prefix([' ', '\t']) {
            Some(rest) if !out.is_empty() => out.push_str(rest),
            _ => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(line);
            }
        }
    }
    out
}

/// Splits `group.NAME;PARAM=a,b;FLAG:value`.
pub fn parse_property(line: &str) -> Option<Property<'_>> {
    // The first unquoted colon ends the name and parameters. Quoted, because a
    // parameter value may legitimately contain one — a TZID often does.
    let mut in_quotes = false;
    let colon = line.char_indices().find(|(_, c)| match c {
        '"' => {
            in_quotes = !in_quotes;
            false
        }
        ':' => !in_quotes,
        _ => false,
    })?;
    let (head, value) = line.split_at(colon.0);
    let value = &value[1..];

    let mut parts = split_unquoted(head, ';');
    let name_part = parts.next()?;

    // A group prefix: item1.EMAIL. Only the last dot separates, because a group
    // name may itself contain one.
    let (group, name) = match name_part.rsplit_once('.') {
        Some((g, n)) => (Some(g.to_ascii_lowercase()), n),
        None => (None, name_part.as_str()),
    };
    if name.trim().is_empty() {
        return None;
    }

    let mut params: HashMap<String, Vec<String>> = HashMap::new();
    for part in parts {
        // `TYPE=WORK` and a bare `PREF` both occur; vCard 3.0 allows the
        // shorthand and real cards use it.
        let (key, raw) = match part.split_once('=') {
            Some((k, v)) => (k.to_ascii_lowercase(), v.to_string()),
            None => ("type".to_string(), part.clone()),
        };
        for one in split_unquoted(&raw, ',') {
            let cleaned = one.trim_matches('"').trim().to_string();
            if !cleaned.is_empty() {
                params.entry(key.clone()).or_default().push(cleaned);
            }
        }
    }

    Some(Property {
        name: name.trim().to_ascii_lowercase(),
        params,
        group,
        value,
    })
}

/// Splits on a separator that is not inside double quotes.
pub fn split_unquoted(input: &str, sep: char) -> std::vec::IntoIter<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in input.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            _ if c == sep && !in_quotes => out.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    out.push(current);
    out.into_iter()
}

/// Splits a structured value on separators that are not escaped.
///
/// Order matters and is easy to get backwards: unescaping first turns an
/// escaped semicolon into a real one, and the split then treats it as
/// structure.
pub fn split_escaped(value: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            current.push(c);
            if let Some(escaped) = chars.next() {
                current.push(escaped);
            }
            continue;
        }
        if c == sep {
            out.push(std::mem::take(&mut current));
            continue;
        }
        current.push(c);
    }
    out.push(current);
    out
}

/// Undoes the escaping a value carries: `\n`, `\,`, `\;`, `\\`.
pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push('\n'),
            // A trailing backslash is malformed; keeping the character after it
            // is friendlier than dropping it.
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folded_line_is_rejoined() {
        let raw = "SUMMARY:a very long\r\n  summary\r\n";
        assert_eq!(unfold(raw).trim(), "SUMMARY:a very long summary");
    }

    #[test]
    fn a_colon_inside_a_quoted_parameter_does_not_end_the_name() {
        // Real calendars carry TZID values with colons in them. Ending the
        // property at the first colon puts half the parameter into the value.
        let p = parse_property(r#"DTSTART;TZID="Europe/London:x":20260101T090000"#).unwrap();
        assert_eq!(p.name, "dtstart");
        assert_eq!(p.value, "20260101T090000");
    }

    #[test]
    fn a_group_prefix_is_separated_from_the_name() {
        let p = parse_property("item1.EMAIL;type=pref:a@b.example").unwrap();
        assert_eq!(p.name, "email");
        assert_eq!(p.group.as_deref(), Some("item1"));
        assert!(p.param_is("type", "PREF"), "compared without case");
    }

    #[test]
    fn splitting_happens_before_unescaping() {
        // The bug this ordering exists to prevent: unescape first and the
        // escaped semicolon becomes structure, cutting the value in half.
        let parts = split_escaped(r"A\; B;Second", ';');
        assert_eq!(parts.len(), 2);
        assert_eq!(unescape(&parts[0]), "A; B");
        assert_eq!(unescape(&parts[1]), "Second");
    }
}
