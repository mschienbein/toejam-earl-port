//! Explicit GPU acceptance test. Uses only a newly authored diagnostic pattern.
//! TJE_SHADER_ARTIFACT_DIR=/private/scratch/... cargo test -p tje-display --test pixels -- --ignored --nocapture
use std::{borrow::Cow, path::Path, time::Instant};
use tje_display::{DisplayShader, EFFECTS_WGSL};
use wgpu::util::DeviceExt;

const NATIVE: &str = include_str!("../../tje-native/src/present.wgsl");
const WEB: &str = include_str!("../../tje-web/src/present.wgsl");

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Gpu {
    fn new() -> (Self, String) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("GPU pixel acceptance needs a Vulkan adapter (software is fine)");
        let info = format!("{:?}", adapter.get_info());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("display pixel acceptance"),
            required_limits: wgpu::Limits::downlevel_defaults(),
            ..Default::default()
        }))
        .unwrap();
        (Self { device, queue }, info)
    }

    fn pipeline(&self, native: bool, before: bool, srgb_target: bool) -> wgpu::RenderPipeline {
        let source = if before {
            Cow::Borrowed(if native {
                include_str!("data/native-before.wgsl")
            } else {
                include_str!("data/web-before.wgsl")
            })
        } else {
            Cow::Owned(format!(
                "{}\n{EFFECTS_WGSL}",
                if native { NATIVE } else { WEB }
            ))
        };
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("presenter under test"),
                source: wgpu::ShaderSource::Wgsl(source),
            });
        let constants: &[(&str, f64)] = if native {
            &[("encode_srgb", (!srgb_target) as u32 as f64)]
        } else if !before {
            &[("target_srgb", srgb_target as u32 as f64)]
        } else {
            &[]
        };
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("presenter pixel oracle"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(if native { "vs_main" } else { "vertex" }),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(if native { "fs_main" } else { "fragment" }),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format(srgb_target),
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
    }

    fn scene(&self, pipeline: &wgpu::RenderPipeline, case: Case) -> Scene {
        let [width, height] = case.source;
        let srgb_texture = (case.native || case.srgb_target) && case.preset == DisplayShader::Off;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("authored test pattern"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: target_format(srgb_texture),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            texture.as_image_copy(),
            &pattern(width, height),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            texture.size(),
        );
        let view = texture.create_view(&Default::default());
        let filtering = if case.filter == 1 && case.preset != DisplayShader::Smooth {
            wgpu::FilterMode::Nearest
        } else {
            wgpu::FilterMode::Linear
        };
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: filtering,
            min_filter: filtering,
            ..Default::default()
        });
        let flags = [
            if case.native {
                u32::from(case.filter == 0)
            } else {
                case.filter
            },
            case.preset as u32,
            0,
            0,
        ];
        let uniform = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("presentation preset"),
                contents: bytemuck::cast_slice(&flags),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("presentation test inputs"),
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let [width, height] = case.target;
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("readable screenshot target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: target_format(case.srgb_target),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Scene { bind_group, target }
    }

    fn draw(&self, pipeline: &wgpu::RenderPipeline, scene: &Scene) {
        let view = scene.target.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &scene.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
    }

    fn pixels(&self, pipeline: &wgpu::RenderPipeline, case: Case) -> Vec<u8> {
        let scene = self.scene(pipeline, case);
        self.draw(pipeline, &scene);
        let [width, height] = case.target;
        let stride = (width * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("padded readback"),
            size: u64::from(stride * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            scene.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(height),
                },
            },
            scene.target.size(),
        );
        self.queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |result| {
            sender.send(result).unwrap()
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        receiver.recv().unwrap().unwrap();
        let mapped = buffer.get_mapped_range(..).unwrap();
        mapped
            .chunks_exact(stride as usize)
            .flat_map(|row| row[..width as usize * 4].iter().copied())
            .collect()
    }

    fn median_ms(&self, pipeline: &wgpu::RenderPipeline, scene: &Scene) -> f64 {
        for _ in 0..5 {
            self.draw(pipeline, scene);
        }
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let mut samples = Vec::new();
        for _ in 0..31 {
            let start = Instant::now();
            self.draw(pipeline, scene);
            self.device
                .poll(wgpu::PollType::wait_indefinitely())
                .unwrap();
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        samples[samples.len() / 2]
    }
}

#[derive(Clone, Copy)]
struct Case {
    source: [u32; 2],
    target: [u32; 2],
    native: bool,
    srgb_target: bool,
    filter: u32,
    preset: DisplayShader,
}

struct Scene {
    bind_group: wgpu::BindGroup,
    target: wgpu::Texture,
}

fn target_format(srgb: bool) -> wgpu::TextureFormat {
    if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    }
}

fn pattern(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let rgb = if x < 4
                || y < 4
                || x >= width - 4
                || y >= height - 4
                || (x < width / 3 && y < height / 3)
            {
                [255, 255, 255]
            } else if y < height / 2 {
                [(x * 255 / width) as u8, (y * 255 / height) as u8, 130]
            } else if (x / 3 + y / 3) % 2 == 0 {
                [230, 50, 80]
            } else {
                [25, 190, 220]
            };
            pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    pixels
}

fn capture(directory: Option<&Path>, name: &str, size: [u32; 2], pixels: &[u8]) {
    if let Some(directory) = directory {
        let file = std::fs::File::create(directory.join(format!("{name}.png"))).unwrap();
        let mut encoder = png::Encoder::new(file, size[0], size[1]);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(pixels)
            .unwrap();
    }
}

