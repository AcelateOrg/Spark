//! `Shader` (custom WGSL) and `graphics` (resolution, sampling, shaders, post passes) globals.

use mlua::{Lua, MetaMethod, Table, UserData, UserDataFields, UserDataMethods, Value, Vector};
use spark_core::{ParamType, PassSize, PostPass, ShaderId, ShaderKind, TextureId, World};

use crate::convert::*;
use crate::types::{LuaColor, LuaTexture};

#[derive(Clone, Copy)]
pub struct LuaShader(pub ShaderId);

pub(crate) fn to_shader(v: &Value, what: &str) -> LuaResult<Option<ShaderId>> {
    match v {
        Value::Nil | Value::Boolean(false) => Ok(None),
        Value::UserData(ud) => Ok(Some(ud.borrow::<LuaShader>().map_err(|_| rt(format!("{what}: expected a Shader or nil")))?.0)),
        _ => Err(rt(format!("{what}: expected a Shader (Shader.load(\"shaders/x.wgsl\")) or nil, got {}", v.type_name()))),
    }
}

fn kind_name(k: ShaderKind) -> &'static str {
    match k {
        ShaderKind::Surface => "surface",
        ShaderKind::Post => "post",
    }
}

/// Numbers for a param of type `ty`: number, bool, vector, Color (-> linear), {x, y, z, w} / {1, 2, 3, 4}.
fn to_param(v: &Value, ty: ParamType, what: &str) -> LuaResult<Vec<f32>> {
    let n = ty.len();
    let out = match v {
        Value::Boolean(b) => vec![if *b { 1.0 } else { 0.0 }],
        Value::Number(_) | Value::Integer(_) => vec![to_num(v, what)?],
        Value::Vector(x) => vec![x.x(), x.y(), x.z()],
        Value::UserData(ud) if ud.is::<LuaColor>() => ud.borrow::<LuaColor>()?.0.to_linear().to_vec(),
        Value::String(_) => to_color(v, what)?.to_linear().to_vec(),
        Value::Table(t) => {
            let mut o = Vec::new();
            for (i, k) in ["x", "y", "z", "w"].iter().enumerate() {
                let a: Value = t.get(*k)?;
                let a = if a.is_nil() { t.get(i as i64 + 1)? } else { a };
                if a.is_nil() {
                    break;
                }
                o.push(to_num(&a, what)?);
            }
            o
        }
        _ => return Err(rt(format!("{what}: expected a number, vector, color or table, got {}", v.type_name()))),
    };
    Ok(match (n, out.len()) {
        (1, _) if out.len() == 1 => out,
        (2, 3) => out[..2].to_vec(),
        (4, 3) => vec![out[0], out[1], out[2], 1.0],
        _ => out,
    })
}

fn texture_arg(lua: &Lua, v: &Value, what: &str) -> LuaResult<Option<TextureId>> {
    match v {
        Value::Nil | Value::Boolean(false) => Ok(None),
        Value::String(s) => {
            let p = s.to_string_lossy();
            Ok(Some(with(lua, |w| w.assets.load_texture(&p).map_err(rt))?))
        }
        Value::UserData(ud) => Ok(Some(ud.borrow::<LuaTexture>().map_err(|_| rt(format!("{what}: expected a Texture, an image path or nil")))?.0)),
        _ => Err(rt(format!("{what}: expected a Texture, an image path or nil, got {}", v.type_name()))),
    }
}

fn get_field(lua: &Lua, id: ShaderId, key: &str) -> LuaResult<Value> {
    let data = with(lua, |w| Ok(w.assets.shader(id).cloned()))?.ok_or_else(|| rt("shader was unloaded"))?;
    match key {
        "name" => return Ok(Value::String(lua.create_string(&data.name)?)),
        "kind" => return Ok(Value::String(lua.create_string(kind_name(data.kind))?)),
        "params" => {
            let t = lua.create_table()?;
            for (i, p) in data.params.iter().enumerate() {
                t.set(i + 1, p.name.as_str())?;
            }
            return Ok(Value::Table(t));
        }
        "texture1" | "texture2" => {
            let t = data.textures[if key == "texture1" { 0 } else { 1 }];
            return match t {
                Some(t) => Ok(Value::UserData(lua.create_userdata(LuaTexture(t))?)),
                None => Ok(Value::Nil),
            };
        }
        _ => {}
    }
    let Some(p) = data.find_param(key) else {
        let names: Vec<&str> = data.params.iter().map(|p| p.name.as_str()).collect();
        return Err(rt(format!(
            "{}: no param or field '{key}' (params: {}; fields: name, kind, params, texture1, texture2)",
            data.name,
            if names.is_empty() { "none - declare `struct Params { ... }` in the shader".to_string() } else { names.join(", ") }
        )));
    };
    let v = data.param(key).expect("param exists");
    Ok(match p.ty {
        ParamType::F32 | ParamType::I32 | ParamType::U32 => Value::Number(v[0] as f64),
        ParamType::Vec2 => Value::Vector(Vector::new(v[0], v[1], 0.0)),
        ParamType::Vec3 => Value::Vector(Vector::new(v[0], v[1], v[2])),
        ParamType::Vec4 => {
            let t = lua.create_table()?;
            for (k, x) in ["x", "y", "z", "w"].iter().zip(v) {
                t.set(*k, x)?;
            }
            Value::Table(t)
        }
    })
}

