//! Lua userdata types: Object, Mesh, Texture, Color, Material, camera, time.

use mlua::{Lua, MetaMethod, Table, UserData, UserDataFields, UserDataMethods, UserDataRef, Value};
use spark_core::{Color, Material, MeshId, Object, ObjectId, TextureId, Vec3, World};

use crate::convert::*;

/// Handle to a scene object.
#[derive(Clone, Copy)]
pub struct Obj(pub ObjectId);

fn read<R>(lua: &Lua, id: ObjectId, f: impl FnOnce(&World, &Object) -> R) -> LuaResult<R> {
    with(lua, |w| {
        let w: &World = w;
        let o = w.scene.get(id).ok_or_else(dead)?;
        Ok(f(w, o))
    })
}

fn write(lua: &Lua, id: ObjectId, f: impl FnOnce(&mut Object)) -> LuaResult<()> {
    with(lua, |w| {
        f(w.scene.get_mut(id).ok_or_else(dead)?);
        Ok(())
    })
}

pub(crate) fn to_parent(v: &Value, what: &str) -> LuaResult<Option<ObjectId>> {
    match v {
        Value::Nil | Value::Boolean(false) => Ok(None),
        Value::UserData(ud) => Ok(Some(ud.borrow::<Obj>().map_err(|_| rt(format!("{what}: expected an object or nil")))?.0)),
        _ => Err(rt(format!("{what}: expected an object or nil, got {}", v.type_name()))),
    }
}

pub(crate) fn set_parent(w: &mut World, id: ObjectId, parent: Option<ObjectId>) -> LuaResult<()> {
    w.scene.set_parent(id, parent).map_err(rt)
}

const PROPS: &str = "name, position, rotation, scale, visible, parent, color, shader, data, body, light";

/// `material.data`: up to 4 numbers, a vector or {x, y, z, w}.
pub(crate) fn to_data(args: &[Value], what: &str) -> LuaResult<[f32; 4]> {
    let mut out = [0.0; 4];
    let mut n = 0;
    let mut push = |x: f32| -> LuaResult<()> {
        if n >= 4 {
            return Err(rt(format!("{what}: at most 4 numbers")));
        }
        out[n] = x;
        n += 1;
        Ok(())
    };
    for a in args {
        match a {
            Value::Vector(v) => {
                push(v.x())?;
                push(v.y())?;
                push(v.z())?;
            }
            Value::Table(t) => {
                for (i, k) in ["x", "y", "z", "w"].iter().enumerate() {
                    let v: Value = t.get(*k)?;
                    let v = if v.is_nil() { t.get(i as i64 + 1)? } else { v };
                    if !v.is_nil() {
                        push(to_num(&v, what)?)?;
                    }
                }
            }
            Value::Nil => {}
            other => push(to_num(other, what)?)?,
        }
    }
    Ok(out)
}

fn data_table(lua: &mlua::Lua, d: [f32; 4]) -> LuaResult<Table> {
    let t = lua.create_table()?;
    for (k, x) in ["x", "y", "z", "w"].iter().zip(d) {
        t.set(*k, x)?;
    }
    Ok(t)
}

/// `spawn(mesh, mat, { position = ..., ... })` property table.
pub(crate) fn apply_props(w: &mut World, id: ObjectId, props: Option<Table>) -> LuaResult<()> {
    let Some(t) = props else { return Ok(()) };
    for pair in t.pairs::<String, Value>() {
        let (k, v) = pair?;
        let what = format!("property '{k}'");
        match k.as_str() {
            "name" => w.scene[id].name = to_str(&v, &what)?,
            "position" => w.scene[id].position = to_vec3(&v, &what)?,
            "rotation" => w.scene[id].rotation = from_euler(to_vec3(&v, &what)?),
            "scale" => w.scene[id].scale = to_scale(&v, &what)?,
            "visible" => w.scene[id].visible = to_bool(&v, &what)?,
            "color" => w.scene[id].material.color = to_color(&v, &what)?,
            "shader" => w.scene[id].material.shader = crate::shader::to_shader(&v, &what)?,
            "data" => w.scene[id].material.data = to_data(&[v], &what)?,
            "parent" => set_parent(w, id, to_parent(&v, &what)?)?,
            "body" => w.scene[id].body = Some(crate::physics::to_body(&v, &what)?),
            "light" => w.scene[id].light = Some(crate::light::to_light(&v, &what)?),
            _ => return Err(rt(format!("unknown property '{k}' (allowed: {PROPS})"))),
        }
    }
    Ok(())
}

