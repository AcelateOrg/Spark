//! Luau `ui` global: immediate-mode widgets drawn on the UI layer.
//!
//! ```lua
//! function render()
//!     if ui.button("start", 10, 10, 200, 40, "Start") then start_game() end
//!     volume = ui.slider("volume", 10, 60, 200, 20, volume, 0, 1)
//!     music = ui.checkbox("music", 10, 90, 20, music)
//! end
//! ```

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use mlua::{Lua, Value};
use spark_core::{Rect, Vec2};

use crate::convert::{rt, to_color, with, LuaResult};

/// Widget ids: any string or integer that stays the same between frames.
fn widget_id(v: &Value, what: &str) -> LuaResult<u64> {
    let mut h = DefaultHasher::new();
    match v {
        Value::String(s) => s.as_bytes().hash(&mut h),
        Value::Integer(i) => i.hash(&mut h),
        Value::Number(n) if n.fract() == 0.0 => (*n as i64).hash(&mut h),
        _ => return Err(rt(format!("{what}: the first argument is the widget id (a string or an integer that stays the same every frame), got {}", v.type_name()))),
    }
    Ok(h.finish())
}

fn color_or(v: &Value, default: spark_core::Color, what: &str) -> LuaResult<spark_core::Color> {
    if v.is_nil() { Ok(default) } else { to_color(v, what) }
}

pub(crate) fn install(lua: &Lua) -> LuaResult<()> {
    let ui = lua.create_table()?;

    ui.set(
        "panel",
        lua.create_function(|lua, (x, y, width, height, bg, border): (f32, f32, f32, f32, Value, Value)| {
            with(lua, |w| {
                let bg = color_or(&bg, w.ui.style.panel, "ui.panel color")?;
                let border = if border.is_nil() { None } else { Some((1.0, to_color(&border, "ui.panel border")?)) };
                Ok(w.ui.panel(&mut w.canvas, Rect::new(x, y, width, height), bg, border))
            })
        })?,
    )?;

    ui.set(
        "button",
        lua.create_function(|lua, (id, x, y, width, height, text): (Value, f32, f32, f32, f32, String)| {
            let id = widget_id(&id, "ui.button")?;
            with(lua, |w| Ok(w.ui.button(&mut w.canvas, id, Rect::new(x, y, width, height), &text)))
        })?,
    )?;

    ui.set(
        "label",
        lua.create_function(|lua, (x, y, text, size, color): (f32, f32, String, Option<f32>, Value)| {
            with(lua, |w| {
                let color = color_or(&color, w.ui.style.text, "ui.label color")?;
                w.ui.label(&mut w.canvas, Vec2::new(x, y), &text, size.unwrap_or(16.0), color);
                Ok(())
            })
        })?,
    )?;

    ui.set(
        "image",
        lua.create_function(|lua, (x, y, width, height, texture): (f32, f32, f32, f32, Value)| {
            with(lua, |w| {
                let tex = crate::draw::texture_of(w, &texture, "ui.image")?;
                w.ui.image(&mut w.canvas, Rect::new(x, y, width, height), tex);
                Ok(())
            })
        })?,
    )?;

    ui.set(
        "slider",
        lua.create_function(
            |lua, (id, x, y, width, height, value, min, max): (Value, f32, f32, f32, f32, f32, Option<f32>, Option<f32>)| {
                let id = widget_id(&id, "ui.slider")?;
                let (min, max) = (min.unwrap_or(0.0), max.unwrap_or(1.0));
                with(lua, |w| Ok(w.ui.slider(&mut w.canvas, id, Rect::new(x, y, width, height), value, min, max).unwrap_or(value)))
            },
        )?,
    )?;

    ui.set(
        "checkbox",
        lua.create_function(|lua, (id, x, y, size, checked): (Value, f32, f32, f32, bool)| {
            let id = widget_id(&id, "ui.checkbox")?;
            with(lua, |w| Ok(w.ui.checkbox(&mut w.canvas, id, Rect::new(x, y, size, size), checked).unwrap_or(checked)))
        })?,
    )?;

    ui.set(
        "progress",
        lua.create_function(|lua, (x, y, width, height, progress, color): (f32, f32, f32, f32, f32, Value)| {
            with(lua, |w| {
                let color = color_or(&color, w.ui.style.fill, "ui.progress color")?;
                w.ui.progress(&mut w.canvas, Rect::new(x, y, width, height), progress, color);
                Ok(())
            })
        })?,
    )?;

    ui.set("hovered", lua.create_function(|lua, ()| with(lua, |w| Ok(w.ui.hovered())))?)?;

    lua.globals().set("ui", ui)?;
    Ok(())
}
