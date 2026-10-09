use glam::{Mat4, Quat, Vec2, Vec3};

use crate::math::look_rotation;

/// Perspective (or orthographic) camera. Looks along its local -Z axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    pub rotation: Quat,
    /// Vertical field of view in degrees.
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    /// `Some(height)` = orthographic camera showing `height` meters vertically (2.5D, strategy,
    /// editors); `fov` is then ignored.
    pub ortho: Option<f32>,
}

impl Default for Camera {
    fn default() -> Self {
        let mut cam = Self { position: Vec3::new(0.0, 3.0, 8.0), rotation: Quat::IDENTITY, fov: 60.0, near: 0.1, far: 500.0, ortho: None };
        cam.look_at(Vec3::ZERO);
        cam
    }
}

impl Camera {
    pub fn look_at(&mut self, target: Vec3) {
        if target != self.position {
            self.rotation = look_rotation(target - self.position);
        }
    }

    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    pub fn right(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    pub fn up(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }

    pub fn view(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.rotation, self.position).inverse()
    }

    pub fn projection(&self, aspect: f32) -> Mat4 {
        // Right-handed view space, WebGPU/DirectX clip space (Z 0..1, Y up).
        let aspect = aspect.max(0.001);
        if let Some(h) = self.ortho {
            let (hh, hw) = (h.max(0.001) * 0.5, h.max(0.001) * 0.5 * aspect);
            return glam::camera::rh::proj::directx::orthographic(-hw, hw, -hh, hh, self.near, self.far);
        }
        glam::camera::rh::proj::directx::perspective(self.fov.clamp(1.0, 179.0).to_radians(), aspect, self.near.max(0.0001), self.far)
    }

    /// World point -> screen pixel (`screen` = output size, (0,0) = top-left) and depth 0..1.
    /// `None` when the point is behind the camera.
    pub fn world_to_screen(&self, point: Vec3, screen: Vec2) -> Option<(Vec2, f32)> {
        let clip = self.projection(screen.x / screen.y.max(1.0)) * self.view() * point.extend(1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some((Vec2::new((ndc.x * 0.5 + 0.5) * screen.x, (0.5 - ndc.y * 0.5) * screen.y), ndc.z))
    }

    /// Screen pixel -> world ray `(origin, direction)` (mouse picking, aiming).
    pub fn screen_to_ray(&self, pixel: Vec2, screen: Vec2) -> (Vec3, Vec3) {
        let ndc = Vec2::new(pixel.x / screen.x.max(1.0) * 2.0 - 1.0, 1.0 - pixel.y / screen.y.max(1.0) * 2.0);
        let inv = (self.projection(screen.x / screen.y.max(1.0)) * self.view()).inverse();
        let near = inv.project_point3(ndc.extend(0.0));
        let far = inv.project_point3(ndc.extend(1.0));
        (near, (far - near).normalize_or(self.forward()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_roundtrip() {
        let mut cam = Camera::default();
        let screen = Vec2::new(1280.0, 720.0);
        for ortho in [None, Some(10.0)] {
            cam.ortho = ortho;
            let p = Vec3::new(1.0, 0.5, -2.0);
            let (px, depth) = cam.world_to_screen(p, screen).unwrap();
            assert!((0.0..=1.0).contains(&depth));
            let (o, d) = cam.screen_to_ray(px, screen);
            let closest = o + d * (p - o).dot(d);
            assert!(closest.distance(p) < 1e-3, "{ortho:?}: {closest} vs {p}");
        }
        cam.ortho = None;
        let (center, _) = cam.world_to_screen(cam.position + cam.forward() * 5.0, screen).unwrap();
        assert!(center.distance(screen * 0.5) < 0.5);
        assert!(cam.world_to_screen(cam.position - cam.forward(), screen).is_none());
    }
}
