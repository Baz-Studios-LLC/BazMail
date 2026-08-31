import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "./api";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Account, EmailBody, Envelope, Mailbox, Status } from "./types";
import { Sidebar, type View } from "./components/Sidebar";
import { MessageList } from "./components/MessageList";
import { Reader } from "./components/Reader";
import { TitleBar } from "./components/TitleBar";
import { SignIn } from "./components/SignIn";
import { Settings } from "./components/Settings";
import { AvatarMenu } from "./components/AvatarMenu";
import {
  CalendarIcon,
  ComposeIcon,
  MailIcon,
  PeopleIcon,
  SearchIcon,
} from "./components/Icons";

const UNIFIED: View = { kind: "unified", title: "Inbox" };

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

  const selected = envelopes.find((e) => e.id === selectedId) ?? null;

  /** Pulls the current view out of the local store — never off the network. */
  const loadFromStore = useCallback(async (current: View, accountList: Account[]) => {
    const boxes: Record<string, Mailbox[]> = {};
    for (const account of accountList) {
      boxes[account.id] = await api.mailboxes(account.id);
    }
    setMailboxes(boxes);

    if (current.kind === "unified") {
      setEnvelopes(await api.unifiedInbox(200));
    } else {
      // The store query filters by mailbox; asking for the unified set and
      // narrowing here keeps one code path until per-mailbox sync exists.
      const all = await api.unifiedInbox(500);
      setEnvelopes(all.filter((e) => e.mailboxIds.includes(current.mailboxId ?? "")));
    }
  }, []);

  const bootstrap = useCallback(async () => {
      setSyncing(true);
      try {
        const s = await api.status();
        setStatus(s);
        setAccounts(s.accounts);

        if (!s.configured) {
          setNote("No account configured yet.");
          return;
        }

        // Anything queued from a previous session goes out before we sync, so
        // the server is not still reporting state the user already changed.
        await api.flushOutbox().catch(() => 0);

        setNote("Syncing…");
        const outcomes = await api.syncAll(200);
        const failed = outcomes.filter((o) => !o.ok);
        const synced = outcomes.reduce((n, o) => n + o.envelopes, 0);

        await loadFromStore(UNIFIED, s.accounts);

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
    if (showSettings || signingIn) {
      setSelectedId(null);
      setBody(null);
      setBodyError(null);
    }
  }, [showSettings, signingIn]);

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
      await loadFromStore(next, accounts);
    },
    [accounts, loadFromStore],
  );

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

  // Updates the way Chrome does them: checked and fetched quietly in the
  // background, applied when you next close the app. Nothing interrupts
  // reading mail, and there is no dialog asking permission to do the thing you
  // already asked for by installing it.
  useEffect(() => {
    let pending: Update | null = null;
    let cancelled = false;

    void (async () => {
      try {
        const update = await check();
        if (!update || cancelled) return;
        await update.download();
        pending = update;
      } catch {
        // A failed check is not worth reporting: offline, a rate limit, no
        // release published yet. It tries again next launch, and in the
        // meantime the copy you have works fine.
      }
    })();

    const stopping = getCurrentWindow().onCloseRequested(async (event) => {
      if (!pending) return;
      event.preventDefault();

      // Cleared before installing rather than after: if the installer fails,
      // the next close has to actually close, not trap the window in a retry.
      const update = pending;
      pending = null;
      try {
        await update.install();
      } catch {
        // Nothing useful to do at exit. The download is kept and reapplied
        // next time rather than being lost.
      }
      void getCurrentWindow().destroy();
    });

    return () => {
      cancelled = true;
      void stopping.then((stop) => stop());
    };
  }, []);

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
  }, [visibleEnvelopes, selectedId, openMessage, archiveSelected, undoArchive]);

  const unreadTotal = envelopes.filter((e) => e.isUnread).length;

  return (
    <div className="window">
      <div className="rail">
        <div className="rail-item active">
          <MailIcon size={19} />
        </div>
        <div className="rail-item">
          <PeopleIcon size={19} />
        </div>
        <div className="rail-item">
          <CalendarIcon size={19} />
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
      <div className="card">
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

        <section className="reader">
          <div className="commandbar">
            <div className="search">
              <SearchIcon size={15} />
              <span style={{ flexGrow: 1 }}>Search or ask</span>
              <span className="kbd">Ctrl K</span>
            </div>
            <div className="btn-primary">
              <ComposeIcon size={15} />
              Compose
            </div>
          </div>

          {showSettings ? (
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
