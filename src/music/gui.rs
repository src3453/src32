use super::project::{Cell, Effect, Instrument, MacroStep, Note, Pattern, Song};
use super::stream::{MusicError, StreamPlayer, compile_song};
use super::wavetable_editor::WavetableEditor;
use crate::devices::sgu::s3w2::S3w2Sound;
use imgui::{Condition, TableFlags, Ui, WindowFocusedFlags};
use std::cell::RefCell;
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use winit::keyboard::KeyCode;

const DRIVER_SOURCE: &str = include_str!("../../sol/cpt32/sgu_music_driver.sol");
const NOTE_NAMES: [&str; 12] = [
    "C-", "C#", "D-", "D#", "E-", "F-", "F#", "G-", "G#", "A-", "A#", "B-",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Note,
    Instrument,
    Volume,
    Effect,
}

impl Field {
    const ALL: [Self; 4] = [Self::Note, Self::Instrument, Self::Volume, Self::Effect];
    fn index(self) -> usize {
        match self {
            Self::Note => 0,
            Self::Instrument => 1,
            Self::Volume => 2,
            Self::Effect => 3,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Note => "NOTE",
            Self::Instrument => "INS",
            Self::Volume => "VOL",
            Self::Effect => "FX",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cursor {
    row: usize,
    channel: usize,
    field: Field,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            row: 0,
            channel: 0,
            field: Field::Note,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EffectKind {
    Volume,
    Pan,
    Tempo,
}

impl EffectKind {
    fn of(effect: Option<Effect>) -> Self {
        match effect {
            Some(Effect::SetVolume(_)) => Self::Volume,
            Some(Effect::SetTempo(_)) => Self::Tempo,
            _ => Self::Pan,
        }
    }
    fn letter(self) -> char {
        match self {
            Self::Volume => 'V',
            Self::Pan => 'P',
            Self::Tempo => 'T',
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathAction {
    Open,
    SaveAs,
}

/// Original SGU tracker interface. Only editing concepts are informed by Furnace documentation.
pub struct TrackerUi {
    song: Song,
    project_path: Option<PathBuf>,
    project_path_input: String,
    export_base_input: String,
    project_dir: PathBuf,
    selected_order: usize,
    cursor: Cursor,
    selected_instrument: usize,
    selected_macro_step: usize,
    input_octave: u8,
    edit_step: usize,
    high_nibble: Option<u8>,
    effect_entry: Option<EffectKind>,
    pattern_focused: bool,
    focus_pattern_requested: bool,
    scroll_cursor_requested: bool,
    path_action: Option<PathAction>,
    open_path_popup: bool,
    export_confirmation: Option<(PathBuf, PathBuf)>,
    open_export_popup: bool,
    pcm_path_input: String,
    pcm_loop_input: String,
    player: Option<StreamPlayer>,
    playing: bool,
    reset_sgus_pending: bool,
    frames_advanced: u32,
    total_frames: u32,
    play_order: usize,
    play_row: usize,
    play_tempo: u16,
    play_accumulator: u16,
    play_ticks: u8,
    play_enter_row: bool,
    playing_location: Option<(usize, usize)>,
    dirty: bool,
    status: String,
    wavetable_editor: WavetableEditor,
}

impl TrackerUi {
    pub fn new(project_path: Option<PathBuf>) -> Self {
        let mut tracker = Self {
            song: Song::default(),
            project_path: None,
            project_path_input: project_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            export_base_input: "music".into(),
            project_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            selected_order: 0,
            cursor: Cursor::default(),
            selected_instrument: 0,
            selected_macro_step: 0,
            input_octave: 4,
            edit_step: 1,
            high_nibble: None,
            effect_entry: None,
            pattern_focused: true,
            focus_pattern_requested: true,
            scroll_cursor_requested: true,
            path_action: None,
            open_path_popup: false,
            export_confirmation: None,
            open_export_popup: false,
            pcm_path_input: String::new(),
            pcm_loop_input: String::new(),
            player: None,
            playing: false,
            reset_sgus_pending: false,
            frames_advanced: 0,
            total_frames: 0,
            play_order: 0,
            play_row: 0,
            play_tempo: 120,
            play_accumulator: 0,
            play_ticks: 0,
            play_enter_row: true,
            playing_location: None,
            dirty: false,
            status: "Ready. F6 focuses the pattern; F5 plays; F8 stops.".into(),
            wavetable_editor: WavetableEditor::new(),
        };
        if project_path.is_some() {
            tracker.open_project();
        }
        tracker.sync_instrument_editor();
        tracker
    }

    fn current_pattern_id(&self) -> usize {
        usize::from(self.song.orders[self.selected_order])
    }
    fn selected_cell(&self) -> &Cell {
        &self.song.patterns[self.current_pattern_id()].rows[self.cursor.row][self.cursor.channel]
    }
    fn selected_cell_mut(&mut self) -> &mut Cell {
        let pattern = self.current_pattern_id();
        &mut self.song.patterns[pattern].rows[self.cursor.row][self.cursor.channel]
    }
    fn reset_entry(&mut self) {
        self.high_nibble = None;
        self.effect_entry = None;
    }
    fn cursor_moved(&mut self) {
        self.reset_entry();
        self.scroll_cursor_requested = true;
    }
    fn advance_edit_row(&mut self) {
        self.cursor.row = (self.cursor.row + self.edit_step) % 64;
        self.cursor_moved();
    }
    fn request_stop(&mut self) {
        // Do not discard this hardware reset when a replacement player is installed before VSYNC.
        self.reset_sgus_pending = true;
        self.player = None;
        self.playing = false;
        self.playing_location = None;
    }
    fn mark_changed(&mut self) {
        self.dirty = true;
        if self.playing {
            self.request_stop();
            self.status =
                "Preview stopped because the song changed. F5 restarts with the edits.".into();
        }
    }
    fn set_note(&mut self, note: Note) {
        let instrument = (self.selected_instrument < self.song.instruments.len())
            .then_some(self.selected_instrument as u16);
        let cell = self.selected_cell_mut();
        cell.note = Some(note);
        if matches!(note, Note::On(_)) {
            cell.instrument = instrument;
        }
        self.mark_changed();
        self.advance_edit_row();
    }
    fn clear_field(&mut self) {
        let field = self.cursor.field;
        let cell = self.selected_cell_mut();
        match field {
            Field::Note => cell.note = None,
            Field::Instrument => cell.instrument = None,
            Field::Volume => cell.volume = None,
            Field::Effect => cell.effect = None,
        }
        self.reset_entry();
        self.mark_changed();
    }
    fn enter_hex(&mut self, nibble: u8) {
        let Some(high) = self.high_nibble.take() else {
            self.high_nibble = Some(nibble);
            return;
        };
        let value = high * 16 + nibble;
        let kind = self
            .effect_entry
            .unwrap_or_else(|| EffectKind::of(self.selected_cell().effect));
        match self.cursor.field {
            Field::Instrument if usize::from(value) >= self.song.instruments.len() => {
                self.status = format!(
                    "Instrument {value:02X} does not exist. Add or select an instrument in the list."
                );
                return;
            }
            Field::Effect if kind == EffectKind::Tempo && !(30..=150).contains(&value) => {
                self.status =
                    format!("Tempo effect must be 1E..96 hex (30..150 BPM), not {value:02X}.");
                return;
            }
            _ => {}
        }
        let field = self.cursor.field;
        let cell = self.selected_cell_mut();
        match field {
            Field::Instrument => cell.instrument = Some(u16::from(value)),
            Field::Volume => cell.volume = Some(value),
            Field::Effect => {
                cell.effect = Some(match kind {
                    EffectKind::Volume => Effect::SetVolume(value),
                    EffectKind::Pan => Effect::SetPan(value),
                    EffectKind::Tempo => Effect::SetTempo(u16::from(value)),
                })
            }
            Field::Note => return,
        }
        self.mark_changed();
        self.advance_edit_row();
    }

    pub fn handle_key(&mut self, key: KeyCode, down: bool) -> bool {
        self.handle_input(key, down, false, false, false)
    }

    /// Called with physical key codes only when ImGui is not accepting text input.
    pub fn handle_input(
        &mut self,
        key: KeyCode,
        down: bool,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> bool {
        if !down {
            return false;
        }
        if self.path_action.is_some() || self.export_confirmation.is_some() {
            return false;
        }
        if ctrl && !alt {
            match key {
                KeyCode::KeyS => {
                    if shift {
                        self.show_path_action(PathAction::SaveAs);
                    } else {
                        self.save_project();
                    }
                    return true;
                }
                KeyCode::KeyO => {
                    self.show_path_action(PathAction::Open);
                    return true;
                }
                KeyCode::NumpadMultiply | KeyCode::BracketRight | KeyCode::Equal => {
                    self.edit_step = (self.edit_step + 1).min(64);
                    return true;
                }
                KeyCode::NumpadDivide | KeyCode::BracketLeft | KeyCode::Minus => {
                    self.edit_step = self.edit_step.saturating_sub(1);
                    return true;
                }
                _ => return false,
            }
        }
        if alt {
            return false;
        }
        match key {
            KeyCode::F5 => {
                self.start_playback();
                return true;
            }
            KeyCode::F8 => {
                self.request_stop();
                self.status = "Preview stopped".into();
                return true;
            }
            KeyCode::F6 => {
                self.pattern_focused = true;
                self.focus_pattern_requested = true;
                self.scroll_cursor_requested = true;
                return true;
            }
            KeyCode::NumpadMultiply | KeyCode::BracketRight => {
                self.input_octave = (self.input_octave + 1).min(9);
                return true;
            }
            KeyCode::NumpadDivide | KeyCode::BracketLeft => {
                self.input_octave = self.input_octave.saturating_sub(1);
                return true;
            }
            _ => {}
        }
        if !self.pattern_focused {
            return false;
        }
        let column = self.cursor.channel * 4 + self.cursor.field.index();
        match key {
            KeyCode::ArrowUp => self.cursor.row = self.cursor.row.saturating_sub(1),
            KeyCode::ArrowDown => self.cursor.row = (self.cursor.row + 1).min(63),
            KeyCode::ArrowLeft | KeyCode::ArrowRight => {
                let column = if key == KeyCode::ArrowLeft {
                    column.saturating_sub(1)
                } else {
                    (column + 1).min(63)
                };
                self.cursor.channel = column / 4;
                self.cursor.field = Field::ALL[column % 4];
            }
            KeyCode::Tab => {
                self.cursor.channel = (self.cursor.channel + if shift { 15 } else { 1 }) % 16;
            }
            KeyCode::Home => {
                self.cursor.row = if shift {
                    self.cursor.row.saturating_sub(1)
                } else {
                    0
                }
            }
            KeyCode::End => {
                self.cursor.row = if shift {
                    (self.cursor.row + 1).min(63)
                } else {
                    63
                }
            }
            KeyCode::PageUp => self.cursor.row = self.cursor.row.saturating_sub(16),
            KeyCode::PageDown => self.cursor.row = (self.cursor.row + 16).min(63),
            KeyCode::Enter | KeyCode::NumpadEnter => self.cursor.row = (self.cursor.row + 1) % 64,
            KeyCode::Delete => {
                self.clear_field();
                return true;
            }
            KeyCode::Escape => {
                self.reset_entry();
                return true;
            }
            KeyCode::Backspace if self.cursor.field == Field::Note => {
                self.set_note(Note::Off);
                return true;
            }
            _ => {
                if self.cursor.field == Field::Note {
                    if let Some(note) = input_note(key, self.input_octave) {
                        self.set_note(Note::On(note));
                        return true;
                    }
                } else {
                    if self.cursor.field == Field::Effect {
                        let kind = match key {
                            KeyCode::KeyP => Some(EffectKind::Pan),
                            KeyCode::KeyT => Some(EffectKind::Tempo),
                            KeyCode::KeyV => Some(EffectKind::Volume),
                            _ => None,
                        };
                        if let Some(kind) = kind {
                            self.effect_entry = Some(kind);
                            self.high_nibble = None;
                            return true;
                        }
                    }
                    if let Some(nibble) = input_hex(key) {
                        self.enter_hex(nibble);
                        return true;
                    }
                }
                return false;
            }
        }
        self.cursor_moved();
        true
    }

    pub fn draw(&mut self, ui: &Ui) {
        let display = ui.io().display_size;
        ui.window("SGU Music Editor")
            .position([0.0, 0.0], Condition::Always)
            .size(
                [display[0].max(1.0), display[1].max(1.0)],
                Condition::Always,
            )
            .title_bar(false)
            .movable(false)
            .resizable(false)
            .collapsible(false)
            .scroll_bar(false)
            .save_settings(false)
            .build(|| {
                let available = ui.content_region_avail();
                let gap = 6.0;
                let height = (available[1] - gap * 2.0).max(3.0);
                let transport_height = (height * 0.25).min(184.0);
                let instruments_height = (height * 0.32).min(320.0);
                let pattern_height = (height - transport_height - instruments_height).max(1.0);
                ui.child_window("Transport")
                    .size([0.0, transport_height])
                    .border(true)
                    .horizontal_scrollbar(true)
                    .build(|| {
                        self.observe_nonpattern_focus(ui);
                        self.draw_transport(ui);
                    });
                let orders_width = (available[0] * 0.26).clamp(1.0, 330.0);
                ui.child_window("Orders")
                    .size([orders_width, instruments_height])
                    .border(true)
                    .horizontal_scrollbar(true)
                    .build(|| {
                        self.observe_nonpattern_focus(ui);
                        self.draw_orders(ui);
                    });
                ui.same_line_with_spacing(0.0, gap);
                ui.child_window("Instruments")
                    .size([0.0, instruments_height])
                    .border(true)
                    .build(|| {
                        self.observe_nonpattern_focus(ui);
                        self.draw_instruments(ui);
                    });
                let focus = std::mem::take(&mut self.focus_pattern_requested);
                ui.child_window("Pattern")
                    .size([0.0, pattern_height])
                    .border(true)
                    .focused(focus)
                    .build(|| {
                        if ui.is_window_focused_with_flags(WindowFocusedFlags::CHILD_WINDOWS) {
                            self.pattern_focused = true;
                        }
                        self.draw_pattern(ui);
                    });
                self.draw_popups(ui);
            });
        if let Some(Instrument::Wavetable { samples, .. }) =
            self.song.instruments.get_mut(self.selected_instrument)
        {
            if self.wavetable_editor.draw(ui, samples) {
                self.mark_changed();
            }
        }
    }

    fn observe_nonpattern_focus(&mut self, ui: &Ui) {
        if ui.is_window_focused_with_flags(WindowFocusedFlags::CHILD_WINDOWS) {
            if self.pattern_focused {
                self.reset_entry();
            }
            self.pattern_focused = false;
        }
    }

    fn draw_transport(&mut self, ui: &Ui) {
        ui.text("SGU MUSIC EDITOR / 16 voices / 60 Hz preview");
        ui.same_line();
        ui.text_colored(
            if self.dirty {
                [1.0, 0.7, 0.25, 1.0]
            } else {
                [0.45, 0.85, 0.6, 1.0]
            },
            if self.dirty { "UNSAVED" } else { "saved" },
        );
        if ui.button("Play [F5]") {
            self.start_playback();
        }
        ui.same_line();
        if ui.button("Stop [F8]") {
            self.request_stop();
            self.status = "Preview stopped".into();
        }
        ui.same_line();
        ui.text(if self.playing { "PLAYING" } else { "STOPPED" });
        ui.same_line();
        if self.playing {
            ui.text(format!(
                "frame {}/{}",
                self.frames_advanced, self.total_frames
            ));
        }
        ui.same_line();
        if bounded_u16(ui, "BPM", &mut self.song.tempo_bpm, 30, 150, 66.0) {
            self.mark_changed();
        }
        ui.same_line();
        if bounded_u8(ui, "Ticks/row", &mut self.song.ticks_per_row, 1, 31, 48.0) {
            self.mark_changed();
        }
        ui.same_line();
        if ui.checkbox("Repeat", &mut self.song.repeat) {
            self.mark_changed();
        }
        ui.same_line();
        bounded_u8(ui, "Octave [ / ]", &mut self.input_octave, 0, 9, 38.0);
        ui.same_line();
        let mut step = self.edit_step as i32;
        ui.set_next_item_width(42.0);
        if ui.input_scalar("Step Ctrl+[ / ]", &mut step).build() {
            self.edit_step = step.clamp(0, 64) as usize;
        }
        ui.set_next_item_width((ui.content_region_avail()[0] - 340.0).max(80.0));
        ui.input_text("##project_path", &mut self.project_path_input)
            .hint("Project TOML path")
            .build();
        ui.same_line();
        if ui.button("Load [Ctrl+O]") {
            self.open_project();
        }
        ui.same_line();
        if ui.button("Save [Ctrl+S]") {
            self.save_project();
        }
        ui.same_line();
        if ui.button("Save As") {
            self.show_path_action(PathAction::SaveAs);
        }
        ui.set_next_item_width((ui.content_region_avail()[0] - 210.0).max(80.0));
        ui.input_text("##export_path", &mut self.export_base_input)
            .hint("Export basename")
            .build();
        ui.same_line();
        if ui.button("Export SGUB + driver source") {
            self.request_export();
        }
        ui.text_wrapped(&self.status);
    }

    fn draw_orders(&mut self, ui: &Ui) {
        ui.text(format!(
            "ORDERS {:02X} / {}",
            self.selected_order,
            self.song.orders.len()
        ));
        if ui.small_button("Add") && self.song.orders.len() < 256 {
            self.song.orders.insert(
                self.selected_order + 1,
                self.song.orders[self.selected_order],
            );
            self.selected_order += 1;
            self.cursor_moved();
            self.mark_changed();
        }
        ui.same_line();
        if ui.small_button("Remove") && self.song.orders.len() > 1 {
            self.song.orders.remove(self.selected_order);
            self.selected_order = self.selected_order.min(self.song.orders.len() - 1);
            self.cursor_moved();
            self.mark_changed();
        }
        ui.same_line();
        if ui.small_button("New pattern") && self.song.patterns.len() < 256 {
            self.song.patterns.push(Pattern::default());
            self.song.orders[self.selected_order] = (self.song.patterns.len() - 1) as u16;
            self.cursor_moved();
            self.mark_changed();
        }
        let mut label = String::with_capacity(64);
        write!(label, "{:02X}", self.current_pattern_id()).unwrap();
        ui.set_next_item_width(80.0);
        if let Some(_combo) = ui.begin_combo("Pattern reference", &label) {
            for pattern in 0..self.song.patterns.len() {
                label.clear();
                write!(label, "{pattern:02X}").unwrap();
                if ui
                    .selectable_config(&label)
                    .selected(pattern == self.current_pattern_id())
                    .build()
                {
                    self.song.orders[self.selected_order] = pattern as u16;
                    self.cursor_moved();
                    self.mark_changed();
                }
            }
        }
        ui.separator();
        for order in 0..self.song.orders.len() {
            label.clear();
            write!(
                label,
                "{order:02X}  ->  pattern {:02X}",
                self.song.orders[order]
            )
            .unwrap();
            if ui
                .selectable_config(&label)
                .selected(order == self.selected_order)
                .build()
            {
                self.selected_order = order;
                self.cursor_moved();
            }
        }
    }

    fn sync_instrument_editor(&mut self) {
        if let Some(Instrument::Pcm {
            path, loop_start, ..
        }) = self.song.instruments.get(self.selected_instrument)
        {
            self.pcm_path_input = path.display().to_string();
            self.pcm_loop_input = loop_start.map(|v| v.to_string()).unwrap_or_default();
        }
        self.selected_macro_step = 0;
    }

    fn draw_instruments(&mut self, ui: &Ui) {
        let width = ui.content_region_avail()[0];
        ui.child_window("Instrument list")
            .size([(width * 0.27).min(190.0), 0.0])
            .build(|| {
                ui.text("INSTRUMENTS");
                if self.song.instruments.len() < 256 {
                    for (kind, title) in [(0, "+ Wave"), (1, "+ PCM"), (2, "+ Noise")] {
                        if ui.small_button(title) {
                            let instrument = match kind {
                                1 => Instrument::Pcm {
                                    path: PathBuf::from("sample.wav"),
                                    base_note: 60,
                                    loop_start: None,
                                    volume: 255,
                                    pan: 255,
                                    macro_: Default::default(),
                                },
                                2 => Instrument::Noise {
                                    volume: 255,
                                    pan: 255,
                                    macro_: Default::default(),
                                },
                                _ => Instrument::default(),
                            };
                            self.song.instruments.push(instrument);
                            self.selected_instrument = self.song.instruments.len() - 1;
                            self.sync_instrument_editor();
                            self.mark_changed();
                        }
                    }
                }
                ui.separator();
                let mut label = String::with_capacity(32);
                for index in 0..self.song.instruments.len() {
                    label.clear();
                    write!(
                        label,
                        "{index:02X}  {}",
                        instrument_name(&self.song.instruments[index])
                    )
                    .unwrap();
                    if ui
                        .selectable_config(&label)
                        .selected(index == self.selected_instrument)
                        .build()
                    {
                        self.selected_instrument = index;
                        self.sync_instrument_editor();
                    }
                }
            });
        ui.same_line();
        ui.child_window("Instrument editor")
            .size([0.0, 0.0])
            .horizontal_scrollbar(true)
            .build(|| {
                if self.song.instruments.is_empty() {
                    ui.text_wrapped("Add an instrument to enable note playback.");
                    return;
                }
                ui.text(format!("INSTRUMENT {:02X}", self.selected_instrument));
                self.draw_instrument_parameters(ui);
                self.draw_macros(ui);
            });
    }

    fn draw_instrument_parameters(&mut self, ui: &Ui) {
        let index = self.selected_instrument;
        let mut kind = match self.song.instruments[index] {
            Instrument::Wavetable { .. } => 0,
            Instrument::Pcm { .. } => 1,
            Instrument::Noise { .. } => 2,
        };
        let old_kind = kind;
        ui.set_next_item_width(130.0);
        ui.combo_simple_string("Type", &mut kind, &["Wavetable", "PCM", "Noise"]);
        if kind != old_kind {
            let old = &self.song.instruments[index];
            let (volume, pan) = old.volume_pan();
            let macro_ = old.macro_data().clone();
            self.song.instruments[index] = match kind {
                1 => Instrument::Pcm {
                    path: PathBuf::from("sample.wav"),
                    base_note: 60,
                    loop_start: None,
                    volume,
                    pan,
                    macro_,
                },
                2 => Instrument::Noise {
                    volume,
                    pan,
                    macro_,
                },
                _ => {
                    let Instrument::Wavetable { samples, .. } = Instrument::default() else {
                        unreachable!()
                    };
                    Instrument::Wavetable {
                        samples,
                        volume,
                        pan,
                        macro_,
                    }
                }
            };
            self.sync_instrument_editor();
            self.mark_changed();
        }
        let mut changed = false;
        let mut error = None;
        let instrument = &mut self.song.instruments[index];
        let (volume, pan) = match instrument {
            Instrument::Wavetable { volume, pan, .. }
            | Instrument::Pcm { volume, pan, .. }
            | Instrument::Noise { volume, pan, .. } => (volume, pan),
        };
        changed |= bounded_u8(ui, "Base volume", volume, 0, 255, 65.0);
        ui.same_line();
        changed |= bounded_u8(ui, "Pan (LR nibbles)", pan, 0, 255, 65.0);
        match instrument {
            Instrument::Wavetable { samples, .. } => {
                changed |= self.wavetable_editor.preview(ui, samples);
            }
            Instrument::Pcm {
                path,
                base_note,
                loop_start,
                ..
            } => {
                ui.set_next_item_width((ui.content_region_avail()[0] - 100.0).max(60.0));
                ui.input_text("PCM WAV path", &mut self.pcm_path_input)
                    .build();
                if ui.is_item_deactivated_after_edit() {
                    if self.pcm_path_input.trim().is_empty() {
                        error = Some("PCM path must not be empty".into());
                    } else {
                        *path = PathBuf::from(&self.pcm_path_input);
                        changed = true;
                    }
                }
                changed |= bounded_u8(ui, "Base note (MIDI)", base_note, 0, 127, 65.0);
                ui.set_next_item_width(100.0);
                ui.input_text("Loop sample (blank = off)", &mut self.pcm_loop_input)
                    .build();
                if ui.is_item_deactivated_after_edit() {
                    if self.pcm_loop_input.trim().is_empty() {
                        *loop_start = None;
                        changed = true;
                    } else if let Ok(value) = self.pcm_loop_input.trim().parse::<usize>() {
                        *loop_start = Some(value);
                        changed = true;
                    } else {
                        error = Some("PCM loop must be an unsigned sample index or blank".into());
                    }
                }
                let resolved = if path.is_absolute() {
                    path.clone()
                } else {
                    self.project_dir.join(&*path)
                };
                ui.text_wrapped(format!(
                    "Asset: {}{}",
                    resolved.display(),
                    if resolved.is_file() {
                        ""
                    } else {
                        " (missing: preview/export will fail; project saving remains allowed)"
                    }
                ));
                ui.text_wrapped("16-bit PCM mono/stereo WAV. Loop bounds are checked against the loaded asset on preview/export.");
            }
            Instrument::Noise { .. } => {
                ui.text("Noise uses the selected note pitch, base volume, pan and macros.")
            }
        }
        if changed {
            self.mark_changed();
        }
        if let Some(error) = error {
            self.status = error;
        }
    }

    fn draw_macros(&mut self, ui: &Ui) {
        ui.separator();
        let macro_data = self.song.instruments[self.selected_instrument].macro_data_mut();
        let mut changed = false;
        ui.text(format!(
            "MACRO / {} steps / one step per VSYNC",
            macro_data.steps.len()
        ));
        if ui.small_button("Add step") && macro_data.steps.len() < 256 {
            macro_data.steps.push(MacroStep::default());
            self.selected_macro_step = macro_data.steps.len() - 1;
            changed = true;
        }
        ui.same_line();
        if ui.small_button("Remove step") && !macro_data.steps.is_empty() {
            let removed = self.selected_macro_step.min(macro_data.steps.len() - 1);
            macro_data.steps.remove(removed);
            macro_data.loop_start = macro_data.loop_start.and_then(|start| {
                if start == removed {
                    None
                } else {
                    Some(if start > removed { start - 1 } else { start })
                }
            });
            self.selected_macro_step = removed.min(macro_data.steps.len().saturating_sub(1));
            changed = true;
        }
        ui.same_line();
        if ui.small_button("Loop at selected") && !macro_data.steps.is_empty() {
            macro_data.loop_start = Some(self.selected_macro_step.min(macro_data.steps.len() - 1));
            changed = true;
        }
        ui.same_line();
        if ui.small_button("No loop") {
            macro_data.loop_start = None;
            changed = true;
        }
        let mut loop_start = macro_data.loop_start.map(|n| n as i32).unwrap_or(-1);
        ui.set_next_item_width(60.0);
        if ui
            .input_scalar("Loop index (-1 = off)", &mut loop_start)
            .build()
        {
            if loop_start == -1 {
                macro_data.loop_start = None;
                changed = true;
            } else if loop_start >= 0 && (loop_start as usize) < macro_data.steps.len() {
                macro_data.loop_start = Some(loop_start as usize);
                changed = true;
            } else {
                self.status =
                    "Macro loop must address an existing step, or be -1 for no loop".into();
            }
        }
        ui.text_wrapped("Enable a lane and type its value. Disabled means hold the previous value. Pitch is -48..48 semitones; pan is decimal LR nibbles.");
        if let Some(_table) =
            ui.begin_table_with_flags("Macro steps", 4, TableFlags::BORDERS | TableFlags::ROW_BG)
        {
            for title in ["Step", "Volume", "Pitch", "Pan"] {
                ui.table_setup_column(title);
            }
            ui.table_headers_row();
            for (index, step) in macro_data.steps.iter_mut().enumerate() {
                let _id = ui.push_id_usize(index);
                ui.table_next_row();
                ui.table_next_column();
                if ui
                    .selectable_config(format!(
                        "{index:02X}{}",
                        if macro_data.loop_start == Some(index) {
                            " LOOP"
                        } else {
                            ""
                        }
                    ))
                    .selected(index == self.selected_macro_step)
                    .build()
                {
                    self.selected_macro_step = index;
                }
                ui.table_next_column();
                changed |= optional_u8(ui, "vol", &mut step.volume);
                ui.table_next_column();
                let mut enabled = step.pitch_semitones.is_some();
                if ui.checkbox("##pitch_enabled", &mut enabled) {
                    step.pitch_semitones = enabled.then_some(step.pitch_semitones.unwrap_or(0));
                    changed = true;
                }
                if enabled {
                    ui.same_line();
                    let mut value = i32::from(step.pitch_semitones.unwrap_or(0));
                    ui.set_next_item_width(52.0);
                    if ui.input_scalar("##pitch_value", &mut value).build() {
                        step.pitch_semitones = Some(value.clamp(-48, 48) as i8);
                        changed = true;
                    }
                }
                ui.table_next_column();
                changed |= optional_u8(ui, "pan", &mut step.pan);
            }
        }
        if changed {
            self.mark_changed();
        }
    }

    fn draw_pattern(&mut self, ui: &Ui) {
        ui.text(format!(
            "PATTERN {:02X} / order {:02X} / row {:02X} / channel {:02X} / {}{}",
            self.current_pattern_id(),
            self.selected_order,
            self.cursor.row,
            self.cursor.channel,
            self.cursor.field.name(),
            if self.pattern_focused {
                " [keyboard focus]"
            } else {
                " [F6 to focus]"
            }
        ));
        ui.text_wrapped("Arrows: fields/rows  Tab: channel  Enter: next row  Z..M/Q..U: notes  Hex: INS/VOL/FX  P/T/V: FX type  Delete: field  Backspace: OFF");
        ui.child_window("Pattern text grid")
            .size([0.0, 0.0])
            .horizontal_scrollbar(true)
            .build(|| {
                if ui.is_window_focused() {
                    self.pattern_focused = true;
                }
                self.draw_pattern_canvas(ui);
            });
    }

    fn draw_pattern_canvas(&mut self, ui: &Ui) {
        let character = ui.calc_text_size("0")[0];
        let row_height = ui.text_line_height() + 6.0;
        let widths = [
            character * 4.0 + 8.0,
            character * 2.0 + 8.0,
            character * 2.0 + 8.0,
            character * 3.0 + 8.0,
        ];
        let channel_width: f32 = widths.iter().sum::<f32>() + character * 2.0;
        let row_number_width = character * 4.0 + 8.0;
        let start = ui.cursor_pos();
        let origin = ui.cursor_screen_pos();
        let viewport = ui.content_region_avail();
        let size = [row_number_width + channel_width * 16.0, row_height * 65.0];
        if self.scroll_cursor_requested {
            let x = row_number_width
                + channel_width * self.cursor.channel as f32
                + widths[..self.cursor.field.index()].iter().sum::<f32>();
            let y = row_height * (self.cursor.row + 1) as f32;
            let scroll_x = ui.scroll_x();
            let scroll_y = ui.scroll_y();
            // Keep the complete selected field within the viewport, including scrollbar space.
            let view_width = (viewport[0] - 20.0).max(widths[self.cursor.field.index()]);
            let view_height = (viewport[1] - 20.0).max(row_height);
            if x < scroll_x {
                ui.set_scroll_x(x);
            } else if x + widths[self.cursor.field.index()] > scroll_x + view_width {
                ui.set_scroll_x(x + widths[self.cursor.field.index()] - view_width);
            }
            if y < scroll_y {
                ui.set_scroll_y(y);
            } else if y + row_height > scroll_y + view_height {
                ui.set_scroll_y(y + row_height - view_height);
            }
            self.scroll_cursor_requested = false;
        }
        let clicked = ui.invisible_button("Pattern canvas", size);
        if clicked {
            let mouse = ui.io().mouse_pos;
            let x = mouse[0] - origin[0];
            let y = mouse[1] - origin[1];
            if y >= row_height && y < size[1] {
                self.cursor.row = (y / row_height) as usize - 1;
                if x >= row_number_width {
                    self.cursor.channel =
                        (((x - row_number_width) / channel_width) as usize).min(15);
                    let within = (x - row_number_width) % channel_width;
                    let mut boundary = 0.0;
                    self.cursor.field = Field::Effect;
                    for field in Field::ALL {
                        boundary += widths[field.index()];
                        if within < boundary {
                            self.cursor.field = field;
                            break;
                        }
                    }
                }
                self.pattern_focused = true;
                self.cursor_moved();
            }
        }
        let draw = ui.get_window_draw_list();
        let first_row = ((ui.scroll_y() / row_height) as usize)
            .saturating_sub(1)
            .min(63);
        let last_row = ((ui.scroll_y() + viewport[1]) / row_height) as usize;
        let first_channel = ((ui.scroll_x() - row_number_width).max(0.0) / channel_width) as usize;
        let last_channel = (((ui.scroll_x() + viewport[0]) / channel_width) as usize + 1).min(15);
        let pattern = &self.song.patterns[self.current_pattern_id()];
        let mut text = String::with_capacity(32);
        for channel in first_channel.min(15)..=last_channel {
            text.clear();
            write!(text, "CH {channel:02X}").unwrap();
            draw.add_text(
                [
                    origin[0] + row_number_width + channel as f32 * channel_width,
                    origin[1],
                ],
                [0.65, 0.78, 1.0, 1.0],
                &text,
            );
        }
        for row in first_row..=last_row.min(63) {
            let y = origin[1] + (row + 1) as f32 * row_height;
            let is_playing = self.playing_location == Some((self.selected_order, row));
            let color = if is_playing {
                [0.12, 0.32, 0.22, 1.0]
            } else if row == self.cursor.row {
                [0.19, 0.22, 0.31, 1.0]
            } else if row % 16 == 0 {
                [0.17, 0.19, 0.25, 1.0]
            } else if row % 4 == 0 {
                [0.12, 0.14, 0.18, 1.0]
            } else {
                [0.075, 0.08, 0.10, 1.0]
            };
            draw.add_rect([origin[0], y], [origin[0] + size[0], y + row_height], color)
                .filled(true)
                .build();
            text.clear();
            write!(text, "{row:02X}").unwrap();
            draw.add_text(
                [origin[0] + 4.0, y + 2.0],
                if row % 4 == 0 {
                    [0.8, 0.8, 1.0, 1.0]
                } else {
                    [0.5, 0.53, 0.6, 1.0]
                },
                &text,
            );
            for channel in first_channel.min(15)..=last_channel {
                let mut x = origin[0] + row_number_width + channel as f32 * channel_width;
                let cell = &pattern.rows[row][channel];
                for field in Field::ALL {
                    let selected = self.cursor
                        == Cursor {
                            row,
                            channel,
                            field,
                        };
                    if selected {
                        draw.add_rect(
                            [x, y],
                            [x + widths[field.index()], y + row_height],
                            if self.pattern_focused {
                                [0.34, 0.40, 0.70, 1.0]
                            } else {
                                [0.28, 0.29, 0.34, 1.0]
                            },
                        )
                        .filled(true)
                        .build();
                    }
                    text.clear();
                    write_field(&mut text, cell, field);
                    if selected
                        && field != Field::Note
                        && (self.high_nibble.is_some() || self.effect_entry.is_some())
                    {
                        text.clear();
                        if field == Field::Effect {
                            text.push(
                                self.effect_entry
                                    .unwrap_or_else(|| EffectKind::of(cell.effect))
                                    .letter(),
                            );
                        }
                        if let Some(high) = self.high_nibble {
                            write!(text, "{high:X}_").unwrap();
                        } else {
                            text.push_str("__");
                        }
                    }
                    draw.add_text(
                        [x + 3.0, y + 2.0],
                        if selected {
                            [1.0, 1.0, 1.0, 1.0]
                        } else {
                            [0.79, 0.83, 0.9, 1.0]
                        },
                        &text,
                    );
                    x += widths[field.index()];
                }
                draw.add_line(
                    [x + character, y],
                    [x + character, y + row_height],
                    [0.25, 0.27, 0.32, 1.0],
                )
                .build();
            }
        }
        // The invisible canvas establishes the full scroll extent; do not append widgets after it.
        ui.set_cursor_pos([start[0], start[1] + size[1]]);
    }

    fn show_path_action(&mut self, action: PathAction) {
        self.path_action = Some(action);
        self.open_path_popup = true;
        self.reset_entry();
    }
    fn draw_popups(&mut self, ui: &Ui) {
        let focus_path = std::mem::take(&mut self.open_path_popup);
        if focus_path {
            ui.open_popup("Project path");
        }
        ui.modal_popup_config("Project path")
            .always_auto_resize(true)
            .build(|| {
                if let Some(action) = self.path_action {
                    ui.text(match action {
                        PathAction::Open => "Load TOML project",
                        PathAction::SaveAs => "Save project as TOML (PCM paths are rebased)",
                    });
                    if focus_path {
                        ui.set_keyboard_focus_here();
                    }
                    ui.set_next_item_width((ui.io().display_size[0] * 0.65).clamp(120.0, 700.0));
                    let enter = ui
                        .input_text("Path", &mut self.project_path_input)
                        .enter_returns_true(true)
                        .build();
                    if enter
                        || ui.button(match action {
                            PathAction::Open => "Load",
                            PathAction::SaveAs => "Save As",
                        })
                    {
                        match action {
                            PathAction::Open => self.open_project(),
                            PathAction::SaveAs => self.save_project_as(),
                        }
                        self.path_action = None;
                        self.focus_pattern_requested = true;
                        ui.close_current_popup();
                    }
                    ui.same_line();
                    if ui.button("Cancel") || ui.is_key_pressed(imgui::Key::Escape) {
                        self.path_action = None;
                        self.focus_pattern_requested = true;
                        ui.close_current_popup();
                    }
                    ui.text_wrapped(&self.status);
                }
            });
        if std::mem::take(&mut self.open_export_popup) {
            ui.open_popup("Replace export files?");
        }
        ui.modal_popup_config("Replace export files?")
            .always_auto_resize(true)
            .build(|| {
                if let Some((stream, driver)) = &self.export_confirmation {
                    ui.text("Replace this exact output pair?");
                    ui.text(stream.display().to_string());
                    ui.text(driver.display().to_string());
                    if ui.button("Confirm overwrite") {
                        let (stream, driver) = self.export_confirmation.take().unwrap();
                        self.export_now(&stream, &driver);
                        ui.close_current_popup();
                    }
                    ui.same_line();
                    if ui.button("Cancel export") || ui.is_key_pressed(imgui::Key::Escape) {
                        self.export_confirmation = None;
                        self.status = "Export cancelled; existing files unchanged".into();
                        ui.close_current_popup();
                    }
                }
            });
    }

    fn open_project(&mut self) {
        let path = PathBuf::from(self.project_path_input.trim());
        match Song::load(&path) {
            Ok(song) => {
                self.request_stop();
                self.song = song;
                self.project_dir = project_directory(&path);
                self.project_path = Some(path.clone());
                self.project_path_input = path.display().to_string();
                self.export_base_input = path.with_extension("").display().to_string();
                self.selected_order = 0;
                self.cursor = Cursor::default();
                self.selected_instrument = 0;
                self.cursor_moved();
                self.sync_instrument_editor();
                self.dirty = false;
                self.status = format!("Loaded {}", path.display());
            }
            Err(error) => self.status = format!("Load failed: {error}"),
        }
    }
    fn save_project(&mut self) {
        if let Some(path) = &self.project_path {
            match self.song.save(path) {
                Ok(()) => {
                    self.dirty = false;
                    self.status = format!("Saved {}", path.display());
                }
                Err(error) => self.status = format!("Save failed: {error}"),
            }
        } else if self.project_path_input.trim().is_empty() {
            self.show_path_action(PathAction::SaveAs);
        } else {
            self.save_project_as();
        }
    }
    fn save_project_as(&mut self) {
        let path = PathBuf::from(self.project_path_input.trim());
        if path.as_os_str().is_empty() {
            self.status = "Enter a project file path before saving".into();
            return;
        }
        match self.song.save_as(&path, &self.project_dir) {
            Ok(song) => {
                self.song = song;
                self.project_dir = project_directory(&path);
                self.project_path = Some(path.clone());
                self.project_path_input = path.display().to_string();
                self.export_base_input = path.with_extension("").display().to_string();
                self.dirty = false;
                self.sync_instrument_editor();
                self.status = format!("Saved as {}", path.display());
            }
            Err(error) => self.status = format!("Save As failed: {error}"),
        }
    }
    fn export_paths(&self) -> (PathBuf, PathBuf) {
        let base = if self.export_base_input.trim().is_empty() {
            PathBuf::from("music")
        } else {
            PathBuf::from(self.export_base_input.trim())
        };
        let stream = base.with_extension("sgub");
        let driver = stream.with_file_name("sgu_music_driver.sol");
        (stream, driver)
    }
    fn request_export(&mut self) {
        let (stream, driver) = self.export_paths();
        if stream.exists() || driver.exists() {
            self.export_confirmation = Some((stream, driver));
            self.open_export_popup = true;
        } else {
            self.export_now(&stream, &driver);
        }
    }
    fn export_now(&mut self, stream: &Path, driver: &Path) {
        let result = compile_song(&self.song, &self.project_dir)
            .and_then(|bytes| export_pair(stream, &bytes, driver, DRIVER_SOURCE.as_bytes()));
        self.status = match result {
            Ok(()) => format!("Exported {} and {}", stream.display(), driver.display()),
            Err(error) => format!("Export failed: {error}"),
        };
    }

    fn reset_play_clock(&mut self) {
        self.play_order = 0;
        self.play_row = 0;
        self.play_tempo = self.song.tempo_bpm;
        self.play_accumulator = 0;
        self.play_ticks = 0;
        self.play_enter_row = true;
    }
    fn start_playback(&mut self) {
        self.request_stop();
        match compile_song(&self.song, &self.project_dir).and_then(|bytes| {
            let player = StreamPlayer::from_bytes(&bytes)?;
            let frames = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
            Ok((player, frames))
        }) {
            Ok((player, frames)) => {
                self.player = Some(player);
                self.playing = true;
                self.total_frames = frames;
                self.frames_advanced = 0;
                self.reset_play_clock();
                self.status = "Preview started from the beginning".into();
            }
            Err(error) => self.status = format!("Preview failed: {error}"),
        }
    }
    fn update_play_clock(&mut self) {
        if self.play_enter_row {
            let pattern = usize::from(self.song.orders[self.play_order]);
            if let Some(tempo) = self.song.patterns[pattern].rows[self.play_row]
                .iter()
                .find_map(|cell| {
                    if let Some(Effect::SetTempo(tempo)) = cell.effect {
                        Some(tempo)
                    } else {
                        None
                    }
                })
            {
                self.play_tempo = tempo;
            }
            self.play_enter_row = false;
        }
        self.playing_location = Some((self.play_order, self.play_row));
        self.play_accumulator += self.play_tempo;
        if self.play_accumulator >= 150 {
            self.play_accumulator -= 150;
            self.play_ticks += 1;
            if self.play_ticks == self.song.ticks_per_row {
                self.play_ticks = 0;
                self.play_row += 1;
                self.play_enter_row = true;
                if self.play_row == 64 {
                    self.play_row = 0;
                    self.play_order += 1;
                }
            }
        }
    }
    pub fn advance_vsync(&mut self, sgus: &[Rc<RefCell<S3w2Sound>>; 2]) {
        if std::mem::take(&mut self.reset_sgus_pending) {
            stop_sgus(sgus);
        }
        if !self.playing {
            return;
        }
        if self.frames_advanced == self.total_frames {
            if self.song.repeat {
                self.frames_advanced = 0;
                self.reset_play_clock();
            } else {
                stop_sgus(sgus);
                self.player = None;
                self.playing = false;
                self.playing_location = None;
                self.status = "Preview finished".into();
                return;
            }
        }
        if let Some(player) = &mut self.player {
            match player.advance_vsync(sgus) {
                Ok(()) => {
                    self.frames_advanced += 1;
                    self.update_play_clock();
                }
                Err(error) => {
                    self.request_stop();
                    stop_sgus(sgus);
                    self.reset_sgus_pending = false;
                    self.status = format!("Preview failed: {error}");
                }
            }
        }
    }
}

fn project_directory(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf()
}
fn bounded_u8(ui: &Ui, label: &str, value: &mut u8, min: u8, max: u8, width: f32) -> bool {
    let mut next = i32::from(*value);
    ui.set_next_item_width(width);
    if ui.input_scalar(label, &mut next).build() {
        *value = next.clamp(i32::from(min), i32::from(max)) as u8;
        true
    } else {
        false
    }
}
fn bounded_u16(ui: &Ui, label: &str, value: &mut u16, min: u16, max: u16, width: f32) -> bool {
    let mut next = i32::from(*value);
    ui.set_next_item_width(width);
    if ui.input_scalar(label, &mut next).build() {
        *value = next.clamp(i32::from(min), i32::from(max)) as u16;
        true
    } else {
        false
    }
}
fn optional_u8(ui: &Ui, id: &str, lane: &mut Option<u8>) -> bool {
    let _id = ui.push_id(id);
    let mut enabled = lane.is_some();
    let mut changed = ui.checkbox("##enabled", &mut enabled);
    if changed {
        *lane = enabled.then_some(lane.unwrap_or(0));
    }
    if enabled {
        ui.same_line();
        let mut value = i32::from(lane.unwrap_or(0));
        ui.set_next_item_width(52.0);
        if ui.input_scalar("##value", &mut value).build() {
            *lane = Some(value.clamp(0, 255) as u8);
            changed = true;
        }
    }
    changed
}
fn input_hex(key: KeyCode) -> Option<u8> {
    Some(match key {
        KeyCode::Digit0 | KeyCode::Numpad0 => 0,
        KeyCode::Digit1 | KeyCode::Numpad1 => 1,
        KeyCode::Digit2 | KeyCode::Numpad2 => 2,
        KeyCode::Digit3 | KeyCode::Numpad3 => 3,
        KeyCode::Digit4 | KeyCode::Numpad4 => 4,
        KeyCode::Digit5 | KeyCode::Numpad5 => 5,
        KeyCode::Digit6 | KeyCode::Numpad6 => 6,
        KeyCode::Digit7 | KeyCode::Numpad7 => 7,
        KeyCode::Digit8 | KeyCode::Numpad8 => 8,
        KeyCode::Digit9 | KeyCode::Numpad9 => 9,
        KeyCode::KeyA => 10,
        KeyCode::KeyB => 11,
        KeyCode::KeyC => 12,
        KeyCode::KeyD => 13,
        KeyCode::KeyE => 14,
        KeyCode::KeyF => 15,
        _ => return None,
    })
}
fn input_note(key: KeyCode, octave: u8) -> Option<u8> {
    let semitone = match key {
        KeyCode::KeyZ => 0,
        KeyCode::KeyS => 1,
        KeyCode::KeyX => 2,
        KeyCode::KeyD => 3,
        KeyCode::KeyC => 4,
        KeyCode::KeyV => 5,
        KeyCode::KeyG => 6,
        KeyCode::KeyB => 7,
        KeyCode::KeyH => 8,
        KeyCode::KeyN => 9,
        KeyCode::KeyJ => 10,
        KeyCode::KeyM => 11,
        KeyCode::KeyQ => 12,
        KeyCode::Digit2 => 13,
        KeyCode::KeyW => 14,
        KeyCode::Digit3 => 15,
        KeyCode::KeyE => 16,
        KeyCode::KeyR => 17,
        KeyCode::Digit5 => 18,
        KeyCode::KeyT => 19,
        KeyCode::Digit6 => 20,
        KeyCode::KeyY => 21,
        KeyCode::Digit7 => 22,
        KeyCode::KeyU => 23,
        _ => return None,
    };
    u8::try_from(u16::from(octave) * 12 + 12 + semitone)
        .ok()
        .filter(|note| *note <= 127)
}
fn write_field(text: &mut String, cell: &Cell, field: Field) {
    match field {
        Field::Note => match cell.note {
            Some(Note::On(note)) => {
                write!(
                    text,
                    "{}{}",
                    NOTE_NAMES[usize::from(note % 12)],
                    i16::from(note / 12) - 1
                )
                .unwrap();
            }
            Some(Note::Off) => text.push_str("OFF"),
            None => text.push_str("---"),
        },
        Field::Instrument => {
            if let Some(id) = cell.instrument {
                write!(text, "{id:02X}").unwrap();
            } else {
                text.push_str("--");
            }
        }
        Field::Volume => {
            if let Some(value) = cell.volume {
                write!(text, "{value:02X}").unwrap();
            } else {
                text.push_str("--");
            }
        }
        Field::Effect => match cell.effect {
            Some(Effect::SetVolume(value)) => {
                write!(text, "V{value:02X}").unwrap();
            }
            Some(Effect::SetPan(value)) => {
                write!(text, "P{value:02X}").unwrap();
            }
            Some(Effect::SetTempo(value)) => {
                write!(text, "T{value:02X}").unwrap();
            }
            None => text.push_str("---"),
        },
    }
}
fn instrument_name(instrument: &Instrument) -> &'static str {
    match instrument {
        Instrument::Wavetable { .. } => "Wavetable",
        Instrument::Pcm { .. } => "PCM",
        Instrument::Noise { .. } => "Noise",
    }
}
fn stop_sgus(sgus: &[Rc<RefCell<S3w2Sound>>; 2]) {
    for sgu in sgus {
        let mut sgu = sgu.borrow_mut();
        for channel in 0..8u32 {
            let base = 0x800 + channel * 0x20;
            sgu.write_register(base + 3, 0);
            sgu.write_register(base + 0x19, 0);
        }
    }
}
fn sibling_path(path: &Path, suffix: &str) -> Result<PathBuf, MusicError> {
    let name = path.file_name().ok_or_else(|| {
        MusicError::new(format!("output path has no file name: {}", path.display()))
    })?;
    let mut file_name = name.to_os_string();
    file_name.push(suffix);
    Ok(path.with_file_name(file_name))
}
fn write_temporary(path: &Path, bytes: &[u8]) -> Result<PathBuf, MusicError> {
    let temporary = sibling_path(path, &format!(".tmp.{}", std::process::id()))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| {
            MusicError::new(format!("cannot create {}: {error}", temporary.display()))
        })?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(MusicError::new(format!(
            "cannot write {}: {error}",
            temporary.display()
        )));
    }
    Ok(temporary)
}

// Stage both files before moving either existing output. Roll back a failed paired replacement.
fn export_pair(
    stream_path: &Path,
    stream: &[u8],
    driver_path: &Path,
    driver: &[u8],
) -> Result<(), MusicError> {
    let suffix = format!(".bak.{}", std::process::id());
    let stream_backup = sibling_path(stream_path, &suffix)?;
    let driver_backup = sibling_path(driver_path, &suffix)?;
    for (destination, backup) in [(stream_path, &stream_backup), (driver_path, &driver_backup)] {
        if destination
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty() && !parent.is_dir())
        {
            return Err(MusicError::new(format!(
                "output parent does not exist: {}",
                destination.display()
            )));
        }
        if destination.is_dir() {
            return Err(MusicError::new(format!(
                "output path is a directory: {}",
                destination.display()
            )));
        }
        if backup.exists() {
            return Err(MusicError::new(format!(
                "previous export backup exists; preserve/recover it before exporting: {}",
                backup.display()
            )));
        }
    }
    let stream_temp = write_temporary(stream_path, stream)?;
    let driver_temp = match write_temporary(driver_path, driver) {
        Ok(path) => path,
        Err(error) => {
            let _ = fs::remove_file(&stream_temp);
            return Err(error);
        }
    };
    let had_stream = stream_path.exists();
    let had_driver = driver_path.exists();
    if had_stream {
        if let Err(error) = fs::rename(stream_path, &stream_backup) {
            let _ = fs::remove_file(&stream_temp);
            let _ = fs::remove_file(&driver_temp);
            return Err(MusicError::new(format!(
                "cannot stage existing output {}: {error}",
                stream_path.display()
            )));
        }
    }
    if had_driver {
        if let Err(error) = fs::rename(driver_path, &driver_backup) {
            let rollback = if had_stream {
                fs::rename(&stream_backup, stream_path)
            } else {
                Ok(())
            };
            let _ = fs::remove_file(&stream_temp);
            let _ = fs::remove_file(&driver_temp);
            return Err(MusicError::new(format!(
                "cannot stage existing output {}: {error}{}",
                driver_path.display(),
                rollback
                    .err()
                    .map(|e| format!("; restore {} manually: {e}", stream_backup.display()))
                    .unwrap_or_default()
            )));
        }
    }
    let mut committed_stream = false;
    let commit = fs::rename(&stream_temp, stream_path).and_then(|()| {
        committed_stream = true;
        fs::rename(&driver_temp, driver_path)
    });
    if let Err(error) = commit {
        if committed_stream {
            let _ = fs::remove_file(stream_path);
        }
        let stream_restore = if had_stream {
            fs::rename(&stream_backup, stream_path)
        } else {
            Ok(())
        };
        let driver_restore = if had_driver {
            fs::rename(&driver_backup, driver_path)
        } else {
            Ok(())
        };
        let _ = fs::remove_file(&stream_temp);
        let _ = fs::remove_file(&driver_temp);
        let mut message = format!("cannot replace export files: {error}");
        for (restore, backup) in [
            (stream_restore, &stream_backup),
            (driver_restore, &driver_backup),
        ] {
            if let Err(error) = restore {
                write!(message, "; restore {} manually: {error}", backup.display()).unwrap();
            }
        }
        return Err(MusicError::new(message));
    }
    if had_stream {
        let _ = fs::remove_file(stream_backup);
    }
    if had_driver {
        let _ = fs::remove_file(driver_backup);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(tracker: &mut TrackerUi, key: KeyCode) {
        assert!(tracker.handle_key(key, true));
    }
    fn temp_dir() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sgub_gui_{}_{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn cores() -> [Rc<RefCell<S3w2Sound>>; 2] {
        std::array::from_fn(|_| Rc::new(RefCell::new(S3w2Sound::new())))
    }

    #[test]
    fn cursor_traverses_each_field_and_channels_without_changing_song() {
        let mut tracker = TrackerUi::new(None);
        let before = tracker.song.clone();
        for field in [Field::Instrument, Field::Volume, Field::Effect] {
            key(&mut tracker, KeyCode::ArrowRight);
            assert_eq!(tracker.cursor.field, field);
            assert_eq!(tracker.cursor.channel, 0);
        }
        key(&mut tracker, KeyCode::ArrowRight);
        assert_eq!(
            tracker.cursor,
            Cursor {
                row: 0,
                channel: 1,
                field: Field::Note
            }
        );
        key(&mut tracker, KeyCode::ArrowLeft);
        assert_eq!(tracker.cursor.channel, 0);
        assert_eq!(tracker.cursor.field, Field::Effect);
        key(&mut tracker, KeyCode::Tab);
        assert_eq!(tracker.cursor.channel, 1);
        assert_eq!(tracker.cursor.field, Field::Effect);
        assert!(tracker.handle_input(KeyCode::Tab, true, false, true, false));
        assert_eq!(tracker.cursor.channel, 0);
        key(&mut tracker, KeyCode::End);
        key(&mut tracker, KeyCode::PageUp);
        assert_eq!(tracker.cursor.row, 47);
        key(&mut tracker, KeyCode::Home);
        assert_eq!(tracker.cursor.row, 0);
        tracker.edit_step = 0;
        key(&mut tracker, KeyCode::Enter);
        assert_eq!(tracker.cursor.row, 1);
        key(&mut tracker, KeyCode::Home);
        assert_eq!(tracker.song, before);
        assert!(!tracker.dirty);
    }

    #[test]
    fn nibble_entry_advances_only_on_complete_valid_value_and_navigation_discards_draft() {
        let mut tracker = TrackerUi::new(None);
        tracker.cursor.field = Field::Volume;
        tracker.edit_step = 3;
        key(&mut tracker, KeyCode::KeyA);
        assert_eq!(tracker.selected_cell().volume, None);
        assert_eq!(tracker.cursor.row, 0);
        key(&mut tracker, KeyCode::KeyB);
        assert_eq!(tracker.song.patterns[0].rows[0][0].volume, Some(0xab));
        assert_eq!(tracker.cursor.row, 3);
        key(&mut tracker, KeyCode::Digit1);
        key(&mut tracker, KeyCode::ArrowDown);
        assert_eq!(tracker.high_nibble, None);
        key(&mut tracker, KeyCode::Digit2);
        key(&mut tracker, KeyCode::Digit3);
        assert_eq!(tracker.song.patterns[0].rows[4][0].volume, Some(0x23));
        tracker.cursor.field = Field::Instrument;
        key(&mut tracker, KeyCode::KeyF);
        key(&mut tracker, KeyCode::KeyF);
        assert_eq!(tracker.cursor.row, 7);
        assert_eq!(tracker.selected_cell().instrument, None);
        assert!(tracker.status.contains("does not exist"));
    }

    #[test]
    fn notes_are_field_specific_and_volume_coexists_with_pan_and_delete_is_local() {
        let mut tracker = TrackerUi::new(None);
        tracker.edit_step = 0;
        key(&mut tracker, KeyCode::KeyZ);
        assert_eq!(tracker.selected_cell().note, Some(Note::On(60)));
        assert_eq!(tracker.selected_cell().instrument, Some(0));
        tracker.cursor.field = Field::Volume;
        assert!(!tracker.handle_key(KeyCode::KeyZ, true));
        key(&mut tracker, KeyCode::Digit8);
        key(&mut tracker, KeyCode::Digit0);
        tracker.cursor.field = Field::Effect;
        key(&mut tracker, KeyCode::KeyP);
        key(&mut tracker, KeyCode::KeyF);
        key(&mut tracker, KeyCode::Digit1);
        assert_eq!(tracker.selected_cell().effect, Some(Effect::SetPan(0xf1)));
        assert_eq!(tracker.selected_cell().volume, Some(0x80));
        tracker.cursor.field = Field::Instrument;
        key(&mut tracker, KeyCode::Delete);
        assert_eq!(tracker.selected_cell().instrument, None);
        assert_eq!(tracker.selected_cell().note, Some(Note::On(60)));
        assert_eq!(tracker.selected_cell().volume, Some(0x80));
        assert_eq!(tracker.selected_cell().effect, Some(Effect::SetPan(0xf1)));
        tracker.cursor.field = Field::Effect;
        key(&mut tracker, KeyCode::Delete);
        assert_eq!(tracker.selected_cell().effect, None);
        assert_eq!(tracker.selected_cell().volume, Some(0x80));
    }

    #[test]
    fn tempo_entry_rejects_invalid_value_without_overwriting_existing_effect() {
        let mut tracker = TrackerUi::new(None);
        tracker.edit_step = 0;
        tracker.cursor.field = Field::Effect;
        tracker.selected_cell_mut().effect = Some(Effect::SetPan(0xff));
        key(&mut tracker, KeyCode::KeyT);
        key(&mut tracker, KeyCode::Digit0);
        key(&mut tracker, KeyCode::Digit0);
        assert_eq!(tracker.selected_cell().effect, Some(Effect::SetPan(0xff)));
        key(&mut tracker, KeyCode::Digit7);
        key(&mut tracker, KeyCode::Digit8);
        assert_eq!(tracker.selected_cell().effect, Some(Effect::SetTempo(120)));
        assert!(tracker.song.validate().is_ok());
    }

    #[test]
    fn shortcuts_do_not_leak_into_note_entry_and_focus_can_be_restored_from_keyboard() {
        let mut tracker = TrackerUi::new(None);
        assert!(!tracker.handle_key(KeyCode::KeyZ, false));
        assert!(tracker.handle_input(KeyCode::KeyO, true, true, false, false));
        assert_eq!(tracker.path_action, Some(PathAction::Open));
        assert_eq!(tracker.selected_cell().note, None);
        tracker.path_action = None;
        tracker.pattern_focused = false;
        assert!(!tracker.handle_key(KeyCode::KeyZ, true));
        key(&mut tracker, KeyCode::F6);
        key(&mut tracker, KeyCode::BracketRight);
        assert_eq!(tracker.input_octave, 5);
        assert!(tracker.handle_input(KeyCode::Equal, true, true, false, false));
        assert_eq!(tracker.edit_step, 2);
        key(&mut tracker, KeyCode::KeyQ);
        assert_eq!(tracker.song.patterns[0].rows[0][0].note, Some(Note::On(84)));
        assert_eq!(tracker.cursor.row, 2);
    }

    #[test]
    fn stop_then_restart_resets_both_chips_even_before_any_player_frame() {
        let mut tracker = TrackerUi::new(None);
        let cores = cores();
        for core in &cores {
            core.borrow_mut().write_register(0x8e3, 0xff);
        }
        key(&mut tracker, KeyCode::F8);
        key(&mut tracker, KeyCode::F5);
        assert!(tracker.reset_sgus_pending);
        assert!(tracker.playing);
        tracker.advance_vsync(&cores);
        for core in &cores {
            assert_eq!(core.borrow_mut().read_register(0x8e3), 0);
        }
        assert!(tracker.playing);
        assert_eq!(tracker.frames_advanced, 1);
    }

    #[test]
    fn failed_restart_silences_old_preview_and_nonrepeat_finishes_naturally() {
        let mut tracker = TrackerUi::new(None);
        tracker.song.repeat = false;
        tracker.song.tempo_bpm = 150;
        tracker.song.ticks_per_row = 1;
        tracker.edit_step = 0;
        key(&mut tracker, KeyCode::KeyZ);
        let cores = cores();
        key(&mut tracker, KeyCode::F5);
        for _ in 0..64 {
            tracker.advance_vsync(&cores);
        }
        assert!(tracker.playing);
        tracker.advance_vsync(&cores);
        assert!(!tracker.playing);
        assert_eq!(tracker.status, "Preview finished");
        assert_eq!(cores[0].borrow_mut().read_register(0x803), 0);
        key(&mut tracker, KeyCode::F5);
        tracker.advance_vsync(&cores);
        assert_ne!(cores[0].borrow_mut().read_register(0x803), 0);
        tracker.song.instruments[0] = Instrument::Pcm {
            path: PathBuf::from("missing-preview-asset.wav"),
            base_note: 60,
            loop_start: None,
            volume: 255,
            pan: 255,
            macro_: Default::default(),
        };
        key(&mut tracker, KeyCode::F5);
        assert!(!tracker.playing);
        assert!(tracker.status.starts_with("Preview failed:"));
        tracker.advance_vsync(&cores);
        assert_eq!(cores[0].borrow_mut().read_register(0x803), 0);
        assert!(tracker.status.starts_with("Preview failed:"));
    }

    #[test]
    fn exports_both_files_and_preserves_existing_pair_if_second_stage_fails() {
        let root = temp_dir();
        let stream = root.join("song.sgub");
        let driver = root.join("sgu_music_driver.sol");
        fs::write(&stream, b"old stream").unwrap();
        fs::write(&driver, b"old driver").unwrap();
        export_pair(&stream, b"new stream", &driver, b"new driver").unwrap();
        assert_eq!(fs::read(&stream).unwrap(), b"new stream");
        assert_eq!(fs::read(&driver).unwrap(), b"new driver");
        let occupied = sibling_path(&driver, &format!(".tmp.{}", std::process::id())).unwrap();
        fs::write(&occupied, b"occupied").unwrap();
        assert!(export_pair(&stream, b"replacement", &driver, b"replacement").is_err());
        assert_eq!(fs::read(&stream).unwrap(), b"new stream");
        assert_eq!(fs::read(&driver).unwrap(), b"new driver");
        assert_eq!(fs::read(&occupied).unwrap(), b"occupied");
        assert!(
            !sibling_path(&stream, &format!(".tmp.{}", std::process::id()))
                .unwrap()
                .exists()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn export_confirmation_pins_paths_and_compile_failure_preserves_existing_outputs() {
        let root = temp_dir();
        let mut tracker = TrackerUi::new(None);
        tracker.export_base_input = root.join("song").display().to_string();
        let (stream, driver) = tracker.export_paths();
        fs::write(&stream, b"preserved stream").unwrap();
        fs::write(&driver, b"preserved driver").unwrap();
        tracker.request_export();
        assert_eq!(
            tracker.export_confirmation,
            Some((stream.clone(), driver.clone()))
        );
        tracker.export_base_input = root.join("changed").display().to_string();
        tracker.song.instruments[0] = Instrument::Pcm {
            path: root.join("missing.wav"),
            base_note: 60,
            loop_start: None,
            volume: 255,
            pan: 255,
            macro_: Default::default(),
        };
        let confirmed = tracker.export_confirmation.take().unwrap();
        tracker.export_now(&confirmed.0, &confirmed.1);
        assert!(tracker.status.starts_with("Export failed:"));
        assert_eq!(fs::read(&stream).unwrap(), b"preserved stream");
        assert_eq!(fs::read(&driver).unwrap(), b"preserved driver");
        assert!(!root.join("changed.sgub").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
