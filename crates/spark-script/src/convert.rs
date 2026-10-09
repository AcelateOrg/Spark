//! Shared world access + Lua value conversions with AI-friendly error messages.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Value, Vector};
use spark_core::{Color, EulerRot, Material, Quat, TextureFilter, Vec3, World};

use crate::types::{LuaColor, LuaMaterial, LuaTexture, Obj};

pub(crate) type LuaResult<T> = mlua::Result<T>;

/// The world the script may touch (swapped in around every Lua call).
pub(crate) struct Shared(pub Rc<RefCell<World>>);

pub(crate) fn rt<S: std::fmt::Display>(msg: S) -> mlua::Error {
    mlua::Error::runtime(msg)
}

pub(crate) fn with<R>(lua: &Lua, f: impl FnOnce(&mut World) -> LuaResult<R>) -> LuaResult<R> {
    let rc = lua.app_data_ref::<Shared>().ok_or_else(|| rt("spark: world is not attached"))?.0.clone();
    let mut w = rc.try_borrow_mut().map_err(|_| rt("spark: world is busy (nested call)"))?;
    f(&mut w)
}

pub(crate) fn dead() -> mlua::Error {
    rt("object was destroyed (check obj:exists() before using it)")
}

pub(crate) fn vv(v: Vec3) -> Vector {
    Vector::new(v.x, v.y, v.z)
}

pub(crate) fn to_num(v: &Value, what: &str) -> LuaResult<f32> {
    match v {
        Value::Number(n) => Ok(*n as f32),
        Value::Integer(i) => Ok(*i as f32),
        _ => Err(rt(format!("{what}: expected a number, got {}", v.type_name()))),
    }
}

pub(crate) fn to_bool(v: &Value, what: &str) -> LuaResult<bool> {
    match v {
        Value::Boolean(b) => Ok(*b),
        Value::Nil => Ok(false),
        _ => Err(rt(format!("{what}: expected true/false, got {}", v.type_name()))),
    }
}

pub(crate) fn to_str(v: &Value, what: &str) -> LuaResult<String> {
    match v {
        Value::String(s) => Ok(s.to_string_lossy()),
        _ => Err(rt(format!("{what}: expected a string, got {}", v.type_name()))),
    }
}

pub(crate) fn to_vec3(v: &Value, what: &str) -> LuaResult<Vec3> {
    match v {
        Value::Vector(v) => Ok(Vec3::new(v.x(), v.y(), v.z())),
        Value::Table(t) => {
            let get = |a: &str, i: i64| -> LuaResult<f32> {
                let v: Value = t.get(a)?;
                let v = if v.is_nil() { t.get(i)? } else { v };
                if v.is_nil() { Ok(0.0) } else { to_num(&v, what) }
            };
            Ok(Vec3::new(get("x", 1)?, get("y", 2)?, get("z", 3)?))
        }
        _ => Err(rt(format!("{what}: expected a vector like vec3(1, 2, 3), got {}", v.type_name()))),
    }
}

/// Scale: a number (uniform) or a vector.
pub(crate) fn to_scale(v: &Value, what: &str) -> LuaResult<Vec3> {
    match v {
        Value::Number(_) | Value::Integer(_) => Ok(Vec3::splat(to_num(v, what)?)),
        _ => to_vec3(v, what),
    }
}

/// A world-space point: a vector or an object (its world position).
pub(crate) fn to_point(w: &World, v: &Value, what: &str) -> LuaResult<Vec3> {
    if let Value::UserData(ud) = v {
        if let Ok(o) = ud.borrow::<Obj>() {
            if !w.scene.contains(o.0) {
                return Err(dead());
            }
            return Ok(w.scene.world_position(o.0));
        }
    }
    to_vec3(v, what)
}

fn color_names() -> String {
    Color::NAMED.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
}

/// Accepts Color, 0xff8800, "#ff8800", "red", vec3(r, g, b) or {r, g, b, a}.
pub(crate) fn to_color(v: &Value, what: &str) -> LuaResult<Color> {
    match v {
        Value::UserData(ud) => {
            if let Ok(c) = ud.borrow::<LuaColor>() {
                return Ok(c.0);
            }
            if let Ok(m) = ud.borrow::<LuaMaterial>() {
                return Ok(m.0.color);
            }
            Err(rt(format!("{what}: expected a color")))
        }
        Value::Number(_) | Value::Integer(_) => Ok(Color::hex(to_num(v, what)? as u32)),
        Value::String(s) => {
            let s = s.to_string_lossy();
            Color::parse(&s).ok_or_else(|| {
                rt(format!("{what}: unknown color '{s}' (use \"#rrggbb\", 0xrrggbb or a name: {})", color_names()))
            })
        }
        Value::Vector(c) => Ok(Color::rgb(c.x(), c.y(), c.z())),
        Value::Table(t) => {
            let get = |a: &str, i: i64, d: f32| -> LuaResult<f32> {
                let v: Value = t.get(a)?;
                let v = if v.is_nil() { t.get(i)? } else { v };
                if v.is_nil() { Ok(d) } else { to_num(&v, what) }
            };
            Ok(Color::rgba(get("r", 1, 0.0)?, get("g", 2, 0.0)?, get("b", 3, 0.0)?, get("a", 4, 1.0)?))
        }
        _ => Err(rt(format!("{what}: expected a color like \"#ff8800\" or \"red\", got {}", v.type_name()))),
    }
}

/// Accepts Material, Texture or anything [`to_color`] accepts. `nil` = white.
pub(crate) fn to_material(v: &Value, what: &str) -> LuaResult<Material> {
    match v {
        Value::Nil => Ok(Material::default()),
        Value::UserData(ud) => {
            if let Ok(m) = ud.borrow::<LuaMaterial>() {
                return Ok(m.0);
            }
            if let Ok(t) = ud.borrow::<LuaTexture>() {
                return Ok(Material::textured(t.0));
            }
            Ok(Material::color(to_color(v, what)?))
        }
        _ => Ok(Material::color(to_color(v, what)?)),
    }
}

pub(crate) fn to_filter(v: &Value, what: &str) -> LuaResult<TextureFilter> {
    match to_str(v, what)?.to_ascii_lowercase().as_str() {
        "nearest" | "pixel" => Ok(TextureFilter::Nearest),
        "linear" | "smooth" => Ok(TextureFilter::Linear),
        other => Err(rt(format!("{what}: unknown filter '{other}' (use \"nearest\" or \"linear\")"))),
    }
}

/// Quaternion -> (pitch, yaw, roll) radians.
pub(crate) fn euler(q: Quat) -> Vec3 {
    let (y, x, z) = q.to_euler(EulerRot::YXZ);
    Vec3::new(x, y, z)
}

pub(crate) fn from_euler(v: Vec3) -> Quat {
    Quat::from_euler(EulerRot::YXZ, v.y, v.x, v.z)
}
