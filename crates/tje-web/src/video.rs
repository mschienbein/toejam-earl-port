use jgenesis_common::frontend::{Color, FrameSize};
use web_sys::HtmlCanvasElement;

pub struct Video {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    texture: wgpu::Texture,
    bind_groups: [wgpu::BindGroup; 2],
    size: FrameSize,
    max_dimension: u32,
    backend: &'static str,
    adapter: String,
}

const SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    let x = f32((index << 1u) & 2u);
    let y = f32(index & 2u);
    var output: VertexOutput;
    output.position = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    output.uv = vec2<f32>(x, 1.0 - y);
    return output;
}
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
@fragment fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, input.uv);
}
"#;

impl Video {
    pub async fn new(canvas: HtmlCanvasElement, backend: &str) -> Result<Self, String> {
        let backends = match backend {
            "webgpu" => wgpu::Backends::BROWSER_WEBGPU,
            "webgl" => wgpu::Backends::GL,
            _ => return Err(format!("unsupported graphics backend: {backend}")),
        };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance
            .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(|error| error.to_string())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("ToeJam & Earl canvas"),
                // WebGL2 has no compute stage; WebGPU default limits are invalid
                // on that fallback even though this pipeline is only a blit.
                required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                    .using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .map_err(|error| error.to_string())?;
        let max_dimension = device.limits().max_texture_dimension_2d;
        let mut config = surface
            .get_default_config(&adapter, 640, 448)
            .ok_or("adapter cannot present to this canvas")?;
        // Choose a non-sRGB target when available. Otherwise the input texture
        // uses sRGB too, cancelling decode/encode rather than washing out pixels.
        if let Some(format) = surface
            .get_capabilities(&adapter)
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
        {
            config.format = format;
        }
        surface.configure(&device, &config);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Framebuffer blit"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Framebuffer presentation"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let size = FrameSize {
            width: 320,
            height: 224,
        };
        let (texture, bind_groups) = Self::create_texture(&device, &pipeline, config.format, size);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            texture,
            bind_groups,
            size,
            max_dimension,
            backend: if info.backend == wgpu::Backend::BrowserWebGpu {
                "webgpu"
            } else {
                "webgl"
            },
            adapter: format!("{} ({:?})", info.name, info.device_type),
        })
    }

    fn create_texture(
        device: &wgpu::Device,
        pipeline: &wgpu::RenderPipeline,
        target_format: wgpu::TextureFormat,
        size: FrameSize,
    ) -> (wgpu::Texture, [wgpu::BindGroup; 2]) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("VDP RGBA framebuffer"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: if target_format.is_srgb() {
                wgpu::TextureFormat::Rgba8UnormSrgb
            } else {
                wgpu::TextureFormat::Rgba8Unorm
            },
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        // Prebuild both choices when the source texture changes. Switching a
        // setting only selects a bind group; no sampler/texture allocation.
        let bind_groups = std::array::from_fn(|index| {
            let filter = if index == 0 {
                wgpu::FilterMode::Nearest
            } else {
                wgpu::FilterMode::Linear
            };
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("Framebuffer sampling"),
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            });
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Framebuffer and sampler"),
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        });
        (texture, bind_groups)
    }

    pub fn render(
        &mut self,
        pixels: &[Color],
        size: FrameSize,
        width: u32,
        height: u32,
        integer_scale: bool,
        linear_filter: bool,
    ) -> Result<bool, String> {
        if width == 0 || height == 0 {
            return Ok(false);
        }
        let width = width.min(self.max_dimension);
        let height = height.min(self.max_dimension);
        if self.config.width != width || self.config.height != height {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
        }
        if size != self.size {
            (self.texture, self.bind_groups) =
                Self::create_texture(&self.device, &self.pipeline, self.config.format, size);
            self.size = size;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&pixels[..size.len() as usize]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * size.width),
                rows_per_image: Some(size.height),
            },
            wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
        );
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(false);
            }
            error => return Err(format!("canvas presentation failed: {error:?}")),
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Scaled VDP frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            let mut scale =
                (width as f32 / size.width as f32).min(height as f32 / size.height as f32);
            if integer_scale && scale >= 1.0 {
                scale = scale.floor();
            }
            let view_width = size.width as f32 * scale;
            let view_height = size.height as f32 * scale;
            pass.set_viewport(
                ((width as f32 - view_width) * 0.5).floor(),
                ((height as f32 - view_height) * 0.5).floor(),
                view_width,
                view_height,
                0.0,
                1.0,
            );
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_groups[usize::from(linear_filter)], &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        Ok(true)
    }

    pub fn backend(&self) -> &str {
        self.backend
    }

    pub fn adapter(&self) -> &str {
        &self.adapter
    }
}
