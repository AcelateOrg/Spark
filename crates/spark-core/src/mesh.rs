use std::f32::consts::{PI, TAU};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// GPU vertex: position, normal, uv.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

impl Vertex {
    pub fn new(position: Vec3, normal: Vec3, uv: [f32; 2]) -> Self {
        Self { position: position.to_array(), normal: normal.to_array(), uv }
    }
}

/// Axis-aligned bounding box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Default for Aabb {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb { min: Vec3::splat(f32::INFINITY), max: Vec3::splat(f32::NEG_INFINITY) };

    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    pub fn from_points(points: impl IntoIterator<Item = Vec3>) -> Self {
        points.into_iter().fold(Self::EMPTY, |b, p| Self { min: b.min.min(p), max: b.max.max(p) })
    }

    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x
    }

    pub fn size(&self) -> Vec3 {
        if self.is_empty() { Vec3::ZERO } else { self.max - self.min }
    }

    pub fn center(&self) -> Vec3 {
        if self.is_empty() { Vec3::ZERO } else { (self.min + self.max) * 0.5 }
    }

    pub fn union(&self, other: &Aabb) -> Aabb {
        Aabb { min: self.min.min(other.min), max: self.max.max(other.max) }
    }

    pub fn corners(&self) -> [Vec3; 8] {
        let (a, b) = (self.min, self.max);
        [
            Vec3::new(a.x, a.y, a.z),
            Vec3::new(b.x, a.y, a.z),
            Vec3::new(a.x, b.y, a.z),
            Vec3::new(b.x, b.y, a.z),
            Vec3::new(a.x, a.y, b.z),
            Vec3::new(b.x, a.y, b.z),
            Vec3::new(a.x, b.y, b.z),
            Vec3::new(b.x, b.y, b.z),
        ]
    }

    /// Bounds of this box after transforming it by `m`.
    pub fn transform(&self, m: &Mat4) -> Aabb {
        if self.is_empty() {
            return Self::EMPTY;
        }
        Self::from_points(self.corners().map(|c| m.transform_point3(c)))
    }
}

/// What a mesh looks like to the physics engine (for `Shape::Auto`). Sizes come from the bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShapeHint {
    /// Arbitrary triangles: triangle mesh (static) or convex hull (dynamic).
    #[default]
    Custom,
    Box,
    Sphere,
    /// Along Y.
    Cylinder,
    /// Flat one-sided surface: becomes a thick slab behind its front face.
    Plane,
}

/// Options for generated flat surfaces (`cuboid_with`, `plane_with`, `quad_with`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceOpts {
    /// Meters per texture repeat (UVs follow the real size, so textures never stretch). `None` = 0..1 per face.
    pub tile: Option<f32>,
    /// Split faces into cells of about this many meters (less PS1 texture warping, smoother lighting).
    pub cell: Option<f32>,
}

/// CPU-side triangle mesh. Counter-clockwise triangles are front faces.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub shape: ShapeHint,
}

