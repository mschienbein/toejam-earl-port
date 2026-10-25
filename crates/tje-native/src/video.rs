use anyhow::{Context, Result, bail};
use std::sync::Arc;
use tje_native::{
    backend::VideoFrame,
    config::{DisplayShader, Filter, PixelShape, Scale},
};
use wgpu::util::DeviceExt;
use winit::{dpi::PhysicalSize, event_loop::ActiveEventLoop, window::Window};

pub struct Video {
    _instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    texture: wgpu::Texture,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    filter_uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    display_shader: DisplayShader,
    ui_renderer: egui_wgpu::Renderer,
    pub presented: u64,
    suspended: bool,
}

impl Video {
    pub async fn new(
        window: Arc<Window>,
        event_loop: &ActiveEventLoop,
        backend: &str,
        filter: Filter,
        display_shader: DisplayShader,
        vsync: bool,
    ) -> Result<Self> {
        let backends = match backend {
            "auto" => wgpu::Backends::all(),
            "vulkan" => wgpu::Backends::VULKAN,
            "gl" => wgpu::Backends::GL,
            "dx12" => wgpu::Backends::DX12,
            "metal" => wgpu::Backends::METAL,
            _ => bail!("unknown GPU backend {backend}; use auto/vulkan/gl/dx12/metal"),
        };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
                event_loop.owned_display_handle(),
            ))
        });
        let surface = instance
            .create_surface(window.clone())
            .context("create window surface")?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .context("find graphics adapter")?;
        let info = adapter.get_info();
        println!(
            "VIDEO backend={:?} adapter={} type={:?}",
            info.backend, info.name, info.device_type
        );
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("ToeJam & Earl"),
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .context("create graphics device")?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .context("surface configuration")?;
        if let Some(format) = surface
            .get_capabilities(&adapter)
            .formats
            .into_iter()
            .find(wgpu::TextureFormat::is_srgb)
        {
            config.format = format;
        }
        config.present_mode = if vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        config.desired_maximum_frame_latency = 2;
        surface.configure(&device, &config);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("game presentation and optional display effects"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    include_str!("present.wgsl"),
                    tje_display::EFFECTS_WGSL
                )
                .into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("game frame"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("game presentation"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("game presentation"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &[("encode_srgb", (!config.format.is_srgb()) as u32 as f64)],
                    ..Default::default()
                },
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let texture = texture(&device, 320, 224, display_shader);
        let sampler = sampler(&device, filter, display_shader);
        let filter_uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("pixel filter mode"),
            contents: bytemuck::cast_slice(&filter_flags(filter, display_shader)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group = bind_group(&device, &layout, &texture, &sampler, &filter_uniform);
        let ui_renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions::default(),
        );
        Ok(Self {
            _instance: instance,
            surface,
            device,
            queue,
            config,
            pipeline,
            texture,
            layout,
            sampler,
            filter_uniform,
            bind_group,
            display_shader,
            ui_renderer,
            presented: 0,
            suspended: size.width == 0 || size.height == 0,
        })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        self.suspended = size.width == 0 || size.height == 0;
        if self.suspended {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn set_vsync(&mut self, enabled: bool) {
        self.config.present_mode = if enabled {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        if !self.suspended {
            self.surface.configure(&self.device, &self.config);
        }
    }

    pub fn set_presentation(&mut self, filter: Filter, display_shader: DisplayShader) {
        self.display_shader = display_shader;
        if self.texture.format() != texture_format(display_shader) {
            // Re-upload the current engine frame at the caller, including when
            // paused. Real texture formats also work on GL without view casts.
            self.texture = texture(
                &self.device,
                self.texture.width(),
                self.texture.height(),
                display_shader,
            );
        }
        self.sampler = sampler(&self.device, filter, display_shader);
        self.queue.write_buffer(
            &self.filter_uniform,
            0,
            bytemuck::cast_slice(&filter_flags(filter, display_shader)),
        );
        self.bind_group = bind_group(
            &self.device,
            &self.layout,
            &self.texture,
            &self.sampler,
            &self.filter_uniform,
        );
    }

    pub fn upload(&mut self, frame: VideoFrame<'_>) {
        if self.texture.width() != frame.width || self.texture.height() != frame.height {
            self.texture = texture(&self.device, frame.width, frame.height, self.display_shader);
            self.bind_group = bind_group(
                &self.device,
                &self.layout,
                &self.texture,
                &self.sampler,
                &self.filter_uniform,
            );
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            frame.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.width * 4),
                rows_per_image: Some(frame.height),
            },
            self.texture.size(),
        );
    }

    pub fn present(
        &mut self,
        window: &Window,
        pixel_shape: PixelShape,
        scale: Scale,
        has_game: bool,
        mut ui: crate::overlay::Frame,
    ) -> Result<()> {
        for (id, deltas) in &ui.textures.set {
            for delta in deltas {
                self.ui_renderer
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        let result = self.present_inner(window, pixel_shape, scale, has_game, &ui);
        for id in &ui.textures.free {
            self.ui_renderer.free_texture(id);
        }
        ui.textures.clear();
        result
    }

    fn present_inner(
        &mut self,
        window: &Window,
        pixel_shape: PixelShape,
        scale: Scale,
        has_game: bool,
        ui: &crate::overlay::Frame,
    ) -> Result<()> {
        if self.suspended {
            return Ok(());
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            other => bail!("acquire surface: {other:?}"),
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.config.width, self.config.height],
            pixels_per_point: ui.pixels_per_point,
        };
        let command_buffers = self.ui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            &ui.jobs,
            &screen,
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present game"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.015,
                            g: 0.035,
                            b: 0.04,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if has_game {
                let [x, y, width, height] = viewport(
                    self.config.width,
                    self.config.height,
                    self.texture.width(),
                    self.texture.height(),
                    pixel_shape,
                    scale,
                );
                pass.set_viewport(x as f32, y as f32, width as f32, height as f32, 0.0, 1.0);
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        if !ui.jobs.is_empty() {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("native controls overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.ui_renderer
                .render(&mut pass.forget_lifetime(), &ui.jobs, &screen);
        }
        self.queue
            .submit(command_buffers.into_iter().chain([encoder.finish()]));
        window.pre_present_notify();
        self.queue.present(frame);
        self.presented += 1;
        Ok(())
    }
}

fn texture_format(display_shader: DisplayShader) -> wgpu::TextureFormat {
    if display_shader == DisplayShader::Off {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    }
}

fn texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    display_shader: DisplayShader,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("VDP RGBA"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: texture_format(display_shader),
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn sampler(device: &wgpu::Device, filter: Filter, display_shader: DisplayShader) -> wgpu::Sampler {
    let filter = if filter == Filter::Nearest && display_shader != DisplayShader::Smooth {
        wgpu::FilterMode::Nearest
    } else {
        wgpu::FilterMode::Linear
    };
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("pixel filter"),
        mag_filter: filter,
        min_filter: filter,
        ..Default::default()
    })
}

fn filter_flags(filter: Filter, display_shader: DisplayShader) -> [u32; 4] {
    [
        u32::from(filter == Filter::Sharp),
        display_shader as u32,
        0,
        0,
    ]
}

fn bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    texture: &wgpu::Texture,
    sampler: &wgpu::Sampler,
    filter_uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("VDP pixels and filter"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &texture.create_view(&Default::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: filter_uniform.as_entire_binding(),
            },
        ],
    })
}

