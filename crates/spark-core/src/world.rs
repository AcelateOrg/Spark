use crate::animation::Animator;
use crate::assets::{Assets, MeshId};
use crate::audio::Audio;
use crate::branding::Branding;
use crate::canvas::Canvas;
use crate::physics::Physics;
use crate::input::Input;
use crate::material::Material;
use crate::mesh::MeshData;
use crate::scene::{Object, ObjectId, Scene};
use crate::render::RenderSettings;
use crate::time::Time;

/// Window state a game can change at runtime. The app applies changes every frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowSettings {
    /// Borderless fullscreen on the current monitor (F11 / Alt+Enter toggle it too).
    pub fullscreen: bool,
    /// Window title (`None` = the app's title).
    pub title: Option<String>,
}

/// Everything the game can touch. Pure data: renderer, window and scripts read/write it.
#[derive(Debug, Default)]
pub struct World {
    pub scene: Scene,
    pub assets: Assets,
    pub input: Input,
    pub time: Time,
    /// Resolution, texture sampling, custom shaders and post passes.
    pub render: RenderSettings,
    pub physics: Physics,
    pub audio: Audio,
    /// Immediate-mode 2D drawing, cleared every frame.
    pub canvas: Canvas,
    /// Spark splash / badge state (see [`crate::branding`]).
    pub branding: Branding,
    /// Spawned glTF models and their animations (see [`crate::animation`]).
    pub animator: Animator,
    /// Fullscreen / title requests for the window (ignored in headless runs).
    pub window: WindowSettings,
    quit: bool,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an object with a mesh and material. Returns its handle.
    pub fn spawn(&mut self, mesh: MeshId, material: Material) -> ObjectId {
        let mut object = Object::empty();
        object.mesh = Some(mesh);
        object.material = material;
        object.local_bounds = self.assets.mesh_bounds(mesh);
        self.scene.add(object)
    }

    /// Adds an empty object (group / pivot) to parent other objects to.
    pub fn spawn_empty(&mut self) -> ObjectId {
        self.scene.add(Object::empty())
    }

    /// Shortcut for `assets.add_mesh`.
    pub fn add_mesh(&mut self, mesh: MeshData) -> MeshId {
        self.assets.add_mesh(mesh)
    }

    /// Changes an object's mesh (updates bounds).
    pub fn set_mesh(&mut self, id: ObjectId, mesh: MeshId) {
        let bounds = self.assets.mesh_bounds(mesh);
        if let Some(o) = self.scene.get_mut(id) {
            o.mesh = Some(mesh);
            o.local_bounds = bounds;
        }
    }

    /// Ask the app to close after this frame.
    pub fn quit(&mut self) {
        self.quit = true;
    }

    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// Clears scene, assets, render settings, bodies and sounds (keeps input, time, backends and asset root).
    /// Used by hot reload.
    pub fn reset(&mut self) {
        self.scene = Scene::new();
        self.physics.clear();
        self.physics.gravity = Physics::default().gravity;
        self.physics.enabled = true;
        self.audio.stop_all(0.0);
        self.audio.clear_cache();
        self.assets.clear();
        self.animator.clear();
        self.render = RenderSettings::default();
        self.canvas.reset();
    }
}