impl UserData for Obj {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("id", |_, this| Ok(this.0.index()));
        f.add_field_method_get("name", |lua, this| read(lua, this.0, |_, o| o.name.clone()));
        f.add_field_method_set("name", |lua, this, v: Value| {
            let name = to_str(&v, "name")?;
            write(lua, this.0, |o| o.name = name)
        });
        f.add_field_method_get("position", |lua, this| read(lua, this.0, |_, o| vv(o.position)));
        f.add_field_method_set("position", |lua, this, v: Value| {
            let p = to_vec3(&v, "position")?;
            write(lua, this.0, |o| o.position = p)
        });
        f.add_field_method_get("x", |lua, this| read(lua, this.0, |_, o| o.position.x));
        f.add_field_method_get("y", |lua, this| read(lua, this.0, |_, o| o.position.y));
        f.add_field_method_get("z", |lua, this| read(lua, this.0, |_, o| o.position.z));
        f.add_field_method_set("x", |lua, this, v: f32| write(lua, this.0, |o| o.position.x = v));
        f.add_field_method_set("y", |lua, this, v: f32| write(lua, this.0, |o| o.position.y = v));
        f.add_field_method_set("z", |lua, this, v: f32| write(lua, this.0, |o| o.position.z = v));
        f.add_field_method_get("rotation", |lua, this| read(lua, this.0, |_, o| vv(euler(o.rotation))));
        f.add_field_method_set("rotation", |lua, this, v: Value| {
            let r = from_euler(to_vec3(&v, "rotation")?);
            write(lua, this.0, |o| o.rotation = r)
        });
        f.add_field_method_get("scale", |lua, this| read(lua, this.0, |_, o| vv(o.scale)));
        f.add_field_method_set("scale", |lua, this, v: Value| {
            let s = to_scale(&v, "scale")?;
            write(lua, this.0, |o| o.scale = s)
        });
        f.add_field_method_get("visible", |lua, this| read(lua, this.0, |_, o| o.visible));
        f.add_field_method_set("visible", |lua, this, v: bool| write(lua, this.0, |o| o.visible = v));
        f.add_field_method_get("color", |lua, this| read(lua, this.0, |_, o| LuaColor(o.material.color)));
        f.add_field_method_set("color", |lua, this, v: Value| {
            let c = to_color(&v, "color")?;
            write(lua, this.0, |o| o.material.color = c)
        });
        f.add_field_method_get("material", |lua, this| read(lua, this.0, |_, o| LuaMaterial(o.material)));
        f.add_field_method_get("shader", |lua, this| read(lua, this.0, |_, o| o.material.shader.map(crate::shader::LuaShader)));
        f.add_field_method_set("shader", |lua, this, v: Value| {
            let s = crate::shader::to_shader(&v, "shader")?;
            write(lua, this.0, |o| o.material.shader = s)
        });
        f.add_field_method_get("data", |lua, this| {
            let d = read(lua, this.0, |_, o| o.material.data)?;
            data_table(lua, d)
        });
        f.add_field_method_set("data", |lua, this, v: Value| {
            let d = to_data(&[v], "data")?;
            write(lua, this.0, |o| o.material.data = d)
        });
        f.add_field_method_set("material", |lua, this, v: Value| {
            let m = to_material(&v, "material")?;
            write(lua, this.0, |o| o.material = m)
        });
        f.add_field_method_get("mesh", |lua, this| read(lua, this.0, |_, o| o.mesh.map(LuaMesh)));
        f.add_field_method_set("mesh", |lua, this, v: Value| {
            let id = this.0;
            match v {
                Value::Nil => write(lua, id, |o| o.mesh = None),
                Value::UserData(ud) => {
                    let m = ud.borrow::<LuaMesh>().map_err(|_| rt("mesh: expected a Mesh or nil"))?.0;
                    with(lua, |w| {
                        if !w.scene.contains(id) {
                            return Err(dead());
                        }
                        w.set_mesh(id, m);
                        Ok(())
                    })
                }
                _ => Err(rt("mesh: expected a Mesh or nil")),
            }
        });
        f.add_field_method_get("parent", |lua, this| read(lua, this.0, |_, o| o.parent().map(Obj)));
        f.add_field_method_set("parent", |lua, this, v: Value| {
            let p = to_parent(&v, "parent")?;
            with(lua, |w| set_parent(w, this.0, p))
        });
        crate::physics::add_object_fields(f);
        crate::light::add_object_fields(f);
    }

    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        crate::flow::add_object_methods(m);
        crate::physics::add_object_methods(m);
        crate::model::add_object_methods(m);
        m.add_method("exists", |lua, this, ()| with(lua, |w| Ok(w.scene.contains(this.0))));
        m.add_method("destroy", |lua, this, ()| with(lua, |w| Ok(w.scene.destroy(this.0))));
        m.add_method("rotate_x", |lua, this, a: f32| write(lua, this.0, |o| o.rotate_x(a)));
        m.add_method("rotate_y", |lua, this, a: f32| write(lua, this.0, |o| o.rotate_y(a)));
        m.add_method("rotate_z", |lua, this, a: f32| write(lua, this.0, |o| o.rotate_z(a)));
        m.add_method("translate", |lua, this, v: Value| {
            let d = to_vec3(&v, "translate")?;
            write(lua, this.0, |o| o.translate(d))
        });
        m.add_method("look_at", |lua, this, target: Value| {
            with(lua, |w| {
                let t = to_point(w, &target, "look_at")?;
                let parent = w.scene.get(this.0).ok_or_else(dead)?.parent();
                let local = match parent {
                    Some(p) => w.scene.world_matrix(p).inverse().transform_point3(t),
                    None => t,
                };
                w.scene[this.0].look_at(local);
                Ok(())
            })
        });
        m.add_method("place_on", |lua, this, other: UserDataRef<Obj>| with(lua, |w| w.scene.place_on(this.0, other.0).map_err(rt)));
        m.add_method("set_parent", |lua, this, v: Value| {
            let p = to_parent(&v, "set_parent")?;
            with(lua, |w| set_parent(w, this.0, p))
        });
        m.add_method("children", |lua, this, ()| {
            with(lua, |w| Ok(w.scene.children(this.0).into_iter().map(Obj).collect::<Vec<_>>()))
        });
        m.add_method("world_position", |lua, this, ()| read(lua, this.0, |w, _| vv(w.scene.world_position(this.0))));
        m.add_method("forward", |lua, this, ()| read(lua, this.0, |_, o| vv(o.forward())));
        m.add_method("right", |lua, this, ()| read(lua, this.0, |_, o| vv(o.rotation * Vec3::X)));
        m.add_method("up", |lua, this, ()| read(lua, this.0, |_, o| vv(o.rotation * Vec3::Y)));
        m.add_method("bounds", |lua, this, ()| {
            read(lua, this.0, |w, _| {
                let b = w.scene.world_bounds(this.0);
                if b.is_empty() { (None, None) } else { (Some(vv(b.min)), Some(vv(b.max))) }
            })
        });
        m.add_method("size", |lua, this, ()| {
            read(lua, this.0, |w, _| {
                let b = w.scene.world_bounds(this.0);
                vv(if b.is_empty() { Vec3::ZERO } else { b.size() })
            })
        });
        m.add_method("distance_to", |lua, this, target: Value| {
            with(lua, |w| {
                if !w.scene.contains(this.0) {
                    return Err(dead());
                }
                let t = to_point(w, &target, "distance_to")?;
                Ok(w.scene.world_position(this.0).distance(t))
            })
        });
        m.add_meta_method(MetaMethod::Eq, |_, this, other: Value| {
            Ok(match other {
                Value::UserData(ud) => ud.borrow::<Obj>().map(|o| o.0 == this.0).unwrap_or(false),
                _ => false,
            })
        });
        m.add_meta_method(MetaMethod::ToString, |lua, this, ()| {
            with(lua, |w| {
                Ok(match w.scene.get(this.0) {
                    Some(o) if !o.name.is_empty() => format!("Object(\"{}\" #{})", o.name, this.0.index()),
                    Some(_) => format!("Object(#{})", this.0.index()),
                    None => format!("Object(#{} destroyed)", this.0.index()),
                })
            })
        });
    }
}

