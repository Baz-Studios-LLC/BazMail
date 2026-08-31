import { useCallback, useEffect, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * How often to look for a new release.
 *
 * Far more often than releases actually appear, but it is one small request
 * against a static manifest, and the polling stops for good once something is
 * downloaded and waiting.
 */
const CHECK_INTERVAL_MS = 5 * 60 * 1000;

/**
 * macOS replaces the whole `.app` bundle, which is safe to do while the app is
 * running: the process keeps executing the copy already mapped into memory and
 * the next launch picks up the new one. That is exactly Chrome's behaviour, and
 * it means the update can be applied the moment it finishes downloading.
 *
 * Windows cannot: its installer has to replace a locked executable, so the app
 * must exit first and the install is deferred to the close handler.
 */
const IS_MAC = navigator.userAgent.includes("Macintosh");

export type UpdateState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "downloading"; version: string }
  /** Downloaded, waiting for the app to quit. Windows. */
  | { kind: "ready"; version: string }
  /** Already swapped in; the next launch runs it. macOS. */
  | { kind: "installed"; version: string }
  | { kind: "current" }
  | { kind: "error"; message: string };

/**
 * Keeps the app up to date without ever interrupting it.
 *
 * Silent means "does not interrupt you", not "refuses to say what happened" —
 * an earlier version swallowed every error, which made a failure that only
 * showed up on one platform impossible to diagnose from the outside. Nothing
 * here pops a dialog, but everything it does is reportable.
 */
export function useUpdater() {
  const [state, setState] = useState<UpdateState>({ kind: "idle" });
  // Held in a ref rather than state: the close handler needs the current value,
  // and a stale closure over it would silently skip the install.
  const pending = useRef<Update | null>(null);
  const busy = useRef(false);

  const run = useCallback(async () => {
    // Once something is downloaded there is nothing left to find, and an
    // in-flight check must not stack up behind a slow download.
    if (busy.current || pending.current) return;
    busy.current = true;
    setState({ kind: "checking" });

    try {
      const update = await check();
      if (!update) {
        setState({ kind: "current" });
        return;
      }

      setState({ kind: "downloading", version: update.version });
      await update.download();

      if (IS_MAC) {
        // Safe while running, so there is nothing to wait for.
        await update.install();
        setState({ kind: "installed", version: update.version });
        return;
      }

      pending.current = update;
      setState({ kind: "ready", version: update.version });
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    } finally {
      busy.current = false;
    }
  }, []);

  useEffect(() => {
    void run();
    const timer = setInterval(() => void run(), CHECK_INTERVAL_MS);

    // Registered only where it can actually do something. On macOS the install
    // already happened at download time, so the handler could only ever no-op —
    // and registering one is not free: Tauri routes the close through
    // window.destroy() as soon as any listener exists, which turned the red
    // traffic light into a no-op when that permission was missing. A listener
    // that cannot act is a liability, not a spare part.
    const stopping = IS_MAC
      ? null
      : getCurrentWindow().onCloseRequested(async (event) => {
      const update = pending.current;
      if (!update) return;
      event.preventDefault();

      // Cleared before installing rather than after: if the installer fails,
      // the next close has to actually close instead of trapping the window.
      pending.current = null;
      try {
        await update.install();
      } catch {
        // Nothing useful to do at exit; the release is still there next launch.
      }
      void getCurrentWindow().destroy();
    });

    return () => {
      clearInterval(timer);
      void stopping?.then((stop) => stop());
    };
  }, [run]);

  return { state, checkNow: run };
}

/** One line describing where an update has got to, for the settings panel. */
export function describeUpdate(state: UpdateState, version: string): string {
  switch (state.kind) {
    case "checking":
      return "Checking…";
    case "downloading":
      return `Downloading ${state.version}…`;
    case "ready":
      return `${state.version} is ready — it installs when you quit BazMail.`;
    case "installed":
      return `${state.version} is installed — restart BazMail to use it.`;
    case "current":
      return `Up to date (${version}).`;
    case "error":
      return state.message;
    case "idle":
      return `Version ${version}.`;
  }
}
