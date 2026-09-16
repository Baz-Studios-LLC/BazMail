//! Reading iCalendar events.
//!
//! Shares its line format with vCard — see `contentline` — so what is left here
//! is the part that is genuinely calendar-shaped: nested components, and times
//! that come in four different kinds.
//!
//! **Nesting is not optional to handle.** A VCALENDAR holds VTIMEZONE blocks
//! beside its events, and a VEVENT holds VALARM blocks inside it. Matching
//! BEGIN and END naively finds four components where there is one event, and
//! the alarm's own properties overwrite the event's.
//!
//! **Times are deliberately not resolved.** iCalendar writes them four ways: a
//! date for an all-day event, a UTC instant, a local time tied to a named zone,
//! and a local time tied to nothing. Turning `TZID=America/New_York` into an
//! instant needs the zone database, which this crate does not carry, and
//! guessing would put events an hour out twice a year — the kind of wrong that
//! looks right until a meeting is missed. What arrives is kept as it was
//! written, labelled with which kind it is, so a caller can decide knowingly.
//!
//! Recurrence is kept raw for the same reason: `RRULE` with its exceptions is a
//! small language, and expanding it badly produces a calendar that is wrong in
//! a way nobody can see.

use crate::contentline::{parse_property, unescape, unfold};

/// When something happens, as the calendar wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum When {
    /// A whole day, with no time on it: `VALUE=DATE`. Stored as `YYYYMMDD`.
    AllDay(String),
    /// An instant, already in UTC: the value ended in `Z`.
    Utc(String),
    /// A local time in a named zone. The zone is kept, not resolved.
    Zoned { local: String, tzid: String },
    /// A local time with no zone at all — "floating", meaning 09:00 wherever
    /// the reader happens to be.
    Floating(String),
}

impl When {
    /// A value that sorts correctly within one calendar.
    ///
    /// Not an instant, and not comparable across zones — it exists so a list
    /// can be put in order without pretending the zone question is answered.
    pub fn sort_key(&self) -> &str {
        match self {
            When::AllDay(v) | When::Utc(v) | When::Floating(v) => v,
            When::Zoned { local, .. } => local,
        }
    }

    pub fn is_all_day(&self) -> bool {
        matches!(self, When::AllDay(_))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Event {
    pub uid: String,
    pub summary: String,
    pub location: Option<String>,
    pub description: Option<String>,
    pub start: Option<When>,
    /// Absent when the event carried a DURATION instead, or nothing at all.
    pub end: Option<When>,
    /// `DURATION`, verbatim, when that is what the event used.
    pub duration: Option<String>,
    /// The recurrence rule as written. Kept rather than expanded.
    pub rrule: Option<String>,
    /// Set on the single occurrence that overrides a recurring series.
    pub recurrence_id: Option<String>,
    /// True when the event says it is cancelled; it still exists and still
    /// shows, because a cancelled meeting is information.
    pub cancelled: bool,
}

/// One component and the lines belonging to it.
struct Component<'a> {
    name: String,
    lines: Vec<&'a str>,
}

/// Splits a calendar into its top-level components, keeping nesting intact.
///
/// Lines inside a nested BEGIN/END are handed to the component that opened
/// them, so an event keeps its alarms rather than being interrupted by them.
fn components(unfolded: &str) -> Vec<Component<'_>> {
    let mut out: Vec<Component> = Vec::new();
    let mut depth = 0usize;
    let mut current: Option<Component> = None;

    for line in unfolded.lines() {
        let upper = line.trim().to_ascii_uppercase();

        if let Some(name) = upper.strip_prefix("BEGIN:") {
            depth += 1;
            // Only the outermost BEGIN starts a component; deeper ones are
            // content — an alarm inside an event, a zone inside a calendar.
            if depth == 1 {
                current = Some(Component {
                    name: name.trim().to_string(),
                    lines: Vec::new(),
                });
                continue;
            }
        }

        if upper.starts_with("END:") {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                if let Some(component) = current.take() {
                    out.push(component);
                }
                continue;
            }
        }

        if let Some(component) = current.as_mut() {
            component.lines.push(line);
        }
    }

    // An unterminated component is still worth what it holds.
    if let Some(component) = current {
        out.push(component);
    }
    out
}

/// Reads the events in a calendar.
///
/// A VCALENDAR wrapper is unwrapped; VTIMEZONE and anything else is passed
/// over. Reminders arrive as VTODO and are not events, which is why a
/// calendar-query asks for VEVENT specifically.
pub fn parse(raw: &str) -> Vec<Event> {
    let unfolded = unfold(raw);
    let top = components(&unfolded);

    let mut events = Vec::new();
    for component in top {
        match component.name.as_str() {
            "VCALENDAR" => {
                // The events are one level in. Re-splitting the inside gives
                // them as components in their own right.
                let inside = component.lines.join("\n");
                for inner in components(&inside) {
                    if inner.name == "VEVENT" {
                        if let Some(event) = read_event(&inner.lines) {
                            events.push(event);
                        }
                    }
                }
            }
            "VEVENT" => {
                if let Some(event) = read_event(&component.lines) {
                    events.push(event);
                }
            }
            _ => {}
        }
    }
    events
}

