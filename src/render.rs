use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use cpt32::devices::vdp::vdp::Vdp;
use wgpu::{
    Color, ColorTargetState, ColorWrites, CommandEncoderDescriptor, DeviceDescriptor, Extent3d,
    Features, FilterMode, FragmentState, Instance, InstanceDescriptor, LoadOp, MultisampleState,
    Operations, PipelineCompilationOptions, PresentMode, PrimitiveState, Queue,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor,
    RequestAdapterOptions, Sampler, SamplerBindingType, SamplerDescriptor, ShaderModuleDescriptor,
    ShaderSource, ShaderStages, Surface, SurfaceConfiguration, SurfaceTargetUnsafe, SurfaceTexture,
    TexelCopyBufferLayout, TexelCopyTextureInfo, Texture, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureView,
    TextureViewDescriptor, TextureViewDimension, Trace,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

/// Active video, as scanned out by the FPGA VDP (`video_timing.sv`: 640×480
/// of an 800×525 raster). Every VDP mode is scaled into this framebuffer by
/// pixel/line repetition, exactly like the RTL does.
pub const FRAME_WIDTH: u32 = 640;
pub const FRAME_HEIGHT: u32 = 480;
/// Overscan margin around the active video, in framebuffer pixels. The border
/// colour is shown only here; the framebuffer itself never contains it.
pub const OVERSCAN: u32 = 8;
/// Visible raster (active video plus overscan) that is fitted to the window.
pub const DISPLAY_WIDTH: u32 = FRAME_WIDTH + OVERSCAN * 2;
pub const DISPLAY_HEIGHT: u32 = FRAME_HEIGHT + OVERSCAN * 2;

/// Game viewport geometry within a window of `size`, reserving
/// `top_reserve_px` physical pixels (debug menu bar) above the content.
/// Returns `(scale, viewport_x, viewport_y)` in physical pixels; the viewport
/// covers the whole visible raster including overscan.
pub fn game_viewport(size: PhysicalSize<u32>, top_reserve_px: f64) -> (f64, f64, f64) {
    if size.width == 0 || size.height == 0 {
        return (0.0, 0.0, 0.0);
    }
    let available_height = (f64::from(size.height) - top_reserve_px).max(1.0);
    let scale = (f64::from(size.width) / f64::from(DISPLAY_WIDTH))
        .min(available_height / f64::from(DISPLAY_HEIGHT));
    if scale <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let viewport_width = f64::from(DISPLAY_WIDTH) * scale;
    let viewport_height = f64::from(DISPLAY_HEIGHT) * scale;
    let viewport_x = (f64::from(size.width) - viewport_width) * 0.5;
    let viewport_y = top_reserve_px + (available_height - viewport_height) * 0.5;
    (scale, viewport_x, viewport_y)
}

/// Shared full-screen triangle vertex stage.
const FULLSCREEN_VS: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(3.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 2.0),
        vec2<f32>(2.0, 0.0),
        vec2<f32>(0.0, 0.0),
    );

    var out: VsOut;
    out.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    out.uv = uvs[vertex_index];
    return out;
}
"#;

/// Builds the visible raster: the 640×480 framebuffer surrounded by the
/// overscan margin in the border colour (linear light).
const RASTER_FS: &str = r#"
struct RasterParams {
    border: vec4<f32>,
    // xy: overscan (raster px), zw: framebuffer size.
    geometry: vec4<f32>,
};

@group(0) @binding(0) var frame_texture: texture_2d<f32>;
@group(0) @binding(1) var<uniform> raster: RasterParams;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(floor(in.position.xy)) - vec2<i32>(raster.geometry.xy);
    let size = vec2<i32>(raster.geometry.zw);
    if (p.x < 0 || p.y < 0 || p.x >= size.x || p.y >= size.y) {
        return vec4<f32>(raster.border.rgb, 1.0);
    }
    return vec4<f32>(textureLoad(frame_texture, p, 0).rgb, 1.0);
}
"#;

/// Plain output: bilinear scale of the raster to the window viewport.
const PLAIN_FS: &str = r#"
@group(0) @binding(0) var raster_texture: texture_2d<f32>;
@group(0) @binding(1) var raster_sampler: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(raster_texture, raster_sampler, in.uv);
}
"#;

