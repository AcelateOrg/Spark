//! 2D drawing for scripts: `draw.*` (shapes, sprites, text, transforms), `screen.*`, `Font.load`.
//!
//! Coordinates are virtual pixels: origin top-left, Y down, screen 720 units tall by default.

use mlua::{Lua, MetaMethod, Table, UserData, UserDataMethods, Value};
use spark_core::{Align, Color, FontId, Layer, SpriteParams, TextParams, TextureId, Vec2, World};

use crate::convert::*;
use crate::types::LuaTexture;

/// Handle returned by `Font.load`.
#[derive(Clone, Copy)]
pub(crate) struct LuaFont(pub FontId);

impl UserData for LuaFont {
    fn add_methods<M: UserDataMethods<Self>>(m: &mut M) {
        m.add_meta_method(MetaMethod::ToString, |_, this, ()| Ok(format!("Font(#{})", this.0.0)));
    }
}

fn color_or_white(v: &Value, what: &str) -> LuaResult<Color> {
    if v.is_nil() { Ok(Color::WHITE) } else { to_color(v, what) }
}

fn to_align(v: &Value, what: &str) -> LuaResult<Align> {
    let s = to_str(v, what)?;
    Align::parse(&s).ok_or_else(|| rt(format!("{what}: unknown alignment '{s}' (use {})", Align::NAMES.join(", "))))
}

/// A 2D vector: `{x, y}`, `{x = .., y = ..}`, `vec3(x, y, _)`, or a number (both components).
fn to_vec2(v: &Value, what: &str) -> LuaResult<Vec2> {
    match v {
        Value::Number(_) | Value::Integer(_) => Ok(Vec2::splat(to_num(v, what)?)),
        _ => Ok(to_vec3(v, what)?.truncate()),
    }
}

