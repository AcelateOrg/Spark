use std::fmt::Write as _;

use glam::{EulerRot, Mat4, Quat, Vec3};

use crate::assets::MeshId;
use crate::camera::Camera;
use crate::color::Color;
use crate::material::Material;
use crate::math::look_rotation;
use crate::mesh::Aabb;
use crate::physics::Body;

/// Handle to an object in a [`Scene`]. Stays valid until the object is destroyed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId {
    index: u32,
    generation: u32,
}

impl ObjectId {
    pub fn index(self) -> u32 {
        self.index
    }

    /// Unique 64-bit value (index + generation), stable while the object lives.
    pub fn to_bits(self) -> u64 {
        ((self.generation as u64) << 32) | self.index as u64
    }

    /// Inverse of [`ObjectId::to_bits`]. The id may refer to a destroyed object.
    pub fn from_bits(bits: u64) -> Self {
        Self { index: bits as u32, generation: (bits >> 32) as u32 }
    }
}

/// Something in the world: a transform plus an optional mesh.
#[derive(Clone, Debug)]
pub struct Object {
    pub name: String,
    /// Position relative to the parent (or the world if no parent).
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub mesh: Option<MeshId>,
    pub material: Material,
    pub visible: bool,
    /// Rigid body simulated by the physics backend (`None` = not physical).
    pub body: Option<Body>,
    /// Light emitted from the object's position (spot lights shine along -Z).
    pub light: Option<Light>,
    pub(crate) parent: Option<ObjectId>,
    /// Direct children (kept in sync by `Scene::set_parent` / `destroy`).
    pub(crate) children: Vec<ObjectId>,
    pub(crate) local_bounds: Aabb,
    /// Fixed-step interpolation state of simulated bodies (see [`Interp`]).
    pub(crate) interp: Option<Interp>,
}

/// Local pose before and after the last fixed (physics) step. The renderer draws the object
/// in between (`Time::alpha`) so motion is smooth on any refresh rate. Teleports (game code
/// changing `position` / `rotation`) are detected and drawn without interpolation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interp {
    pub prev_position: Vec3,
    pub prev_rotation: Quat,
    pub position: Vec3,
    pub rotation: Quat,
}

impl Object {
    /// Empty object (a group / pivot).
    pub fn empty() -> Self {
        Self {
            name: String::new(),
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            mesh: None,
            material: Material::default(),
            visible: true,
            body: None,
            light: None,
            parent: None,
            children: Vec::new(),
            local_bounds: Aabb::EMPTY,
            interp: None,
        }
    }

    /// Direct children of this object.
    pub fn child_ids(&self) -> &[ObjectId] {
        &self.children
    }

    /// Local matrix drawn this frame: interpolated between the last two fixed steps for
    /// simulated bodies (`alpha` 0..1), the plain local matrix otherwise.
    pub fn render_local_matrix(&self, alpha: f32) -> Mat4 {
        if let Some(i) = &self.interp {
            if i.position == self.position && i.rotation == self.rotation {
                let p = i.prev_position.lerp(i.position, alpha);
                let r = i.prev_rotation.slerp(i.rotation, alpha);
                return Mat4::from_scale_rotation_translation(self.scale, r, p);
            }
        }
        self.local_matrix()
    }

    pub fn parent(&self) -> Option<ObjectId> {
        self.parent
    }

    /// Mesh bounds in the object's own space (no transform applied).
    pub fn local_bounds(&self) -> Aabb {
        self.local_bounds
    }

    pub fn local_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position)
    }

    /// Rotates around the object's own X axis.
    pub fn rotate_x(&mut self, angle: f32) {
        self.rotation = (self.rotation * Quat::from_rotation_x(angle)).normalize();
    }

    /// Rotates around the object's own Y axis.
    pub fn rotate_y(&mut self, angle: f32) {
        self.rotation = (self.rotation * Quat::from_rotation_y(angle)).normalize();
    }

    /// Rotates around the object's own Z axis.
    pub fn rotate_z(&mut self, angle: f32) {
        self.rotation = (self.rotation * Quat::from_rotation_z(angle)).normalize();
    }

    pub fn translate(&mut self, delta: Vec3) {
        self.position += delta;
    }

    /// Euler angles in radians (x = pitch, y = yaw, z = roll), applied in Y-X-Z order.
    pub fn euler(&self) -> Vec3 {
        let (y, x, z) = self.rotation.to_euler(EulerRot::YXZ);
        Vec3::new(x, y, z)
    }

    pub fn set_euler(&mut self, angles: Vec3) {
        self.rotation = Quat::from_euler(EulerRot::YXZ, angles.y, angles.x, angles.z);
    }

    /// Turns the object so its -Z axis points at `target` (in parent space).
    pub fn look_at(&mut self, target: Vec3) {
        if target != self.position {
            self.rotation = look_rotation(target - self.position);
        }
    }

    /// Local -Z axis.
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }
}