/// Separable Gaussian blur used for the CRT bloom/halation term. Runs at half
/// the raster resolution and keeps linear light in `Rgba16Float`.
const CRT_BLUR_FS: &str = r#"
struct BlurParams {
    // xy: one tap step along the blur direction (uv units).
    step: vec4<f32>,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;
@group(0) @binding(2) var<uniform> blur: BlurParams;

fn tap(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(src_tex, src_sampler, uv, 0.0).rgb;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = blur.step.xy;
    var c = tap(in.uv) * 0.2270270270;
    c += (tap(in.uv + d) + tap(in.uv - d)) * 0.1945945946;
    c += (tap(in.uv + d * 2.0) + tap(in.uv - d * 2.0)) * 0.1216216216;
    c += (tap(in.uv + d * 3.0) + tap(in.uv - d * 3.0)) * 0.0540540541;
    c += (tap(in.uv + d * 4.0) + tap(in.uv - d * 4.0)) * 0.0162162162;
    return vec4<f32>(c, 1.0);
}
"#;

/// CRT composite in the spirit of crt-royale: luminance-dependent Gaussian
/// scanline beams, anti-ringing Lanczos2 along each line, a phosphor mask in
/// output pixels, bloom/halation, barrel curvature and a rounded tube edge.
///
/// The beam grid comes from `pitch`, the raster pixels per source pixel of
/// the current VDP mode (2 lines for the 240-line modes, 1 for the 480-line
/// mode). The overscan margin is scanned with the same grid, so the border
/// gets scanlines too.
const CRT_FS: &str = r#"
struct CrtParams {
    // xy: raster size, zw: source grid origin (raster px).
    geometry: vec4<f32>,
    // xy: raster px per source pixel, zw: viewport size (physical px).
    pitch_out: vec4<f32>,
    // scanline strength, beam sigma min, beam sigma max, mask strength.
    beam: vec4<f32>,
    // mask type, curvature, bloom, brightness.
    look: vec4<f32>,
    // encode sRGB (0/1), vignette, unused, unused.
    output: vec4<f32>,
};

@group(0) @binding(0) var raster_texture: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var bloom_texture: texture_2d<f32>;
@group(0) @binding(3) var<uniform> crt: CrtParams;

fn warp(uv: vec2<f32>) -> vec2<f32> {
    let k = crt.look.y;
    var c = uv * 2.0 - 1.0;
    c = c * vec2<f32>(1.0 + c.y * c.y * k, 1.0 + c.x * c.x * k * 1.3333333);
    return c * 0.5 + 0.5;
}

// Linear-light colour of source pixel (ix, iy) of the current mode's grid.
// Cells outside the raster clamp to its edge, which is the border colour.
fn fetch(ix: f32, iy: f32) -> vec3<f32> {
    let p = crt.geometry.zw + (vec2<f32>(ix, iy) + 0.5) * crt.pitch_out.xy;
    let max_texel = vec2<i32>(crt.geometry.xy) - vec2<i32>(1, 1);
    let t = clamp(vec2<i32>(floor(p)), vec2<i32>(0, 0), max_texel);
    return textureLoad(raster_texture, t, 0).rgb;
}

fn lanczos2(x: f32) -> f32 {
    let ax = abs(x);
    if (ax < 1e-4) {
        return 1.0;
    }
    if (ax >= 2.0) {
        return 0.0;
    }
    let pi_x = 3.14159265 * ax;
    return 2.0 * sin(pi_x) * sin(pi_x * 0.5) / (pi_x * pi_x);
}

// One scanline, horizontally resampled with an anti-ringing Lanczos2.
fn scanline_color(gx: f32, iy: f32) -> vec3<f32> {
    let x = gx - 0.5;
    let x0 = floor(x);
    let f = x - x0;
    let a = fetch(x0 - 1.0, iy);
    let b = fetch(x0, iy);
    let c = fetch(x0 + 1.0, iy);
    let d = fetch(x0 + 2.0, iy);
    let wa = lanczos2(f + 1.0);
    let wb = lanczos2(f);
    let wc = lanczos2(1.0 - f);
    let wd = lanczos2(2.0 - f);
    let sum = (a * wa + b * wb + c * wc + d * wd) / (wa + wb + wc + wd);
    return clamp(sum, min(b, c), max(b, c));
}

// Gaussian beam whose width grows with brightness (per channel), normalised
// so a fully lit line keeps its average energy.
fn beam(color: vec3<f32>, distance: f32) -> vec3<f32> {
    let sigma = mix(vec3<f32>(crt.beam.y), vec3<f32>(crt.beam.z), sqrt(color));
    let x = vec3<f32>(distance) / sigma;
    return color * exp(-0.5 * x * x) / (sigma * 2.5066283);
}

fn triad(i: i32, dark: f32, light: f32) -> vec3<f32> {
    var m = vec3<f32>(dark);
    m[i] = light;
    return m;
}

fn phosphor_mask(pos: vec2<f32>) -> vec3<f32> {
    let kind = i32(crt.look.x + 0.5);
    let s = crt.beam.w;
    let dark = 1.0 - s;
    let light = 1.0 + s;
    let column = i32(floor(pos.x)) % 3;
    if (kind == 1) {
        // Aperture grille: vertical RGB stripes.
        return triad(column, dark, light);
    }
    if (kind == 2) {
        // Slot mask: RGB triads with staggered horizontal gaps.
        let group = i32(floor(pos.x / 3.0));
        let row = (i32(floor(pos.y)) + (group & 1) * 2) % 4;
        if (row == 3) {
            return vec3<f32>(dark);
        }
        return triad(column, dark, light);
    }
    if (kind == 3) {
        // Shadow mask: dot triads shifted on every row.
        let shifted = i32(floor(pos.x) + floor(pos.y) * 3.0);
        return triad(shifted % 3, dark, light);
    }
    return vec3<f32>(1.0);
}

fn encode_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = warp(in.uv);
    let src_size = crt.geometry.xy;
    let pitch = crt.pitch_out.xy;
    let out_size = crt.pitch_out.zw;

