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
    pub(crate) local_bounds: Aabb,
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
            local_bounds: Aabb::EMPTY,
        }
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
    pub fn add(&mut self, object: Object) -> ObjectId {
        self.count += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.object = Some(object);
            ObjectId { index, generation: slot.generation }
        } else {
            self.slots.push(Slot { generation: 0, object: Some(object) });
            ObjectId { index: self.slots.len() as u32 - 1, generation: 0 }
        }
    }

    /// Removes an object and all its children. Returns false if it did not exist.
    pub fn destroy(&mut self, id: ObjectId) -> bool {
        if !self.contains(id) {
            return false;
        }
        let children: Vec<ObjectId> = self.iter().filter(|(_, o)| o.parent == Some(id)).map(|(c, _)| c).collect();
        for child in children {
            self.destroy(child);
        }
        let slot = &mut self.slots[id.index as usize];
        slot.object = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.index);
        self.count -= 1;
        true
    }

    /// Removes all objects (camera and lighting stay).
    pub fn clear_objects(&mut self) {
        self.slots.clear();
        self.free.clear();
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
        self.iter().filter(|(_, o)| o.parent == Some(id)).map(|(c, _)| c).collect()
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
            if depth > 64 {
                break;
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
