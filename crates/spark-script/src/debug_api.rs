//! Unified Luau `debug` global: overlay control + standard library passthrough.
//!
//! Merges Spark's debug-overlay API with Luau's built-in `debug` library so
//! scripts can use a single `debug` table:
//!
//! ```lua
//! debug.enabled = true           -- show/hide the overlay
//! debug.fps                     -- current FPS (read-only)
//! debug.frame_time              -- current frame time in ms (read-only)
//! debug.show_fps = true         -- toggle individual overlay sections
//! debug.hotkey = false          -- F1 no longer toggles the overlay (for games that use F1)
//! debug.add_line("hello", color) -- add a custom line
//! debug.clear_lines()           -- clear user-added lines
//! debug.toggle()                -- toggle overlay on/off
//! debug.avg_frame_ms()         -- average frame time over the last 120 frames
//! -- Standard library still works:
//! debug.traceback()             -- Luau standard debug.traceback
//! ```

use mlua::{IntoLua, Lua, Table, Value};

use crate::convert::{to_bool, to_color, with, LuaResult};

/// Install the unified `debug` global, preserving the standard Luau debug library.
pub(crate) fn install(lua: &Lua) -> LuaResult<()> {
    // Save the standard Luau debug library (if present) before replacing the global.
    let std_debug = lua.globals().get::<Table>("debug").ok();

    // Our custom debug table — functions are set directly on it;
    // dynamic fields (enabled, fps, show_*) go through the metatable.
    let dbg = lua.create_table()?;

    // ── Functions (directly on the table) ───────────────────────────

    dbg.set("add_line", lua.create_function(|lua, (text, color): (String, Value)| {
        let c = if color.is_nil() {
            spark_core::Color::WHITE
        } else {
            to_color(&color, "debug.add_line")?
        };
        with(lua, |w| Ok(w.debug.add_line(text, c)))
    })?)?;

    dbg.set("clear_lines", lua.create_function(|lua, ()| {
        with(lua, |w| Ok(w.debug.clear_lines()))
    })?)?;

    dbg.set("toggle", lua.create_function(|lua, ()| {
        with(lua, |w| {
            w.debug.toggle();
            Ok(w.debug.enabled())
        })
    })?)?;

    dbg.set("avg_frame_ms", lua.create_function(|lua, ()| {
        with(lua, |w| Ok(w.debug.avg_frame_ms()))
    })?)?;

    // ── Metatable for dynamic fields + standard-library passthrough ──

    let mt = lua.create_table()?;

    // `__index`: custom reads first, then fall back to the std library (traceback, info, ...).
    let std_for_index = std_debug;
    mt.set(
        "__index",
        lua.create_function(move |lua, (_self, key): (Table, String)| {
            match key.as_str() {
                "enabled" => with(lua, |w| w.debug.enabled().into_lua(lua)),
                "hotkey" => with(lua, |w| w.debug.hotkey.into_lua(lua)),
                "fps" => with(lua, |w| w.debug.fps.into_lua(lua)),
                "frame_time" => with(lua, |w| w.debug.frame_time_ms.into_lua(lua)),
                "show_fps" => with(lua, |w| w.debug.show_fps().into_lua(lua)),
                "show_gpu" => with(lua, |w| w.debug.show_gpu().into_lua(lua)),
                "show_scene" => with(lua, |w| w.debug.show_scene().into_lua(lua)),
                "show_physics" => with(lua, |w| w.debug.show_physics().into_lua(lua)),
                _ => match &std_for_index {
                    Some(std) => std.get(key),
                    None => Ok(Value::Nil),
                },
            }
        })?,
    )?;

    // `__newindex`: overlay settings only. Writes never reach the shared standard library table.
    mt.set(
        "__newindex",
        lua.create_function(|lua, (_self, key, value): (Table, String, Value)| {
            let what = format!("debug.{key}");
            let on = || to_bool(&value, &what);
            match key.as_str() {
                "enabled" => with(lua, |w| Ok(w.debug.set_enabled(on()?))),
                "hotkey" => with(lua, |w| Ok(w.debug.hotkey = on()?)),
                "show_fps" => with(lua, |w| Ok(w.debug.set_show_fps(on()?))),
                "show_gpu" => with(lua, |w| Ok(w.debug.set_show_gpu(on()?))),
                "show_scene" => with(lua, |w| Ok(w.debug.set_show_scene(on()?))),
                "show_physics" => with(lua, |w| Ok(w.debug.set_show_physics(on()?))),
                "fps" | "frame_time" => Err(mlua::Error::runtime(format!("{what} is read-only"))),
                _ => Err(mlua::Error::runtime(format!(
                    "{what}: unknown field (settable: enabled, hotkey, show_fps, show_gpu, show_scene, show_physics)"
                ))),
            }
        })?,
    )?;

    dbg.set_metatable(Some(mt))?;
    lua.globals().set("debug", dbg)?;
    Ok(())
}
