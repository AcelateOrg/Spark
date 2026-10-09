//! Scripts: glTF models (`Model.load`, `spawn(model)`), animation playback, `obj:find`, `obj:paint`,
//! and the character controller (`obj:move`).

use mlua::{Lua, MetaMethod, Table, UserData, UserDataFields, UserDataMethods, Value};
use spark_core::animation::{self, AnimParams};
use spark_core::{Material, ModelId, ObjectId, World};

use crate::convert::*;
use crate::types::Obj;

/// Handle to a loaded model.
#[derive(Clone, Copy)]
pub struct LuaModel(pub ModelId);

impl UserData for LuaModel {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("name", |lua, this| {
            with(lua, |w| Ok(w.assets.model(this.0).map(|m| m.name.clone()).unwrap_or_default()))
        });
        f.add_field_method_get("animations", |lua, this| {
            with(lua, |w| Ok(w.assets.model(this.0).map(|m| m.clip_names()).unwrap_or_default()))
        });
        f.add_field_method_get("size", |lua, this| {
            with(lua, |w| Ok(vv(w.assets.model(this.0).map(|m| m.bounds.size()).unwrap_or_default())))
        });
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("duration", |lua, this, name: String| {
            with(lua, |w| {
                let model = w.assets.model(this.0).ok_or_else(|| rt("model was unloaded"))?;
                let c = model.clip(&name).ok_or_else(|| rt(format!("duration: no animation '{name}' (available: {})", model.clip_names().join(", "))))?;
                Ok(model.clips[c].duration)
            })
        });
        m.add_meta_method(MetaMethod::ToString, |lua, this, ()| {
            with(lua, |w| Ok(format!("Model(\"{}\")", w.assets.model(this.0).map(|m| m.name.as_str()).unwrap_or("?"))))
        });
    }
}

pub(crate) fn is_model_path(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.ends_with(".glb") || l.ends_with(".gltf")
}

/// A `Model` userdata or a `"file.glb"` path.
pub(crate) fn to_model(w: &mut World, v: &Value, what: &str) -> LuaResult<Option<ModelId>> {
    match v {
        Value::UserData(ud) => Ok(ud.borrow::<LuaModel>().ok().map(|m| m.0)),
        Value::String(s) => {
            let s = s.to_string_lossy();
            if is_model_path(&s) { w.assets.load_model(&s).map(Some).map_err(|e| rt(format!("{what}: {e}"))) } else { Ok(None) }
        }
        _ => Ok(None),
    }
}

/// `spawn(model, [material], [props])`.
pub(crate) fn spawn_model(lua: &Lua, model: ModelId, second: Value, third: Option<Table>) -> LuaResult<Obj> {
    let (material, props) = match second {
        Value::Nil => (None, third),
        Value::Table(t) => (None, Some(t)),
        other => (Some(to_material(&other, "spawn")?), third),
    };
    with(lua, |w| {
        let id = animation::spawn_model(w, model, material).ok_or_else(|| rt("spawn: model was unloaded"))?;
        if let Err(e) = crate::types::apply_props(w, id, props) {
            w.scene.destroy(id);
            return Err(e);
        }
        Ok(Obj(id))
    })
}

fn find_in(w: &World, id: ObjectId, name: &str, depth: u32) -> Option<ObjectId> {
    if depth > 64 {
        return None;
    }
    let children = w.scene.children(id);
    if let Some(&c) = children.iter().find(|&&c| w.scene.get(c).is_some_and(|o| o.name == name)) {
        return Some(c);
    }
    children.into_iter().find_map(|c| find_in(w, c, name, depth + 1))
}

fn paint(w: &mut World, id: ObjectId, m: Material, depth: u32) {
    if depth > 64 {
        return;
    }
    if let Some(o) = w.scene.get_mut(id) {
        if o.mesh.is_some() {
            o.material = m;
        }
    }
    for c in w.scene.children(id) {
        paint(w, c, m, depth + 1);
    }
}

fn anim_params(opts: Option<Table>) -> LuaResult<AnimParams> {
    let mut p = AnimParams::default();
    let Some(t) = opts else { return Ok(p) };
    for pair in t.pairs::<String, Value>() {
        let (k, v) = pair?;
        let w = format!("play option '{k}'");
        match k.as_str() {
            "loop" => p.looping = to_bool(&v, &w)?,
            "speed" => p.speed = to_num(&v, &w)?,
            "fade" => p.fade = to_num(&v, &w)?,
            "restart" => p.restart = to_bool(&v, &w)?,
            _ => return Err(rt(format!("play: unknown option '{k}' (allowed: loop, speed, fade, restart)"))),
        }
    }
    Ok(p)
}

pub(crate) fn add_object_methods<M: UserDataMethods<Obj>>(m: &mut M) {
    m.add_method("play", |lua, this, (name, opts): (String, Option<Table>)| {
        let p = anim_params(opts)?;
        with(lua, |w| animation::play(w, this.0, &name, p).map_err(rt))
    });
    m.add_method("stop", |lua, this, fade: Option<f32>| {
        with(lua, |w| {
            animation::stop(w, this.0, fade.unwrap_or(0.2));
            Ok(())
        })
    });
    m.add_method("is_playing", |lua, this, name: Option<String>| with(lua, |w| Ok(animation::is_playing(w, this.0, name.as_deref()))));
    m.add_method("animation", |lua, this, ()| with(lua, |w| Ok(animation::current(w, this.0))));
    m.add_method("animation_time", |lua, this, ()| with(lua, |w| Ok(animation::time(w, this.0))));
    m.add_method("set_animation_speed", |lua, this, s: f32| {
        with(lua, |w| {
            animation::set_speed(w, this.0, s);
            Ok(())
        })
    });
    m.add_method("animations", |lua, this, ()| {
        with(lua, |w| {
            let names = w.animator.instance(this.0).and_then(|i| w.assets.model(i.model)).map(|m| m.clip_names());
            Ok(names.unwrap_or_default())
        })
    });
    m.add_method("find", |lua, this, name: String| with(lua, |w| Ok(find_in(w, this.0, &name, 0).map(Obj))));
    m.add_method("paint", |lua, this, v: Value| {
        let mat = to_material(&v, "paint")?;
        with(lua, |w| {
            paint(w, this.0, mat, 0);
            Ok(())
        })
    });
    m.add_method("move", |lua, this, delta: Value| {
        let d = to_vec3(&delta, "move")?;
        with(lua, |w| {
            let o = w.scene.get(this.0).ok_or_else(dead)?;
            if o.body.is_none() {
                return Err(rt(
                    "move: the object needs a body, e.g. obj:add_body({ type = \"kinematic\", shape = \"capsule\", radius = 0.3, height = 1.8 })",
                ));
            }
            let (_, rot, pos) = w.scene.world_matrix(this.0).to_scale_rotation_translation();
            let dt = w.time.dt;
            let (t, grounded) = match w.physics.move_character(this.0, pos, rot, d, dt) {
                Some(m) => (m.translation, m.grounded),
                None => (d, false),
            };
            w.scene.translate_world(this.0, t);
            Ok((vv(t), grounded))
        })
    });
}

pub(crate) fn install(lua: &Lua, g: &Table) -> LuaResult<()> {
    let model = lua.create_table()?;
    model.set(
        "load",
        lua.create_function(|lua, path: String| with(lua, |w| w.assets.load_model(&path).map(LuaModel).map_err(rt)))?,
    )?;
    g.set("Model", model)?;
    Ok(())
}
