//! Spark Engine - Physics: rapier3d backend for `spark_core::physics`.
//!
//! The scene stays the source of truth: every step the backend reads `Object::body` + transforms,
//! simulates, and writes transforms / velocities back. Moving an object by hand teleports its body.
//!
//! ```ignore
//! world.physics.set_backend(Box::new(spark_physics::RapierBackend::new()));
//! ```

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

use rapier3d::prelude as rp;
use rapier3d::prelude::{
    ActiveCollisionTypes, ActiveEvents, ChannelEventCollector, ColliderBuilder, ColliderHandle, CollisionEvent,
    PhysicsWorld, QueryFilter, Ray, RigidBodyBuilder, RigidBodyHandle,
};
use rapier3d::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use spark_core::{
    Body, BodyKind, CharacterMove, Collision, MeshId, Object, ObjectId, PhysicsBackend, Quat, RayHit, Shape, ShapeHint, Vec3,
    World,
};

/// Teleport / velocity-change detection tolerance.
const EPS: f32 = 1e-4;

fn rv(v: Vec3) -> rp::Vector {
    rp::Vector::new(v.x, v.y, v.z)
}

fn sv(v: rp::Vector) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

fn rq(q: Quat) -> rp::Rotation {
    rp::Rotation::from_xyzw(q.x, q.y, q.z, q.w)
}

fn sq(q: rp::Rotation) -> Quat {
    Quat::from_xyzw(q.x, q.y, q.z, q.w).normalize()
}

fn pose(pos: Vec3, rot: Quat) -> rp::Pose {
    rp::Pose::from_parts(rv(pos), rq(rot))
}

/// Everything that requires rebuilding the collider when it changes.
#[derive(Clone, PartialEq)]
struct ColliderKey {
    shape: Shape,
    mesh: Option<MeshId>,
    scale: Vec3,
    mass: Option<f32>,
    friction: f32,
    bounciness: f32,
    sensor: bool,
}

/// Body settings applied with setters when they change.
#[derive(Clone, PartialEq)]
struct BodyConfig {
    lock_rotation: bool,
    gravity_scale: f32,
    linear_damping: f32,
    angular_damping: f32,
    ccd: bool,
}

impl BodyConfig {
    fn of(b: &Body) -> Self {
        Self {
            lock_rotation: b.lock_rotation,
            gravity_scale: b.gravity_scale,
            linear_damping: b.linear_damping,
            angular_damping: b.angular_damping,
            ccd: b.ccd,
        }
    }
}

struct Entry {
    body: RigidBodyHandle,
    collider: ColliderHandle,
    kind: BodyKind,
    key: ColliderKey,
    config: BodyConfig,
    /// World transform / velocity as last seen or written by us.
    pos: Vec3,
    rot: Quat,
    vel: Vec3,
    angvel: Vec3,
}

/// rapier3d-based physics backend.
pub struct RapierBackend {
    world: PhysicsWorld,
    entries: HashMap<ObjectId, Entry>,
}

impl Default for RapierBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RapierBackend {
    pub fn new() -> Self {
        Self { world: PhysicsWorld::new(), entries: HashMap::new() }
    }

    /// Number of simulated bodies.
    pub fn body_count(&self) -> usize {
        self.entries.len()
    }

    fn remove(&mut self, id: ObjectId) {
        if let Some(e) = self.entries.remove(&id) {
            self.world.remove_body(e.body);
        }
    }

    fn create(&mut self, world: &World, id: ObjectId, o: &Object, body: &Body, pos: Vec3, rot: Quat, key: ColliderKey) {
        let builder = match body.kind {
            BodyKind::Dynamic => RigidBodyBuilder::dynamic(),
            BodyKind::Static => RigidBodyBuilder::fixed(),
            BodyKind::Kinematic => RigidBodyBuilder::kinematic_position_based(),
        };
        let mut builder = builder
            .pose(pose(pos, rot))
            .linvel(rv(body.velocity))
            .angvel(rv(body.angular_velocity))
            .gravity_scale(body.gravity_scale)
            .linear_damping(body.linear_damping)
            .angular_damping(body.angular_damping)
            .ccd_enabled(body.ccd)
            .user_data(id.to_bits() as u128);
        if body.lock_rotation {
            builder = builder.lock_rotations();
        }
        let collider = make_collider(world, id, o, body, key.scale);
        let (bh, ch) = self.world.insert(builder, collider);
        self.entries.insert(
            id,
            Entry {
                body: bh,
                collider: ch,
                kind: body.kind,
                key,
                config: BodyConfig::of(body),
                pos,
                rot,
                vel: body.velocity,
                angvel: body.angular_velocity,
            },
        );
    }

