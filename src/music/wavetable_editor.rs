use imgui::{Condition, MouseButton, Ui};

#[derive(Default)]
pub struct WavetableEditor {
    pub open: bool,
    selected: usize,
    stroke: Option<(usize, u8)>,
    hexadecimal: bool,
    gain: f32,
    offset: i32,
    phase: i32,
    levels: i32,
}

impl WavetableEditor {
    pub fn new() -> Self {
        Self {
            gain: 1.0,
            levels: 256,
            ..Self::default()
        }
    }

    pub fn preview(&mut self, ui: &Ui, samples: &mut [u8]) -> bool {
        ui.text("WAVETABLE / 256 samples / unsigned 8-bit");
        ui.same_line();
        if ui.button("Open Wavetable Editor") {
            self.open = true;
        }
        self.canvas(ui, samples, [ui.content_region_avail()[0].max(80.0), 120.0])
    }

    pub fn draw(&mut self, ui: &Ui, samples: &mut [u8]) -> bool {
        if !self.open {
            return false;
        }
        let mut open = self.open;
        let mut changed = false;
        ui.window("Wavetable Editor")
            .opened(&mut open)
            .bg_alpha(1.0)
            .position([80.0, 110.0], Condition::FirstUseEver)
            .size([800.0, 480.0], Condition::FirstUseEver)
            .size_constraints([480.0, 340.0], [f32::MAX, f32::MAX])
            .build(|| {
                ui.text("Width 256 / Height 256 / draw with left mouse, drag to edit samples");
                ui.checkbox("Hexadecimal values", &mut self.hexadecimal);
                let available = ui.content_region_avail();
                let tool_width = 175.0;
                let graph_width = (available[0] - tool_width - 12.0).max(100.0);
                let graph_height = (available[1] - 95.0).max(120.0);
                ui.child_window("Wave plot")
                    .size([graph_width, graph_height])
                    .build(|| {
                        changed |=
                            self.canvas(ui, samples, [graph_width - 8.0, graph_height - 8.0]);
                    });
                ui.same_line();
                ui.child_window("Wave tools")
                    .size([0.0, graph_height])
                    .build(|| {
                        ui.text("SHAPES");
                        for (label, shape) in [
                            ("Sine", Shape::Sine),
                            ("Triangle", Shape::Triangle),
                            ("Saw", Shape::Saw),
                            ("Square", Shape::Square),
                        ] {
                            if ui.button(label) {
                                fill_shape(samples, shape);
                                changed = true;
                            }
                            if label == "Sine" || label == "Saw" {
                                ui.same_line();
                            }
                        }
                        ui.separator();
                        ui.text("WAVE TOOLS");
                        ui.set_next_item_width(65.0);
                        ui.input_scalar("Gain", &mut self.gain).build();
                        ui.set_next_item_width(65.0);
                        ui.input_scalar("Offset Y", &mut self.offset).build();
                        if ui.button("Apply amplitude") {
                            if self.gain.is_finite() {
                                for value in samples.iter_mut() {
                                    *value = ((f32::from(*value) - 128.0) * self.gain
                                        + 128.0
                                        + self.offset as f32)
                                        .round()
                                        .clamp(0.0, 255.0)
                                        as u8;
                                }
                                changed = true;
                            }
                        }
                        ui.set_next_item_width(65.0);
                        ui.input_scalar("Offset X", &mut self.phase).build();
                        if ui.button("Shift phase") {
                            samples.rotate_right(self.phase.rem_euclid(256) as usize);
                            changed = true;
                        }
                        if ui.button("Normalize") {
                            normalize(samples);
                            changed = true;
                        }
                        if ui.button("Invert") {
                            for value in samples.iter_mut() {
                                *value = 255 - *value;
                            }
                            changed = true;
                        }
                        ui.same_line();
                        if ui.button("Reverse") {
                            samples.reverse();
                            changed = true;
                        }
                        if ui.button("Smooth") {
                            smooth(samples);
                            changed = true;
                        }
                        ui.same_line();
                        if ui.button("Double") {
                            let original: [u8; 256] =
                                samples.try_into().expect("validated wavetable");
                            for (index, value) in samples.iter_mut().enumerate() {
                                *value = original[(index * 2) % 256];
                            }
                            changed = true;
                        }
                        ui.set_next_item_width(65.0);
                        ui.input_scalar("Levels", &mut self.levels).build();
                        if ui.button("Quantize") {
                            let intervals = self.levels.clamp(2, 256) - 1;
                            for value in samples.iter_mut() {
                                let level = (f32::from(*value) * intervals as f32 / 255.0).round();
                                *value = (level * 255.0 / intervals as f32).round() as u8;
                            }
                            changed = true;
                        }
                    });
                let mut index = self.selected as i32;
                ui.set_next_item_width(65.0);
                if ui.input_scalar("Sample index", &mut index).build() {
                    self.selected = index.clamp(0, 255) as usize;
                }
                ui.same_line();
                let mut value = i32::from(samples[self.selected]);
                ui.set_next_item_width(65.0);
                let changed_value = if self.hexadecimal {
                    ui.input_scalar("Value", &mut value)
                        .display_format("%02X")
                        .chars_hexadecimal(true)
                        .build()
                } else {
                    ui.input_scalar("Value", &mut value).build()
                };
                if changed_value {
                    samples[self.selected] = value.clamp(0, 255) as u8;
                    changed = true;
                }
                ui.child_window("Wave sample values")
                    .size([0.0, 34.0])
                    .horizontal_scrollbar(true)
                    .build(|| {
                        for (index, value) in samples.iter().enumerate() {
                            if index != 0 {
                                ui.same_line();
                            }
                            let label = if self.hexadecimal {
                                format!("{value:02X}##wave{index}")
                            } else {
                                format!("{value:03}##wave{index}")
                            };
                            if ui
                                .selectable_config(label)
                                .size([26.0, 16.0])
                                .selected(index == self.selected)
                                .build()
                            {
                                self.selected = index;
                            }
                            if ui.is_item_hovered() {
                                ui.tooltip_text(format!("Sample {index}: {value}"));
                            }
                        }
                    });
            });
        self.open = open;
        changed
    }

