#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use egui_wgpu::ScreenDescriptor;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Instant;
use wgpu::util::DeviceExt;
use winit::{
    dpi::PhysicalSize,
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    window::WindowBuilder,
};

mod font;
mod leftpanel;
mod pty;
#[path = "startup-page.rs"]
mod startup_page;
mod terminal;
mod devtools;
mod topbar;
mod quickcmd;
mod settings;

const WINDOW_WIDTH: u32 = 1638;
const WINDOW_HEIGHT: u32 = 1024;
const SQUARE_SIZE: f32 = 200.0;
const FONT_SIZE: f32 = 120.0;
const ENABLE_QUICKCMD_KEYBINDINGS: bool = true;
struct UiState {
    terminal: Option<terminal::TerminalInstance>,
    terminal_selection: terminal::TerminalSelectionState,
    pending_terminal: Option<terminal::TerminalInstance>,
    terminal_init_error: Option<String>,
    terminal_exited: bool,
    terminal_connecting: bool,
    reconnect_requested: bool,
    terminal_scroll_request: Option<terminal::ScrollRequest>,
    terminal_scroll_request_frames_left: u8,
    terminal_scroll_id: u64,
    terminal_view_size_px: egui::Vec2,
    pty_render_size_px: egui::Vec2,
    pty_grid_size: (usize, usize),
    loading_started_at: Instant,
    startup_dir: PathBuf,
    close_confirm_open: bool,
    close_confirmed: bool,
    close_focus_pending: bool,
    devtools_open: bool,
    devtools_state: devtools::DevToolsState,
    quickcmd_config: quickcmd::QuickCommandConfig,
    settings_state: settings::SettingsState,
    /// Pending quick command to write to PTY (set by UI, consumed by event loop).
    pending_quick_cmd: Option<(String, bool)>,
    /// Terminal content area rect (egui points), used for file-drop hit testing.
    terminal_drop_rect: Option<egui::Rect>,
    /// Latest cursor position in egui points.
    last_cursor_pos: Option<egui::Pos2>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    screen_size: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ColorVertex {
    position: [f32; 2],
    color: [f32; 4],
}

impl ColorVertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ColorVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlyphVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

impl GlyphVertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GlyphVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                },
            ],
        }
    }
}

struct GlyphTexture {
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    width: u32,
    height: u32,
}

struct State {
    window: Arc<winit::window::Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,

    uniforms: Uniforms,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,

    color_pipeline: wgpu::RenderPipeline,
    glyph_pipeline: wgpu::RenderPipeline,

    square_vertex_buffer: wgpu::Buffer,
    glyph_vertex_buffer: wgpu::Buffer,
    glyph_vertex_count: u32,

    glyph_bind_group_layout: wgpu::BindGroupLayout,
    glyph_bind_group: wgpu::BindGroup,
    glyph_texture: GlyphTexture,
    glyph_dims: Option<(u32, u32)>,

    font: font::FontRasterizer,
}

impl State {
    async fn new(window: Arc<winit::window::Window>) -> Self {
        let size = window.inner_size();

        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .expect("Create surface");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Request adapter");

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await
            .expect("Request device");

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: surface_caps.present_modes[0],
            desired_maximum_frame_latency: 2,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let uniforms = Uniforms {
            screen_size: [config.width as f32, config.height as f32],
            _pad: [0.0; 2],
        };
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("uniform buffer"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("uniform bind group layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniform bind group"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let glyph_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("glyph bind group layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("main shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let color_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("color pipeline layout"),
                bind_group_layouts: &[&uniform_bind_group_layout],
                push_constant_ranges: &[],
            });

        let color_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("color pipeline"),
            layout: Some(&color_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_color",
                buffers: &[ColorVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_color",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let glyph_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("glyph pipeline layout"),
                bind_group_layouts: &[&glyph_bind_group_layout],
                push_constant_ranges: &[],
            });

        let glyph_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("glyph pipeline"),
            layout: Some(&glyph_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_glyph",
                buffers: &[GlyphVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_glyph",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let square_vertices = make_square_vertices(size);
        let square_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("square vertex buffer"),
            contents: bytemuck::cast_slice(&square_vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let glyph_vertices = [GlyphVertex {
            position: [0.0, 0.0],
            uv: [0.0, 0.0],
        }; 6];
        let glyph_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("glyph vertex buffer"),
            contents: bytemuck::cast_slice(&glyph_vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let glyph_texture = create_empty_glyph_texture(&device);
        let glyph_bind_group = create_glyph_bind_group(
            &device,
            &glyph_bind_group_layout,
            &uniform_buffer,
            &glyph_texture,
        );

        let font = font::FontRasterizer::load_system();

        Self {
            window,
            surface,
            device,
            queue,
            config,
            size,
            uniforms,
            uniform_buffer,
            uniform_bind_group,
            color_pipeline,
            glyph_pipeline,
            square_vertex_buffer,
            glyph_vertex_buffer,
            glyph_vertex_count: 0,
            glyph_bind_group_layout,
            glyph_bind_group,
            glyph_texture,
            glyph_dims: None,
            font,
        }
    }

    fn window(&self) -> &winit::window::Window {
        self.window.as_ref()
    }

    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);

        self.uniforms.screen_size = [self.config.width as f32, self.config.height as f32];
        self.queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));

        self.update_square_vertices();
        self.update_glyph_vertices();
    }

    fn update_square_vertices(&mut self) {
        let vertices = make_square_vertices(self.size);
        self.queue.write_buffer(
            &self.square_vertex_buffer,
            0,
            bytemuck::cast_slice(&vertices),
        );
    }

    fn update_glyph_vertices(&mut self) {
        if let Some((w, h)) = self.glyph_dims {
            let vertices = make_glyph_vertices(self.size, w as f32, h as f32);
            self.queue.write_buffer(
                &self.glyph_vertex_buffer,
                0,
                bytemuck::cast_slice(&vertices),
            );
            self.glyph_vertex_count = 6;
        } else {
            self.glyph_vertex_count = 0;
        }
    }

    fn set_glyph(&mut self, ch: char) {
        // Rasterize glyph into a grayscale bitmap and upload to GPU.
        let (metrics, bitmap) = self.font.rasterize(ch, FONT_SIZE);
        if metrics.width == 0 || metrics.height == 0 {
            self.glyph_dims = None;
            self.glyph_vertex_count = 0;
            return;
        }

        let (padded, row_pitch) = pad_glyph(&bitmap, metrics.width as u32, metrics.height as u32);
        let extent = wgpu::Extent3d {
            width: metrics.width as u32,
            height: metrics.height as u32,
            depth_or_array_layers: 1,
        };

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph texture"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &padded,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(row_pitch),
                rows_per_image: Some(metrics.height as u32),
            },
            extent,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("glyph sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        self.glyph_texture = GlyphTexture {
            view,
            sampler,
            width: metrics.width as u32,
            height: metrics.height as u32,
        };
        self.glyph_bind_group = create_glyph_bind_group(
            &self.device,
            &self.glyph_bind_group_layout,
            &self.uniform_buffer,
            &self.glyph_texture,
        );

        self.glyph_dims = Some((self.glyph_texture.width, self.glyph_texture.height));
        self.update_glyph_vertices();
    }

