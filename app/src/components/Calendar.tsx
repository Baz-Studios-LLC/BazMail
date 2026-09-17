import { useMemo, useState } from "react";
import { Panel } from "./Panel";
import type { Contact, ContactSync } from "../types";

interface CalendarProps {
  events: Contact[];
  onSync: () => void;
  syncing: boolean;
  outcomes: ContactSync[];
  onClose: () => void;
}

interface EventDetails {
  start?: string;
  end?: string;
  allDay?: boolean;
  /** The zone the event was written in, when it named one. */
  tzid?: string | null;
  location?: string | null;
  recurring?: boolean;
  cancelled?: boolean;
  calendar?: string;
}

function details(event: Contact): EventDetails {
  try {
    return JSON.parse(event.detailsJson || "{}");
  } catch {
    return {};
  }
}

/**
 * Reads the stored form, `YYYYMMDD` or `YYYYMMDDTHHMMSS[Z]`.
 *
 * Built by hand rather than handed to Date, because Date would apply this
 * machine's zone to a time that may belong to another one — the exact guess
 * the parser refused to make. What comes back is the wall time as written.
 */
function readStamp(value: string | undefined) {
  if (!value || value.length < 8) return null;
  const year = Number(value.slice(0, 4));
  const month = Number(value.slice(4, 6));
  const day = Number(value.slice(6, 8));
  if (!year || !month || !day) return null;

  const hasTime = value.length >= 15 && value[8] === "T";
  return {
    year,
    month,
    day,
    hour: hasTime ? Number(value.slice(9, 11)) : 0,
    minute: hasTime ? Number(value.slice(11, 13)) : 0,
    hasTime,
    /** Day identity, for grouping. */
    key: value.slice(0, 8),
  };
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

function dayHeading(key: string) {
  const stamp = readStamp(key);
  if (!stamp) return key;
  // Midday, so a date never slips across a boundary when the browser applies
  // its own zone to work out which weekday it was.
  const at = new Date(stamp.year, stamp.month - 1, stamp.day, 12);
  const today = new Date();
  const sameDay =
    at.getFullYear() === today.getFullYear() &&
    at.getMonth() === today.getMonth() &&
    at.getDate() === today.getDate();

  const label = `${DAYS[at.getDay()]} ${stamp.day} ${MONTHS[stamp.month - 1]}`;
  return sameDay ? `Today · ${label}` : label;
}

function clockOf(value: string | undefined) {
  const stamp = readStamp(value);
  if (!stamp || !stamp.hasTime) return null;
  return `${String(stamp.hour).padStart(2, "0")}:${String(stamp.minute).padStart(2, "0")}`;
}

export function Calendar({ events, onSync, syncing, outcomes, onClose }: CalendarProps) {
  const [showPast, setShowPast] = useState(false);

  const today = useMemo(() => {
    const now = new Date();
    return `${now.getFullYear()}${String(now.getMonth() + 1).padStart(2, "0")}${String(
      now.getDate(),
    ).padStart(2, "0")}`;
  }, []);

  const { upcoming, pastCount } = useMemo(() => {
    const withDay = events
      .map((event) => ({ event, detail: details(event) }))
      .map((row) => ({ ...row, day: readStamp(row.detail.start)?.key ?? "" }))
      .filter((row) => row.day);

    const past = withDay.filter((row) => row.day < today);
    const ahead = withDay.filter((row) => row.day >= today);
    return {
      upcoming: showPast ? withDay : ahead,
      pastCount: past.length,
    };
  }, [events, today, showPast]);

  // Grouped in order, so each day's heading is written once.
  const days = useMemo(() => {
    const out: { day: string; rows: typeof upcoming }[] = [];
    for (const row of upcoming) {
      const last = out[out.length - 1];
      if (last && last.day === row.day) last.rows.push(row);
      else out.push({ day: row.day, rows: [row] });
    }
    return out;
  }, [upcoming]);

  const failures = outcomes.filter((o) => o.error);

  return (
    <Panel
      title="Calendar"
      onBack={onClose}
      backLabel="Back to mail"
      actions={
        <button className="btn-quiet" onClick={onSync} disabled={syncing}>
          {syncing ? "Syncing…" : "Sync"}
        </button>
      }
    >
      <div className="agenda">
        {events.length === 0 && (
          <div className="empty" style={{ padding: 28 }}>
            No events yet. Sync to fetch them.
          </div>
        )}

        {events.length > 0 && days.length === 0 && (
          <div className="empty" style={{ padding: 28 }}>
            Nothing coming up.
          </div>
        )}

        {!showPast && pastCount > 0 && days.length > 0 && (
          <button className="agenda-past" onClick={() => setShowPast(true)}>
            Show {pastCount} earlier {pastCount === 1 ? "event" : "events"}
          </button>
        )}

        {days.map(({ day, rows }) => (
          <div className="agenda-day" key={day}>
            <div className="agenda-heading">{dayHeading(day)}</div>
            {rows.map(({ event, detail }) => (
              <div
                className={`agenda-row ${detail.cancelled ? "cancelled" : ""}`}
                key={event.url}
              >
                <div className="agenda-when">
                  {detail.allDay ? "All day" : (clockOf(detail.start) ?? "—")}
                </div>
                <div className="agenda-what">
                  <div className="agenda-summary">
                    {event.displayName || "(no title)"}
                    {detail.recurring && <span className="agenda-tag">repeats</span>}
                    {detail.cancelled && <span className="agenda-tag">cancelled</span>}
                  </div>
                  {detail.location && (
                    <div className="agenda-where">{detail.location}</div>
                  )}
                  {/* Shown when the event names a zone that is not this one:
                      the time above is the wall time where the event is, and
                      saying so is better than quietly converting it wrongly. */}
                  {detail.tzid && (
                    <div className="agenda-zone">{detail.tzid}</div>
                  )}
                </div>
              </div>
            ))}
          </div>
        ))}
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