#[derive(Clone, Copy)]
pub struct LuaMesh(pub MeshId);

impl UserData for LuaMesh {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("Mesh(#{})", this.0.0)));
    }
}

#[derive(Clone, Copy)]
pub struct LuaTexture(pub TextureId);

impl UserData for LuaTexture {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("width", |lua, this| with(lua, |w| Ok(w.assets.texture(this.0).map(|t| t.width).unwrap_or(0))));
        f.add_field_method_get("height", |lua, this| with(lua, |w| Ok(w.assets.texture(this.0).map(|t| t.height).unwrap_or(0))));
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("Texture(#{})", this.0.0)));
    }
}

#[derive(Clone, Copy)]
pub struct LuaColor(pub Color);

impl UserData for LuaColor {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("r", |_, this| Ok(this.0.r));
        f.add_field_method_get("g", |_, this| Ok(this.0.g));
        f.add_field_method_get("b", |_, this| Ok(this.0.b));
        f.add_field_method_get("a", |_, this| Ok(this.0.a));
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("lerp", |_, this, (other, t): (Value, f32)| Ok(LuaColor(this.0.lerp(to_color(&other, "lerp")?, t))));
        m.add_method("with_alpha", |_, this, a: f32| Ok(LuaColor(this.0.with_alpha(a))));
        m.add_meta_method(MetaMethod::Eq, |_, this, other: Value| Ok(to_color(&other, "==").map(|c| c == this.0).unwrap_or(false)));
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            let [r, g, b, a] = this.0.to_rgba8();
            Ok(if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") })
        });
    }
}

