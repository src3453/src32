//! GUI panel extension (`doc/spec_WASM_expansion_card.md` §3.4).
//!
//! A `gui` card describes its panel from `cpt32_ui` with `ui_*` imports
//! (immediate mode). The host records the widgets, the debug GUI draws them,
//! and user input is queued per widget ID and returned by the same widget
//! import on the next `cpt32_ui` call.

use std::collections::{HashMap, HashSet};

use wasmtime::{Caller, Linker};

use super::{HostState, guest_memory};

/// Longest label or text value in bytes.
pub const UI_MAX_TEXT: usize = 256;
/// Widgets one `cpt32_ui` call may describe.
pub const UI_MAX_ITEMS: usize = 512;

/// One widget described by a card, in drawing order.
#[derive(Clone, Debug, PartialEq)]
pub enum ExpansionUiItem {
    Text {
        text: String,
        /// RGBA in 0.0..=1.0, from the `0xRRGGBBAA` bit pattern.
        color: Option<[f32; 4]>,
    },
    Separator,
    SameLine,
    Button {
        id: i32,
        label: String,
    },
    Checkbox {
        id: i32,
        label: String,
        value: bool,
    },
    InputInt {
        id: i32,
        label: String,
        value: i32,
        min: i32,
        max: i32,
    },
    InputText {
        id: i32,
        label: String,
        value: String,
        capacity: usize,
    },
}

/// User input for one widget, delivered on the next `cpt32_ui` call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpansionUiEvent {
    Clicked,
    Checkbox(bool),
    Int(i32),
    /// Text submitted with Enter.
    Text(String),
}

#[derive(Default)]
pub(super) struct UiHost {
    /// Set only while `cpt32_ui` runs.
    active: bool,
    items: Vec<ExpansionUiItem>,
    ids: HashSet<i32>,
    events: HashMap<i32, ExpansionUiEvent>,
}

impl UiHost {
    pub(super) fn begin(&mut self) {
        self.active = true;
        self.items.clear();
        self.ids.clear();
    }

    /// End a call: return the widgets and drop undelivered events.
    pub(super) fn finish(&mut self) -> Vec<ExpansionUiItem> {
        self.active = false;
        self.ids.clear();
        self.events.clear();
        std::mem::take(&mut self.items)
    }

    pub(super) fn queue(&mut self, id: i32, event: ExpansionUiEvent) {
        self.events.insert(id, event);
    }

    pub(super) fn clear_events(&mut self) {
        self.events.clear();
    }
}

fn check_active(caller: &Caller<'_, HostState>) -> wasmtime::Result<()> {
    if caller.data().ui.active {
        Ok(())
    } else {
        Err(wasmtime::Error::msg(
            "ui imports may only be called from cpt32_ui",
        ))
    }
}

fn read_bytes(
    caller: &mut Caller<'_, HostState>,
    ptr: i32,
    len: i32,
    limit: usize,
) -> wasmtime::Result<Vec<u8>> {
    let len = usize::try_from(len)
        .ok()
        .filter(|&len| len <= limit)
        .ok_or_else(|| wasmtime::Error::msg("ui text length out of range"))?;
    let memory = guest_memory(caller)?;
    let mut bytes = vec![0u8; len];
    memory
        .read(&*caller, ptr as u32 as usize, &mut bytes)
        .map_err(|_| wasmtime::Error::msg("ui text pointer out of range"))?;
    Ok(bytes)
}

