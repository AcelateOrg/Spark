//! 3D models: glTF 2.0 (`.glb` / `.gltf`) loading - meshes, base-color materials, the node hierarchy,
//! skins and animation clips. Spawning and playback live in [`crate::animation`].

use std::path::Path;

use glam::{Mat4, Quat, Vec3, Vec4};

use crate::assets::{Assets, ImageData, MeshId, TextureId};
use crate::color::Color;
use crate::material::Material;
use crate::mesh::{Aabb, MeshData, Vertex};

/// Handle to a model stored in [`Assets`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModelId(pub u32);

/// One node of the model hierarchy (a bone, a mesh holder or an empty).
#[derive(Clone, Debug)]
pub struct ModelNode {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    /// Index into [`ModelData::meshes`].
    pub mesh: Option<usize>,
    /// Index into [`ModelData::skins`].
    pub skin: Option<usize>,
}

/// Per-vertex bone influences of a skinned primitive (4 bones per vertex).
#[derive(Clone, Debug, Default)]
pub struct SkinWeights {
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
}

/// Part of a mesh with one material.
#[derive(Clone, Debug)]
pub struct Primitive {
    /// Bind-pose mesh (shared by every instance).
    pub mesh: MeshId,
    pub material: Material,
    pub skin: Option<SkinWeights>,
}

#[derive(Clone, Debug, Default)]
pub struct Skin {
    /// Joint node indices.
    pub joints: Vec<usize>,
    pub inverse_bind: Vec<Mat4>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Property {
    Translation,
    Rotation,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Step,
    Linear,
    /// Hermite spline: 3 values per key (in-tangent, value, out-tangent).
    Cubic,
}

/// Keyframes of one node property. Vectors use xyz, rotations are quaternions (xyzw).
#[derive(Clone, Debug)]
pub struct Channel {
    pub node: usize,
    pub property: Property,
    pub interpolation: Interpolation,
    pub times: Vec<f32>,
    pub values: Vec<Vec4>,
}

impl Channel {
    /// Value at time `t` (clamped to the key range).
    pub fn sample(&self, t: f32) -> Vec4 {
        let n = self.times.len();
        let cubic = self.interpolation == Interpolation::Cubic;
        let value = |i: usize| if cubic { self.values[i * 3 + 1] } else { self.values[i] };
        if n == 0 || self.values.is_empty() {
            return Vec4::ZERO;
        }
        if n == 1 || t <= self.times[0] {
            return value(0);
        }
        if t >= self.times[n - 1] {
            return value(n - 1);
        }
        let i = self.times.partition_point(|&k| k <= t).saturating_sub(1).min(n - 2);
        let (t0, t1) = (self.times[i], self.times[i + 1]);
        let dt = (t1 - t0).max(1e-6);
        let k = ((t - t0) / dt).clamp(0.0, 1.0);
        let rot = self.property == Property::Rotation;
        match self.interpolation {
            Interpolation::Step => value(i),
            Interpolation::Linear => {
                let (a, b) = (value(i), value(i + 1));
                if rot { slerp(a, b, k) } else { a.lerp(b, k) }
            }
            Interpolation::Cubic => {
                let (p0, m0) = (self.values[i * 3 + 1], self.values[i * 3 + 2] * dt);
                let (p1, m1) = (self.values[(i + 1) * 3 + 1], self.values[(i + 1) * 3] * dt);
                let (k2, k3) = (k * k, k * k * k);
                let v = p0 * (2.0 * k3 - 3.0 * k2 + 1.0)
                    + m0 * (k3 - 2.0 * k2 + k)
                    + p1 * (-2.0 * k3 + 3.0 * k2)
                    + m1 * (k3 - k2);
                if rot { Quat::from_vec4(v).normalize().into() } else { v }
            }
        }
    }
}

fn slerp(a: Vec4, b: Vec4, k: f32) -> Vec4 {
    Quat::from_vec4(a).normalize().slerp(Quat::from_vec4(b).normalize(), k).into()
}

/// An animation (e.g. "walk").
#[derive(Clone, Debug)]
pub struct Clip {
    pub name: String,
    /// Seconds.
    pub duration: f32,
    pub channels: Vec<Channel>,
}

/// A loaded model.
#[derive(Clone, Debug, Default)]
pub struct ModelData {
    pub name: String,
    pub nodes: Vec<ModelNode>,
    /// Top-level nodes of the default scene.
    pub roots: Vec<usize>,
    pub meshes: Vec<Vec<Primitive>>,
    pub skins: Vec<Skin>,
    pub clips: Vec<Clip>,
    /// Bind-pose bounds in model space.
    pub bounds: Aabb,
}

impl ModelData {
    pub fn clip(&self, name: &str) -> Option<usize> {
        self.clips.iter().position(|c| c.name == name)
    }

