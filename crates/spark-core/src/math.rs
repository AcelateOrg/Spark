use glam::{Mat3, Quat, Vec3};

/// Rotation that points local -Z (forward) along `direction`, keeping +Y up.
pub fn look_rotation(direction: Vec3) -> Quat {
    let f = direction.normalize_or(Vec3::NEG_Z);
    let up = if f.y.abs() > 0.999 {
        if f.y < 0.0 { Vec3::NEG_Z } else { Vec3::Z }
    } else {
        Vec3::Y
    };
    let right = f.cross(up).normalize();
    let up = right.cross(f);
    Quat::from_mat3(&Mat3::from_cols(right, up, -f))
}
