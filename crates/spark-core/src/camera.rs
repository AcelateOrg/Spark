use glam::{Mat4, Quat, Vec3};

use crate::math::look_rotation;

/// Perspective camera. Looks along its local -Z axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub position: Vec3,
    pub rotation: Quat,
    /// Vertical field of view in degrees.
    pub fov: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for Camera {
    fn default() -> Self {
        let mut cam = Self { position: Vec3::new(0.0, 3.0, 8.0), rotation: Quat::IDENTITY, fov: 60.0, near: 0.1, far: 500.0 };
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
        glam::camera::rh::proj::directx::perspective(self.fov.clamp(1.0, 179.0).to_radians(), aspect.max(0.001), self.near.max(0.0001), self.far)
    }
}