    fn render_with_egui(
        &mut self,
        egui_renderer: &mut egui_wgpu::Renderer,
        paint_jobs: &[egui::epaint::ClippedPrimitive],
        screen_desc: &ScreenDescriptor,
    ) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render encoder"),
            });

        egui_renderer.update_buffers(
            &self.device,
            &self.queue,
            &mut encoder,
            paint_jobs,
            screen_desc,
        );

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.12,
                            g: 0.12,
                            b: 0.12,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            rpass.set_pipeline(&self.color_pipeline);
            rpass.set_bind_group(0, &self.uniform_bind_group, &[]);
            rpass.set_vertex_buffer(0, self.square_vertex_buffer.slice(..));
            rpass.draw(0..6, 0..1);

            if self.glyph_vertex_count > 0 {
                rpass.set_pipeline(&self.glyph_pipeline);
                rpass.set_bind_group(0, &self.glyph_bind_group, &[]);
                rpass.set_vertex_buffer(0, self.glyph_vertex_buffer.slice(..));
                rpass.draw(0..self.glyph_vertex_count, 0..1);
            }

            egui_renderer.render(&mut rpass, paint_jobs, screen_desc);
        }

        self.queue.submit(Some(encoder.finish()));
        output.present();
        Ok(())
    }
}

fn make_square_vertices(size: PhysicalSize<u32>) -> [ColorVertex; 6] {
    let (x0, y0, x1, y1) = centered_rect(size, SQUARE_SIZE, SQUARE_SIZE);
    let color = [0.0, 0.0, 0.0, 1.0];
    [
        ColorVertex {
            position: [x0, y0],
            color,
        },
        ColorVertex {
            position: [x1, y0],
            color,
        },
        ColorVertex {
            position: [x1, y1],
            color,
        },
        ColorVertex {
            position: [x0, y0],
            color,
        },
        ColorVertex {
            position: [x1, y1],
            color,
        },
        ColorVertex {
            position: [x0, y1],
            color,
        },
    ]
}

fn make_glyph_vertices(size: PhysicalSize<u32>, glyph_w: f32, glyph_h: f32) -> [GlyphVertex; 6] {
    let (square_x0, square_y0, square_x1, square_y1) =
        centered_rect(size, SQUARE_SIZE, SQUARE_SIZE);
    let square_cx = (square_x0 + square_x1) * 0.5;
    let square_cy = (square_y0 + square_y1) * 0.5;

    let x0 = square_cx - glyph_w * 0.5;
    let y0 = square_cy - glyph_h * 0.5;
    let x1 = square_cx + glyph_w * 0.5;
    let y1 = square_cy + glyph_h * 0.5;

    [
        GlyphVertex {
            position: [x0, y0],
            uv: [0.0, 0.0],
        },
        GlyphVertex {
            position: [x1, y0],
            uv: [1.0, 0.0],
        },
        GlyphVertex {
            position: [x1, y1],
            uv: [1.0, 1.0],
        },
        GlyphVertex {
            position: [x0, y0],
            uv: [0.0, 0.0],
        },
        GlyphVertex {
            position: [x1, y1],
            uv: [1.0, 1.0],
        },
        GlyphVertex {
            position: [x0, y1],
            uv: [0.0, 1.0],
        },
    ]
}

