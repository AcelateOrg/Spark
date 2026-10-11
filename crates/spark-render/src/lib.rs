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
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use spark_core::shader::{COPY_POST, MAX_PARAMS_SIZE};
use spark_core::{
    Aabb, Assets, Batch, Canvas, CanvasTexture, Color, CubemapId, ImageData, Layer, Mat4, MeshId, PassSize, ShaderData,
    ShaderKind, TextureFilter, TextureId, Vec3, Vec4, Vertex, Vertex2D, World,
};
use spark_core::material::BlendMode;
use wgpu::util::DeviceExt;

pub use wgpu;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Scene and intermediate post images when the GPU supports it (see `Renderer::scene_format`).
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];
const CANVAS_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];
/// Glyph atlas: coverage in alpha, not sRGB-encoded.
const GLYPH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Pipeline variant bits: blend mode (0 opaque, 1 alpha, 2 additive) | double-sided.
/// Objects per instanced draw: `array<Object, 64>` uniform window (fits every GPU incl. GL).
/// Must match the array length in `surface.wgsl` and `shadow.wgsl`.
const MAX_INSTANCES: usize = 64;
const VARIANT_ALPHA: u8 = 1;
const VARIANT_ADDITIVE: u8 = 2;
const VARIANT_DOUBLE_SIDED: u8 = 4;

/// Pipeline cache key: output format, variant bits, MSAA sample count.
type PipelineKey = (wgpu::TextureFormat, u8, u32);

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
    shadow_view_proj: [[f32; 4]; 4],
    /// enabled, 1 / map size, normal offset, shadow distance
    shadow: [f32; 4],
    prev_view_proj: [[f32; 4]; 4],
    sky_top: [f32; 4],
    sky_bottom: [f32; 4],
    /// enabled, cubemap blend, lighting strength, cubemap mip count
    sky: [f32; 4],
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
    /// tiling x, tiling y, unlit, pbr
    info: [f32; 4],
    /// metallic, roughness, has normal map, -
    pbr: [f32; 4],
    /// rgb (linear), -
    emissive: [f32; 4],
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

/// Sky cubemap on the GPU (full mip chain: blurrier mips light rough surfaces).
struct GpuCube {
    view: wgpu::TextureView,
    mips: u32,
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
    pipelines: HashMap<PipelineKey, Option<wgpu::RenderPipeline>>,
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
    /// Built-in post-processing effect.
    Fx(spark_core::shader::PostFxKind),
}

struct SceneTarget {
    size: (u32, u32),
    samples: u32,
    _color: wgpu::Texture,
    _depth: wgpu::Texture,
    /// Single-sample (resolved) scene image and depth: read by the post chain.
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    /// Multisampled color + depth when MSAA is on (rendered into, then resolved).
    msaa: Option<(wgpu::Texture, wgpu::TextureView, wgpu::Texture, wgpu::TextureView)>,
    /// Bind group reading the multisampled depth (depth resolve pass).
    depth_resolve: Option<wgpu::BindGroup>,
}

/// Sun shadow map.
struct ShadowMap {
    size: u32,
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// One object to draw this frame (before sorting / batching).
struct Candidate {
    uniform: ObjectUniform,
    shader: ShaderRef,
    variant: u8,
    mesh: usize,
    texture: Option<usize>,
    normal_map: Option<usize>,
    /// View-space distance (transparent sorting).
    depth: f32,
    in_view: bool,
    caster: bool,
    triangles: u64,
}

/// Consecutive objects drawn with one instanced draw call.
struct Batch3 {
    shader: ShaderRef,
    variant: u8,
    mesh: usize,
    texture: Option<usize>,
    normal_map: Option<usize>,
    /// Byte offset of the batch's first object (dynamic uniform offset).
    offset: u32,
    count: u32,
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
    /// Object buffer size in bytes.
    object_capacity: usize,
    /// Dynamic offset alignment of the object buffer.
    object_align: u64,
    frame_layout: wgpu::BindGroupLayout,
    adapter_formats: HashMap<wgpu::TextureFormat, wgpu::TextureFormatFeatureFlags>,
    shadow_sampler: wgpu::Sampler,
    /// 1x1 map bound when shadows are off.
    shadow_dummy: ShadowMap,
    shadow: Option<ShadowMap>,
    /// Trilinear sampler of sky cubemaps.
    sky_sampler: wgpu::Sampler,
    /// 1x1 white cube bound when the sky has no cubemap.
    sky_dummy: GpuCube,
    cubemaps: Vec<GpuCube>,
    /// What `frame_bind_group` was built with: shadow map size (None = dummy), sky cubemap.
    frame_group_key: (Option<u32>, Option<usize>),
    sky_pipeline_layout: wgpu::PipelineLayout,
    sky_shader: wgpu::ShaderModule,
    /// Skybox pipelines per MSAA sample count.
    sky_pipelines: HashMap<u32, wgpu::RenderPipeline>,
    shadow_buffer: wgpu::Buffer,
    shadow_bind_group: wgpu::BindGroup,
    shadow_pipeline: wgpu::RenderPipeline,
    depth_resolve_layout: wgpu::BindGroupLayout,
    depth_resolve_layout_p: wgpu::PipelineLayout,
    depth_resolve_shader: wgpu::ShaderModule,
    depth_resolve_pipelines: HashMap<u32, wgpu::RenderPipeline>,
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
    /// Built-in post-processing effect shaders (indexed by PostFxKind).
    built_in_post: Vec<GpuShader>,
    /// ShaderData for built-in post effects (for param value management).
    built_in_post_data: Vec<spark_core::shader::ShaderData>,
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
    /// Flat tangent-space normal (bound as `normal_texture` when a material has no normal map).
    flat_normal: wgpu::TextureView,
    /// Texture bind groups of normal-mapped materials: (base texture, normal map, nearest).
    normal_mapped_groups: HashMap<(Option<usize>, usize, bool), wgpu::BindGroup>,
    assets_generation: u64,
    /// `Assets::mesh_edits` already applied.
    mesh_edits: u64,
    candidates: Vec<Candidate>,
    order: Vec<u32>,
    batches: Vec<Batch3>,
    shadow_batches: Vec<Batch3>,
    scratch: Vec<u8>,
    world_matrices: Vec<Mat4>,
    world_visible: Vec<bool>,
    device_lost: Arc<AtomicBool>,
    /// Format of the 3D scene image and intermediate post images: HDR (Rgba16Float) when supported.
    scene_format: wgpu::TextureFormat,
    /// (frame number, view_proj) of the last drawn frame and the view_proj before it (motion blur).
    camera_history: Option<(u64, Mat4)>,
    prev_view_proj: Mat4,
    /// Numbers about the last frame drawn by [`Renderer::render`] (shown by the debug overlay).
    last_stats: FrameStats,
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

    /// Numbers about the last rendered frame.
    pub fn last_stats(&self) -> FrameStats {
        self.last_stats
    }