    // Source-grid coordinates of this output pixel.
    let g = (uv * src_size - crt.geometry.zw) / pitch;
    let ly = g.y - 0.5;
    let l0 = floor(ly);
    let f = ly - l0;
    let c0 = scanline_color(g.x, l0);
    let c1 = scanline_color(g.x, l0 + 1.0);
    // Without beams, blend lines over a short window so curved edges stay
    // anti-aliased while lines remain sharp.
    let flat_color = mix(c0, c1, smoothstep(0.3, 0.7, f));
    let beam_color = beam(c0, f) + beam(c1, 1.0 - f);

    // Fade the beam out when one source line covers too few output pixels
    // to draw it without moire (e.g. the 480-line mode in a small window).
    let line_px = out_size.y * pitch.y / src_size.y;
    let strength = crt.beam.x * smoothstep(1.5, 3.0, line_px);
    var color = mix(flat_color, beam_color, strength);

    color = color * phosphor_mask(in.position.xy) * crt.look.w;
    let bloom = textureSampleLevel(bloom_texture, linear_sampler, uv, 0.0).rgb;
    color = color + bloom * crt.look.z;

    // Vignette and a rounded, anti-aliased tube edge.
    let v = clamp(16.0 * uv.x * uv.y * (1.0 - uv.x) * (1.0 - uv.y), 0.0, 1.0);
    color = color * mix(1.0, pow(v, 0.25), crt.output.y);
    let radius = min(out_size.x, out_size.y) * clamp(crt.look.y * 1.5, 0.0, 0.08);
    let q = abs(uv - 0.5) * out_size - (out_size * 0.5 - vec2<f32>(radius));
    let dist = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    color = color * clamp(0.5 - dist, 0.0, 1.0);

    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    if (crt.output.x > 0.5) {
        color = encode_srgb(color);
    }
    return vec4<f32>(color, 1.0);
}
"#;

/// Phosphor mask pattern drawn by the CRT filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrtMask {
    None,
    ApertureGrille,
    SlotMask,
    ShadowMask,
}

impl CrtMask {
    pub const ALL: [CrtMask; 4] = [
        CrtMask::None,
        CrtMask::ApertureGrille,
        CrtMask::SlotMask,
        CrtMask::ShadowMask,
    ];

    /// Name used in `config.yaml` (`display.crt.mask`).
    pub fn name(self) -> &'static str {
        match self {
            CrtMask::None => "none",
            CrtMask::ApertureGrille => "aperture_grille",
            CrtMask::SlotMask => "slot_mask",
            CrtMask::ShadowMask => "shadow_mask",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CrtMask::None => "None",
            CrtMask::ApertureGrille => "Aperture grille",
            CrtMask::SlotMask => "Slot mask",
            CrtMask::ShadowMask => "Shadow mask",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mask| mask.name() == name)
    }

    fn shader_index(self) -> f32 {
        match self {
            CrtMask::None => 0.0,
            CrtMask::ApertureGrille => 1.0,
            CrtMask::SlotMask => 2.0,
            CrtMask::ShadowMask => 3.0,
        }
    }
}

/// GUI-only CRT display filter settings (`config.yaml` `display.crt`).
/// Beam widths are Gaussian sigmas in units of one guest scanline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrtSettings {
    pub enabled: bool,
    pub scanline_strength: f32,
    pub beam_min: f32,
    pub beam_max: f32,
    pub mask: CrtMask,
    pub mask_strength: f32,
    pub curvature: f32,
    pub bloom: f32,
    pub brightness: f32,
    pub vignette: f32,
}

impl Default for CrtSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            scanline_strength: 1.0,
            beam_min: 0.18,
            beam_max: 0.33,
            mask: CrtMask::ApertureGrille,
            mask_strength: 0.3,
            curvature: 0.02,
            bloom: 0.15,
            brightness: 1.15,
            vignette: 0.3,
        }
    }
}

impl CrtSettings {
    /// Curvature actually applied to the picture (0 when the filter is off).
    pub fn effective_curvature(&self) -> f64 {
        if self.enabled {
            f64::from(self.curvature)
        } else {
            0.0
        }
    }

    fn validate(&self) -> Result<(), String> {
        let check = |name: &str, value: f32, min: f32, max: f32| {
            if value.is_finite() && (min..=max).contains(&value) {
                Ok(())
            } else {
                Err(format!(
                    "`display.crt.{name}` must be between {min} and {max}"
                ))
            }
        };
        check("scanlines", self.scanline_strength, 0.0, 1.0)?;
        check("beam_min", self.beam_min, 0.05, 1.0)?;
        check("beam_max", self.beam_max, 0.05, 1.0)?;
        if self.beam_max < self.beam_min {
            return Err("`display.crt.beam_max` must not be smaller than `beam_min`".into());
        }
        check("mask_strength", self.mask_strength, 0.0, 1.0)?;
        check("curvature", self.curvature, 0.0, 0.2)?;
        check("bloom", self.bloom, 0.0, 2.0)?;
        check("brightness", self.brightness, 0.25, 4.0)?;
        check("vignette", self.vignette, 0.0, 1.0)
    }
}

