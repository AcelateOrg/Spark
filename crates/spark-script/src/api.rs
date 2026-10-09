//! Registers every Spark global (see docs/LUAU_API.md).

use std::cell::RefCell;
use std::collections::HashMap;

use mlua::{Lua, Table, UserDataRef, Value, Variadic, Vector};
use spark_core::{
    Color, Fog, Gamepad, ImageData, Key, Material, MeshData, MeshId, MouseButton, PadAxis, PadButton, SurfaceOpts, Vec3, World,
};

use crate::convert::*;
use crate::types::*;

/// Primitive meshes are created once per unique parameter set.
struct MeshCache(RefCell<HashMap<String, MeshId>>);

fn cached_mesh(lua: &Lua, key: String, make: impl FnOnce() -> MeshData) -> LuaResult<LuaMesh> {
    if let Some(id) = lua.app_data_ref::<MeshCache>().and_then(|c| c.0.borrow().get(&key).copied()) {
        return Ok(LuaMesh(id));
    }
    let id = with(lua, |w| Ok(w.add_mesh(make())))?;
    if let Some(c) = lua.app_data_ref::<MeshCache>() {
        c.0.borrow_mut().insert(key, id);
    }
    Ok(LuaMesh(id))
}

/// Optional `{ tile = meters, cell = meters }` for box / plane / quad.
fn surface_opts(t: Option<Table>, what: &str) -> LuaResult<Option<SurfaceOpts>> {
    let Some(t) = t else { return Ok(None) };
    let mut o = SurfaceOpts::default();
    for pair in t.pairs::<String, Value>() {
        let (k, v) = pair?;
        let w = format!("{what}.{k}");
        match k.as_str() {
            "tile" => o.tile = Some(to_num(&v, &w)?.max(0.001)),
            "cell" => o.cell = Some(to_num(&v, &w)?.max(0.05)),
            _ => return Err(rt(format!("{what}: unknown option '{k}' (allowed: tile, cell)"))),
        }
    }
    Ok(Some(o))
}

fn keys(name: &str) -> LuaResult<Vec<Key>> {
    Key::resolve(name).ok_or_else(|| {
        rt(format!(
            "unknown key '{name}' (keys: a-z, 0-9, punctuation like minus comma slash or \"-\" \",\" \"/\", numpad0-9, f1-f24, all names: {})",
            Key::names()
        ))
    })
}

fn button(name: Option<String>) -> LuaResult<MouseButton> {
    let name = name.unwrap_or_else(|| "left".into());
    MouseButton::from_name(&name).ok_or_else(|| rt(format!("unknown mouse button '{name}' (use \"left\", \"right\", \"middle\", \"back\", \"forward\")")))
}

fn pad_button(name: &str) -> LuaResult<PadButton> {
    PadButton::from_name(name)
        .ok_or_else(|| rt(format!("unknown gamepad button '{name}' (buttons: {})", PadButton::NAMES.trim_end())))
}

fn pad_axis(name: &str) -> LuaResult<PadAxis> {
    PadAxis::from_name(name).ok_or_else(|| rt(format!("unknown gamepad axis '{name}' (axes: {})", PadAxis::NAMES.trim_end())))
}

/// Pads a call looks at: pad `n` (1-based) or every connected pad when `n` is nil.
fn pads_of(w: &World, n: Option<usize>) -> Vec<&Gamepad> {
    match n {
        Some(n) => w.input.pad(n).into_iter().collect(),
        None => w.input.pads.iter().collect(),
    }
}

/// Axis value: of pad `n`, or the strongest one over all pads.
fn axis_value(w: &World, a: PadAxis, n: Option<usize>) -> f32 {
    pads_of(w, n).iter().map(|p| p.axis(a)).fold(0.0, |best, v| if v.abs() > best.abs() { v } else { best })
}

