//! Sound for scripts: `sound.play(path, opts)`, handles, mixer buses, master volume.

use mlua::{Lua, MetaMethod, Table, UserData, UserDataFields, UserDataMethods, Value};
use spark_core::{BusId, PlayParams, SoundId, SoundSource};

use crate::convert::*;

/// Handle returned by `sound.play`.
#[derive(Clone, Copy)]
pub(crate) struct LuaSound(pub SoundId);

impl UserData for LuaSound {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("id", |_, this| Ok(this.0.0));
        f.add_field_method_get("playing", |lua, this| with(lua, |w| Ok(w.audio.is_playing(this.0))));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("stop", |lua, this, fade: Option<f32>| with(lua, |w| Ok(w.audio.stop(this.0, fade.unwrap_or(0.0)))));
        m.add_method("set_volume", |lua, this, (v, fade): (f32, Option<f32>)| {
            with(lua, |w| Ok(w.audio.set_volume(this.0, v, fade.unwrap_or(0.0))))
        });
        m.add_method("set_pitch", |lua, this, (v, fade): (f32, Option<f32>)| {
            with(lua, |w| Ok(w.audio.set_pitch(this.0, v, fade.unwrap_or(0.0))))
        });
        m.add_method("set_pan", |lua, this, v: f32| with(lua, |w| Ok(w.audio.set_pan(this.0, v))));
        m.add_method("set_position", |lua, this, v: Value| {
            let src = to_source(&v, "sound:set_position")?;
            with(lua, |w| {
                if !w.audio.is_spatial(this.0) {
                    return Err(rt("sound:set_position: only for 3D sounds (play with { at = position_or_object })"));
                }
                Ok(w.audio.set_source(this.0, src))
            })
        });
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("Sound#{}", this.0.0)));
    }
}

/// Handle returned by `sound.bus(name)`.
#[derive(Clone)]
pub(crate) struct LuaBus(pub BusId, pub String);

impl UserData for LuaBus {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("name", |_, this| Ok(this.1.clone()));
        f.add_field_method_get("volume", |lua, this| with(lua, |w| Ok(w.audio.bus_volume(this.0))));
        f.add_field_method_set("volume", |lua, this, v: f32| with(lua, |w| Ok(w.audio.set_bus_volume(this.0, v, 0.0))));
        f.add_field_method_get("paused", |lua, this| with(lua, |w| Ok(w.audio.bus_paused(this.0))));
        f.add_field_method_set("paused", |lua, this, v: bool| with(lua, |w| Ok(w.audio.pause_bus(this.0, v, 0.0))));
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("set_volume", |lua, this, (v, fade): (f32, Option<f32>)| {
            with(lua, |w| Ok(w.audio.set_bus_volume(this.0, v, fade.unwrap_or(0.0))))
        });
        m.add_method("stop", |lua, this, fade: Option<f32>| with(lua, |w| Ok(w.audio.stop_bus(this.0, fade.unwrap_or(0.0)))));
        m.add_method("pause", |lua, this, fade: Option<f32>| with(lua, |w| Ok(w.audio.pause_bus(this.0, true, fade.unwrap_or(0.0)))));
        m.add_method("resume", |lua, this, fade: Option<f32>| {
            with(lua, |w| Ok(w.audio.pause_bus(this.0, false, fade.unwrap_or(0.0))))
        });
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("Bus({})", this.1)));
        m.add_meta_method(MetaMethod::Eq, |_, this, other: mlua::UserDataRef<LuaBus>| Ok(this.0 == other.0));
    }
}

fn bus_of(lua: &Lua, v: &Value, what: &str) -> LuaResult<BusId> {
    match v {
        Value::UserData(ud) if ud.is::<LuaBus>() => Ok(ud.borrow::<LuaBus>()?.0),
        Value::String(s) => {
            let name = s.to_str()?.trim().to_string();
            if name.is_empty() {
                return Err(rt(format!("{what}: bus name is empty")));
            }
            with(lua, |w| Ok(w.audio.bus(&name)))
        }
        _ => Err(rt(format!("{what}: expected a bus name like \"music\" or sound.bus(...)"))),
    }
}

