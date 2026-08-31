import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import type { Account, Envelope } from "./types";
import { decodeEntities, displayName } from "./types";

/** Remembered so the app is not asked to notify before anyone said it could. */
export const NOTIFY_KEY = "bazmail.notify";

export function notificationsEnabled(): boolean {
  // Default on: a mail client that never tells you mail arrived is a strange
  // thing. Off is one checkbox away.
  return localStorage.getItem(NOTIFY_KEY) !== "false";
}

/**
 * Asks the OS for permission, once, and only when it would be used.
 *
 * Requesting at launch is how apps train people to say no — the prompt arrives
 * before there is anything to permit. This runs the first time mail actually
 * arrives, when the request explains itself.
 */
async function permitted(): Promise<boolean> {
  try {
    if (await isPermissionGranted()) return true;
    return (await requestPermission()) === "granted";
  } catch {
    // No notification service, or a platform that refuses. Not worth surfacing:
    // mail still arrives, it just arrives quietly.
    return false;
  }
}

/**
 * Announces new mail.
 *
 * One notification for a single message so it can say who it is from, and one
 * summary beyond that — a stack of twelve toasts after a sync is noise, and the
 * useful information at that point is the number, not each sender.
 */
export async function announce(fresh: Envelope[], accounts: Account[]): Promise<void> {
  if (fresh.length === 0 || !notificationsEnabled()) return;
  if (!(await permitted())) return;

  if (fresh.length === 1) {
    const message = fresh[0];
    const account = accounts.find((a) => a.id === message.accountId);
    sendNotification({
      title: displayName(message.from),
      body: [decodeEntities(message.subject) || "(no subject)", account?.label]
        .filter(Boolean)
        .join(" · "),
    });
    return;
  }

  const senders = [...new Set(fresh.map((m) => displayName(m.from)))];
  sendNotification({
    title: `${fresh.length} new messages`,
    body:
      senders.length <= 3
        ? senders.join(", ")
        : `${senders.slice(0, 3).join(", ")} and ${senders.length - 3} more`,
  });
}
