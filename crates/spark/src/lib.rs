//! Spark Engine - code-first game engine library. No editor: your game depends on this crate.
//!
//! ```no_run
//! use spark::prelude::*;
//!
//! struct Hello;
//! impl Game for Hello {
//!     fn start(&mut self, world: &mut World) {
//!         let cube = world.add_mesh(MeshData::cube(1.0));
//!         world.spawn(cube, Material::color(Color::ORANGE));
//!     }
//! }
//!
//! fn main() {
//!     App::new("Hello").run(Hello);
//! }
//! ```
//!
//! Conventions: Y-up, right-handed, meters, radians, sRGB colors.

mod app;

pub use app::{App, HELP, RunArgs};
pub use spark_core::*;
pub use spark_render::{FrameStats, GpuInfo, Renderer};

pub use spark_audio as audio;
pub use spark_physics as physics;
pub use spark_render as render;
pub use spark_script as script;
pub use spark_window as window;

/// Everything a typical game needs: `use spark::prelude::*;`
pub mod prelude {
    pub use crate::App;
    pub use spark_core::{
        Aabb, Assets, Body, BodyKind, Camera, Collision, PlayParams, RayHit, Shape, SoundId, Color, EulerRot, Fog, Game, ImageData, Input, Key, Mat4, Material, MeshData, MeshId,
        MouseButton, Object, ObjectId, Quat, PassSize, PostPass, RenderSettings, Scene, ShaderData, ShaderId, ShaderKind, Sun, TextureFilter, TextureId, Time, VERSION, Vec2,
        Vec3, Vec4, World, vec2, vec3, vec4,
    };
}
