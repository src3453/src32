//! Files and directories (`doc/spec_Foundation_API.md` §11.3, §11.4).
//! Paths are C strings: write them as `c"/dir/name"` or take them from
//! [`crate::arg_c`].

use core::ffi::CStr;

use crate::{Result, check, io, sys};

pub use sys::{
    FND_OPEN_APPEND, FND_OPEN_CREATE, FND_OPEN_EXCL, FND_OPEN_READ, FND_OPEN_TRUNC, FND_OPEN_WRITE,
};

/// An open file; closed when dropped.
pub struct File {
    handle: sys::fnd_handle,
}

impl File {
    /// `fnd_file_open` with `FND_OPEN_*` flags and the default mode.
    pub fn open(path: &CStr, flags: u32) -> Result<File> {
        let mut handle = 0;
        check(unsafe {
            sys::fnd_file_open(path.as_ptr(), flags, sys::FND_MODE_DEFAULT, &mut handle)
        })?;
        Ok(File { handle })
    }

    /// Creates or truncates `path` for writing.
    pub fn create(path: &CStr) -> Result<File> {
        File::open(path, FND_OPEN_WRITE | FND_OPEN_CREATE | FND_OPEN_TRUNC)
    }

    pub fn handle(&self) -> sys::fnd_handle {
        self.handle
    }

    /// Up to `buf.len()` bytes; `Ok(0)` at the end of the file.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        io::read(self.handle, buf)
    }

    pub fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        io::write_all(self.handle, bytes)
    }

    pub fn stat(&self) -> Result<sys::fnd_stat_t> {
        let mut st = sys::fnd_stat_t::default();
        check(unsafe { sys::fnd_stream_stat(self.handle, &mut st) })?;
        Ok(st)
    }

    pub fn sync(&mut self) -> Result<()> {
        check(unsafe { sys::fnd_sync(self.handle) }).map(drop)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        unsafe { sys::fnd_close(self.handle) };
    }
}

impl core::fmt::Write for File {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        self.write_all(text.as_bytes())
            .map_err(|_| core::fmt::Error)
    }
}

pub fn stat(path: &CStr) -> Result<sys::fnd_stat_t> {
    let mut st = sys::fnd_stat_t::default();
    check(unsafe { sys::fnd_path_stat(path.as_ptr(), &mut st) })?;
    Ok(st)
}

pub fn remove_file(path: &CStr) -> Result<()> {
    check(unsafe { sys::fnd_file_delete(path.as_ptr()) }).map(drop)
}

pub fn rename(old: &CStr, new: &CStr) -> Result<()> {
    check(unsafe { sys::fnd_file_rename(old.as_ptr(), new.as_ptr()) }).map(drop)
}

pub fn create_dir(path: &CStr) -> Result<()> {
    check(unsafe { sys::fnd_dir_make(path.as_ptr(), sys::FND_MODE_DEFAULT) }).map(drop)
}

pub fn remove_dir(path: &CStr) -> Result<()> {
    check(unsafe { sys::fnd_dir_remove(path.as_ptr()) }).map(drop)
}

pub fn set_current_dir(path: &CStr) -> Result<()> {
    check(unsafe { sys::fnd_cwd_set(path.as_ptr()) }).map(drop)
}

/// The current directory, NUL-terminated in `buf`; returns its text.
pub fn current_dir(buf: &mut [u8]) -> Result<&str> {
    let mut len = 0u32;
    check(unsafe { sys::fnd_cwd_get(buf.as_mut_ptr().cast(), buf.len() as u32, &mut len) })?;
    core::str::from_utf8(&buf[..len as usize]).map_err(|_| crate::Error::BAD_ARG)
}

/// A directory listing (`fnd_dir_open` / `fnd_dir_next`); closed when dropped.
pub struct ReadDir {
    handle: sys::fnd_handle,
    entry: sys::fnd_dirent_t,
}

impl ReadDir {
    pub fn open(path: &CStr) -> Result<ReadDir> {
        let mut handle = 0;
        check(unsafe { sys::fnd_dir_open(path.as_ptr(), &mut handle) })?;
        let entry = sys::fnd_dirent_t {
            stat: Default::default(),
            name_bytes: 0,
            name_chars: 0,
            name: [0; 1025],
        };
        Ok(ReadDir { handle, entry })
    }

    /// The next entry (valid until the following call), or `None` at the end.
    pub fn next_entry(&mut self) -> Result<Option<(&str, &sys::fnd_stat_t)>> {
        if check(unsafe { sys::fnd_dir_next(self.handle, &mut self.entry) })? == sys::FND_END {
            return Ok(None);
        }
        let name = &self.entry.name[..self.entry.name_bytes as usize];
        let name = core::str::from_utf8(name).map_err(|_| crate::Error::BAD_ARG)?;
        Ok(Some((name, &self.entry.stat)))
    }
}

impl Drop for ReadDir {
    fn drop(&mut self) {
        unsafe { sys::fnd_close(self.handle) };
    }
}
