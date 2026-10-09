use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::color::Color;
use crate::mesh::{Aabb, MeshData, Vertex};
use crate::model::{ModelData, ModelId};
use crate::shader::{ShaderData, ShaderId};

/// Handle to a mesh stored in [`Assets`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshId(pub u32);

/// Handle to a texture stored in [`Assets`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(pub u32);

/// RGBA8 image in sRGB space.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl ImageData {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        assert_eq!(pixels.len(), (width * height * 4) as usize, "ImageData: pixels must be width*height*4 bytes");
        Self { width, height, pixels }
    }

    /// 1x1 image of one color.
    pub fn solid(color: Color) -> Self {
        Self::new(1, 1, color.to_rgba8().to_vec())
    }

    /// Checkerboard: `size` pixels wide, `cells` squares per side.
    pub fn checker(size: u32, cells: u32, a: Color, b: Color) -> Self {
        let size = size.max(1);
        let cell = (size / cells.max(1)).max(1);
        let (a, b) = (a.to_rgba8(), b.to_rgba8());
        let mut pixels = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                pixels.extend(if ((x / cell) + (y / cell)) % 2 == 0 { a } else { b });
            }
        }
        Self::new(size, size, pixels)
    }

    /// Loads a PNG or JPEG file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = crate::vfs::read(path)?;
        let img = image::load_from_memory(&bytes).map_err(|e| format!("cannot load image '{}': {e}", path.display()))?.to_rgba8();
        Ok(Self { width: img.width(), height: img.height(), pixels: img.into_raw() })
    }

    /// Saves as PNG, creating parent folders.
    pub fn save_png(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create '{}': {e}", dir.display()))?;
        }
        image::save_buffer_with_format(
            path,
            &self.pixels,
            self.width,
            self.height,
            image::ExtendedColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .map_err(|e| format!("cannot save '{}': {e}", path.display()))
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.pixels[i], self.pixels[i + 1], self.pixels[i + 2], self.pixels[i + 3]]
    }
}

/// All meshes and textures of a game. Pure data: the renderer uploads new entries automatically.
#[derive(Debug, Default)]
pub struct Assets {
    meshes: Vec<(MeshData, Aabb)>,
    /// Bumped when a mesh is edited in place (renderer re-uploads it).
    mesh_versions: Vec<u64>,
    mesh_edits: u64,
    models: Vec<ModelData>,
    model_paths: HashMap<PathBuf, ModelId>,
    textures: Vec<ImageData>,
    texture_paths: HashMap<PathBuf, TextureId>,
    shaders: Vec<ShaderData>,
    shader_paths: HashMap<PathBuf, ShaderId>,
    generation: u64,
    /// Base folder for relative asset paths.
    pub root: PathBuf,
}

impl Assets {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_mesh(&mut self, mesh: MeshData) -> MeshId {
        let bounds = mesh.bounds();
        self.meshes.push((mesh, bounds));
        self.mesh_versions.push(0);
        MeshId(self.meshes.len() as u32 - 1)
    }

    pub fn mesh(&self, id: MeshId) -> Option<&MeshData> {
        self.meshes.get(id.0 as usize).map(|(m, _)| m)
    }

    pub fn mesh_bounds(&self, id: MeshId) -> Aabb {
        self.meshes.get(id.0 as usize).map(|(_, b)| *b).unwrap_or(Aabb::EMPTY)
    }

    /// Replaces a mesh's vertices (same count, e.g. skinning). The renderer re-uploads it.
    pub fn update_mesh_vertices(&mut self, id: MeshId, vertices: Vec<Vertex>) {
        let i = id.0 as usize;
        if let Some((m, b)) = self.meshes.get_mut(i) {
            m.vertices = vertices;
            *b = m.bounds();
            self.mesh_versions[i] += 1;
            self.mesh_edits += 1;
        }
    }

    /// Edit counter of one mesh (see [`Assets::update_mesh_vertices`]).
    pub fn mesh_version(&self, id: MeshId) -> u64 {
        self.mesh_versions.get(id.0 as usize).copied().unwrap_or(0)
    }

    /// Total number of in-place mesh edits so far.
    pub fn mesh_edits(&self) -> u64 {
        self.mesh_edits
    }

    /// Loads a glTF model (`.glb` / `.gltf`) once; later calls with the same path return the same id.
    pub fn load_model(&mut self, path: &str) -> Result<ModelId, String> {
        let full = self.resolve(path);
        if let Some(&id) = self.model_paths.get(&full) {
            return Ok(id);
        }
        let model = ModelData::load(&full, self)?;
        self.models.push(model);
        let id = ModelId(self.models.len() as u32 - 1);
        self.model_paths.insert(full, id);
        Ok(id)
    }

    pub fn model(&self, id: ModelId) -> Option<&ModelData> {
        self.models.get(id.0 as usize)
    }

    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    pub fn add_texture(&mut self, image: ImageData) -> TextureId {
        self.textures.push(image);
        TextureId(self.textures.len() as u32 - 1)
    }

    /// Loads an image file once; later calls with the same path return the same id.
    pub fn load_texture(&mut self, path: &str) -> Result<TextureId, String> {
        let full = self.resolve(path);
        if let Some(&id) = self.texture_paths.get(&full) {
            return Ok(id);
        }
        let id = self.add_texture(ImageData::load(&full)?);
        self.texture_paths.insert(full, id);
        Ok(id)
    }

    pub fn texture(&self, id: TextureId) -> Option<&ImageData> {
        self.textures.get(id.0 as usize)
    }

    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// Compiles WGSL code (see [`crate::shader`]).
    pub fn add_shader(&mut self, name: &str, code: &str) -> Result<ShaderId, String> {
        self.shaders.push(ShaderData::compile(name, code)?);
        Ok(ShaderId(self.shaders.len() as u32 - 1))
    }

    /// Loads and compiles a `.wgsl` file once; later calls with the same path return the same id.
    pub fn load_shader(&mut self, path: &str) -> Result<ShaderId, String> {
        let full = self.resolve(path);
        if let Some(&id) = self.shader_paths.get(&full) {
            return Ok(id);
        }
        let code = crate::vfs::read_string(&full).map_err(|e| format!("cannot read shader: {e}"))?;
        let id = self.add_shader(path, &code)?;
        self.shader_paths.insert(full, id);
        Ok(id)
    }

    pub fn shader(&self, id: ShaderId) -> Option<&ShaderData> {
        self.shaders.get(id.0 as usize)
    }

    pub fn shader_mut(&mut self, id: ShaderId) -> Option<&mut ShaderData> {
        self.shaders.get_mut(id.0 as usize)
    }

    pub fn shader_count(&self) -> usize {
        self.shaders.len()
    }

    /// Shaders loaded from files (for hot reload).
    pub fn shader_files(&self) -> impl Iterator<Item = (&Path, ShaderId)> {
        self.shader_paths.iter().map(|(p, id)| (p.as_path(), *id))
    }

    /// Resolves a path relative to [`Assets::root`].
    pub fn resolve(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() { p.to_path_buf() } else { self.root.join(p) }
    }

    /// Removes everything. Old ids become invalid.
    pub fn clear(&mut self) {
        self.meshes.clear();
        self.mesh_versions.clear();
        self.models.clear();
        self.model_paths.clear();
        self.textures.clear();
        self.texture_paths.clear();
        self.shaders.clear();
        self.shader_paths.clear();
        self.generation += 1;
    }

    /// Changes every time [`Assets::clear`] is called (renderer uses it to drop GPU copies).
    pub fn generation(&self) -> u64 {
        self.generation
    }
}
