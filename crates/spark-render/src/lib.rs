//! Spark Engine - Rendering: wgpu backend (DX12 / Vulkan / Metal / OpenGL), custom WGSL shaders, screenshots.
//!
//! The renderer is a *view* of [`World`]: it uploads new assets automatically and draws the scene.
//! Games never touch GPU objects. Force a backend with the `WGPU_BACKEND` env var (`dx12`, `vulkan`, `gl`).
//!
//! Frame = scene pass at the internal resolution (surface shaders) -> 2D canvas "scene" layer -> post
//! passes (game shaders, or a plain upscale) -> 2D canvas "ui" layer -> window/screenshot.
//! The engine has no looks of its own; see `spark_core::shader`.

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use spark_core::shader::{COPY_POST, MAX_PARAMS_SIZE};
use spark_core::{
    Aabb, Assets, Batch, Canvas, CanvasTexture, Color, ImageData, Layer, Mat4, MeshId, PassSize, ShaderData, ShaderKind,
    TextureFilter, TextureId, Vec3, Vertex, Vertex2D, World,
};
use wgpu::util::DeviceExt;

pub use wgpu;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
const CANVAS_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
/// Glyph atlas: coverage in alpha, not sRGB-encoded.
const GLYPH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Matches `struct Frame` in spark-core/src/shaders/common.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniform {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    camera: [f32; 4],
    screen: [f32; 4],
    time: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
    fog_color: [f32; 4],
    fog: [f32; 4],
    light_count: [u32; 4],
    lights: [LightUniform; MAX_LIGHTS],
}

const MAX_LIGHTS: usize = spark_core::Light::MAX_VISIBLE;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightUniform {
    /// xyz = world position, w = range.
    position: [f32; 4],
    /// rgb = linear color * intensity, w = 1 for spot.
    color: [f32; 4],
    /// xyz = direction, w = cos(outer half angle).
    direction: [f32; 4],
    /// x = cos(inner half angle).
    params: [f32; 4],
}

/// Matches `struct Object` in spark-core/src/shaders/surface.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ObjectUniform {
    model: [[f32; 4]; 4],
    normal_matrix: [[f32; 4]; 4],
    color: [f32; 4],
    data: [f32; 4],
    /// tiling x, tiling y, unlit, -
    info: [f32; 4],
}

/// Matches `struct PassInfo` in spark-core/src/shaders/post.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PassUniform {
    input_size: [f32; 4],
    output_size: [f32; 4],
}

/// GPU / driver info.
#[derive(Clone, Debug)]
pub struct GpuInfo {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
}

impl std::fmt::Display for GpuInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}, {}, driver: {})", self.name, self.backend, self.device_type, self.driver)
    }
}

/// Numbers about the last rendered frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub draw_calls: u32,
    /// Objects skipped because they were outside the camera view.
    pub culled: u32,
    pub triangles: u64,
    pub internal_size: (u32, u32),
    /// Post passes run (0 = plain upscale).
    pub post_passes: u32,
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    vertex_count: usize,
    /// `Assets::mesh_version` when uploaded.
    version: u64,
}

struct GpuTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    nearest: wgpu::BindGroup,
    linear: wgpu::BindGroup,
    nearest_clamp: wgpu::BindGroup,
    linear_clamp: wgpu::BindGroup,
}

/// A shader on the GPU: module, its `Params` buffer + resources, pipelines per output format.
struct GpuShader {
    name: String,
    kind: ShaderKind,
    version: u64,
    /// `None` when the GPU rejected it (logged once; the default shader is used instead).
    module: Option<wgpu::ShaderModule>,
    pipelines: HashMap<wgpu::TextureFormat, Option<wgpu::RenderPipeline>>,
    params: wgpu::Buffer,
    resources: wgpu::BindGroup,
    textures: [Option<TextureId>; 2],
}

/// Which shader draws something.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum ShaderRef {
    Default,
    Copy,
    Asset(usize),
}

