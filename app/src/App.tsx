import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "./api";
import { useUpdater } from "./useUpdater";
import { announce } from "./notify";
import { Resizer, clampListWidth, LIST_DEFAULT } from "./components/Resizer";
import type { Account, EmailBody, Envelope, Mailbox, Status } from "./types";
import { Sidebar, type View } from "./components/Sidebar";
import { MessageList } from "./components/MessageList";
import { Reader } from "./components/Reader";
import { TitleBar } from "./components/TitleBar";
import { SignIn } from "./components/SignIn";
import { Settings } from "./components/Settings";
import { Compose, type Draft } from "./components/Compose";
import { AvatarMenu } from "./components/AvatarMenu";
import {
  CalendarIcon,
  ComposeIcon,
  MailIcon,
  PeopleIcon,
  SearchIcon,
} from "./components/Icons";

const UNIFIED: View = { kind: "unified", title: "Inbox" };

/**
 * How often to look for new mail.
 *
 * Polling, and honestly so: JMAP has a push channel and IMAP has IDLE, and
 * either would beat this. Three minutes is the compromise until one of them
 * exists — often enough that mail is not visibly stale, rare enough that two
 * accounts are not being woken constantly.
 */
const MAIL_CHECK_INTERVAL_MS = 3 * 60 * 1000;