fn install_gamepad(lua: &Lua, input: &Table) -> LuaResult<()> {
    type State = fn(&Gamepad, PadButton) -> bool;
    let states: [(&str, State); 3] =
        [("pad_down", Gamepad::down), ("pad_pressed", Gamepad::pressed), ("pad_released", Gamepad::released)];
    for (name, f) in states {
        input.set(
            name,
            lua.create_function(move |lua, (b, n): (String, Option<usize>)| {
                let b = pad_button(&b)?;
                with(lua, |w| Ok(pads_of(w, n).iter().any(|p| f(p, b))))
            })?,
        )?;
    }
    input.set(
        "pad_axis",
        lua.create_function(|lua, (a, n): (String, Option<usize>)| {
            let a = pad_axis(&a)?;
            with(lua, |w| Ok(axis_value(w, a, n)))
        })?,
    )?;
    input.set(
        "pad_stick",
        lua.create_function(|lua, (s, n): (Option<String>, Option<usize>)| {
            let (x, y) = match s.as_deref().unwrap_or("left") {
                "left" => (PadAxis::LeftX, PadAxis::LeftY),
                "right" => (PadAxis::RightX, PadAxis::RightY),
                other => return Err(rt(format!("input.pad_stick: unknown stick '{other}' (use \"left\" or \"right\")"))),
            };
            with(lua, |w| Ok((axis_value(w, x, n), axis_value(w, y, n))))
        })?,
    )?;
    input.set("pads", lua.create_function(|lua, ()| with(lua, |w| Ok(w.input.pads.len())))?)?;
    input.set(
        "pad_name",
        lua.create_function(|lua, n: Option<usize>| with(lua, |w| Ok(w.input.pad(n.unwrap_or(1)).map(|p| p.name.clone()))))?,
    )?;
    input.set("any_pressed", lua.create_function(|lua, ()| with(lua, |w| Ok(w.input.any_pressed())))?)?;
    Ok(())
}

fn lerp_values(lua: &Lua, a: &Value, b: &Value, t: f32) -> LuaResult<Value> {
    let is_color = |v: &Value| matches!(v, Value::UserData(u) if u.is::<LuaColor>());
    match (a, b) {
        _ if is_color(a) || is_color(b) => {
            let (a, b) = (to_color(a, "lerp")?, to_color(b, "lerp")?);
            Ok(Value::UserData(lua.create_userdata(LuaColor(a.lerp(b, t)))?))
        }
        (Value::Vector(_), _) | (_, Value::Vector(_)) => {
            let (a, b) = (to_vec3(a, "lerp")?, to_vec3(b, "lerp")?);
            Ok(Value::Vector(vv(a.lerp(b, t))))
        }
        _ => {
            let (a, b) = (to_num(a, "lerp")?, to_num(b, "lerp")?);
            Ok(Value::Number((a + (b - a) * t) as f64))
        }
    }
}

fn spawn_with(w: &mut World, id: spark_core::ObjectId, props: Option<Table>) -> LuaResult<Obj> {
    if let Err(e) = apply_props(w, id, props) {
        w.scene.destroy(id);
        return Err(e);
    }
    Ok(Obj(id))
}

fn opt_color(v: &Value, default: Color, what: &str) -> LuaResult<Color> {
    if v.is_nil() { Ok(default) } else { to_color(v, what) }
}

fn install_math(lua: &Lua, g: &Table) -> LuaResult<()> {
    g.set(
        "vec3",
        lua.create_function(|_, (x, y, z): (Option<f32>, Option<f32>, Option<f32>)| {
            Ok(match (x, y, z) {
                (Some(s), None, None) => Vector::new(s, s, s),
                _ => Vector::new(x.unwrap_or(0.0), y.unwrap_or(0.0), z.unwrap_or(0.0)),
            })
        })?,
    )?;
    g.set("length", lua.create_function(|_, v: Value| Ok(to_vec3(&v, "length")?.length()))?)?;
    g.set("normalize", lua.create_function(|_, v: Value| Ok(vv(to_vec3(&v, "normalize")?.normalize_or_zero())))?)?;
    g.set("dot", lua.create_function(|_, (a, b): (Value, Value)| Ok(to_vec3(&a, "dot")?.dot(to_vec3(&b, "dot")?)))?)?;
    g.set("cross", lua.create_function(|_, (a, b): (Value, Value)| Ok(vv(to_vec3(&a, "cross")?.cross(to_vec3(&b, "cross")?))))?)?;
    g.set(
        "distance",
        lua.create_function(|_, (a, b): (Value, Value)| Ok(to_vec3(&a, "distance")?.distance(to_vec3(&b, "distance")?)))?,
    )?;
    g.set("lerp", lua.create_function(|lua, (a, b, t): (Value, Value, f32)| lerp_values(lua, &a, &b, t))?)?;
    Ok(())
}