struct SceneTarget {
    size: (u32, u32),
    _color: wgpu::Texture,
    _depth: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

/// Intermediate image of the post chain.
struct PostTarget {
    size: (u32, u32),
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct Capture {
    size: (u32, u32),
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct SurfaceState {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    view_format: wgpu::TextureFormat,
}

type SurfaceFactory = Box<dyn Fn(&wgpu::Instance) -> Option<wgpu::Surface<'static>>>;

struct Draw {
    shader: ShaderRef,
    mesh: usize,
    texture: Option<usize>,
    slot: u32,
}

/// Draws a [`World`] to a window or to an image.
pub struct Renderer {
    instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: GpuInfo,
    surface: Option<SurfaceState>,
    surface_factory: Option<SurfaceFactory>,
    width: u32,
    height: u32,

    frame_buffer: wgpu::Buffer,
    frame_bind_group: wgpu::BindGroup,
    object_layout: wgpu::BindGroupLayout,
    object_buffer: wgpu::Buffer,
    object_bind_group: wgpu::BindGroup,
    object_capacity: usize,
    object_stride: u64,
    texture_layout: wgpu::BindGroupLayout,
    /// nearest / linear with Repeat (meshes), then nearest / linear with ClampToEdge (2D canvas, post input).
    texture_samplers: [wgpu::Sampler; 4],
    /// Group 3 (surface) / group 1 (post): params, texture1, texture2, linear + nearest samplers.
    shader_layout: wgpu::BindGroupLayout,
    mesh_pipeline_layout: wgpu::PipelineLayout,
    post_layout: wgpu::BindGroupLayout,
    post_pipeline_layout: wgpu::PipelineLayout,
    default_surface: GpuShader,
    copy_post: GpuShader,
    shaders: Vec<GpuShader>,
    post_targets: Vec<PostTarget>,
    pass_buffers: Vec<wgpu::Buffer>,

    canvas_buffer: wgpu::Buffer,
    canvas_bind_group: wgpu::BindGroup,
    canvas_pipeline_layout: wgpu::PipelineLayout,
    canvas_shader: wgpu::ShaderModule,
    canvas_pipelines: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    canvas_vertices: wgpu::Buffer,
    /// First vertex of the "ui" layer in `canvas_vertices` (the "scene" layer comes first).
    canvas_ui_base: u32,
    glyphs: Option<GpuTexture>,
    glyph_version: u64,

    scene_target: Option<SceneTarget>,
    capture: Option<Capture>,
    meshes: Vec<Option<GpuMesh>>,
    textures: Vec<GpuTexture>,
    white: GpuTexture,
    assets_generation: u64,
    /// `Assets::mesh_edits` already applied.
    mesh_edits: u64,
    draws: Vec<Draw>,
    scratch: Vec<u8>,
}

fn sanitize_backend_env() {
    if std::env::var_os("WGPU_BACKEND").is_some_and(|v| v.to_string_lossy().trim().is_empty()) {
        // SAFETY: called before the GPU instance (and any of its threads) exists.
        unsafe { std::env::remove_var("WGPU_BACKEND") };
    }
}

impl Renderer {
    /// Renderer that presents to a window.
    pub fn new_windowed<W>(window: Arc<W>, width: u32, height: u32, vsync: bool) -> Result<Self, String>
    where
        W: wgpu::rwh::HasWindowHandle + wgpu::rwh::HasDisplayHandle + std::fmt::Debug + Send + Sync + 'static,
    {
        sanitize_backend_env();
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(window.clone())));
        let surface = instance.create_surface(window.clone()).map_err(|e| format!("cannot create window surface: {e}"))?;
        let adapter = request_adapter(&instance, Some(&surface))?;
        let (device, queue) = request_device(&adapter)?;

        let caps = surface.get_capabilities(&adapter);
        let Some(&first) = caps.formats.first() else {
            return Err("the GPU cannot present to this window".into());
        };
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(first);
        let view_format = format.add_srgb_suffix();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: if view_format != format { vec![view_format] } else { vec![] },
            alpha_mode: caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto),
            width: width.max(1),
            height: height.max(1),
            desired_maximum_frame_latency: 2,
            present_mode: if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync },
        };
        surface.configure(&device, &config);

        let mut renderer = Self::build(instance, &adapter, device, queue, width, height);
        renderer.surface = Some(SurfaceState { surface, config, view_format });
        renderer.surface_factory = Some(Box::new(move |instance| instance.create_surface(window.clone()).ok()));
        Ok(renderer)
    }

    /// Renderer without a window (tests, CI, AI screenshots).
    pub fn new_headless(width: u32, height: u32) -> Result<Self, String> {
        sanitize_backend_env();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = request_adapter(&instance, None)?;
        let (device, queue) = request_device(&adapter)?;
        Ok(Self::build(instance, &adapter, device, queue, width, height))
    }

    pub fn info(&self) -> &GpuInfo {
        &self.info
    }

    /// Output size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.width = width;
        self.height = height;
        if let Some(s) = &mut self.surface {
            s.config.width = width;
            s.config.height = height;
            s.surface.configure(&self.device, &s.config);
        }
    }

    /// Draws the world to the window.
    pub fn render(&mut self, world: &World) -> Result<FrameStats, String> {
        let Some(state) = &self.surface else {
            return Err("render(): this renderer is headless, use screenshot()".into());
        };
        let view_format = state.view_format;
        let frame = match state.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                drop(t);
                self.reconfigure();
                return Ok(FrameStats::default());
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.reconfigure();
                return Ok(FrameStats::default());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.recreate_surface();
                return Ok(FrameStats::default());
            }
            wgpu::CurrentSurfaceTexture::Validation => return Err("window surface validation error".into()),
            #[allow(unreachable_patterns)]
            _ => return Ok(FrameStats::default()),
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(view_format), ..Default::default() });
        self.ensure_canvas_pipeline(view_format);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("spark frame") });
        let mut stats = self.draw_scene(&mut encoder, world);
        stats.post_passes = self.draw_post(&mut encoder, &view, view_format, world);
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        Ok(stats)
    }

    /// Draws the world and returns the final image (same as what the window shows).
    pub fn screenshot(&mut self, world: &World) -> Result<ImageData, String> {
        let size = (self.width, self.height);
        if self.capture.as_ref().is_none_or(|c| c.size != size) {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("spark capture"),
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: COLOR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.capture = Some(Capture { size, texture, view });
        }
        self.ensure_canvas_pipeline(COLOR_FORMAT);

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("spark screenshot") });
        self.draw_scene(&mut encoder, world);
        let capture = self.capture.take().expect("capture target");
        self.draw_post(&mut encoder, &capture.view, COLOR_FORMAT, world);
        let capture = self.capture.insert(capture);

        let unpadded = size.0 * 4;
        let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spark readback"),
            size: padded as u64 * size.1 as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &capture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(size.1) },
            },
            extent(size),
        );
        self.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("GPU wait failed: {e}"))?;
        let data = slice.get_mapped_range().map_err(|e| format!("cannot read screenshot: {e:?}"))?;
        let mut pixels = Vec::with_capacity((unpadded * size.1) as usize);
        for row in 0..size.1 as usize {
            let start = row * padded as usize;
            pixels.extend_from_slice(&data[start..start + unpadded as usize]);
        }
        drop(data);
        buffer.unmap();
        Ok(ImageData::new(size.0, size.1, pixels))
    }

    // ------------------------------------------------------------------------------------------


    // ------------------------------------------------------------------------------------------

    fn build(
        instance: wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Self {
        let ai = adapter.get_info();
        let info = GpuInfo {
            name: ai.name.clone(),
            backend: format!("{:?}", ai.backend),
            device_type: format!("{:?}", ai.device_type),
            driver: format!("{} {}", ai.driver, ai.driver_info).trim().to_string(),
        };

        let frame_size = size_of::<FrameUniform>() as u64;
        let object_size = size_of::<ObjectUniform>() as u64;
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fs = wgpu::ShaderStages::FRAGMENT;

        let frame_layout = bind_group_layout(&device, "spark frame", &[uniform_entry(0, false, frame_size, vf)]);
        let object_layout = bind_group_layout(&device, "spark object", &[uniform_entry(0, true, object_size, vf)]);
        let texture_layout = bind_group_layout(&device, "spark texture", &[texture_entry(0, fs), sampler_entry(1, fs)]);
        let shader_layout = bind_group_layout(
            &device,
            "spark shader resources",
            &[uniform_entry(0, false, 0, vf), texture_entry(1, vf), texture_entry(2, vf), sampler_entry(3, vf), sampler_entry(4, vf)],
        );
        let post_layout = bind_group_layout(
            &device,
            "spark post",
            &[
                uniform_entry(0, false, frame_size, vf),
                texture_entry(1, fs),
                sampler_entry(2, fs),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: fs,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                uniform_entry(4, false, size_of::<PassUniform>() as u64, fs),
            ],
        );

        let frame_buffer = uniform_buffer(&device, "spark frame", frame_size);
        let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spark frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: frame_buffer.as_entire_binding() }],
        });

        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let object_stride = object_size.div_ceil(align) * align;
        let object_capacity = 256;
        let (object_buffer, object_bind_group) =
            object_buffer(&device, &object_layout, object_capacity, object_stride, object_size);

        let sampler = |filter: wgpu::FilterMode, address: wgpu::AddressMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("spark sampler"),
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            })
        };
        use wgpu::{AddressMode, FilterMode};
        let texture_samplers = [
            sampler(FilterMode::Nearest, AddressMode::Repeat),
            sampler(FilterMode::Linear, AddressMode::Repeat),
            sampler(FilterMode::Nearest, AddressMode::ClampToEdge),
            sampler(FilterMode::Linear, AddressMode::ClampToEdge),
        ];

        let mesh_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spark mesh"),
            bind_group_layouts: &[Some(&frame_layout), Some(&object_layout), Some(&texture_layout), Some(&shader_layout)],
            immediate_size: 0,
        });
        let post_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spark post"),
            bind_group_layouts: &[Some(&post_layout), Some(&shader_layout)],
            immediate_size: 0,
        });
        let white =
            upload_texture(&device, &queue, &texture_layout, &texture_samplers, &ImageData::solid(Color::WHITE), COLOR_FORMAT, false);

        let builtin = |name: &str, code: &str| {
            let data = ShaderData::compile(name, code).expect("built-in shader compiles");
            gpu_shader(&device, &shader_layout, &texture_samplers, &white, &[], &data)
        };
        let default_surface = builtin("spark default surface", "");
        let copy_post = builtin("spark upscale", COPY_POST);

        let canvas_layout = bind_group_layout(&device, "spark canvas", &[uniform_entry(0, false, 16, wgpu::ShaderStages::VERTEX)]);
        let canvas_buffer = uniform_buffer(&device, "spark canvas", 16);
        let canvas_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spark canvas"),
            layout: &canvas_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: canvas_buffer.as_entire_binding() }],
        });
        let canvas_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spark canvas"),
            bind_group_layouts: &[Some(&canvas_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let canvas_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spark canvas shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/canvas.wgsl").into()),
        });
        let mut canvas_pipelines = HashMap::new();
        canvas_pipelines
            .insert(COLOR_FORMAT, canvas_pipeline(&device, &canvas_pipeline_layout, &canvas_shader, COLOR_FORMAT));
        let canvas_vertices = vertex_buffer(&device, 64 * 1024);

        Self {
            instance,
            device,
            queue,
            info,
            surface: None,
            surface_factory: None,
            width: width.max(1),
            height: height.max(1),
            frame_buffer,
            frame_bind_group,
            object_layout,
            object_buffer,
            object_bind_group,
            object_capacity,
            object_stride,
            texture_layout,
            texture_samplers,
            shader_layout,
            mesh_pipeline_layout,
            post_layout,
            post_pipeline_layout,
            default_surface,
            copy_post,
            shaders: Vec::new(),
            post_targets: Vec::new(),
            pass_buffers: Vec::new(),
            canvas_buffer,
            canvas_bind_group,
            canvas_pipeline_layout,
            canvas_shader,
            canvas_pipelines,
            canvas_vertices,
            canvas_ui_base: 0,
            glyphs: None,
            glyph_version: 0,
            scene_target: None,
            capture: None,
            meshes: Vec::new(),
            textures: Vec::new(),
            white,
            assets_generation: u64::MAX,
            mesh_edits: 0,
            draws: Vec::new(),
            scratch: Vec::new(),
        }
    }

    fn reconfigure(&mut self) {
        if let Some(s) = &self.surface {
            s.surface.configure(&self.device, &s.config);
        }
    }

    fn recreate_surface(&mut self) {
        if let (Some(factory), Some(state)) = (&self.surface_factory, &mut self.surface) {
            match factory(&self.instance) {
                Some(surface) => {
                    state.surface = surface;
                    state.surface.configure(&self.device, &state.config);
                }
                None => log::error!("window surface lost and could not be recreated"),
            }
        }
    }

    /// Uploads meshes/textures added to `assets` since the last frame.

    fn sync_assets(&mut self, assets: &Assets) {
        if assets.generation() != self.assets_generation {
            self.meshes.clear();
            self.textures.clear();
            self.shaders.clear();
            self.assets_generation = assets.generation();
        }
        while self.meshes.len() < assets.mesh_count() {
            let data = assets.mesh(MeshId(self.meshes.len() as u32)).expect("mesh id in range");
            let gpu = (!data.vertices.is_empty() && !data.indices.is_empty()).then(|| GpuMesh {
                vertices: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("spark vertices"),
                    contents: bytemuck::cast_slice(&data.vertices),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                }),
                indices: self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("spark indices"),
                    contents: bytemuck::cast_slice(&data.indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                index_count: data.indices.len() as u32,
                vertex_count: data.vertices.len(),
                version: assets.mesh_version(MeshId(self.meshes.len() as u32)),
            });
            self.meshes.push(gpu);
        }
        // Meshes edited in place (skinned models): rewrite their vertex buffers.
        if assets.mesh_edits() != self.mesh_edits {
            self.mesh_edits = assets.mesh_edits();
            for (i, slot) in self.meshes.iter_mut().enumerate() {
                let Some(gpu) = slot else { continue };
                let version = assets.mesh_version(MeshId(i as u32));
                if gpu.version == version {
                    continue;
                }
                gpu.version = version;
                if let Some(data) = assets.mesh(MeshId(i as u32)).filter(|d| d.vertices.len() == gpu.vertex_count) {
                    self.queue.write_buffer(&gpu.vertices, 0, bytemuck::cast_slice(&data.vertices));
                }
            }
        }
        let max = self.device.limits().max_texture_dimension_2d;
        while self.textures.len() < assets.texture_count() {
            let image = assets.texture(TextureId(self.textures.len() as u32)).expect("texture id in range");
            let gpu = if image.width == 0 || image.height == 0 || image.width > max || image.height > max {
                log::error!("texture {}x{} is not supported (max {max}x{max}); using magenta", image.width, image.height);
                let magenta = ImageData::solid(Color::hex(0xff00ff));
                upload_texture(&self.device, &self.queue, &self.texture_layout, &self.texture_samplers, &magenta, COLOR_FORMAT, false)
            } else {
                upload_texture(&self.device, &self.queue, &self.texture_layout, &self.texture_samplers, image, COLOR_FORMAT, true)
            };
            self.textures.push(gpu);
        }
    }


    fn ensure_scene_target(&mut self, size: (u32, u32)) {
        let max = self.device.limits().max_texture_dimension_2d;
        let size = (size.0.clamp(1, max), size.1.clamp(1, max));
        if self.scene_target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let make = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = make(
            "spark scene color",
            COLOR_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth = make(
            "spark scene depth",
            DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        self.scene_target = Some(SceneTarget {
            size,
            color_view: color.create_view(&Default::default()),
            depth_view: depth.create_view(&Default::default()),
            _color: color,
            _depth: depth,
        });
    }

    /// Uploads the 2D vertices of both layers, the screen transform and the glyph atlas.

    fn prepare_canvas(&mut self, canvas: &Canvas) {
        let scene = canvas.layer_data(Layer::Scene);
        let ui = canvas.layer_data(Layer::Ui);
        self.canvas_ui_base = scene.vertices.len() as u32;
        if canvas.is_empty() {
            return;
        }
        let size = canvas.size();
        let uniform: [f32; 4] = [2.0 / size.x, -2.0 / size.y, -1.0, 1.0];
        self.queue.write_buffer(&self.canvas_buffer, 0, bytemuck::bytes_of(&uniform));

        let stride = size_of::<Vertex2D>() as u64;
        let scene_bytes = scene.vertices.len() as u64 * stride;
        let total = scene_bytes + ui.vertices.len() as u64 * stride;
        if total > self.canvas_vertices.size() {
            self.canvas_vertices = vertex_buffer(&self.device, total.next_power_of_two());
        }
        if !scene.vertices.is_empty() {
            self.queue.write_buffer(&self.canvas_vertices, 0, bytemuck::cast_slice(&scene.vertices));
        }
        if !ui.vertices.is_empty() {
            self.queue.write_buffer(&self.canvas_vertices, scene_bytes, bytemuck::cast_slice(&ui.vertices));
        }

        let fonts = &canvas.fonts;
        if fonts.version() != self.glyph_version {
            let atlas = fonts.atlas();
            match &self.glyphs {
                Some(g) if g.texture.width() == atlas.width && g.texture.height() == atlas.height => {
                    self.queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &g.texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        &atlas.pixels,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(atlas.width * 4),
                            rows_per_image: Some(atlas.height),
                        },
                        extent((atlas.width, atlas.height)),
                    );
                }
                _ => {
                    self.glyphs =
                        Some(upload_texture(&self.device, &self.queue, &self.texture_layout, &self.texture_samplers, atlas, GLYPH_FORMAT, false));
                }
            }
            self.glyph_version = fonts.version();
        }
    }

    /// Draws 2D batches into an open pass. `base` = offset of the layer in `canvas_vertices`.
    fn draw_canvas_batches(&self, pass: &mut wgpu::RenderPass<'_>, format: wgpu::TextureFormat, batches: &[Batch], base: u32) {
        if batches.is_empty() {
            return;
        }
        pass.set_pipeline(&self.canvas_pipelines[&format]);
        pass.set_bind_group(0, &self.canvas_bind_group, &[]);
        pass.set_vertex_buffer(0, self.canvas_vertices.slice(..));
        for b in batches {
            let tex = match b.texture {
                CanvasTexture::White => &self.white,
                CanvasTexture::Texture(id) => self.textures.get(id.0 as usize).unwrap_or(&self.white),
                CanvasTexture::Glyphs => self.glyphs.as_ref().unwrap_or(&self.white),
            };
            pass.set_bind_group(1, if b.nearest { &tex.nearest_clamp } else { &tex.linear_clamp }, &[]);
            pass.draw(base + b.start..base + b.start + b.count, 0..1);
        }
    }


    fn ensure_canvas_pipeline(&mut self, format: wgpu::TextureFormat) {
        if !self.canvas_pipelines.contains_key(&format) {
            let p = canvas_pipeline(&self.device, &self.canvas_pipeline_layout, &self.canvas_shader, format);
            self.canvas_pipelines.insert(format, p);
        }
    }

    /// GPU copies of the game's shaders: (re)built when added / hot-reloaded, params uploaded every frame.
    fn sync_shaders(&mut self, assets: &Assets) {
        for i in 0..assets.shader_count() {
            let data = assets.shader(spark_core::ShaderId(i as u32)).expect("shader id in range");
            if self.shaders.get(i).is_none_or(|s| s.version != data.version) {
                let g = gpu_shader(&self.device, &self.shader_layout, &self.texture_samplers, &self.white, &self.textures, data);
                if i < self.shaders.len() {
                    self.shaders[i] = g;
                } else {
                    self.shaders.push(g);
                }
            } else if self.shaders[i].textures != data.textures {
                let s = &mut self.shaders[i];
                s.resources = shader_resources(&self.device, &self.shader_layout, &self.texture_samplers, &self.white, &self.textures, &s.params, data.textures);
                s.textures = data.textures;
            }
            self.queue.write_buffer(&self.shaders[i].params, 0, &data.values);
        }
    }

    fn shader(&self, r: ShaderRef) -> &GpuShader {
        match r {
            ShaderRef::Default => &self.default_surface,
            ShaderRef::Copy => &self.copy_post,
            ShaderRef::Asset(i) => &self.shaders[i],
        }
    }

    /// Asset shader `id` if it exists and is of `kind`.
    fn asset_shader(&self, id: Option<spark_core::ShaderId>, kind: ShaderKind) -> Option<ShaderRef> {
        let i = id?.0 as usize;
        (self.shaders.get(i)?.kind == kind).then_some(ShaderRef::Asset(i))
    }

    /// Builds the pipeline of `r` for `format` if needed. `false` = the shader cannot be used.
    fn ensure_shader_pipeline(&mut self, r: ShaderRef, format: wgpu::TextureFormat) -> bool {
        let s = match r {
            ShaderRef::Default => &mut self.default_surface,
            ShaderRef::Copy => &mut self.copy_post,
            ShaderRef::Asset(i) => &mut self.shaders[i],
        };
        let layout = match s.kind {
            ShaderKind::Surface => &self.mesh_pipeline_layout,
            ShaderKind::Post => &self.post_pipeline_layout,
        };
        ensure_pipeline(&self.device, layout, s, format);
        s.pipelines.get(&format).is_some_and(|p| p.is_some())
    }

    fn pipeline(&self, r: ShaderRef, format: wgpu::TextureFormat) -> &wgpu::RenderPipeline {
        self.shader(r).pipelines[&format].as_ref().expect("pipeline ensured")
    }

    fn draw_scene(&mut self, encoder: &mut wgpu::CommandEncoder, world: &World) -> FrameStats {
        self.sync_assets(&world.assets);
        self.sync_shaders(&world.assets);
        let rs = &world.render;
        self.ensure_scene_target(rs.internal_size(self.width, self.height));
        let internal = self.scene_target.as_ref().expect("scene target").size;

        let scene = &world.scene;
        let cam = &scene.camera;
        let aspect = self.width as f32 / self.height as f32;
        let (view, proj) = (cam.view(), cam.projection(aspect));
        let view_proj = proj * view;
        let sun = scene.sun.color.to_linear();
        let si = scene.sun.intensity;
        let (fog_color, fog) = match scene.fog {
            Some(f) => (f.color.to_linear(), [f.near, f.far, 1.0, 0.0]),
            None => ([0.0; 4], [0.0; 4]),
        };
        let (lights, light_count) = collect_lights(scene);
        let t = &world.time;
        let frame = FrameUniform {
            view_proj: view_proj.to_cols_array_2d(),
            view: view.to_cols_array_2d(),
            proj: proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            camera_pos: cam.position.extend(1.0).to_array(),
            camera: [cam.near.max(0.0001), cam.far, cam.fov.clamp(1.0, 179.0).to_radians(), aspect],
            screen: [internal.0 as f32, internal.1 as f32, self.width as f32, self.height as f32],
            time: [t.elapsed as f32, t.dt, t.unscaled_elapsed as f32, t.frame as f32],
            sun_dir: scene.sun.direction.normalize_or(Vec3::NEG_Y).extend(0.0).to_array(),
            sun_color: [sun[0] * si, sun[1] * si, sun[2] * si, 1.0],
            ambient: scene.ambient.to_linear(),
            fog_color,
            fog,
            light_count: [light_count as u32, 0, 0, 0],
            lights,
        };
        self.queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&frame));

        // Collect visible objects into the per-object uniform buffer.
        let mut draws = std::mem::take(&mut self.draws);
        let mut scratch = std::mem::take(&mut self.scratch);
        draws.clear();
        scratch.clear();
        let mut stats = FrameStats { internal_size: internal, ..Default::default() };
        let fallback = self.asset_shader(rs.shader, ShaderKind::Surface).unwrap_or(ShaderRef::Default);
        for (id, obj) in scene.iter() {
            let Some(mesh) = obj.mesh else { continue };
            let mesh = mesh.0 as usize;
            let Some(Some(gpu_mesh)) = self.meshes.get(mesh) else { continue };
            if !scene.is_visible(id) {
                continue;
            }
            let model = scene.world_matrix(id);
            if outside_frustum(&(view_proj * model), &obj.local_bounds()) {
                stats.culled += 1;
                continue;
            }
            let m = &obj.material;
            let uniform = ObjectUniform {
                model: model.to_cols_array_2d(),
                normal_matrix: normal_matrix(&model).to_cols_array_2d(),
                color: m.color.to_linear(),
                data: m.data,
                info: [m.tiling.x, m.tiling.y, flag(m.unlit), 0.0],
            };
            let slot = draws.len() as u32;
            scratch.resize((slot as u64 * self.object_stride) as usize, 0);
            scratch.extend_from_slice(bytemuck::bytes_of(&uniform));
            stats.triangles += (gpu_mesh.index_count / 3) as u64;
            let texture = m.texture.map(|t| t.0 as usize).filter(|&t| t < self.textures.len());
            let shader = self.asset_shader(m.shader, ShaderKind::Surface).unwrap_or(fallback);
            draws.push(Draw { shader, mesh, texture, slot });
        }
        stats.draw_calls = draws.len() as u32;

        if draws.len() > self.object_capacity {
            self.object_capacity = draws.len().next_power_of_two();
            let (buffer, bind_group) = object_buffer(
                &self.device,
                &self.object_layout,
                self.object_capacity,
                self.object_stride,
                size_of::<ObjectUniform>() as u64,
            );
            self.object_buffer = buffer;
            self.object_bind_group = bind_group;
        }
        if !scratch.is_empty() {
            self.queue.write_buffer(&self.object_buffer, 0, &scratch);
        }
        // Pipelines for every shader in use; broken ones fall back to the default.
        self.ensure_shader_pipeline(ShaderRef::Default, COLOR_FORMAT);
        let mut usable: HashMap<ShaderRef, bool> = HashMap::new();
        for d in &mut draws {
            let ok = match usable.get(&d.shader) {
                Some(&ok) => ok,
                None => {
                    let ok = self.ensure_shader_pipeline(d.shader, COLOR_FORMAT);
                    usable.insert(d.shader, ok);
                    ok
                }
            };
            if !ok {
                d.shader = ShaderRef::Default;
            }
        }
        draws.sort_by_key(|d| (d.shader, d.texture, d.mesh));

        let target = self.scene_target.as_ref().expect("scene target");
        let bg = scene.background.to_linear();
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spark scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: bg[0] as f64, g: bg[1] as f64, b: bg[2] as f64, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            let nearest = rs.filter == TextureFilter::Nearest;
            let (mut last_shader, mut last_mesh, mut last_texture) = (None, usize::MAX, None::<Option<usize>>);
            for d in &draws {
                if last_shader != Some(d.shader) {
                    pass.set_pipeline(self.pipeline(d.shader, COLOR_FORMAT));
                    pass.set_bind_group(3, &self.shader(d.shader).resources, &[]);
                    last_shader = Some(d.shader);
                }
                let mesh = self.meshes[d.mesh].as_ref().expect("uploaded mesh");
                pass.set_bind_group(1, &self.object_bind_group, &[(d.slot as u64 * self.object_stride) as u32]);
                if last_texture != Some(d.texture) {
                    let tex = d.texture.map(|t| &self.textures[t]).unwrap_or(&self.white);
                    pass.set_bind_group(2, if nearest { &tex.nearest } else { &tex.linear }, &[]);
                    last_texture = Some(d.texture);
                }
                if last_mesh != d.mesh {
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    last_mesh = d.mesh;
                }
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
        }
        self.draws = draws;
        self.scratch = scratch;

        // 2D "scene" layer: drawn into the 3D image, so it goes through the post passes.
        self.prepare_canvas(&world.canvas);
        let scene_layer = world.canvas.layer_data(Layer::Scene);
        if !scene_layer.is_empty() {
            let target = self.scene_target.as_ref().expect("scene target");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spark canvas scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.draw_canvas_batches(&mut pass, COLOR_FORMAT, &scene_layer.batches, 0);
        }
        stats
    }

    /// Post chain: the game's passes in order, ending at the output (a plain upscale is added when
    /// needed), then the 2D "ui" layer. Returns the number of game passes run.
    fn draw_post(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        world: &World,
    ) -> u32 {
        let output = (self.width, self.height);
        let scene_size = self.scene_target.as_ref().expect("scene target").size;
        let mut chain: Vec<(ShaderRef, (u32, u32))> = Vec::new();
        for p in &world.render.post {
            let Some(r) = self.asset_shader(Some(p.shader), ShaderKind::Post) else { continue };
            chain.push((r, if p.size == PassSize::Scene { scene_size } else { output }));
        }
        if chain.last().is_none_or(|l| l.1 != output) {
            chain.push((ShaderRef::Copy, output));
        }
        // Pipelines: intermediate passes render to COLOR_FORMAT, the last one to `format`.
        let n = chain.len();
        let mut keep = Vec::with_capacity(n);
        for (i, (r, _)) in chain.iter().enumerate() {
            let f = if i + 1 == n { format } else { COLOR_FORMAT };
            keep.push(self.ensure_shader_pipeline(*r, f));
        }
        let mut chain: Vec<_> = chain.into_iter().zip(keep).filter(|(_, ok)| *ok).map(|(c, _)| c).collect();
        if chain.last().is_none_or(|l| l.1 != output || !self.shader(l.0).pipelines.get(&format).is_some_and(|p| p.is_some())) {
            self.ensure_shader_pipeline(ShaderRef::Copy, format);
            chain.push((ShaderRef::Copy, output));
        }
        let n = chain.len();
        let game_passes = chain.iter().filter(|c| c.0 != ShaderRef::Copy).count() as u32;

        // Intermediate images and per-pass info buffers.
        let max = self.device.limits().max_texture_dimension_2d;
        for i in 0..n - 1 {
            let size = (chain[i].1.0.clamp(1, max), chain[i].1.1.clamp(1, max));
            if self.post_targets.get(i).is_none_or(|t| t.size != size) {
                let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("spark post target"),
                    size: extent(size),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: COLOR_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let t = PostTarget { size, view: texture.create_view(&Default::default()), _texture: texture };
                if i < self.post_targets.len() {
                    self.post_targets[i] = t;
                } else {
                    self.post_targets.push(t);
                }
            }
        }
        while self.pass_buffers.len() < n {
            self.pass_buffers.push(uniform_buffer(&self.device, "spark pass info", size_of::<PassUniform>() as u64));
        }
        let sampler = match world.render.upscale {
            TextureFilter::Nearest => &self.texture_samplers[2],
            TextureFilter::Linear => &self.texture_samplers[3],
        };
        let target = self.scene_target.as_ref().expect("scene target");
        let mut input_size = scene_size;
        for (i, &(r, size)) in chain.iter().enumerate() {
            let last = i + 1 == n;
            let info = PassUniform {
                input_size: [input_size.0 as f32, input_size.1 as f32, 0.0, 0.0],
                output_size: [size.0 as f32, size.1 as f32, 0.0, 0.0],
            };
            self.queue.write_buffer(&self.pass_buffers[i], 0, bytemuck::bytes_of(&info));
            let input = if i == 0 { &target.color_view } else { &self.post_targets[i - 1].view };
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("spark post"),
                layout: &self.post_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.frame_buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(input) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&target.depth_view) },
                    wgpu::BindGroupEntry { binding: 4, resource: self.pass_buffers[i].as_entire_binding() },
                ],
            });
            let out = if last { view } else { &self.post_targets[i].view };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spark post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: out,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(self.pipeline(r, if last { format } else { COLOR_FORMAT }));
            pass.set_bind_group(0, &bind_group, &[]);
            pass.set_bind_group(1, &self.shader(r).resources, &[]);
            pass.draw(0..3, 0..1);
            if last {
                // 2D "ui" layer: crisp, at full output resolution.
                self.draw_canvas_batches(&mut pass, format, &world.canvas.layer_data(Layer::Ui).batches, self.canvas_ui_base);
            }
            input_size = size;
        }
        game_passes
    }
}

