//! FIFO command interpreter and RGBA plane for the VPU.

pub(crate) const VPU_MMIO_BASE: u32 = 0x80030000;
pub(crate) const VPU_MMIO_SIZE: u32 = 0x10000;
pub const VPU_WIDTH: usize = 320;
pub const VPU_HEIGHT: usize = 240;
const FIFO_CAPACITY: usize = 4096;
const MAX_COMMAND_WORDS: usize = 3 + 1024 * 1024 / 2;

#[derive(Clone, Copy)]
struct ClipVertex {
    position: [f32; 4],
    view_position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 4],
    uv: [f32; 2],
}

#[derive(Clone, Copy)]
struct Light {
    enabled: bool,
    ambient: [f32; 3],
    diffuse: [f32; 3],
    specular: [f32; 3],
    emission: [f32; 3],
    position: [f32; 3],
}

impl Light {
    const DISABLED: Self = Self {
        enabled: false,
        ambient: [0.0; 3],
        diffuse: [0.0; 3],
        specular: [0.0; 3],
        emission: [0.0; 3],
        position: [0.0; 3],
    };
}

pub struct Vpu {
    pub(crate) plane: Vec<u8>,
    depth: Vec<i32>,
    depth_f32: Vec<f32>,
    matrices: [[f32; 16]; 3],
    shading_mode: u32,
    lighting_enabled: bool,
    lights: [Light; 8],
    textures: std::collections::HashMap<u32, (usize, usize, Vec<u8>)>,
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
            lighting_enabled: false,
            lights: [Light::DISABLED; 8],
            textures: std::collections::HashMap::new(),
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
                if count > MAX_COMMAND_WORDS {
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
            0x01 if p.len() == 2 && p[0] == 1 && p[1] <= 1 => {
                self.lighting_enabled = p[1] != 0;
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
            0x15 if p.len() == 17 && p[0] < 8 && p[1] <= 1 => {
                let values: Vec<f32> = p[2..].iter().map(|bits| f32::from_bits(*bits)).collect();
                if values.iter().all(|value| value.is_finite()) {
                    let light = Light {
                        enabled: p[1] != 0,
                        ambient: [values[0], values[1], values[2]],
                        diffuse: [values[3], values[4], values[5]],
                        specular: [values[6], values[7], values[8]],
                        emission: [values[9], values[10], values[11]],
                        position: [values[12], values[13], values[14]],
                    };
                    self.lights[p[0] as usize] = light;
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
            // DRAW_TL_LIT_TRIANGLE: three POSITION(float3)+NORMAL(float3)+COLOR(RGBA8).
            0x16 if p.len() == 21 => self.draw_transformed_triangle_with_normals(&p),
            // TEXTURE_UPLOAD: id, width, height, then two RGBA4444 texels per word.
            0x17 if p.len() >= 3 => self.upload_texture(&p),
            // DRAW_TEXTURED_TRIANGLE: texture id, then 3x position(float3), uv(float2), rgba8.
            0x18 if p.len() == 20 => self.draw_textured_triangle(&p),
            // The interpreter deliberately rejects commands it cannot safely consume.
            0x01 | 0x02 | 0x03 | 0x10 | 0x11 | 0x13 | 0x15 | 0x7f => self.status |= 1 << 3,
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
        self.draw_transformed_triangle_impl(p, false);
    }

    fn draw_transformed_triangle_with_normals(&mut self, p: &[u32]) {
        self.draw_transformed_triangle_impl(p, true);
    }

    fn upload_texture(&mut self, p: &[u32]) {
        let (id, width, height) = (p[0], p[1] as usize, p[2] as usize);
        let pixels = width.saturating_mul(height);
        if width == 0
            || height == 0
            || width > 1024
            || height > 1024
            || p.len() != 3 + (pixels + 1) / 2
        {
            self.status |= 1 << 3;
            return;
        }
        let mut rgba = Vec::with_capacity(pixels * 4);
        for word in &p[3..] {
            for texel in [(*word >> 16) as u16, *word as u16] {
                if rgba.len() / 4 == pixels {
                    break;
                }
                for nibble in [
                    (texel >> 12) as u8,
                    (texel >> 8) as u8 & 15,
                    (texel >> 4) as u8 & 15,
                    texel as u8 & 15,
                ] {
                    rgba.push(nibble * 17);
                }
            }
        }
        self.textures.insert(id, (width, height, rgba));
    }

    fn draw_textured_triangle(&mut self, p: &[u32]) {
        let mode = p[1];
        if mode > 2 {
            self.status |= 1 << 3;
            return;
        }
        let Some((width, height, data)) = self.textures.get(&p[0]) else {
            self.status |= 1 << 3;
            return;
        };
        let (width, height, data) = (*width, *height, data.clone());
        let mut vertices = [ClipVertex {
            position: [0.0; 4],
            view_position: [0.0; 3],
            normal: [0.0; 3],
            color: [0.0; 4],
            uv: [0.0; 2],
        }; 3];
        for i in 0..3 {
            let o = 2 + i * 6;
            let pos = [
                f32::from_bits(p[o]),
                f32::from_bits(p[o + 1]),
                f32::from_bits(p[o + 2]),
                1.0,
            ];
            let model = transform_matrix(&self.matrices[0], pos);
            let view = transform_matrix(&self.matrices[1], model);
            vertices[i].position = transform_matrix(&self.matrices[2], view);
            vertices[i].view_position = [view[0], view[1], view[2]];
            vertices[i].uv = [f32::from_bits(p[o + 3]), f32::from_bits(p[o + 4])];
            vertices[i].color = p[o + 5].to_be_bytes().map(|c| c as f32 / 255.0);
            if !vertices[i]
                .position
                .iter()
                .chain(vertices[i].uv.iter())
                .all(|v| v.is_finite())
            {
                self.status |= 1 << 3;
                return;
            }
        }
        let edge_a: [f32; 3] = std::array::from_fn(|axis| {
            vertices[1].view_position[axis] - vertices[0].view_position[axis]
        });
        let edge_b: [f32; 3] = std::array::from_fn(|axis| {
            vertices[2].view_position[axis] - vertices[0].view_position[axis]
        });
        let face_normal = normalize([
            edge_a[1] * edge_b[2] - edge_a[2] * edge_b[1],
            edge_a[2] * edge_b[0] - edge_a[0] * edge_b[2],
            edge_a[0] * edge_b[1] - edge_a[1] * edge_b[0],
        ]);
        if self.lighting_enabled && mode == 1 {
            vertices[0].color = shade_vertex(
                vertices[0].color,
                vertices[0].view_position,
                face_normal,
                &self.lights,
            );
        } else if self.lighting_enabled && mode == 2 {
            for vertex in &mut vertices {
                vertex.color = shade_vertex(
                    vertex.color,
                    vertex.view_position,
                    face_normal,
                    &self.lights,
                );
            }
        }
        let mut polygon = vertices.to_vec();
        for plane in 0..6 {
            polygon = clip_polygon(polygon, plane);
            if polygon.len() < 3 {
                return;
            }
        }
        let flat_color = vertices[0].color;
        for i in 1..polygon.len() - 1 {
            self.rasterize_textured_triangle(
                [polygon[0], polygon[i], polygon[i + 1]],
                width,
                height,
                &data,
                mode,
                flat_color,
            );
        }
    }

    fn rasterize_textured_triangle(
        &mut self,
        tri: [ClipVertex; 3],
        width: usize,
        height: usize,
        data: &[u8],
        mode: u32,
        flat_color: [f32; 4],
    ) {
        let mut screen = [[0.0f32; 2]; 3];
        let mut inv_w = [0.0; 3];
        let mut depth = [0.0; 3];
        for i in 0..3 {
            let w = tri[i].position[3];
            if w <= f32::EPSILON {
                return;
            }
            inv_w[i] = 1.0 / w;
            screen[i] = [
                (tri[i].position[0] * inv_w[i] + 1.0) * VPU_WIDTH as f32 * 0.5,
                (1.0 - tri[i].position[1] * inv_w[i]) * VPU_HEIGHT as f32 * 0.5,
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
        let minx = screen
            .iter()
            .map(|p| p[0].floor() as i32)
            .min()
            .unwrap()
            .max(0) as usize;
        let maxx = screen
            .iter()
            .map(|p| p[0].ceil() as i32)
            .max()
            .unwrap()
            .min(VPU_WIDTH as i32 - 1);
        let miny = screen
            .iter()
            .map(|p| p[1].floor() as i32)
            .min()
            .unwrap()
            .max(0) as usize;
        let maxy = screen
            .iter()
            .map(|p| p[1].ceil() as i32)
            .max()
            .unwrap()
            .min(VPU_HEIGHT as i32 - 1);
        if maxx < minx as i32 || maxy < miny as i32 {
            return;
        }
        for y in miny..=maxy as usize {
            for x in minx..=maxx as usize {
                let weights = [
                    edge(1, 2, x as f32 + 0.5, y as f32 + 0.5),
                    edge(2, 0, x as f32 + 0.5, y as f32 + 0.5),
                    edge(0, 1, x as f32 + 0.5, y as f32 + 0.5),
                ];
                if !weights
                    .iter()
                    .all(|w| if area > 0.0 { *w >= 0.0 } else { *w <= 0.0 })
                {
                    continue;
                }
                let b = weights.map(|w| w / area);
                let z = b[0] * depth[0] + b[1] * depth[1] + b[2] * depth[2];
                let idx = y * VPU_WIDTH + x;
                if z < 0.0 || z > 1.0 || z >= self.depth_f32[idx] {
                    continue;
                }
                self.depth_f32[idx] = z;
                let interp = |f: fn(ClipVertex) -> [f32; 2]| {
                    std::array::from_fn::<_, 2, _>(|c| (0..3).map(|i| b[i] * f(tri[i])[c]).sum())
                };
                let uv: [f32; 2] = interp(|v| v.uv);
                let c: [f32; 4] = if mode == 1 {
                    flat_color
                } else {
                    std::array::from_fn(|ch| {
                        (0..3)
                            .map(|i| b[i] * tri[i].color[ch])
                            .sum::<f32>()
                            .clamp(0.0, 1.0)
                    })
                };
                let tx = (uv[0] * width as f32)
                    .floor()
                    .clamp(0.0, (width - 1) as f32) as usize;
                let ty = (uv[1] * height as f32)
                    .floor()
                    .clamp(0.0, (height - 1) as f32) as usize;
                let ti = (ty * width + tx) * 4;
                let rgba: [u8; 4] = std::array::from_fn(|ch| {
                    let t = data[ti + ch] as f32 / 255.0;
                    let v = if mode == 0 { t } else { t * c[ch] };
                    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
                });
                self.plane[idx * 4..idx * 4 + 4].copy_from_slice(&rgba);
            }
        }
    }

    fn draw_transformed_triangle_impl(&mut self, p: &[u32], has_normal: bool) {
        let stride = if has_normal { 7 } else { 4 };
        let model_view = multiply_matrix(&self.matrices[1], &self.matrices[0]);
        let normal_matrix = if has_normal {
            let Some(matrix) = inverse_transpose_3x3(&model_view) else {
                self.status |= 1 << 3;
                return;
            };
            matrix
        } else {
            [[0.0, 0.0, 0.0]; 3]
        };
        let mut vertices = [ClipVertex {
            position: [0.0; 4],
            view_position: [0.0; 3],
            normal: [0.0; 3],
            color: [0.0; 4],
            uv: [0.0; 2],
        }; 3];
        for (i, vertex) in vertices.iter_mut().enumerate() {
            let offset = i * stride;
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
            let model_position = transform_matrix(&self.matrices[0], position);
            let view_position = transform_matrix(&self.matrices[1], model_position);
            vertex.position = transform_matrix(&self.matrices[2], view_position);
            vertex.view_position = [view_position[0], view_position[1], view_position[2]];
            if has_normal {
                let normal = [
                    f32::from_bits(p[offset + 3]),
                    f32::from_bits(p[offset + 4]),
                    f32::from_bits(p[offset + 5]),
                ];
                if !normal.iter().all(|value| value.is_finite()) {
                    self.status |= 1 << 3;
                    return;
                }
                vertex.normal = normalize(apply_normal_matrix(&normal_matrix, normal));
            }
            let rgba = p[offset + if has_normal { 6 } else { 3 }].to_be_bytes();
            vertex.color = rgba.map(|channel| channel as f32 / 255.0);
        }
        if vertices
            .iter()
            .any(|vertex| !vertex.position.iter().all(|value| value.is_finite()))
        {
            self.status |= 1 << 3;
            return;
        }

        if has_normal && self.lighting_enabled {
            if self.shading_mode == 0 {
                let normal = normalize(std::array::from_fn(|axis| {
                    vertices
                        .iter()
                        .map(|vertex| vertex.normal[axis])
                        .sum::<f32>()
                }));
                // Use the first vertex as the flat-shading sample point. The two
                // triangles of a quad share this anchor, so point-light terms
                // remain identical across their common face.
                let position = vertices[0].view_position;
                let color = shade_vertex(vertices[0].color, position, normal, &self.lights);
                for vertex in &mut vertices {
                    vertex.color = color;
                }
            } else {
                for vertex in &mut vertices {
                    vertex.color = shade_vertex(
                        vertex.color,
                        vertex.view_position,
                        vertex.normal,
                        &self.lights,
                    );
                }
            }
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

fn transform_matrix(matrix: &[f32; 16], position: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|row| {
        (0..4)
            .map(|col| matrix[row * 4 + col] * position[col])
            .sum()
    })
}

fn multiply_matrix(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    std::array::from_fn(|index| {
        let row = index / 4;
        let col = index % 4;
        (0..4).map(|k| a[row * 4 + k] * b[k * 4 + col]).sum()
    })
}

fn inverse_transpose_3x3(matrix: &[f32; 16]) -> Option<[[f32; 3]; 3]> {
    let a = matrix[0];
    let b = matrix[1];
    let c = matrix[2];
    let d = matrix[4];
    let e = matrix[5];
    let f = matrix[6];
    let g = matrix[8];
    let h = matrix[9];
    let i = matrix[10];
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if !det.is_finite() || det.abs() <= f32::EPSILON {
        return None;
    }
    let inv_det = 1.0 / det;
    let inverse = [
        [
            (e * i - f * h) * inv_det,
            (c * h - b * i) * inv_det,
            (b * f - c * e) * inv_det,
        ],
        [
            (f * g - d * i) * inv_det,
            (a * i - c * g) * inv_det,
            (c * d - a * f) * inv_det,
        ],
        [
            (d * h - e * g) * inv_det,
            (b * g - a * h) * inv_det,
            (a * e - b * d) * inv_det,
        ],
    ];
    Some(std::array::from_fn(|row| {
        std::array::from_fn(|col| inverse[col][row])
    }))
}

fn apply_normal_matrix(matrix: &[[f32; 3]; 3], normal: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|row| (0..3).map(|col| matrix[row][col] * normal[col]).sum())
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length.is_finite() && length > f32::EPSILON {
        vector.map(|value| value / length)
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn shade_vertex(
    albedo: [f32; 4],
    position: [f32; 3],
    normal: [f32; 3],
    lights: &[Light; 8],
) -> [f32; 4] {
    let view = normalize([-position[0], -position[1], -position[2]]);
    let mut rgb = [0.0; 3];
    for light in lights.iter().filter(|light| light.enabled) {
        let to_light = normalize([
            light.position[0] - position[0],
            light.position[1] - position[1],
            light.position[2] - position[2],
        ]);
        let ndotl = (0..3)
            .map(|i| normal[i] * to_light[i])
            .sum::<f32>()
            .max(0.0);
        let half_vector = normalize(std::array::from_fn(|i| to_light[i] + view[i]));
        let ndoth = (0..3)
            .map(|i| normal[i] * half_vector[i])
            .sum::<f32>()
            .max(0.0);
        let specular = if ndotl > 0.0 { ndoth.powi(16) } else { 0.0 };
        for channel in 0..3 {
            rgb[channel] += albedo[channel] * light.ambient[channel]
                + albedo[channel] * light.diffuse[channel] * ndotl
                + light.specular[channel] * specular
                + light.emission[channel];
        }
    }
    [rgb[0], rgb[1], rgb[2], albedo[3]]
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
                view_position: std::array::from_fn(|i| {
                    previous.view_position[i]
                        + t * (current.view_position[i] - previous.view_position[i])
                }),
                normal: normalize(std::array::from_fn(|i| {
                    previous.normal[i] + t * (current.normal[i] - previous.normal[i])
                })),
                color: std::array::from_fn(|i| {
                    previous.color[i] + t * (current.color[i] - previous.color[i])
                }),
                uv: std::array::from_fn(|i| previous.uv[i] + t * (current.uv[i] - previous.uv[i])),
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

#[cfg(test)]
mod texture_tests {
    use super::*;

    fn send(vpu: &mut Vpu, opcode: u8, payload: &[u32]) {
        vpu.push(((opcode as u32) << 24) | payload.len() as u32);
        for word in payload {
            vpu.push(*word);
        }
    }

    #[test]
    fn upload_unpacks_rgba4444_and_expands_nibbles() {
        let mut vpu = Vpu::new();
        send(&mut vpu, 0x17, &[7, 2, 1, 0x1234_abcd]);
        assert_eq!(
            vpu.textures[&7],
            (2, 1, vec![0x11, 0x22, 0x33, 0x44, 0xaa, 0xbb, 0xcc, 0xdd])
        );
        assert_eq!(vpu.status & (1 << 3), 0);
    }

    #[test]
    fn invalid_upload_preserves_previous_texture_and_sets_error() {
        let mut vpu = Vpu::new();
        send(&mut vpu, 0x17, &[1, 1, 1, 0x1234_0000]);
        let old = vpu.textures[&1].clone();
        send(&mut vpu, 0x17, &[1, 1025, 1, 0]);
        assert_eq!(vpu.textures[&1], old);
        assert_ne!(vpu.status & (1 << 3), 0);
    }

    #[test]
    fn textured_triangle_samples_rgba_and_unknown_id_errors() {
        let mut vpu = Vpu::new();
        vpu.textures.insert(4, (1, 1, vec![200, 100, 50, 255]));
        let screen = [[20.0, 20.0], [100.0, 20.0], [20.0, 100.0]];
        let tri = std::array::from_fn(|i| ClipVertex {
            position: [
                screen[i][0] / 160.0 - 1.0,
                1.0 - screen[i][1] / 120.0,
                0.5,
                1.0,
            ],
            view_position: [0.0; 3],
            normal: [0.0; 3],
            color: [1.0; 4],
            uv: [0.5; 2],
        });
        let texture_data = vpu.textures[&4].2.clone();
        vpu.rasterize_textured_triangle(tri, 1, 1, &texture_data, 0, [1.0; 4]);
        assert_eq!(vpu.pixel(30, 30), (200, 100, 50, 255));
        let mut command = vec![99, 0];
        command.extend([0, 0, 0, 0, 0, 0xffff_ffff].repeat(3));
        send(&mut vpu, 0x18, &command);
        assert_ne!(vpu.status & (1 << 3), 0);
    }

    #[test]
    fn gouraud_modulates_rgba_and_clamps_uv_to_edge() {
        let mut vpu = Vpu::new();
        let screen = [[20.0, 20.0], [100.0, 20.0], [20.0, 100.0]];
        let tri = std::array::from_fn(|i| ClipVertex {
            position: [
                screen[i][0] / 160.0 - 1.0,
                1.0 - screen[i][1] / 120.0,
                0.5,
                1.0,
            ],
            view_position: [0.0; 3],
            normal: [0.0; 3],
            color: [0.5; 4],
            uv: [2.0, -1.0],
        });
        vpu.rasterize_textured_triangle(tri, 1, 1, &[200, 100, 50, 255], 2, [1.0; 4]);
        assert_eq!(vpu.pixel(30, 30), (100, 50, 25, 127));
    }

    #[test]
    fn flat_uses_vertex_zero_color_for_entire_triangle() {
        let mut vpu = Vpu::new();
        let screen = [[20.0, 20.0], [100.0, 20.0], [20.0, 100.0]];
        let colors = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]];
        let tri = std::array::from_fn(|i| ClipVertex {
            position: [
                screen[i][0] / 160.0 - 1.0,
                1.0 - screen[i][1] / 120.0,
                0.5,
                1.0,
            ],
            view_position: [0.0; 3],
            normal: [0.0; 3],
            color: colors[i],
            uv: [0.5; 2],
        });
        vpu.rasterize_textured_triangle(tri, 1, 1, &[255; 4], 1, tri[0].color);
        assert_eq!(vpu.pixel(30, 30), (255, 0, 0, 255));
    }

    #[test]
    fn textured_flat_triangle_uses_enabled_vpu_light() {
        let mut vpu = Vpu::new();
        vpu.textures.insert(3, (1, 1, vec![255; 4]));
        vpu.lighting_enabled = true;
        vpu.lights[0] = Light {
            enabled: true,
            ambient: [0.0; 3],
            diffuse: [0.25; 3],
            specular: [0.0; 3],
            emission: [0.0; 3],
            position: [0.0, 0.0, 2.0],
        };
        let mut payload = vec![3, 1];
        for [x, y] in [[-0.8f32, -0.8f32], [0.8, -0.8], [-0.8, 0.8]] {
            payload.extend([
                x.to_bits(),
                y.to_bits(),
                0.5f32.to_bits(),
                0.5f32.to_bits(),
                0.5f32.to_bits(),
                0xffff_ffff,
            ]);
        }
        send(&mut vpu, 0x18, &payload);
        let lit = vpu.pixel(80, 100);
        assert!(lit.0 > 40 && lit.0 < 100, "expected diffuse illumination, got {lit:?}");
        assert_eq!(lit.0, lit.1);
        assert_eq!(lit.1, lit.2);
    }
}