fn install_assets(lua: &Lua, g: &Table) -> LuaResult<()> {
    let mesh = lua.create_table()?;
    mesh.set(
        "cube",
        lua.create_function(|lua, size: Option<f32>| {
            let s = size.unwrap_or(1.0);
            cached_mesh(lua, format!("cube {s}"), || MeshData::cube(s))
        })?,
    )?;
    mesh.set(
        "box",
        lua.create_function(|lua, (w, h, d, o): (Option<f32>, Option<f32>, Option<f32>, Option<Table>)| {
            let (w, h, d) = (w.unwrap_or(1.0), h.unwrap_or(1.0), d.unwrap_or(1.0));
            match surface_opts(o, "Mesh.box")? {
                Some(o) => cached_mesh(lua, format!("box {w} {h} {d} {o:?}"), || MeshData::cuboid_with(w, h, d, o)),
                None => cached_mesh(lua, format!("box {w} {h} {d}"), || MeshData::cuboid(w, h, d)),
            }
        })?,
    )?;
    mesh.set(
        "plane",
        lua.create_function(|lua, (w, d, o): (Option<f32>, Option<f32>, Option<Table>)| {
            let w = w.unwrap_or(10.0);
            let d = d.unwrap_or(w);
            match surface_opts(o, "Mesh.plane")? {
                Some(o) => cached_mesh(lua, format!("plane {w} {d} {o:?}"), || MeshData::plane_with(w, d, o)),
                None => cached_mesh(lua, format!("plane {w} {d}"), || MeshData::plane(w, d)),
            }
        })?,
    )?;
    mesh.set(
        "quad",
        lua.create_function(|lua, (w, h, o): (Option<f32>, Option<f32>, Option<Table>)| {
            let w = w.unwrap_or(1.0);
            let h = h.unwrap_or(w);
            match surface_opts(o, "Mesh.quad")? {
                Some(o) => cached_mesh(lua, format!("quad {w} {h} {o:?}"), || MeshData::quad_with(w, h, o)),
                None => cached_mesh(lua, format!("quad {w} {h}"), || MeshData::quad(w, h)),
            }
        })?,
    )?;
    mesh.set(
        "sphere",
        lua.create_function(|lua, (r, seg): (Option<f32>, Option<u32>)| {
            let (r, seg) = (r.unwrap_or(0.5), seg.unwrap_or(24).clamp(3, 256));
            cached_mesh(lua, format!("sphere {r} {seg}"), || MeshData::sphere(r, seg))
        })?,
    )?;
    mesh.set(
        "cylinder",
        lua.create_function(|lua, (r, h, seg): (Option<f32>, Option<f32>, Option<u32>)| {
            let (r, h, seg) = (r.unwrap_or(0.5), h.unwrap_or(1.0), seg.unwrap_or(24).clamp(3, 256));
            cached_mesh(lua, format!("cylinder {r} {h} {seg}"), || MeshData::cylinder(r, h, seg))
        })?,
    )?;
    mesh.set(
        "triangles",
        lua.create_function(|lua, points: Vec<Value>| {
            if points.len() < 3 || points.len() % 3 != 0 {
                return Err(rt(format!("Mesh.triangles: expected 3, 6, 9... points, got {}", points.len())));
            }
            let pts = points.iter().map(|p| to_vec3(p, "Mesh.triangles")).collect::<LuaResult<Vec<_>>>()?;
            with(lua, |w| Ok(LuaMesh(w.add_mesh(MeshData::from_triangles(&pts)))))
        })?,
    )?;
    g.set("Mesh", mesh)?;

    let texture = lua.create_table()?;
    texture.set("load", lua.create_function(|lua, path: String| with(lua, |w| w.assets.load_texture(&path).map(LuaTexture).map_err(rt)))?)?;
    texture.set(
        "checker",
        lua.create_function(|lua, (a, b, cells, size): (Value, Value, Option<u32>, Option<u32>)| {
            let a = opt_color(&a, Color::WHITE, "Texture.checker")?;
            let b = opt_color(&b, Color::GRAY, "Texture.checker")?;
            let img = ImageData::checker(size.unwrap_or(64).clamp(2, 4096), cells.unwrap_or(8).max(1), a, b);
            with(lua, |w| Ok(LuaTexture(w.assets.add_texture(img))))
        })?,
    )?;
    texture.set(
        "solid",
        lua.create_function(|lua, c: Value| {
            let img = ImageData::solid(to_color(&c, "Texture.solid")?);
            with(lua, |w| Ok(LuaTexture(w.assets.add_texture(img))))
        })?,
    )?;
    g.set("Texture", texture)?;

    let material = lua.create_table()?;
    material.set("color", lua.create_function(|_, c: Value| Ok(LuaMaterial(Material::color(to_color(&c, "Material.color")?))))?)?;
    material.set("unlit", lua.create_function(|_, c: Value| Ok(LuaMaterial(Material::unlit(to_color(&c, "Material.unlit")?))))?)?;
    material.set(
        "texture",
        lua.create_function(|lua, (t, c): (Value, Value)| {
            let bad = || rt("Material.texture: expected a Texture or an image path like \"grass.png\"");
            let tex = match &t {
                Value::String(s) => {
                    let p = s.to_string_lossy();
                    with(lua, |w| w.assets.load_texture(&p).map_err(rt))?
                }
                Value::UserData(ud) => ud.borrow::<LuaTexture>().map_err(|_| bad())?.0,
                _ => return Err(bad()),
            };
            Ok(LuaMaterial(Material::textured(tex).with_color(opt_color(&c, Color::WHITE, "Material.texture")?)))
        })?,
    )?;
    material.set(
        "checker",
        lua.create_function(|lua, (a, b, cells): (Value, Value, Option<f32>)| {
            let a = opt_color(&a, Color::WHITE, "Material.checker")?;
            let b = opt_color(&b, Color::GRAY, "Material.checker")?;
            let tex = with(lua, |w| Ok(w.assets.add_texture(ImageData::checker(64, 2, a, b))))?;
            let n = cells.unwrap_or(1.0).max(0.01);
            Ok(LuaMaterial(Material::textured(tex).with_tiling(n, n)))
        })?,
    )?;
    g.set("Material", material)?;

    let color = lua.create_table()?;
    color.set("rgb", lua.create_function(|_, (r, g, b): (f32, f32, f32)| Ok(LuaColor(Color::rgb(r, g, b))))?)?;
    color.set("rgba", lua.create_function(|_, (r, g, b, a): (f32, f32, f32, f32)| Ok(LuaColor(Color::rgba(r, g, b, a))))?)?;
    color.set("hex", lua.create_function(|_, v: Value| Ok(LuaColor(to_color(&v, "Color.hex")?)))?)?;
    color.set(
        "lerp",
        lua.create_function(|_, (a, b, t): (Value, Value, f32)| Ok(LuaColor(to_color(&a, "Color.lerp")?.lerp(to_color(&b, "Color.lerp")?, t))))?,
    )?;
    for (name, c) in Color::NAMED {
        color.set(name.to_ascii_uppercase(), LuaColor(*c))?;
    }
    g.set("Color", color)?;
    Ok(())
}