// ---- helpers ------------------------------------------------------------------------------------

/// Uploads a shader to the GPU (its module is validated by the GPU here; a rejected shader logs an error).
fn gpu_shader(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    samplers: &[wgpu::Sampler; 4],
    white: &GpuTexture,
    textures: &[GpuTexture],
    data: &ShaderData,
) -> GpuShader {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(&data.name),
        source: wgpu::ShaderSource::Wgsl(data.source.as_str().into()),
    });
    let module = match pollster::block_on(scope.pop()) {
        None => Some(module),
        Some(e) => {
            log::error!("shader {}: the GPU rejected it, using the default instead: {e}", data.name);
            None
        }
    };
    let params = uniform_buffer(device, "spark shader params", data.values.len().clamp(16, MAX_PARAMS_SIZE) as u64);
    let resources = shader_resources(device, layout, samplers, white, textures, &params, data.textures);
    GpuShader {
        name: data.name.clone(),
        kind: data.kind,
        version: data.version,
        module,
        pipelines: HashMap::new(),
        params,
        resources,
        textures: data.textures,
    }
}

fn shader_resources(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    samplers: &[wgpu::Sampler; 4],
    white: &GpuTexture,
    textures: &[GpuTexture],
    params: &wgpu::Buffer,
    bound: [Option<TextureId>; 2],
) -> wgpu::BindGroup {
    let view = |t: Option<TextureId>| t.and_then(|t| textures.get(t.0 as usize)).map(|g| &g.view).unwrap_or(&white.view);
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("spark shader resources"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view(bound[0])) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(view(bound[1])) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&samplers[1]) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&samplers[0]) },
        ],
    })
}

