//! Ray picking against rendered meshes (no physics colliders needed).

use crate::scene::ObjectId;
use crate::world::World;
use glam::Vec3;

/// Result of [`World::pick`].
#[derive(Clone, Copy, Debug)]
pub struct PickHit {
    pub object: ObjectId,
    pub point: Vec3,
    /// World-space surface normal facing the ray origin.
    pub normal: Vec3,
    pub distance: f32,
}

fn ray_box(o: Vec3, inv_d: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let t1 = (min - o) * inv_d;
    let t2 = (max - o) * inv_d;
    let tmin = t1.min(t2).max_element();
    let tmax = t1.max(t2).min_element();
    if tmax >= tmin.max(0.0) && tmax.is_finite() { Some(tmin.max(0.0)) } else { None }
}

/// Two-sided Möller–Trumbore. Returns `t` along the (unnormalized) ray.
fn ray_tri(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t >= 0.0).then_some(t)
}

impl World {
    fn under(&self, id: ObjectId, root: ObjectId) -> bool {
        let mut cur = Some(id);
        let mut depth = 0;
        while let Some(c) = cur {
            if c == root {
                return true;
            }
            cur = self.scene.get(c).and_then(|o| o.parent);
            depth += 1;
            if depth > 64 {
                break;
            }
        }
        false
    }

    /// Casts a ray against the triangles of every visible mesh object.
    /// `ignore` skips that object and all its children (e.g. the player's own model).
    pub fn pick(&self, origin: Vec3, dir: Vec3, max: f32, ignore: Option<ObjectId>) -> Option<PickHit> {
        let d = dir.normalize_or_zero();
        if d == Vec3::ZERO || !(max > 0.0) {
            return None;
        }
        let inv_d = d.recip();
        let mut cands: Vec<(f32, ObjectId)> = Vec::new();
        for (id, o) in self.scene.iter() {
            if o.mesh.is_none() || !self.scene.is_visible(id) {
                continue;
            }
            if let Some(ig) = ignore {
                if self.under(id, ig) {
                    continue;
                }
            }
            let b = self.scene.world_bounds(id);
            if b.is_empty() {
                continue;
            }
            // Pad flat boxes (planes) slightly so the slab test stays robust.
            let pad = Vec3::splat(1e-4);
            if let Some(t) = ray_box(origin, inv_d, b.min - pad, b.max + pad) {
                if t <= max {
                    cands.push((t, id));
                }
            }
        }
        cands.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut best: Option<PickHit> = None;
        for (tbox, id) in cands {
            let limit = best.map_or(max, |h| h.distance);
            if tbox > limit {
                break;
            }
            let Some(mesh) = self.scene.get(id).and_then(|o| o.mesh).and_then(|m| self.assets.mesh(m)) else { continue };
            let m = self.scene.world_matrix(id);
            if m.determinant().abs() < 1e-12 {
                continue;
            }
            let inv = m.inverse();
            let lo = inv.transform_point3(origin);
            let ld = inv.transform_vector3(d);
            let mut local_best: Option<(f32, Vec3)> = None;
            let pos = |i: u32| mesh.vertices.get(i as usize).map(|v| Vec3::from(v.position));
            for tri in mesh.indices.chunks_exact(3) {
                let (Some(a), Some(b), Some(c)) = (pos(tri[0]), pos(tri[1]), pos(tri[2])) else { continue };
                if let Some(t) = ray_tri(lo, ld, a, b, c) {
                    let lim = local_best.map_or(limit, |x| x.0);
                    if t < lim {
                        local_best = Some((t, (b - a).cross(c - a)));
                    }
                }
            }
            if let Some((t, n)) = local_best {
                let mut n = inv.transpose().transform_vector3(n).normalize_or_zero();
                if n.dot(d) > 0.0 {
                    n = -n;
                }
                best = Some(PickHit { object: id, point: origin + d * t, normal: n, distance: t });
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::MeshData;

    #[test]
    fn pick_cubes() {
        let mut w = World::new();
        let cube = w.assets.add_mesh(MeshData::cube(1.0));
        let near = w.spawn(cube, Default::default());
        w.scene.get_mut(near).unwrap().position = Vec3::new(0.0, 0.0, -5.0);
        let far = w.spawn(cube, Default::default());
        {
            let o = w.scene.get_mut(far).unwrap();
            o.position = Vec3::new(0.0, 0.0, -10.0);
            o.scale = Vec3::splat(3.0);
        }

        let h = w.pick(Vec3::ZERO, Vec3::NEG_Z, 100.0, None).unwrap();
        assert_eq!(h.object, near);
        assert!((h.distance - 4.5).abs() < 1e-3, "{}", h.distance);
        assert!((h.normal - Vec3::Z).length() < 1e-3);

        let h = w.pick(Vec3::ZERO, Vec3::NEG_Z, 100.0, Some(near)).unwrap();
        assert_eq!(h.object, far);
        assert!((h.distance - 8.5).abs() < 1e-3, "{}", h.distance);

        assert!(w.pick(Vec3::ZERO, Vec3::NEG_Z, 4.0, None).is_none());
        assert!(w.pick(Vec3::ZERO, Vec3::X, 100.0, None).is_none());
        w.scene.get_mut(near).unwrap().visible = false;
        assert_eq!(w.pick(Vec3::ZERO, Vec3::NEG_Z, 100.0, None).unwrap().object, far);
    }
}