/// Kind of a [`Light`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightKind {
    /// Shines in all directions.
    Point,
    /// Cone along the object's -Z. `angle` = full cone angle in radians, `softness` 0..1 = blurry edge.
    Spot { angle: f32, softness: f32 },
}

/// Point or spot light attached to an object. Reaches exactly `range` meters (smooth falloff).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    pub color: Color,
    pub intensity: f32,
    pub range: f32,
}

impl Light {
    pub const MAX_VISIBLE: usize = 32;

    pub fn point(color: Color, intensity: f32, range: f32) -> Self {
        Self { kind: LightKind::Point, color, intensity, range }
    }

    pub fn spot(color: Color, intensity: f32, range: f32, angle: f32, softness: f32) -> Self {
        Self { kind: LightKind::Spot { angle, softness }, color, intensity, range }
    }
}

/// Directional light (the sun).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// Direction the light travels (points away from the sun).
    pub direction: Vec3,
    pub color: Color,
    pub intensity: f32,
}

/// Linear distance fog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fog {
    pub color: Color,
    pub near: f32,
    pub far: f32,
}

#[derive(Clone, Debug)]
struct Slot {
    generation: u32,
    object: Option<Object>,
}

/// All objects plus camera and lighting.
#[derive(Clone, Debug)]
pub struct Scene {
    slots: Vec<Slot>,
    free: Vec<u32>,
    count: usize,
    pub camera: Camera,
    pub sun: Sun,
    pub ambient: Color,
    pub background: Color,
    pub fog: Option<Fog>,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            count: 0,
            camera: Camera::default(),
            sun: Sun { direction: Vec3::new(-0.4, -1.0, -0.3), color: Color::WHITE, intensity: 1.0 },
            ambient: Color::rgb(0.35, 0.38, 0.45),
            background: Color::rgb(0.08, 0.09, 0.12),
            fog: None,
        }
    }

    /// Adds an object. Prefer `World::spawn`, which also sets mesh bounds.
    pub fn add(&mut self, mut object: Object) -> ObjectId {
        self.count += 1;
        object.children.clear();
        let parent = object.parent.filter(|p| self.contains(*p));
        object.parent = parent;
        let id = if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.object = Some(object);
            ObjectId { index, generation: slot.generation }
        } else {
            self.slots.push(Slot { generation: 0, object: Some(object) });
            ObjectId { index: self.slots.len() as u32 - 1, generation: 0 }
        };
        if let Some(p) = parent {
            self[p].children.push(id);
        }
        id
    }

    /// Removes an object and all its children. Returns false if it did not exist.
    pub fn destroy(&mut self, id: ObjectId) -> bool {
        let Some(parent) = self.get(id).map(|o| o.parent) else { return false };
        if let Some(p) = parent.and_then(|p| self.get_mut(p)) {
            p.children.retain(|c| *c != id);
        }
        // Iterative: deep hierarchies cannot overflow the stack. O(size of the subtree).
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            let slot = &mut self.slots[cur.index as usize];
            if slot.generation != cur.generation {
                continue;
            }
            let Some(o) = slot.object.take() else { continue };
            stack.extend(o.children);
            slot.generation = slot.generation.wrapping_add(1);
            self.free.push(cur.index);
            self.count -= 1;
        }
        true
    }

    /// Removes all objects (camera and lighting stay). Every old [`ObjectId`] becomes invalid.
    pub fn clear_objects(&mut self) {
        self.free.clear();
        for (i, slot) in self.slots.iter_mut().enumerate() {
            slot.object = None;
            slot.generation = slot.generation.wrapping_add(1);
            self.free.push(i as u32);
        }
        self.free.reverse();
        self.count = 0;
    }

    pub fn contains(&self, id: ObjectId) -> bool {
        self.get(id).is_some()
    }

    pub fn get(&self, id: ObjectId) -> Option<&Object> {
        self.slots.get(id.index as usize).filter(|s| s.generation == id.generation).and_then(|s| s.object.as_ref())
    }

    pub fn get_mut(&mut self, id: ObjectId) -> Option<&mut Object> {
        self.slots.get_mut(id.index as usize).filter(|s| s.generation == id.generation).and_then(|s| s.object.as_mut())
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = (ObjectId, &Object)> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.object.as_ref().map(|o| (ObjectId { index: i as u32, generation: s.generation }, o))
        })
    }

    /// First object with this name.
    pub fn find(&self, name: &str) -> Option<ObjectId> {
        self.iter().find(|(_, o)| o.name == name).map(|(id, _)| id)
    }

    pub fn children(&self, id: ObjectId) -> Vec<ObjectId> {
        self.get(id).map(|o| o.children.clone()).unwrap_or_default()
    }

    /// Called before every fixed step: remembers the pose of every moving body.
    pub fn begin_interp_step(&mut self) {
        for slot in &mut self.slots {
            let Some(o) = slot.object.as_mut() else { continue };
            let moving = o.body.as_ref().is_some_and(|b| b.kind != crate::physics::BodyKind::Static);
            o.interp = moving.then_some(Interp {
                prev_position: o.position,
                prev_rotation: o.rotation,
                position: o.position,
                rotation: o.rotation,
            });
        }
    }

    /// Called after every fixed step (physics included): stores the new pose of moving bodies.
    pub fn end_interp_step(&mut self) {
        for slot in &mut self.slots {
            let Some(o) = slot.object.as_mut() else { continue };
            if let Some(i) = &mut o.interp {
                i.position = o.position;
                i.rotation = o.rotation;
            }
        }
    }

    /// Number of slots (live + free). Object indices are `0..capacity()`.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// World matrix and effective visibility of every object, indexed by `ObjectId::index`, in
    /// one O(n) pass (parents first). Simulated bodies are interpolated by `alpha` (see
    /// [`Object::render_local_matrix`]). The renderer calls this once per frame.
    pub fn compute_world_transforms(&self, alpha: f32, matrices: &mut Vec<Mat4>, visible: &mut Vec<bool>) {
        let n = self.slots.len();
        matrices.clear();
        matrices.resize(n, Mat4::IDENTITY);
        visible.clear();
        visible.resize(n, false);
        let mut stack: Vec<(u32, Mat4, bool)> = Vec::with_capacity(64);
        for (i, slot) in self.slots.iter().enumerate() {
            let Some(o) = &slot.object else { continue };
            if o.parent.is_some() {
                continue;
            }
            stack.push((i as u32, Mat4::IDENTITY, true));
            while let Some((idx, parent_m, parent_vis)) = stack.pop() {
                let Some(o) = self.slots[idx as usize].object.as_ref() else { continue };
                let m = parent_m * o.render_local_matrix(alpha);
                let vis = parent_vis && o.visible;
                matrices[idx as usize] = m;
                visible[idx as usize] = vis;
                for c in &o.children {
                    stack.push((c.index, m, vis));
                }
            }
        }
    }

    /// Attaches `child` to `parent` (keeps the child's local transform), or detaches with `None`.
    pub fn set_parent(&mut self, child: ObjectId, parent: Option<ObjectId>) -> Result<(), String> {
        if !self.contains(child) {
            return Err("set_parent: child object does not exist".into());
        }
        if let Some(p) = parent {
            if !self.contains(p) {
                return Err("set_parent: parent object does not exist".into());
            }
            let mut cur = Some(p);
            while let Some(c) = cur {
                if c == child {
                    return Err("set_parent: would create a cycle (parent is a child of this object)".into());
                }
                cur = self.get(c).and_then(|o| o.parent);
            }
        }
        let old = self[child].parent;
        if old == parent {
            return Ok(());
        }
        if let Some(o) = old.and_then(|o| self.get_mut(o)) {
            o.children.retain(|c| *c != child);
        }
        if let Some(p) = parent {
            self[p].children.push(child);
        }
        self[child].parent = parent;
        Ok(())
    }

    /// Object-to-world matrix including all parents.
    pub fn world_matrix(&self, id: ObjectId) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        let mut cur = Some(id);
        let mut depth = 0;
        while let Some(c) = cur {
            let Some(o) = self.get(c) else { break };
            m = o.local_matrix() * m;
            cur = o.parent;
            depth += 1;
            if depth > self.slots.len() {
                break; // cycle guard (set_parent prevents cycles)
            }
        }
        m
    }

    pub fn world_position(&self, id: ObjectId) -> Vec3 {
        self.world_matrix(id).transform_point3(Vec3::ZERO)
    }

    /// World-space bounds of the object's own mesh (children not included).
    pub fn world_bounds(&self, id: ObjectId) -> Aabb {
        match self.get(id) {
            Some(o) => o.local_bounds.transform(&self.world_matrix(id)),
            None => Aabb::EMPTY,
        }
    }

    /// Visible itself and all parents visible.
    pub fn is_visible(&self, id: ObjectId) -> bool {
        let mut cur = Some(id);
        while let Some(c) = cur {
            match self.get(c) {
                Some(o) if o.visible => cur = o.parent,
                _ => return false,
            }
        }
        true
    }

    /// Moves the object by a world-space offset (works with parents).
    pub fn translate_world(&mut self, id: ObjectId, delta: Vec3) {
        let local = match self.get(id).and_then(|o| o.parent) {
            Some(p) => self.world_matrix(p).inverse().transform_vector3(delta),
            None => delta,
        };
        if let Some(o) = self.get_mut(id) {
            o.position += local;
        }
    }

    /// Moves the object up/down so its bottom rests on top of `target`. X/Z stay the same.
    pub fn place_on(&mut self, id: ObjectId, target: ObjectId) -> Result<(), String> {
        let top = self.world_bounds(target);
        if top.is_empty() {
            return Err("place_on: target has no mesh (no bounds)".into());
        }
        if !self.contains(id) {
            return Err("place_on: object does not exist".into());
        }
        let own = self.world_bounds(id);
        let bottom = if own.is_empty() { self.world_position(id).y } else { own.min.y };
        self.translate_world(id, Vec3::new(0.0, top.max.y - bottom, 0.0));
        Ok(())
    }

    /// Machine readable dump (JSON) of the camera and every object: for tests, CI and AI tools.
    pub fn dump_json(&self) -> String {
        fn esc(s: &str) -> String {
            let mut o = String::with_capacity(s.len() + 2);
            for ch in s.chars() {
                match ch {
                    '"' => o.push_str("\\\""),
                    '\\' => o.push_str("\\\\"),
                    c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                    c => o.push(c),
                }
            }
            o
        }
        fn v(x: Vec3) -> String {
            let f = |n: f32| if n.is_finite() { format!("{n:.4}") } else { "null".into() };
            format!("[{},{},{}]", f(x.x), f(x.y), f(x.z))
        }
        let c = &self.camera;
        let mut out = format!(
            "{{\"camera\":{{\"position\":{},\"forward\":{},\"fov\":{:.2},\"ortho\":{}}},\"objects\":[",
            v(c.position),
            v(c.forward()),
            c.fov,
            c.ortho.map_or("null".into(), |o| format!("{o:.3}"))
        );
        let (mut mats, mut vis) = (Vec::new(), Vec::new());
        self.compute_world_transforms(1.0, &mut mats, &mut vis);
        for (n, (id, o)) in self.iter().enumerate() {
            let i = id.index() as usize;
            let b = o.local_bounds.transform(&mats[i]);
            let e = o.euler();
            let _ = write!(
                out,
                "{}{{\"id\":{},\"name\":\"{}\",\"parent\":{},\"mesh\":{},\"position\":{},\"world_position\":{},\"rotation_deg\":{},\"scale\":{},\"visible\":{},\"bounds\":{},\"body\":{},\"light\":{}}}",
                if n > 0 { "," } else { "" },
                id.index(),
                esc(&o.name),
                o.parent.map_or("null".into(), |p| p.index().to_string()),
                o.mesh.map_or("null".into(), |m| m.0.to_string()),
                v(o.position),
                v(mats[i].w_axis.truncate()),
                v(Vec3::new(e.x.to_degrees(), e.y.to_degrees(), e.z.to_degrees())),
                v(o.scale),
                vis[i],
                if b.is_empty() { "null".into() } else { format!("[{},{}]", v(b.min), v(b.max)) },
                o.body.as_ref().map_or("null".into(), |b| format!("{{\"kind\":\"{}\",\"velocity\":{}}}", b.kind.name(), v(b.velocity))),
                o.light.is_some()
            );
        }
        out.push_str("]}");
        out
    }

    /// Human/AI readable dump of the whole scene.
    pub fn dump(&self) -> String {
        let mut out = String::new();
        let c = &self.camera;
        let _ = writeln!(
            out,
            "Scene: {} objects | camera pos {} forward {} fov {:.0}",
            self.count,
            fmt_v(c.position),
            fmt_v(c.forward()),
            c.fov
        );
        for (id, o) in self.iter().filter(|(_, o)| o.parent.is_none()) {
            self.dump_object(&mut out, id, o, 0);
        }
        out
    }

    fn dump_object(&self, out: &mut String, id: ObjectId, o: &Object, depth: usize) {
        let name = if o.name.is_empty() { format!("#{}", id.index) } else { format!("\"{}\" #{}", o.name, id.index) };
        let e = o.euler();
        let b = self.world_bounds(id);
        let bounds = if b.is_empty() { "none".to_string() } else { format!("{}..{}", fmt_v(b.min), fmt_v(b.max)) };
        let _ = writeln!(
            out,
            "{}- {} mesh={} pos {} rot(deg) {} scale {} world-bounds {}{}",
            "  ".repeat(depth),
            name,
            o.mesh.map(|m| m.0.to_string()).unwrap_or_else(|| "-".into()),
            fmt_v(o.position),
            fmt_v(Vec3::new(e.x.to_degrees(), e.y.to_degrees(), e.z.to_degrees())),
            fmt_v(o.scale),
            bounds,
            if o.visible { "" } else { " HIDDEN" }
        );
        if let Some(l) = &o.light {
            let kind = match l.kind {
                LightKind::Point => "point".to_string(),
                LightKind::Spot { angle, .. } => format!("spot {:.0}deg", angle.to_degrees()),
            };
            let _ = writeln!(out, "{}  light: {kind} range {:.1} intensity {:.2}", "  ".repeat(depth), l.range, l.intensity);
        }
        for child in self.children(id) {
            if let Some(co) = self.get(child) {
                self.dump_object(out, child, co, depth + 1);
            }
        }
    }
}

