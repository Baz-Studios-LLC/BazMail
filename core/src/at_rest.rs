//! At-rest protection for the local mirror.
//!
//! The store is a full copy of your mail sitting in a plain SQLite file. The
//! Fastmail token is already safe — the OS credential store encrypts it per-user
//! — but the *mail* was not, and locking the app's window would not have changed
//! that: anyone able to read the file could open it in any SQLite browser.
//!
//! We use the platform's own file encryption rather than SQLCipher. Both put the
//! key behind the same thing in practice — the user's OS login — but SQLCipher
//! would mean building OpenSSL, and the platform mechanism is transparent to
//! SQLite, so FTS keeps working. Encrypting individual columns ourselves would
//! have broken search outright, which is the one thing the local mirror exists
//! to make fast.
//!
//! This is best-effort by design: a system that cannot do it is not a reason to
//! refuse to run. But it is verified rather than assumed — EFS can be restricted
//! so that both `EncryptFileW` and `cipher /e` report success while encrypting
//! nothing, and silently believing them would be worse than not trying, because
//! the mail would sit in the clear while we reported it protected.

use anyhow::Result;
use serde::Serialize;
use std::path::Path;

/// How the local mirror is protected at rest, and who is responsible for it.
///
/// A boolean could not tell "we asked for encryption and it did not happen"
/// apart from "this platform protects the whole volume, so there is nothing
/// per-file for us to do" — and those two deserve opposite words in the
/// settings panel. Collapsing them is not cosmetic: it would show a macOS user
/// a warning about Windows refusing a request that was never made there.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Protection {
    /// We marked the file for encryption and the filesystem confirms the
    /// attribute is set.
    Encrypted,
    /// We asked and the attribute is not set: the mail is on disk in the clear.
    Refused,
    /// The platform encrypts at the volume level instead. We neither perform
    /// nor verify that, and say so rather than taking credit for it.
    Volume,
}

/// What is actually protecting the store on this platform.
pub fn protection(path: &Path) -> Protection {
    #[cfg(windows)]
    {
        if is_encrypted(path) {
            Protection::Encrypted
        } else {
            Protection::Refused
        }
    }

    // FileVault covers the Mac case at the volume level and iOS files land
    // under Data Protection. Neither is ours to claim — reporting `Refused`
    // here would be a lie in the alarming direction, and `Encrypted` a lie in
    // the reassuring one.
    #[cfg(not(windows))]
    {
        let _ = path;
        Protection::Volume
    }
}

/// Marks a path for encryption at rest.
///
/// Applied to the *directory*, new files created inside it inherit the flag —
/// which is what covers SQLite's `-wal` and `-shm` sidecars without having to
/// know when they appear.
#[cfg(windows)]
pub fn protect(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::EncryptFileW;

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: the pointer is a NUL-terminated wide string that outlives the call.
    let ok = unsafe { EncryptFileW(wide.as_ptr()) };
    if ok == 0 {
        let err = std::io::Error::last_os_error();
        anyhow::bail!("could not encrypt {}: {err}", path.display());
    }

    // Trust the attribute, not the return value. EFS can be restricted such that
    // both this call and `cipher /e` report success and encrypt nothing — which
    // is the worst possible outcome, because the caller would otherwise believe
    // the mail on disk is protected when it is in the clear.
    if !is_encrypted(path) {
        anyhow::bail!(
            "{} reported encrypted but the attribute is not set — EFS is refused on this system",
            path.display()
        );
    }
    Ok(())
}

/// Whether the filesystem actually marked the path encrypted.
#[cfg(windows)]
pub fn is_encrypted(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_ENCRYPTED: u32 = 0x4000;
    std::fs::metadata(path)
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_ENCRYPTED != 0)
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn is_encrypted(_path: &Path) -> bool {
    false
}

/// Other platforms get this when their client is built: FileVault covers the Mac
/// case at the volume level, and iOS files land under Data Protection. Neither
/// needs anything from us, so this is deliberately a no-op rather than a stub
/// that pretends to do something.
#[cfg(not(windows))]
pub fn protect(_path: &Path) -> Result<()> {
    Ok(())
}

/// Whether a file is a plain, unencrypted SQLite database.
///
/// SQLite writes a fixed 16-byte magic at the head of every database. Checking
/// for it is how we tell a legacy plaintext store from one already protected,
/// without guessing from a failed open.
pub fn is_plaintext_sqlite(path: &Path) -> bool {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 16];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    &header == b"SQLite format 3\0"
}
