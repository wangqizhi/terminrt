use std::sync::Arc;
use egui_wgpu::ScreenDescriptor;
use wgpu::util::DeviceExt;
use winit::{
    dpi::PhysicalSize,
    event::{Event, WindowEvent},
    event_loop::EventLoop,
    window::WindowBuilder,
};

mod font;
mod pty;
mod terminal;

const WINDOW_WIDTH: u32 = 1024;
const WINDOW_HEIGHT: u32 = 768;
const SQUARE_SIZE: f32 = 200.0;
const FONT_SIZE: f32 = 120.0;
struct UiState {
    terminal: Option<terminal::TerminalInstance>,
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

        let uniform_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        let glyph_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        let color_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
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

        let glyph_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
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

        let glyph_vertices = [GlyphVertex { position: [0.0, 0.0], uv: [0.0, 0.0] }; 6];
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
        self.queue
            .write_buffer(&self.square_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
    }

    fn update_glyph_vertices(&mut self) {
        if let Some((w, h)) = self.glyph_dims {
            let vertices = make_glyph_vertices(self.size, w as f32, h as f32);
            self.queue
                .write_buffer(&self.glyph_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
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
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("render encoder") });

        egui_renderer.update_buffers(&self.device, &self.queue, &mut encoder, paint_jobs, screen_desc);

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
        ColorVertex { position: [x0, y0], color },
        ColorVertex { position: [x1, y0], color },
        ColorVertex { position: [x1, y1], color },
        ColorVertex { position: [x0, y0], color },
        ColorVertex { position: [x1, y1], color },
        ColorVertex { position: [x0, y1], color },
    ]
}