fn fmt_v(v: Vec3) -> String {
    format!("({:.2}, {:.2}, {:.2})", v.x, v.y, v.z)
}

impl std::ops::Index<ObjectId> for Scene {
    type Output = Object;
    fn index(&self, id: ObjectId) -> &Object {
        self.get(id).unwrap_or_else(|| panic!("object #{} does not exist (destroyed?)", id.index))
    }
}

impl std::ops::IndexMut<ObjectId> for Scene {
    fn index_mut(&mut self, id: ObjectId) -> &mut Object {
        self.get_mut(id).unwrap_or_else(|| panic!("object #{} does not exist (destroyed?)", id.index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::Body;

    #[test]
    fn clear_objects_invalidates_ids() {
        let mut s = Scene::new();
        let a = s.add(Object::empty());
        s.clear_objects();
        assert!(!s.contains(a));
        let b = s.add(Object::empty());
        assert_eq!(b.index(), a.index());
        assert!(!s.contains(a) && s.contains(b));
    }

    #[test]
    fn hierarchy_index_and_destroy() {
        let mut s = Scene::new();
        let root = s.add(Object::empty());
        let mut last = root;
        for _ in 0..10_000 {
            let mut o = Object::empty();
            o.position = Vec3::X;
            o.parent = Some(last);
            last = s.add(o);
        }
        assert_eq!(s.children(root).len(), 1);
        let (mut m, mut v) = (Vec::new(), Vec::new());
        s.compute_world_transforms(1.0, &mut m, &mut v);
        assert!((m[last.index() as usize].w_axis.x - 10_000.0).abs() < 1e-2);
        assert!((s.world_position(last).x - m[last.index() as usize].w_axis.x).abs() < 1e-2);
        let other = s.add(Object::empty());
        s.set_parent(last, Some(other)).unwrap();
        assert_eq!(s.children(other), vec![last]);
        assert!(s.destroy(root)); // deep chain: no stack overflow
        assert_eq!(s.len(), 2);
        let json = s.dump_json();
        assert!(json.starts_with("{\"camera\"") && json.ends_with("]}"));
        assert!(s.destroy(other));
        assert!(s.is_empty());
    }

    #[test]
    fn interpolation_and_teleport() {
        let mut s = Scene::new();
        let mut o = Object::empty();
        o.body = Some(Body::dynamic());
        let id = s.add(o);
        s.begin_interp_step();
        s[id].position = Vec3::new(1.0, 0.0, 0.0); // what physics would do
        s.end_interp_step();
        let half = s[id].render_local_matrix(0.5).w_axis.x;
        assert!((half - 0.5).abs() < 1e-5);
        s[id].position = Vec3::new(10.0, 0.0, 0.0); // teleport by game code: no smear
        assert_eq!(s[id].render_local_matrix(0.5).w_axis.x, 10.0);
    }
}
