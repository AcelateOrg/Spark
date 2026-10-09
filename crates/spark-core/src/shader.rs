//! Custom WGSL shaders. The engine ships no looks: games write their own surface and post shaders.
//!
//! A shader file is plain WGSL. The engine appends a prelude (uniforms, helpers, entry points),
//! so a whole shader can be one function:
//!
//! ```wgsl
//! // Surface shader (materials): optional `vertex` and / or `fragment`.
//! fn fragment(f: Fragment) -> vec4<f32> {
//!     let c = default_fragment(f);
//!     return vec4<f32>(floor(c.rgb * 4.0) / 4.0, c.a);   // posterize
//! }
//!
//! // Post shader (full screen): `post`.
//! struct Params { strength: f32 }
//! fn post(p: PostInput) -> vec4<f32> {
//!     let c = input_color(p.uv);
//!     return vec4<f32>(mix(c.rgb, vec3<f32>(dot(c.rgb, vec3<f32>(0.3, 0.59, 0.11))), params.strength), 1.0);
//! }
//! ```
//!
//! An optional `struct Params { ... }` (f32 / i32 / u32 / vec2-4<f32> fields) becomes the
//! `params` uniform; games set its fields by name ([`ShaderData::set_param`]). Your code comes
//! first in the final source, so compile errors point at the lines of your file.

use naga::{ScalarKind, TypeInner, VectorSize};

use crate::assets::TextureId;

/// Handle to a shader in [`crate::Assets`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShaderId(pub u32);

/// What a shader draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderKind {
    /// Meshes (materials): optional `fn vertex(v: VertexInput) -> Fragment` / `fn fragment(f: Fragment) -> vec4<f32>`.
    Surface,
    /// Full-screen pass: `fn post(p: PostInput) -> vec4<f32>`.
    Post,
}

/// Type of one `Params` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamType {
    F32,
    I32,
    U32,
    Vec2,
    Vec3,
    Vec4,
}

impl ParamType {
    /// Number of components.
    pub fn len(self) -> usize {
        match self {
            Self::F32 | Self::I32 | Self::U32 => 1,
            Self::Vec2 => 2,
            Self::Vec3 => 3,
            Self::Vec4 => 4,
        }
    }

    pub fn wgsl(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::Vec2 => "vec2<f32>",
            Self::Vec3 => "vec3<f32>",
            Self::Vec4 => "vec4<f32>",
        }
    }
}

/// One field of the shader's `Params` struct.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: ParamType,
    /// Byte offset in the uniform buffer.
    pub offset: u32,
}

/// Largest `Params` struct.
pub const MAX_PARAMS_SIZE: usize = 256;

const COMMON: &str = include_str!("shaders/common.wgsl");
const SURFACE: &str = include_str!("shaders/surface.wgsl");
const POST: &str = include_str!("shaders/post.wgsl");

/// A compiled (validated) shader plus its parameter values.
#[derive(Clone, Debug)]
pub struct ShaderData {
    /// File name / label, used in error messages.
    pub name: String,
    pub kind: ShaderKind,
    /// The game's code.
    pub code: String,
    /// Full WGSL given to the GPU (code + prelude).
    pub source: String,
    pub params: Vec<Param>,
    /// `Params` uniform contents (std140-like WGSL uniform layout, at least 16 bytes).
    pub values: Vec<u8>,
    /// `texture1` / `texture2` (white when `None`).
    pub textures: [Option<TextureId>; 2],
    /// Bumped when the code changes (hot reload); the renderer rebuilds its pipeline.
    pub version: u64,
}