    /// Whether the GPU device was lost (driver crash, TDR, etc.).
    pub fn is_device_lost(&self) -> bool {
        self.device_lost.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Recreates device, queue and GPU resources after a device loss (the window surface is
    /// reconfigured). Assets are re-uploaded on the next frame.
    pub fn recover(&mut self) -> Result<(), String> {
        log::info!("attempting GPU device recovery...");
        self.device_lost.store(false, std::sync::atomic::Ordering::SeqCst);

        let surface = self.surface.take();
        let surface_factory = self.surface_factory.take();
        let width = self.width;
        let height = self.height;

        // Move the instance out (replace with a dummy that will be discarded on overwrite).
        let dummy = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let instance = std::mem::replace(&mut self.instance, dummy);

        let adapter = request_adapter(&instance, surface.as_ref().map(|s| &s.surface))?;
        let (device, queue) = request_device(&adapter)?;

        let mut new = Self::build(instance, &adapter, device, queue, width, height);
        new.surface = surface;
        new.surface_factory = surface_factory;

        if let Some(s) = &new.surface {
            s.surface.configure(&new.device, &s.config);
        }

        *self = new;
        Ok(())
    }

    /// Draws the world to the window.
    pub fn render(&mut self, world: &World) -> Result<FrameStats, String> {
        self.render_frame(world, false).map(|(stats, _)| stats)
    }

    /// Draws the world to the window and also returns that frame as an image (F12): the scene
    /// is drawn once, only the cheap post chain runs a second time into the capture target.
    pub fn render_and_capture(&mut self, world: &World) -> Result<(FrameStats, Option<ImageData>), String> {
        self.render_frame(world, true)
    }

    /// [`Self::render`], plus the frame as an image when `capture` is true.
    pub fn render_and_capture_if(&mut self, world: &World, capture: bool) -> Result<Option<ImageData>, String> {
        self.render_frame(world, capture).map(|(_, image)| image)
    }

    fn render_frame(&mut self, world: &World, capture: bool) -> Result<(FrameStats, Option<ImageData>), String> {
        if self.is_device_lost() {
            return Err("GPU device lost".into());
        }
        let Some(state) = &self.surface else {
            return Err("render(): this renderer is headless, use screenshot()".into());
        };
        let view_format = state.view_format;
        let frame = match state.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                drop(t);
                self.reconfigure();
                return Ok((FrameStats::default(), None));
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.reconfigure();
                return Ok((FrameStats::default(), None));
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.recreate_surface();
                return Ok((FrameStats::default(), None));
            }
            wgpu::CurrentSurfaceTexture::Validation => return Err("window surface validation error".into()),
            #[allow(unreachable_patterns)]
            _ => return Ok((FrameStats::default(), None)),
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor { format: Some(view_format), ..Default::default() });
        self.ensure_canvas_pipeline(view_format);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("spark frame") });
        let mut stats = self.draw_scene(&mut encoder, world);
        stats.post_passes = self.draw_post(&mut encoder, &view, view_format, world);
        self.last_stats = stats;
        if !capture {
            self.queue.submit([encoder.finish()]);
            self.queue.present(frame);
            return Ok((stats, None));
        }
        self.ensure_capture();
        self.ensure_canvas_pipeline(COLOR_FORMAT);
        // ensure_capture() was just called, so self.capture is Some.
        let target = self.capture.take().ok_or_else(|| "capture target was not ensured".to_string())?;
        self.draw_post(&mut encoder, &target.view, COLOR_FORMAT, world);
        self.capture = Some(target);
        let image = self.read_capture(encoder);
        self.queue.present(frame);
        Ok((stats, Some(image?)))
    }

    fn ensure_capture(&mut self) {
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
    }

    /// Draws the world and returns the final image (same as what the window shows).
    pub fn screenshot(&mut self, world: &World) -> Result<ImageData, String> {
        if self.is_device_lost() {
            return Err("GPU device lost".into());
        }
        self.ensure_capture();
        self.ensure_canvas_pipeline(COLOR_FORMAT);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("spark screenshot") });
        let mut stats = self.draw_scene(&mut encoder, world);
        // ensure_capture() was just called, so self.capture is Some.
        let capture = self.capture.take().ok_or_else(|| "capture target was not ensured".to_string())?;
        stats.post_passes = self.draw_post(&mut encoder, &capture.view, COLOR_FORMAT, world);
        self.last_stats = stats;
        self.capture = Some(capture);
        self.read_capture(encoder)
    }

    /// Submits `encoder` plus a copy of the capture target and reads the pixels back.
    fn read_capture(&mut self, mut encoder: wgpu::CommandEncoder) -> Result<ImageData, String> {
        // caller restores self.capture = Some(...) before invoking read_capture.
        let capture = self.capture.as_ref().ok_or_else(|| "capture target was not ensured".to_string())?;
        let size = capture.size;
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

        // GPU error handling: log uncaptured errors instead of panicking.
        device.on_uncaptured_error(Arc::new(|e| {
            match e {
                wgpu::Error::Validation { description, .. } => log::error!("GPU validation error: {description}"),
                wgpu::Error::OutOfMemory { source, .. } => log::error!("GPU out of memory: {source}"),
                wgpu::Error::Internal { description, .. } => log::error!("GPU internal error: {description}"),
            }
        }));

        // Device loss tracking: set a flag the renderer can check before drawing.
        let device_lost = Arc::new(AtomicBool::new(false));
        device.set_device_lost_callback({
            let flag = device_lost.clone();
            Box::new(move |reason, msg| {
                log::error!("GPU device lost: {reason:?}: {msg}");
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            })
        });


        // HDR scene (values above 1 survive until tonemapping / bloom) where the GPU can render,
        // blend and filter half floats; 8-bit sRGB otherwise (some GLES devices).
        let hdr = adapter.get_texture_format_features(HDR_FORMAT);
        let hdr_ok = hdr.allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING)
            && hdr.flags.contains(wgpu::TextureFormatFeatureFlags::FILTERABLE | wgpu::TextureFormatFeatureFlags::BLENDABLE);
        let scene_format = if hdr_ok { HDR_FORMAT } else { COLOR_FORMAT };
        let adapter_formats = [COLOR_FORMAT, DEPTH_FORMAT, scene_format]
            .into_iter()
            .map(|f| (f, adapter.get_texture_format_features(f).flags))
            .collect::<HashMap<_, _>>();
        let frame_size = size_of::<FrameUniform>() as u64;
        let object_size = size_of::<ObjectUniform>() as u64;
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fs = wgpu::ShaderStages::FRAGMENT;

        let depth_entry = |binding: u32, multisampled: bool, visibility: wgpu::ShaderStages| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled,
            },
            count: None,
        };
        let frame_layout = bind_group_layout(
            &device,
            "spark frame",
            &[
                uniform_entry(0, false, frame_size, vf),
                depth_entry(1, false, vf),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: vf,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: fs,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                sampler_entry(4, fs),
            ],
        );
        let object_layout = bind_group_layout(
            &device,
            "spark objects",
            &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: vf,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: NonZeroU64::new(object_size * MAX_INSTANCES as u64),
                },
                count: None,
            }],
        );
        // Base texture, its sampler and the material normal map (2D canvas uses only the first two).
        let texture_layout = bind_group_layout(&device, "spark texture", &[texture_entry(0, fs), sampler_entry(1, fs), texture_entry(2, fs)]);
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
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("spark shadow sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_dummy = shadow_map(&device, 1);
        let sky_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("spark sky sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let sky_dummy = upload_cube(&device, &queue, &std::array::from_fn(|_| ImageData::solid(Color::WHITE)));
        let frame_bind_group =
            frame_bind_group(&device, &frame_layout, &frame_buffer, &shadow_dummy.view, &shadow_sampler, &sky_dummy.view, &sky_sampler);
        let sky_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spark sky"),
            bind_group_layouts: &[Some(&frame_layout)],
            immediate_size: 0,
        });
        let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spark sky"),
            source: wgpu::ShaderSource::Wgsl(spark_core::shader::skybox_source().into()),
        });

        // Per-object uniforms, read in 64-object windows (dynamic offsets) by instanced draws.
        let object_align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let object_capacity = 64 * 1024;
        let (object_buffer, object_bind_group) = object_buffer(&device, &object_layout, object_capacity);

        let shadow_layout =
            bind_group_layout(&device, "spark shadow", &[uniform_entry(0, false, 64, wgpu::ShaderStages::VERTEX)]);
        let shadow_buffer = uniform_buffer(&device, "spark shadow view", 64);
        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spark shadow"),
            layout: &shadow_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: shadow_buffer.as_entire_binding() }],
        });
        let shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spark shadow"),
            bind_group_layouts: &[Some(&shadow_layout), Some(&object_layout)],
            immediate_size: 0,
        });
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spark shadow shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow.wgsl").into()),
        });
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spark shadow"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &VERTEX_ATTRIBUTES[..1],
                })],
            },
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.5, clamp: 0.0 },
            }),
            multisample: Default::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let depth_resolve_layout = bind_group_layout(&device, "spark depth resolve", &[depth_entry(0, true, fs)]);
        let depth_resolve_layout_p = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spark depth resolve"),
            bind_group_layouts: &[Some(&depth_resolve_layout)],
            immediate_size: 0,
        });
        let depth_resolve_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spark depth resolve"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/depth_resolve.wgsl").into()),
        });

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
        // (128, 128, 255) = +Z in tangent space; read back as data by `surface_normal`.
        let flat_normal = device
            .create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some("spark flat normal"),
                    size: extent((1, 1)),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: COLOR_FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[128, 128, 255, 255],
            )
            .create_view(&Default::default());
        let white = upload_texture(
            &device,
            &queue,
            &texture_layout,
            &texture_samplers,
            &flat_normal,
            &ImageData::solid(Color::WHITE),
            COLOR_FORMAT,
            false,
        );

        let builtin = |name: &str, code: &str| {
            // built-in shaders are known-good at compile time.
            let data = ShaderData::compile(name, code).expect("built-in shader compiles");
            gpu_shader(&device, &shader_layout, &texture_samplers, &white, &[], &data)
        };
        let default_surface = builtin("spark default surface", "");
        let copy_post = builtin("spark upscale", COPY_POST);

        // Compile all built-in post-processing effect shaders.
        let mut built_in_post = Vec::with_capacity(spark_core::shader::PostFxKind::COUNT);
        let mut built_in_post_data = Vec::with_capacity(spark_core::shader::PostFxKind::COUNT);
        for kind in spark_core::shader::PostFxKind::ALL {
            let name = kind.name();
            let code = kind.shader_source();
            // Validated by spark-core's `builtin_post_effects_compile` test.
            let data = ShaderData::compile(name, code).unwrap_or_else(|e| panic!("built-in post shader '{name}': {e}"));
            let gpu = gpu_shader(&device, &shader_layout, &texture_samplers, &white, &[], &data);
            built_in_post.push(gpu);
            built_in_post_data.push(data);
        }

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
        for f in [COLOR_FORMAT, scene_format] {
            canvas_pipelines.entry(f).or_insert_with(|| canvas_pipeline(&device, &canvas_pipeline_layout, &canvas_shader, f));
        }
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
            object_align,
            frame_layout,
            adapter_formats,
            shadow_sampler,
            shadow_dummy,
            shadow: None,
            sky_sampler,
            sky_dummy,
            cubemaps: Vec::new(),
            frame_group_key: (None, None),
            sky_pipeline_layout,
            sky_shader,
            sky_pipelines: HashMap::new(),
            shadow_buffer,
            shadow_bind_group,
            shadow_pipeline,
            depth_resolve_layout,
            depth_resolve_layout_p,
            depth_resolve_shader,
            depth_resolve_pipelines: HashMap::new(),
            texture_layout,
            texture_samplers,
            shader_layout,
            mesh_pipeline_layout,
            post_layout,
            post_pipeline_layout,
            default_surface,
            copy_post,
            built_in_post,
            built_in_post_data,
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
            flat_normal,
            normal_mapped_groups: HashMap::new(),
            assets_generation: u64::MAX,
            mesh_edits: 0,
            candidates: Vec::new(),
            order: Vec::new(),
            batches: Vec::new(),
            shadow_batches: Vec::new(),
            scratch: Vec::new(),
            world_matrices: Vec::new(),
            world_visible: Vec::new(),
            device_lost,
            scene_format,
            camera_history: None,
            prev_view_proj: Mat4::IDENTITY,
            last_stats: FrameStats::default(),
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
            self.cubemaps.clear();
            self.normal_mapped_groups.clear();
            self.shaders.clear();
            self.assets_generation = assets.generation();
        }
        while self.meshes.len() < assets.mesh_count() {
            // loop condition guarantees mesh id < assets.mesh_count().
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
            // loop condition guarantees texture id < assets.texture_count().
            let image = assets.texture(TextureId(self.textures.len() as u32)).expect("texture id in range");
            let gpu = if image.width == 0 || image.height == 0 || image.width > max || image.height > max {
                log::error!("texture {}x{} is not supported (max {max}x{max}); using magenta", image.width, image.height);
                let magenta = ImageData::solid(Color::hex(0xff00ff));
                upload_texture(&self.device, &self.queue, &self.texture_layout, &self.texture_samplers, &self.flat_normal, &magenta, COLOR_FORMAT, false)
            } else {
                upload_texture(&self.device, &self.queue, &self.texture_layout, &self.texture_samplers, &self.flat_normal, image, COLOR_FORMAT, true)
            };
            self.textures.push(gpu);
        }
        while self.cubemaps.len() < assets.cubemap_count() {
            // loop condition guarantees cubemap id < assets.cubemap_count().
            let faces = assets.cubemap(CubemapId(self.cubemaps.len() as u32)).expect("cubemap id in range");
            let gpu = if faces[0].width > max {
                log::error!("cubemap faces {0}x{0} are too big (max {1}); using white", faces[0].width, max);
                upload_cube(&self.device, &self.queue, &std::array::from_fn(|_| ImageData::solid(Color::WHITE)))
            } else {
                upload_cube(&self.device, &self.queue, faces)
            };
            self.cubemaps.push(gpu);
        }
    }


    /// Skybox pipeline for the scene pass at `samples` MSAA (depth test off, no depth write).
    fn ensure_sky_pipeline(&mut self, samples: u32) {
        if self.sky_pipelines.contains_key(&samples) {
            return;
        }
        let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spark sky"),
            layout: Some(&self.sky_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &self.sky_shader,
                entry_point: Some("sky_vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &self.sky_shader,
                entry_point: Some("sky_fs"),
                compilation_options: Default::default(),
                targets: &[Some(self.scene_format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.sky_pipelines.insert(samples, p);
    }

    fn ensure_scene_target(&mut self, size: (u32, u32), samples: u32) {
        let max = self.device.limits().max_texture_dimension_2d;
        let size = (size.0.clamp(1, max), size.1.clamp(1, max));
        if self.scene_target.as_ref().is_some_and(|t| t.size == size && t.samples == samples) {
            return;
        }
        let make = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages, sample_count: u32| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent(size),
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let color = make("spark scene color", self.scene_format, rt, 1);
        let depth = make("spark scene depth", DEPTH_FORMAT, rt, 1);
        let (msaa, depth_resolve) = if samples > 1 {
            let mc = make("spark scene color msaa", self.scene_format, wgpu::TextureUsages::RENDER_ATTACHMENT, samples);
            let md = make("spark scene depth msaa", DEPTH_FORMAT, rt, samples);
            let (mcv, mdv) = (mc.create_view(&Default::default()), md.create_view(&Default::default()));
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("spark depth resolve"),
                layout: &self.depth_resolve_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&mdv) }],
            });
            (Some((mc, mcv, md, mdv)), Some(bg))
        } else {
            (None, None)
        };
        self.scene_target = Some(SceneTarget {
            size,
            samples,
            color_view: color.create_view(&Default::default()),
            depth_view: depth.create_view(&Default::default()),
            _color: color,
            _depth: depth,
            msaa,
            depth_resolve,
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
                        Some(upload_texture(&self.device, &self.queue, &self.texture_layout, &self.texture_samplers, &self.flat_normal, atlas, GLYPH_FORMAT, false));
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
            // loop bound is assets.shader_count(), so id is in range.
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

    /// Uploads the parameters of the enabled built-in effects. A name mismatch is an engine bug
    /// (covered by tests); it is logged instead of panicking a running game.
    fn update_postfx_params(&mut self, passes: &[(spark_core::shader::PostFxKind, spark_core::render::PostFxParams)]) {
        for (kind, params) in passes {
            let i = *kind as usize;
            let data = &mut self.built_in_post_data[i];
            for (name, value) in params {
                if let Err(e) = data.set_param(name, value) {
                    log::error!("{e}");
                }
            }
            self.queue.write_buffer(&self.built_in_post[i].params, 0, &data.values);
        }
    }

    fn shader(&self, r: ShaderRef) -> &GpuShader {
        match r {
            ShaderRef::Default => &self.default_surface,
            ShaderRef::Copy => &self.copy_post,
            ShaderRef::Asset(i) => &self.shaders[i],
            ShaderRef::Fx(k) => &self.built_in_post[k as usize],
        }
    }

    /// Asset shader `id` if it exists and is of `kind`.
    fn asset_shader(&self, id: Option<spark_core::ShaderId>, kind: ShaderKind) -> Option<ShaderRef> {
        let i = id?.0 as usize;
        (self.shaders.get(i)?.kind == kind).then_some(ShaderRef::Asset(i))
    }

    /// Builds the pipeline of `r` for `format` if needed. `false` = the shader cannot be used.
    fn ensure_shader_pipeline(&mut self, r: ShaderRef, key: PipelineKey) -> bool {
        let s = match r {
            ShaderRef::Default => &mut self.default_surface,
            ShaderRef::Copy => &mut self.copy_post,
            ShaderRef::Asset(i) => &mut self.shaders[i],
            ShaderRef::Fx(k) => &mut self.built_in_post[k as usize],
        };
        let layout = match s.kind {
            ShaderKind::Surface => &self.mesh_pipeline_layout,
            ShaderKind::Post => &self.post_pipeline_layout,
        };
        ensure_pipeline(&self.device, layout, s, key);
        s.pipelines.get(&key).is_some_and(|p| p.is_some())
    }

    fn pipeline(&self, r: ShaderRef, key: PipelineKey) -> &wgpu::RenderPipeline {
        // caller must ensure_shader_pipeline() before calling pipeline().
        self.shader(r).pipelines[&key].as_ref().expect("pipeline ensured")
    }

    /// MSAA sample count supported for both scene formats (falls back to 1).
    fn supported_samples(&self, wanted: u32) -> u32 {
        let ok = |n: u32| {
            [self.scene_format, DEPTH_FORMAT].iter().all(|f| self.adapter_formats.get(f).is_some_and(|fl| fl.sample_count_supported(n)))
        };
        if self.info.backend == "Gl" {
            return 1; // GL / GLES: no multisampled depth reads for the post chain
        }
        [8, 4, 2].into_iter().find(|&n| n <= wanted && ok(n)).unwrap_or(1)
    }

    fn draw_scene(&mut self, encoder: &mut wgpu::CommandEncoder, world: &World) -> FrameStats {
        self.sync_assets(&world.assets);
        self.sync_shaders(&world.assets);
        let rs = &world.render;
        let samples = self.supported_samples(rs.msaa.max(1));
        self.ensure_scene_target(rs.internal_size(self.width, self.height), samples);
        let internal = self.scene_target.as_ref().expect("scene target").size; // ensure_scene_target() was called

        let scene = &world.scene;
        let cam = &scene.camera;
        let aspect = self.width as f32 / self.height as f32;
        let (view, proj) = (cam.view(), cam.projection(aspect));
        let view_proj = proj * view;
        let planes = frustum_planes(&view_proj);
        let sun = scene.sun.color.to_linear();
        let si = scene.sun.intensity;
        let (fog_color, fog) = match scene.fog {
            Some(f) => (f.color.to_linear(), [f.near, f.far, 1.0, 0.0]),
            None => ([0.0; 4], [0.0; 4]),
        };

        // World matrices + visibility of every object in one O(n) pass (interpolated bodies).
        let mut mats = std::mem::take(&mut self.world_matrices);
        let mut vis = std::mem::take(&mut self.world_visible);
        scene.compute_world_transforms(world.time.alpha(), &mut mats, &mut vis);
        let (lights, light_count) = collect_lights(scene, &mats, &vis, &planes);

        // Sun shadow map.
        let max_dim = self.device.limits().max_texture_dimension_2d;
        let shadow_size = rs.shadow_size.clamp(256, max_dim.min(8192));
        let shadows = rs.shadows && si > 0.0 && rs.shadow_distance > 0.0;
        if shadows && self.shadow.as_ref().is_none_or(|m| m.size != shadow_size) {
            self.shadow = Some(shadow_map(&self.device, shadow_size));
        } else if !shadows {
            self.shadow = None;
        }
        let sky = scene.sky;
        let sky_cube = sky.and_then(|s| s.cubemap).map(|c| c.0 as usize).filter(|&c| c < self.cubemaps.len());
        let key = (self.shadow.as_ref().map(|m| m.size), sky_cube);
        if key != self.frame_group_key {
            let shadow_view = self.shadow.as_ref().map_or(&self.shadow_dummy.view, |m| &m.view);
            let sky_view = sky_cube.map_or(&self.sky_dummy.view, |c| &self.cubemaps[c].view);
            self.frame_bind_group = frame_bind_group(
                &self.device,
                &self.frame_layout,
                &self.frame_buffer,
                shadow_view,
                &self.shadow_sampler,
                sky_view,
                &self.sky_sampler,
            );
            self.frame_group_key = key;
        }
        let light_vp = if shadows { sun_view_proj(cam, scene.sun.direction, rs.shadow_distance, shadow_size) } else { Mat4::IDENTITY };
        let texel_world = rs.shadow_distance / shadow_size as f32;

        let t = &world.time;
        // Only a new frame moves the history (a screenshot of the same frame keeps it).
        if self.camera_history.is_none_or(|(f, _)| f != t.frame) {
            self.prev_view_proj = self.camera_history.map_or(view_proj, |(_, vp)| vp);
            self.camera_history = Some((t.frame, view_proj));
        }
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
            shadow_view_proj: light_vp.to_cols_array_2d(),
            shadow: [flag(shadows), 1.0 / shadow_size as f32, texel_world * 1.5, rs.shadow_distance],
            prev_view_proj: self.prev_view_proj.to_cols_array_2d(),
            sky_top: sky.map_or([0.0; 4], |s| s.top.to_linear()),
            sky_bottom: sky.map_or([0.0; 4], |s| s.bottom.to_linear()),
            sky: match sky {
                Some(s) => [
                    1.0,
                    if sky_cube.is_some() && s.blend > 0.0 { s.blend.min(1.0) } else { 0.0 }, // NaN -> 0
                    s.lighting.max(0.0),
                    sky_cube.map_or(1, |c| self.cubemaps[c].mips) as f32,
                ],
                None => [0.0; 4],
            },
        };
        self.queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&frame));
        if shadows {
            self.queue.write_buffer(&self.shadow_buffer, 0, bytemuck::bytes_of(&light_vp.to_cols_array_2d()));
        }

        // Candidates: everything visible in the camera or casting into the shadow map.
        let mut cands = std::mem::take(&mut self.candidates);
        cands.clear();
        let mut stats = FrameStats { internal_size: internal, ..Default::default() };
        let fallback = self.asset_shader(rs.shader, ShaderKind::Surface).unwrap_or(ShaderRef::Default);
        for (id, obj) in scene.iter() {
            if obj.mesh.is_none() {
                continue;
            }
            let i = id.index() as usize;
            if !vis[i] {
                continue;
            }
            let model = mats[i];
            let m = &obj.material;
            let bounds = obj.local_bounds();
            let center = model.transform_point3(if bounds.is_empty() { Vec3::ZERO } else { (bounds.min + bounds.max) * 0.5 });
            // Level of detail by camera distance; a missing / empty LOD mesh falls back to the base mesh.
            let uploaded = |m: Option<spark_core::MeshId>| m.map(|m| m.0 as usize).filter(|&m| matches!(self.meshes.get(m), Some(Some(_))));
            let Some(mesh) = uploaded(obj.mesh_at(center.distance(cam.position))).or_else(|| uploaded(obj.mesh)) else { continue };
            let Some(Some(gpu_mesh)) = self.meshes.get(mesh) else { continue };
            let in_view = !outside_frustum(&(view_proj * model), &bounds);
            let transparent = m.is_transparent();
            let caster = shadows && m.cast_shadow && !transparent && !outside_frustum(&(light_vp * model), &bounds);
            if !in_view {
                stats.culled += 1;
                if !caster {
                    continue;
                }
            }
            let variant = match m.effective_blend() {
                BlendMode::Alpha => VARIANT_ALPHA,
                BlendMode::Additive => VARIANT_ADDITIVE,
                _ => 0,
            } | if m.double_sided { VARIANT_DOUBLE_SIDED } else { 0 };
            let normal_map = m.normal_map.map(|t| t.0 as usize).filter(|&t| t < self.textures.len());
            let emissive = m.emissive.to_linear();
            cands.push(Candidate {
                uniform: ObjectUniform {
                    model: model.to_cols_array_2d(),
                    normal_matrix: normal_matrix(&model).to_cols_array_2d(),
                    color: m.color.to_linear(),
                    data: m.data,
                    info: [m.tiling.x, m.tiling.y, flag(m.unlit), flag(m.pbr)],
                    pbr: [m.metallic, m.roughness, flag(normal_map.is_some()), 0.0],
                    emissive: [emissive[0], emissive[1], emissive[2], 0.0],
                },
                shader: self.asset_shader(m.shader, ShaderKind::Surface).unwrap_or(fallback),
                variant,
                mesh,
                texture: m.texture.map(|t| t.0 as usize).filter(|&t| t < self.textures.len()),
                normal_map,
                depth: -view.transform_point3(center).z,
                in_view,
                caster,
                triangles: (gpu_mesh.index_count / 3) as u64,
            });
        }
        self.world_matrices = mats;
        self.world_visible = vis;

        // Pipelines for every (shader, variant) in use; broken shaders fall back to the default.
        let mut usable: HashMap<(ShaderRef, u8), bool> = HashMap::new();
        for c in cands.iter_mut().filter(|c| c.in_view) {
            let key = (c.shader, c.variant);
            let ok = match usable.get(&key) {
                Some(&ok) => ok,
                None => {
                    let ok = self.ensure_shader_pipeline(c.shader, (self.scene_format, c.variant, samples));
                    usable.insert(key, ok);
                    ok
                }
            };
            if !ok {
                c.shader = ShaderRef::Default;
                self.ensure_shader_pipeline(ShaderRef::Default, (self.scene_format, c.variant, samples));
            }
        }

        // Order: opaque grouped by state (instanced), then transparent back to front.
        let mut order = std::mem::take(&mut self.order);
        order.clear();
        order.extend((0..cands.len() as u32).filter(|&i| cands[i as usize].in_view));
        let transparent = |c: &Candidate| c.variant & (VARIANT_ALPHA | VARIANT_ADDITIVE) != 0;
        order.sort_by(|&a, &b| {
            let (a, b) = (&cands[a as usize], &cands[b as usize]);
            match (transparent(a), transparent(b)) {
                (false, false) => (a.variant, a.shader, a.texture, a.normal_map, a.mesh).cmp(&(b.variant, b.shader, b.texture, b.normal_map, b.mesh)),
                (true, true) => b.depth.total_cmp(&a.depth),
                (ta, tb) => ta.cmp(&tb),
            }
        });
        let scene_count = order.len();
        order.extend((0..cands.len() as u32).filter(|&i| cands[i as usize].caster));
        order[scene_count..].sort_by_key(|&i| cands[i as usize].mesh);

        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.clear();
        let mut batches = std::mem::take(&mut self.batches);
        let mut shadow_batches = std::mem::take(&mut self.shadow_batches);
        batches.clear();
        shadow_batches.clear();
        let align = self.object_align as usize;
        for (slot, &ci) in order.iter().enumerate() {
            let c = &cands[ci as usize];
            let shadow_part = slot >= scene_count;
            if !shadow_part {
                stats.triangles += c.triangles;
            }
            let list = if shadow_part { &mut shadow_batches } else { &mut batches };
            let same = list.last().is_some_and(|b: &Batch3| {
                (b.count as usize) < MAX_INSTANCES
                    && b.mesh == c.mesh
                    && (shadow_part
                        || (b.shader == c.shader && b.variant == c.variant && b.texture == c.texture && b.normal_map == c.normal_map))
            });
            if same {
                list.last_mut().expect("batch").count += 1; // checked is_some() above
            } else {
                scratch.resize(scratch.len().div_ceil(align) * align, 0);
                list.push(Batch3 {
                    shader: c.shader,
                    variant: c.variant,
                    mesh: c.mesh,
                    texture: c.texture,
                    normal_map: c.normal_map,
                    offset: scratch.len() as u32,
                    count: 1,
                });
            }
            scratch.extend_from_slice(bytemuck::bytes_of(&c.uniform));
        }
        stats.draw_calls = batches.len() as u32;
        self.candidates = cands;
        self.order = order;

        // The last batch binds a full 64-object window: keep that much room after it.
        let needed = scratch.len() + MAX_INSTANCES * size_of::<ObjectUniform>();
        if needed > self.object_capacity {
            self.object_capacity = needed.next_power_of_two();
            let (buffer, bind_group) = object_buffer(&self.device, &self.object_layout, self.object_capacity);
            self.object_buffer = buffer;
            self.object_bind_group = bind_group;
        }
        if !scratch.is_empty() {
            self.queue.write_buffer(&self.object_buffer, 0, &scratch);
        }
        self.scratch = scratch;

        // Shadow pass.
        if let Some(map) = self.shadow.as_ref().filter(|_| shadows) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spark shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &map.view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.shadow_bind_group, &[]);
            for b in &shadow_batches {
                pass.set_bind_group(1, &self.object_bind_group, &[b.offset]);
                let mesh = self.meshes[b.mesh].as_ref().expect("uploaded mesh"); // candidates filtered to have uploaded meshes
                pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, 0..b.count);
            }
        }

        let nearest = rs.filter == TextureFilter::Nearest;
        for b in &batches {
            if let Some(normal) = b.normal_map {
                let key = (b.texture, normal, nearest);
                if !self.normal_mapped_groups.contains_key(&key) {
                    let base = b.texture.map_or(&self.white.view, |t| &self.textures[t].view);
                    let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("spark normal-mapped material"),
                        layout: &self.texture_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(base) },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.texture_samplers[if nearest { 0 } else { 1 }]),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::TextureView(&self.textures[normal].view),
                            },
                        ],
                    });
                    self.normal_mapped_groups.insert(key, group);
                }
            }
        }

        if sky.is_some() {
            self.ensure_sky_pipeline(samples);
        }
        let target = self.scene_target.as_ref().expect("scene target"); // ensure_scene_target() was called
        let bg = scene.background.to_linear();
        {
            let (color_view, resolve, depth_view) = match &target.msaa {
                Some((_, cv, _, dv)) => (cv, Some(&target.color_view), dv),
                None => (&target.color_view, None, &target.depth_view),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spark scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: bg[0] as f64, g: bg[1] as f64, b: bg[2] as f64, a: 1.0 }),
                        store: if resolve.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            if sky.is_some() {
                pass.set_pipeline(&self.sky_pipelines[&samples]); // ensure_sky_pipeline() above
                pass.draw(0..3, 0..1);
            }
            let (mut last_pipe, mut last_shader, mut last_mesh, mut last_texture) =
                (None, None, usize::MAX, None::<(Option<usize>, Option<usize>)>);
            for b in &batches {
                if last_pipe != Some((b.shader, b.variant)) {
                    pass.set_pipeline(self.pipeline(b.shader, (self.scene_format, b.variant, samples)));
                    last_pipe = Some((b.shader, b.variant));
                }
                if last_shader != Some(b.shader) {
                    pass.set_bind_group(3, &self.shader(b.shader).resources, &[]);
                    last_shader = Some(b.shader);
                }
                pass.set_bind_group(1, &self.object_bind_group, &[b.offset]);
                let mesh = self.meshes[b.mesh].as_ref().expect("uploaded mesh"); // candidates filtered to have uploaded meshes
                if last_texture != Some((b.texture, b.normal_map)) {
                    let group = match b.normal_map {
                        Some(normal) => &self.normal_mapped_groups[&(b.texture, normal, nearest)], // created above
                        None => {
                            let tex = b.texture.map(|t| &self.textures[t]).unwrap_or(&self.white);
                            if nearest { &tex.nearest } else { &tex.linear }
                        }
                    };
                    pass.set_bind_group(2, group, &[]);
                    last_texture = Some((b.texture, b.normal_map));
                }
                if last_mesh != b.mesh {
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    last_mesh = b.mesh;
                }
                pass.draw_indexed(0..mesh.index_count, 0, 0..b.count);
            }
        }
        self.batches = batches;
        self.shadow_batches = shadow_batches;

        // Post shaders and depth-based built-in effects (SSAO, depth of field, motion blur)
        // read the single-sample depth: resolve it only when one of them will run.
        if samples > 1 && (!world.render.post.is_empty() || world.render.postfx.is_active()) {
            if !self.depth_resolve_pipelines.contains_key(&samples) {
                let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("spark depth resolve"),
                    layout: Some(&self.depth_resolve_layout_p),
                    vertex: wgpu::VertexState {
                        module: &self.depth_resolve_shader,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: Default::default(),
                    depth_stencil: Some(wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(wgpu::CompareFunction::Always),
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &self.depth_resolve_shader,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[],
                    }),
                    multiview_mask: None,
                    cache: None,
                });
                self.depth_resolve_pipelines.insert(samples, p);
            }
            let target = self.scene_target.as_ref().expect("scene target"); // ensure_scene_target() was called
            if let Some(bg) = &target.depth_resolve {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("spark depth resolve"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &target.depth_view,
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.depth_resolve_pipelines[&samples]);
                pass.set_bind_group(0, bg, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        // 2D "scene" layer: drawn into the 3D image, so it goes through the post passes.
        self.prepare_canvas(&world.canvas);
        let scene_layer = world.canvas.layer_data(Layer::Scene);
        if !scene_layer.is_empty() {
            let target = self.scene_target.as_ref().expect("scene target"); // ensure_scene_target() was called
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
            self.draw_canvas_batches(&mut pass, self.scene_format, &scene_layer.batches, 0);
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
        let scene_size = self.scene_target.as_ref().expect("scene target").size; // ensure_scene_target() was called
        let mut chain: Vec<(ShaderRef, (u32, u32))> = Vec::new();

        // Built-in post effects (run before user passes, at scene resolution).
        let passes = world.render.postfx.passes();
        self.update_postfx_params(&passes);
        chain.extend(passes.iter().map(|(kind, _)| (ShaderRef::Fx(*kind), scene_size)));

        // User-defined post passes.
        for p in &world.render.post {
            let Some(r) = self.asset_shader(Some(p.shader), ShaderKind::Post) else { continue };
            chain.push((r, if p.size == PassSize::Scene { scene_size } else { output }));
        }
        if chain.last().is_none_or(|l| l.1 != output) {
            chain.push((ShaderRef::Copy, output));
        }
        // Pipelines: intermediate passes render to the scene format, the last one to `format`.
        let n = chain.len();
        let mut keep = Vec::with_capacity(n);
        for (i, (r, _)) in chain.iter().enumerate() {
            let f = if i + 1 == n { format } else { self.scene_format };
            keep.push(self.ensure_shader_pipeline(*r, (f, 0, 1)));
        }
        let mut chain: Vec<_> = chain.into_iter().zip(keep).filter(|(_, ok)| *ok).map(|(c, _)| c).collect();
        if chain.last().is_none_or(|l| l.1 != output || !self.shader(l.0).pipelines.get(&(format, 0, 1)).is_some_and(|p| p.is_some())) {
            self.ensure_shader_pipeline(ShaderRef::Copy, (format, 0, 1));
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
                    format: self.scene_format,
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
        let target = self.scene_target.as_ref().expect("scene target"); // ensure_scene_target() was called
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
            pass.set_pipeline(self.pipeline(r, (if last { format } else { self.scene_format }, 0, 1)));
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
fn ensure_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, s: &mut GpuShader, key: PipelineKey) {
    if s.pipelines.contains_key(&key) {
        return;
    }
    let (format, variant, samples) = key;
    let Some(module) = &s.module else {
        s.pipelines.insert(key, None);
        return;
    };
    let transparent = variant & (VARIANT_ALPHA | VARIANT_ADDITIVE) != 0;
    let blend = if variant & VARIANT_ADDITIVE != 0 {
        Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        })
    } else if variant & VARIANT_ALPHA != 0 {
        Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        })
    } else {
        None
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
                cull_mode: if variant & VARIANT_DOUBLE_SIDED != 0 { None } else { Some(wgpu::Face::Back) },
                ..Default::default()
            }
        } else {
            wgpu::PrimitiveState::default()
        },
        depth_stencil: surface.then(|| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(!transparent),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("spark_fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
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
    s.pipelines.insert(key, pipeline);
}



/// Lights that can touch the view (sphere vs frustum), closest first, packed for the shader.
/// Always sorted, ties broken by object index, so the chosen set does not flicker between frames.
fn collect_lights(scene: &spark_core::Scene, mats: &[Mat4], vis: &[bool], planes: &[Vec4; 6]) -> ([LightUniform; MAX_LIGHTS], usize) {
    let cam = scene.camera.position;
    let mut found: Vec<(f32, u32, LightUniform)> = Vec::new();
    for (id, obj) in scene.iter() {
        let Some(light) = obj.light else { continue };
        let i = id.index() as usize;
        if light.intensity <= 0.0 || !vis[i] {
            continue;
        }
        let m = mats[i];
        let pos = m.transform_point3(Vec3::ZERO);
        if sphere_outside(planes, pos, light.range) {
            continue;
        }
        let dir = m.transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Z);
        let c = light.color.to_linear();
        let li = light.intensity;
        let (spot, outer, inner) = match light.kind {
            spark_core::LightKind::Point => (0.0, -2.0, -1.0),
            spark_core::LightKind::Spot { angle, softness } => {
                let half = angle * 0.5;
                (1.0, half.cos(), (half * (1.0 - softness.clamp(0.0, 1.0))).cos())
            }
        };
        let u = LightUniform {
            position: pos.extend(light.range).to_array(),
            color: [c[0] * li, c[1] * li, c[2] * li, spot],
            direction: dir.extend(outer).to_array(),
            params: [inner, 0.0, 0.0, 0.0],
        };
        found.push(((pos.distance(cam) - light.range).max(0.0), id.index(), u));
    }
    if found.len() > MAX_LIGHTS {
        found.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    }
    let mut out = [LightUniform::zeroed(); MAX_LIGHTS];
    let n = found.len().min(MAX_LIGHTS);
    for (slot, (_, _, u)) in out.iter_mut().zip(found) {
        *slot = u;
    }
    (out, n)
}