fn install_scene(lua: &Lua, g: &Table) -> LuaResult<()> {
    g.set(
        "spawn",
        lua.create_function(|lua, (mesh, mat, props): (Value, Value, Option<Table>)| {
            if let Some(model) = with(lua, |w| crate::model::to_model(w, &mesh, "spawn"))? {
                return crate::model::spawn_model(lua, model, mat, props);
            }
            let bad = || {
                rt(format!(
                    "spawn: first argument must be a Mesh (e.g. Mesh.cube()) or a Model (Model.load(\"x.glb\")), got {}",
                    mesh.type_name()
                ))
            };
            let mesh = match &mesh {
                Value::UserData(ud) => ud.borrow::<LuaMesh>().map_err(|_| bad())?.0,
                _ => return Err(bad()),
            };
            let material = to_material(&mat, "spawn")?;
            with(lua, |w| {
                let id = w.spawn(mesh, material);
                spawn_with(w, id, props)
            })
        })?,
    )?;
    g.set(
        "group",
        lua.create_function(|lua, props: Option<Table>| {
            with(lua, |w| {
                let id = w.spawn_empty();
                spawn_with(w, id, props)
            })
        })?,
    )?;
    g.set("find", lua.create_function(|lua, name: String| with(lua, |w| Ok(w.scene.find(&name).map(Obj))))?)?;
    g.set(
        "find_all",
        lua.create_function(|lua, name: String| {
            with(lua, |w| Ok(w.scene.iter().filter(|(_, o)| o.name == name).map(|(id, _)| Obj(id)).collect::<Vec<_>>()))
        })?,
    )?;
    g.set(
        "destroy",
        lua.create_function(|lua, o: Option<UserDataRef<Obj>>| with(lua, |w| Ok(o.map(|o| w.scene.destroy(o.0)).unwrap_or(false))))?,
    )?;
    g.set("camera", lua.create_userdata(Cam)?)?;
    g.set(
        "sun",
        lua.create_function(|lua, (dir, c, intensity): (Value, Value, Option<f32>)| {
            let dir = to_vec3(&dir, "sun direction")?;
            let c = if c.is_nil() { None } else { Some(to_color(&c, "sun color")?) };
            with(lua, |w| {
                let sun = &mut w.scene.sun;
                if dir != Vec3::ZERO {
                    sun.direction = dir;
                }
                if let Some(c) = c {
                    sun.color = c;
                }
                if let Some(i) = intensity {
                    sun.intensity = i;
                }
                Ok(())
            })
        })?,
    )?;
    g.set(
        "ambient",
        lua.create_function(|lua, c: Value| {
            let c = to_color(&c, "ambient")?;
            with(lua, |w| Ok(w.scene.ambient = c))
        })?,
    )?;
    g.set(
        "background",
        lua.create_function(|lua, c: Value| {
            let c = to_color(&c, "background")?;
            with(lua, |w| Ok(w.scene.background = c))
        })?,
    )?;
    g.set(
        "fog",
        lua.create_function(|lua, (c, near, far): (Value, Option<f32>, Option<f32>)| {
            let fog = match c {
                Value::Nil | Value::Boolean(false) => None,
                _ => Some(Fog { color: to_color(&c, "fog")?, near: near.unwrap_or(10.0), far: far.unwrap_or(60.0) }),
            };
            with(lua, |w| Ok(w.scene.fog = fog))
        })?,
    )?;
    g.set("dump_scene", lua.create_function(|lua, ()| with(lua, |w| Ok(w.scene.dump())))?)?;
    g.set("quit", lua.create_function(|lua, ()| with(lua, |w| Ok(w.quit())))?)?;
    g.set("time", lua.create_userdata(TimeRef)?)?;
    Ok(())
}

