//! Physics for scripts: bodies on objects, raycasts, gravity, collision events.

use mlua::{Lua, MultiValue, Table, UserData, UserDataFields, UserDataMethods, Value};
use spark_core::{Body, BodyKind, ObjectId, Shape, Vec3, World};

use crate::convert::*;
use crate::runtime::{self, object_event};
use crate::types::Obj;

const BODY_KEYS: &str = "type, shape, size, radius, height, mass, friction, bounciness, sensor, lock_rotation, \
                         gravity_scale, damping, angular_damping, ccd, velocity, angular_velocity";

fn kind_of(v: &Value, what: &str) -> LuaResult<BodyKind> {
    let s = to_str(v, what)?;
    BodyKind::parse(&s).ok_or_else(|| rt(format!("{what}: unknown body type '{s}' (use \"dynamic\", \"static\" or \"kinematic\")")))
}

/// `"dynamic"` / `"static"` / `"kinematic"` / `true` (dynamic) / `{ type = ..., mass = ..., ... }`.
pub(crate) fn to_body(v: &Value, what: &str) -> LuaResult<Body> {
    match v {
        Value::Boolean(true) => Ok(Body::dynamic()),
        Value::String(_) => Ok(Body::new(kind_of(v, what)?)),
        Value::Table(t) => body_from_table(t, what),
        _ => Err(rt(format!("{what}: expected \"dynamic\", \"static\", \"kinematic\" or a table, got {}", v.type_name()))),
    }
}

fn body_from_table(t: &Table, what: &str) -> LuaResult<Body> {
    let mut body = Body::dynamic();
    let (mut shape, mut size, mut radius, mut height) = (None::<String>, None, None, None);
    for pair in t.pairs::<String, Value>() {
        let (k, v) = pair?;
        let w = format!("{what}.{k}");
        match k.as_str() {
            "type" | "kind" => body.kind = kind_of(&v, &w)?,
            "shape" => shape = Some(to_str(&v, &w)?),
            "size" => size = Some(to_scale(&v, &w)?),
            "radius" => radius = Some(to_num(&v, &w)?),
            "height" => height = Some(to_num(&v, &w)?),
            "mass" => body.mass = Some(to_num(&v, &w)?),
            "friction" => body.friction = to_num(&v, &w)?,
            "bounciness" | "bounce" => body.bounciness = to_num(&v, &w)?,
            "sensor" => body.sensor = to_bool(&v, &w)?,
            "lock_rotation" => body.lock_rotation = to_bool(&v, &w)?,
            "gravity_scale" => body.gravity_scale = to_num(&v, &w)?,
            "damping" => body.linear_damping = to_num(&v, &w)?,
            "angular_damping" => body.angular_damping = to_num(&v, &w)?,
            "ccd" => body.ccd = to_bool(&v, &w)?,
            "velocity" => body.velocity = to_vec3(&v, &w)?,
            "angular_velocity" => body.angular_velocity = to_vec3(&v, &w)?,
            _ => return Err(rt(format!("{what}: unknown key '{k}' (allowed: {BODY_KEYS})"))),
        }
    }
    let r = radius.unwrap_or(0.5);
    let h = height.unwrap_or(1.0);
    body.shape = match shape.as_deref() {
        None | Some("auto") => match size {
            Some(s) => Shape::Box(s),
            None => Shape::Auto,
        },
        Some("box") => Shape::Box(size.unwrap_or(Vec3::ONE)),
        Some("sphere") | Some("ball") => Shape::Sphere(r),
        Some("capsule") => Shape::Capsule { radius: r, height: height.unwrap_or(2.0) },
        Some("cylinder") => Shape::Cylinder { radius: r, height: h },
        Some(other) => {
            return Err(rt(format!("{what}.shape: unknown shape '{other}' (use auto, box, sphere, capsule, cylinder)")));
        }
    };
    Ok(body)
}

fn body_mut<R>(lua: &Lua, id: ObjectId, what: &str, f: impl FnOnce(&mut Body) -> R) -> LuaResult<R> {
    with(lua, |w| {
        let o = w.scene.get_mut(id).ok_or_else(dead)?;
        let b = o.body.as_mut().ok_or_else(|| rt(format!("{what}: object has no body (add one with obj:add_body() or spawn(..., {{ body = \"dynamic\" }}))")))?;
        Ok(f(b))
    })
}

