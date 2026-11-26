//! A thin layer over WebGL2: shader programs with cached uniform locations, meshes in vertex array objects,
//! textures and framebuffers.

use std::cell::RefCell;
use std::collections::HashMap;

use glam::{Mat4, Vec3};
use wasm_bindgen::JsCast;
use web_sys::{WebGl2RenderingContext as GL, WebGlProgram, WebGlTexture, WebGlUniformLocation, WebGlVertexArrayObject};

pub fn bytes<T: Copy>(v: &[T]) -> &[u8] {
    // SAFETY: plain numbers (f32, u32) with no padding, viewed as bytes for upload.
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

pub struct Program {
    pub p: WebGlProgram,
    locs: RefCell<HashMap<&'static str, Option<WebGlUniformLocation>>>,
}

pub const ATTR_POS: u32 = 0;
pub const ATTR_NORMAL: u32 = 1;
pub const ATTR_A2: u32 = 2;
pub const ATTR_A3: u32 = 3;
pub const ATTR_A4: u32 = 4;

impl Program {
    pub fn new(gl: &GL, vs: &str, fs: &str, attrs: &[(&str, u32)]) -> Result<Program, String> {
        let compile = |kind: u32, src: &str| -> Result<web_sys::WebGlShader, String> {
            let s = gl.create_shader(kind).ok_or("create_shader")?;
            gl.shader_source(&s, src);
            gl.compile_shader(&s);
            if gl.get_shader_parameter(&s, GL::COMPILE_STATUS).as_bool() != Some(true) {
                return Err(format!("shader: {}\n{}", gl.get_shader_info_log(&s).unwrap_or_default(), src));
            }
            Ok(s)
        };
        let v = compile(GL::VERTEX_SHADER, vs)?;
        let f = compile(GL::FRAGMENT_SHADER, fs)?;
        let p = gl.create_program().ok_or("create_program")?;
        gl.attach_shader(&p, &v);
        gl.attach_shader(&p, &f);
        for (name, i) in attrs {
            gl.bind_attrib_location(&p, *i, name);
        }
        gl.link_program(&p);
        if gl.get_program_parameter(&p, GL::LINK_STATUS).as_bool() != Some(true) {
            return Err(format!("link: {}", gl.get_program_info_log(&p).unwrap_or_default()));
        }
        Ok(Program { p, locs: RefCell::new(HashMap::new()) })
    }

    pub fn loc(&self, gl: &GL, name: &'static str) -> Option<WebGlUniformLocation> {
        self.locs.borrow_mut().entry(name).or_insert_with(|| gl.get_uniform_location(&self.p, name)).clone()
    }

    pub fn use_(&self, gl: &GL) {
        gl.use_program(Some(&self.p));
    }

    pub fn f1(&self, gl: &GL, n: &'static str, v: f32) {
        gl.uniform1f(self.loc(gl, n).as_ref(), v);
    }
    pub fn i1(&self, gl: &GL, n: &'static str, v: i32) {
        gl.uniform1i(self.loc(gl, n).as_ref(), v);
    }
    pub fn f2(&self, gl: &GL, n: &'static str, a: f32, b: f32) {
        gl.uniform2f(self.loc(gl, n).as_ref(), a, b);
    }
    pub fn v3(&self, gl: &GL, n: &'static str, v: Vec3) {
        gl.uniform3f(self.loc(gl, n).as_ref(), v.x, v.y, v.z);
    }
    pub fn m4(&self, gl: &GL, n: &'static str, m: &Mat4) {
        gl.uniform_matrix4fv_with_f32_array(self.loc(gl, n).as_ref(), false, &m.to_cols_array());
    }
    pub fn fv(&self, gl: &GL, n: &'static str, v: &[f32]) {
        gl.uniform1fv_with_f32_array(self.loc(gl, n).as_ref(), v);
    }
}

/// Vertex data, one array per attribute, plus triangle indices.
#[derive(Default, Clone)]
pub struct Geometry {
    pub pos: Vec<f32>,
    pub normal: Vec<f32>,
    pub extra: Vec<(u32, i32, Vec<f32>)>, // attribute index, components, data
    pub index: Vec<u32>,
}

pub struct Mesh {
    pub vao: WebGlVertexArrayObject,
    pub count: i32,
    pub indexed: bool,
    pub mode: u32,
    pub bufs: Vec<web_sys::WebGlBuffer>,
}

impl Mesh {
    pub fn new(gl: &GL, g: &Geometry, mode: u32) -> Mesh {
        let vao = gl.create_vertex_array().expect("vao");
        gl.bind_vertex_array(Some(&vao));
        let mut bufs = Vec::new();
        let mut attr = |idx: u32, comps: i32, data: &[f32], usage: u32| {
            let b = gl.create_buffer().expect("buffer");
            gl.bind_buffer(GL::ARRAY_BUFFER, Some(&b));
            gl.buffer_data_with_u8_array(GL::ARRAY_BUFFER, bytes(data), usage);
            gl.enable_vertex_attrib_array(idx);
            gl.vertex_attrib_pointer_with_i32(idx, comps, GL::FLOAT, false, 0, 0);
            bufs.push(b);
        };
        attr(ATTR_POS, 3, &g.pos, GL::STATIC_DRAW);
        if !g.normal.is_empty() {
            attr(ATTR_NORMAL, 3, &g.normal, GL::STATIC_DRAW);
        }
        for (idx, comps, data) in &g.extra {
            attr(*idx, *comps, data, GL::STATIC_DRAW);
        }
        let indexed = !g.index.is_empty();
        if indexed {
            let b = gl.create_buffer().expect("buffer");
            gl.bind_buffer(GL::ELEMENT_ARRAY_BUFFER, Some(&b));
            gl.buffer_data_with_u8_array(GL::ELEMENT_ARRAY_BUFFER, bytes(&g.index), GL::STATIC_DRAW);
            bufs.push(b);
        }
        gl.bind_vertex_array(None);
        let count = if indexed { g.index.len() } else { g.pos.len() / 3 } as i32;
        Mesh { vao, count, indexed, mode, bufs }
    }

    pub fn draw(&self, gl: &GL) {
        gl.bind_vertex_array(Some(&self.vao));
        if self.indexed {
            gl.draw_elements_with_i32(self.mode, self.count, GL::UNSIGNED_INT, 0);
        } else {
            gl.draw_arrays(self.mode, 0, self.count);
        }
    }
}

/// A vertex buffer refilled every frame (particles, trails), with interleaved float attributes.
pub struct Dynamic {
    pub vao: WebGlVertexArrayObject,
    pub buf: web_sys::WebGlBuffer,
    pub stride: i32,
}

impl Dynamic {
    /// `layout`: (attribute index, components) in order.
    pub fn new(gl: &GL, layout: &[(u32, i32)]) -> Dynamic {
        let vao = gl.create_vertex_array().expect("vao");
        gl.bind_vertex_array(Some(&vao));
        let buf = gl.create_buffer().expect("buffer");
        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&buf));
        let stride: i32 = layout.iter().map(|l| l.1).sum::<i32>() * 4;
        let mut off = 0;
        for &(idx, comps) in layout {
            gl.enable_vertex_attrib_array(idx);
            gl.vertex_attrib_pointer_with_i32(idx, comps, GL::FLOAT, false, stride, off);
            off += comps * 4;
        }
        gl.bind_vertex_array(None);
        Dynamic { vao, buf, stride }
    }

    pub fn draw(&self, gl: &GL, data: &[f32], mode: u32) {
        if data.is_empty() {
            return;
        }
        gl.bind_vertex_array(Some(&self.vao));
        gl.bind_buffer(GL::ARRAY_BUFFER, Some(&self.buf));
        gl.buffer_data_with_u8_array(GL::ARRAY_BUFFER, bytes(data), GL::STREAM_DRAW);
        gl.draw_arrays(mode, 0, (data.len() * 4) as i32 / self.stride);
    }
}

