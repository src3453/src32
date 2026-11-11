//! Raw bindings of libfnd, the WAL "fnd" 1.3 (`foundation/include/foundation/fnd.h`,
//! exports in `foundation/lib/fnd/fnd.exports`, `doc/spec_Foundation_API.md` §11).
//!
//! A program imports these from `/lib/fnd.1.wal`: build it with
//! `cargo_src32.py --foundation-wal <OUT>/root/lib/fnd.1.wal`. Only u32/i32 and
//! pointers cross the WAL boundary (`doc/spec_WAX_v3.md` §6), so every
//! signature here uses just those.
#![allow(non_camel_case_types)]

use core::ffi::c_char;

pub type fnd_status = i32;
pub type fnd_handle = u32;

pub const FND_OK: i32 = 0;
pub const FND_END: i32 = 1;

pub const FND_OPEN_READ: u32 = 0x01;
pub const FND_OPEN_WRITE: u32 = 0x02;
pub const FND_OPEN_CREATE: u32 = 0x04;
pub const FND_OPEN_EXCL: u32 = 0x08;
pub const FND_OPEN_TRUNC: u32 = 0x10;
pub const FND_OPEN_APPEND: u32 = 0x20;
pub const FND_OPEN_EXEC: u32 = 0x40;
pub const FND_MODE_DEFAULT: u32 = 0xFFFF_FFFF;

pub const FND_CON_LINE: u32 = 0;
pub const FND_CON_RAW: u32 = 1;

pub const FND_KEY_ENTER: u32 = 1;
pub const FND_KEY_BACKSPACE: u32 = 2;
pub const FND_KEY_DELETE: u32 = 3;
pub const FND_KEY_LEFT: u32 = 4;
pub const FND_KEY_RIGHT: u32 = 5;
pub const FND_KEY_UP: u32 = 6;
pub const FND_KEY_DOWN: u32 = 7;
pub const FND_KEY_HOME: u32 = 8;
pub const FND_KEY_END: u32 = 9;
pub const FND_KEY_TAB: u32 = 10;
pub const FND_KEY_ESC: u32 = 11;
pub const FND_KEY_CHAR: u32 = 0x100;
pub const FND_MOD_SHIFT: u32 = 1;
pub const FND_MOD_CTRL: u32 = 2;
pub const FND_MOD_ALT: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct fnd_key_t {
    pub code: u32,
    pub ch: u32,
    pub mods: u32,
}

/// §10.1, 64 bytes.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct fnd_stat_t {
    pub r#type: u32,
    pub mode: u32,
    pub uid: u32,
    pub flags: u32,
    pub size_hi: u32,
    pub size_lo: u32,
    pub created_sec_hi: u32,
    pub created_sec_lo: u32,
    pub created_ms: u32,
    pub modified_sec_hi: u32,
    pub modified_sec_lo: u32,
    pub modified_ms: u32,
    pub changed_sec_hi: u32,
    pub changed_sec_lo: u32,
    pub changed_ms: u32,
    pub reserved: u32,
}

/// §11.4, one entry with a NUL-terminated name (1100 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct fnd_dirent_t {
    pub stat: fnd_stat_t,
    pub name_bytes: u32,
    pub name_chars: u32,
    pub name: [u8; 1025],
}

/// §11.6
#[repr(C)]
pub struct fnd_exec_t {
    pub argc: u32,
    pub argv: *const *const c_char,
    pub cwd: *const c_char,
    pub std_in: fnd_handle,
    pub std_out: fnd_handle,
    pub std_err: fnd_handle,
    pub flags: u32,
    pub envp: *const *const c_char,
}

unsafe extern "C" {
    // §11.2
    pub fn fnd_init() -> fnd_status;
    pub fn fnd_argc() -> u32;
    pub fn fnd_argv(i: u32) -> *const c_char;
    pub fn fnd_envc() -> u32;
    pub fn fnd_env(i: u32) -> *const c_char;
    pub fn fnd_std_in() -> fnd_handle;
    pub fn fnd_std_out() -> fnd_handle;
    pub fn fnd_std_err() -> fnd_handle;
    pub fn fnd_console() -> fnd_handle;
    pub fn fnd_fs() -> fnd_handle;
    pub fn fnd_exit(code: i32) -> !;
    // §11.3
    pub fn fnd_file_open(
        path: *const c_char,
        flags: u32,
        mode: u32,
        out: *mut fnd_handle,
    ) -> fnd_status;
    pub fn fnd_read(h: fnd_handle, buf: *mut u8, len: u32, got: *mut u32) -> fnd_status;
    pub fn fnd_write(h: fnd_handle, buf: *const u8, len: u32, put: *mut u32) -> fnd_status;
    pub fn fnd_stream_stat(h: fnd_handle, st: *mut fnd_stat_t) -> fnd_status;
    pub fn fnd_close(h: fnd_handle) -> fnd_status;
    pub fn fnd_sync(h: fnd_handle) -> fnd_status;
    // §11.4
    pub fn fnd_path_stat(path: *const c_char, st: *mut fnd_stat_t) -> fnd_status;
    pub fn fnd_file_delete(path: *const c_char) -> fnd_status;
    pub fn fnd_file_rename(old_path: *const c_char, new_path: *const c_char) -> fnd_status;
    pub fn fnd_dir_make(path: *const c_char, mode: u32) -> fnd_status;
    pub fn fnd_dir_remove(path: *const c_char) -> fnd_status;
    pub fn fnd_dir_open(path: *const c_char, out: *mut fnd_handle) -> fnd_status;
    pub fn fnd_dir_next(d: fnd_handle, ent: *mut fnd_dirent_t) -> fnd_status;
    pub fn fnd_dir_rewind(d: fnd_handle) -> fnd_status;
    pub fn fnd_cwd_set(path: *const c_char) -> fnd_status;
    pub fn fnd_cwd_get(buf: *mut c_char, cap: u32, len: *mut u32) -> fnd_status;
    // §11.5
    pub fn fnd_con_write(text: *const c_char) -> fnd_status;
    pub fn fnd_con_getkey(key: *mut fnd_key_t) -> fnd_status;
    pub fn fnd_con_set_mode(mode: u32) -> fnd_status;
    pub fn fnd_con_pollkey(key: *mut fnd_key_t) -> fnd_status;
    pub fn fnd_con_clear() -> fnd_status;
    pub fn fnd_con_size(cols: *mut u32, rows: *mut u32) -> fnd_status;
    // §11.6
    pub fn fnd_proc_exec(
        path: *const c_char,
        ex: *const fnd_exec_t,
        proc_: *mut fnd_handle,
    ) -> fnd_status;
    pub fn fnd_proc_fork() -> i32;
    pub fn fnd_proc_wait(proc_: fnd_handle, code: *mut i32) -> fnd_status;
    pub fn fnd_proc_kill(proc_: fnd_handle) -> fnd_status;
    // §11.8
    pub fn fnd_mem_alloc(size: u32) -> *mut u8;
    pub fn fnd_mem_free(p: *mut u8);
    // §11.9
    pub fn fnd_strerror(code: fnd_status) -> *const c_char;
}