/// Reads `display.crt` from a parsed `config.yaml`. The value is either a
/// bool (enable with defaults) or a mapping; a mapping enables the filter
/// unless it says `enabled: false`. A missing key keeps the filter off.
pub fn crt_settings_from_yaml(root: &yaml_serde::Value) -> Result<CrtSettings, String> {
    let mut settings = CrtSettings::default();
    let Some(display) = root.get("display") else {
        return Ok(settings);
    };
    let display = display.as_mapping().ok_or("`display` must be a mapping")?;
    let Some(crt) = display.get("crt") else {
        return Ok(settings);
    };
    if let Some(enabled) = crt.as_bool() {
        settings.enabled = enabled;
        return Ok(settings);
    }
    let table = crt
        .as_mapping()
        .ok_or("`display.crt` must be a bool or a mapping")?;
    settings.enabled = true;
    for (key, value) in table {
        let key = key.as_str().ok_or("`display.crt` keys must be strings")?;
        let number = || {
            value
                .as_f64()
                .map(|v| v as f32)
                .ok_or_else(|| format!("`display.crt.{key}` must be a number"))
        };
        match key {
            "enabled" => {
                settings.enabled = value
                    .as_bool()
                    .ok_or("`display.crt.enabled` must be a bool")?;
            }
            "scanlines" => settings.scanline_strength = number()?,
            "beam_min" => settings.beam_min = number()?,
            "beam_max" => settings.beam_max = number()?,
            "mask" => {
                let name = value
                    .as_str()
                    .ok_or("`display.crt.mask` must be a string")?;
                settings.mask = CrtMask::from_name(name).ok_or_else(|| {
                    format!(
                        "unknown `display.crt.mask` '{name}' (expected none, aperture_grille, slot_mask or shadow_mask)"
                    )
                })?;
            }
            "mask_strength" => settings.mask_strength = number()?,
            "curvature" => settings.curvature = number()?,
            "bloom" => settings.bloom = number()?,
            "brightness" => settings.brightness = number()?,
            "vignette" => settings.vignette = number()?,
            other => return Err(format!("unknown key `display.crt.{other}`")),
        }
    }
    settings.validate()?;
    Ok(settings)
}

/// Same barrel warp as the CRT shader: maps a viewport uv (0..1) to the
/// raster uv displayed there. Keeps the guest mouse on the picture when
/// curvature is enabled.
pub fn crt_warp(u: f64, v: f64, curvature: f64) -> (f64, f64) {
    let cx = u * 2.0 - 1.0;
    let cy = v * 2.0 - 1.0;
    let wx = cx * (1.0 + cy * cy * curvature);
    let wy = cy * (1.0 + cx * cx * curvature * (4.0 / 3.0));
    (wx * 0.5 + 0.5, wy * 0.5 + 0.5)
}

/// Framebuffer pixels per source pixel for a VDP frame of the given size:
/// (2, 2) for 320×240, (1, 2) for 640×240 and (1, 1) for 640×480. The CRT
/// filter draws one beam per source row, so `.1` is the scanline pitch.
pub fn source_pixel_pitch(frame_width: usize, frame_height: usize) -> (f32, f32) {
    (
        FRAME_WIDTH as f32 / frame_width.max(1) as f32,
        FRAME_HEIGHT as f32 / frame_height.max(1) as f32,
    )
}

/// Scales a VDP frame of `width`×`height` into the 640×480 framebuffer by
/// pixel/line repetition (FPGA: `rx>>1` / `ry>>1`). Returns the source
/// coordinate shown at framebuffer pixel `(x, y)`.
pub fn framebuffer_source(x: u32, y: u32, width: usize, height: usize) -> (usize, usize) {
    (
        x as usize * width / FRAME_WIDTH as usize,
        y as usize * height / FRAME_HEIGHT as usize,
    )
}