fn body_ref<R>(lua: &Lua, id: ObjectId, f: impl FnOnce(Option<&Body>) -> R) -> LuaResult<R> {
    with(lua, |w| {
        let o = w.scene.get(id).ok_or_else(dead)?;
        Ok(f(o.body.as_ref()))
    })
}

pub(crate) fn add_object_fields<F: UserDataFields<Obj>>(f: &mut F) {
    f.add_field_method_get("body", |lua, this| body_ref(lua, this.0, |b| b.map(|b| b.kind.name())));
    f.add_field_method_set("body", |lua, this, v: Value| {
        let body = if v.is_nil() || v == Value::Boolean(false) { None } else { Some(to_body(&v, "obj.body")?) };
        with(lua, |w| {
            w.scene.get_mut(this.0).ok_or_else(dead)?.body = body;
            Ok(())
        })
    });
    f.add_field_method_get("velocity", |lua, this| body_ref(lua, this.0, |b| vv(b.map(|b| b.velocity).unwrap_or(Vec3::ZERO))));
    f.add_field_method_set("velocity", |lua, this, v: Value| {
        let v = to_vec3(&v, "obj.velocity")?;
        body_mut(lua, this.0, "obj.velocity", |b| b.velocity = v)
    });
    f.add_field_method_get("angular_velocity", |lua, this| {
        body_ref(lua, this.0, |b| vv(b.map(|b| b.angular_velocity).unwrap_or(Vec3::ZERO)))
    });
    f.add_field_method_set("angular_velocity", |lua, this, v: Value| {
        let v = to_vec3(&v, "obj.angular_velocity")?;
        body_mut(lua, this.0, "obj.angular_velocity", |b| b.angular_velocity = v)
    });
    f.add_field_method_get("mass", |lua, this| body_ref(lua, this.0, |b| b.and_then(|b| b.mass)));
    f.add_field_method_set("mass", |lua, this, v: Option<f32>| body_mut(lua, this.0, "obj.mass", |b| b.mass = v));
    f.add_field_method_get("gravity_scale", |lua, this| body_ref(lua, this.0, |b| b.map(|b| b.gravity_scale)));
    f.add_field_method_set("gravity_scale", |lua, this, v: f32| body_mut(lua, this.0, "obj.gravity_scale", |b| b.gravity_scale = v));
    f.add_field_method_get("sensor", |lua, this| body_ref(lua, this.0, |b| b.is_some_and(|b| b.sensor)));
    f.add_field_method_set("sensor", |lua, this, v: bool| body_mut(lua, this.0, "obj.sensor", |b| b.sensor = v));
}

pub(crate) fn add_object_methods<M: UserDataMethods<Obj>>(m: &mut M) {
    m.add_method("add_body", |lua, this, spec: Option<Value>| {
        let body = match spec {
            None => Body::dynamic(),
            Some(v) => to_body(&v, "add_body")?,
        };
        with(lua, |w| {
            w.scene.get_mut(this.0).ok_or_else(dead)?.body = Some(body);
            Ok(())
        })
    });
    m.add_method("remove_body", |lua, this, ()| {
        with(lua, |w| {
            w.scene.get_mut(this.0).ok_or_else(dead)?.body = None;
            Ok(())
        })
    });
    m.add_method("impulse", |lua, this, v: Value| {
        let v = to_vec3(&v, "impulse")?;
        body_mut(lua, this.0, "impulse", |b| b.impulse += v)
    });
    m.add_method("force", |lua, this, v: Value| {
        let v = to_vec3(&v, "force")?;
        body_mut(lua, this.0, "force", |b| b.force += v)
    });
    m.add_method("torque", |lua, this, v: Value| {
        let v = to_vec3(&v, "torque")?;
        body_mut(lua, this.0, "torque", |b| b.torque_impulse += v)
    });
}

/// The global `physics` object.
struct PhysicsRef;