    pub fn clip_names(&self) -> Vec<String> {
        self.clips.iter().map(|c| c.name.clone()).collect()
    }

    pub fn node(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    /// Model-space bind matrix of a node.
    pub fn node_matrix(&self, mut node: usize) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        for _ in 0..128 {
            let n = &self.nodes[node];
            m = Mat4::from_scale_rotation_translation(n.scale, n.rotation, n.translation) * m;
            match n.parent {
                Some(p) => node = p,
                None => break,
            }
        }
        m
    }

    /// Loads a `.glb` / `.gltf` file, adding its meshes and textures to `assets`.
    pub fn load(path: &Path, assets: &mut Assets) -> Result<Self, String> {
        let err = |e: String| format!("cannot load model '{}': {e}", path.display());
        let bytes = crate::vfs::read(path).map_err(err)?;
        let gltf = gltf::Gltf::from_slice(&bytes).map_err(|e| err(e.to_string()))?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut buffers: Vec<Vec<u8>> = Vec::new();
        for b in gltf.buffers() {
            let data = match b.source() {
                gltf::buffer::Source::Bin => gltf.blob.clone().ok_or_else(|| err("missing GLB binary chunk".into()))?,
                gltf::buffer::Source::Uri(uri) => read_uri(dir, uri).map_err(err)?,
            };
            if data.len() < b.length() {
                return Err(err(format!("buffer {} is too short", b.index())));
            }
            buffers.push(data);
        }
        let get = |b: gltf::Buffer| buffers.get(b.index()).map(|v| v.as_slice());
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut model = ModelData { name, bounds: Aabb::EMPTY, ..Default::default() };

        // Images (decoded lazily: only those used as base color).
        let mut images: Vec<Option<TextureId>> = vec![None; gltf.images().len()];
        let mut image = |index: usize, assets: &mut Assets| -> Result<TextureId, String> {
            if let Some(id) = images[index] {
                return Ok(id);
            }
            let img = gltf.images().nth(index).ok_or("bad image index")?;
            let data = match img.source() {
                gltf::image::Source::View { view, .. } => {
                    let buf = &buffers[view.buffer().index()];
                    buf.get(view.offset()..view.offset() + view.length()).ok_or("image view out of range")?.to_vec()
                }
                gltf::image::Source::Uri { uri, .. } => read_uri(dir, uri)?,
            };
            let rgba = image::load_from_memory(&data).map_err(|e| format!("image {index}: {e}"))?.to_rgba8();
            let id = assets.add_texture(ImageData::new(rgba.width(), rgba.height(), rgba.into_raw()));
            images[index] = Some(id);
            Ok(id)
        };

        for mesh in gltf.meshes() {
            let mut prims = Vec::new();
            for p in mesh.primitives() {
                if p.mode() != gltf::mesh::Mode::Triangles {
                    log::warn!("{}: skipping non-triangle primitive", model.name);
                    continue;
                }
                let r = p.reader(get);
                let Some(positions) = r.read_positions() else { continue };
                let positions: Vec<[f32; 3]> = positions.collect();
                let uvs: Vec<[f32; 2]> =
                    r.read_tex_coords(0).map(|t| t.into_f32().collect()).unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);
                let indices: Vec<u32> =
                    r.read_indices().map(|i| i.into_u32().collect()).unwrap_or_else(|| (0..positions.len() as u32).collect());
                let normals: Vec<[f32; 3]> = match r.read_normals() {
                    Some(n) => n.collect(),
                    None => smooth_normals(&positions, &indices),
                };
                let vertices = (0..positions.len())
                    .map(|i| Vertex {
                        position: positions[i],
                        normal: normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]),
                        uv: uvs.get(i).copied().unwrap_or([0.0, 0.0]),
                    })
                    .collect();
                let skin = match (r.read_joints(0), r.read_weights(0)) {
                    (Some(j), Some(w)) => {
                        let joints: Vec<[u16; 4]> = j.into_u16().collect();
                        let weights: Vec<[f32; 4]> = w
                            .into_f32()
                            .map(|w| {
                                let s = w[0] + w[1] + w[2] + w[3];
                                if s > 0.0 { w.map(|x| x / s) } else { [1.0, 0.0, 0.0, 0.0] }
                            })
                            .collect();
                        (joints.len() == positions.len() && weights.len() == positions.len())
                            .then_some(SkinWeights { joints, weights })
                    }
                    _ => None,
                };
                let m = p.material();
                let pbr = m.pbr_metallic_roughness();
                let f = pbr.base_color_factor();
                let mut material = Material {
                    color: Color::rgba(to_srgb(f[0]), to_srgb(f[1]), to_srgb(f[2]), f[3]),
                    unlit: m.unlit(),
                    double_sided: m.double_sided(),
                    blend: match m.alpha_mode() {
                        gltf::material::AlphaMode::Blend => crate::material::BlendMode::Alpha,
                        _ => crate::material::BlendMode::Opaque,
                    },
                    ..Default::default()
                };
                if let Some(info) = pbr.base_color_texture() {
                    match image(info.texture().source().index(), assets) {
                        Ok(id) => material.texture = Some(id),
                        Err(e) => log::warn!("{}: {e}", model.name),
                    }
                }
                let mesh_id = assets.add_mesh(MeshData::new(vertices, indices));
                prims.push(Primitive { mesh: mesh_id, material, skin });
            }
            model.meshes.push(prims);
        }

        for node in gltf.nodes() {
            let (t, r, s) = node.transform().decomposed();
            model.nodes.push(ModelNode {
                name: node.name().map(str::to_owned).unwrap_or_else(|| format!("node{}", node.index())),
                parent: None,
                children: node.children().map(|c| c.index()).collect(),
                translation: Vec3::from_array(t),
                rotation: Quat::from_array(r).normalize(),
                scale: Vec3::from_array(s),
                mesh: node.mesh().map(|m| m.index()),
                skin: node.skin().map(|s| s.index()),
            });
        }
        for i in 0..model.nodes.len() {
            for c in model.nodes[i].children.clone() {
                if let Some(n) = model.nodes.get_mut(c) {
                    n.parent = Some(i);
                }
            }
        }
        model.roots = match gltf.default_scene().or_else(|| gltf.scenes().next()) {
            Some(scene) => scene.nodes().map(|n| n.index()).collect(),
            None => (0..model.nodes.len()).filter(|&i| model.nodes[i].parent.is_none()).collect(),
        };

        for skin in gltf.skins() {
            let joints: Vec<usize> = skin.joints().map(|j| j.index()).collect();
            let inverse_bind: Vec<Mat4> = match skin.reader(get).read_inverse_bind_matrices() {
                Some(m) => m.map(|c| Mat4::from_cols_array_2d(&c)).collect(),
                None => vec![Mat4::IDENTITY; joints.len()],
            };
            model.skins.push(Skin { joints, inverse_bind });
        }

        for (ai, anim) in gltf.animations().enumerate() {
            let mut clip = Clip {
                name: anim.name().map(str::to_owned).unwrap_or_else(|| format!("animation{ai}")),
                duration: 0.0,
                channels: Vec::new(),
            };
            for ch in anim.channels() {
                let r = ch.reader(get);
                let (Some(inputs), Some(outputs)) = (r.read_inputs(), r.read_outputs()) else { continue };
                use gltf::animation::util::ReadOutputs;
                let (property, values): (Property, Vec<Vec4>) = match outputs {
                    ReadOutputs::Translations(v) => (Property::Translation, v.map(|p| Vec3::from_array(p).extend(0.0)).collect()),
                    ReadOutputs::Scales(v) => (Property::Scale, v.map(|p| Vec3::from_array(p).extend(0.0)).collect()),
                    ReadOutputs::Rotations(v) => (Property::Rotation, v.into_f32().map(Vec4::from_array).collect()),
                    ReadOutputs::MorphTargetWeights(_) => continue,
                };
                let interpolation = match ch.sampler().interpolation() {
                    gltf::animation::Interpolation::Step => Interpolation::Step,
                    gltf::animation::Interpolation::Linear => Interpolation::Linear,
                    gltf::animation::Interpolation::CubicSpline => Interpolation::Cubic,
                };
                let times: Vec<f32> = inputs.collect();
                let need = if interpolation == Interpolation::Cubic { times.len() * 3 } else { times.len() };
                if times.is_empty() || values.len() < need {
                    continue;
                }
                clip.duration = clip.duration.max(*times.last().unwrap_or(&0.0));
                clip.channels.push(Channel { node: ch.target().node().index(), property, interpolation, times, values });
            }
            model.clips.push(clip);
        }

        // Bind-pose bounds (skinned meshes use their own vertex positions, like most viewers).
        for (i, n) in model.nodes.iter().enumerate() {
            if let Some(m) = n.mesh {
                let matrix = if n.skin.is_some() { Mat4::IDENTITY } else { model.node_matrix(i) };
                for p in &model.meshes[m] {
                    model.bounds = model.bounds.union(&assets.mesh_bounds(p.mesh).transform(&matrix));
                }
            }
        }
        Ok(model)
    }
}