fn set_field(lua: &Lua, id: ShaderId, key: &str, v: Value) -> LuaResult<()> {
    if key == "texture1" || key == "texture2" {
        let t = texture_arg(lua, &v, key)?;
        return with(lua, |w| {
            let s = w.assets.shader_mut(id).ok_or_else(|| rt("shader was unloaded"))?;
            s.textures[if key == "texture1" { 0 } else { 1 }] = t;
            Ok(())
        });
    }
    with(lua, |w| {
        let s = w.assets.shader_mut(id).ok_or_else(|| rt("shader was unloaded"))?;
        let Some(ty) = s.find_param(key).map(|p| p.ty) else {
            return Err(rt(s.set_param(key, &[]).unwrap_err()));
        };
        let what = format!("{}.{key}", s.name);
        let values = to_param(&v, ty, &what)?;
        s.set_param(key, &values).map_err(rt)
    })
}

impl UserData for LuaShader {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("set", |lua, this, t: Table| {
            for pair in t.pairs::<String, Value>() {
                let (k, v) = pair?;
                set_field(lua, this.0, &k, v)?;
            }
            Ok(())
        });
        m.add_meta_method(MetaMethod::Index, |lua, this, key: String| get_field(lua, this.0, &key));
        m.add_meta_method(MetaMethod::NewIndex, |lua, this, (key, v): (String, Value)| set_field(lua, this.0, &key, v));
        m.add_meta_method(MetaMethod::Eq, |_, this, other: mlua::UserDataRef<LuaShader>| Ok(this.0 == other.0));
        m.add_meta_method(MetaMethod::ToString, |lua, this, ()| {
            with(lua, |w| {
                Ok(match w.assets.shader(this.0) {
                    Some(s) => format!("Shader({}, {})", s.name, kind_name(s.kind)),
                    None => "Shader(unloaded)".into(),
                })
            })
        });
    }
}

/// The `graphics` global.
struct LuaRender;

fn filter_name(f: spark_core::TextureFilter) -> &'static str {
    match f {
        spark_core::TextureFilter::Nearest => "nearest",
        spark_core::TextureFilter::Linear => "linear",
    }
}

fn to_post(w: &World, v: &Value) -> LuaResult<Vec<PostPass>> {
    let one = |v: &Value, what: &str| -> LuaResult<PostPass> {
        let (shader, size) = match v {
            Value::Table(t) => {
                let mut size = PassSize::Screen;
                for pair in t.pairs::<String, Value>() {
                    let (k, x) = pair?;
                    match k.as_str() {
                        "shader" => {}
                        "size" => {
                            size = match to_str(&x, &format!("{what}.size"))?.as_str() {
                                "screen" => PassSize::Screen,
                                "scene" => PassSize::Scene,
                                s => return Err(rt(format!("{what}.size: unknown '{s}' (use \"screen\" or \"scene\")"))),
                            }
                        }
                        _ => return Err(rt(format!("{what}: unknown field '{k}' (allowed: shader, size)"))),
                    }
                }
                (to_shader(&t.get::<Value>("shader")?, &format!("{what}.shader"))?, size)
            }
            other => (to_shader(other, what)?, PassSize::Screen),
        };
        let shader = shader.ok_or_else(|| rt(format!("{what}: expected a post Shader")))?;
        match w.assets.shader(shader) {
            Some(s) if s.kind == ShaderKind::Post => Ok(PostPass { shader, size }),
            Some(s) => Err(rt(format!("{what}: {} is a surface shader; post passes need `fn post(p: PostInput) -> vec4<f32>`", s.name))),
            None => Err(rt(format!("{what}: shader was unloaded"))),
        }
    };
    match v {
        Value::Nil | Value::Boolean(false) => Ok(Vec::new()),
        Value::UserData(_) => Ok(vec![one(v, "graphics.post")?]),
        Value::Table(t) if t.contains_key("shader")? => Ok(vec![one(v, "graphics.post")?]),
        Value::Table(t) => {
            let mut out = Vec::new();
            for (i, x) in t.sequence_values::<Value>().enumerate() {
                out.push(one(&x?, &format!("graphics.post[{}]", i + 1))?);
            }
            Ok(out)
        }
        _ => Err(rt(format!("graphics.post: expected a Shader, a list of shaders or nil, got {}", v.type_name()))),
    }
}

