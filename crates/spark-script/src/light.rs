//! Point and spot lights: `light({...})`, `spawn(..., { light = {...} })`, `obj.light.intensity = ...`.

use mlua::{Lua, Table, UserData, UserDataFields, UserDataMethods, Value};
use spark_core::{Color, Light, LightKind, ObjectId};

use crate::convert::*;
use crate::types::{Obj, apply_props};

const LIGHT_KEYS: &str = "type, color, intensity, range, angle, softness";

fn default_light(kind: &str, what: &str) -> LuaResult<Light> {
    match kind {
        "point" | "omni" => Ok(Light::point(Color::WHITE, 1.0, 10.0)),
        "spot" | "cone" | "flashlight" => Ok(Light::spot(Color::WHITE, 1.0, 15.0, 45f32.to_radians(), 0.3)),
        other => Err(rt(format!("{what}: unknown light type '{other}' (use \"point\" or \"spot\")"))),
    }
}

fn is_light_key(k: &str) -> bool {
    matches!(k, "type" | "color" | "intensity" | "range" | "angle" | "softness")
}

fn set_light_key(l: &mut Light, k: &str, v: &Value, what: &str) -> LuaResult<()> {
    let w = format!("{what}.{k}");
    match k {
        "type" => {
            let mut fresh = default_light(&to_str(v, &w)?, what)?;
            fresh.color = l.color;
            fresh.intensity = l.intensity;
            *l = fresh;
        }
        "color" => l.color = to_color(v, &w)?,
        "intensity" => l.intensity = to_num(v, &w)?.max(0.0),
        "range" => l.range = to_num(v, &w)?.max(0.01),
        "angle" | "softness" => {
            let n = to_num(v, &w)?;
            let LightKind::Spot { angle, softness } = &mut l.kind else {
                return Err(rt(format!("{w}: only spot lights have '{k}' (set type = \"spot\")")));
            };
            if k == "angle" {
                *angle = n.clamp(1.0, 179.0).to_radians();
            } else {
                *softness = n.clamp(0.0, 1.0);
            }
        }
        _ => return Err(rt(format!("{what}: unknown field '{k}' (allowed: {LIGHT_KEYS})"))),
    }
    Ok(())
}

/// `true`, `"point"`, `"spot"` or `{ type, color, intensity, range, angle, softness }`.
pub(crate) fn to_light(v: &Value, what: &str) -> LuaResult<Light> {
    match v {
        Value::Boolean(true) => default_light("point", what),
        Value::String(s) => default_light(&s.to_string_lossy(), what),
        Value::Table(t) => {
            let kind: Option<String> = t.get("type")?;
            let mut l = default_light(kind.as_deref().unwrap_or("point"), what)?;
            for pair in t.pairs::<String, Value>() {
                let (k, v) = pair?;
                if k != "type" {
                    set_light_key(&mut l, &k, &v, what)?;
                }
            }
            Ok(l)
        }
        _ => Err(rt(format!("{what}: expected \"point\", \"spot\" or a table {{ type, color, range, ... }}, got {}", v.type_name()))),
    }
}

/// `obj.light`: live view of an object's light.
pub struct LightRef(ObjectId);

fn with_light<R>(lua: &Lua, id: ObjectId, f: impl FnOnce(&mut Light) -> LuaResult<R>) -> LuaResult<R> {
    with(lua, |w| {
        let o = w.scene.get_mut(id).ok_or_else(dead)?;
        let l = o.light.as_mut().ok_or_else(|| rt("light was removed from this object"))?;
        f(l)
    })
}

impl UserData for LightRef {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("type", |lua, this| {
            with_light(lua, this.0, |l| Ok(if matches!(l.kind, LightKind::Spot { .. }) { "spot" } else { "point" }))
        });
        f.add_field_method_get("color", |lua, this| with_light(lua, this.0, |l| Ok(crate::types::LuaColor(l.color))));
        f.add_field_method_get("intensity", |lua, this| with_light(lua, this.0, |l| Ok(l.intensity)));
        f.add_field_method_get("range", |lua, this| with_light(lua, this.0, |l| Ok(l.range)));
        f.add_field_method_get("angle", |lua, this| {
            with_light(lua, this.0, |l| Ok(match l.kind { LightKind::Spot { angle, .. } => Some(angle.to_degrees()), _ => None }))
        });
        f.add_field_method_get("softness", |lua, this| {
            with_light(lua, this.0, |l| Ok(match l.kind { LightKind::Spot { softness, .. } => Some(softness), _ => None }))
        });
        for key in ["type", "color", "intensity", "range", "angle", "softness"] {
            f.add_field_method_set(key, move |lua, this, v: Value| with_light(lua, this.0, |l| set_light_key(l, key, &v, "light")));
        }
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(mlua::MetaMethod::ToString, |lua, this, ()| {
            with_light(lua, this.0, |l| Ok(format!("Light({:?}, range {:.1}, intensity {:.2})", l.kind, l.range, l.intensity)))
        });
    }
}

pub(crate) fn add_object_fields<F: UserDataFields<Obj>>(f: &mut F) {
    f.add_field_method_get("light", |lua, this| {
        with(lua, |w| Ok(w.scene.get(this.0).ok_or_else(dead)?.light.is_some().then_some(LightRef(this.0))))
    });
    f.add_field_method_set("light", |lua, this, v: Value| {
        let light = if v.is_nil() || v == Value::Boolean(false) { None } else { Some(to_light(&v, "obj.light")?) };
        with(lua, |w| {
            w.scene.get_mut(this.0).ok_or_else(dead)?.light = light;
            Ok(())
        })
    });
}

pub(crate) fn install(lua: &Lua, g: &Table) -> LuaResult<()> {
    g.set(
        "light",
        lua.create_function(|lua, props: Option<Table>| {
            let rest = lua.create_table()?;
            let spec = lua.create_table()?;
            if let Some(p) = &props {
                for pair in p.pairs::<String, Value>() {
                    let (k, v) = pair?;
                    if is_light_key(&k) { spec.set(k, v)? } else { rest.set(k, v)? }
                }
            }
            let light = to_light(&Value::Table(spec), "light")?;
            with(lua, |w| {
                let id = w.spawn_empty();
                w.scene[id].light = Some(light);
                if let Err(e) = apply_props(w, id, Some(rest)) {
                    w.scene.destroy(id);
                    return Err(e);
                }
                Ok(Obj(id))
            })
        })?,
    )?;
    Ok(())
}
