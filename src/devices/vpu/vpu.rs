//! FIFO command interpreter and RGBA plane for the VPU.

pub(crate) const VPU_MMIO_BASE: u32 = 0x80030000;
pub(crate) const VPU_MMIO_SIZE: u32 = 0x10000;
pub const VPU_WIDTH: usize = 320;
pub const VPU_HEIGHT: usize = 240;
const FIFO_CAPACITY: usize = 4096;

#[derive(Clone, Copy)]
struct ClipVertex {
    position: [f32; 4],
    color: [f32; 4],
}

pub struct Vpu {
    pub(crate) plane: Vec<u8>,
    depth: Vec<i32>,
    depth_f32: Vec<f32>,
    matrices: [[f32; 16]; 3],
    shading_mode: u32,
    pub(crate) fifo: Vec<u32>,
    pub(crate) command: Vec<u32>,
    pub(crate) expected: Option<usize>,
    pub(crate) status: u32,
    pub(crate) write_latch: u32,
    pub(crate) write_mask: u8,
    pub(crate) output_gp: u8,
}

impl Vpu {
    pub fn new() -> Self {
        Self {
            plane: vec![0; VPU_WIDTH * VPU_HEIGHT * 4],
            depth: vec![i32::MAX; VPU_WIDTH * VPU_HEIGHT],
            depth_f32: vec![1.0; VPU_WIDTH * VPU_HEIGHT],
            matrices: [identity_matrix(); 3],
            shading_mode: 0,
            fifo: Vec::new(),
            command: Vec::new(),
            expected: None,
            status: 0,
            write_latch: 0,
            write_mask: 0,
            output_gp: 7,
        }
    }