fn read_event(lines: &[&str]) -> Option<Event> {
    let mut event = Event::default();
    let mut depth = 0usize;

    for line in lines {
        let upper = line.trim().to_ascii_uppercase();
        // Skip anything belonging to a nested component: an alarm carries its
        // own DESCRIPTION, and taking it would replace the event's.
        if upper.starts_with("BEGIN:") {
            depth += 1;
            continue;
        }
        if upper.starts_with("END:") {
            depth = depth.saturating_sub(1);
            continue;
        }
        if depth > 0 {
            continue;
        }

        let Some(property) = parse_property(line) else {
            continue;
        };
        let value = unescape(property.value).trim().to_string();
        if value.is_empty() {
            continue;
        }

        match property.name.as_str() {
            "uid" => event.uid = value,
            "summary" => event.summary = value,
            "location" => event.location = Some(value),
            "description" => event.description = Some(value),
            "rrule" => event.rrule = Some(value),
            "recurrence-id" => event.recurrence_id = Some(value),
            "status" => event.cancelled = value.eq_ignore_ascii_case("CANCELLED"),
            "duration" => event.duration = Some(value),
            "dtstart" => event.start = Some(read_when(&property, &value)),
            "dtend" => event.end = Some(read_when(&property, &value)),
            _ => {}
        }
    }

    // Without a start there is nothing to put on a calendar, and without a UID
    // there is nothing to match it against on the next sync.
    (event.start.is_some() && !event.uid.is_empty()).then_some(event)
}