fn centered_rect(size: PhysicalSize<u32>, width: f32, height: f32) -> (f32, f32, f32, f32) {
    let cx = size.width as f32 * 0.5;
    let cy = size.height as f32 * 0.5;
    let x0 = cx - width * 0.5;
    let y0 = cy - height * 0.5;
    let x1 = cx + width * 0.5;
    let y1 = cy + height * 0.5;
    (x0, y0, x1, y1)
}

fn pad_glyph(bitmap: &[u8], width: u32, height: u32) -> (Vec<u8>, u32) {
    let row_pitch = ((width + 255) / 256) * 256;
    let mut padded = vec![0u8; (row_pitch * height) as usize];
    for y in 0..height as usize {
        let src_start = y * width as usize;
        let src_end = src_start + width as usize;
        let dst_start = y * row_pitch as usize;
        let dst_end = dst_start + width as usize;
        padded[dst_start..dst_end].copy_from_slice(&bitmap[src_start..src_end]);
    }
    (padded, row_pitch)
}

fn create_empty_glyph_texture(device: &wgpu::Device) -> GlyphTexture {
    let extent = wgpu::Extent3d {
        width: 1,
        height: 1,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("empty glyph texture"),
        size: extent,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("empty glyph sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    GlyphTexture {
        view,
        sampler,
        width: 1,
        height: 1,
    }
}

fn create_glyph_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    glyph_texture: &GlyphTexture,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("glyph bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&glyph_texture.view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&glyph_texture.sampler),
            },
        ],
    })
}

fn spawn_terminal_async(
    startup_dir: PathBuf,
) -> mpsc::Receiver<std::io::Result<terminal::TerminalInstance>> {
    let (terminal_init_tx, terminal_init_rx) =
        mpsc::channel::<std::io::Result<terminal::TerminalInstance>>();
    thread::spawn(move || {
        let result = terminal::TerminalInstance::new(24, 80, startup_dir);
        let _ = terminal_init_tx.send(result);
    });
    terminal_init_rx
}

fn format_dropped_path_for_powershell(path: &std::path::Path) -> String {
    let raw = path.to_string_lossy();
    if raw.is_empty() {
        return String::new();
    }

    // PowerShell single-quoted string escaping: ' -> ''
    let escaped = raw.replace('\'', "''");
    format!("'{}' ", escaped)
}

fn show_close_confirm_dialog(ctx: &egui::Context, ui_state: &mut UiState) {
    if !ui_state.close_confirm_open {
        return;
    }

    // Draw a dim background behind the confirmation window.
    // Keep this layer non-interactive to avoid stealing pointer events
    // from the dialog buttons and drag handle.
    let screen_rect = ctx.screen_rect();
    let blocker_layer = egui::LayerId::new(
        egui::Order::Middle,
        egui::Id::new("close_confirm_modal_blocker"),
    );
    ctx.layer_painter(blocker_layer).rect_filled(
        screen_rect,
        0.0,
        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 70),
    );

    let window_size = egui::vec2(270.0, 130.0);
    let center = screen_rect.center();
    let default_pos = egui::pos2(
        center.x - window_size.x * 0.5,
        center.y - window_size.y * 0.5,
    );

    egui::Window::new("Confirm Close")
        .id(egui::Id::new("close_confirm_dialog"))
        .collapsible(false)
        .resizable(false)
        .fixed_size(window_size)
        .default_pos(default_pos)
        .movable(true)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);

            egui::Frame::none()
                .fill(egui::Color32::from_rgb(24, 24, 24))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(70)))
                .rounding(egui::Rounding::same(8.0))
                .inner_margin(egui::Margin::symmetric(12.0, 10.0))
                .show(ui, |ui| {
                    ui.set_min_size(egui::vec2(250.0, 105.0));

                    ui.label(
                        egui::RichText::new("Are you sure you want to close this window?")
                            .size(16.0)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new("Your current terminal session will be interrupted.")
                            .size(13.0),
                    );

                    ui.add_space(6.0);
                    let button_w = 92.0;
                    let button_h = 30.0;
                    let total_buttons_w = button_w * 2.0 + ui.spacing().item_spacing.x;
                    let left_pad = ((ui.available_width() - total_buttons_w) * 0.5).max(0.0);
                    ui.horizontal(|ui| {
                        ui.add_space(left_pad);
                        let close_button = egui::Button::new(
                            egui::RichText::new("Close")
                                .color(egui::Color32::WHITE)
                                .strong(),
                        )
                        .min_size(egui::vec2(button_w, button_h))
                        .fill(egui::Color32::from_rgb(45, 125, 235))
                        .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(90, 160, 255)));
                        let close_response = ui.add(close_button);
                        if ui_state.close_focus_pending {
                            close_response.request_focus();
                            ui_state.close_focus_pending = false;
                        }
                        if close_response.clicked() {
                            ui_state.close_confirm_open = false;
                            ui_state.close_confirmed = true;
                        }

                        let cancel_button =
                            egui::Button::new("Cancel").min_size(egui::vec2(button_w, button_h));
                        if ui.add(cancel_button).clicked() {
                            ui_state.close_confirm_open = false;
                        }
                    });
                });
        });
}

