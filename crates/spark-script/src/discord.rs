//! `discord` global: Discord Rich Presence (the IPC itself lives in `spark-discord`, outside the Lua state, so
//! presence and its "elapsed" clock survive hot reload). Without the `discord` feature every call is a no-op and
//! `discord.status()` is "off", so games run unchanged on builds without it.
//!
//! ```lua
//! discord.connect("123456789012345678")
//! discord.set({ details = "Main menu", large_image = "logo", start = true })
//! ```

use mlua::{Lua, Table};

use crate::convert::*;

#[cfg(feature = "discord")]
fn to_presence(t: &Table) -> LuaResult<spark_discord::Presence> {
    use mlua::Value;
    const KEYS: &str = "details, state, large_image, large_text, small_image, small_text, start, finish, buttons";
    let mut p = spark_discord::Presence::default();
    for pair in t.pairs::<String, Value>() {
        let (k, v) = pair?;
        let what = format!("discord.set: {k}");
        let text = |v: &Value| -> LuaResult<Option<String>> { if v.is_nil() { Ok(None) } else { to_str(v, &what).map(Some) } };
        let time = |v: &Value| -> LuaResult<Option<i64>> {
            match v {
                Value::Nil | Value::Boolean(false) => Ok(None),
                Value::Boolean(true) => Ok(Some(spark_discord::session_start())),
                _ => Ok(Some(to_num(v, &what)? as i64)),
            }
        };
        match k.as_str() {
            "details" => p.details = text(&v)?,
            "state" => p.state = text(&v)?,
            "large_image" => p.large_image = text(&v)?,
            "large_text" => p.large_text = text(&v)?,
            "small_image" => p.small_image = text(&v)?,
            "small_text" => p.small_text = text(&v)?,
            "start" => p.start = time(&v)?,
            // `end` is a Luau keyword: `{ finish = os.time() + 60 }` reads better than `{ ["end"] = ... }`
            "finish" | "end" => {
                p.end = match v {
                    Value::Boolean(true) => return Err(rt(format!("{what}: expected unix seconds (os.time() + n)"))),
                    _ => time(&v)?,
                }
            }
            "buttons" => {
                let Value::Table(list) = v else {
                    return Err(rt(format!("{what}: expected {{ {{ label = \"...\", url = \"https://...\" }}, ... }}")));
                };
                for b in list.sequence_values::<Table>() {
                    let b = b.map_err(|_| rt(format!("{what}: every button is {{ label = \"...\", url = \"https://...\" }}")))?;
                    let label = to_str(&b.get("label")?, &format!("{what}.label"))?;
                    let url = to_str(&b.get("url")?, &format!("{what}.url"))?;
                    if !(url.starts_with("https://") || url.starts_with("http://")) {
                        return Err(rt(format!("{what}: button url must start with https:// (got \"{url}\")")));
                    }
                    p.buttons.push((label, url));
                }
                if p.buttons.len() > 2 {
                    return Err(rt(format!("{what}: Discord shows at most 2 buttons, got {}", p.buttons.len())));
                }
            }
            _ => return Err(rt(format!("discord.set: unknown key '{k}' (allowed: {KEYS})"))),
        }
    }
    Ok(p)
}

pub(crate) fn install(lua: &Lua, g: &Table) -> LuaResult<()> {
    let d = lua.create_table()?;
    #[cfg(feature = "discord")]
    {
        d.set(
            "connect",
            lua.create_function(|_, app_id: String| {
                if app_id.trim().is_empty() || !app_id.trim().chars().all(|c| c.is_ascii_digit()) {
                    return Err(rt(format!(
                        "discord.connect: expected the Application ID (digits) from discord.com/developers, got \"{app_id}\""
                    )));
                }
                spark_discord::connect(&app_id);
                Ok(())
            })?,
        )?;
        d.set(
            "set",
            lua.create_function(|_, t: Table| {
                spark_discord::set(to_presence(&t)?);
                Ok(())
            })?,
        )?;
        d.set(
            "clear",
            lua.create_function(|_, ()| {
                spark_discord::clear();
                Ok(())
            })?,
        )?;
        d.set(
            "shutdown",
            lua.create_function(|_, ()| {
                spark_discord::shutdown();
                Ok(())
            })?,
        )?;
        d.set(
            "status",
            lua.create_function(|_, ()| {
                Ok(match spark_discord::status() {
                    spark_discord::Status::Off => "off",
                    spark_discord::Status::Connecting => "connecting",
                    spark_discord::Status::Connected => "connected",
                })
            })?,
        )?;
    }
    #[cfg(not(feature = "discord"))]
    {
        d.set("connect", lua.create_function(|_, _: String| Ok(()))?)?;
        d.set("set", lua.create_function(|_, _: Table| Ok(()))?)?;
        d.set("clear", lua.create_function(|_, ()| Ok(()))?)?;
        d.set("shutdown", lua.create_function(|_, ()| Ok(()))?)?;
        d.set("status", lua.create_function(|_, ()| Ok("off"))?)?;
    }
    g.set("discord", d)?;
    Ok(())
}
