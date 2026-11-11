//! The console (`doc/spec_Foundation_API.md` §11.5).

use crate::{Result, check, sys};

pub use sys::fnd_key_t as Key;

/// Waits for a key.
pub fn get_key() -> Result<Key> {
    let mut key = Key::default();
    check(unsafe { sys::fnd_con_getkey(&mut key) })?;
    Ok(key)
}

/// A key if one is waiting.
pub fn poll_key() -> Result<Option<Key>> {
    let mut key = Key::default();
    Ok((check(unsafe { sys::fnd_con_pollkey(&mut key) })? != sys::FND_END).then_some(key))
}

/// `FND_CON_LINE` or `FND_CON_RAW`.
pub fn set_mode(mode: u32) -> Result<()> {
    check(unsafe { sys::fnd_con_set_mode(mode) }).map(drop)
}

pub fn clear() -> Result<()> {
    check(unsafe { sys::fnd_con_clear() }).map(drop)
}

/// (columns, rows)
pub fn size() -> Result<(u32, u32)> {
    let (mut cols, mut rows) = (0, 0);
    check(unsafe { sys::fnd_con_size(&mut cols, &mut rows) })?;
    Ok((cols, rows))
}