fn request_adapter(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<wgpu::Adapter, String> {
    let mut tried: Vec<&str> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for (pref, fallback, label) in [
        (wgpu::PowerPreference::HighPerformance, false, "HighPerformance"),
        (wgpu::PowerPreference::LowPower, false, "LowPower"),
        (wgpu::PowerPreference::None, true, "force_fallback_adapter"),
    ] {
        tried.push(label);
        match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: pref,
            force_fallback_adapter: fallback,
            compatible_surface: surface,
            ..Default::default()
        })) {
            Ok(adapter) => {
                let info = adapter.get_info();
                log::info!("GPU adapter selected via {label}: {} ({:?})", info.name, info.backend);
                return Ok(adapter);
            }
            Err(e) => {
                log::warn!("request_adapter {label} failed: {e}");
                errors.push(format!("{label}: {e}"));
            }
        }
    }

    let available: Vec<String> = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
        .iter()
        .map(|a| {
            let info = a.get_info();
            format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type)
        })
        .collect();

    Err(format!(
        "no compatible GPU found.\n\
         Tried: {}\n\
         Errors: {}\n\
         Available GPUs: {}\n\
         Hint: install up-to-date GPU drivers, or try WGPU_BACKEND=gl for the OpenGL backend.",
        tried.join(", "),
        errors.join("; "),
        if available.is_empty() { "none".into() } else { available.join(", ") },
    ))
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