/// A render target: a colour texture (and a depth buffer when asked), or a multisampled one resolved into it.
pub struct Target {
    pub fb: web_sys::WebGlFramebuffer,
    pub tex: WebGlTexture,
    pub w: i32,
    pub h: i32,
    ms: Option<(web_sys::WebGlFramebuffer, web_sys::WebGlRenderbuffer, web_sys::WebGlRenderbuffer)>,
    depth: Option<web_sys::WebGlRenderbuffer>,
}

pub fn texture(gl: &GL, w: i32, h: i32, internal: u32, format: u32, ty: u32, filter: u32) -> WebGlTexture {
    let t = gl.create_texture().expect("texture");
    gl.bind_texture(GL::TEXTURE_2D, Some(&t));
    gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
        GL::TEXTURE_2D, 0, internal as i32, w, h, 0, format, ty, None,
    )
    .ok();
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MIN_FILTER, filter as i32);
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, filter as i32);
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_S, GL::CLAMP_TO_EDGE as i32);
    gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_T, GL::CLAMP_TO_EDGE as i32);
    t
}

impl Target {
    /// `hdr`: half-float colour (needs EXT_color_buffer_float), else 8-bit. `samples` > 0: multisampled with depth.
    pub fn new(gl: &GL, w: i32, h: i32, hdr: bool, depth: bool, samples: i32) -> Target {
        let (internal, ty) = if hdr { (GL::RGBA16F, GL::HALF_FLOAT) } else { (GL::RGBA8, GL::UNSIGNED_BYTE) };
        let tex = texture(gl, w, h, internal, GL::RGBA, ty, GL::LINEAR);
        let fb = gl.create_framebuffer().expect("framebuffer");
        gl.bind_framebuffer(GL::FRAMEBUFFER, Some(&fb));
        gl.framebuffer_texture_2d(GL::FRAMEBUFFER, GL::COLOR_ATTACHMENT0, GL::TEXTURE_2D, Some(&tex), 0);
        let mut ms = None;
        let mut dep = None;
        if depth && samples > 0 {
            let mfb = gl.create_framebuffer().expect("framebuffer");
            gl.bind_framebuffer(GL::FRAMEBUFFER, Some(&mfb));
            let c = gl.create_renderbuffer().expect("renderbuffer");
            gl.bind_renderbuffer(GL::RENDERBUFFER, Some(&c));
            gl.renderbuffer_storage_multisample(GL::RENDERBUFFER, samples, internal, w, h);
            gl.framebuffer_renderbuffer(GL::FRAMEBUFFER, GL::COLOR_ATTACHMENT0, GL::RENDERBUFFER, Some(&c));
            let d = gl.create_renderbuffer().expect("renderbuffer");
            gl.bind_renderbuffer(GL::RENDERBUFFER, Some(&d));
            gl.renderbuffer_storage_multisample(GL::RENDERBUFFER, samples, GL::DEPTH_COMPONENT24, w, h);
            gl.framebuffer_renderbuffer(GL::FRAMEBUFFER, GL::DEPTH_ATTACHMENT, GL::RENDERBUFFER, Some(&d));
            ms = Some((mfb, c, d));
        } else if depth {
            let d = gl.create_renderbuffer().expect("renderbuffer");
            gl.bind_renderbuffer(GL::RENDERBUFFER, Some(&d));
            gl.renderbuffer_storage(GL::RENDERBUFFER, GL::DEPTH_COMPONENT24, w, h);
            gl.framebuffer_renderbuffer(GL::FRAMEBUFFER, GL::DEPTH_ATTACHMENT, GL::RENDERBUFFER, Some(&d));
            dep = Some(d);
        }
        gl.bind_framebuffer(GL::FRAMEBUFFER, None);
        Target { fb, tex, w, h, ms, depth: dep }
    }