impl UserData for PhysicsRef {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("gravity", |lua, _| with(lua, |w| Ok(vv(w.physics.gravity))));
        f.add_field_method_set("gravity", |lua, _, v: Value| {
            let g = to_vec3(&v, "physics.gravity")?;
            with(lua, |w| Ok(w.physics.gravity = g))
        });
        f.add_field_method_get("enabled", |lua, _| with(lua, |w| Ok(w.physics.enabled)));
        f.add_field_method_set("enabled", |lua, _, v: bool| with(lua, |w| Ok(w.physics.enabled = v)));
    }
}

fn raycast(lua: &Lua, (origin, dir, max, opts): (Value, Value, Option<f32>, Option<Table>)) -> LuaResult<Value> {
    let ignore = match &opts {
        Some(t) => crate::types::to_parent(&t.get::<Value>("ignore")?, "raycast ignore")?,
        None => None,
    };
    let hit = with(lua, |w| {
        let o = to_point(w, &origin, "raycast origin")?;
        let d = to_vec3(&dir, "raycast direction")?;
        Ok(w.physics.raycast(o, d, max.unwrap_or(1000.0), ignore))
    })?;
    let Some(hit) = hit else { return Ok(Value::Nil) };
    let t = lua.create_table()?;
    t.set("object", Obj(hit.object))?;
    t.set("point", vv(hit.point))?;
    t.set("normal", vv(hit.normal))?;
    t.set("distance", hit.distance)?;
    Ok(Value::Table(t))
}

/// `pick(origin, dir, max?, {ignore = obj})`: ray vs rendered triangles (no colliders needed).
fn pick(lua: &Lua, (origin, dir, max, opts): (Value, Value, Option<f32>, Option<Table>)) -> LuaResult<Value> {
    let ignore = match &opts {
        Some(t) => crate::types::to_parent(&t.get::<Value>("ignore")?, "pick ignore")?,
        None => None,
    };
    let hit = with(lua, |w| {
        let o = to_point(w, &origin, "pick origin")?;
        let d = to_vec3(&dir, "pick direction")?;
        Ok(w.pick(o, d, max.unwrap_or(1000.0), ignore))
    })?;
    let Some(hit) = hit else { return Ok(Value::Nil) };
    let t = lua.create_table()?;
    t.set("object", Obj(hit.object))?;
    t.set("point", vv(hit.point))?;
    t.set("normal", vv(hit.normal))?;
    t.set("distance", hit.distance)?;
    Ok(Value::Table(t))
}

pub(crate) fn install(lua: &Lua, g: &Table) -> LuaResult<()> {
    g.set("physics", lua.create_userdata(PhysicsRef)?)?;
    g.set("pick", lua.create_function(pick)?)?;
    g.set("raycast", lua.create_function(raycast)?)?;
    Ok(())
}

fn has_backend(w: &World) -> bool {
    w.physics.has_backend()
}

/// Delivers collision events of the last physics steps:
/// `obj:on("collision", fn(other, info))`, `"collision_end"`, and global `on("collision", fn(a, b, info))`.
pub(crate) fn dispatch_collisions(lua: &Lua) -> LuaResult<()> {
    let events = with(lua, |w| Ok(if has_backend(w) { std::mem::take(&mut w.physics.events) } else { Vec::new() }))?;
    if events.is_empty() {
        return Ok(());
    }
    let rtc = runtime::runtime(lua)?;
    for c in events {
        let name = if c.started { "collision" } else { "collision_end" };
        let info = lua.create_table()?;
        info.set("sensor", c.sensor)?;
        info.set("speed", c.speed)?;
        let listeners = |n: &str| rtc.borrow().bus.listener_count(n) > 0;
        for (me, other) in [(c.a, c.b), (c.b, c.a)] {
            let ev = object_event(name, me);
            if listeners(&ev) {
                let args = MultiValue::from_vec(vec![Value::UserData(lua.create_userdata(Obj(other))?), Value::Table(info.clone())]);
                runtime::emit(lua, &ev, args)?;
            }
        }
        if listeners(name) {
            let args = MultiValue::from_vec(vec![
                Value::UserData(lua.create_userdata(Obj(c.a))?),
                Value::UserData(lua.create_userdata(Obj(c.b))?),
                Value::Table(info),
            ]);
            runtime::emit(lua, name, args)?;
        }
    }
    Ok(())
}