fn build_ui(
    ctx: &egui::Context,
    ui_state: &mut UiState,
    window: &winit::window::Window,
) -> Option<egui::Rect> {
    let screen_rect = ctx.screen_rect();
    let mut ime_cursor_rect = None;
    ui_state.terminal_drop_rect = None;

    let total_w = screen_rect.width().max(1.0);
    let right_w = if ui_state.devtools_open { total_w * 0.25 } else { 0.0 };

    let panel_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(70));
    let center_fill = if ui_state.terminal.is_none() {
        egui::Color32::from_rgb(14, 14, 14)
    } else {
        egui::Color32::from_gray(20)
    };

    let left_action = leftpanel::render(ctx, &mut ui_state.devtools_open);
    if left_action.open_settings {
        ui_state.settings_state.open = true;
    }

    if ui_state.devtools_open {
        let qcmd_action = devtools::render_devtools(
            ctx,
            &mut ui_state.devtools_state,
            ui_state.terminal.as_ref(),
            &ui_state.quickcmd_config,
            &mut ui_state.settings_state,
            right_w,
        );
        if let Some(act) = qcmd_action {
            ui_state.pending_quick_cmd = Some((act.command, act.auto_execute));
        }
    }

    // Settings modal (rendered on top)
    if settings::render_settings(ctx, &mut ui_state.settings_state, &mut ui_state.quickcmd_config) {
        quickcmd::save_config(&ui_state.quickcmd_config);
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(center_fill).stroke(panel_stroke))
        .show(ctx, |ui| {
            let origin = ui.min_rect().min;
            let available = ui.available_size();

            // ── Unified status bar parameters (adjust these to tune) ──
            let bar_h: f32 = 22.0;        // 状态栏高度（上下共用）
            let bar_pad: f32 = 14.0;       // 状态栏与终端之间的间距（上下共用）
            let bar_fade: f32 = 30.0;      // 渐变长度（上下共用）
            let bar_gray: u8 = 26;         // 状态栏底色灰度（上下共用）
            // ───────────────────────────────────────────────────────────

            let prompt_h = bar_h;
            let term_top_pad = bar_pad;
            let term_bot_pad = bar_pad;
            let bottom_h = bar_h;
            let terminal_h = (available.y - prompt_h - term_top_pad - term_bot_pad - bottom_h).max(0.0);

            let prompt_rect = egui::Rect::from_min_size(origin, egui::vec2(available.x, prompt_h));
            let term_left_pad: f32 = 8.0;
            let terminal_rect = egui::Rect::from_min_size(
                egui::pos2(origin.x + term_left_pad, origin.y + prompt_h + term_top_pad),
                egui::vec2((available.x - term_left_pad).max(0.0), terminal_h),
            );
            ui_state.terminal_drop_rect = Some(terminal_rect);
            let bottom_rect = egui::Rect::from_min_size(
                egui::pos2(origin.x, origin.y + prompt_h + term_top_pad + terminal_h + term_bot_pad),
                egui::vec2(available.x, bottom_h),
            );

            // Top area: custom title bar with reconnect controls + window buttons.
            ui.allocate_ui_at_rect(prompt_rect, |ui| {
                let action = topbar::render(
                    ui,
                    topbar::TopBarInput {
                        terminal_exited: ui_state.terminal_exited,
                        terminal_connecting: ui_state.terminal_connecting,
                        reconnect_requested: &mut ui_state.reconnect_requested,
                    },
                    egui::Color32::from_gray(bar_gray),
                );
                if action.request_minimize {
                    window.set_minimized(true);
                }
                if action.request_toggle_maximize {
                    window.set_maximized(!window.is_maximized());
                }
                if action.request_drag_window {
                    let _ = window.drag_window();
                }
                if action.request_close {
                    ui_state.close_confirm_open = true;
                    ui_state.close_focus_pending = true;
                }
            });

            // Middle area: terminal display
            ui.allocate_ui_at_rect(terminal_rect, |ui| {
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(18, 18, 18))
                    .show(ui, |ui| {
                        let available = ui.available_size();
                        ui_state.terminal_view_size_px = available;

                        if let Some(term) = ui_state.terminal.as_mut() {
                            let font_id = egui::FontId::monospace(terminal::TERM_FONT_SIZE);
                            let row_height = terminal::aligned_row_height(ui, &font_id);
                            let char_width = terminal::aligned_glyph_width(ui, &font_id, 'M');
                            if row_height > 0.0 && char_width > 0.0 {
                                let new_rows = (available.y / row_height).floor() as u16;
                                let new_cols = (available.x / char_width).floor() as u16;
                                if new_rows > 0
                                    && new_cols > 0
                                    && (new_rows as usize != term.rows()
                                        || new_cols as usize != term.cols())
                                {
                                    term.resize(new_rows, new_cols);
                                    ui_state.terminal_scroll_request =
                                        Some(terminal::ScrollRequest::ScreenTop);
                                    ui_state.terminal_scroll_request_frames_left = 30;
                                    ui_state.terminal_scroll_id =
                                        ui_state.terminal_scroll_id.wrapping_add(1);
                                }
                            }

                            let pty_cols = term.cols();
                            let pty_rows = term.rows();
                            ui_state.pty_grid_size = (pty_cols, pty_rows);
                            ui_state.pty_render_size_px = if row_height > 0.0 && char_width > 0.0 {
                                egui::vec2(
                                    char_width * pty_cols as f32,
                                    row_height * pty_rows as f32,
                                )
                            } else {
                                egui::Vec2::ZERO
                            };
                        } else {
                            ui_state.pty_grid_size = (0, 0);
                            ui_state.pty_render_size_px = egui::Vec2::ZERO;
                        }

                        if ui_state.terminal.is_some() {
                            let scroll_request = if ui_state.terminal_scroll_request_frames_left > 0
                            {
                                ui_state.terminal_scroll_request
                            } else {
                                None
                            };

                            ime_cursor_rect = terminal::render_terminal(
                                ui,
                                ui_state.terminal.as_ref(),
                                &mut ui_state.terminal_selection,
                                ui_state.close_confirm_open,
                                scroll_request,
                                ui_state.terminal_scroll_id,
                            );

                            if ui_state.terminal_scroll_request_frames_left > 0 {
                                ui_state.terminal_scroll_request_frames_left -= 1;
                                if ui_state.terminal_scroll_request_frames_left == 0 {
                                    ui_state.terminal_scroll_request = None;
                                }
                            }
                        } else {
                            startup_page::render(
                                ui,
                                ui_state.loading_started_at,
                                ui_state.terminal_init_error.as_deref(),
                            );
                        }
                    });
            });

            // Bottom area: reserve space (text painted later on top layer)
            ui.allocate_ui_at_rect(bottom_rect, |_ui| {});

            // --- Layer 1 (Foreground): gradient overlays on top of terminal content ---
            let fg_layer = egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("gradient_overlays"),
            );
            let fg_painter = ui.ctx().layer_painter(fg_layer);

            // Expand rects by 1px on each side to cover panel stroke edges
            let prompt_fill = prompt_rect.expand(1.0);
            let bottom_fill = bottom_rect.expand(1.0);

            let bar_color = egui::Color32::from_gray(bar_gray);
            let bar_transparent = egui::Color32::from_rgba_unmultiplied(bar_gray, bar_gray, bar_gray, 0);

            // Top gradient: solid → transparent (downward)
            {
                let grad_top = prompt_rect.bottom();
                let grad_bottom = grad_top + bar_fade;
                let mut mesh = egui::Mesh::default();
                mesh.colored_vertex(egui::pos2(prompt_fill.left(), grad_top), bar_color);
                mesh.colored_vertex(egui::pos2(prompt_fill.right(), grad_top), bar_color);
                mesh.colored_vertex(
                    egui::pos2(prompt_fill.right(), grad_bottom),
                    bar_transparent,
                );
                mesh.colored_vertex(
                    egui::pos2(prompt_fill.left(), grad_bottom),
                    bar_transparent,
                );
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                fg_painter.add(egui::Shape::mesh(mesh));
            }

            // Bottom status bar solid background
            fg_painter.rect_filled(bottom_fill, 0.0, bar_color);

            // Bottom gradient: transparent → solid (upward)
            {
                let grad_bottom = bottom_rect.top();
                let grad_top = grad_bottom - bar_fade;
                let mut mesh = egui::Mesh::default();
                mesh.colored_vertex(egui::pos2(bottom_fill.left(), grad_top), bar_transparent);
                mesh.colored_vertex(egui::pos2(bottom_fill.right(), grad_top), bar_transparent);
                mesh.colored_vertex(egui::pos2(bottom_fill.right(), grad_bottom), bar_color);
                mesh.colored_vertex(egui::pos2(bottom_fill.left(), grad_bottom), bar_color);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                fg_painter.add(egui::Shape::mesh(mesh));
            }

            // --- Layer 2 (Tooltip): text labels on top of gradients ---
            let text_layer = egui::LayerId::new(
                egui::Order::Tooltip,
                egui::Id::new("overlay_text"),
            );
            let text_painter = ui.ctx().layer_painter(text_layer);

            // Top prompt bar: reserved for future use

            // Bottom status text
            {
                let connect_status = if ui_state.terminal.is_some() {
                    if ui_state.terminal_exited {
                        "exited"
                    } else if ui_state.terminal_connecting {
                        "reconnecting"
                    } else {
                        "connected"
                    }
                } else if ui_state.terminal_init_error.is_some() {
                    "failed"
                } else {
                    "starting"
                };
                let status = format!(
                    "Terminal: {} | View: {:.0}x{:.0}px | PTY: {:.0}x{:.0}px ({}x{} cells)",
                    connect_status,
                    ui_state.terminal_view_size_px.x,
                    ui_state.terminal_view_size_px.y,
                    ui_state.pty_render_size_px.x,
                    ui_state.pty_render_size_px.y,
                    ui_state.pty_grid_size.0,
                    ui_state.pty_grid_size.1,
                );
                let font_id = egui::FontId::monospace(12.0);
                let galley = text_painter.layout_no_wrap(
                    status,
                    font_id,
                    egui::Color32::from_gray(120),
                );
                let text_pos = egui::pos2(bottom_rect.left() + 8.0, bottom_rect.top() + 8.0);
                text_painter.galley(text_pos, galley, egui::Color32::from_gray(120));
            }
        });

    show_close_confirm_dialog(ctx, ui_state);
    ime_cursor_rect
}