fn object_buffer(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, capacity: usize) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = uniform_buffer(device, "spark objects", capacity as u64);
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("spark objects"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: NonZeroU64::new((MAX_INSTANCES * size_of::<ObjectUniform>()) as u64),
            }),
        }],
    });
    (buffer, bind_group)
}

fn shadow_map(device: &wgpu::Device, size: u32) -> ShadowMap {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("spark shadow map"),
        size: extent((size, size)),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SHADOW_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    ShadowMap { size, view: texture.create_view(&Default::default()), _texture: texture }
}

fn frame_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
    shadow: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    sky: &wgpu::TextureView,
    sky_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("spark frame"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(shadow) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(sky) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(sky_sampler) },
        ],
    })
}

/// Uploads six square faces (+X, -X, +Y, -Y, +Z, -Z) with a full mip chain each.
fn upload_cube(device: &wgpu::Device, queue: &wgpu::Queue, faces: &[ImageData; 6]) -> GpuCube {
    let size = faces[0].width;
    let mut mips = 1;
    let mut data = Vec::new();
    for face in faces {
        let (levels, bytes) = build_mips(face);
        mips = levels;
        data.extend_from_slice(&bytes);
    }
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("spark sky cubemap"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 6 },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &data,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    });
    GpuCube { view, mips }
}