#[derive(Clone, Copy)]
pub struct LuaMaterial(pub Material);

impl UserData for LuaMaterial {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("color", |_, this| Ok(LuaColor(this.0.color)));
        f.add_field_method_get("unlit", |_, this| Ok(this.0.unlit));
        f.add_field_method_get("texture", |_, this| Ok(this.0.texture.map(LuaTexture)));
        f.add_field_method_get("shader", |_, this| Ok(this.0.shader.map(crate::shader::LuaShader)));
        f.add_field_method_get("data", |lua, this| data_table(lua, this.0.data));
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("tiling", |_, this, (x, y): (f32, Option<f32>)| Ok(LuaMaterial(this.0.with_tiling(x, y.unwrap_or(x)))));
        m.add_method("with_color", |_, this, c: Value| Ok(LuaMaterial(this.0.with_color(to_color(&c, "with_color")?))));
        m.add_method("with_unlit", |_, this, on: Option<bool>| Ok(LuaMaterial(this.0.with_unlit(on.unwrap_or(true)))));
        m.add_method("with_shader", |_, this, s: Value| {
            Ok(LuaMaterial(this.0.with_shader(crate::shader::to_shader(&s, "with_shader")?)))
        });
        m.add_method("with_data", |_, this, v: mlua::Variadic<Value>| Ok(LuaMaterial(this.0.with_data(to_data(&v, "with_data")?))));
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| {
            let [r, g, b, _] = this.0.color.to_rgba8();
            Ok(format!(
                "Material(#{r:02x}{g:02x}{b:02x}{}{})",
                if this.0.texture.is_some() { " textured" } else { "" },
                if this.0.unlit { " unlit" } else { "" }
            ))
        });
    }
}

/// The global `camera`.
pub(crate) struct Cam;

