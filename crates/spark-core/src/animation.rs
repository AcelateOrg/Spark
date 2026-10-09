//! Model instances in the scene: spawning glTF models as object trees, playing / blending animation
//! clips and CPU skinning. [`update`] runs every frame between `update` and `late_update`.

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3, Vec4};

use crate::assets::MeshId;
use crate::material::Material;
use crate::model::{ModelId, Property};
use crate::scene::{Object, ObjectId};
use crate::world::World;

/// Options for [`play`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimParams {
    pub looping: bool,
    pub speed: f32,
    /// Cross-fade time from the current animation (seconds).
    pub fade: f32,
    /// Start from the beginning even if this clip is already playing.
    pub restart: bool,
}

impl Default for AnimParams {
    fn default() -> Self {
        Self { looping: true, speed: 1.0, fade: 0.2, restart: false }
    }
}

#[derive(Clone, Debug)]
struct Track {
    clip: usize,
    time: f32,
    speed: f32,
    looping: bool,
    weight: f32,
    target: f32,
    /// Weight change per second.
    rate: f32,
}

#[derive(Clone, Debug)]
struct SkinnedPart {
    object: ObjectId,
    /// Node that holds the mesh (its world transform is cancelled, as glTF requires).
    node_object: ObjectId,
    skin: usize,
    mesh_index: usize,
    prim: usize,
    /// This instance's own deformed copy of the mesh.
    mesh: MeshId,
}

/// A spawned model.
#[derive(Clone, Debug)]
pub struct Instance {
    pub model: ModelId,
    /// Scene object of every model node (same order as `ModelData::nodes`).
    pub nodes: Vec<ObjectId>,
    tracks: Vec<Track>,
    parts: Vec<SkinnedPart>,
    /// Multiplies every track's speed (0 = frozen).
    pub speed: f32,
    skinned_once: bool,
}

/// All model instances of the world.
#[derive(Debug, Default)]
pub struct Animator {
    instances: HashMap<ObjectId, Instance>,
    /// Deformed mesh copies of destroyed instances, reused by new ones: (model, mesh, prim) -> meshes.
    spare: HashMap<(ModelId, usize, usize), Vec<MeshId>>,
}

impl Animator {
    pub fn clear(&mut self) {
        self.instances.clear();
        self.spare.clear();
    }

    pub fn instance(&self, root: ObjectId) -> Option<&Instance> {
        self.instances.get(&root)
    }

    pub fn len(&self) -> usize {
        self.instances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }
}

/// Spawns a model as a tree of objects. Returns the root object (named after the file).
/// `material` replaces every part's material.
pub fn spawn_model(world: &mut World, model_id: ModelId, material: Option<Material>) -> Option<ObjectId> {
    let model = world.assets.model(model_id)?.clone();
    let mut root_obj = Object::empty();
    root_obj.name = model.name.clone();
    let root = world.scene.add(root_obj);
    let mut nodes = vec![root; model.nodes.len()];
    let mut parts = Vec::new();
    // Create in hierarchy order so parents exist first.
    let mut stack: Vec<(usize, ObjectId)> = model.roots.iter().rev().map(|&n| (n, root)).collect();
    let mut seen = vec![false; model.nodes.len()];
    while let Some((ni, parent)) = stack.pop() {
        if ni >= model.nodes.len() || seen[ni] {
            continue;
        }
        seen[ni] = true;
        let n = &model.nodes[ni];
        let mut o = Object::empty();
        o.name = n.name.clone();
        o.position = n.translation;
        o.rotation = n.rotation;
        o.scale = n.scale;
        let id = world.scene.add(o);
        let _ = world.scene.set_parent(id, Some(parent));
        nodes[ni] = id;
        if let Some(mi) = n.mesh {
            for (pi, p) in model.meshes[mi].iter().enumerate() {
                let target = if pi == 0 {
                    id
                } else {
                    let mut child = Object::empty();
                    child.name = format!("{}.{pi}", n.name);
                    let c = world.scene.add(child);
                    let _ = world.scene.set_parent(c, Some(id));
                    c
                };
                let skinned = n.skin.filter(|_| p.skin.is_some());
                let mesh = match skinned {
                    Some(_) => {
                        let reuse = world.animator.spare.get_mut(&(model_id, mi, pi)).and_then(Vec::pop);
                        match reuse {
                            Some(m) => m,
                            None => {
                                let data = world.assets.mesh(p.mesh).cloned().unwrap_or_default();
                                world.assets.add_mesh(data)
                            }
                        }
                    }
                    None => p.mesh,
                };
                world.set_mesh(target, mesh);
                world.scene[target].material = material.unwrap_or(p.material);
                if let Some(skin) = skinned {
                    parts.push(SkinnedPart { object: target, node_object: id, skin, mesh_index: mi, prim: pi, mesh });
                }
            }
        }
        for &c in n.children.iter().rev() {
            stack.push((c, id));
        }
    }
    if !model.clips.is_empty() || !parts.is_empty() {
        let inst = Instance { model: model_id, nodes, tracks: Vec::new(), parts, speed: 1.0, skinned_once: false };
        world.animator.instances.insert(root, inst);
        if !model.clips.is_empty() {
            // Show the first frame of the first clip (usually a rest / idle pose) without playing it.
            skin_and_pose(world, root, true);
        }
    }
    Some(root)
}

