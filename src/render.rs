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
    ShaderSource, ShaderStages, Surface, SurfaceConfiguration, SurfaceTargetUnsafe,
    SurfaceTexture, Texture, TextureAspect, TextureDescriptor, TextureDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor, TextureViewDimension,
    TexelCopyBufferLayout, TexelCopyTextureInfo, Trace,
};
use winit::dpi::PhysicalSize;
use winit::window::Window;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;

const SHADER: &str = r#"
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

@group(0) @binding(0) var frame_texture: texture_2d<f32>;
@group(0) @binding(1) var frame_sampler: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(frame_texture, frame_sampler, in.uv);
}
"#;

pub struct WgpuPresenter {
    surface: Surface<'static>,
    device: wgpu::Device,
    queue: Queue,
    config: SurfaceConfiguration,
    texture: Texture,
    texture_view: TextureView,
    sampler: Sampler,
    pipeline: RenderPipeline,
    bind_group: wgpu::BindGroup,
    size: PhysicalSize<u32>,
}

#[derive(Debug)]
pub enum FrameError {
    Lost,
    Outdated,
    Timeout,
    Occluded,
    Validation,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl WgpuPresenter {
    pub fn new(window: &Window) -> Self {
        let size = window.inner_size();
        let instance = Instance::new(InstanceDescriptor::new_without_display_handle());
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

        let texture = device.create_texture(&TextureDescriptor {
            label: Some("CPT32 Frame Texture"),
            size: Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&TextureViewDescriptor::default());
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("CPT32 Frame Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..SamplerDescriptor::default()
        });

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("CPT32 Shader"),
            source: ShaderSource::Wgsl(SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CPT32 Frame Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: TextureViewDimension::D2,
                        sample_type: TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CPT32 Frame Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("CPT32 Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("CPT32 Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format: config.format,
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
        });

        Self {
            surface,
            device,
            queue,
            config,
            texture,
            texture_view,
            sampler,
            pipeline,
            bind_group,
            size,
        }
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

    pub fn render(&mut self, vdp: &Rc<RefCell<Vdp>>) -> Result<(), FrameError> {
        let framebuffer = vdp.borrow();
        let fb = framebuffer.framebuffer();
        let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];

        for y in 0..HEIGHT as usize {
            for x in 0..WIDTH as usize {
                let (r, g, b) = fb.get_pixel(x, y);
                let offset = (y * WIDTH as usize + x) * 4;
                pixels[offset] = r;
                pixels[offset + 1] = g;
                pixels[offset + 2] = b;
                pixels[offset + 3] = 255;
            }
        }

        self.queue.write_texture(
            TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &pixels,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
            Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
        );

        let output: SurfaceTexture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(surface_texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(surface_texture) => surface_texture,
            wgpu::CurrentSurfaceTexture::Timeout => return Err(FrameError::Timeout),
            wgpu::CurrentSurfaceTexture::Occluded => return Err(FrameError::Occluded),
            wgpu::CurrentSurfaceTexture::Outdated => return Err(FrameError::Outdated),
            wgpu::CurrentSurfaceTexture::Lost => return Err(FrameError::Lost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(FrameError::Validation),
        };
        let view = output.texture.create_view(&TextureViewDescriptor::default());
        let mut encoder = self.device.create_command_encoder(&CommandEncoderDescriptor {
            label: Some("CPT32 Render Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("CPT32 Render Pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &view,
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

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
        output.present();
        Ok(())
    }
}