impl UserData for Cam {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("position", |lua, _| with(lua, |w| Ok(vv(w.scene.camera.position))));
        f.add_field_method_set("position", |lua, _, v: Value| {
            let p = to_vec3(&v, "camera.position")?;
            with(lua, |w| Ok(w.scene.camera.position = p))
        });
        f.add_field_method_get("x", |lua, _| with(lua, |w| Ok(w.scene.camera.position.x)));
        f.add_field_method_get("y", |lua, _| with(lua, |w| Ok(w.scene.camera.position.y)));
        f.add_field_method_get("z", |lua, _| with(lua, |w| Ok(w.scene.camera.position.z)));
        f.add_field_method_set("x", |lua, _, v: f32| with(lua, |w| Ok(w.scene.camera.position.x = v)));
        f.add_field_method_set("y", |lua, _, v: f32| with(lua, |w| Ok(w.scene.camera.position.y = v)));
        f.add_field_method_set("z", |lua, _, v: f32| with(lua, |w| Ok(w.scene.camera.position.z = v)));
        f.add_field_method_get("rotation", |lua, _| with(lua, |w| Ok(vv(euler(w.scene.camera.rotation)))));
        f.add_field_method_set("rotation", |lua, _, v: Value| {
            let r = from_euler(to_vec3(&v, "camera.rotation")?);
            with(lua, |w| Ok(w.scene.camera.rotation = r))
        });
        f.add_field_method_get("fov", |lua, _| with(lua, |w| Ok(w.scene.camera.fov)));
        f.add_field_method_set("fov", |lua, _, v: f32| with(lua, |w| Ok(w.scene.camera.fov = v)));
        f.add_field_method_get("near", |lua, _| with(lua, |w| Ok(w.scene.camera.near)));
        f.add_field_method_set("near", |lua, _, v: f32| with(lua, |w| Ok(w.scene.camera.near = v)));
        f.add_field_method_get("far", |lua, _| with(lua, |w| Ok(w.scene.camera.far)));
        f.add_field_method_set("far", |lua, _, v: f32| with(lua, |w| Ok(w.scene.camera.far = v)));
    }
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_method("look_at", |lua, _, target: Value| {
            with(lua, |w| {
                let t = to_point(w, &target, "camera:look_at")?;
                w.scene.camera.look_at(t);
                Ok(())
            })
        });
        m.add_method("translate", |lua, _, v: Value| {
            let d = to_vec3(&v, "camera:translate")?;
            with(lua, |w| Ok(w.scene.camera.position += d))
        });
        m.add_method("forward", |lua, _, ()| with(lua, |w| Ok(vv(w.scene.camera.forward()))));
        m.add_method("right", |lua, _, ()| with(lua, |w| Ok(vv(w.scene.camera.right()))));
        m.add_method("up", |lua, _, ()| with(lua, |w| Ok(vv(w.scene.camera.up()))));
        m.add_meta_method(MetaMethod::ToString, |lua, _, ()| {
            with(lua, |w| {
                let c = &w.scene.camera;
                Ok(format!("Camera(pos {:.2} {:.2} {:.2}, fov {:.0})", c.position.x, c.position.y, c.position.z, c.fov))
            })
        });
    }
}

/// The global `time`.
pub(crate) struct TimeRef;

impl UserData for TimeRef {
    fn add_fields<F: UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("dt", |lua, _| with(lua, |w| Ok(w.time.dt)));
        f.add_field_method_get("elapsed", |lua, _| with(lua, |w| Ok(w.time.elapsed)));
        f.add_field_method_get("unscaled_dt", |lua, _| with(lua, |w| Ok(w.time.unscaled_dt)));
        f.add_field_method_get("unscaled_elapsed", |lua, _| with(lua, |w| Ok(w.time.unscaled_elapsed)));
        f.add_field_method_get("scale", |lua, _| with(lua, |w| Ok(w.time.scale)));
        f.add_field_method_set("scale", |lua, _, v: f32| {
            if !(v >= 0.0 && v.is_finite()) {
                return Err(rt("time.scale must be a number >= 0 (0 = paused, 1 = normal)"));
            }
            with(lua, |w| Ok(w.time.scale = v))
        });
        f.add_field_method_get("fixed_dt", |lua, _| with(lua, |w| Ok(w.time.fixed_dt)));
        f.add_field_method_set("fixed_dt", |lua, _, v: f32| {
            if !(v >= 0.001 && v <= 1.0) {
                return Err(rt("time.fixed_dt must be between 0.001 and 1 seconds (default 1/60)"));
            }
            with(lua, |w| Ok(w.time.fixed_dt = v))
        });
        f.add_field_method_get("frame", |lua, _| with(lua, |w| Ok(w.time.frame)));
        f.add_field_method_get("fps", |lua, _| with(lua, |w| Ok(w.time.fps)));
    }
}