#[test]
#[ignore = "explicit Vulkan GPU pixel/screenshot/performance acceptance"]
fn faithful_pixels_shared_effects_and_performance() {
    let artifact_path = std::env::var_os("TJE_SHADER_ARTIFACT_DIR").map(std::path::PathBuf::from);
    if let Some(path) = &artifact_path {
        assert!(
            path.components().any(|part| part.as_os_str() == "scratch"),
            "screenshots stay private under scratch/"
        );
        std::fs::create_dir_all(path).unwrap();
    }
    let directory = artifact_path.as_deref();
    let (gpu, adapter) = Gpu::new();
    let target = [960, 672];
    let mut off_checks = 0;
    // All legacy filters, both target encodings, exact/fractional/sub-1x sizes,
    // and dynamic H32/interlace/extended source sizes retain byte-identical Off.
    for native in [true, false] {
        for srgb_target in [false, true] {
            let before = gpu.pipeline(native, true, srgb_target);
            let after = gpu.pipeline(native, false, srgb_target);
            for (source, target) in [
                ([320, 224], target),
                ([320, 224], [977, 711]),
                ([256, 240], [191, 137]),
                ([320, 448], [941, 799]),
                ([400, 224], [1200, 672]),
            ] {
                for filter in 0..3 {
                    let case = Case {
                        source,
                        target,
                        native,
                        srgb_target,
                        filter,
                        preset: DisplayShader::Off,
                    };
                    let expected = gpu.pixels(&before, case);
                    let actual = gpu.pixels(&after, case);
                    assert!(
                        actual == expected,
                        "Off changed: native={native} srgb={srgb_target} source={source:?} target={target:?} filter={filter}"
                    );
                    off_checks += 1;
                    if source == [320, 224] && target == [960, 672] && filter == 0 && !srgb_target {
                        let frontend = if native { "native" } else { "web" };
                        capture(directory, &format!("{frontend}-before"), target, &expected);
                        capture(directory, &format!("{frontend}-off"), target, &actual);
                    }
                }
            }
        }
    }
    let native = gpu.pipeline(true, false, false);
    let srgb_native = gpu.pipeline(true, false, true);
    let web = gpu.pipeline(false, false, false);
    let srgb_web = gpu.pipeline(false, false, true);
    let case = Case {
        source: [320, 224],
        target,
        native: true,
        srgb_target: false,
        filter: 0,
        preset: DisplayShader::Off,
    };
    let off = gpu.pixels(&native, case);
    let mut comparisons = Vec::new();
    let mut performance = Vec::new();
    for preset in DisplayShader::ALL {
        let case = Case { preset, ..case };
        for srgb_target in [false, true] {
            let pipeline = if srgb_target { &srgb_native } else { &native };
            let scene = gpu.scene(
                pipeline,
                Case {
                    srgb_target,
                    ..case
                },
            );
            let median_ms = gpu.median_ms(pipeline, &scene);
            performance.push(serde_json::json!({ "preset": preset, "srgbTarget": srgb_target, "medianCompletionMs": median_ms, "samples": 31, "target": target }));
        }
        if preset == DisplayShader::Off {
            continue;
        }
        for filter in 0..3 {
            let case = Case { filter, ..case };
            let actual = gpu.pixels(&native, case);
            let browser = gpu.pixels(
                &web,
                Case {
                    native: false,
                    ..case
                },
            );
            let srgb_browser = gpu.pixels(
                &srgb_web,
                Case {
                    native: false,
                    srgb_target: true,
                    ..case
                },
            );
            let native_srgb = gpu.pixels(
                &srgb_native,
                Case {
                    srgb_target: true,
                    ..case
                },
            );
            let native_srgb_delta = native_srgb
                .iter()
                .zip(&browser)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(
                native_srgb_delta <= 2,
                "{preset:?}/{filter}: native sRGB/browser delta {native_srgb_delta}"
            );
            let max_delta = actual
                .iter()
                .zip(&browser)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            let srgb_delta = browser
                .iter()
                .zip(&srgb_browser)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(
                max_delta <= 1,
                "{preset:?}/{filter}: frontend color delta {max_delta}"
            );
            // sRGB render targets add hardware transfer-function/filtering
            // rounding. Allow two code values there, not a spatial/color drift.
            assert!(
                srgb_delta <= 2,
                "{preset:?}/{filter}: browser target encoding delta {srgb_delta}, first differing pixels {:?}",
                browser
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(srgb_browser.as_chunks::<4>().0)
                    .enumerate()
                    .find(|(_, (a, b))| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 2))
            );
            assert!(
                actual != off,
                "opt-in preset must visibly change the diagnostic pattern"
            );
            if preset == DisplayShader::CrtCurved {
                assert_eq!(&actual[..3], &[0, 0, 0], "curved corners must be black");
            }
            comparisons.push(serde_json::json!({ "preset": preset, "filter": filter, "nativeWebMaxByteDelta": max_delta, "webSrgbMaxByteDelta": srgb_delta, "nativeSrgbWebMaxByteDelta": native_srgb_delta }));
            if filter == 0 {
                let name = serde_json::to_string(&preset)
                    .unwrap()
                    .trim_matches('"')
                    .to_owned();
                capture(directory, &format!("native-{name}"), target, &actual);
                capture(
                    directory,
                    &format!("native-srgb-{name}"),
                    target,
                    &native_srgb,
                );
                capture(directory, &format!("web-{name}"), target, &browser);
            }
        }
    }
    let evidence = serde_json::json!({ "adapter": adapter, "faithfulPixelComparisons": off_checks, "effects": comparisons, "performance": performance,
        "timingMethod": "CPU wall time per submit plus GPU completion, 5 warmups then 31 samples, presentation only; no engine or readback in timed region" });
    println!("{evidence}");
    if let Some(directory) = directory {
        std::fs::write(
            directory.join("pixels-and-performance.json"),
            serde_json::to_string_pretty(&evidence).unwrap(),
        )
        .unwrap();
    }
}