impl MeshData {
    pub fn new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        Self { vertices, indices, shape: ShapeHint::Custom }
    }

    /// Flat-shaded mesh from a triangle list (3 points per triangle, counter-clockwise).
    pub fn from_triangles(points: &[Vec3]) -> Self {
        let mut mesh = Self::default();
        for tri in points.chunks_exact(3) {
            let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or(Vec3::Y);
            let base = mesh.vertices.len() as u32;
            mesh.vertices.push(Vertex::new(tri[0], n, [0.0, 1.0]));
            mesh.vertices.push(Vertex::new(tri[1], n, [1.0, 1.0]));
            mesh.vertices.push(Vertex::new(tri[2], n, [0.5, 0.0]));
            mesh.indices.extend([base, base + 1, base + 2]);
        }
        mesh
    }

    pub fn bounds(&self) -> Aabb {
        Aabb::from_points(self.vertices.iter().map(|v| Vec3::from_array(v.position)))
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Cube centered at the origin.
    pub fn cube(size: f32) -> Self {
        Self::cuboid(size, size, size)
    }

    /// Box with world-scaled UVs and/or subdivided faces (see [`SurfaceOpts`]).
    pub fn cuboid_with(width: f32, height: f32, depth: f32, opts: SurfaceOpts) -> Self {
        let half = Vec3::new(width, height, depth) * 0.5;
        let mut mesh = Self::default();
        for (n, u, v) in Self::BOX_FACES {
            mesh.push_face(n * half, u * half, v * half, n, opts);
        }
        mesh.shape = ShapeHint::Box;
        mesh
    }

    /// Plane (XZ, facing +Y) with world-scaled UVs and/or subdivision.
    pub fn plane_with(width: f32, depth: f32, opts: SurfaceOpts) -> Self {
        let mut mesh = Self::default();
        mesh.push_face(Vec3::ZERO, Vec3::X * width * 0.5, Vec3::NEG_Z * depth * 0.5, Vec3::Y, opts);
        mesh.shape = ShapeHint::Plane;
        mesh
    }

    /// Quad (XY, facing +Z) with world-scaled UVs and/or subdivision.
    pub fn quad_with(width: f32, height: f32, opts: SurfaceOpts) -> Self {
        let mut mesh = Self::default();
        mesh.push_face(Vec3::ZERO, Vec3::X * width * 0.5, Vec3::Y * height * 0.5, Vec3::Z, opts);
        mesh.shape = ShapeHint::Plane;
        mesh
    }

    const BOX_FACES: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::NEG_Z, Vec3::Y),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, Vec3::NEG_Z),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
    ];

    /// Face centered at `c` spanning `c +- hu +- hv`, split into a grid. UV v = 1 at the -hv edge.
    fn push_face(&mut self, c: Vec3, hu: Vec3, hv: Vec3, n: Vec3, opts: SurfaceOpts) {
        let (lu, lv) = (hu.length() * 2.0, hv.length() * 2.0);
        let cells = |len: f32| match opts.cell {
            Some(cell) if cell > 0.0 => ((len / cell).ceil() as u32).clamp(1, 64),
            _ => 1,
        };
        let (nu, nv) = (cells(lu), cells(lv));
        let (su, sv) = match opts.tile {
            Some(t) if t > 0.0 => (lu / t, lv / t),
            _ => (1.0, 1.0),
        };
        let base = self.vertices.len() as u32;
        for j in 0..=nv {
            let t = j as f32 / nv as f32;
            for i in 0..=nu {
                let s = i as f32 / nu as f32;
                let p = c + hu * (s * 2.0 - 1.0) + hv * (t * 2.0 - 1.0);
                self.vertices.push(Vertex::new(p, n, [s * su, 1.0 - t * sv]));
            }
        }
        let row = nu + 1;
        for j in 0..nv {
            for i in 0..nu {
                let a = base + j * row + i;
                self.indices.extend([a, a + 1, a + row + 1, a, a + row + 1, a + row]);
            }
        }
    }

    /// Box centered at the origin: width (X), height (Y), depth (Z).
    pub fn cuboid(width: f32, height: f32, depth: f32) -> Self {
        let half = Vec3::new(width, height, depth) * 0.5;
        // (normal, u axis, v axis) with u x v = normal, so quads are counter-clockwise.
        let faces = [
            (Vec3::X, Vec3::NEG_Z, Vec3::Y),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::X, Vec3::NEG_Z),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
        ];
        let mut mesh = Self::default();
        for (n, u, v) in faces {
            let (c, hu, hv) = (n * half, u * half, v * half);
            mesh.push_quad(
                [c - hu - hv, c + hu - hv, c + hu + hv, c - hu + hv],
                n,
                [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            );
        }
        mesh.shape = ShapeHint::Box;
        mesh
    }

    /// Flat plane on the XZ axes facing +Y, centered at the origin.
    pub fn plane(width: f32, depth: f32) -> Self {
        let (x, z) = (width * 0.5, depth * 0.5);
        let mut mesh = Self::default();
        mesh.push_quad(
            [Vec3::new(-x, 0.0, z), Vec3::new(x, 0.0, z), Vec3::new(x, 0.0, -z), Vec3::new(-x, 0.0, -z)],
            Vec3::Y,
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        );
        mesh.shape = ShapeHint::Plane;
        mesh
    }

    /// Flat quad on the XY axes facing +Z (sprites, billboards, UI-in-world).
    pub fn quad(width: f32, height: f32) -> Self {
        let (x, y) = (width * 0.5, height * 0.5);
        let mut mesh = Self::default();
        mesh.push_quad(
            [Vec3::new(-x, -y, 0.0), Vec3::new(x, -y, 0.0), Vec3::new(x, y, 0.0), Vec3::new(-x, y, 0.0)],
            Vec3::Z,
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        );
        mesh.shape = ShapeHint::Plane;
        mesh
    }

    /// UV sphere centered at the origin. `segments` around the equator (min 3).
    pub fn sphere(radius: f32, segments: u32) -> Self {
        let seg = segments.max(3);
        let rings = (seg / 2).max(2);
        let mut mesh = Self::default();
        for r in 0..=rings {
            let v = r as f32 / rings as f32;
            let phi = v * PI;
            for s in 0..=seg {
                let u = s as f32 / seg as f32;
                let theta = u * TAU;
                let n = Vec3::new(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin());
                mesh.vertices.push(Vertex::new(n * radius, n, [u, v]));
            }
        }
        let row = seg + 1;
        for r in 0..rings {
            for s in 0..seg {
                let a = r * row + s;
                let (b, c, d) = (a + row, a + row + 1, a + 1);
                mesh.indices.extend([a, c, b, a, d, c]);
            }
        }
        mesh.shape = ShapeHint::Sphere;
        mesh
    }

    /// Cylinder along Y, centered at the origin. Use few segments for low-poly.
    pub fn cylinder(radius: f32, height: f32, segments: u32) -> Self {
        let seg = segments.max(3);
        let hy = height * 0.5;
        let mut mesh = Self::default();
        let ring = |t: f32| Vec3::new((t * TAU).sin(), 0.0, (t * TAU).cos());
        // Sides.
        for s in 0..seg {
            let (t0, t1) = (s as f32 / seg as f32, (s + 1) as f32 / seg as f32);
            let (n0, n1) = (ring(t0), ring(t1));
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend([
                Vertex::new(n0 * radius - Vec3::Y * hy, n0, [t0, 1.0]),
                Vertex::new(n1 * radius - Vec3::Y * hy, n1, [t1, 1.0]),
                Vertex::new(n1 * radius + Vec3::Y * hy, n1, [t1, 0.0]),
                Vertex::new(n0 * radius + Vec3::Y * hy, n0, [t0, 0.0]),
            ]);
            mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        // Caps.
        for (y, n) in [(hy, Vec3::Y), (-hy, Vec3::NEG_Y)] {
            let center = mesh.vertices.len() as u32;
            mesh.vertices.push(Vertex::new(Vec3::new(0.0, y, 0.0), n, [0.5, 0.5]));
            for s in 0..=seg {
                let p = ring(s as f32 / seg as f32);
                mesh.vertices.push(Vertex::new(p * radius + Vec3::Y * y, n, [0.5 + p.x * 0.5, 0.5 + p.z * 0.5]));
            }
            for s in 0..seg {
                let (a, b) = (center + 1 + s, center + 2 + s);
                if n.y > 0.0 {
                    mesh.indices.extend([center, a, b]);
                } else {
                    mesh.indices.extend([center, b, a]);
                }
            }
        }
        mesh.shape = ShapeHint::Cylinder;
        mesh
    }

    fn push_quad(&mut self, corners: [Vec3; 4], normal: Vec3, uvs: [[f32; 2]; 4]) {
        let base = self.vertices.len() as u32;
        for i in 0..4 {
            self.vertices.push(Vertex::new(corners[i], normal, uvs[i]));
        }
        self.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}