/// Orthographic sun view around the camera, snapped to shadow texels so edges don't shimmer.
fn sun_view_proj(cam: &spark_core::Camera, sun_dir: Vec3, distance: f32, size: u32) -> Mat4 {
    let dir = sun_dir.normalize_or(Vec3::NEG_Y);
    let r = distance.max(1.0) * 0.5;
    let center = cam.position + cam.forward() * r;
    let up = if dir.y.abs() > 0.99 { Vec3::Z } else { Vec3::Y };
    let view = spark_core::glam::camera::rh::view::look_to_mat4(Vec3::ZERO, dir, up);
    let c = view.transform_point3(center);
    let texel = 2.0 * r / size.max(1) as f32;
    let (cx, cy) = ((c.x / texel).floor() * texel, (c.y / texel).floor() * texel);
    // Casters up to 100 m towards the sun (tall buildings, mountains) still cast into the box.
    let proj = spark_core::glam::camera::rh::proj::directx::orthographic(cx - r, cx + r, cy - r, cy + r, -c.z - r - 100.0, -c.z + r);
    proj * view
}

/// Frustum planes (a, b, c, d) of a view-projection matrix (WebGPU depth 0..1), normals inward.
fn frustum_planes(vp: &Mat4) -> [Vec4; 6] {
    let r = [vp.row(0), vp.row(1), vp.row(2), vp.row(3)];
    [r[3] + r[0], r[3] - r[0], r[3] + r[1], r[3] - r[1], r[2], r[3] - r[2]]
}