/// Starts (or cross-fades to) a clip on a model instance.
pub fn play(world: &mut World, root: ObjectId, clip: &str, p: AnimParams) -> Result<(), String> {
    let model_id = world.animator.instances.get(&root).ok_or("play: object is not a spawned model with animations")?.model;
    let model = world.assets.model(model_id).ok_or("play: model was unloaded")?;
    let index = model.clip(clip).ok_or_else(|| {
        let names = model.clip_names();
        if names.is_empty() {
            "play: this model has no animations".to_string()
        } else {
            format!("play: no animation '{clip}' (available: {})", names.join(", "))
        }
    })?;
    let inst = world.animator.instances.get_mut(&root).expect("checked above");
    let fade = p.fade.max(0.0);
    let existing = inst.tracks.iter().position(|t| t.clip == index);
    if let Some(i) = existing {
        let t = &mut inst.tracks[i];
        t.speed = p.speed;
        t.looping = p.looping;
        if p.restart {
            t.time = if p.speed < 0.0 { model.clips[index].duration } else { 0.0 };
        }
    }
    for (i, t) in inst.tracks.iter_mut().enumerate() {
        let on = Some(i) == existing;
        t.target = if on { 1.0 } else { 0.0 };
        if fade <= 0.0 {
            t.weight = t.target;
        }
        t.rate = if fade > 0.0 { 1.0 / fade } else { f32::INFINITY };
    }
    if existing.is_none() {
        let start = if p.speed < 0.0 { model.clips[index].duration } else { 0.0 };
        inst.tracks.push(Track {
            clip: index,
            time: start,
            speed: p.speed,
            looping: p.looping,
            weight: if fade > 0.0 && !inst.tracks.is_empty() { 0.0 } else { 1.0 },
            target: 1.0,
            rate: if fade > 0.0 { 1.0 / fade } else { f32::INFINITY },
        });
    }
    inst.tracks.retain(|t| t.weight > 0.0 || t.target > 0.0);
    Ok(())
}

/// Fades out every clip (the pose stays where it was when weights reach zero... the bind pose shows through).
pub fn stop(world: &mut World, root: ObjectId, fade: f32) {
    if let Some(inst) = world.animator.instances.get_mut(&root) {
        for t in &mut inst.tracks {
            t.target = 0.0;
            t.rate = if fade > 0.0 { 1.0 / fade } else { f32::INFINITY };
            if fade <= 0.0 {
                t.weight = 0.0;
            }
        }
        inst.tracks.retain(|t| t.weight > 0.0);
    }
}