fn load_system_chinese_font() -> Option<Vec<u8>> {
    let font_paths = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyhbd.ttc",
        "C:\\Windows\\Fonts\\msyhl.ttc",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "C:\\Windows\\Fonts\\simsun.ttc",
        "C:\\Windows\\Fonts\\simkai.ttf",
    ];

    for path in font_paths {
        if let Ok(data) = std::fs::read(path) {
            return Some(data);
        }
    }

    None
}

fn main() {
    let startup_dir = resolve_startup_dir();

    let event_loop = EventLoop::new().expect("event loop");
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("terminrt")
            .with_inner_size(PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
            .with_decorations(false)
            .with_visible(false)
            .build(&event_loop)
            .expect("create window"),
    );
    window.set_ime_allowed(true);
    window.set_ime_purpose(winit::window::ImePurpose::Terminal);

    let mut state = pollster::block_on(State::new(window.clone()));
    let egui_ctx = egui::Context::default();
    if let Some(font_data) = load_system_chinese_font() {
        let mut fonts = egui::FontDefinitions::default();
        fonts
            .font_data
            .insert("zh".to_string(), egui::FontData::from_owned(font_data));
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .push("zh".to_string());
        fonts
            .families
            .get_mut(&egui::FontFamily::Monospace)
            .unwrap()
            .push("zh".to_string());
        egui_ctx.set_fonts(fonts);
    }
    let mut egui_state = egui_winit::State::new(
        egui_ctx.clone(),
        egui::ViewportId::ROOT,
        window.as_ref(),
        None,
        None,
    );
    let mut egui_renderer = egui_wgpu::Renderer::new(&state.device, state.config.format, None, 1);

    let mut terminal_init_rx = Some(spawn_terminal_async(startup_dir.clone()));

    let mut ui_state = UiState {
        terminal: None,
        terminal_selection: terminal::TerminalSelectionState::default(),
        pending_terminal: None,
        terminal_init_error: None,
        terminal_exited: false,
        terminal_connecting: true,
        reconnect_requested: false,
        terminal_scroll_request: None,
        terminal_scroll_request_frames_left: 0,
        terminal_scroll_id: 0,
        terminal_view_size_px: egui::Vec2::ZERO,
        pty_render_size_px: egui::Vec2::ZERO,
        pty_grid_size: (0, 0),
        loading_started_at: Instant::now(),
        startup_dir,
        close_confirm_open: false,
        close_confirmed: false,
        close_focus_pending: false,
        devtools_open: false,
        devtools_state: devtools::DevToolsState::default(),
        quickcmd_config: quickcmd::load_config(),
        settings_state: settings::SettingsState::default(),
        pending_quick_cmd: None,
        terminal_drop_rect: None,
        last_cursor_pos: None,
    };
    let mut window_shown = false;

    let mut current_modifiers = winit::event::Modifiers::default();

    let _ = event_loop.run(move |event, elwt| {
        match event {
            Event::WindowEvent { event, window_id } if window_id == state.window().id() => {
                let terminal_input_active = ui_state.terminal.is_some()
                    && !ui_state.close_confirm_open
                    && !ui_state.settings_state.open
                    && !ui_state.terminal_exited;

                // Track modifier state
                if let WindowEvent::ModifiersChanged(mods) = &event {
                    current_modifiers = mods.clone();
                }

                if let WindowEvent::CursorMoved { position, .. } = &event {
                    let scale = window.scale_factor() as f32;
                    if scale > 0.0 {
                        ui_state.last_cursor_pos = Some(egui::pos2(
                            position.x as f32 / scale,
                            position.y as f32 / scale,
                        ));
                    }
                }

                if let WindowEvent::DroppedFile(path) = &event {
                    let dropped_over_terminal = ui_state
                        .terminal_drop_rect
                        .zip(ui_state.last_cursor_pos)
                        .map(|(rect, pos)| rect.contains(pos))
                        .unwrap_or(false);

                    if terminal_input_active && dropped_over_terminal {
                        if let Some(ref mut terminal) = ui_state.terminal {
                            let dropped_text = format_dropped_path_for_powershell(path);
                            if !dropped_text.is_empty() {
                                ui_state.terminal_scroll_request =
                                    Some(terminal::ScrollRequest::CursorLine);
                                ui_state.terminal_scroll_request_frames_left = 1;
                                terminal.write_to_pty(dropped_text.as_bytes());
                            }
                        }
                    }
                }

                // Forward keyboard input to terminal BEFORE egui processes it
                if let WindowEvent::Ime(winit::event::Ime::Commit(text)) = &event {
                    if terminal_input_active && !text.is_empty() {
                        if let Some(ref mut terminal) = ui_state.terminal {
                            ui_state.terminal_scroll_request =
                                Some(terminal::ScrollRequest::CursorLine);
                            ui_state.terminal_scroll_request_frames_left = 1;
                            terminal.write_to_pty(text.as_bytes());
                        }
                    }
                }

                if let WindowEvent::KeyboardInput { ref event, .. } = event {
                    // --- Quick command keybinding matching ---
                    if ENABLE_QUICKCMD_KEYBINDINGS
                        && event.state.is_pressed()
                        && !event.repeat
                        && !ui_state.close_confirm_open
                        && !ui_state.settings_state.open
                        && !ui_state.terminal_exited
                        && ui_state.terminal.is_some()
                    {
                        let ctrl = current_modifiers.state().control_key();
                        let alt = current_modifiers.state().alt_key();
                        let shift = current_modifiers.state().shift_key();
                        let key_name = match &event.logical_key {
                            winit::keyboard::Key::Character(text) => {
                                Some(format!("{}", text.to_uppercase()))
                            }
                            winit::keyboard::Key::Named(named) => {
                                Some(format!("{:?}", named))
                            }
                            _ => None,
                        };

                        if let Some(kn) = key_name {
                            // Only match when at least one modifier is held
                            // (to avoid intercepting normal typing)
                            if ctrl || alt {
                                let probe = quickcmd::KeyBinding {
                                    ctrl,
                                    alt,
                                    shift,
                                    key: kn,
                                };
                                if let Some(cmd) = ui_state.quickcmd_config.find_by_keybinding(&probe) {
                                    ui_state.pending_quick_cmd =
                                        Some((cmd.command.clone(), cmd.auto_execute));
                                }
                            }
                        }
                    }

                    if let Some(ref mut terminal) = ui_state.terminal {
                        if terminal_input_active {
                            let ctrl = current_modifiers.state().control_key();
                            let is_ctrl_l = ctrl
                                && matches!(
                                    &event.logical_key,
                                    winit::keyboard::Key::Character(text) if text.eq_ignore_ascii_case("l")
                                );

                            if is_ctrl_l {
                                if event.state.is_pressed() && !event.repeat {
                                    ui_state.terminal_scroll_request =
                                        Some(terminal::ScrollRequest::ScreenTop);
                                    ui_state.terminal_scroll_request_frames_left = 60;
                                    ui_state.terminal_scroll_id =
                                        ui_state.terminal_scroll_id.wrapping_add(1);
                                    terminal.write_to_pty(&[0x0c]);
                                }
                            } else if let Some(input_bytes) =
                                terminal::key_to_terminal_input(event, &current_modifiers)
                            {
                                ui_state.terminal_scroll_request =
                                    Some(terminal::ScrollRequest::CursorLine);
                                ui_state.terminal_scroll_request_frames_left = 1;
                                terminal.write_to_pty(&input_bytes);
                            }
                        }
                    }
                }

                if let WindowEvent::MouseInput { state, button, .. } = &event {
                    if *state == winit::event::ElementState::Pressed
                        && *button == winit::event::MouseButton::Right
                    {
                        if let Some(ref mut terminal) = ui_state.terminal {
                            if !ui_state.close_confirm_open
                                && !ui_state.settings_state.open
                                && !ui_state.terminal_exited
                            {
                                if let Ok(mut cb) = arboard::Clipboard::new() {
                                    if ui_state.terminal_selection.has_selection() {
                                        if let Some(text) = terminal::selected_text_for_copy(
                                            terminal,
                                            &ui_state.terminal_selection,
                                        ) {
                                            if !text.is_empty() {
                                                let _ = cb.set_text(text);
                                            }
                                        }
                                        ui_state.terminal_selection.clear();
                                    } else if let Ok(text) = cb.get_text() {
                                        if !text.is_empty() {
                                            if terminal.is_bracketed_paste_enabled() {
                                                let mut bytes = Vec::with_capacity(text.len() + 12);
                                                bytes.extend_from_slice(b"\x1b[200~");
                                                bytes.extend_from_slice(text.as_bytes());
                                                bytes.extend_from_slice(b"\x1b[201~");
                                                terminal.write_to_pty(&bytes);
                                            } else {
                                                terminal.write_to_pty(text.as_bytes());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if let WindowEvent::Focused(focused) = &event {
                    if let Some(ref mut terminal) = ui_state.terminal {
                        if !ui_state.close_confirm_open
                            && !ui_state.settings_state.open
                            && !ui_state.terminal_exited
                            && terminal.is_focus_in_out_enabled()
                        {
                            let seq: &[u8] = if *focused { b"\x1b[I" } else { b"\x1b[O" };
                            terminal.write_to_pty(seq);
                        }
                    }
                }

                // While terminal input is active, keep keyboard/IME from reaching egui
                // to avoid focus-navigation activating window controls.
                let forward_to_egui = match &event {
                    WindowEvent::KeyboardInput { .. } | WindowEvent::Ime(_) => {
                        !terminal_input_active
                    }
                    _ => true,
                };
                if forward_to_egui {
                    let response = egui_state.on_window_event(window.as_ref(), &event);
                    let _ = response;
                }
                match event {
                    WindowEvent::CloseRequested => {
                        ui_state.close_confirm_open = true;
                        ui_state.close_focus_pending = true;
                        state.window().request_redraw();
                    }
                    WindowEvent::Resized(size) => state.resize(size),
                    WindowEvent::RedrawRequested => {
                        let loading_elapsed = ui_state.loading_started_at.elapsed().as_secs_f32();

                        if ui_state.reconnect_requested && terminal_init_rx.is_none() {
                            terminal_init_rx = Some(spawn_terminal_async(ui_state.startup_dir.clone()));
                            ui_state.reconnect_requested = false;
                            ui_state.terminal_connecting = true;
                            ui_state.terminal_init_error = None;
                        }

                        if let Some(rx) = terminal_init_rx.as_ref() {
                            match rx.try_recv() {
                                Ok(Ok(term)) => {
                                    eprintln!("Terminal started successfully");
                                    ui_state.pending_terminal = Some(term);
                                    ui_state.terminal_init_error = None;
                                    ui_state.terminal_connecting = false;
                                    terminal_init_rx = None;
                                }
                                Ok(Err(e)) => {
                                    eprintln!("Failed to start terminal: {}", e);
                                    ui_state.terminal_init_error = Some(e.to_string());
                                    ui_state.terminal_connecting = false;
                                    terminal_init_rx = None;
                                }
                                Err(mpsc::TryRecvError::Empty) => {}
                                Err(mpsc::TryRecvError::Disconnected) => {
                                    ui_state.terminal_init_error =
                                        Some("terminal init channel disconnected".to_string());
                                    ui_state.terminal_connecting = false;
                                    terminal_init_rx = None;
                                }
                            }
                        }

                        if let Some(term) = ui_state.pending_terminal.take() {
                            if ui_state.terminal.is_none()
                                && !startup_page::is_animation_done(loading_elapsed)
                            {
                                ui_state.pending_terminal = Some(term);
                            } else {
                                ui_state.terminal = Some(term);
                                ui_state.terminal_selection.clear();
                                ui_state.terminal_exited = false;
                                ui_state.terminal_scroll_request =
                                    Some(terminal::ScrollRequest::ScreenTop);
                                ui_state.terminal_scroll_request_frames_left = 30;
                                ui_state.terminal_scroll_id =
                                    ui_state.terminal_scroll_id.wrapping_add(1);
                            }
                        }

                        // Process PTY output before rendering
                        if let Some(ref mut terminal) = ui_state.terminal {
                            let process_result = terminal.process_input();
                            if process_result.had_input {
                                // Don't downgrade a ScreenTop request (e.g. from Ctrl+L) to
                                // CursorLine – the ScreenTop scroll must persist for its full
                                // frame budget so the viewport stays at the right position.
                                let has_screen_top = matches!(
                                    ui_state.terminal_scroll_request,
                                    Some(terminal::ScrollRequest::ScreenTop)
                                ) && ui_state.terminal_scroll_request_frames_left > 0;
                                if !has_screen_top {
                                    ui_state.terminal_scroll_request =
                                        Some(terminal::ScrollRequest::CursorLine);
                                    ui_state.terminal_scroll_request_frames_left = 1;
                                }
                            }
                            if process_result.pty_closed || !terminal.is_alive() {
                                ui_state.terminal_exited = true;
                                ui_state.terminal_connecting = false;
                            }
                        }

                        // Execute pending quick command (from UI click or keybinding)
                        if let Some((cmd_text, auto_exec)) = ui_state.pending_quick_cmd.take() {
                            if let Some(ref mut terminal) = ui_state.terminal {
                                if !ui_state.terminal_exited {
                                    terminal.write_to_pty(cmd_text.as_bytes());
                                    if auto_exec {
                                        terminal.write_to_pty(b"\r");
                                    }
                                    ui_state.terminal_scroll_request =
                                        Some(terminal::ScrollRequest::CursorLine);
                                    ui_state.terminal_scroll_request_frames_left = 1;
                                }
                            }
                        }

                        let raw_input = egui_state.take_egui_input(window.as_ref());
                        let mut ime_cursor_rect = None;
                        let full_output = egui_ctx.run(raw_input, |ctx| {
                            ime_cursor_rect = build_ui(ctx, &mut ui_state, window.as_ref());
                        });

                        if ui_state.close_confirmed {
                            elwt.exit();
                            return;
                        }

                        egui_state
                            .handle_platform_output(window.as_ref(), full_output.platform_output);
                        if let Some(rect) = ime_cursor_rect {
                            let ppp = full_output.pixels_per_point;
                            window.set_ime_cursor_area(
                                winit::dpi::PhysicalPosition::new(rect.min.x * ppp, rect.min.y * ppp),
                                winit::dpi::PhysicalSize::new(
                                    (rect.width() * ppp).max(1.0),
                                    (rect.height() * ppp).max(1.0),
                                ),
                            );
                        }

                        let paint_jobs =
                            egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
                        let screen_desc = ScreenDescriptor {
                            size_in_pixels: [state.config.width, state.config.height],
                            pixels_per_point: full_output.pixels_per_point,
                        };

                        for (id, image_delta) in &full_output.textures_delta.set {
                            egui_renderer.update_texture(
                                &state.device,
                                &state.queue,
                                *id,
                                image_delta,
                            );
                        }

                        match state.render_with_egui(&mut egui_renderer, &paint_jobs, &screen_desc)
                        {
                            Ok(()) => {}
                            Err(wgpu::SurfaceError::Lost) => state.resize(state.size),
                            Err(wgpu::SurfaceError::OutOfMemory) => elwt.exit(),
                            Err(_) => {}
                        }

                        for id in &full_output.textures_delta.free {
                            egui_renderer.free_texture(id);
                        }
                    }
                    _ => {}
                }
            }
            Event::AboutToWait => {
                // If the hidden window never gets a redraw while invisible on some platforms,
                // force-show it here so rendering can proceed.
                if !window_shown {
                    state.window().set_visible(true);
                    window_shown = true;
                }
                state.window().request_redraw();
            }
            _ => {}
        }
    });
}

fn resolve_startup_dir() -> PathBuf {
    let default_dir = PathBuf::from("C:\\");
    let arg_dir = std::env::args_os().nth(1).map(PathBuf::from);

    match arg_dir {
        Some(path) if path.is_dir() => path,
        _ => default_dir,
    }
}
