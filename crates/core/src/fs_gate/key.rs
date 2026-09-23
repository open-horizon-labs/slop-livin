//! The store's authority key: 32 random bytes that bind a stored plan or
//! grant to the swamp code that wrote it (`actions`, `authority`).
//!
//! A plan or grant on disk carries a keyed blake3 MAC over its canonical
//! JSON; the loader recomputes it and refuses a record that does not
//! match, so a plan edited after approval, a grant widened by hand, or a
//! file written by anything but swamp's propose/approve paths never
//! reaches `authority::authorize`. Its own capability group: only
//! `actions`, `authority` and `recheck` may name it.
//!
//! **Limit, stated:** the key is a file in the store, readable by the
//! user swamp runs as. A process with that user's filesystem access and
//! the will to reimplement the MAC can forge a record; what the key rules
//! out is every path that does not go through swamp's own code -- a hand
//! edit, a JSON writer, a copied plan from another store.

use super::store::StoreDir;
use std::io::{self, Write};

/// The store's authority key (`<store>/authority.key`, 32 random bytes,
/// mode 0600): what binds a plan or grant to the swamp code that wrote
/// it (`authority`). Created on first use with `O_EXCL`, so two racing
/// writers agree on one key.
pub fn authority_key(store: &StoreDir) -> io::Result<[u8; 32]> {
    let path = store.path().join("authority.key");
    match std::fs::read(&path) {
        Ok(bytes) => {
            return bytes.try_into().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} is not a 32-byte swamp authority key", path.display()),
                )
            });
        }
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        Err(_) => {}
    }
    store.create()?;
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    match opts.open(&path) {
        Ok(mut f) => {
            f.write_all(&key)?;
            f.sync_all()?;
            Ok(key)
        }
        // Another writer created it first: use theirs.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => authority_key(store),
        Err(e) => Err(e),
    }
}

/// Whether `store` holds an authority key: a store swamp has granted or
/// planned in. Never creates one.
pub fn has_authority_key(store: &StoreDir) -> bool {
    std::fs::symlink_metadata(store.path().join("authority.key")).is_ok_and(|m| m.is_file())
}