fn sphere_outside(planes: &[Vec4; 6], center: Vec3, radius: f32) -> bool {
    planes.iter().any(|p| {
        let len = p.truncate().length().max(1e-6);
        (p.truncate().dot(center) + p.w) / len < -radius
    })
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
    normal: &wgpu::TextureView,
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
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(normal) },
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

#[cfg(test)]
mod tests {
    use super::*;
    use spark_core::glam::Quat;
    use spark_core::{Material, MeshData};

    #[test]
    fn uniforms_are_16_byte_aligned() {
        assert_eq!(size_of::<FrameUniform>() % 16, 0);
        assert_eq!(size_of::<ObjectUniform>() % 16, 0);
        // 64 objects per draw must fit the 16 KB uniform binding every backend (incl. GL) offers.
        assert!(size_of::<ObjectUniform>() * MAX_INSTANCES <= 16 * 1024);
    }

    #[test]
    fn normal_matrix_keeps_normals_perpendicular_under_shear() {
        // Child rotated 45° under a parent scaled (3, 1, 1): the world matrix has shear.
        let parent = Mat4::from_scale(Vec3::new(3.0, 1.0, 1.0));
        let child = Mat4::from_quat(Quat::from_rotation_z(std::f32::consts::FRAC_PI_4));
        let model = parent * child;
        let nm = normal_matrix(&model);
        // A surface with local normal +Y contains the local +X direction.
        let tangent = model.transform_vector3(Vec3::X);
        let normal = nm.transform_vector3(Vec3::Y).normalize();
        assert!(tangent.dot(normal).abs() < 1e-5, "normal not perpendicular: {}", tangent.dot(normal));
    }