/// Speed multiplier for all clips of an instance (0 freezes the pose).
pub fn set_speed(world: &mut World, root: ObjectId, speed: f32) {
    if let Some(inst) = world.animator.instances.get_mut(&root) {
        inst.speed = speed;
    }
}

/// Name of the clip fading in / playing at full weight, if any.
pub fn current(world: &World, root: ObjectId) -> Option<String> {
    let inst = world.animator.instances.get(&root)?;
    let model = world.assets.model(inst.model)?;
    inst.tracks.iter().rev().find(|t| t.target > 0.0).map(|t| model.clips[t.clip].name.clone())
}

/// Is `clip` (or any clip with `None`) still running? Finished one-shot clips count as stopped.
pub fn is_playing(world: &World, root: ObjectId, clip: Option<&str>) -> bool {
    let Some(inst) = world.animator.instances.get(&root) else { return false };
    let Some(model) = world.assets.model(inst.model) else { return false };
    inst.tracks.iter().any(|t| {
        let c = &model.clips[t.clip];
        let running = t.target > 0.0 && (t.looping || (if t.speed >= 0.0 { t.time < c.duration } else { t.time > 0.0 }));
        running && clip.is_none_or(|n| c.name == n)
    })
}

/// Playback time (seconds) of the current clip.
pub fn time(world: &World, root: ObjectId) -> f32 {
    world
        .animator
        .instances
        .get(&root)
        .and_then(|i| i.tracks.iter().rev().find(|t| t.target > 0.0))
        .map(|t| t.time)
        .unwrap_or(0.0)
}

/// Advances all instances, poses their node objects and deforms skinned meshes.
pub fn update(world: &mut World, dt: f32) {
    if world.animator.instances.is_empty() {
        return;
    }
    // Forget destroyed instances (keep their mesh copies for reuse).
    let dead: Vec<ObjectId> = world.animator.instances.keys().copied().filter(|&r| !world.scene.contains(r)).collect();
    for r in dead {
        if let Some(inst) = world.animator.instances.remove(&r) {
            for p in inst.parts {
                world.animator.spare.entry((inst.model, p.mesh_index, p.prim)).or_default().push(p.mesh);
            }
        }
    }
    let roots: Vec<ObjectId> = world.animator.instances.keys().copied().collect();
    for root in roots {
        let Some(inst) = world.animator.instances.get_mut(&root) else { continue };
        let Some(model) = world.assets.model(inst.model) else { continue };
        let speed = inst.speed;
        for t in &mut inst.tracks {
            let d = model.clips[t.clip].duration;
            t.time += dt * t.speed * speed;
            if t.looping && d > 0.0 {
                t.time = t.time.rem_euclid(d);
            } else {
                t.time = t.time.clamp(0.0, d);
            }
            let step = if t.rate.is_finite() { t.rate * dt } else { 1.0 };
            if t.weight < t.target {
                t.weight = (t.weight + step).min(t.target);
            } else if t.weight > t.target {
                t.weight = (t.weight - step).max(t.target);
            }
        }
        inst.tracks.retain(|t| t.weight > 0.0 || t.target > 0.0);
        let active = !inst.tracks.is_empty() && speed != 0.0 || !inst.skinned_once;
        if active {
            skin_and_pose(world, root, false);
        }
    }
}