/// Pipeline of a shader for one output format (cached; `None` cached when the GPU rejects it).
fn ensure_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, s: &mut GpuShader, format: wgpu::TextureFormat) {
    if s.pipelines.contains_key(&format) {
        return;
    }
    let Some(module) = &s.module else {
        s.pipelines.insert(format, None);
        return;
    };
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let vertex_buffers = [Some(wgpu::VertexBufferLayout {
        array_stride: size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &VERTEX_ATTRIBUTES,
    })];
    let surface = s.kind == ShaderKind::Surface;
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&s.name),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("spark_vs"),
            compilation_options: Default::default(),
            buffers: if surface { &vertex_buffers } else { &[] },
        },
        primitive: if surface {
            wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            }
        } else {
            wgpu::PrimitiveState::default()
        },
        depth_stencil: surface.then(|| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("spark_fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let pipeline = match pollster::block_on(scope.pop()) {
        None => Some(pipeline),
        Some(e) => {
            log::error!("shader {}: cannot build its pipeline, using the default instead: {e}", s.name);
            None
        }
    };
    s.pipelines.insert(format, pipeline);
}



/// Visible lights closest to the camera (by distance minus range), packed for the shader.
fn collect_lights(scene: &spark_core::Scene) -> ([LightUniform; MAX_LIGHTS], usize) {
    let cam = scene.camera.position;
    let mut found: Vec<(f32, LightUniform)> = Vec::new();
    for (id, obj) in scene.iter() {
        let Some(light) = obj.light else { continue };
        if light.intensity <= 0.0 || !scene.is_visible(id) {
            continue;
        }
        let m = scene.world_matrix(id);
        let pos = m.transform_point3(Vec3::ZERO);
        let dir = m.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Z);
        let c = light.color.to_linear();
        let i = light.intensity;
        let (spot, outer, inner) = match light.kind {
            spark_core::LightKind::Point => (0.0, -2.0, -1.0),
            spark_core::LightKind::Spot { angle, softness } => {
                let half = angle * 0.5;
                (1.0, half.cos(), (half * (1.0 - softness.clamp(0.0, 1.0))).cos())
            }
        };
        let u = LightUniform {
            position: pos.extend(light.range).to_array(),
            color: [c[0] * i, c[1] * i, c[2] * i, spot],
            direction: dir.extend(outer).to_array(),
            params: [inner, 0.0, 0.0, 0.0],
        };
        found.push((pos.distance(cam) - light.range, u));
    }
    if found.len() > MAX_LIGHTS {
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    let mut out = [LightUniform::zeroed(); MAX_LIGHTS];
    let n = found.len().min(MAX_LIGHTS);
    for (slot, (_, u)) in out.iter_mut().zip(found) {
        *slot = u;
    }
    (out, n)
}

