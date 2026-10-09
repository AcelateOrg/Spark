//! Spark Engine - Core: world data, scene, math, input, time.
//!
//! Conventions (used everywhere in Spark):
//! - Y-up, right-handed, 1 unit = 1 meter.
//! - Objects and cameras look along their local -Z axis.
//! - Angles are in radians (camera FOV is in degrees).
//! - Colors are sRGB 0..1, like CSS / three.js.

pub mod animation;
pub mod assets;
pub mod audio;
pub mod branding;
pub mod camera;
pub mod canvas;
pub mod color;
pub mod events;
pub mod game;
pub mod input;
pub mod material;
pub mod math;
pub mod mesh;
pub mod model;
pub mod physics;
pub mod pick;
pub mod render;
pub mod scene;
pub mod schedule;
pub mod shader;
pub mod text;
pub mod time;
pub mod vfs;
pub mod world;

pub use glam::{self, EulerRot, Mat3, Mat4, Quat, Vec2, Vec3, Vec4, vec2, vec3, vec4};

pub use animation::{AnimParams, Animator};
pub use assets::{Assets, ImageData, MeshId, TextureId};
pub use audio::{Audio, AudioBackend, AudioCommand, BusId, PlayParams, SoundId, SoundSource};
pub use branding::{BadgeParams, BadgeState, Branding, SPARK_URL, Splash};
pub use camera::Camera;
pub use canvas::{Align, Batch, Canvas, CanvasTexture, Layer, LayerData, SpriteParams, TextParams, Vertex2D};
pub use color::Color;
pub use events::{EventBus, ListenerId};
pub use game::{Game, run_frame};
pub use input::{Gamepad, Input, Key, MouseButton, PadAxis, PadButton};
pub use material::Material;
pub use math::look_rotation;
pub use mesh::{Aabb, MeshData, ShapeHint, SurfaceOpts, Vertex};
pub use model::{ModelData, ModelId};
pub use pick::PickHit;
pub use physics::{Body, BodyKind, CharacterMove, Collision, Physics, PhysicsBackend, RayHit, Shape};
pub use render::{PassSize, PostPass, RenderSettings, TextureFilter};
pub use scene::{Fog, Light, LightKind, Object, ObjectId, Scene, Sun};
pub use schedule::{Scheduler, TimerId};
pub use shader::{Param, ParamType, ShaderData, ShaderId, ShaderKind};
pub use text::{FontId, Fonts};
pub use time::Time;
pub use world::{WindowSettings, World};

/// Engine version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