/// Shared with the browser via tests/data/viewport-vectors.json. Only the GPU
/// boundary converts to f32, keeping floor/fit decisions deterministic.
fn viewport(
    window_width: u32,
    window_height: u32,
    width: u32,
    height: u32,
    pixel_shape: PixelShape,
    scale: Scale,
) -> [f64; 4] {
    let ww = f64::from(window_width);
    let wh = f64::from(window_height);
    let w = f64::from(width);
    let h = f64::from(height);
    // Genesis NTSC pixel aspect: H32=8/7, H40=32/35. An interlaced
    // doubled-height buffer needs the corresponding doubled PAR.
    let par = match pixel_shape {
        PixelShape::Native => {
            let normal_par = if width < 300 { 8.0 / 7.0 } else { 32.0 / 35.0 };
            normal_par * if height >= 448 { 2.0 } else { 1.0 }
        }
        PixelShape::Square => 1.0,
    };
    let fit = (ww / (w * par)).min(wh / h);
    let factor = match scale {
        Scale::Integer if fit >= 1.0 => fit.floor(),
        _ => fit,
    };
    let display_width = w * par * factor;
    let display_height = h * factor;
    [
        ((ww - display_width) / 2.0).floor(),
        ((wh - display_height) / 2.0).floor(),
        display_width,
        display_height,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Vectors {
        version: u32,
        vectors: Vec<Vector>,
    }

    #[derive(serde::Deserialize)]
    struct Vector {
        name: String,
        source: [u32; 2],
        target: [u32; 2],
        pixel_shape: PixelShape,
        scale: Scale,
        expected: [f64; 4],
    }

    #[test]
    fn shared_viewport_vectors() {
        let data: Vectors =
            serde_json::from_str(include_str!("../tests/data/viewport-vectors.json")).unwrap();
        assert_eq!(data.version, 1);
        assert_eq!(data.vectors.len(), 192);
        for vector in data.vectors {
            let actual = viewport(
                vector.target[0],
                vector.target[1],
                vector.source[0],
                vector.source[1],
                vector.pixel_shape,
                vector.scale,
            );
            for (actual, expected) in actual.into_iter().zip(vector.expected) {
                assert!(
                    (actual - expected).abs() < 0.0001,
                    "{}: {actual} != {expected}",
                    vector.name
                );
            }
        }
    }

    #[test]
    fn default_filter_enables_sharp_bilinear_only() {
        assert_eq!(
            filter_flags(Filter::default(), DisplayShader::default()),
            [1, 0, 0, 0]
        );
        assert_eq!(filter_flags(Filter::Nearest, DisplayShader::Off), [0; 4]);
        assert_eq!(filter_flags(Filter::Linear, DisplayShader::Off), [0; 4]);
        for preset in DisplayShader::ALL {
            assert_eq!(
                filter_flags(Filter::Sharp, preset),
                [1, preset as u32, 0, 0]
            );
        }
    }
}