fn request_adapter(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<wgpu::Adapter, String> {
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: surface,
        ..Default::default()
    }))
    .map_err(|e| format!("no compatible GPU found: {e}"))
}

fn request_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), String> {
    // Ask only for WebGL2-level limits so weak / old GPUs (and the GL backend) work.
    let desc = wgpu::DeviceDescriptor {
        label: Some("spark"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
        memory_hints: wgpu::MemoryHints::Performance,
        ..Default::default()
    };
    pollster::block_on(adapter.request_device(&desc)).map_err(|e| format!("cannot open GPU device: {e}"))
}

fn bind_group_layout(device: &wgpu::Device, label: &str, entries: &[wgpu::BindGroupLayoutEntry]) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries })
}

fn uniform_entry(binding: u32, dynamic: bool, size: u64, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: NonZeroU64::new(size),
        },
        count: None,
    }
}

fn texture_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn uniform_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn object_buffer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    capacity: usize,
    stride: u64,
    size: u64,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = uniform_buffer(device, "spark objects", stride * capacity as u64);
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("spark objects"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &buffer, offset: 0, size: NonZeroU64::new(size) }),
        }],
    });
    (buffer, bind_group)
}

fn vertex_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("spark canvas vertices"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn canvas_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("spark canvas"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: size_of::<Vertex2D>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &CANVAS_ATTRIBUTES,
            })],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    samplers: &[wgpu::Sampler; 4],
    image: &ImageData,
    format: wgpu::TextureFormat,
    mipmaps: bool,
) -> GpuTexture {
    let (levels, data) = if mipmaps { build_mips(image) } else { (1, std::borrow::Cow::Borrowed(&image.pixels[..])) };
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("spark texture"),
            size: extent((image.width, image.height)),
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &data,
    );
    let view = texture.create_view(&Default::default());
    let bind_group = |sampler: &wgpu::Sampler| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spark texture"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        })
    };
    GpuTexture {
        nearest: bind_group(&samplers[0]),
        linear: bind_group(&samplers[1]),
        nearest_clamp: bind_group(&samplers[2]),
        linear_clamp: bind_group(&samplers[3]),
        texture,
        view,
    }
}