/// Text accepts strings and numbers (`draw.text(score, ...)`).
fn to_text(v: &Value, what: &str) -> LuaResult<String> {
    match v {
        Value::String(s) => Ok(s.to_string_lossy()),
        Value::Integer(i) => Ok(i.to_string()),
        Value::Number(n) => Ok(if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", *n as i64) } else { format!("{n}") }),
        Value::Boolean(b) => Ok(b.to_string()),
        _ => Err(rt(format!("{what}: expected a string or a number, got {}", v.type_name()))),
    }
}

fn texture_of(w: &mut World, v: &Value, what: &str) -> LuaResult<TextureId> {
    match v {
        Value::String(s) => w.assets.load_texture(&s.to_string_lossy()).map_err(|e| rt(format!("{what}: {e}"))),
        Value::UserData(ud) => ud
            .borrow::<LuaTexture>()
            .map(|t| t.0)
            .map_err(|_| rt(format!("{what}: expected a Texture or an image path like \"player.png\""))),
        _ => Err(rt(format!("{what}: expected a Texture or an image path like \"player.png\", got {}", v.type_name()))),
    }
}

/// Calls `f(key, value)` for every option; unknown keys are reported with the allowed list.
fn each_opt(opts: &Option<Table>, f: &mut dyn FnMut(&str, &Value) -> LuaResult<bool>, what: &str, allowed: &str) -> LuaResult<()> {
    let Some(t) = opts else { return Ok(()) };
    for pair in t.pairs::<String, Value>() {
        let (k, v) = pair?;
        if !f(&k, &v)? {
            return Err(rt(format!("{what}: unknown option '{k}' (allowed: {allowed})")));
        }
    }
    Ok(())
}

const SHAPE_KEYS: &str = "align, line";
const SPRITE_KEYS: &str = "w, h, scale, rotation, color, align, flip_x, flip_y, region, grid, frame, filter";
const TEXT_KEYS: &str = "size, color, align, font, width, line_height, shadow";

fn text_params(opts: &Option<Table>, what: &str) -> LuaResult<(TextParams, Option<Color>)> {
    let mut p = TextParams::default();
    let mut shadow = None;
    each_opt(
        opts,
        &mut |k, v| {
            let w = format!("{what} option '{k}'");
            match k {
                "size" => p.size = to_num(v, &w)?,
                "color" => p.color = to_color(v, &w)?,
                "align" => p.align = to_align(v, &w)?,
                "width" => p.width = Some(to_num(v, &w)?),
                "line_height" => p.line_height = to_num(v, &w)?,
                "font" => {
                    let Value::UserData(ud) = v else { return Err(rt(format!("{w}: expected a font from Font.load(...)"))) };
                    p.font = ud.borrow::<LuaFont>().map_err(|_| rt(format!("{w}: expected a font from Font.load(...)")))?.0;
                }
                "shadow" => {
                    shadow = match v {
                        Value::Boolean(true) => Some(Color::rgba(0.0, 0.0, 0.0, 0.6)),
                        Value::Boolean(false) | Value::Nil => None,
                        _ => Some(to_color(v, &w)?),
                    }
                }
                _ => return Ok(false),
            }
            Ok(true)
        },
        what,
        TEXT_KEYS,
    )?;
    if p.size <= 0.0 {
        return Err(rt(format!("{what}: size must be > 0")));
    }
    Ok((p, shadow))
}

fn shape_opts(opts: &Option<Table>, what: &str, default_align: Align) -> LuaResult<(Align, Option<f32>)> {
    let (mut align, mut line) = (default_align, None);
    each_opt(
        opts,
        &mut |k, v| {
            let w = format!("{what} option '{k}'");
            match k {
                "align" => align = to_align(v, &w)?,
                "line" => line = Some(to_num(v, &w)?),
                _ => return Ok(false),
            }
            Ok(true)
        },
        what,
        SHAPE_KEYS,
    )?;
    Ok((align, line))
}

fn sprite(lua: &Lua, (tex, x, y, opts): (Value, f32, f32, Option<Table>)) -> LuaResult<()> {
    let what = "draw.sprite";
    with(lua, |w| {
        let id = texture_of(w, &tex, what)?;
        let size = w.assets.texture(id).map(|t| (t.width, t.height)).unwrap_or((1, 1));
        let mut p = SpriteParams::default();
        let (mut sw, mut sh) = (None, None);
        let (mut grid, mut frame) = (None, 1.0f32);
        each_opt(
            &opts,
            &mut |k, v| {
                let wh = format!("{what} option '{k}'");
                match k {
                    "w" => sw = Some(to_num(v, &wh)?),
                    "h" => sh = Some(to_num(v, &wh)?),
                    "scale" => p.scale = to_vec2(v, &wh)?,
                    "rotation" => p.rotation = to_num(v, &wh)?,
                    "color" => p.color = to_color(v, &wh)?,
                    "align" => p.align = to_align(v, &wh)?,
                    "flip_x" => p.flip_x = to_bool(v, &wh)?,
                    "flip_y" => p.flip_y = to_bool(v, &wh)?,
                    "filter" => p.nearest = Some(to_filter(v, &wh)? == spark_core::TextureFilter::Nearest),
                    "region" => {
                        let Value::Table(t) = v else { return Err(rt(format!("{wh}: expected {{x, y, w, h}} in texture pixels"))) };
                        let get = |a: &str, i: i64| -> LuaResult<f32> {
                            let v: Value = t.get(a)?;
                            let v = if v.is_nil() { t.get(i)? } else { v };
                            to_num(&v, &format!("{wh}.{a}"))
                        };
                        p.region = Some([get("x", 1)?, get("y", 2)?, get("w", 3)?, get("h", 4)?]);
                    }
                    "grid" => {
                        let g = to_vec2(v, &wh)?;
                        if g.x < 1.0 || g.y < 1.0 {
                            return Err(rt(format!("{wh}: expected {{columns, rows}}, both >= 1")));
                        }
                        grid = Some((g.x.floor(), g.y.floor()));
                    }
                    "frame" => frame = to_num(v, &wh)?,
                    _ => return Ok(false),
                }
                Ok(true)
            },
            what,
            SPRITE_KEYS,
        )?;
        if let Some((cols, rows)) = grid {
            // Frames are numbered from 1, left to right, top to bottom, and wrap around.
            let count = (cols * rows) as i64;
            let i = ((frame.floor() as i64 - 1).rem_euclid(count)) as f32;
            let (cw, ch) = (size.0 as f32 / cols, size.1 as f32 / rows);
            p.region = Some([(i % cols).floor() * cw, (i / cols).floor() * ch, cw, ch]);
        }
        let [_, _, rw, rh] = p.region.unwrap_or([0.0, 0.0, size.0 as f32, size.1 as f32]);
        p.size = match (sw, sh) {
            (None, None) => None,
            (Some(a), Some(b)) => Some(Vec2::new(a, b)),
            // One side given: keep the aspect ratio.
            (Some(a), None) => Some(Vec2::new(a, a * rh / rw.max(1e-6))),
            (None, Some(b)) => Some(Vec2::new(b * rw / rh.max(1e-6), b)),
        };
        w.canvas.sprite(id, size, Vec2::new(x, y), &p);
        Ok(())
    })
}

/// The global `screen` table: size in virtual pixels, mouse position.
fn install_screen(lua: &Lua, g: &Table) -> LuaResult<()> {
    let screen = lua.create_table()?;
    screen.set(
        "mouse",
        lua.create_function(|lua, ()| {
            with(lua, |w| {
                let p = w.canvas.to_virtual(w.input.mouse_position);
                Ok((p.x, p.y))
            })
        })?,
    )?;
    screen.set(
        "center",
        lua.create_function(|lua, ()| {
            with(lua, |w| {
                let s = w.canvas.size() * 0.5;
                Ok((s.x, s.y))
            })
        })?,
    )?;
    let meta = lua.create_table()?;
    meta.set(
        "__index",
        lua.create_function(|lua, (_, k): (Table, String)| {
            with(lua, |w| {
                let c = &w.canvas;
                Ok(match k.as_str() {
                    "width" => c.size().x,
                    "height" => c.size().y,
                    "scale" => c.scale(),
                    "pixel_width" => c.output_size().0 as f32,
                    "pixel_height" => c.output_size().1 as f32,
                    _ => {
                        return Err(rt(format!(
                            "screen.{k} does not exist (use screen.width, screen.height, screen.scale, screen.pixel_width, screen.pixel_height, screen.mouse(), screen.center())"
                        )));
                    }
                })
            })
        })?,
    )?;
    meta.set(
        "__newindex",
        lua.create_function(|lua, (_, k, v): (Table, String, Value)| match k.as_str() {
            "height" => {
                let h = to_num(&v, "screen.height")?;
                if h <= 0.0 {
                    return Err(rt("screen.height must be > 0 (virtual pixels, default 720)"));
                }
                with(lua, |w| Ok(w.canvas.virtual_height = h))
            }
            "width" => Err(rt("screen.width can't be set: it follows the window aspect ratio (set screen.height instead)")),
            _ => Err(rt(format!("screen.{k} can't be set (only screen.height)"))),
        })?,
    )?;
    screen.set_metatable(Some(meta))?;
    g.set("screen", screen)?;
    Ok(())
}

pub(crate) fn install(lua: &Lua, g: &Table) -> LuaResult<()> {
    let draw = lua.create_table()?;
    draw.set(
        "rect",
        lua.create_function(|lua, (x, y, ww, h, color, opts): (f32, f32, f32, f32, Value, Option<Table>)| {
            let c = color_or_white(&color, "draw.rect color")?;
            let (align, line) = shape_opts(&opts, "draw.rect", Align::TOP_LEFT)?;
            with(lua, |w| {
                let (pos, size) = (Vec2::new(x, y), Vec2::new(ww, h));
                match line {
                    Some(t) => w.canvas.rect_line(pos, size, t, c, align),
                    None => w.canvas.rect(pos, size, c, align),
                }
                Ok(())
            })
        })?,
    )?;
    draw.set(
        "circle",
        lua.create_function(|lua, (x, y, r, color, opts): (f32, f32, f32, Value, Option<Table>)| {
            let c = color_or_white(&color, "draw.circle color")?;
            let (_, line) = shape_opts(&opts, "draw.circle", Align::CENTER)?;
            with(lua, |w| {
                match line {
                    Some(t) => w.canvas.circle_line(Vec2::new(x, y), r, t, c),
                    None => w.canvas.circle(Vec2::new(x, y), r, c),
                }
                Ok(())
            })
        })?,
    )?;
    draw.set(
        "line",
        lua.create_function(|lua, (x1, y1, x2, y2, color, width): (f32, f32, f32, f32, Value, Option<f32>)| {
            let c = color_or_white(&color, "draw.line color")?;
            with(lua, |w| Ok(w.canvas.line(Vec2::new(x1, y1), Vec2::new(x2, y2), width.unwrap_or(2.0), c)))
        })?,
    )?;
    draw.set(
        "polygon",
        lua.create_function(|lua, (points, color, opts): (Table, Value, Option<Table>)| {
            let what = "draw.polygon";
            let c = color_or_white(&color, "draw.polygon color")?;
            let (_, line) = shape_opts(&opts, what, Align::TOP_LEFT)?;
            // Accepts {x1, y1, x2, y2, ...} or {{x, y}, {x, y}, ...}.
            let values: Vec<Value> = points.sequence_values::<Value>().collect::<LuaResult<_>>()?;
            let pts: Vec<Vec2> = if values.first().is_some_and(|v| matches!(v, Value::Number(_) | Value::Integer(_))) {
                if values.len() % 2 != 0 {
                    return Err(rt(format!("{what}: flat point list needs an even count (x1, y1, x2, y2, ...)")));
                }
                values.chunks(2).map(|p| Ok(Vec2::new(to_num(&p[0], what)?, to_num(&p[1], what)?))).collect::<LuaResult<_>>()?
            } else {
                values.iter().map(|v| to_vec2(v, what)).collect::<LuaResult<_>>()?
            };
            if pts.len() < 3 {
                return Err(rt(format!("{what}: needs at least 3 points, got {}", pts.len())));
            }
            with(lua, |w| {
                match line {
                    Some(t) => w.canvas.polygon_line(&pts, t, c),
                    None => w.canvas.polygon(&pts, c),
                }
                Ok(())
            })
        })?,
    )?;
    draw.set("sprite", lua.create_function(sprite)?)?;
    draw.set(
        "text",
        lua.create_function(|lua, (text, x, y, opts): (Value, f32, f32, Option<Table>)| {
            let text = to_text(&text, "draw.text")?;
            let (p, shadow) = text_params(&opts, "draw.text")?;
            with(lua, |w| {
                if let Some(sc) = shadow {
                    let off = (p.size / 16.0).max(1.0);
                    w.canvas.text(&text, Vec2::new(x + off, y + off), &TextParams { color: sc, ..p });
                }
                let s = w.canvas.text(&text, Vec2::new(x, y), &p);
                Ok((s.x, s.y))
            })
        })?,
    )?;
    draw.set(
        "measure",
        lua.create_function(|lua, (text, opts): (Value, Option<Table>)| {
            let text = to_text(&text, "draw.measure")?;
            let (p, _) = text_params(&opts, "draw.measure")?;
            with(lua, |w| {
                let s = w.canvas.measure_text(&text, &p);
                Ok((s.x, s.y))
            })
        })?,
    )?;
    draw.set(
        "spark_badge",
        lua.create_function(|lua, opts: Option<Table>| {
            let what = "draw.spark_badge";
            let mut p = spark_core::BadgeParams::default();
            let (mut x, mut y) = (None, None);
            each_opt(
                &opts,
                &mut |k, v| {
                    let wk = format!("{what} option '{k}'");
                    match k {
                        "x" => x = Some(to_num(v, &wk)?),
                        "y" => y = Some(to_num(v, &wk)?),
                        "align" => p.align = to_align(v, &wk)?,
                        "width" => p.width = to_num(v, &wk)?,
                        "margin" => p.margin = to_num(v, &wk)?,
                        "alpha" => p.alpha = to_num(v, &wk)?,
                        _ => return Ok(false),
                    }
                    Ok(true)
                },
                what,
                "x, y, align, width, margin, alpha",
            )?;
            with(lua, |w| {
                if x.is_some() || y.is_some() {
                    let s = w.canvas.size();
                    p.pos = Some(Vec2::new(x.unwrap_or(s.x * p.align.x), y.unwrap_or(s.y * p.align.y)));
                }
                let st = spark_core::branding::badge(w, &p);
                Ok((st.hovered, st.clicked))
            })
        })?,
    )?;
    draw.set("push", lua.create_function(|lua, ()| with(lua, |w| Ok(w.canvas.push())))?)?;
    draw.set(
        "pop",
        lua.create_function(|lua, ()| {
            with(lua, |w| {
                if w.canvas.pop() { Ok(()) } else { Err(rt("draw.pop() without a matching draw.push()")) }
            })
        })?,
    )?;
    draw.set("translate", lua.create_function(|lua, (x, y): (f32, f32)| with(lua, |w| Ok(w.canvas.translate(Vec2::new(x, y)))))?)?;
    draw.set("rotate", lua.create_function(|lua, a: f32| with(lua, |w| Ok(w.canvas.rotate(a))))?)?;
    draw.set(
        "scale",
        lua.create_function(|lua, (sx, sy): (f32, Option<f32>)| with(lua, |w| Ok(w.canvas.scale_by(Vec2::new(sx, sy.unwrap_or(sx))))))?,
    )?;
    draw.set(
        "layer",
        lua.create_function(|lua, name: Option<String>| {
            with(lua, |w| {
                let old = w.canvas.layer().name();
                if let Some(n) = name {
                    let l = Layer::parse(&n).ok_or_else(|| rt(format!("draw.layer: unknown layer '{n}' (use \"ui\" or \"scene\")")))?;
                    w.canvas.set_layer(l);
                }
                Ok(old)
            })
        })?,
    )?;
    draw.set(
        "filter",
        lua.create_function(|lua, v: Value| {
            let f = to_filter(&v, "draw.filter")?;
            with(lua, |w| Ok(w.canvas.nearest = f == spark_core::TextureFilter::Nearest))
        })?,
    )?;
    draw.set(
        "stats",
        lua.create_function(|lua, ()| {
            let t = lua.create_table()?;
            with(lua, |w| {
                for (name, layer) in [("ui", Layer::Ui), ("scene", Layer::Scene)] {
                    let d = w.canvas.layer_data(layer);
                    t.set(format!("{name}_triangles"), d.vertices.len() / 3)?;
                    t.set(format!("{name}_batches"), d.batches.len())?;
                }
                Ok(())
            })?;
            Ok(t)
        })?,
    )?;
    g.set("draw", draw)?;

    let font = lua.create_table()?;
    font.set(
        "load",
        lua.create_function(|lua, path: String| {
            with(lua, |w| {
                let full = w.assets.resolve(&path);
                w.canvas.fonts.load(&full).map(LuaFont).map_err(rt)
            })
        })?,
    )?;
    g.set("Font", font)?;
    install_screen(lua, g)
}