// Native decorations are off in Tauri, so we draw our own caption. In a
// browser the surrounding chrome is the browser's own and there is nothing to
// replace.
const NATIVE = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export default function App() {
  const [status, setStatus] = useState<Status | null>(null);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [mailboxesByAccount, setMailboxes] = useState<Record<string, Mailbox[]>>({});
  const [envelopes, setEnvelopes] = useState<Envelope[]>([]);
  const [view, setView] = useState<View>(UNIFIED);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [body, setBody] = useState<EmailBody | null>(null);
  const [bodyLoading, setBodyLoading] = useState(false);
  const [bodyError, setBodyError] = useState<string | null>(null);

  const [note, setNote] = useState("Starting up…");
  // Domains allowed to load images without asking. Verified domains only;
  // the engine refuses anything else, so this cannot drift into a list of
  // From addresses.
  const [imageDomains, setImageDomains] = useState<string[]>([]);
  const [syncing, setSyncing] = useState(false);
  // Shown on first run, and whenever another account is being added.
  const [signingIn, setSigningIn] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  // How long a message must stay open before it counts as read. Persisted so it
  // survives a restart; -1 disables it entirely.
  const [markReadDelay, setMarkReadDelay] = useState<number>(() => {
    const stored = Number(localStorage.getItem("bazmail.markReadDelay"));
    return Number.isFinite(stored) && stored !== 0 ? stored : 900;
  });
  const [unreadOnly, setUnreadOnly] = useState(
    () => localStorage.getItem("bazmail.unreadOnly") === "true",
  );
  // The account whose credential is being renewed, if any. Reconnecting is not
  // adding: the id, label and colour all survive, so the unified list does not
  // reshuffle underneath you and archived mail keeps its account.
  const [reconnecting, setReconnecting] = useState<Account | null>(null);
  // Non-null while composing. Holds the whole draft so switching to a
  // message and back does not lose what was typed.
  const [draft, setDraft] = useState<Draft | null>(null);
  const { state: updateState, checkNow } = useUpdater();
  // Read by the background poll. Held in refs rather than passed as
  // dependencies so that changing view does not tear down and restart the
  // timer — with a three-minute interval, navigating every couple of minutes
  // would mean it never fired at all.
  const viewRef = useRef(view);
  const accountsRef = useRef(accounts);
  const checking = useRef(false);
  // Every message id already seen in this session. Primed on the first load so
  // launching the app does not announce an inbox you have been reading for
  // weeks as though it all just arrived.
  const seen = useRef<Set<string> | null>(null);
  // Remembered across restarts: a column width is a preference, and
  // re-dragging it every launch would make it a chore instead.
  const [listWidth, setListWidth] = useState(() => {
    const stored = Number(localStorage.getItem("bazmail.listWidth"));
    return Number.isFinite(stored) && stored > 0
      ? clampListWidth(stored)
      : LIST_DEFAULT;
  });

  /**
   * Renews an account's credential in place.
   *
   * A revoked OAuth grant is the common case and costs one click — the browser
   * opens, you approve, and the same account keeps working. IMAP cannot be
   * renewed silently because the app password was never kept anywhere it could
   * be read back, which is the point of storing it in the OS credential store.
   */
  async function reconnect(account: Account) {
    setShowSettings(false);
    if (account.connection !== "oauth") {
      setReconnecting(account);
      setSigningIn(true);
      return;
    }
    try {
      setNote(`Reconnecting ${account.label} — approve it in your browser.`);
      await api.connectFastmail(account.color);
      void bootstrap();
    } catch (e) {
      setNote(String(e));
    }
  }
  // The last archive, so Z can reverse it. Undo re-issues the opposite move
  // rather than cancelling the queued one, so it works whether or not the
  // archive already reached the server.
  const [lastArchived, setLastArchived] = useState<Envelope | null>(null);

  useEffect(() => {
    viewRef.current = view;
    accountsRef.current = accounts;
  }, [view, accounts]);

  const selected = envelopes.find((e) => e.id === selectedId) ?? null;

  /**
   * Pulls the current view out of the local store — never off the network.
   *
   * Returns what it loaded as well as setting it, so the background check can
   * work out what is new without querying a second time.
   */
  const loadFromStore = useCallback(
    async (current: View, accountList: Account[]): Promise<Envelope[]> => {
    const boxes: Record<string, Mailbox[]> = {};
    for (const account of accountList) {
      boxes[account.id] = await api.mailboxes(account.id);
    }
    setMailboxes(boxes);

    let loaded: Envelope[];
    if (current.kind === "unified") {
      loaded = await api.unifiedInbox(200);
    } else if (current.mailboxId) {
      // Asked for directly rather than filtered out of the unified set. That
      // set is inbox-only by design, so narrowing it to Spam or Archive could
      // never match anything — the folder looked permanently empty while its
      // unread count, which comes from the server's mailbox list, was right.
      loaded = await api.mailboxEnvelopes(current.mailboxId, 300);
    } else {
      loaded = [];
    }
    setEnvelopes(loaded);
    return loaded;
  }, []);

  const bootstrap = useCallback(async () => {
      setSyncing(true);
      try {
        const s = await api.status();
        setStatus(s);
        setAccounts(s.accounts);

        // Before the first message can be opened: a reader that does not yet
        // know a domain is allowed would block images the user already said to
        // stop asking about, then load them a moment later.
        await api
          .imageDomains()
          .then(setImageDomains)
          .catch(() => setImageDomains([]));

        if (!s.configured) {
          setNote("No account configured yet.");
          return;
        }

        // The mirror is a complete copy of what was last synced, so there is no
        // reason to look at an empty window while the network answers. Paint it
        // first, then go and find out what changed.
        await loadFromStore(UNIFIED, s.accounts);

        // Anything queued from a previous session goes out before we sync, so
        // the server is not still reporting state the user already changed.
        await api.flushOutbox().catch(() => 0);

        setNote("Syncing…");
        const outcomes = await api.syncAll(200);
        const failed = outcomes.filter((o) => !o.ok);
        const synced = outcomes.reduce((n, o) => n + o.envelopes, 0);

        const loaded = await loadFromStore(UNIFIED, s.accounts);
        seen.current = new Set(loaded.map((e) => e.id));

        setNote(
          failed.length > 0
            ? `${failed.map((f) => `${f.accountId}: ${f.error}`).join(" · ")}`
            : `Synced ${synced} messages across ${outcomes.length} account${
                outcomes.length === 1 ? "" : "s"
              }.`,
        );

        // Every account failing almost always means a missing or revoked
        // credential, and the fix is to sign in again — so go there rather than
        // leaving an error in the status bar and a healthy-looking empty inbox.
        if (outcomes.length > 0 && failed.length === outcomes.length) {
          setSigningIn(true);
        }
      } catch (e) {
        setNote(String(e));
      } finally {
        setSyncing(false);
      }
  }, [loadFromStore]);

  useEffect(() => {
    void bootstrap();
  }, [bootstrap]);

  /**
   * Checks for mail without disturbing what you are doing.
   *
   * Deliberately not `bootstrap`, which reloads the *unified* view and would
   * throw you back to the inbox every few minutes if you were reading Spam.
   * This reloads whatever view is open and leaves the selection alone, so a
   * message arriving while you read one does not move the one you are on.
   *
   * Failures are silent. A poll that cannot reach the network is the ordinary
   * state of a laptop, and an error in the status bar every few minutes would
   * train you to ignore the status bar.
   */
  const checkForMail = useCallback(async () => {
    // A slow sync must not stack up behind itself: an account on a poor
    // connection can easily outlast the interval.
    if (checking.current) return;
    checking.current = true;
    try {
      await api.syncAll(200);
      const loaded = await loadFromStore(viewRef.current, accountsRef.current);

      // Nothing is announced until the first load has established what was
      // already there — otherwise the first check after launch would report
      // the whole inbox as new.
      if (seen.current) {
        const previous = seen.current;
        const fresh = loaded.filter((e) => e.isUnread && !previous.has(e.id));
        void announce(fresh, accountsRef.current);
      }
      seen.current = new Set(loaded.map((e) => e.id));
    } catch {
      // Offline, a revoked token, a server having a moment. The next tick tries
      // again and the mirror still holds everything from last time.
    } finally {
      checking.current = false;
    }
  }, [loadFromStore]);

  useEffect(() => {
    if (accounts.length === 0) return;
    const timer = setInterval(() => void checkForMail(), MAIL_CHECK_INTERVAL_MS);
    return () => clearInterval(timer);
  }, [checkForMail, accounts.length]);

  const openMessage = useCallback(async (envelope: Envelope) => {
    // Clicking a message is a decision to go back to reading, so it takes you
    // out of whatever panel is open rather than being swallowed by it.
    setShowSettings(false);
    setSigningIn(false);
    setSelectedId(envelope.id);
    setBody(null);
    setBodyError(null);
    setBodyLoading(true);
    try {
      setBody(await api.body(envelope.accountId, envelope.id));
    } catch (e) {
      setBodyError(String(e));
    } finally {
      setBodyLoading(false);
    }
  }, []);

  // A panel takes over the reading pane, so leaving a row highlighted behind it
  // claims something is open that is not. Clearing it also means j/k resume from
  // the top rather than from a message you can no longer see.
  useEffect(() => {
    if (showSettings || signingIn || draft) {
      setSelectedId(null);
      setBody(null);
      setBodyError(null);
    }
  }, [showSettings, signingIn, draft]);

  // Marking read is deferred rather than immediate. j/k opens as it moves, so
  // marking on open would mark everything you scroll past — the classic way this
  // gets it wrong. A short dwell means only what you actually stopped on counts
  // as read, and moving on before it elapses cancels it.
  useEffect(() => {
    if (markReadDelay < 0) return;
    const envelope = envelopes.find((e) => e.id === selectedId);
    if (!envelope || !envelope.isUnread) return;

    const timer = setTimeout(() => {
      void api
        .markRead(envelope.accountId, envelope.id, true)
        .then(() =>
          setEnvelopes((current) =>
            current.map((e) =>
              e.id === envelope.id ? { ...e, isUnread: false } : e,
            ),
          ),
        )
        .catch((e) => setNote(`Could not mark read: ${e}`));
    }, Math.max(0, markReadDelay));

    return () => clearTimeout(timer);
  }, [selectedId, envelopes, markReadDelay]);

  const selectView = useCallback(
    async (next: View) => {
      setView(next);
      setSelectedId(null);
      setBody(null);
      // Whatever the mirror already holds, immediately — the network comes
      // after, so opening a folder never waits on it.
      await loadFromStore(next, accounts);

      // Only inboxes are pulled by the periodic sync, because syncing every
      // folder on every account would make startup cost far more than it is
      // worth. Everything else is fetched the first time you look at it, which
      // is why Spam stayed empty: nothing had ever asked for it.
      if (next.kind !== "mailbox" || !next.accountId || !next.mailboxId) return;
      try {
        setSyncing(true);
        await api.syncMailbox(next.accountId, next.mailboxId, 200);
        await loadFromStore(next, accounts);
      } catch (e) {
        setNote(String(e));
      } finally {
        setSyncing(false);
      }
    },
    [accounts, loadFromStore],
  );

  /**
   * Opens a reply to the selected message.
   *
   * `all` keeps everyone who was on it minus yourself — replying to a list and
   * quietly dropping the other recipients is the kind of thing that only shows
   * up after the conversation has already split in two.
   */
  const replyTo = useCallback(
    (envelope: Envelope, all: boolean) => {
      const account = accounts.find((a) => a.id === envelope.accountId);
      const me = account?.identity.toLowerCase();
      const format = (list: { name: string | null; email: string }[]) =>
        list
          .filter((a) => a.email.toLowerCase() !== me)
          .map((a) => (a.name ? `${a.name} <${a.email}>` : a.email))
          .join(", ");

      // References is the existing chain plus the message being answered; a
      // reply that drops it threads correctly in some clients and not others.
      const references = [...envelope.references];
      if (envelope.messageId && !references.includes(envelope.messageId)) {
        references.push(envelope.messageId);
      }

      setShowSettings(false);
      setSigningIn(false);
      setDraft({
        accountId: envelope.accountId,
        to: format(envelope.from),
        cc: all ? format(envelope.to) : "",
        subject: /^re:/i.test(envelope.subject)
          ? envelope.subject
          : `Re: ${envelope.subject}`,
        // Two blank lines above it, so the cursor lands where you write rather
        // than on top of your own name.
        text: account?.signature ? `\n\n${account.signature}` : "",
        inReplyTo: envelope.messageId,
        references,
      });
    },
    [accounts],
  );

  /**
   * Deletes the selected message and advances, exactly like archive.
   *
   * The row goes immediately because the mirror has already been written; the
   * queue carries it to the server afterwards.
   */
  const trashSelected = useCallback(async () => {
    const envelope = envelopes.find((e) => e.id === selectedId);
    if (!envelope) return;

    const index = envelopes.findIndex((e) => e.id === selectedId);
    const remaining = envelopes.filter((e) => e.id !== envelope.id);
    const next = remaining[Math.min(index, remaining.length - 1)] ?? null;

    setEnvelopes(remaining);
    if (next) {
      void openMessage(next);
    } else {
      setSelectedId(null);
      setBody(null);
    }
    try {
      await api.trash(envelope.accountId, envelope.id);
    } catch (e) {
      setNote(String(e));
    }
  }, [envelopes, selectedId, openMessage]);

  /** Flags or unflags without moving the selection — it is a mark, not a verb. */
  const toggleFlag = useCallback(async () => {
    const envelope = envelopes.find((e) => e.id === selectedId);
    if (!envelope) return;
    const flagged = !envelope.isFlagged;
    setEnvelopes((list) =>
      list.map((e) => (e.id === envelope.id ? { ...e, isFlagged: flagged } : e)),
    );
    try {
      await api.setFlagged(envelope.accountId, envelope.id, flagged);
    } catch (e) {
      setNote(String(e));
    }
  }, [envelopes, selectedId]);

  /**
   * Marks the selected message unread and steps off it.
   *
   * Staying on it would let the dwell timer mark it read again a second later,
   * which is the least useful possible outcome of asking for it to be unread.
   */
  const markUnread = useCallback(async () => {
    const envelope = envelopes.find((e) => e.id === selectedId);
    if (!envelope) return;
    setEnvelopes((list) =>
      list.map((e) => (e.id === envelope.id ? { ...e, isUnread: true } : e)),
    );
    setSelectedId(null);
    setBody(null);
    try {
      await api.markRead(envelope.accountId, envelope.id, false);
    } catch (e) {
      setNote(String(e));
    }
  }, [envelopes, selectedId]);

  /// Archive and advance — the whole triage loop in one keystroke. The row is
  /// dropped from the list immediately; the engine has already applied it to the
  /// local mirror, so there is nothing to wait for.
  const archiveSelected = useCallback(async () => {
    const envelope = envelopes.find((e) => e.id === selectedId);
    if (!envelope) return;

    const index = envelopes.findIndex((e) => e.id === selectedId);
    const remaining = envelopes.filter((e) => e.id !== envelope.id);
    const next = remaining[Math.min(index, remaining.length - 1)] ?? null;

    setEnvelopes(remaining);
    setLastArchived(envelope);
    if (next) {
      void openMessage(next);
    } else {
      setSelectedId(null);
      setBody(null);
    }

    try {
      const outcome = await api.archive(envelope.accountId, envelope.id);
      setNote(
        outcome.queued > 0
          ? `Archived — ${outcome.queued} change${
              outcome.queued === 1 ? "" : "s"
            } still queued. Z to undo.`
          : "Archived. Z to undo.",
      );
    } catch (e) {
      // The mirror already moved it, so the list is not rolled back — the change
      // is what was asked for and stays queued. Only the message changes.
      setNote(`Archived locally, not yet sent: ${e}`);
    }
  }, [envelopes, selectedId, openMessage]);

  const undoArchive = useCallback(async () => {
    if (!lastArchived) return;
    const envelope = lastArchived;
    setLastArchived(null);
    try {
      await api.unarchive(envelope.accountId, envelope.id);
      await loadFromStore(view, accounts);
      setNote(`Moved "${envelope.subject || "(no subject)"}" back to the inbox.`);
    } catch (e) {
      setNote(`Could not undo: ${e}`);
    }
  }, [lastArchived, loadFromStore, view, accounts]);

  // Filtering to unread is a reading mode, not a one-off, so it survives a
  // restart the way the mark-read delay does.
  const visibleEnvelopes = useMemo(
    () =>
      unreadOnly
        // The open message stays in the list even once it has been read.
        // Without this it vanishes under the cursor a second after opening —
        // the dwell timer marks it read — and you lose your place in the
        // middle of reading it.
        ? envelopes.filter((e) => e.isUnread || e.id === selectedId)
        : envelopes,
    [envelopes, unreadOnly, selectedId],
  );

  // The triage loop: j / k move, e archives and advances, z undoes.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      const target = event.target as HTMLElement | null;
      if (target && /^(INPUT|TEXTAREA)$/.test(target.tagName)) return;

      if (event.key === "Escape") {
        setSelectedId(null);
        setBody(null);
        return;
      }
      if (event.key === "r" || event.key === "R") {
        const envelope = envelopes.find((e) => e.id === selectedId);
        if (!envelope) return;
        event.preventDefault();
        replyTo(envelope, event.key === "R");
        return;
      }
      if (event.key === "#" || event.key === "Delete" || event.key === "Backspace") {
        event.preventDefault();
        void trashSelected();
        return;
      }
      if (event.key === "f") {
        event.preventDefault();
        void toggleFlag();
        return;
      }
      if (event.key === "u") {
        event.preventDefault();
        void markUnread();
        return;
      }
      if (event.key === "e") {
        event.preventDefault();
        void archiveSelected();
        return;
      }
      if (event.key === "z") {
        event.preventDefault();
        void undoArchive();
        return;
      }
      if (event.key !== "j" && event.key !== "k") return;

      event.preventDefault();
      if (visibleEnvelopes.length === 0) return;

      const current = visibleEnvelopes.findIndex((e) => e.id === selectedId);
      const step = event.key === "j" ? 1 : -1;
      const nextIndex =
        current < 0
          ? 0
          : Math.min(visibleEnvelopes.length - 1, Math.max(0, current + step));
      void openMessage(visibleEnvelopes[nextIndex]);
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [visibleEnvelopes, envelopes, selectedId, openMessage, archiveSelected, undoArchive, replyTo, trashSelected, toggleFlag, markUnread]);

  const unreadTotal = envelopes.filter((e) => e.isUnread).length;

  return (
    <div className="window">
      <div className="rail">
        <div className="rail-item active">
          <MailIcon size={22} />
        </div>
        <div className="rail-item">
          <PeopleIcon size={22} />
        </div>
        <div className="rail-item">
          <CalendarIcon size={22} />
        </div>
        <div className="rail-spacer" />
        <AvatarMenu
          initials="BB"
          syncing={syncing}
          onSync={() => void bootstrap()}
          onSettings={() => {
            setSigningIn(false);
            setShowSettings(true);
          }}
          onAddAccount={() => {
            setShowSettings(false);
            setSigningIn(true);
          }}
        />
      </div>

      <div className="main">
      {NATIVE && <TitleBar />}
      <div className="shell">

      <Sidebar
        accounts={accounts}
        mailboxesByAccount={mailboxesByAccount}
        view={view}
        unreadTotal={unreadTotal}
        onSelect={(next) => void selectView(next)}
        onAddAccount={() => setSigningIn(true)}
        onMoveAccount={(accountId, up) => {
          void api
            .moveAccount(accountId, up)
            .then(() => bootstrap())
            .catch((e) => setNote(String(e)));
        }}
      />

      <div className="pane">
      {/* The list's width is set here rather than in the stylesheet so the
          drag has one number to move, and the card is what carries it because
          the strip sits between two of its children. */}
      <div className="card" style={{ "--list-width": `${listWidth}px` } as React.CSSProperties}>
        <MessageList
          title={view.title}
          envelopes={visibleEnvelopes}
          accounts={accounts}
          selectedId={selectedId}
          onSelect={(envelope) => void openMessage(envelope)}
          unreadOnly={unreadOnly}
          onToggleUnreadOnly={() => {
            const next = !unreadOnly;
            setUnreadOnly(next);
            localStorage.setItem("bazmail.unreadOnly", String(next));
          }}
        />

        <Resizer
          width={listWidth}
          onChange={setListWidth}
          onCommit={(w) => localStorage.setItem("bazmail.listWidth", String(w))}
        />

        <section className="reader">
          <div className="commandbar">
            <div className="search">
              <SearchIcon size={15} />
              <span style={{ flexGrow: 1 }}>Search or ask</span>
              <span className="kbd">Ctrl K</span>
            </div>
            <button
              className="btn-primary"
              disabled={accounts.length === 0}
              onClick={() => {
                // Every account can send now — JMAP through submission, IMAP
                // through SMTP — so the first one is as good as any.
                const sender = accounts[0];
                setShowSettings(false);
                setSigningIn(false);
                setDraft({
                  accountId: sender?.id ?? "",
                  to: "",
                  cc: "",
                  subject: "",
                  text: sender?.signature ? `\n\n${sender.signature}` : "",
                });
              }}
            >
              <ComposeIcon size={15} />
              Compose
            </button>
          </div>

          {draft ? (
            <Compose
              accounts={accounts}
              draft={draft}
              onClose={() => setDraft(null)}
              onSent={(warning) => {
                setDraft(null);
                // A warning means it went but the copy did not file. Said here
                // rather than swallowed: the send succeeded, so this is the only
                // place it can be mentioned at all.
                setNote(warning ?? "Sent.");
                void bootstrap();
              }}
            />
          ) : showSettings ? (
            <Settings
              status={status}
              accounts={accounts}
              markReadDelay={markReadDelay}
              onMarkReadDelayChange={(ms) => {
                setMarkReadDelay(ms);
                localStorage.setItem("bazmail.markReadDelay", String(ms));
              }}
              onAddAccount={() => {
                setShowSettings(false);
                setReconnecting(null);
                setSigningIn(true);
              }}
              onReconnect={(account) => void reconnect(account)}
              updateState={updateState}
              onCheckForUpdates={() => void checkNow()}
              onChanged={() => void bootstrap()}
              onClose={() => setShowSettings(false)}
            />
          ) : (signingIn || (status && !status.configured)) ? (
            <SignIn
              existingCount={accounts.length}
              usedColors={accounts.map((a) => a.color)}
              reconnecting={reconnecting}
              onCancel={
                signingIn
                  ? () => {
                      setSigningIn(false);
                      setReconnecting(null);
                    }
                  : undefined
              }
              onConnected={() => {
                setSigningIn(false);
                setReconnecting(null);
                void bootstrap();
              }}
            />
          ) : (
            <Reader
              envelope={selected}
              body={body}
              loading={bodyLoading}
              error={bodyError}
              accounts={accounts}
              imageDomains={imageDomains}
              onAllowImages={(domain) => {
                // Applied locally first so the images appear on this message
                // straight away; the write is what makes it outlast the app.
                setImageDomains((current) =>
                  current.includes(domain) ? current : [...current, domain],
                );
                api
                  .allowImagesFrom(domain)
                  .catch((e) => setNote(`Could not remember that: ${e}`));
              }}
              onReply={(all) => selected && replyTo(selected, all)}
            />
          )}

          <div className="status-line">{note}</div>
          </section>
        </div>
      </div>
      </div>
      </div>
    </div>
  );
}