    pub fn ok(&self, gl: &GL) -> bool {
        let fb = self.ms.as_ref().map(|m| &m.0).unwrap_or(&self.fb);
        gl.bind_framebuffer(GL::FRAMEBUFFER, Some(fb));
        let ok = gl.check_framebuffer_status(GL::FRAMEBUFFER) == GL::FRAMEBUFFER_COMPLETE;
        gl.bind_framebuffer(GL::FRAMEBUFFER, None);
        ok
    }

    /// Bind for drawing (the multisampled buffer when there is one).
    pub fn bind(&self, gl: &GL) {
        let fb = self.ms.as_ref().map(|m| &m.0).unwrap_or(&self.fb);
        gl.bind_framebuffer(GL::FRAMEBUFFER, Some(fb));
        gl.viewport(0, 0, self.w, self.h);
    }

    /// Resolve the multisampled buffer into the texture.
    pub fn resolve(&self, gl: &GL) {
        if let Some((mfb, _, _)) = &self.ms {
            gl.bind_framebuffer(GL::READ_FRAMEBUFFER, Some(mfb));
            gl.bind_framebuffer(GL::DRAW_FRAMEBUFFER, Some(&self.fb));
            gl.blit_framebuffer(0, 0, self.w, self.h, 0, 0, self.w, self.h, GL::COLOR_BUFFER_BIT, GL::NEAREST);
            gl.bind_framebuffer(GL::READ_FRAMEBUFFER, None);
            gl.bind_framebuffer(GL::DRAW_FRAMEBUFFER, None);
        }
    }

    pub fn delete(&self, gl: &GL) {
        gl.delete_framebuffer(Some(&self.fb));
        gl.delete_texture(Some(&self.tex));
        if let Some((f, c, d)) = &self.ms {
            gl.delete_framebuffer(Some(f));
            gl.delete_renderbuffer(Some(c));
            gl.delete_renderbuffer(Some(d));
        }
        if let Some(d) = &self.depth {
            gl.delete_renderbuffer(Some(d));
        }
    }
}

pub fn context(canvas: &web_sys::HtmlCanvasElement) -> Result<GL, String> {
    let opts = js_sys::Object::new();
    let set = |k: &str, v: wasm_bindgen::JsValue| {
        js_sys::Reflect::set(&opts, &k.into(), &v).ok();
    };
    set("antialias", false.into());
    set("alpha", false.into());
    set("depth", true.into());
    set("powerPreference", "high-performance".into());
    set("premultipliedAlpha", true.into());
    let ctx = canvas
        .get_context_with_context_options("webgl2", &opts)
        .map_err(|_| "WebGL2 failed".to_string())?
        .ok_or_else(|| "This browser has no WebGL2.".to_string())?;
    ctx.dyn_into::<GL>().map_err(|_| "not a WebGL2 context".to_string())
}