impl UserData for LuaRender {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("height", |lua, _| with(lua, |w| Ok(w.render.height)));
        f.add_field_method_set("height", |lua, _, v: Value| {
            let h = match v {
                Value::Nil | Value::Boolean(false) => None,
                _ => Some(to_num(&v, "graphics.height")?.max(1.0) as u32),
            };
            with(lua, |w| Ok(w.render.height = h))
        });
        f.add_field_method_get("scale", |lua, _| with(lua, |w| Ok(w.render.scale)));
        f.add_field_method_set("scale", |lua, _, v: f32| with(lua, |w| Ok(w.render.scale = v.clamp(0.05, 2.0))));
        f.add_field_method_get("upscale", |lua, _| with(lua, |w| Ok(filter_name(w.render.upscale))));
        f.add_field_method_set("upscale", |lua, _, v: Value| {
            let f = to_filter(&v, "graphics.upscale")?;
            with(lua, |w| Ok(w.render.upscale = f))
        });
        f.add_field_method_get("msaa", |lua, _| with(lua, |w| Ok(w.render.msaa)));
        f.add_field_method_set("msaa", |lua, _, v: Value| {
            let n = match v {
                Value::Boolean(b) => if b { 4 } else { 1 },
                _ => to_num(&v, "graphics.msaa")?.clamp(1.0, 8.0) as u32,
            };
            with(lua, |w| Ok(w.render.msaa = n))
        });
        f.add_field_method_get("shadows", |lua, _| with(lua, |w| Ok(w.render.shadows)));
        f.add_field_method_set("shadows", |lua, _, v: bool| with(lua, |w| Ok(w.render.shadows = v)));
        f.add_field_method_get("shadow_distance", |lua, _| with(lua, |w| Ok(w.render.shadow_distance)));
        f.add_field_method_set("shadow_distance", |lua, _, v: f32| with(lua, |w| Ok(w.render.shadow_distance = v.max(1.0))));
        f.add_field_method_get("shadow_size", |lua, _| with(lua, |w| Ok(w.render.shadow_size)));
        f.add_field_method_set("shadow_size", |lua, _, v: f32| with(lua, |w| Ok(w.render.shadow_size = v.clamp(256.0, 8192.0) as u32)));
        f.add_field_method_get("filter", |lua, _| with(lua, |w| Ok(filter_name(w.render.filter))));
        f.add_field_method_set("filter", |lua, _, v: Value| {
            let f = to_filter(&v, "graphics.filter")?;
            with(lua, |w| Ok(w.render.filter = f))
        });
        f.add_field_method_get("shader", |lua, _| with(lua, |w| Ok(w.render.shader.map(LuaShader))));
        f.add_field_method_set("shader", |lua, _, v: Value| {
            let s = to_shader(&v, "graphics.shader")?;
            with(lua, |w| {
                if let Some(d) = s.and_then(|s| w.assets.shader(s)) {
                    if d.kind != ShaderKind::Surface {
                        return Err(rt(format!("graphics.shader: {} is a post shader; use graphics.post = {{ shader }}", d.name)));
                    }
                }
                Ok(w.render.shader = s)
            })
        });
        f.add_field_method_get("post", |lua, _| {
            let post = with(lua, |w| Ok(w.render.post.clone()))?;
            let t = lua.create_table()?;
            for (i, p) in post.iter().enumerate() {
                t.set(i + 1, LuaShader(p.shader))?;
            }
            Ok(t)
        });
        f.add_field_method_set("post", |lua, _, v: Value| {
            with(lua, |w| {
                let p = to_post(w, &v)?;
                Ok(w.render.post = p)
            })
        });
        f.add_field_method_get("scene_width", |lua, _| {
            with(lua, |w| {
                let (ow, oh) = w.canvas.output_size();
                Ok(w.render.internal_size(ow, oh).0)
            })
        });
        f.add_field_method_get("scene_height", |lua, _| {
            with(lua, |w| {
                let (ow, oh) = w.canvas.output_size();
                Ok(w.render.internal_size(ow, oh).1)
            })
        });