    /// Brings rapier in line with the scene (creates / removes / updates bodies, applies impulses).
    fn sync(&mut self, world: &mut World) {
        let ids: Vec<ObjectId> =
            world.scene.iter().filter(|(_, o)| o.body.is_some()).map(|(id, _)| id).collect();
        let alive: HashSet<ObjectId> = ids.iter().copied().collect();
        let dead: Vec<ObjectId> = self.entries.keys().filter(|id| !alive.contains(id)).copied().collect();
        for id in dead {
            self.remove(id);
        }

        for id in ids {
            let (scale, rot, pos) = world.scene.world_matrix(id).to_scale_rotation_translation();
            let o = &world.scene[id];
            let body = o.body.as_ref().expect("filtered");
            let key = ColliderKey {
                shape: body.shape,
                mesh: o.mesh,
                scale,
                mass: body.mass,
                friction: body.friction,
                bounciness: body.bounciness,
                sensor: body.sensor,
            };
            let recreate = self.entries.get(&id).is_none_or(|e| e.kind != body.kind);
            if recreate {
                self.remove(id);
                self.create(world, id, o, body, pos, rot, key);
            } else {
                let e = self.entries.get_mut(&id).expect("exists");
                if e.key != key {
                    self.world.remove_collider(e.collider);
                    let c = make_collider(world, id, o, body, key.scale);
                    e.collider = self.world.insert_collider(c, Some(e.body));
                    e.key = key;
                }
                let rb = &mut self.world.bodies[e.body];
                let config = BodyConfig::of(body);
                if e.config != config {
                    rb.lock_rotations(config.lock_rotation, true);
                    rb.set_gravity_scale(config.gravity_scale, true);
                    rb.set_linear_damping(config.linear_damping);
                    rb.set_angular_damping(config.angular_damping);
                    rb.enable_ccd(config.ccd);
                    e.config = config;
                }
                let moved = pos.distance(e.pos) > EPS || rot.angle_between(e.rot) > EPS;
                match body.kind {
                    BodyKind::Kinematic => rb.set_next_kinematic_position(pose(pos, rot)),
                    _ if moved => rb.set_position(pose(pos, rot), true),
                    _ => {}
                }
                if body.kind == BodyKind::Dynamic {
                    if body.velocity.distance(e.vel) > EPS {
                        rb.set_linvel(rv(body.velocity), true);
                    }
                    if body.angular_velocity.distance(e.angvel) > EPS {
                        rb.set_angvel(rv(body.angular_velocity), true);
                    }
                }
            }

            // One-shot impulses / forces.
            let e = &self.entries[&id];
            let rb = &mut self.world.bodies[e.body];
            rb.reset_forces(false);
            if body.kind == BodyKind::Dynamic {
                if body.impulse != Vec3::ZERO {
                    rb.apply_impulse(rv(body.impulse), true);
                }
                if body.torque_impulse != Vec3::ZERO {
                    rb.apply_torque_impulse(rv(body.torque_impulse), true);
                }
                if body.force != Vec3::ZERO {
                    rb.add_force(rv(body.force), true);
                }
            }
            if let Some(b) = world.scene.get_mut(id).and_then(|o| o.body.as_mut()) {
                b.impulse = Vec3::ZERO;
                b.torque_impulse = Vec3::ZERO;
                b.force = Vec3::ZERO;
            }
        }
    }