const PLAY_KEYS: &str = "volume, pitch, pan, loop, fade_in, at, range, bus";

fn to_source(v: &Value, what: &str) -> LuaResult<SoundSource> {
    match v {
        Value::UserData(ud) if ud.is::<crate::types::Obj>() => Ok(SoundSource::Object(ud.borrow::<crate::types::Obj>()?.0)),
        _ => Ok(SoundSource::Point(to_vec3(v, what).map_err(|_| rt(format!("{what}: expected a position vec3(...) or an Object")))?)),
    }
}

struct Spatial {
    at: Option<SoundSource>,
    range: f32,
}

fn params(lua: &Lua, opts: Option<Table>) -> LuaResult<(PlayParams, Spatial)> {
    let mut p = PlayParams::default();
    let mut s = Spatial { at: None, range: 20.0 };
    let Some(t) = opts else { return Ok((p, s)) };
    for pair in t.pairs::<String, Value>() {
        let (k, v) = pair?;
        let what = format!("sound.play option '{k}'");
        match k.as_str() {
            "volume" => p.volume = to_num(&v, &what)?,
            "pitch" => p.pitch = to_num(&v, &what)?,
            "pan" => p.pan = to_num(&v, &what)?,
            "loop" => p.looped = to_bool(&v, &what)?,
            "fade_in" => p.fade_in = to_num(&v, &what)?,
            "at" => s.at = Some(to_source(&v, &what)?),
            "range" => s.range = to_num(&v, &what)?.max(0.1),
            "bus" => p.bus = Some(bus_of(lua, &v, &what)?),
            _ => return Err(rt(format!("unknown {what} (allowed: {PLAY_KEYS})"))),
        }
    }
    Ok((p, s))
}

pub(crate) fn install(lua: &Lua, g: &Table) -> LuaResult<()> {
    let sound = lua.create_table()?;
    sound.set(
        "play",
        lua.create_function(|lua, (path, opts): (String, Option<Table>)| {
            let (p, s) = params(lua, opts)?;
            with(lua, |w| {
                let full = w.assets.resolve(&path);
                if !spark_core::vfs::is_file(&full) {
                    return Err(rt(format!("sound.play: file not found: '{path}' (looked at {})", full.display())));
                }
                Ok(LuaSound(match s.at {
                    Some(src) => w.audio.play_at(full, p, src, s.range),
                    None => w.audio.play(full, p),
                }))
            })
        })?,
    )?;
    sound.set(
        "stop_all",
        lua.create_function(|lua, fade: Option<f32>| with(lua, |w| Ok(w.audio.stop_all(fade.unwrap_or(0.0)))))?,
    )?;
    sound.set("enabled", lua.create_function(|lua, ()| with(lua, |w| Ok(w.audio.has_backend())))?)?;
    sound.set(
        "bus",
        lua.create_function(|lua, name: Value| {
            let id = bus_of(lua, &name, "sound.bus")?;
            let label = with(lua, |w| Ok(w.audio.bus_names()[id.0 as usize].to_string()))?;
            Ok(LuaBus(id, label))
        })?,
    )?;
    let meta = lua.create_table()?;
    meta.set(
        "__index",
        lua.create_function(|lua, (_, k): (Table, String)| match k.as_str() {
            "volume" => with(lua, |w| Ok(Value::Number(w.audio.master_volume() as f64))),
            _ => Err(rt(format!("sound.{k} does not exist (use sound.play, sound.bus, sound.stop_all, sound.volume, sound.enabled)"))),
        })?,
    )?;
    meta.set(
        "__newindex",
        lua.create_function(|lua, (_, k, v): (Table, String, Value)| match k.as_str() {
            "volume" => {
                let v = to_num(&v, "sound.volume")?;
                with(lua, |w| Ok(w.audio.set_master_volume(v)))
            }
            _ => Err(rt(format!("sound.{k} can't be set (only sound.volume)"))),
        })?,
    )?;
    sound.set_metatable(Some(meta))?;
    g.set("sound", sound)?;
    Ok(())
}