fn to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

fn smooth_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut n = vec![Vec3::ZERO; positions.len()];
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        if a >= n.len() || b >= n.len() || c >= n.len() {
            continue;
        }
        let (pa, pb, pc) = (Vec3::from(positions[a]), Vec3::from(positions[b]), Vec3::from(positions[c]));
        let f = (pb - pa).cross(pc - pa);
        n[a] += f;
        n[b] += f;
        n[c] += f;
    }
    n.into_iter().map(|v| v.normalize_or(Vec3::Y).to_array()).collect()
}

/// Reads an external file or a `data:...;base64,` URI.
fn read_uri(dir: &Path, uri: &str) -> Result<Vec<u8>, String> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let (_, data) = rest.split_once(";base64,").ok_or("unsupported data URI")?;
        return base64_decode(data).ok_or_else(|| "bad base64 data".into());
    }
    let rel = safe_relative(&percent_decode(uri)).ok_or_else(|| {
        format!("refusing external file '{uri}': model files may only reference files next to / below the model")
    })?;
    crate::vfs::read(&dir.join(rel))
}

/// A relative path that stays inside its base folder (no absolute paths, drive letters, URL
/// schemes or `..` climbing out). A downloaded model cannot read arbitrary files.
fn safe_relative(uri: &str) -> Option<std::path::PathBuf> {
    use std::path::Component;
    if uri.contains("://") || uri.starts_with('/') || uri.starts_with('\\') || uri.get(1..2) == Some(":") {
        return None;
    }
    let mut out = std::path::PathBuf::new();
    for c in Path::new(&uri.replace('\\', "/")).components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\r' | b'\n' | b' ' => continue,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn uri_sandbox() {
        use super::{percent_decode, safe_relative};
        assert_eq!(safe_relative("textures/a.png").unwrap(), std::path::PathBuf::from("textures/a.png"));
        assert_eq!(safe_relative("./x/../b.bin").unwrap(), std::path::PathBuf::from("b.bin"));
        for bad in ["../secret.txt", "/etc/passwd", "C:/Windows/win.ini", "a/../../b", "http://x/y.png", "\\\\server\\share", ""] {
            assert!(safe_relative(bad).is_none(), "{bad}");
        }
        assert_eq!(percent_decode("a%20b%20"), "a b ");
    }

    use super::*;

    #[test]
    fn base64() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(percent_decode("a%20b.bin"), "a b.bin");
    }

    #[test]
    fn channel_sampling() {
        let ch = Channel {
            node: 0,
            property: Property::Translation,
            interpolation: Interpolation::Linear,
            times: vec![0.0, 1.0, 2.0],
            values: vec![Vec4::ZERO, Vec4::new(2.0, 0.0, 0.0, 0.0), Vec4::new(2.0, 4.0, 0.0, 0.0)],
        };
        assert_eq!(ch.sample(0.5).x, 1.0);
        assert_eq!(ch.sample(1.5).y, 2.0);
        assert_eq!(ch.sample(9.0).y, 4.0);
        assert_eq!(ch.sample(-1.0).x, 0.0);
    }
}
