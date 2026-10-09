//! Physics data. The simulation itself lives in a backend (`spark-physics`, rapier) so the core stays
//! dependency-free. Without a backend bodies simply don't move.

use glam::{Quat, Vec3};

use crate::scene::ObjectId;
use crate::world::World;

/// How a body moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BodyKind {
    /// Moved by the simulation (gravity, collisions, impulses).
    #[default]
    Dynamic,
    /// Never moves on its own (ground, walls). You can still teleport it.
    Static,
    /// Moved only by your code (position / rotation); pushes dynamic bodies.
    Kinematic,
}

impl BodyKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "dynamic" => Some(Self::Dynamic),
            "static" | "fixed" => Some(Self::Static),
            "kinematic" => Some(Self::Kinematic),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Dynamic => "dynamic",
            Self::Static => "static",
            Self::Kinematic => "kinematic",
        }
    }
}

/// Collision shape in the object's local space (multiplied by the object's scale).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Shape {
    /// Derived from the mesh: box / sphere / cylinder for primitives, triangle mesh (static,
    /// kinematic) or convex hull (dynamic) for custom meshes.
    #[default]
    Auto,
    /// Box with full size (width, height, depth).
    Box(Vec3),
    Sphere(f32),
    /// Capsule along Y. `height` is the total height including the caps.
    Capsule { radius: f32, height: f32 },
    Cylinder { radius: f32, height: f32 },
}

/// A rigid body attached to an object (`Object::body`).
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub kind: BodyKind,
    pub shape: Shape,
    /// Total mass in kg. `None` = computed from volume (density 1).
    pub mass: Option<f32>,
    pub friction: f32,
    /// 0 = no bounce, 1 = perfectly elastic.
    pub bounciness: f32,
    /// Sensors detect overlaps (collision events) but don't push anything.
    pub sensor: bool,
    /// Keeps the body upright (characters).
    pub lock_rotation: bool,
    pub gravity_scale: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// Continuous collision detection for fast objects (bullets).
    pub ccd: bool,
    /// World-space velocity (m/s). Written back by the simulation; set it to change it.
    pub velocity: Vec3,
    /// World-space angular velocity (rad/s).
    pub angular_velocity: Vec3,
    /// Impulse applied at the next physics step, then reset.
    pub impulse: Vec3,
    /// Angular impulse applied at the next physics step, then reset.
    pub torque_impulse: Vec3,
    /// Force applied during the next physics step, then reset (call every fixed_update).
    pub force: Vec3,
}

impl Default for Body {
    fn default() -> Self {
        Self::new(BodyKind::Dynamic)
    }
}

impl Body {
    pub fn new(kind: BodyKind) -> Self {
        Self {
            kind,
            shape: Shape::Auto,
            mass: None,
            friction: 0.5,
            bounciness: 0.0,
            sensor: false,
            lock_rotation: false,
            gravity_scale: 1.0,
            linear_damping: 0.0,
            angular_damping: 0.05,
            ccd: false,
            velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            impulse: Vec3::ZERO,
            torque_impulse: Vec3::ZERO,
            force: Vec3::ZERO,
        }
    }

    pub fn dynamic() -> Self {
        Self::new(BodyKind::Dynamic)
    }

    pub fn fixed() -> Self {
        Self::new(BodyKind::Static)
    }

    pub fn kinematic() -> Self {
        Self::new(BodyKind::Kinematic)
    }

    pub fn with_shape(mut self, shape: Shape) -> Self {
        self.shape = shape;
        self
    }
}

/// A collision between two objects with bodies (delivered after the physics step).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Collision {
    pub a: ObjectId,
    pub b: ObjectId,
    /// `true` when the contact begins, `false` when it ends.
    pub started: bool,
    /// At least one of the two is a sensor.
    pub sensor: bool,
    /// Relative speed of the two bodies when the contact started (m/s). Handy for impact sounds.
    pub speed: f32,
}

/// Result of [`Physics::raycast`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub object: ObjectId,
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}

/// A physics engine plugged into the world (see `spark-physics`).
pub trait PhysicsBackend {
    /// Syncs bodies from the scene, simulates `dt` seconds, writes transforms/velocities back and
    /// pushes collision events into `world.physics.events`.
    fn step(&mut self, world: &mut World, dt: f32);
    /// Casts a ray against the state of the last step. `dir` doesn't need to be normalized.
    fn raycast(&self, origin: Vec3, dir: Vec3, max_distance: f32, ignore: Option<ObjectId>) -> Option<RayHit>;
    /// Drops every body (scene reset / hot reload).
    fn clear(&mut self);
    /// Collide-and-slide for the body of `id` placed at `position` / `rotation` (world space).
    /// `None` if the object has no simulated body yet.
    fn move_character(&self, _id: ObjectId, _position: Vec3, _rotation: Quat, _desired: Vec3, _dt: f32) -> Option<CharacterMove> {
        None
    }
}

/// Result of [`Physics::move_character`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterMove {
    /// The movement that is actually possible (slides along walls, climbs small steps).
    pub translation: Vec3,
    /// Standing on something after the move.
    pub grounded: bool,
}

/// Physics settings, events and the backend.
pub struct Physics {
    pub gravity: Vec3,
    /// Pause the simulation without removing bodies.
    pub enabled: bool,
    /// Collisions from the steps of the current frame. Cleared at the start of every frame.
    pub events: Vec<Collision>,
    backend: Option<Box<dyn PhysicsBackend>>,
}

impl Default for Physics {
    fn default() -> Self {
        Self { gravity: Vec3::new(0.0, -9.81, 0.0), enabled: true, events: Vec::new(), backend: None }
    }
}

impl std::fmt::Debug for Physics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Physics")
            .field("gravity", &self.gravity)
            .field("enabled", &self.enabled)
            .field("events", &self.events.len())
            .field("backend", &self.backend.is_some())
            .finish()
    }
}

impl Physics {
    pub fn set_backend(&mut self, backend: Box<dyn PhysicsBackend>) {
        self.backend = Some(backend);
    }

    pub fn has_backend(&self) -> bool {
        self.backend.is_some()
    }

    /// First body hit by the ray (state of the last physics step). `None` without a backend.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_distance: f32, ignore: Option<ObjectId>) -> Option<RayHit> {
        self.backend.as_ref()?.raycast(origin, dir, max_distance, ignore)
    }

    /// Character-controller movement for a (kinematic) body: how far it can really move by `desired`.
    pub fn move_character(&self, id: ObjectId, position: Vec3, rotation: Quat, desired: Vec3, dt: f32) -> Option<CharacterMove> {
        self.backend.as_ref()?.move_character(id, position, rotation, desired, dt)
    }

    /// Drops every simulated body (keeps settings).
    pub fn clear(&mut self) {
        self.events.clear();
        if let Some(b) = self.backend.as_mut() {
            b.clear();
        }
    }
}

/// Runs one physics step of `dt` seconds (no-op without a backend or when disabled).
pub fn step(world: &mut World, dt: f32) {
    if !world.physics.enabled {
        return;
    }
    if let Some(mut backend) = world.physics.backend.take() {
        backend.step(world, dt);
        world.physics.backend = Some(backend);
    }
}