    /// Copies simulated transforms and velocities back into the scene.
    fn write_back(&mut self, world: &mut World) {
        for (&id, e) in self.entries.iter_mut() {
            let rb = &self.world.bodies[e.body];
            let (vel, angvel) = (sv(rb.linvel()), sv(rb.angvel()));
            if e.kind == BodyKind::Dynamic {
                let (pos, rot) = (sv(rb.translation()), sq(*rb.rotation()));
                let parent = world.scene.get(id).and_then(|o| o.parent());
                let (lpos, lrot) = match parent {
                    None => (pos, rot),
                    Some(p) => {
                        let m = world.scene.world_matrix(p).inverse() * spark_core::Mat4::from_rotation_translation(rot, pos);
                        let (_, r, t) = m.to_scale_rotation_translation();
                        (t, r.normalize())
                    }
                };
                if let Some(o) = world.scene.get_mut(id) {
                    o.position = lpos;
                    o.rotation = lrot;
                }
            }
            let (_, rot, pos) = world.scene.world_matrix(id).to_scale_rotation_translation();
            e.pos = pos;
            e.rot = rot;
            e.vel = vel;
            e.angvel = angvel;
            if let Some(b) = world.scene.get_mut(id).and_then(|o| o.body.as_mut()) {
                b.velocity = vel;
                b.angular_velocity = angvel;
            }
        }
    }

    fn object_of(&self, collider: ColliderHandle) -> Option<ObjectId> {
        self.world.colliders.get(collider).map(|c| ObjectId::from_bits(c.user_data as u64))
    }

    fn speed_between(&self, a: ColliderHandle, b: ColliderHandle) -> f32 {
        let vel = |h: ColliderHandle| {
            self.world
                .colliders
                .get(h)
                .and_then(|c| c.parent())
                .and_then(|p| self.world.bodies.get(p))
                .map(|rb| sv(rb.linvel()))
                .unwrap_or(Vec3::ZERO)
        };
        (vel(a) - vel(b)).length()
    }
}

impl PhysicsBackend for RapierBackend {
    fn step(&mut self, world: &mut World, dt: f32) {
        self.world.gravity = rv(world.physics.gravity);
        self.world.integration_parameters.dt = dt;
        self.sync(world);

        // Relative speeds must be measured before the solver resolves the impact.
        let before: HashMap<ObjectId, Vec3> = self
            .entries
            .iter()
            .map(|(&id, e)| (id, sv(self.world.bodies[e.body].linvel())))
            .collect();

        let (collision_send, collision_recv) = mpsc::channel();
        let (force_send, _force_recv) = mpsc::channel();
        let (tear_send, _tear_recv) = mpsc::channel();
        let collector = ChannelEventCollector::new(collision_send, force_send, tear_send);
        self.world.step_with_events(&(), &collector);
        self.write_back(world);

        while let Ok(event) = collision_recv.try_recv() {
            let (h1, h2, started) = match event {
                CollisionEvent::Started(a, b, _) => (a, b, true),
                CollisionEvent::Stopped(a, b, _) => (a, b, false),
            };
            let (Some(a), Some(b)) = (self.object_of(h1), self.object_of(h2)) else { continue };
            if !world.scene.contains(a) || !world.scene.contains(b) {
                continue;
            }
            let speed = match (before.get(&a), before.get(&b)) {
                (Some(va), Some(vb)) if started => (*va - *vb).length(),
                _ if started => self.speed_between(h1, h2),
                _ => 0.0,
            };
            world.physics.events.push(Collision { a, b, started, sensor: event.sensor(), speed });
        }
    }

    fn raycast(&self, origin: Vec3, dir: Vec3, max_distance: f32, ignore: Option<ObjectId>) -> Option<RayHit> {
        let dir = dir.normalize_or_zero();
        if dir == Vec3::ZERO {
            return None;
        }
        let skip = ignore.and_then(|id| self.entries.get(&id)).map(|e| e.body);
        let mut filter = QueryFilter::default().exclude_sensors();
        if let Some(b) = skip {
            filter = filter.exclude_rigid_body(b);
        }
        let ray = Ray::new(rv(origin), rv(dir));
        let (h, hit) = self.world.cast_ray_and_get_normal(&ray, max_distance, true, filter)?;
        let object = self.object_of(h)?;
        Some(RayHit {
            object,
            point: origin + dir * hit.time_of_impact,
            normal: sv(hit.normal),
            distance: hit.time_of_impact,
        })
    }

    fn clear(&mut self) {
        self.world = PhysicsWorld::new();
        self.entries.clear();
    }