fn install_input(lua: &Lua, g: &Table) -> LuaResult<()> {
    let input = lua.create_table()?;
    input.set("down", lua.create_function(|lua, n: String| { let ks = keys(&n)?; with(lua, |w| Ok(ks.iter().any(|k| w.input.down(*k)))) })?)?;
    input.set("pressed", lua.create_function(|lua, n: String| { let ks = keys(&n)?; with(lua, |w| Ok(ks.iter().any(|k| w.input.pressed(*k)))) })?)?;
    input.set("released", lua.create_function(|lua, n: String| { let ks = keys(&n)?; with(lua, |w| Ok(ks.iter().any(|k| w.input.released(*k)))) })?)?;
    input.set("repeated", lua.create_function(|lua, n: String| { let ks = keys(&n)?; with(lua, |w| Ok(ks.iter().any(|k| w.input.repeated(*k)))) })?)?;
    input.set("text", lua.create_function(|lua, ()| with(lua, |w| Ok(w.input.text.clone())))?)?;
    input.set("mouse_down", lua.create_function(|lua, b: Option<String>| { let b = button(b)?; with(lua, |w| Ok(w.input.mouse_down(b))) })?)?;
    input.set("mouse_pressed", lua.create_function(|lua, b: Option<String>| { let b = button(b)?; with(lua, |w| Ok(w.input.mouse_pressed(b))) })?)?;
    input.set("mouse_released", lua.create_function(|lua, b: Option<String>| { let b = button(b)?; with(lua, |w| Ok(w.input.mouse_released(b))) })?)?;
    input.set("mouse", lua.create_function(|lua, ()| with(lua, |w| Ok((w.input.mouse_position.x, w.input.mouse_position.y))))?)?;
    input.set("mouse_delta", lua.create_function(|lua, ()| with(lua, |w| Ok((w.input.mouse_delta.x, w.input.mouse_delta.y))))?)?;
    input.set("wheel", lua.create_function(|lua, ()| with(lua, |w| Ok(w.input.wheel)))?)?;
    input.set(
        "lock_mouse",
        lua.create_function(|lua, on: Option<bool>| with(lua, |w| Ok(w.input.lock_mouse = on.unwrap_or(true))))?,
    )?;
    input.set("mouse_locked", lua.create_function(|lua, ()| with(lua, |w| Ok(w.input.lock_mouse)))?)?;
    install_gamepad(lua, &input)?;
    g.set("input", input)?;

    g.set(
        "print",
        lua.create_function(|_, args: Variadic<Value>| {
            let parts = args.iter().map(|v| v.to_string()).collect::<LuaResult<Vec<_>>>()?;
            log::info!("[script] {}", parts.join("\t"));
            Ok(())
        })?,
    )?;
    Ok(())
}

