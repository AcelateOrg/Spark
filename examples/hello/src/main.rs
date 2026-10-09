//! Spark in pure Rust. Run: cargo run -p hello   (1 = plain, 2 = low-res pixels, 3 = custom grayscale post shader)
//! Headless check: cargo run -p hello -- --headless --frames 60 --screenshot screenshots/hello.png

use spark::prelude::*;

/// A post shader is plain WGSL: the engine adds the uniforms and helpers (`input_color`, ...).
const GRAYSCALE: &str = "
struct Params { amount: f32 }
fn post(p: PostInput) -> vec4<f32> {
    let c = input_color(p.uv).rgb;
    let g = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec4<f32>(mix(c, vec3<f32>(g), params.amount), 1.0);
}
";

#[derive(Default)]
struct Hello {
    cubes: Vec<ObjectId>,
    grayscale: Option<ShaderId>,
}

impl Game for Hello {
    fn start(&mut self, world: &mut World) {
        world.scene.background = Color::SKY;

        let checker = world.assets.add_texture(ImageData::checker(64, 2, Color::hex(0x6fae4f), Color::hex(0x5d9a42)));
        let plane = world.add_mesh(MeshData::plane(30.0, 30.0));
        let ground = world.spawn(plane, Material::textured(checker).with_tiling(15.0, 15.0));
        world.scene[ground].name = "ground".into();

        let cube = world.add_mesh(MeshData::cube(1.0));
        let colors = [Color::RED, Color::ORANGE, Color::YELLOW, Color::GREEN, Color::BLUE];
        for (i, color) in colors.into_iter().enumerate() {
            let id = world.spawn(cube, Material::color(color));
            world.scene[id].name = format!("cube{i}");
            world.scene[id].position = vec3(i as f32 * 1.8 - 3.6, 0.0, 0.0);
            world.scene.place_on(id, ground).unwrap();
            self.cubes.push(id);
        }

        let gray = world.assets.add_shader("grayscale", GRAYSCALE).expect("valid WGSL");
        world.assets.shader_mut(gray).unwrap().set_param("amount", &[0.85]).unwrap();
        self.grayscale = Some(gray);

        world.scene.camera.position = vec3(0.0, 3.0, 7.0);
        world.scene.camera.look_at(vec3(0.0, 0.5, 0.0));
    }

    fn update(&mut self, world: &mut World, dt: f32) {
        for (i, &id) in self.cubes.iter().enumerate() {
            world.scene[id].rotate_y(dt * (0.6 + i as f32 * 0.3));
        }
        if world.input.pressed(Key::Num1) {
            world.render = RenderSettings::default();
        }
        if world.input.pressed(Key::Num2) {
            world.render = RenderSettings { height: Some(180), upscale: TextureFilter::Nearest, filter: TextureFilter::Nearest, ..Default::default() };
        }
        if world.input.pressed(Key::Num3) {
            let shader = self.grayscale.expect("loaded in start");
            world.render = RenderSettings { post: vec![PostPass { shader, size: PassSize::Screen }], ..Default::default() };
        }
        if world.input.pressed(Key::Escape) {
            world.quit();
        }
    }
}

fn main() {
    App::new("Spark - Hello").run(Hello::default());
}