fn skin_and_pose(world: &mut World, root: ObjectId, first_clip_pose: bool) {
    let Some(inst) = world.animator.instances.get(&root) else { return };
    let Some(model) = world.assets.model(inst.model) else { return };

    // 1. Pose: blend tracks over the bind pose for every animated node.
    let mut tracks: Vec<(usize, f32, f32)> = inst.tracks.iter().map(|t| (t.clip, t.time, t.weight)).collect();
    if first_clip_pose && tracks.is_empty() && !model.clips.is_empty() {
        tracks.push((0, 0.0, 1.0));
    }
    let mut pose: HashMap<usize, (Vec3, Quat, Vec3)> = HashMap::new();
    let total: f32 = tracks.iter().map(|t| t.2).sum();
    if total > 0.0 {
        let mut animated: Vec<usize> = Vec::new();
        for &(c, _, _) in &tracks {
            for ch in &model.clips[c].channels {
                if !animated.contains(&ch.node) {
                    animated.push(ch.node);
                }
            }
        }
        for &node in &animated {
            let Some(n) = model.nodes.get(node) else { continue };
            let bind = (n.translation, n.rotation, n.scale);
            let mut acc = bind;
            let mut acc_w = 0.0;
            for &(c, time, w) in &tracks {
                if w <= 0.0 {
                    continue;
                }
                let mut v = bind;
                for ch in model.clips[c].channels.iter().filter(|ch| ch.node == node) {
                    let s = ch.sample(time);
                    match ch.property {
                        Property::Translation => v.0 = s.truncate(),
                        Property::Rotation => v.1 = Quat::from_vec4(s).normalize(),
                        Property::Scale => v.2 = s.truncate(),
                    }
                }
                acc_w += w;
                let k = w / acc_w;
                acc = (acc.0.lerp(v.0, k), nlerp(acc.1, v.1, k), acc.2.lerp(v.2, k));
            }
            if total < 1.0 {
                acc = (bind.0.lerp(acc.0, total), nlerp(bind.1, acc.1, total), bind.2.lerp(acc.2, total));
            }
            pose.insert(node, acc);
        }
    }
    let nodes = inst.nodes.clone();
    let parts = inst.parts.clone();
    let model_id = inst.model;
    for (node, (t, r, s)) in pose {
        if let Some(o) = nodes.get(node).and_then(|&id| world.scene.get_mut(id)) {
            o.position = t;
            o.rotation = r;
            o.scale = s;
        }
    }

    // 2. Skinning on the CPU.
    let Some(model) = world.assets.model(model_id) else { return };
    let mut jobs = Vec::new();
    for p in &parts {
        let skin = &model.skins[p.skin];
        let inv_node = world.scene.world_matrix(p.node_object).inverse();
        let joints: Vec<Mat4> = skin
            .joints
            .iter()
            .enumerate()
            .map(|(j, &node)| {
                let jw = nodes.get(node).map(|&id| world.scene.world_matrix(id)).unwrap_or(Mat4::IDENTITY);
                inv_node * jw * skin.inverse_bind.get(j).copied().unwrap_or(Mat4::IDENTITY)
            })
            .collect();
        let prim = &model.meshes[p.mesh_index][p.prim];
        let (Some(base), Some(weights)) = (world.assets.mesh(prim.mesh), prim.skin.as_ref()) else { continue };
        let mut vertices = base.vertices.clone();
        for (i, v) in vertices.iter_mut().enumerate() {
            let (js, ws) = (weights.joints[i], weights.weights[i]);
            let mut m = Mat4::ZERO;
            for k in 0..4 {
                if ws[k] > 0.0 {
                    m += joints.get(js[k] as usize).copied().unwrap_or(Mat4::IDENTITY) * ws[k];
                }
            }
            let pos = m * Vec4::new(v.position[0], v.position[1], v.position[2], 1.0);
            let nrm = m * Vec4::new(v.normal[0], v.normal[1], v.normal[2], 0.0);
            v.position = pos.truncate().to_array();
            v.normal = nrm.truncate().normalize_or(Vec3::Y).to_array();
        }
        jobs.push((p.object, p.mesh, vertices));
    }
    for (object, mesh, vertices) in jobs {
        world.assets.update_mesh_vertices(mesh, vertices);
        let b = world.assets.mesh_bounds(mesh);
        if let Some(o) = world.scene.get_mut(object) {
            o.local_bounds = b;
        }
    }
    if let Some(inst) = world.animator.instances.get_mut(&root) {
        inst.skinned_once = true;
    }
}

fn nlerp(a: Quat, b: Quat, k: f32) -> Quat {
    let b = if a.dot(b) < 0.0 { -b } else { b };
    a.lerp(b, k).normalize()
}