    #[test]
    fn normal_matrix_degenerate_is_identity() {
        assert_eq!(normal_matrix(&Mat4::from_scale(Vec3::ZERO)), Mat4::IDENTITY);
    }

    /// Real GPU round trip: a white unlit cube in front of the camera ends up in the image.
    /// Skipped when the machine has no GPU / software Vulkan (CI installs lavapipe).
    #[test]
    fn headless_render_draws_scene() {
        let mut r = match Renderer::new_headless(64, 64) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skipping GPU test: {e}");
                return;
            }
        };
        let mut world = World::default();
        world.scene.background = Color::BLACK;
        let cube = world.add_mesh(MeshData::cube(2.0));
        world.spawn(cube, Material::unlit(Color::WHITE));
        world.scene.camera.position = Vec3::new(0.0, 0.0, 4.0);
        world.scene.camera.look_at(Vec3::ZERO);
        let image = r.screenshot(&world).expect("screenshot");
        assert_eq!((image.width, image.height), (64, 64));
        let center = image.pixel(32, 32);
        let corner = image.pixel(0, 0);
        assert!(center[0] > 200, "cube not drawn: center = {center:?}");
        assert!(corner[0] < 30, "background not cleared: corner = {corner:?}");
        assert!(r.last_stats().draw_calls >= 1);
        assert!(!r.is_device_lost());
    }

    fn test_world() -> World {
        let mut world = World::default();
        world.scene.background = Color::BLACK;
        let cube = world.add_mesh(MeshData::cube(2.0));
        // 4x brighter than white: only an HDR scene keeps that until tonemapping.
        world.spawn(cube, Material::unlit(Color::rgb(4.0, 4.0, 4.0)));
        world.scene.camera.position = Vec3::new(0.0, 0.0, 4.0);
        world.scene.camera.look_at(Vec3::ZERO);
        world
    }

    /// All ten built-in effects together render without GPU validation errors.
    #[test]
    fn builtin_effects_render() {
        use spark_core::render::*;
        let Ok(mut r) = Renderer::new_headless(96, 64) else { return eprintln!("skipping GPU test: no adapter") };
        let mut world = test_world();
        world.render.postfx = PostProcessing {
            tonemap: Tonemap::Aces,
            bloom: Some(Bloom::default()),
            ssao: Some(Ssao::default()),
            motion_blur: Some(MotionBlur::default()),
            dof: Some(DepthOfField::default()),
            color_grading: Some(ColorGrading::default()),
            fxaa: true,
            chromatic_aberration: Some(ChromaticAberration::default()),
            film_grain: Some(FilmGrain::default()),
            vignette: Some(Vignette::default()),
        };
        let scope = r.device.push_error_scope(wgpu::ErrorFilter::Validation);
        for frame in 1..=3 {
            world.time.frame = frame;
            world.scene.camera.position.x = frame as f32 * 0.1; // moving camera: motion blur path
            r.screenshot(&world).expect("screenshot");
        }
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "GPU validation error: {error:?}");
        assert_eq!(r.last_stats().post_passes, 10);
    }

    /// PBR (metal + rough dielectric), normal maps, emissive and mesh LOD render without GPU
    /// validation errors, with and without MSAA.
    #[test]
    fn pbr_normal_map_and_lod_render() {
        use spark_core::ImageData;
        let Ok(mut r) = Renderer::new_headless(64, 64) else { return eprintln!("skipping GPU test: no adapter") };
        let mut world = World::default();
        world.scene.background = Color::BLACK;
        let normal = world.assets.add_texture(ImageData::checker(16, 4, Color::rgb(0.5, 0.5, 1.0), Color::rgb(0.7, 0.5, 0.8)));
        let base = world.assets.add_texture(ImageData::checker(16, 2, Color::WHITE, Color::GRAY));
        let sphere = world.add_mesh(MeshData::sphere(0.8, 16));
        let cube = world.add_mesh(MeshData::cube(1.0));
        let metal = Material::pbr(Color::WHITE, 1.0, 0.2).with_normal_map(Some(normal));
        let a = world.spawn(sphere, metal);
        world.scene.get_mut(a).unwrap().position = Vec3::new(-1.0, 0.0, 0.0);
        let mut plastic = Material::pbr(Color::RED, 0.0, 0.8).with_normal_map(Some(normal));
        plastic.texture = Some(base);
        let b = world.spawn(cube, plastic);
        world.scene.get_mut(b).unwrap().position = Vec3::new(1.0, 0.0, 0.0);
        let classic = Material::color(Color::GREEN).with_normal_map(Some(normal));
        let c = world.spawn(cube, classic);
        world.scene.get_mut(c).unwrap().position = Vec3::new(0.0, 1.0, 0.0);
        world.scene.camera.position = Vec3::new(0.0, 0.0, 5.0);
        world.scene.camera.look_at(Vec3::ZERO);
        let scope = r.device.push_error_scope(wgpu::ErrorFilter::Validation);
        for samples in [4, 1] {
            world.render.msaa = samples;
            r.screenshot(&world).expect("screenshot");
        }
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "GPU validation error: {error:?}");
    }

    /// Emissive light shows on a black surface in the dark; past its LOD distance an object
    /// draws its LOD mesh instead (here a small cube that leaves the edge pixels empty).
    #[test]
    fn emissive_and_lod_switch() {
        let Ok(mut r) = Renderer::new_headless(64, 64) else { return eprintln!("skipping GPU test: no adapter") };
        let mut world = World::default();
        world.scene.background = Color::BLACK;
        world.scene.ambient = Color::BLACK;
        world.scene.sun.intensity = 0.0;
        let big = world.add_mesh(MeshData::cube(2.0));
        let tiny = world.add_mesh(MeshData::cube(0.5));
        let id = world.spawn(big, Material::pbr(Color::BLACK, 0.0, 1.0).with_emissive(Color::rgb(1.0, 0.0, 0.0)));
        world.scene.camera.position = Vec3::new(0.0, 0.0, 4.0);
        world.scene.camera.look_at(Vec3::ZERO);
        let image = r.screenshot(&world).expect("screenshot");
        let (center, edge) = (image.pixel(32, 32), image.pixel(20, 32));
        assert!(center[0] > 200 && center[1] < 40, "emissive not drawn: {center:?}");
        assert!(edge[0] > 200, "full mesh covers the edge: {edge:?}");
        world.scene.get_mut(id).unwrap().lod = vec![(3.0, tiny)];
        let image = r.screenshot(&world).expect("screenshot");
        assert!(image.pixel(32, 32)[0] > 200, "LOD mesh drawn");
        assert!(image.pixel(20, 32)[0] < 30, "LOD mesh replaces the full mesh: {:?}", image.pixel(20, 32));
    }

    /// Gradient sky: up shows `top`, down shows `bottom`; with a cubemap the face in view shows.
    #[test]
    fn sky_gradient_and_cubemap() {
        use spark_core::{ImageData, Sky};
        let Ok(mut r) = Renderer::new_headless(32, 32) else { return eprintln!("skipping GPU test: no adapter") };
        let mut world = World::default();
        world.scene.background = Color::BLACK;
        world.scene.sky = Some(Sky::gradient(Color::rgb(0.0, 0.0, 1.0), Color::rgb(0.0, 1.0, 0.0)));
        world.scene.camera.position = Vec3::ZERO;
        world.scene.camera.look_at(Vec3::new(0.0, 10.0, -0.01)); // straight up
        let up = r.screenshot(&world).expect("screenshot").pixel(16, 16);
        world.scene.camera.look_at(Vec3::new(0.0, -10.0, -0.01)); // straight down
        let down = r.screenshot(&world).expect("screenshot").pixel(16, 16);
        assert!(up[2] > 200 && up[1] < 60, "up = {up:?}");
        assert!(down[1] > 200 && down[2] < 60, "down = {down:?}");

        // Faces: +X red, -X black, +Y black, -Y black, +Z black, -Z white (the camera looks along -Z).
        let colors = [Color::rgb(1.0, 0.0, 0.0), Color::BLACK, Color::BLACK, Color::BLACK, Color::BLACK, Color::WHITE];
        let cube = world.assets.add_cubemap(colors.map(|c| ImageData::new(4, 4, c.to_rgba8().repeat(16)))).unwrap();
        world.scene.sky = Some(Sky::cubemap(cube));
        let scope = r.device.push_error_scope(wgpu::ErrorFilter::Validation);
        for samples in [4, 1] {
            world.render.msaa = samples;
            world.scene.camera.look_at(Vec3::new(0.0, 0.0, -10.0));
            let fwd = r.screenshot(&world).expect("screenshot").pixel(16, 16);
            assert!(fwd.iter().take(3).all(|&c| c > 200), "-Z face: {fwd:?}");
            world.scene.camera.look_at(Vec3::new(10.0, 0.0, 0.0));
            let right = r.screenshot(&world).expect("screenshot").pixel(16, 16);
            assert!(right[0] > 200 && right[1] < 60, "+X face: {right:?}");
        }
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "GPU validation error: {error:?}");
    }

    /// Sky lighting: in a scene with no ambient color and no sun, a white diffuse cube is lit by
    /// the sky only when `lighting` is above 0, and a metal reflects the sky.
    #[test]
    fn sky_lights_the_scene() {
        use spark_core::Sky;
        let Ok(mut r) = Renderer::new_headless(32, 32) else { return eprintln!("skipping GPU test: no adapter") };
        let mut world = World::default();
        world.scene.ambient = Color::BLACK;
        world.scene.sun.intensity = 0.0;
        let cube = world.add_mesh(MeshData::cube(2.0));
        let id = world.spawn(cube, Material::color(Color::WHITE));
        world.scene.camera.position = Vec3::new(0.0, 0.0, 4.0);
        world.scene.camera.look_at(Vec3::ZERO);
        let sky = Sky::gradient(Color::rgb(0.8, 0.8, 0.8), Color::rgb(0.8, 0.8, 0.8));
        world.scene.sky = Some(sky);
        let dark = r.screenshot(&world).expect("screenshot").pixel(16, 16);
        assert!(dark[0] < 20, "lighting 0 keeps the flat (black) ambient: {dark:?}");
        world.scene.sky = Some(sky.with_lighting(1.0));
        let lit = r.screenshot(&world).expect("screenshot").pixel(16, 16);
        assert!(lit[0] > 150, "sky lights the cube: {lit:?}");
        world.scene.get_mut(id).unwrap().material = Material::pbr(Color::WHITE, 1.0, 0.1);
        let metal = r.screenshot(&world).expect("screenshot").pixel(16, 16);
        assert!(metal[0] > 150, "metal reflects the sky: {metal:?}");
    }

    /// Values above 1 reach the tonemapper: Reinhard x / (1 + x) stays above 0.5 (sRGB 188) only
    /// if the scene kept x > 1; an 8-bit scene clips to 1.0 first and gives exactly ~188.
    #[test]
    fn hdr_scene_reaches_tonemapper() {
        use spark_core::render::Tonemap;
        let Ok(mut r) = Renderer::new_headless(64, 64) else { return eprintln!("skipping GPU test: no adapter") };
        if r.scene_format != HDR_FORMAT {
            return eprintln!("skipping: adapter has no HDR render target");
        }
        let mut world = test_world();
        world.render.postfx.tonemap = Tonemap::Reinhard;
        let center = r.screenshot(&world).expect("screenshot").pixel(32, 32);
        assert!((200..=252).contains(&center[0]), "center = {center:?}");
    }
}