/// Full mip chain (box filter in linear space, sRGB RGBA8): kills moire on distant tiled surfaces.
fn build_mips(image: &ImageData) -> (u32, std::borrow::Cow<'_, [u8]>) {
    let (mut w, mut h) = (image.width as usize, image.height as usize);
    let levels = 32 - (w.max(h) as u32).leading_zeros();
    if levels <= 1 {
        return (1, std::borrow::Cow::Borrowed(&image.pixels[..]));
    }
    let to_lin: Vec<f32> = (0..256).map(|i| (i as f32 / 255.0).powf(2.2)).collect();
    let mut out = image.pixels.clone();
    let mut cur: Vec<f32> = image.pixels.chunks(4).flat_map(|p| [to_lin[p[0] as usize], to_lin[p[1] as usize], to_lin[p[2] as usize], p[3] as f32 / 255.0]).collect();
    for _ in 1..levels {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0.0f32; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                let mut acc = [0.0f32; 4];
                let mut wsum = 0.0;
                for (sx, sy) in [(2 * x, 2 * y), (2 * x + 1, 2 * y), (2 * x, 2 * y + 1), (2 * x + 1, 2 * y + 1)] {
                    let (sx, sy) = (sx.min(w - 1), sy.min(h - 1));
                    let p = &cur[(sy * w + sx) * 4..][..4];
                    // alpha-weighted color so transparent texels don't bleed dark fringes
                    let a = p[3].max(1e-4);
                    for c in 0..3 {
                        acc[c] += p[c] * a;
                    }
                    acc[3] += p[3];
                    wsum += a;
                }
                let o = &mut next[(y * nw + x) * 4..][..4];
                for c in 0..3 {
                    o[c] = acc[c] / wsum;
                }
                o[3] = acc[3] / 4.0;
            }
        }
        out.extend(next.chunks(4).flat_map(|p| {
            let e = |v: f32| (v.max(0.0).powf(1.0 / 2.2) * 255.0 + 0.5).min(255.0) as u8;
            [e(p[0]), e(p[1]), e(p[2]), (p[3] * 255.0 + 0.5).min(255.0) as u8]
        }));
        cur = next;
        (w, h) = (nw, nh);
    }
    (levels, std::borrow::Cow::Owned(out))
}

fn extent(size: (u32, u32)) -> wgpu::Extent3d {
    wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 }
}

fn flag(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}

fn normal_matrix(model: &Mat4) -> Mat4 {
    if model.determinant().abs() < 1e-12 { Mat4::IDENTITY } else { model.inverse().transpose() }
}

/// True if the box is completely outside one clip plane.
fn outside_frustum(mvp: &Mat4, bounds: &Aabb) -> bool {
    if bounds.is_empty() {
        return false;
    }
    let mut out = [true; 6];
    for corner in bounds.corners() {
        let c = *mvp * corner.extend(1.0);
        out[0] &= c.x < -c.w;
        out[1] &= c.x > c.w;
        out[2] &= c.y < -c.w;
        out[3] &= c.y > c.w;
        out[4] &= c.z < 0.0;
        out[5] &= c.z > c.w;
    }
    out.iter().any(|&o| o)
}