fn make_glyph_vertices(size: PhysicalSize<u32>, glyph_w: f32, glyph_h: f32) -> [GlyphVertex; 6] {
    let (square_x0, square_y0, square_x1, square_y1) = centered_rect(size, SQUARE_SIZE, SQUARE_SIZE);
    let square_cx = (square_x0 + square_x1) * 0.5;
    let square_cy = (square_y0 + square_y1) * 0.5;

    let x0 = square_cx - glyph_w * 0.5;
    let y0 = square_cy - glyph_h * 0.5;
    let x1 = square_cx + glyph_w * 0.5;
    let y1 = square_cy + glyph_h * 0.5;

    [
        GlyphVertex { position: [x0, y0], uv: [0.0, 0.0] },
        GlyphVertex { position: [x1, y0], uv: [1.0, 0.0] },
        GlyphVertex { position: [x1, y1], uv: [1.0, 1.0] },
        GlyphVertex { position: [x0, y0], uv: [0.0, 0.0] },
        GlyphVertex { position: [x1, y1], uv: [1.0, 1.0] },
        GlyphVertex { position: [x0, y1], uv: [0.0, 1.0] },
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

fn build_ui(ctx: &egui::Context, ui_state: &mut UiState) {
    let screen_rect = ctx.screen_rect();
    let total_w = screen_rect.width().max(1.0);
    let side_w = total_w * 0.25;

    let panel_stroke = egui::Stroke::new(1.0, egui::Color32::from_gray(70));
    let side_fill = egui::Color32::from_gray(18);
    let center_fill = egui::Color32::from_gray(20);

    egui::SidePanel::left("left_panel")
        .resizable(false)
        .exact_width(side_w)
        .frame(egui::Frame::none().fill(side_fill).stroke(panel_stroke))
        .show(ctx, |_ui| {});

    egui::SidePanel::right("right_panel")
        .resizable(false)
        .exact_width(side_w)
        .frame(egui::Frame::none().fill(side_fill).stroke(panel_stroke))
        .show(ctx, |_ui| {});

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(center_fill).stroke(panel_stroke))
        .show(ctx, |ui| {
            let origin = ui.min_rect().min;
            let available = ui.available_size();
            let bottom_h = 28.0; // Fixed height: just enough for status text
            let top_h = (available.y - bottom_h).max(0.0);

            let top_rect = egui::Rect::from_min_size(origin, egui::vec2(available.x, top_h));
            let bottom_rect = egui::Rect::from_min_size(
                egui::pos2(origin.x, origin.y + top_h),
                egui::vec2(available.x, bottom_h),
            );

            // Resize terminal to match available area
            if let Some(ref mut term) = ui_state.terminal {
                let margin = 4.0;
                let content_w = (top_rect.width() - margin * 2.0).max(0.0);
                let content_h = (top_rect.height() - margin * 2.0).max(0.0);
                let font_id = egui::FontId::monospace(terminal::TERM_FONT_SIZE);
                let row_height = ui.fonts(|f| f.row_height(&font_id));
                let char_width = ui.fonts(|f| f.glyph_width(&font_id, 'M'));
                if row_height > 0.0 && char_width > 0.0 {
                    let new_rows = (content_h / row_height).floor() as u16;
                    let new_cols = (content_w / char_width).floor() as u16;
                    if new_rows > 0 && new_cols > 0
                        && (new_rows as usize != term.rows() || new_cols as usize != term.cols())
                    {
                        term.resize(new_rows, new_cols);
                    }
                }
            }

            // Top area: terminal display
            ui.allocate_ui_at_rect(top_rect, |ui| {
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(18, 18, 18))
                    .stroke(panel_stroke)
                    .inner_margin(egui::Margin {
                        left: 4.0,
                        right: 4.0,
                        top: 4.0,
                        bottom: 4.0,
                    })
                    .show(ui, |ui| {
                        terminal::render_terminal(ui, ui_state.terminal.as_ref());
                    });
            });

            // Bottom area: status/info
            ui.allocate_ui_at_rect(bottom_rect, |ui| {
                egui::Frame::none()
                    .fill(egui::Color32::from_gray(24))
                    .inner_margin(egui::Margin {
                        left: 8.0,
                        right: 8.0,
                        top: 8.0,
                        bottom: 8.0,
                    })
                    .show(ui, |ui| {
                        let status = if ui_state.terminal.is_some() {
                            "Terminal: connected"
                        } else {
                            "Terminal: not connected"
                        };
                        ui.label(
                            egui::RichText::new(status)
                                .color(egui::Color32::from_gray(120))
                                .monospace()
                                .size(12.0),
                        );
                    });
            });
        });
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
    let event_loop = EventLoop::new().expect("event loop");
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("terminrt")
            .with_inner_size(PhysicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
            .build(&event_loop)
            .expect("create window"),
    );

    let mut state = pollster::block_on(State::new(window.clone()));
    let egui_ctx = egui::Context::default();
    if let Some(font_data) = load_system_chinese_font() {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert("zh".to_string(), egui::FontData::from_owned(font_data));
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .insert(0, "zh".to_string());
        fonts
            .families
            .get_mut(&egui::FontFamily::Monospace)
            .unwrap()
            .insert(0, "zh".to_string());
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

    // Initialize the terminal with default size (will be resized later)
    let terminal_instance = match terminal::TerminalInstance::new(24, 80) {
        Ok(t) => {
            eprintln!("Terminal started successfully");
            Some(t)
        }
        Err(e) => {
            eprintln!("Failed to start terminal: {}", e);
            None
        }
    };
    let mut ui_state = UiState {
        terminal: terminal_instance,
    };

    let mut current_modifiers = winit::event::Modifiers::default();

    let _ = event_loop.run(move |event, elwt| {
        match event {
            Event::WindowEvent { event, window_id } if window_id == state.window().id() => {
                // Track modifier state
                if let WindowEvent::ModifiersChanged(mods) = &event {
                    current_modifiers = mods.clone();
                }

                // Forward keyboard input to terminal BEFORE egui processes it
                if let WindowEvent::KeyboardInput { ref event, .. } = event {
                    if let Some(ref terminal) = ui_state.terminal {
                        if let Some(input_bytes) =
                            terminal::key_to_terminal_input(event, &current_modifiers)
                        {
                            terminal.write_to_pty(&input_bytes);
                        }
                    }
                }

                let response = egui_state.on_window_event(window.as_ref(), &event);
                let _ = response;
                match event {
                    WindowEvent::CloseRequested => elwt.exit(),
                    WindowEvent::Resized(size) => state.resize(size),
                    WindowEvent::RedrawRequested => {
                        // Process PTY output before rendering
                        if let Some(ref mut terminal) = ui_state.terminal {
                            terminal.process_input();
                        }

                        let raw_input = egui_state.take_egui_input(window.as_ref());
                        let full_output = egui_ctx.run(raw_input, |ctx| {
                            build_ui(ctx, &mut ui_state);
                        });
                        egui_state
                            .handle_platform_output(window.as_ref(), full_output.platform_output);

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

                        match state.render_with_egui(&mut egui_renderer, &paint_jobs, &screen_desc) {
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
                state.window().request_redraw();
            }
            _ => {}
        }
    });
}