fn srgb_to_linear(value: u8) -> f32 {
    let c = f32::from(value) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

const RASTER_FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;
const BLOOM_WIDTH: u32 = DISPLAY_WIDTH / 2;
const BLOOM_HEIGHT: u32 = DISPLAY_HEIGHT / 2;
const BLOOM_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
const CRT_UNIFORM_FLOATS: usize = 20;

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            view_dimension: TextureViewDimension::D2,
            sample_type: TextureSampleType::Float { filterable: true },
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(SamplerBindingType::Filtering),
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

enum Binding<'a> {
    View(&'a TextureView),
    Sampler(&'a Sampler),
    Buffer(&'a wgpu::Buffer),
}

fn bind_group(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::BindGroupLayout,
    bindings: &[Binding<'_>],
) -> wgpu::BindGroup {
    let entries: Vec<_> = bindings
        .iter()
        .enumerate()
        .map(|(index, binding)| wgpu::BindGroupEntry {
            binding: index as u32,
            resource: match binding {
                Binding::View(view) => wgpu::BindingResource::TextureView(view),
                Binding::Sampler(sampler) => wgpu::BindingResource::Sampler(sampler),
                Binding::Buffer(buffer) => buffer.as_entire_binding(),
            },
        })
        .collect();
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &entries,
    })
}

fn fullscreen_pipeline(
    device: &wgpu::Device,
    label: &str,
    fragment: &str,
    layout: &wgpu::BindGroupLayout,
    format: TextureFormat,
) -> RenderPipeline {
    let module = device.create_shader_module(ShaderModuleDescriptor {
        label: Some(label),
        source: ShaderSource::Wgsl(format!("{FULLSCREEN_VS}{fragment}").into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: PipelineCompilationOptions::default(),
        },
        fragment: Some(FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            targets: &[Some(ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: ColorWrites::ALL,
            })],
            compilation_options: PipelineCompilationOptions::default(),
        }),
        primitive: PrimitiveState::default(),
        depth_stencil: None,
        multisample: MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn render_target(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    format: TextureFormat,
) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some(label),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn fullscreen_pass(
    encoder: &mut wgpu::CommandEncoder,
    label: &str,
    target: &TextureView,
    pipeline: &RenderPipeline,
    bind_group: &wgpu::BindGroup,
    viewport: Option<(f64, f64, f64, f64)>,
) {
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Clear(Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    if let Some((x, y, width, height)) = viewport {
        pass.set_viewport(x as f32, y as f32, width as f32, height as f32, 0.0, 1.0);
    }
    pass.set_bind_group(0, bind_group, &[]);
    pass.draw(0..3, 0..1);
}

struct CrtPipeline {
    blur_pipeline: RenderPipeline,
    crt_pipeline: RenderPipeline,
    _bloom_a: Texture,
    bloom_a_view: TextureView,
    _bloom_b: Texture,
    bloom_b_view: TextureView,
    _blur_uniforms: [wgpu::Buffer; 2],
    crt_uniform: wgpu::Buffer,
    blur_h_bind_group: wgpu::BindGroup,
    blur_v_bind_group: wgpu::BindGroup,
    crt_bind_group: wgpu::BindGroup,
}

impl CrtPipeline {
    fn new(
        device: &wgpu::Device,
        queue: &Queue,
        surface_format: TextureFormat,
        raster_view: &TextureView,
        sampler: &Sampler,
    ) -> Self {
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CPT32 CRT Blur Layout"),
            entries: &[texture_entry(0), sampler_entry(1), uniform_entry(2)],
        });
        let crt_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CPT32 CRT Layout"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                texture_entry(2),
                uniform_entry(3),
            ],
        });
        let blur_pipeline = fullscreen_pipeline(
            device,
            "CPT32 CRT Blur",
            CRT_BLUR_FS,
            &blur_layout,
            BLOOM_FORMAT,
        );
        let crt_pipeline =
            fullscreen_pipeline(device, "CPT32 CRT", CRT_FS, &crt_layout, surface_format);

        let bloom_a = render_target(
            device,
            "CPT32 CRT Bloom A",
            BLOOM_WIDTH,
            BLOOM_HEIGHT,
            BLOOM_FORMAT,
        );
        let bloom_a_view = bloom_a.create_view(&TextureViewDescriptor::default());
        let bloom_b = render_target(
            device,
            "CPT32 CRT Bloom B",
            BLOOM_WIDTH,
            BLOOM_HEIGHT,
            BLOOM_FORMAT,
        );
        let bloom_b_view = bloom_b.create_view(&TextureViewDescriptor::default());

        let uniform = |label, size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let blur_h = uniform("CPT32 CRT Blur H Params", 16);
        let blur_v = uniform("CPT32 CRT Blur V Params", 16);
        let crt_uniform = uniform("CPT32 CRT Params", CRT_UNIFORM_FLOATS * 4);
        // Both passes step 2 raster px per tap: the horizontal pass also
        // halves the resolution, the vertical one runs on half-size texels.
        queue.write_buffer(
            &blur_h,
            0,
            &f32_bytes(&[2.0 / DISPLAY_WIDTH as f32, 0.0, 0.0, 0.0]),
        );
        queue.write_buffer(
            &blur_v,
            0,
            &f32_bytes(&[0.0, 1.0 / BLOOM_HEIGHT as f32, 0.0, 0.0]),
        );

        let blur_h_bind_group = bind_group(
            device,
            "CPT32 CRT Blur H",
            &blur_layout,
            &[
                Binding::View(raster_view),
                Binding::Sampler(sampler),
                Binding::Buffer(&blur_h),
            ],
        );
        let blur_v_bind_group = bind_group(
            device,
            "CPT32 CRT Blur V",
            &blur_layout,
            &[
                Binding::View(&bloom_a_view),
                Binding::Sampler(sampler),
                Binding::Buffer(&blur_v),
            ],
        );
        let crt_bind_group = bind_group(
            device,
            "CPT32 CRT",
            &crt_layout,
            &[
                Binding::View(raster_view),
                Binding::Sampler(sampler),
                Binding::View(&bloom_b_view),
                Binding::Buffer(&crt_uniform),
            ],
        );

        Self {
            blur_pipeline,
            crt_pipeline,
            _bloom_a: bloom_a,
            bloom_a_view,
            _bloom_b: bloom_b,
            bloom_b_view,
            _blur_uniforms: [blur_h, blur_v],
            crt_uniform,
            blur_h_bind_group,
            blur_v_bind_group,
            crt_bind_group,
        }
    }

    fn write_params(
        &self,
        queue: &Queue,
        settings: &CrtSettings,
        (frame_width, frame_height): (usize, usize),
        (viewport_width, viewport_height): (f64, f64),
        encode_srgb: bool,
    ) {
        let (pitch_x, pitch_y) = source_pixel_pitch(frame_width, frame_height);
        let params: [f32; CRT_UNIFORM_FLOATS] = [
            DISPLAY_WIDTH as f32,
            DISPLAY_HEIGHT as f32,
            OVERSCAN as f32,
            OVERSCAN as f32,
            pitch_x,
            pitch_y,
            viewport_width as f32,
            viewport_height as f32,
            settings.scanline_strength,
            settings.beam_min,
            settings.beam_max,
            if settings.mask == CrtMask::None {
                0.0
            } else {
                settings.mask_strength
            },
            settings.mask.shader_index(),
            settings.curvature,
            settings.bloom,
            settings.brightness,
            if encode_srgb { 1.0 } else { 0.0 },
            settings.vignette,
            0.0,
            0.0,
        ];
        queue.write_buffer(&self.crt_uniform, 0, &f32_bytes(&params));
    }

    fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &TextureView,
        viewport: (f64, f64, f64, f64),
    ) {
        fullscreen_pass(
            encoder,
            "CPT32 CRT Bloom H Pass",
            &self.bloom_a_view,
            &self.blur_pipeline,
            &self.blur_h_bind_group,
            None,
        );
        fullscreen_pass(
            encoder,
            "CPT32 CRT Bloom V Pass",
            &self.bloom_b_view,
            &self.blur_pipeline,
            &self.blur_v_bind_group,
            None,
        );
        fullscreen_pass(
            encoder,
            "CPT32 CRT Surface Pass",
            surface_view,
            &self.crt_pipeline,
            &self.crt_bind_group,
            Some(viewport),
        );
    }
}