/// `code` with `// line` and `/* block */` comments replaced by spaces (line numbers kept).
fn strip_comments(code: &str) -> String {
    let b = code.as_bytes();
    let mut out = String::with_capacity(code.len());
    let mut i = 0;
    let mut depth = 0usize; // WGSL block comments nest
    let mut line = false;
    let mut start = 0;
    while i < b.len() {
        if line {
            if b[i] == b'\n' {
                line = false;
                start = i; // the newline itself is kept
            }
            i += 1;
        } else if depth > 0 {
            if b[i..].starts_with(b"*/") {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    start = i;
                }
            } else if b[i..].starts_with(b"/*") {
                depth += 1;
                i += 2;
            } else {
                if b[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
        } else if b[i..].starts_with(b"//") {
            out.push_str(&code[start..i]);
            line = true;
            i += 2;
        } else if b[i..].starts_with(b"/*") {
            out.push_str(&code[start..i]);
            out.push(' ');
            depth = 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    if !line && depth == 0 {
        out.push_str(&code[start..]);
    }
    out
}

/// `fn <name>(` somewhere in `code`, ignoring comments.
fn defines_fn(code: &str, name: &str) -> bool {
    let code = strip_comments(code);
    let mut rest = code.as_str();
    while let Some(i) = rest.find("fn") {
        let before_ok = i == 0 || !rest.as_bytes()[i - 1].is_ascii_alphanumeric() && rest.as_bytes()[i - 1] != b'_';
        let after = rest[i + 2..].trim_start();
        if before_ok && rest[i + 2..].starts_with(char::is_whitespace) {
            if let Some(tail) = after.strip_prefix(name) {
                if tail.trim_start().starts_with('(') {
                    return true;
                }
            }
        }
        rest = &rest[i + 2..];
    }
    false
}

fn defines_struct(code: &str, name: &str) -> bool {
    let code = strip_comments(code);
    code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .collect::<Vec<_>>()
        .windows(2)
        .any(|w| w[0] == "struct" && w[1] == name)
        || code.contains(&format!("struct {name}{{"))
}

/// Game code + engine prelude for `kind`.
pub fn compose(code: &str, kind: ShaderKind) -> String {
    let mut s = String::with_capacity(code.len() + 8192);
    s.push_str(code);
    s.push_str(COMMON);
    if !defines_struct(code, "Params") {
        s.push_str("\nstruct Params { spark_unused: vec4<f32>, };\n");
    }
    match kind {
        ShaderKind::Surface => {
            let vs = if defines_fn(code, "vertex") { "vertex" } else { "default_vertex" };
            let fs = if defines_fn(code, "fragment") { "fragment" } else { "default_fragment" };
            s.push_str(&SURFACE.replace("SPARK_VERTEX", vs).replace("SPARK_FRAGMENT", fs));
        }
        ShaderKind::Post => s.push_str(POST),
    }
    s
}

impl ShaderData {
    /// Validates `code` and builds the shader. `kind` is detected: `fn post(` = post shader,
    /// anything else = surface shader. Errors are human readable, with line numbers of `code`.
    pub fn compile(name: &str, code: &str) -> Result<Self, String> {
        let post = defines_fn(code, "post");
        if post && (defines_fn(code, "vertex") || defines_fn(code, "fragment")) {
            return Err(format!(
                "{name}: a shader is either a post shader (fn post) or a surface shader (fn vertex / fn fragment), not both"
            ));
        }
        let kind = if post { ShaderKind::Post } else { ShaderKind::Surface };
        let source = compose(code, kind);
        let module = naga::front::wgsl::parse_str(&source).map_err(|e| ascii(e.emit_to_string_with_path(&source, name)))?;
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
            .validate(&module)
            .map_err(|e| ascii(e.emit_to_string_with_path(&source, name)))?;
        let (params, size) = params_layout(&module).map_err(|e| format!("{name}: {e}"))?;
        Ok(Self {
            name: name.to_string(),
            kind,
            code: code.to_string(),
            source,
            params,
            values: vec![0; size.max(16)],
            textures: [None, None],
            version: 0,
        })
    }

    /// Replaces the code (hot reload). Keeps values of params that still exist with the same type.
    pub fn recompile(&mut self, code: &str) -> Result<(), String> {
        let mut new = Self::compile(&self.name, code)?;
        for p in &self.params {
            if new.params.iter().any(|q| q.name == p.name && q.ty == p.ty) {
                let v = self.param(&p.name).expect("own param");
                new.set_param(&p.name, &v).expect("same type");
            }
        }
        new.textures = self.textures;
        new.version = self.version + 1;
        *self = new;
        Ok(())
    }

    pub fn find_param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|p| p.name == name)
    }

    /// Sets a `Params` field. `value` must have exactly the field's component count, except that a
    /// vec4 accepts 3 values (alpha = 1) and a vec3 accepts 4 (alpha dropped), so colors fit both.
    pub fn set_param(&mut self, name: &str, value: &[f32]) -> Result<(), String> {
        let Some(p) = self.find_param(name).cloned() else {
            let names: Vec<&str> = self.params.iter().map(|p| p.name.as_str()).collect();
            return Err(if names.is_empty() {
                format!("{}: no param '{name}' (the shader has no `struct Params`)", self.name)
            } else {
                format!("{}: no param '{name}' (params: {})", self.name, names.join(", "))
            });
        };
        let n = p.ty.len();
        let mut v = [0.0f32, 0.0, 0.0, 1.0];
        let ok = value.len() == n || (n == 4 && value.len() == 3) || (n == 3 && value.len() == 4);
        if !ok {
            return Err(format!("{}: param '{name}' is {}, got {} number(s)", self.name, p.ty.wgsl(), value.len()));
        }
        for (i, x) in value.iter().take(n).enumerate() {
            v[i] = *x;
        }
        let at = p.offset as usize;
        for i in 0..n {
            let bytes = match p.ty {
                ParamType::I32 => (v[i] as i32).to_le_bytes(),
                ParamType::U32 => (v[i].max(0.0) as u32).to_le_bytes(),
                _ => v[i].to_le_bytes(),
            };
            self.values[at + i * 4..at + i * 4 + 4].copy_from_slice(&bytes);
        }
        Ok(())
    }

    /// Current value of a `Params` field.
    pub fn param(&self, name: &str) -> Option<Vec<f32>> {
        let p = self.find_param(name)?;
        let at = p.offset as usize;
        Some(
            (0..p.ty.len())
                .map(|i| {
                    let b: [u8; 4] = self.values[at + i * 4..at + i * 4 + 4].try_into().expect("4 bytes");
                    match p.ty {
                        ParamType::I32 => i32::from_le_bytes(b) as f32,
                        ParamType::U32 => u32::from_le_bytes(b) as f32,
                        _ => f32::from_le_bytes(b),
                    }
                })
                .collect(),
        )
    }
}

/// Compiler messages use box-drawing characters; game fonts may not have them.
fn ascii(msg: String) -> String {
    msg.replace("┌─", "-->").replace('│', "|").replace('─', "-").replace('·', "-").trim_end().to_string()
}

/// Fields of `struct Params` (if the game declared one) and the uniform size.
fn params_layout(module: &naga::Module) -> Result<(Vec<Param>, usize), String> {
    for (_, ty) in module.types.iter() {
        if ty.name.as_deref() != Some("Params") {
            continue;
        }
        let TypeInner::Struct { members, span } = &ty.inner else { continue };
        if *span as usize > MAX_PARAMS_SIZE {
            return Err(format!("struct Params is {span} bytes, the limit is {MAX_PARAMS_SIZE}"));
        }
        let mut out = Vec::new();
        for m in members {
            let name = m.name.clone().unwrap_or_default();
            if name == "spark_unused" {
                continue; // placeholder added by `compose` when the game declares no Params
            }
            let pt = match &module.types[m.ty].inner {
                TypeInner::Scalar(s) if s.width == 4 => match s.kind {
                    ScalarKind::Float => Some(ParamType::F32),
                    ScalarKind::Sint => Some(ParamType::I32),
                    ScalarKind::Uint => Some(ParamType::U32),
                    _ => None,
                },
                TypeInner::Vector { size, scalar } if scalar.kind == ScalarKind::Float && scalar.width == 4 => Some(match size {
                    VectorSize::Bi => ParamType::Vec2,
                    VectorSize::Tri => ParamType::Vec3,
                    VectorSize::Quad => ParamType::Vec4,
                }),
                _ => None,
            };
            let Some(ty) = pt else {
                return Err(format!("Params.{name}: only f32, i32, u32, vec2<f32>, vec3<f32>, vec4<f32> are supported"));
            };
            out.push(Param { name, ty, offset: m.offset });
        }
        return Ok((out, (*span as usize).div_ceil(16) * 16));
    }
    Ok((Vec::new(), 16))
}

/// Built-in passthrough post shader (used when the game sets no post passes).
pub const COPY_POST: &str = "fn post(p: PostInput) -> vec4<f32> { return input_color(p.uv); }\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_surface_and_copy_compile() {
        let s = ShaderData::compile("empty.wgsl", "").unwrap();
        assert_eq!(s.kind, ShaderKind::Surface);
        assert!(s.params.is_empty());
        let p = ShaderData::compile("copy.wgsl", COPY_POST).unwrap();
        assert_eq!(p.kind, ShaderKind::Post);
    }

    #[test]
    fn custom_hooks_and_params() {
        let code = "struct Params { amount: f32, tint: vec3<f32>, steps: i32, }\n\
            fn vertex(v: VertexInput) -> Fragment { var f = default_vertex(v); f.custom = vec4<f32>(params.amount); return f; }\n\
            fn fragment(f: Fragment) -> vec4<f32> { let c = default_fragment(f); return vec4<f32>(c.rgb * params.tint * f.custom.x, c.a); }\n";
        let mut s = ShaderData::compile("fx.wgsl", code).unwrap();
        assert!(s.source.contains("var f = vertex(v);") && s.source.contains("return fragment(f);"));
        assert_eq!(s.params.len(), 3);
        assert_eq!(s.find_param("tint").unwrap().offset, 16);
        assert_eq!(s.values.len(), 32);
        s.set_param("amount", &[0.5]).unwrap();
        s.set_param("tint", &[1.0, 0.5, 0.25, 1.0]).unwrap();
        s.set_param("steps", &[4.0]).unwrap();
        assert_eq!(s.param("tint").unwrap(), vec![1.0, 0.5, 0.25]);
        assert_eq!(s.param("steps").unwrap(), vec![4.0]);
        assert!(s.set_param("nope", &[1.0]).unwrap_err().contains("amount, tint, steps"));
        assert!(s.set_param("amount", &[1.0, 2.0]).is_err());
        s.recompile(&code.replace("steps: i32", "steps: f32")).unwrap();
        assert_eq!(s.param("amount").unwrap(), vec![0.5]);
        assert_eq!(s.param("steps").unwrap(), vec![0.0]);
        assert_eq!(s.version, 1);
    }

    #[test]
    fn post_with_depth_and_errors_point_at_user_lines() {
        let ok = "fn post(p: PostInput) -> vec4<f32> {\n  let d = linear_depth(p.uv);\n  return vec4<f32>(input_color(p.uv).rgb / d, 1.0);\n}\n";
        assert_eq!(ShaderData::compile("fog.wgsl", ok).unwrap().kind, ShaderKind::Post);
        let bad = "fn post(p: PostInput) -> vec4<f32> {\n  return input_color(p.uv) * oops;\n}\n";
        let err = ShaderData::compile("bad.wgsl", bad).unwrap_err();
        assert!(err.contains("bad.wgsl:2"), "{err}");
        let both = "fn post(p: PostInput) -> vec4<f32> { return vec4<f32>(1.0); }\nfn fragment(f: Fragment) -> vec4<f32> { return vec4<f32>(1.0); }";
        assert!(ShaderData::compile("both.wgsl", both).is_err());
    }

    #[test]
    fn fn_detection() {
        assert!(defines_fn("fn vertex(v: VertexInput)", "vertex"));
        assert!(!defines_fn("// fn vertex(v: VertexInput)\nfn fragment(f: Fragment)", "vertex"));
        assert!(!defines_fn("/* old: fn post(p: PostInput) /* nested */ */ fn fragment(f: Fragment)", "post"));
        assert!(defines_fn("/* c */ fn post(p: PostInput)", "post"));
        assert!(!defines_struct("// struct Params { a: f32 }", "Params"));
        assert_eq!(strip_comments("a // b\nc /* d\n */ e").lines().count(), 3);
        assert_eq!(strip_comments("x // привет\ny /* мир */ z"), "x \ny   z");
        assert!(defines_fn("x;\nfn  fragment (f: Fragment)", "fragment"));
        assert!(!defines_fn("fn my_vertex(v: VertexInput)", "vertex"));
        assert!(!defines_fn("fn vertex2(v: VertexInput)", "vertex"));
        assert!(defines_struct("struct Params {", "Params"));
        assert!(!defines_struct("struct MyParams {", "Params"));
    }
}