    fn move_character(&self, id: ObjectId, position: Vec3, rotation: Quat, desired: Vec3, dt: f32) -> Option<CharacterMove> {
        let e = self.entries.get(&id)?;
        let collider = self.world.colliders.get(e.collider)?;
        let mut at = pose(position, rotation);
        if let Some(offset) = collider.position_wrt_parent() {
            at = at * *offset;
        }
        let filter = QueryFilter::default().exclude_sensors().exclude_rigid_body(e.body);
        let queries = self.world.query_pipeline_with_filter(filter);
        let controller = KinematicCharacterController {
            autostep: Some(CharacterAutostep {
                max_height: CharacterLength::Absolute(0.35),
                min_width: CharacterLength::Absolute(0.15),
                include_dynamic_bodies: false,
            }),
            snap_to_ground: Some(CharacterLength::Absolute(0.25)),
            max_slope_climb_angle: 50f32.to_radians(),
            min_slope_slide_angle: 40f32.to_radians(),
            ..Default::default()
        };
        let m = controller.move_shape(dt.max(1e-4), &queries, collider.shape(), &at, rv(desired), |_| {});
        Some(CharacterMove { translation: sv(m.translation), grounded: m.grounded })
    }
}

/// Builds the collider for an object (shape scaled by the object's world scale).
fn make_collider(world: &World, id: ObjectId, o: &Object, body: &Body, scale: Vec3) -> ColliderBuilder {
    let s = scale.abs();
    let radial = s.x.max(s.z);
    let builder = match body.shape {
        Shape::Box(size) => cuboid(size * s * 0.5),
        Shape::Sphere(r) => ColliderBuilder::ball(r * s.max_element()),
        Shape::Capsule { radius, height } => {
            let r = radius * radial;
            ColliderBuilder::capsule_y((height * s.y * 0.5 - r).max(0.0), r)
        }
        Shape::Cylinder { radius, height } => ColliderBuilder::cylinder(height * s.y * 0.5, radius * radial),
        Shape::Auto => auto_collider(world, o, body.kind, s),
    };
    let builder = builder
        .friction(body.friction)
        .restitution(body.bounciness)
        .sensor(body.sensor)
        .active_events(ActiveEvents::COLLISION_EVENTS)
        .active_collision_types(ActiveCollisionTypes::all())
        .user_data(id.to_bits() as u128);
    match body.mass {
        Some(m) => builder.mass(m.max(1e-4)),
        None => builder.density(1.0),
    }
}

fn cuboid(half: Vec3) -> ColliderBuilder {
    let h = half.max(Vec3::splat(0.005));
    ColliderBuilder::cuboid(h.x, h.y, h.z)
}

fn auto_collider(world: &World, o: &Object, kind: BodyKind, s: Vec3) -> ColliderBuilder {
    let bounds = o.local_bounds();
    if bounds.is_empty() {
        return ColliderBuilder::ball(0.5 * s.max_element());
    }
    let mesh = o.mesh.and_then(|m| world.assets.mesh(m));
    let hint = mesh.map(|m| m.shape).unwrap_or(ShapeHint::Box);
    let size = bounds.size() * s;
    let mut center = bounds.center() * s;
    let builder = match hint {
        ShapeHint::Box => cuboid(size * 0.5),
        ShapeHint::Sphere => ColliderBuilder::ball(size.max_element() * 0.5),
        ShapeHint::Cylinder => ColliderBuilder::cylinder(size.y * 0.5, size.x.max(size.z) * 0.5),
        ShapeHint::Plane => {
            // Slab of 0.5 m behind the front face so fast objects don't tunnel through.
            const SLAB: f32 = 0.25;
            let mut half = size * 0.5;
            let thin = if size.y <= size.x && size.y <= size.z { 1 } else if size.z <= size.x { 2 } else { 0 };
            half[thin] = SLAB;
            center[thin] -= SLAB;
            cuboid(half)
        }
        ShapeHint::Custom => {
            let mesh = mesh.expect("custom hint comes from a mesh");
            let points: Vec<rp::Vector> =
                mesh.vertices.iter().map(|v| rv(Vec3::from_array(v.position) * s)).collect();
            let built = if kind == BodyKind::Dynamic {
                ColliderBuilder::convex_hull(&points)
            } else {
                let tris: Vec<[u32; 3]> = mesh.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
                ColliderBuilder::trimesh(points, tris).ok()
            };
            match built {
                Some(b) => return b,
                None => cuboid(size * 0.5),
            }
        }
    };
    builder.translation(rv(center))
}