/// `window` global: fullscreen / title / size.
struct LuaWindow;

impl mlua::UserData for LuaWindow {
    fn add_fields<F: mlua::UserDataFields<Self>>(f: &mut F) {
        f.add_field_method_get("fullscreen", |lua, _| with(lua, |w| Ok(w.window.fullscreen)));
        f.add_field_method_set("fullscreen", |lua, _, on: bool| with(lua, |w| Ok(w.window.fullscreen = on)));
        f.add_field_method_get("title", |lua, _| with(lua, |w| Ok(w.window.title.clone())));
        f.add_field_method_set("title", |lua, _, t: Option<String>| with(lua, |w| Ok(w.window.title = t)));
        f.add_field_method_get("width", |lua, _| with(lua, |w| Ok(w.canvas.output_size().0)));
        f.add_field_method_get("height", |lua, _| with(lua, |w| Ok(w.canvas.output_size().1)));
    }
}

/// Registers every Spark global in `lua`. `lua` must already have [`Shared`] app data.
pub(crate) fn install(lua: &Lua) -> LuaResult<()> {
    lua.set_app_data(MeshCache(RefCell::new(HashMap::new())));
    let g = lua.globals();
    install_math(lua, &g)?;
    install_assets(lua, &g)?;
    install_scene(lua, &g)?;
    install_input(lua, &g)?;
    crate::flow::install(lua, &g)?;
    crate::physics::install(lua, &g)?;
    crate::audio::install(lua, &g)?;
    crate::draw::install(lua, &g)?;
    crate::light::install(lua, &g)?;
    crate::model::install(lua, &g)?;
    crate::save::install(lua, &g)?;
    crate::shader::install(lua, &g)?;
    crate::tween::install(lua)?;
    g.set("window", lua.create_userdata(LuaWindow)?)?;
    Ok(())
}