    fn canvas(&mut self, ui: &Ui, samples: &mut [u8], size: [f32; 2]) -> bool {
        let origin = ui.cursor_screen_pos();
        let clicked = ui.invisible_button("Waveform canvas", size);
        let active = ui.is_item_active();
        let hovered = ui.is_item_hovered();
        let mut changed = false;
        if (clicked || active) && ui.is_mouse_down(MouseButton::Left) {
            let mouse = ui.io().mouse_pos;
            let index = (((mouse[0] - origin[0]) / size[0]) * 256.0)
                .floor()
                .clamp(0.0, 255.0) as usize;
            let value = ((1.0 - (mouse[1] - origin[1]) / size[1]) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8;
            paint_stroke(samples, self.stroke, (index, value));
            self.stroke = Some((index, value));
            self.selected = index;
            changed = true;
        } else if !ui.is_mouse_down(MouseButton::Left) {
            self.stroke = None;
        }
        let draw = ui.get_window_draw_list();
        let bottom = origin[1] + size[1];
        draw.add_rect(
            origin,
            [origin[0] + size[0], bottom],
            [0.08, 0.13, 0.19, 1.0],
        )
        .filled(true)
        .build();
        for value in [0, 64, 128, 192, 255] {
            let y = bottom - value as f32 / 255.0 * size[1];
            draw.add_line(
                [origin[0], y],
                [origin[0] + size[0], y],
                [0.2, 0.27, 0.34, 1.0],
            )
            .build();
        }
        let dx = size[0] / 256.0;
        for (index, value) in samples.iter().enumerate() {
            let x = origin[0] + index as f32 * dx;
            let y = bottom - f32::from(*value) / 255.0 * size[1];
            draw.add_rect([x, y], [x + dx, bottom], [0.15, 0.29, 0.4, 0.7])
                .filled(true)
                .build();
            draw.add_line([x, y], [x + dx, y], [0.65, 0.9, 1.0, 1.0])
                .thickness(1.5)
                .build();
            if index < 255 {
                let next_y = bottom - f32::from(samples[index + 1]) / 255.0 * size[1];
                draw.add_line([x + dx, y], [x + dx, next_y], [0.65, 0.9, 1.0, 1.0])
                    .build();
            }
        }
        let selected_x = origin[0] + (self.selected as f32 + 0.5) * dx;
        draw.add_line(
            [selected_x, origin[1]],
            [selected_x, bottom],
            [1.0, 0.7, 0.25, 1.0],
        )
        .build();
        if hovered {
            let index = (((ui.io().mouse_pos[0] - origin[0]) / size[0]) * 256.0)
                .floor()
                .clamp(0.0, 255.0) as usize;
            ui.tooltip_text(format!(
                "{index:03} / 0x{index:02X}: {} / 0x{:02X}",
                samples[index], samples[index]
            ));
        }
        changed
    }
}

#[derive(Clone, Copy)]
enum Shape {
    Sine,
    Triangle,
    Saw,
    Square,
}

fn fill_shape(samples: &mut [u8], shape: Shape) {
    for (index, value) in samples.iter_mut().enumerate() {
        *value = match shape {
            Shape::Sine => {
                (128.0 + 127.0 * (std::f64::consts::TAU * index as f64 / 256.0).sin()).round() as u8
            }
            Shape::Triangle => {
                if index < 128 {
                    (index * 2) as u8
                } else {
                    (255 - (index - 128) * 2) as u8
                }
            }
            Shape::Saw => index as u8,
            Shape::Square => {
                if index < 128 {
                    255
                } else {
                    0
                }
            }
        };
    }
}

fn paint_stroke(samples: &mut [u8], previous: Option<(usize, u8)>, current: (usize, u8)) {
    if let Some((previous_index, previous_value)) = previous {
        let start = previous_index.min(current.0);
        let end = previous_index.max(current.0);
        if start != end {
            for (index, value) in samples.iter_mut().enumerate().take(end + 1).skip(start) {
                let fraction = (index as f32 - previous_index as f32)
                    / (current.0 as f32 - previous_index as f32);
                *value = (f32::from(previous_value)
                    + fraction * (f32::from(current.1) - f32::from(previous_value)))
                .round() as u8;
            }
        }
    }
    samples[current.0] = current.1;
}

fn normalize(samples: &mut [u8]) {
    let min = *samples.iter().min().unwrap();
    let max = *samples.iter().max().unwrap();
    if min == max {
        return;
    }
    for value in samples {
        *value = (u32::from(*value - min) * 255 / u32::from(max - min)) as u8;
    }
}

fn smooth(samples: &mut [u8]) {
    let original: [u8; 256] = (&*samples).try_into().expect("validated wavetable");
    for (index, value) in samples.iter_mut().enumerate() {
        *value = ((u16::from(original[(index + 255) % 256])
            + u16::from(original[index])
            + u16::from(original[(index + 1) % 256]))
            / 3) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dragging_in_either_direction_interpolates_all_crossed_samples() {
        let mut samples = [128; 256];
        paint_stroke(&mut samples, Some((4, 0)), (8, 200));
        assert_eq!(&samples[4..=8], &[0, 50, 100, 150, 200]);
        paint_stroke(&mut samples, Some((8, 200)), (4, 0));
        assert_eq!(&samples[4..=8], &[0, 50, 100, 150, 200]);
        assert_eq!(samples[3], 128);
        assert_eq!(samples[9], 128);
    }
    #[test]
    fn normalized_and_smoothed_tables_preserve_unsigned_range_and_wrap_neighbors() {
        let mut samples = [100; 256];
        samples[0] = 200;
        normalize(&mut samples);
        assert_eq!(samples[0], 255);
        assert_eq!(samples[1], 0);
        smooth(&mut samples);
        assert_eq!(samples[0], 85);
        assert_eq!(samples[1], 85);
        assert_eq!(samples[255], 85);
        assert_eq!(samples[128], 0);
    }
}