        // ── Built-in post-processing effects ───────────────────────────
        // Each effect: nil / false = off, true = default settings, table = settings (missing keys
        // keep their defaults, unknown keys and wrong types are errors). Reading gives the table.
        use spark_core::render::*;

        f.add_field_method_get("tonemap", |lua, _| {
            with(lua, |w| Ok(match w.render.postfx.tonemap {
                Tonemap::Off => "off",
                Tonemap::Aces => "aces",
                Tonemap::Reinhard => "reinhard",
                Tonemap::Hejl => "hejl",
            }))
        });
        f.add_field_method_set("tonemap", |lua, _, v: String| {
            let t = match v.as_str() {
                "off" => Tonemap::Off,
                "aces" => Tonemap::Aces,
                "reinhard" => Tonemap::Reinhard,
                "hejl" => Tonemap::Hejl,
                s => return Err(rt(format!("graphics.tonemap: unknown '{s}' (use \"off\", \"aces\", \"reinhard\", \"hejl\")"))),
            };
            with(lua, |w| Ok(w.render.postfx.tonemap = t))
        });
        f.add_field_method_get("fxaa", |lua, _| with(lua, |w| Ok(w.render.postfx.fxaa)));
        f.add_field_method_set("fxaa", |lua, _, v: Value| {
            let on = to_bool(&v, "graphics.fxaa")?;
            with(lua, |w| Ok(w.render.postfx.fxaa = on))
        });

        macro_rules! fx_field {
            ($name:literal, $field:ident) => {
                f.add_field_method_get($name, |lua, _| with(lua, |w| fx_to_lua(lua, &w.render.postfx.$field)));
                f.add_field_method_set($name, |lua, _, v: Value| {
                    let settings = fx_from_lua(&v)?;
                    with(lua, |w| Ok(w.render.postfx.$field = settings))
                });
            };
        }
        fx_field!("bloom", bloom);
        fx_field!("ssao", ssao);
        fx_field!("motion_blur", motion_blur);
        fx_field!("dof", dof);
        fx_field!("color_grading", color_grading);
        fx_field!("vignette", vignette);
        fx_field!("film_grain", film_grain);
        fx_field!("chromatic_aberration", chromatic_aberration);
    }
}

/// Settings struct of one built-in effect, readable from / writable to a Luau table.
trait FxSettings: Default {
    const NAME: &'static str;
    const KEYS: &'static [&'static str];
    fn read(&mut self, key: &str, v: &Value, what: &str) -> LuaResult<()>;
    fn write(&self, lua: &Lua, t: &Table) -> LuaResult<()>;
}

/// Value kinds of effect settings: `read` converts with a clear error, `write` back to Luau.
mod fx_kind {
    use super::*;
    pub mod num {
        use super::*;
        pub fn read(v: &Value, what: &str) -> LuaResult<f32> {
            to_num(v, what)
        }
        pub fn write(_: &Lua, x: &f32) -> LuaResult<Value> {
            Ok(Value::Number(f64::from(*x)))
        }
    }
    /// Sample counts: rounded and clamped to 1..=64.
    pub mod count {
        use super::*;
        pub fn read(v: &Value, what: &str) -> LuaResult<u32> {
            Ok(to_num(v, what)?.round().clamp(1.0, 64.0) as u32)
        }
        pub fn write(_: &Lua, x: &u32) -> LuaResult<Value> {
            Ok(Value::Integer(i64::from(*x)))
        }
    }
    pub mod flag {
        use super::*;
        pub fn read(v: &Value, what: &str) -> LuaResult<bool> {
            to_bool(v, what)
        }
        pub fn write(_: &Lua, x: &bool) -> LuaResult<Value> {
            Ok(Value::Boolean(*x))
        }
    }
    /// Per-channel numbers: `vec3(r, g, b)` or `{r, g, b}`.
    pub mod vec3 {
        use super::*;
        pub fn read(v: &Value, what: &str) -> LuaResult<[f32; 3]> {
            Ok(to_vec3(v, what)?.to_array())
        }
        pub fn write(_: &Lua, x: &[f32; 3]) -> LuaResult<Value> {
            Ok(Value::Vector(Vector::new(x[0], x[1], x[2])))
        }
    }
    pub mod color {
        use super::*;
        pub fn read(v: &Value, what: &str) -> LuaResult<spark_core::Color> {
            to_color(v, what)
        }
        pub fn write(lua: &Lua, x: &spark_core::Color) -> LuaResult<Value> {
            lua.create_userdata(LuaColor(*x)).map(Value::UserData)
        }
    }
}