    pub fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
        if x >= VPU_WIDTH || y >= VPU_HEIGHT {
            return (0, 0, 0, 0);
        }
        let i = (y * VPU_WIDTH + x) * 4;
        (
            self.plane[i],
            self.plane[i + 1],
            self.plane[i + 2],
            self.plane[i + 3],
        )
    }

    pub(crate) fn push(&mut self, word: u32) {
        if self.fifo.len() == FIFO_CAPACITY {
            self.status |= 1 << 5;
            return;
        }
        self.fifo.push(word);
        self.status |= 1;
        self.pump();
    }

    fn pump(&mut self) {
        while !self.fifo.is_empty() {
            let word = self.fifo.remove(0);
            if self.expected.is_none() {
                let opcode = (word >> 24) as u8;
                let count = (word & 0x00ff_ffff) as usize;
                if opcode == 0 {
                    self.status |= 1 << 2;
                    continue;
                }
                if count > FIFO_CAPACITY {
                    self.status |= 1 << 3;
                    continue;
                }
                self.command.clear();
                self.command.push(opcode as u32);
                self.expected = Some(count);
                if count == 0 {
                    self.execute();
                }
            } else {
                self.command.push(word);
                let left = self.expected.unwrap() - 1;
                self.expected = Some(left);
                if left == 0 {
                    self.execute();
                }
            }
        }
        self.status &= !1;
    }

    fn execute(&mut self) {
        let opcode = self.command[0] as u8;
        let p = self.command[1..].to_vec();
        match opcode {
            0x04 if p.len() == 3 => {
                let flags = p[0];
                if flags & 1 != 0 {
                    let rgba = p[1].to_be_bytes();
                    for px in self.plane.chunks_exact_mut(4) {
                        px.copy_from_slice(&rgba);
                    }
                }
                if flags & 2 != 0 {
                    self.depth.fill(i32::MAX);
                    self.depth_f32.fill(1.0);
                }
                if flags & !3 != 0 {
                    self.status |= 1 << 3;
                }
            }
            0x01 if p.len() == 2 && p[0] == 0 && p[1] <= 1 => {
                self.shading_mode = p[1];
            }
            0x02 if p.len() == 17 && p[0] <= 2 => {
                let mut matrix = [0.0; 16];
                for (dst, bits) in matrix.iter_mut().zip(&p[1..]) {
                    *dst = f32::from_bits(*bits);
                }
                if matrix.iter().all(|value| value.is_finite()) {
                    self.matrices[p[0] as usize] = matrix;
                } else {
                    self.status |= 1 << 3;
                }
            }
            // DRAW_FLAT_TRIANGLE: three (x, y, depth) integer vertices,
            // packed RGB, and an 8-bit diffuse intensity. Color is constant
            // across the face (flat shading); no texture or interpolation.
            0x12 if p.len() == 11 => self.draw_flat_triangle(&p),
            // DRAW_TL_TRIANGLE: three POSITION(float3)+COLOR(RGBA8) vertices.
            0x14 if p.len() == 12 => self.draw_transformed_triangle(&p),
            // The interpreter deliberately rejects commands it cannot safely consume.
            0x01 | 0x02 | 0x03 | 0x10 | 0x11 | 0x13 | 0x7f => self.status |= 1 << 3,
            _ => self.status |= 1 << 3,
        }
        self.expected = None;
        self.command.clear();
    }

    fn draw_flat_triangle(&mut self, p: &[u32]) {
        let x = [p[0] as i32, p[3] as i32, p[6] as i32];
        let y = [p[1] as i32, p[4] as i32, p[7] as i32];
        let z = [p[2] as i32, p[5] as i32, p[8] as i32];
        let rgb = p[9].to_be_bytes();
        let shade = p[10].min(255) as u16;
        let color = [
            (rgb[1] as u16 * shade / 255) as u8,
            (rgb[2] as u16 * shade / 255) as u8,
            (rgb[3] as u16 * shade / 255) as u8,
            255,
        ];
        let edge = |a: usize, b: usize, px: i32, py: i32| {
            (px - x[a]) as i64 * (y[b] - y[a]) as i64 - (py - y[a]) as i64 * (x[b] - x[a]) as i64
        };
        let area = edge(0, 1, x[2], y[2]);
        if area == 0 {
            return;
        }
        let min_x = x.iter().copied().min().unwrap().max(0) as usize;
        let max_x = x.iter().copied().max().unwrap().min(VPU_WIDTH as i32 - 1);
        let min_y = y.iter().copied().min().unwrap().max(0) as usize;
        let max_y = y.iter().copied().max().unwrap().min(VPU_HEIGHT as i32 - 1);
        if max_x < min_x as i32 || max_y < min_y as i32 {
            return;
        }
        for py in min_y..=max_y as usize {
            for px in min_x..=max_x as usize {
                let px_i = px as i32;
                let py_i = py as i32;
                let w0 = edge(1, 2, px_i, py_i);
                let w1 = edge(2, 0, px_i, py_i);
                let w2 = edge(0, 1, px_i, py_i);
                let inside = if area > 0 {
                    w0 >= 0 && w1 >= 0 && w2 >= 0
                } else {
                    w0 <= 0 && w1 <= 0 && w2 <= 0
                };
                if !inside {
                    continue;
                }
                let d = ((w0 * z[0] as i64 + w1 * z[1] as i64 + w2 * z[2] as i64) / area) as i32;
                let index = py * VPU_WIDTH + px;
                if d < self.depth[index] {
                    self.depth[index] = d;
                    self.plane[index * 4..index * 4 + 4].copy_from_slice(&color);
                }
            }
        }
    }

    fn draw_transformed_triangle(&mut self, p: &[u32]) {
        let mut vertices = [ClipVertex {
            position: [0.0; 4],
            color: [0.0; 4],
        }; 3];
        for (i, vertex) in vertices.iter_mut().enumerate() {
            let offset = i * 4;
            let position = [
                f32::from_bits(p[offset]),
                f32::from_bits(p[offset + 1]),
                f32::from_bits(p[offset + 2]),
                1.0,
            ];
            if !position[..3].iter().all(|value| value.is_finite()) {
                self.status |= 1 << 3;
                return;
            }
            vertex.position = transform_position(&self.matrices, position);
            let rgba = p[offset + 3].to_be_bytes();
            vertex.color = rgba.map(|channel| channel as f32 / 255.0);
        }
        if vertices
            .iter()
            .any(|vertex| !vertex.position.iter().all(|value| value.is_finite()))
        {
            self.status |= 1 << 3;
            return;
        }

        let flat_color = vertices[0].color;
        let mut polygon = vertices.to_vec();
        for plane in 0..6 {
            polygon = clip_polygon(polygon, plane);
            if polygon.len() < 3 {
                return;
            }
        }
        for i in 1..polygon.len() - 1 {
            let tri = [polygon[0], polygon[i], polygon[i + 1]];
            let color = if self.shading_mode == 0 {
                Some(flat_color)
            } else {
                None
            };
            self.rasterize_transformed_triangle(tri, color);
        }
    }

    fn rasterize_transformed_triangle(&mut self, tri: [ClipVertex; 3], flat: Option<[f32; 4]>) {
        let mut screen = [[0.0f32; 2]; 3];
        let mut inv_w = [0.0f32; 3];
        let mut depth = [0.0f32; 3];
        for i in 0..3 {
            let w = tri[i].position[3];
            if w <= f32::EPSILON {
                return;
            }
            inv_w[i] = 1.0 / w;
            screen[i] = [
                (tri[i].position[0] * inv_w[i] + 1.0) * (VPU_WIDTH as f32 * 0.5),
                (1.0 - tri[i].position[1] * inv_w[i]) * (VPU_HEIGHT as f32 * 0.5),
            ];
            depth[i] = tri[i].position[2] * inv_w[i];
        }
        let edge = |a: usize, b: usize, x: f32, y: f32| {
            (x - screen[a][0]) * (screen[b][1] - screen[a][1])
                - (y - screen[a][1]) * (screen[b][0] - screen[a][0])
        };
        let area = edge(0, 1, screen[2][0], screen[2][1]);
        if area.abs() <= f32::EPSILON {
            return;
        }
        let min_x = screen
            .iter()
            .map(|p| p[0].floor() as i32)
            .min()
            .unwrap()
            .max(0) as usize;
        let max_x = screen
            .iter()
            .map(|p| p[0].ceil() as i32)
            .max()
            .unwrap()
            .min(VPU_WIDTH as i32 - 1);
        let min_y = screen
            .iter()
            .map(|p| p[1].floor() as i32)
            .min()
            .unwrap()
            .max(0) as usize;
        let max_y = screen
            .iter()
            .map(|p| p[1].ceil() as i32)
            .max()
            .unwrap()
            .min(VPU_HEIGHT as i32 - 1);
        if max_x < min_x as i32 || max_y < min_y as i32 {
            return;
        }
        for y in min_y..=max_y as usize {
            for x in min_x..=max_x as usize {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let weights = [edge(1, 2, px, py), edge(2, 0, px, py), edge(0, 1, px, py)];
                let inside = if area > 0.0 {
                    weights.iter().all(|weight| *weight >= 0.0)
                } else {
                    weights.iter().all(|weight| *weight <= 0.0)
                };
                if !inside {
                    continue;
                }
                let bary = weights.map(|weight| weight / area);
                let z = bary[0] * depth[0] + bary[1] * depth[1] + bary[2] * depth[2];
                let index = y * VPU_WIDTH + x;
                if z < 0.0 || z > 1.0 || z >= self.depth_f32[index] {
                    continue;
                }
                self.depth_f32[index] = z;
                let color = if let Some(flat_color) = flat {
                    flat_color
                } else {
                    let denominator = bary[0] * inv_w[0] + bary[1] * inv_w[1] + bary[2] * inv_w[2];
                    if denominator.abs() <= f32::EPSILON {
                        continue;
                    }
                    std::array::from_fn(|channel| {
                        (bary[0] * tri[0].color[channel] * inv_w[0]
                            + bary[1] * tri[1].color[channel] * inv_w[1]
                            + bary[2] * tri[2].color[channel] * inv_w[2])
                            / denominator
                    })
                };
                let rgba = color.map(|value| (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                self.plane[index * 4..index * 4 + 4].copy_from_slice(&rgba);
            }
        }
    }
}

fn identity_matrix() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

fn transform_position(matrices: &[[f32; 16]; 3], mut position: [f32; 4]) -> [f32; 4] {
    for matrix in matrices {
        let input = position;
        for row in 0..4 {
            position[row] = (0..4).map(|col| matrix[row * 4 + col] * input[col]).sum();
        }
    }
    position
}

fn clip_distance(vertex: ClipVertex, plane: usize) -> f32 {
    let [x, y, z, w] = vertex.position;
    match plane {
        0 => x + w,
        1 => w - x,
        2 => y + w,
        3 => w - y,
        4 => z,
        _ => w - z,
    }
}

fn clip_polygon(polygon: Vec<ClipVertex>, plane: usize) -> Vec<ClipVertex> {
    let Some(mut previous) = polygon.last().copied() else {
        return polygon;
    };
    let mut previous_distance = clip_distance(previous, plane);
    let mut output = Vec::with_capacity(polygon.len() + 1);
    for current in polygon {
        let current_distance = clip_distance(current, plane);
        let previous_inside = previous_distance >= 0.0;
        let current_inside = current_distance >= 0.0;
        if previous_inside != current_inside {
            let t = previous_distance / (previous_distance - current_distance);
            output.push(ClipVertex {
                position: std::array::from_fn(|i| {
                    previous.position[i] + t * (current.position[i] - previous.position[i])
                }),
                color: std::array::from_fn(|i| {
                    previous.color[i] + t * (current.color[i] - previous.color[i])
                }),
            });
        }
        if current_inside {
            output.push(current);
        }
        previous = current;
        previous_distance = current_distance;
    }
    output
}