fn read_when(property: &crate::contentline::Property<'_>, value: &str) -> When {
    if property.param_is("value", "DATE") {
        return When::AllDay(value.to_string());
    }
    if value.ends_with('Z') {
        return When::Utc(value.to_string());
    }
    match property.param("tzid") {
        // The parameter keeps its own case in the source; zone names are
        // case-sensitive, so the original is what gets stored.
        Some(_) => When::Zoned {
            local: value.to_string(),
            tzid: property
                .params
                .get("tzid")
                .and_then(|v| v.first())
                .cloned()
                .unwrap_or_default(),
        },
        None => When::Floating(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like what iCloud sends: a timezone block, an alarm, and the
    /// event's own properties around them.
    const REAL: &str = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Apple Inc.//iOS 18.0//EN\r\n\
BEGIN:VTIMEZONE\r\n\
TZID:America/New_York\r\n\
BEGIN:DAYLIGHT\r\n\
TZNAME:EDT\r\n\
END:DAYLIGHT\r\n\
END:VTIMEZONE\r\n\
BEGIN:VEVENT\r\n\
UID:EVENT-1\r\n\
SUMMARY:Dentist\r\n\
LOCATION:12 High St\r\n\
DTSTART;TZID=America/New_York:20260917T090000\r\n\
DTEND;TZID=America/New_York:20260917T093000\r\n\
BEGIN:VALARM\r\n\
ACTION:DISPLAY\r\n\
DESCRIPTION:Reminder\r\n\
END:VALARM\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

    /// Parses every event in the backup taken from the real account.
    ///
    /// Ignored because it needs that backup on disk, but it reaches no
    /// network and needs no credential. Synthetic calendars prove the rules;
    /// a real one is what finds the shapes nobody thought to write a test for.
    #[test]
    #[ignore = "reads the local backup"]
    fn parses_the_real_calendar_backup() {
        let root = crate::config::Config::config_dir()
            .expect("config dir")
            .join("backups");

        let mut newest: Option<std::path::PathBuf> = None;
        for entry in std::fs::read_dir(&root).expect("no backups directory").flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("calendars").is_dir() {
                newest = Some(match newest {
                    Some(best) if best > path => best,
                    _ => path,
                });
            }
        }
        let calendars = newest.expect("no backup with calendars").join("calendars");

        let mut files = 0usize;
        let mut events = 0usize;
        let mut unreadable = Vec::new();
        let mut all_day = 0usize;
        let mut zoned = 0usize;
        let mut utc = 0usize;
        let mut floating = 0usize;
        let mut recurring = 0usize;

        let mut stack = vec![calendars];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_some_and(|e| e == "json") {
                    continue;
                }

                files += 1;
                let raw = std::fs::read_to_string(&path).unwrap_or_default();
                let found = parse(&raw);
                if found.is_empty() {
                    unreadable.push(path.clone());
                }
                for event in &found {
                    events += 1;
                    if event.rrule.is_some() {
                        recurring += 1;
                    }
                    match event.start.as_ref() {
                        Some(When::AllDay(_)) => all_day += 1,
                        Some(When::Zoned { .. }) => zoned += 1,
                        Some(When::Utc(_)) => utc += 1,
                        Some(When::Floating(_)) => floating += 1,
                        None => {}
                    }
                }
            }
        }

        println!("files {files}, events {events}");
        println!("  all-day {all_day}, zoned {zoned}, utc {utc}, floating {floating}");
        println!("  recurring {recurring}");
        for path in unreadable.iter().take(5) {
            println!("  no event found in {}", path.display());
        }

        assert!(files > 0, "the backup had no calendar files");
        assert!(
            unreadable.len() * 5 < files,
            "{} of {files} files produced no event",
            unreadable.len()
        );
    }
    #[test]
    fn reads_an_event_from_a_real_calendar() {
        let events = parse(REAL);
        assert_eq!(events.len(), 1, "one event, not one per BEGIN");
        let e = &events[0];
        assert_eq!(e.uid, "EVENT-1");
        assert_eq!(e.summary, "Dentist");
        assert_eq!(e.location.as_deref(), Some("12 High St"));
    }

    #[test]
    fn a_timezone_block_is_not_an_event() {
        // VTIMEZONE sits beside the events and has its own BEGIN/END. Counting
        // those as components produces phantom entries in the calendar.
        assert!(parse(REAL).iter().all(|e| e.uid == "EVENT-1"));
    }

    #[test]
    fn an_alarm_does_not_overwrite_its_event() {
        // The VALARM carries DESCRIPTION:Reminder. Reading nested lines as the
        // event's own would give every alarmed event the same description.
        assert_eq!(parse(REAL)[0].description, None);
    }

    #[test]
    fn a_zoned_time_keeps_its_zone_rather_than_being_guessed_at() {
        // Resolving it needs the zone database, and guessing puts events an
        // hour out twice a year.
        let start = parse(REAL)[0].start.clone().unwrap();
        assert_eq!(
            start,
            When::Zoned {
                local: "20260917T090000".into(),
                tzid: "America/New_York".into()
            }
        );
    }

    #[test]
    fn the_four_kinds_of_time_are_told_apart() {
        let raw = "BEGIN:VEVENT\r\nUID:A\r\nDTSTART;VALUE=DATE:20260917\r\nEND:VEVENT\r\n";
        assert_eq!(parse(raw)[0].start, Some(When::AllDay("20260917".into())));
        assert!(parse(raw)[0].start.as_ref().unwrap().is_all_day());

        let raw = "BEGIN:VEVENT\r\nUID:B\r\nDTSTART:20260917T130000Z\r\nEND:VEVENT\r\n";
        assert_eq!(parse(raw)[0].start, Some(When::Utc("20260917T130000Z".into())));

        let raw = "BEGIN:VEVENT\r\nUID:C\r\nDTSTART:20260917T090000\r\nEND:VEVENT\r\n";
        assert_eq!(
            parse(raw)[0].start,
            Some(When::Floating("20260917T090000".into()))
        );
    }

    #[test]
    fn an_event_without_a_start_is_not_an_event() {
        let raw = "BEGIN:VEVENT\r\nUID:D\r\nSUMMARY:Nothing\r\nEND:VEVENT\r\n";
        assert!(parse(raw).is_empty());
    }

    #[test]
    fn recurrence_is_kept_as_written() {
        // Expanding it badly produces a calendar wrong in a way nobody sees.
        let raw = "BEGIN:VEVENT\r\nUID:E\r\nDTSTART:20260917T090000Z\r\nRRULE:FREQ=WEEKLY;BYDAY=TH\r\nEND:VEVENT\r\n";
        assert_eq!(parse(raw)[0].rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=TH"));
    }

    #[test]
    fn a_cancelled_event_still_exists() {
        // A meeting called off is information, not an absence.
        let raw = "BEGIN:VEVENT\r\nUID:F\r\nDTSTART:20260917T090000Z\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n";
        assert!(parse(raw)[0].cancelled);
    }

    #[test]
    fn several_events_in_one_calendar() {
        let raw = format!(
            "BEGIN:VCALENDAR\r\n{}{}END:VCALENDAR\r\n",
            "BEGIN:VEVENT\r\nUID:1\r\nDTSTART:20260917T090000Z\r\nEND:VEVENT\r\n",
            "BEGIN:VEVENT\r\nUID:2\r\nDTSTART:20260918T090000Z\r\nEND:VEVENT\r\n",
        );
        assert_eq!(parse(&raw).len(), 2);
    }

    #[test]
    fn escaped_text_comes_back_readable() {
        let raw = "BEGIN:VEVENT\r\nUID:G\r\nDTSTART:20260917T090000Z\r\nSUMMARY:Lunch\\, then a walk\r\nEND:VEVENT\r\n";
        assert_eq!(parse(raw)[0].summary, "Lunch, then a walk");
    }
}