fn read_text(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> wasmtime::Result<String> {
    let bytes = read_bytes(caller, ptr, len, UI_MAX_TEXT)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn push(caller: &mut Caller<'_, HostState>, item: ExpansionUiItem) -> wasmtime::Result<()> {
    let ui = &mut caller.data_mut().ui;
    if ui.items.len() >= UI_MAX_ITEMS {
        return Err(wasmtime::Error::msg(
            "too many ui widgets in one cpt32_ui call",
        ));
    }
    ui.items.push(item);
    Ok(())
}

/// Register an input widget ID and take its pending event.
fn claim(
    caller: &mut Caller<'_, HostState>,
    id: i32,
) -> wasmtime::Result<Option<ExpansionUiEvent>> {
    let ui = &mut caller.data_mut().ui;
    if !ui.ids.insert(id) {
        return Err(wasmtime::Error::msg(format!("duplicate ui widget id {id}")));
    }
    Ok(ui.events.remove(&id))
}

fn rgba(color: i32) -> [f32; 4] {
    color
        .to_be_bytes()
        .map(|channel| f32::from(channel) / 255.0)
}

/// Longest prefix of `text` within `cap` bytes that ends on a char boundary.
fn truncate(text: &str, cap: usize) -> &str {
    if text.len() <= cap {
        return text;
    }
    let mut end = cap;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub(super) fn add_imports(linker: &mut Linker<HostState>) -> wasmtime::Result<()> {
    linker.func_wrap(
        "cpt32_host",
        "ui_text",
        |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> wasmtime::Result<()> {
            check_active(&caller)?;
            let text = read_text(&mut caller, ptr, len)?;
            push(&mut caller, ExpansionUiItem::Text { text, color: None })
        },
    )?;
    linker.func_wrap(
        "cpt32_host",
        "ui_text_colored",
        |mut caller: Caller<'_, HostState>,
         color: i32,
         ptr: i32,
         len: i32|
         -> wasmtime::Result<()> {
            check_active(&caller)?;
            let text = read_text(&mut caller, ptr, len)?;
            push(
                &mut caller,
                ExpansionUiItem::Text {
                    text,
                    color: Some(rgba(color)),
                },
            )
        },
    )?;
    linker.func_wrap(
        "cpt32_host",
        "ui_separator",
        |mut caller: Caller<'_, HostState>| -> wasmtime::Result<()> {
            check_active(&caller)?;
            push(&mut caller, ExpansionUiItem::Separator)
        },
    )?;
    linker.func_wrap(
        "cpt32_host",
        "ui_same_line",
        |mut caller: Caller<'_, HostState>| -> wasmtime::Result<()> {
            check_active(&caller)?;
            push(&mut caller, ExpansionUiItem::SameLine)
        },
    )?;
    linker.func_wrap(
        "cpt32_host",
        "ui_button",
        |mut caller: Caller<'_, HostState>, id: i32, ptr: i32, len: i32| -> wasmtime::Result<i32> {
            check_active(&caller)?;
            let label = read_text(&mut caller, ptr, len)?;
            let clicked = matches!(claim(&mut caller, id)?, Some(ExpansionUiEvent::Clicked));
            push(&mut caller, ExpansionUiItem::Button { id, label })?;
            Ok(i32::from(clicked))
        },
    )?;
    linker.func_wrap(
        "cpt32_host",
        "ui_checkbox",
        |mut caller: Caller<'_, HostState>,
         id: i32,
         ptr: i32,
         len: i32,
         value: i32|
         -> wasmtime::Result<i32> {
            check_active(&caller)?;
            let value = match value {
                0 => false,
                1 => true,
                _ => return Err(wasmtime::Error::msg("ui_checkbox value must be 0 or 1")),
            };
            let label = read_text(&mut caller, ptr, len)?;
            let value = match claim(&mut caller, id)? {
                Some(ExpansionUiEvent::Checkbox(new)) => new,
                _ => value,
            };
            push(&mut caller, ExpansionUiItem::Checkbox { id, label, value })?;
            Ok(i32::from(value))
        },
    )?;
    linker.func_wrap(
        "cpt32_host",
        "ui_input_int",
        |mut caller: Caller<'_, HostState>,
         id: i32,
         ptr: i32,
         len: i32,
         value: i32,
         min: i32,
         max: i32|
         -> wasmtime::Result<i32> {
            check_active(&caller)?;
            if min > max {
                return Err(wasmtime::Error::msg("ui_input_int min is greater than max"));
            }
            let label = read_text(&mut caller, ptr, len)?;
            let value = match claim(&mut caller, id)? {
                Some(ExpansionUiEvent::Int(new)) => new,
                _ => value,
            }
            .clamp(min, max);
            push(
                &mut caller,
                ExpansionUiItem::InputInt {
                    id,
                    label,
                    value,
                    min,
                    max,
                },
            )?;
            Ok(value)
        },
    )?;
    linker.func_wrap(
        "cpt32_host",
        "ui_input_text",
        |mut caller: Caller<'_, HostState>,
         id: i32,
         label_ptr: i32,
         label_len: i32,
         buf_ptr: i32,
         cap: i32,
         len: i32|
         -> wasmtime::Result<i32> {
            check_active(&caller)?;
            let capacity = usize::try_from(cap)
                .ok()
                .filter(|&cap| cap <= UI_MAX_TEXT)
                .ok_or_else(|| wasmtime::Error::msg("ui_input_text capacity out of range"))?;
            if usize::try_from(len).map_or(true, |len| len > capacity) {
                return Err(wasmtime::Error::msg(
                    "ui_input_text length exceeds capacity",
                ));
            }
            let label = read_text(&mut caller, label_ptr, label_len)?;
            // Validate the whole buffer so a submitted value always fits.
            read_bytes(&mut caller, buf_ptr, cap, UI_MAX_TEXT)?;
            let current = read_text(&mut caller, buf_ptr, len)?;
            let (value, result) = match claim(&mut caller, id)? {
                Some(ExpansionUiEvent::Text(text)) => {
                    let text = truncate(&text, capacity).to_owned();
                    let memory = guest_memory(&mut caller)?;
                    memory
                        .write(&mut caller, buf_ptr as u32 as usize, text.as_bytes())
                        .map_err(|_| wasmtime::Error::msg("ui text pointer out of range"))?;
                    let length = text.len() as i32;
                    (text, length)
                }
                _ => (current, -1),
            };
            push(
                &mut caller,
                ExpansionUiItem::InputText {
                    id,
                    label,
                    value,
                    capacity,
                },
            )?;
            Ok(result)
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_keeps_char_boundaries() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("aあ", 2), "a");
        assert_eq!(truncate("aあ", 4), "aあ");
    }

    #[test]
    fn colors_unpack_rrggbbaa() {
        assert_eq!(rgba(0xFF00_80FFu32 as i32), [1.0, 0.0, 128.0 / 255.0, 1.0]);
    }
}
