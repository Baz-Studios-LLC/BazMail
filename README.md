# BazMail

A cross-platform email client. Windows first, then macOS, iPhone and iPad, then web.

## Shape

```
core/            bazmail-core — the engine. No UI, no platform assumptions.
  model.rs         canonical model (JMAP's, deliberately)
  jmap.rs          JMAP client — the subset we need, hand-rolled
  store.rs         SQLite local mirror
  config.rs        account + credential loading
app/             Tauri desktop shell
  src/             React frontend
  src-tauri/       Tauri host — window + command surface only
design/          design canvas working files (.dc.html artboards)
```

The split matters: `bazmail-core` is where every decision lives, so the same
crate can later be exposed to Swift through UniFFI for the Apple apps and
compiled to WASM for the web client. Tauri is just its first host, and the Rust
in `app/src-tauri` is deliberately thin enough to throw away.

## Running it

```bash
./run.cmd
```

Works from PowerShell or Explorer. Or directly, if you prefer:

```bash
cd app && npm run tauri dev
```

Cold Rust builds take a few minutes. After that, frontend changes hot-reload and
only Rust changes trigger a rebuild.

## Connecting an account

Press **Connect Fastmail**. Your browser opens, you approve, done.

BazMail registers itself with Fastmail dynamically (RFC 7591) the first time, so
there is no developer sign-up and no client secret — which matters, because a
secret shipped inside a desktop app is not a secret. The flow uses PKCE with
S256 and a loopback redirect on a random port (RFC 8252), never a custom URL
scheme that another app on the machine could claim.

It asks for `mail` and `offline_access` only — not contacts, not calendars — and
you can revoke it from Fastmail's own settings, which is the main thing an API
token cannot give you.

What gets stored is the **refresh token**, in the Windows Credential Manager (the
Keychain on macOS and iOS). Access tokens are short-lived and never written to
disk. `config.json` holds only non-secret metadata: id, label, colour, identity,
session URL, and the OAuth client id.

**Using an API token instead** is still available behind a link on the sign-in
screen, for a machine where the browser round trip cannot complete. Create one at
**Settings → Privacy & Security → Integrations** with read and write access to
Mail. Environment variables named in an account's `tokenEnv` still take
precedence over everything, which is useful for CI.

## What works

- OAuth sign-in with PKCE and dynamic client registration; refresh token in the
  OS credential store, access tokens in memory only
- **IMAP accounts, including iCloud** — app-specific password over TLS, mailbox
  roles from RFC 6154 special-use with a name-based fallback, bodies parsed from
  raw RFC 5322, and archive via UID MOVE
- API-token sign-in as a fallback, verified before saving
- JMAP connection to Fastmail, mailbox and message sync
- SQLite mirror; every read in the UI comes from disk, not the network
- Unified inbox across configured accounts, newest first
- Per-account mailbox navigation
- Message bodies in a sandboxed iframe with remote images blocked, so tracking
  pixels do not fire
- **Archive with an outbox.** `e` archives the selected thread and advances to
  the next; `z` undoes. The mirror is written and the change queued before
  anything touches the network, so the row leaves the list on the same frame as
  the keypress. A failed send keeps its place in the queue with the error
  recorded and is retried at startup — it is never silently dropped, and the UI
  is not rolled back, because the local state is what was asked for.
- `j` / `k` to move through the list, `Esc` to deselect

## What does not

- **Reply, snooze and compose** are still unbound. The outbox behind archive is
  the shape they all need, so each is now a much smaller job than the first one
  was.
- **No triage lanes yet.** Needs you / Waiting on / FYI are designed but not
  built, so the sidebar shows only what actually works.
- **No push.** Sync runs once at startup. Real-time needs the JMAP EventSource
  connection and the sync service.
- **No search.**
- **IMAP threading.** Every IMAP message is currently its own thread. IMAP has
  no server-side threading, so this needs the JWZ algorithm over
  `References`/`In-Reply-To`. JMAP accounts thread correctly already.
- **IMAP mailbox counts.** The sidebar shows no unread counts for IMAP accounts:
  `LIST` does not carry them and fetching them means a `STATUS` round trip per
  mailbox.
- **At-rest encryption is Windows-only, and best-effort even there.**
  `bazmail.db` holds every synced message. The app asks Windows to encrypt it
  (EFS) and *verifies the attribute afterwards* rather than trusting the return
  value, because EFS can be restricted such that both `EncryptFileW` and
  `cipher /e` report success while encrypting nothing. Where that happens you
  get a warning on stderr at startup and an honest "No" in Settings.

  On macOS there is no per-file step: FileVault covers the volume. BazMail
  neither performs nor verifies that, and Settings says so rather than claiming
  credit for protection it did not provide.

  The real fix is SQLCipher, which keeps FTS working because SQLite decrypts
  pages transparently — encrypting columns ourselves would break search, which
  is the point of the local mirror. It needs `rusqlite`'s
  `bundled-sqlcipher-vendored-openssl` feature, and on Windows that needs a
  native Strawberry Perl and NASM on PATH; the Perl that ships with Git Bash is
  Cygwin's and will not build OpenSSL for MSVC.

  Check the current state with:

  ```bash
  cargo run -p bazmail-core --example config_check
  ```

## Builds

Tauri cannot cross-compile to macOS — the bundle needs Apple's own toolchain —
so builds run in GitHub Actions on their native platforms.

- **A build now:** Actions → Build → *Run workflow*. Installers appear as run
  artifacts.
- **A release:** push a tag. Same build, plus a draft release with the
  installers attached.

  ```bash
  git tag v0.1.0 && git push origin v0.1.0
  ```

Windows gets an `.msi` and an NSIS `.exe`. macOS gets a universal `.dmg` that
runs natively on both Apple Silicon and Intel, so there is nothing to choose
between.

### Nothing is code-signed yet

So both systems will object, and they are right to:

- **Windows** — SmartScreen warns about an unknown publisher. *More info → Run
  anyway.*
- **macOS** — Gatekeeper refuses an unsigned app downloaded from the internet.
  Right-click → *Open*, or clear the quarantine flag:

  ```bash
  xattr -dr com.apple.quarantine /Applications/BazMail.app
  ```

Signing means an Apple Developer account for notarization and a Windows
certificate. Worth doing before anyone else installs this, unnecessary while it
is just us. One consequence in the meantime: an unsigned macOS app has an
unstable code identity, so the Keychain may re-prompt after each rebuild — which
this app leans on more than most.

## Tests

```bash
cargo test --workspace
```
