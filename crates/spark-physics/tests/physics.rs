use spark_core::{Body, Game, Material, MeshData, Vec3, World, run_frame};
use spark_physics::RapierBackend;

struct Nothing;
impl Game for Nothing {}

fn setup() -> World {
    let mut world = World::new();
    world.physics.set_backend(Box::new(RapierBackend::new()));
    let plane = world.add_mesh(MeshData::plane(20.0, 20.0));
    let ground = world.spawn(plane, Material::default());
    world.scene[ground].body = Some(Body::fixed());
    world
}

fn run(world: &mut World, seconds: f32) -> usize {
    let mut events = 0;
    let frames = (seconds * 60.0) as usize;
    for _ in 0..frames {
        run_frame(&mut Nothing, world, 1.0 / 60.0);
        events += world.physics.events.len();
    }
    events
}

#[test]
fn box_falls_and_lands_on_plane() {
    let mut world = setup();
    let cube = world.add_mesh(MeshData::cube(1.0));
    let id = world.spawn(cube, Material::default());
    world.scene[id].position = Vec3::new(0.0, 5.0, 0.0);
    world.scene[id].body = Some(Body::dynamic());

    let events = run(&mut world, 3.0);
    let y = world.scene[id].position.y;
    assert!((y - 0.5).abs() < 0.05, "cube rests on the plane, y = {y}");
    assert!(events >= 1, "landing produced a collision event");
    let v = world.scene[id].body.as_ref().unwrap().velocity;
    assert!(v.length() < 0.1, "cube is at rest, v = {v}");

    let hit = world.physics.raycast(Vec3::new(0.0, 10.0, 0.0), Vec3::NEG_Y, 100.0, None).expect("hit");
    assert_eq!(hit.object, id);
    assert!((hit.point.y - 1.0).abs() < 0.05, "top of the cube, {:?}", hit.point);
    let hit = world.physics.raycast(Vec3::new(0.0, 10.0, 0.0), Vec3::NEG_Y, 100.0, Some(id)).expect("hit");
    assert!(hit.point.y.abs() < 0.01, "ignore skips the cube, {:?}", hit.point);
}

#[test]
fn teleport_velocity_impulse_and_destroy() {
    let mut world = setup();
    let ball = world.add_mesh(MeshData::sphere(0.5, 12));
    let id = world.spawn(ball, Material::default());
    world.scene[id].position = Vec3::new(0.0, 0.5, 0.0);
    world.scene[id].body = Some(Body { gravity_scale: 0.0, ..Body::dynamic() });
    run(&mut world, 0.1);

    world.scene[id].position = Vec3::new(3.0, 4.0, 0.0);
    run(&mut world, 1.0 / 60.0);
    assert!(world.scene[id].position.distance(Vec3::new(3.0, 4.0, 0.0)) < 0.01, "teleported");

    world.scene[id].body.as_mut().unwrap().velocity = Vec3::new(2.0, 0.0, 0.0);
    run(&mut world, 1.0);
    let x = world.scene[id].position.x;
    assert!((x - 5.0).abs() < 0.1, "moved 2 m in 1 s, x = {x}");

    world.scene[id].body.as_mut().unwrap().velocity = Vec3::ZERO;
    run(&mut world, 1.0 / 60.0);
    world.scene[id].body.as_mut().unwrap().impulse = Vec3::new(0.0, 0.0, 1.0);
    run(&mut world, 1.0 / 60.0);
    assert!(world.scene[id].body.as_ref().unwrap().velocity.z > 0.1, "impulse applied");

    world.scene.destroy(id);
    run(&mut world, 1.0 / 60.0);
    assert!(world.physics.raycast(Vec3::new(3.0, 10.0, 0.0), Vec3::NEG_Y, 7.0, None).is_none());
}

#[test]
fn sensor_reports_overlap() {
    let mut world = setup();
    let cube = world.add_mesh(MeshData::cube(1.0));
    let zone = world.spawn(cube, Material::default());
    world.scene[zone].position = Vec3::new(0.0, 2.0, 0.0);
    world.scene[zone].scale = Vec3::splat(2.0);
    world.scene[zone].body = Some(Body { sensor: true, ..Body::fixed() });
    let id = world.spawn(cube, Material::default());
    world.scene[id].position = Vec3::new(0.0, 6.0, 0.0);
    world.scene[id].body = Some(Body::dynamic());

    let mut entered = false;
    let mut left = false;
    for _ in 0..180 {
        run_frame(&mut Nothing, &mut world, 1.0 / 60.0);
        for c in &world.physics.events {
            if c.sensor && (c.a == zone || c.b == zone) {
                if c.started { entered = true } else { left = true }
            }
        }
    }
    assert!(entered && left, "fell through the sensor: entered={entered} left={left}");
    assert!((world.scene[id].position.y - 0.5).abs() < 0.05);
}