macro_rules! fx_settings {
    ($ty:ty, $name:literal, { $($field:ident : $kind:ident),* $(,)? }) => {
        impl FxSettings for $ty {
            const NAME: &'static str = $name;
            const KEYS: &'static [&'static str] = &[$(stringify!($field)),*];
            fn read(&mut self, key: &str, v: &Value, what: &str) -> LuaResult<()> {
                match key {
                    $(stringify!($field) => self.$field = fx_kind::$kind::read(v, what)?,)*
                    _ => unreachable!("key checked against KEYS"),
                }
                Ok(())
            }
            fn write(&self, lua: &Lua, t: &Table) -> LuaResult<()> {
                $(t.set(stringify!($field), fx_kind::$kind::write(lua, &self.$field)?)?;)*
                Ok(())
            }
        }
    };
}

use spark_core::render::{Bloom, ChromaticAberration, ColorGrading, DepthOfField, FilmGrain, MotionBlur, Ssao, Vignette};
fx_settings!(Bloom, "bloom", { threshold: num, knee: num, strength: num, radius: num });
fx_settings!(Ssao, "ssao", { radius: num, samples: count, strength: num, bias: num });
fx_settings!(MotionBlur, "motion_blur", { strength: num, samples: count });
fx_settings!(DepthOfField, "dof", { focus: num, aperture: num, focal_range: num, samples: count });
fx_settings!(ColorGrading, "color_grading", {
    exposure: num, contrast: num, saturation: num, temperature: num, tint: num, lift: vec3, gamma: vec3, gain: vec3,
});
fx_settings!(Vignette, "vignette", { strength: num, inner: num, outer: num, color: color });
fx_settings!(FilmGrain, "film_grain", { strength: num, size: num });
fx_settings!(ChromaticAberration, "chromatic_aberration", { amount: num, radial: flag });

/// `nil` / `false` -> off, `true` -> defaults, table -> defaults overridden by its keys.
fn fx_from_lua<T: FxSettings>(v: &Value) -> LuaResult<Option<T>> {
    let t = match v {
        Value::Nil | Value::Boolean(false) => return Ok(None),
        Value::Boolean(true) => return Ok(Some(T::default())),
        Value::Table(t) => t,
        _ => return Err(rt(format!("graphics.{}: expected a table, true or nil, got {}", T::NAME, v.type_name()))),
    };
    let mut settings = T::default();
    for pair in t.pairs::<Value, Value>() {
        let (k, val) = pair?;
        let key = match &k {
            Value::String(s) => s.to_string_lossy(),
            _ => return Err(rt(format!("graphics.{}: keys must be names, got {}", T::NAME, k.type_name()))),
        };
        if !T::KEYS.contains(&key.as_str()) {
            return Err(rt(format!("graphics.{}: unknown setting '{key}' (allowed: {})", T::NAME, T::KEYS.join(", "))));
        }
        settings.read(&key, &val, &format!("graphics.{}.{key}", T::NAME))?;
    }
    Ok(Some(settings))
}

fn fx_to_lua<T: FxSettings>(lua: &Lua, v: &Option<T>) -> LuaResult<Value> {
    let Some(s) = v else { return Ok(Value::Nil) };
    let t = lua.create_table()?;
    s.write(lua, &t)?;
    Ok(Value::Table(t))
}


pub(crate) fn install(lua: &Lua, g: &Table) -> LuaResult<()> {
    let shader = lua.create_table()?;
    shader.set(
        "load",
        lua.create_function(|lua, path: String| with(lua, |w| w.assets.load_shader(&path).map(LuaShader).map_err(rt)))?,
    )?;
    shader.set(
        "new",
        lua.create_function(|lua, (code, name): (String, Option<String>)| {
            let name = name.unwrap_or_else(|| "shader".into());
            with(lua, |w| w.assets.add_shader(&name, &code).map(LuaShader).map_err(rt))
        })?,
    )?;
    g.set("Shader", shader)?;
    g.set("graphics", lua.create_userdata(LuaRender)?)?;
    Ok(())
}