pub struct WgpuPresenter {
    surface: Surface<'static>,
    device: wgpu::Device,
    queue: Queue,
    config: SurfaceConfiguration,
    frame_texture: Texture,
    _raster_texture: Texture,
    raster_view: TextureView,
    raster_uniform: wgpu::Buffer,
    raster_pipeline: RenderPipeline,
    raster_bind_group: wgpu::BindGroup,
    plain_pipeline: RenderPipeline,
    plain_bind_group: wgpu::BindGroup,
    crt: CrtPipeline,
    crt_settings: CrtSettings,
    pixels: Vec<u8>,
    size: PhysicalSize<u32>,
}

#[derive(Debug)]
pub enum FrameError {
    Lost,
    Outdated,
    Timeout,
    Occluded,
    Validation,
    Overlay(String),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl WgpuPresenter {
    pub fn new(
        window: &Window,
        display_handle: impl winit::raw_window_handle::HasDisplayHandle
        + fmt::Debug
        + Send
        + Sync
        + 'static,
    ) -> Self {
        let size = window.inner_size();
        let instance = Instance::new(InstanceDescriptor::new_with_display_handle(Box::new(
            display_handle,
        )));
        let surface = unsafe {
            instance
                .create_surface_unsafe(
                    SurfaceTargetUnsafe::from_window(window)
                        .expect("Failed to create surface target"),
                )
                .expect("Failed to create wgpu surface")
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime");

        let (adapter, (device, queue)) = runtime.block_on(async {
            let adapter = instance
                .request_adapter(&RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                })
                .await
                .expect("Failed to find a suitable GPU adapter");

            let device_and_queue = adapter
                .request_device(&DeviceDescriptor {
                    label: Some("CPT32 Device"),
                    required_features: Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                    trace: Trace::default(),
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                })
                .await
                .expect("Failed to create GPU device");

            (adapter, device_and_queue)
        });

        let info = adapter.get_info();
        tracing::info!(adapter = %info.name, backend = ?info.backend, "Graphics adapter selected");

        let surface_caps = surface.get_capabilities(&adapter);
        let format = surface_caps
            .formats
            .iter()
            .copied()
            .find(TextureFormat::is_srgb)
            .unwrap_or(surface_caps.formats[0]);
        let present_mode = if surface_caps.present_modes.contains(&PresentMode::Fifo) {
            PresentMode::Fifo
        } else {
            surface_caps.present_modes[0]
        };
        let alpha_mode = surface_caps.alpha_modes[0];

        let config = SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let frame_texture = device.create_texture(&TextureDescriptor {
            label: Some("CPT32 Framebuffer 640x480"),
            size: Extent3d {
                width: FRAME_WIDTH,
                height: FRAME_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let frame_view = frame_texture.create_view(&TextureViewDescriptor::default());
        let raster_texture = render_target(
            &device,
            "CPT32 Raster (active + overscan)",
            DISPLAY_WIDTH,
            DISPLAY_HEIGHT,
            RASTER_FORMAT,
        );
        let raster_view = raster_texture.create_view(&TextureViewDescriptor::default());
        let linear_sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("CPT32 Raster Linear Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..SamplerDescriptor::default()
        });

        let raster_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CPT32 Raster Layout"),
            entries: &[texture_entry(0), uniform_entry(1)],
        });
        let raster_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("CPT32 Raster Params"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let raster_pipeline = fullscreen_pipeline(
            &device,
            "CPT32 Raster",
            RASTER_FS,
            &raster_layout,
            RASTER_FORMAT,
        );
        let raster_bind_group = bind_group(
            &device,
            "CPT32 Raster",
            &raster_layout,
            &[Binding::View(&frame_view), Binding::Buffer(&raster_uniform)],
        );

        let plain_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CPT32 Plain Layout"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        let plain_pipeline = fullscreen_pipeline(
            &device,
            "CPT32 Plain",
            PLAIN_FS,
            &plain_layout,
            config.format,
        );
        let plain_bind_group = bind_group(
            &device,
            "CPT32 Plain",
            &plain_layout,
            &[
                Binding::View(&raster_view),
                Binding::Sampler(&linear_sampler),
            ],
        );

        let crt = CrtPipeline::new(
            &device,
            &queue,
            config.format,
            &raster_view,
            &linear_sampler,
        );

        Self {
            surface,
            device,
            queue,
            config,
            frame_texture,
            _raster_texture: raster_texture,
            raster_view,
            raster_uniform,
            raster_pipeline,
            raster_bind_group,
            plain_pipeline,
            plain_bind_group,
            crt,
            crt_settings: CrtSettings::default(),
            pixels: vec![0u8; (FRAME_WIDTH * FRAME_HEIGHT * 4) as usize],
            size,
        }
    }

    pub fn crt_settings(&self) -> CrtSettings {
        self.crt_settings
    }

    pub fn set_crt_settings(&mut self, settings: CrtSettings) {
        self.crt_settings = settings;
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }

        self.size = size;
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn surface_format(&self) -> TextureFormat {
        self.config.format
    }

    pub fn render_with_overlay<F>(
        &mut self,
        vdp: &Rc<RefCell<Vdp>>,
        top_reserve_px: f64,
        overlay: F,
    ) -> Result<(), FrameError>
    where
        F: FnMut(
            &wgpu::Device,
            &wgpu::Queue,
            &mut wgpu::CommandEncoder,
            &TextureView,
        ) -> Result<(), FrameError>,
    {
        self.render_with_overlay_inner(vdp, top_reserve_px, overlay)
    }

    pub fn render(&mut self, vdp: &Rc<RefCell<Vdp>>) -> Result<(), FrameError> {
        self.render_with_overlay_inner(vdp, 0.0, |_, _, _, _| Ok(()))
    }

    /// Uploads the VDP frame into the 640×480 framebuffer (pixel/line
    /// repetition) and the border colour as the raster's overscan colour.
    /// Returns the VDP frame size, which sets the CRT scanline pitch.
    fn upload_frame(&mut self, vdp: &Rc<RefCell<Vdp>>) -> (usize, usize) {
        let vdp = vdp.borrow();
        // VdpFramebuffer::get_pixel composites the VPU RGBA plane over the
        // selected VDP graphics/PCG plane.
        let fb = vdp.framebuffer();
        let (frame_width, frame_height) = fb.dimensions();
        let (border_r, border_g, border_b) = fb.border_pixel();

        for y in 0..FRAME_HEIGHT {
            for x in 0..FRAME_WIDTH {
                let (src_x, src_y) = framebuffer_source(x, y, frame_width, frame_height);
                let (r, g, b) = fb.get_pixel(src_x, src_y);
                let offset = (y * FRAME_WIDTH + x) as usize * 4;
                self.pixels[offset..offset + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }

        self.queue.write_texture(
            TexelCopyTextureInfo {
                texture: &self.frame_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &self.pixels,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(FRAME_WIDTH * 4),
                rows_per_image: Some(FRAME_HEIGHT),
            },
            Extent3d {
                width: FRAME_WIDTH,
                height: FRAME_HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        self.queue.write_buffer(
            &self.raster_uniform,
            0,
            &f32_bytes(&[
                srgb_to_linear(border_r),
                srgb_to_linear(border_g),
                srgb_to_linear(border_b),
                1.0,
                OVERSCAN as f32,
                OVERSCAN as f32,
                FRAME_WIDTH as f32,
                FRAME_HEIGHT as f32,
            ]),
        );
        (frame_width, frame_height)
    }

    fn render_with_overlay_inner<F>(
        &mut self,
        vdp: &Rc<RefCell<Vdp>>,
        top_reserve_px: f64,
        mut overlay: F,
    ) -> Result<(), FrameError>
    where
        F: FnMut(
            &wgpu::Device,
            &wgpu::Queue,
            &mut wgpu::CommandEncoder,
            &TextureView,
        ) -> Result<(), FrameError>,
    {
        let frame_size = self.upload_frame(vdp);

        let output: SurfaceTexture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(surface_texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(surface_texture) => surface_texture,
            wgpu::CurrentSurfaceTexture::Timeout => return Err(FrameError::Timeout),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(FrameError::Occluded),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(FrameError::Outdated),
            wgpu::CurrentSurfaceTexture::Lost => return Err(FrameError::Lost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(FrameError::Validation),
        };
        let surface_view = output
            .texture
            .create_view(&TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor {
                label: Some("CPT32 Render Encoder"),
            });

        fullscreen_pass(
            &mut encoder,
            "CPT32 Raster Pass",
            &self.raster_view,
            &self.raster_pipeline,
            &self.raster_bind_group,
            None,
        );

        let (scale, viewport_x, viewport_y) = game_viewport(self.size, top_reserve_px);
        let viewport_width = f64::from(DISPLAY_WIDTH) * scale;
        let viewport_height = f64::from(DISPLAY_HEIGHT) * scale;
        let viewport = (viewport_x, viewport_y, viewport_width, viewport_height);

        if self.crt_settings.enabled {
            self.crt.write_params(
                &self.queue,
                &self.crt_settings,
                frame_size,
                (viewport_width, viewport_height),
                !self.config.format.is_srgb(),
            );
            self.crt.encode(&mut encoder, &surface_view, viewport);
        } else {
            fullscreen_pass(
                &mut encoder,
                "CPT32 Surface Render Pass",
                &surface_view,
                &self.plain_pipeline,
                &self.plain_bind_group,
                Some(viewport),
            );
        }

        overlay(&self.device, &self.queue, &mut encoder, &surface_view)?;

        self.queue.submit(Some(encoder.finish()));
        output.present();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framebuffer_is_fpga_active_video_with_overscan_around_it() {
        assert_eq!((FRAME_WIDTH, FRAME_HEIGHT), (640, 480));
        assert_eq!(DISPLAY_WIDTH, FRAME_WIDTH + 2 * OVERSCAN);
        assert_eq!(DISPLAY_HEIGHT, FRAME_HEIGHT + 2 * OVERSCAN);
    }

    #[test]
    fn modes_scale_into_640x480_by_repetition() {
        // 320x240: both axes doubled (FPGA rx>>1, ry>>1).
        assert_eq!(framebuffer_source(639, 479, 320, 240), (319, 239));
        assert_eq!(framebuffer_source(1, 1, 320, 240), (0, 0));
        // 640x240: lines doubled only.
        assert_eq!(framebuffer_source(639, 479, 640, 240), (639, 239));
        // 640x480: identity.
        assert_eq!(framebuffer_source(123, 456, 640, 480), (123, 456));
    }

    #[test]
    fn scanline_pitch_follows_display_mode() {
        assert_eq!(source_pixel_pitch(320, 240), (2.0, 2.0));
        assert_eq!(source_pixel_pitch(640, 240), (1.0, 2.0));
        assert_eq!(source_pixel_pitch(640, 480), (1.0, 1.0));
    }

    #[test]
    fn crt_warp_keeps_centre_and_edge_midpoints() {
        assert_eq!(crt_warp(0.5, 0.5, 0.05), (0.5, 0.5));
        let (u, v) = crt_warp(1.0, 0.5, 0.05);
        assert!((u - 1.0).abs() < 1e-12 && (v - 0.5).abs() < 1e-12);
        let (u, v) = crt_warp(0.9, 0.9, 0.05);
        assert!(u > 0.9 && v > 0.9);
        assert_eq!(crt_warp(0.25, 0.75, 0.0), (0.25, 0.75));
    }

    fn parse(text: &str) -> Result<CrtSettings, String> {
        crt_settings_from_yaml(&yaml_serde::from_str(text).unwrap())
    }

    #[test]
    fn crt_config_defaults_off_and_accepts_bool_or_mapping() {
        assert!(!parse("cpu: {}\n").unwrap().enabled);
        assert!(parse("display:\n  crt: true\n").unwrap().enabled);
        assert!(!parse("display:\n  crt: false\n").unwrap().enabled);
        let settings =
            parse("display:\n  crt:\n    mask: slot_mask\n    scanlines: 0.5\n    curvature: 0\n")
                .unwrap();
        assert!(settings.enabled);
        assert_eq!(settings.mask, CrtMask::SlotMask);
        assert_eq!(settings.scanline_strength, 0.5);
        assert_eq!(settings.curvature, 0.0);
        assert!(
            !parse("display:\n  crt:\n    enabled: false\n")
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn crt_config_rejects_bad_values() {
        assert!(parse("display:\n  crt:\n    mask: trinitron\n").is_err());
        assert!(parse("display:\n  crt:\n    scanlines: 2\n").is_err());
        assert!(parse("display:\n  crt:\n    beam_min: 0.5\n    beam_max: 0.3\n").is_err());
        assert!(parse("display:\n  crt:\n    sharpness: 1\n").is_err());
        assert!(parse("display: 1\n").is_err());
    }
}
