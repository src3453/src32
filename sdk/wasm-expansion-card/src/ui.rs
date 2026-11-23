//! Debug-GUI panel widgets (`doc/spec_WASM_expansion_card.md` §3.4).
//!
//! Requires `capabilities: [gui]` in `manifest.yaml`. Call these only from
//! [`crate::Card::ui`]; the host traps calls made elsewhere. Input arrives one
//! GUI frame late: a click in frame N is returned by the same widget call in
//! frame N+1. Widget IDs must be unique within one panel.

use core::fmt;

/// Longest label or text value in bytes; longer strings are cut at a char
/// boundary before calling the host.
pub const MAX_TEXT: usize = 256;

#[link(wasm_import_module = "cpt32_host")]
unsafe extern "C" {
    #[link_name = "ui_text"]
    fn host_ui_text(ptr: i32, len: i32);
    #[link_name = "ui_text_colored"]
    fn host_ui_text_colored(rgba: i32, ptr: i32, len: i32);
    #[link_name = "ui_separator"]
    fn host_ui_separator();
    #[link_name = "ui_same_line"]
    fn host_ui_same_line();
    #[link_name = "ui_button"]
    fn host_ui_button(id: i32, ptr: i32, len: i32) -> i32;
    #[link_name = "ui_checkbox"]
    fn host_ui_checkbox(id: i32, ptr: i32, len: i32, value: i32) -> i32;
    #[link_name = "ui_input_int"]
    fn host_ui_input_int(id: i32, ptr: i32, len: i32, value: i32, min: i32, max: i32) -> i32;
    #[link_name = "ui_input_text"]
    fn host_ui_input_text(
        id: i32,
        label_ptr: i32,
        label_len: i32,
        buf_ptr: i32,
        cap: i32,
        len: i32,
    ) -> i32;
}

fn clip(text: &str) -> (i32, i32) {
    let mut end = text.len().min(MAX_TEXT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text.as_ptr() as usize as i32, end as i32)
}

/// One line of text.
pub fn text(text: &str) {
    let (ptr, len) = clip(text);
    // SAFETY: the string lies in linear memory and is at most MAX_TEXT bytes.
    unsafe { host_ui_text(ptr, len) }
}

/// One line of text in `rgba` (`0xRRGGBBAA`).
pub fn text_colored(rgba: u32, text: &str) {
    let (ptr, len) = clip(text);
    // SAFETY: as in `text`.
    unsafe { host_ui_text_colored(rgba as i32, ptr, len) }
}

/// A horizontal separator.
pub fn separator() {
    // SAFETY: takes no arguments.
    unsafe { host_ui_separator() }
}

/// Place the next widget on the same line as the previous one.
pub fn same_line() {
    // SAFETY: takes no arguments.
    unsafe { host_ui_same_line() }
}

/// A button; true when it was pressed since the previous panel frame.
pub fn button(id: i32, label: &str) -> bool {
    let (ptr, len) = clip(label);
    // SAFETY: as in `text`.
    unsafe { host_ui_button(id, ptr, len) == 1 }
}

/// A checkbox showing `*value`; returns true and updates it when the user
/// toggled it.
pub fn checkbox(id: i32, label: &str, value: &mut bool) -> bool {
    let (ptr, len) = clip(label);
    // SAFETY: as in `text`; the value is 0 or 1.
    let new = unsafe { host_ui_checkbox(id, ptr, len, i32::from(*value)) } == 1;
    let changed = new != *value;
    *value = new;
    changed
}

/// An integer field limited to `min..=max` (`min <= max`, or the call
/// panics). Returns true and updates `*value` when the shown value changed,
/// including clamping into the range.
pub fn input_int(id: i32, label: &str, value: &mut i32, min: i32, max: i32) -> bool {
    assert!(min <= max);
    let (ptr, len) = clip(label);
    // SAFETY: as in `text`; min <= max was checked.
    let new = unsafe { host_ui_input_int(id, ptr, len, *value, min, max) };
    let changed = new != *value;
    *value = new;
    changed
}

/// A text field showing `buffer[..*len]` (UTF-8, at most [`MAX_TEXT`] bytes
/// of `buffer` are used). When the user pressed Enter, the submitted text is
/// written to `buffer`, `*len` is updated and the call returns true.
pub fn input_text(id: i32, label: &str, buffer: &mut [u8], len: &mut usize) -> bool {
    let cap = buffer.len().min(MAX_TEXT);
    assert!(*len <= cap);
    let (label_ptr, label_len) = clip(label);
    // SAFETY: the label and `buffer[..cap]` lie in linear memory; len <= cap <= MAX_TEXT.
    let result = unsafe {
        host_ui_input_text(
            id,
            label_ptr,
            label_len,
            buffer.as_mut_ptr() as usize as i32,
            cap as i32,
            *len as i32,
        )
    };
    match usize::try_from(result) {
        Ok(new) => {
            *len = new;
            true
        }
        Err(_) => false,
    }
}

/// Fixed-capacity text for formatting labels without an allocator. Output
/// past [`MAX_TEXT`] bytes is dropped at a char boundary.
pub struct TextBuf {
    bytes: [u8; MAX_TEXT],
    len: usize,
}

impl TextBuf {
    pub const fn new() -> Self {
        Self {
            bytes: [0; MAX_TEXT],
            len: 0,
        }
    }

    pub fn as_str(&self) -> &str {
        // SAFETY: only whole UTF-8 chars are appended in `write_str`.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len]) }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }
}

impl Default for TextBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Write for TextBuf {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let room = MAX_TEXT - self.len;
        let mut end = text.len().min(room);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.bytes[self.len..self.len + end].copy_from_slice(&text.as_bytes()[..end]);
        self.len += end;
        Ok(())
    }
}

/// Format `args` into a [`TextBuf`].
pub fn format(args: fmt::Arguments<'_>) -> TextBuf {
    let mut buffer = TextBuf::new();
    let _ = fmt::write(&mut buffer, args);
    buffer
}

/// `ui_text!("{} bytes", n)` shows one formatted line.
#[macro_export]
macro_rules! ui_text {
    ($($arg:tt)*) => {
        $crate::ui::text($crate::ui::format(core::format_args!($($arg)*)).as_str())
    };
}

/// `ui_text_colored!(0x40FF40FF, "{}", state)` shows one formatted colored line.
#[macro_export]
macro_rules! ui_text_colored {
    ($rgba:expr, $($arg:tt)*) => {
        $crate::ui::text_colored($rgba, $crate::ui::format(core::format_args!($($arg)*)).as_str())
    };
}
