//! The renderer: shared shapes, a frame's list of things to draw, and the passes (scene into a float buffer, bloom,
//! tone-mapped output).

use glam::{Mat4, Vec3, Vec4};
use web_sys::{WebGl2RenderingContext as GL, WebGlTexture};

use crate::geom;
use crate::gl::{Dynamic, Geometry, Mesh, Program, Target, ATTR_A2, ATTR_A3, ATTR_A4, ATTR_NORMAL, ATTR_POS};
use crate::shaders::*;

/// Colour from "#rrggbb", converted from sRGB to linear (as three.js does for colours given in hex).
pub fn hex(s: &str) -> Vec3 {
    let s = s.trim_start_matches('#');
    let v = u32::from_str_radix(s, 16).unwrap_or(0xffffff);
    hexu(v)
}

pub fn hexu(v: u32) -> Vec3 {
    let f = |c: u32| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    Vec3::new(f((v >> 16) & 255), f((v >> 8) & 255), f(v & 255))
}

/// The shared shapes, by name.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Shape {
    Cyl20,      // r 1, h 1, closed
    CylOpen20,  // r 1, h 1, open (rings round pillars)
    Circle20,   // r 1
    Octa,       // r 1
    BeamCyl,    // r 1, h 1, open, 8 sides, base at the origin
    SpotRing,   // ring 2.6..3.4
    AuraRing,   // ring 1.8..2.3
    Sphere16,   // r 1, 16 x 12
    Sphere20,   // r 1, 20 x 14
    Sphere10,   // r 1, 10 x 8
    Capsule,    // r 1.15, length 2.4
    Torus,      // 1.2, tube 0.11
    Cube,       // 1 x 1 x 1
    Cone10,     // r 1, h 1
    Cone8,      // r 1, h 1
    Cyl10,      // r 1, h 1, closed, 10 sides
    Cyl18,      // r 1, h 1, closed, 18 sides
    Muzzle,     // the view gun's muzzle: r 0.06 / 0.08, h 0.12
    Quad,       // -0.5..0.5 square in xy
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Blend {
    Opaque,
    Alpha,
    Add,
}

#[derive(Clone, Copy, Debug)]
pub enum Mat {
    /// Lit: colour, emissive, metalness, roughness.
    Std { color: Vec3, emissive: Vec3, metal: f32, rough: f32 },
    /// Flat colour.
    Basic { color: Vec3, opacity: f32, blend: Blend, fog: bool, double: bool, depth_write: bool },
}

impl Mat {
    pub fn std(color: Vec3, emissive: Vec3, metal: f32, rough: f32) -> Mat {
        Mat::Std { color, emissive, metal, rough }
    }
    pub fn glow(color: Vec3) -> Mat {
        Mat::Basic { color, opacity: 1.0, blend: Blend::Opaque, fog: true, double: false, depth_write: true }
    }
    pub fn add(color: Vec3, opacity: f32) -> Mat {
        Mat::Basic { color, opacity, blend: Blend::Add, fog: true, double: false, depth_write: false }
    }
    fn transparent(&self) -> bool {
        matches!(self, Mat::Basic { blend: Blend::Alpha | Blend::Add, .. })
    }
}

#[derive(Clone, Copy)]
pub struct Item {
    pub shape: Shape,
    pub model: Mat4,
    pub mat: Mat,
}

pub struct Sprite {
    pub tex: WebGlTexture,
    pub center: Vec3,
    pub scale: (f32, f32),
}

/// What to draw this frame, beyond the fixed world.
#[derive(Default)]
pub struct Frame {
    pub items: Vec<Item>,
    /// Items drawn last, over everything, in camera space (the view gun).
    pub overlay: Vec<Item>,
    /// Sparks: x, y, z, r, g, b, a per point.
    pub sparks: Vec<f32>,
    /// Trails: each a line strip of x, y, z, r, g, b, a.
    pub trails: Vec<Vec<f32>>,
    pub sprites: Vec<Sprite>,
}

impl Frame {
    pub fn push(&mut self, shape: Shape, model: Mat4, mat: Mat) {
        self.items.push(Item { shape, model, mat });
    }
}

pub struct Camera {
    pub pos: Vec3,
    pub view: Mat4,
    pub proj: Mat4,
    pub fov: f32,
    pub aspect: f32,
}

impl Camera {
    pub fn world(&self) -> Mat4 {
        self.view.inverse()
    }
    pub fn view_proj(&self) -> Mat4 {
        self.proj * self.view
    }
    /// Camera looking along yaw (turned from -z towards +x) and pitch (up), as three.js's YXZ Euler order.
    pub fn set_euler(&mut self, pos: Vec3, pitch: f32, yaw: f32) {
        self.pos = pos;
        let world = Mat4::from_translation(pos) * Mat4::from_rotation_y(-yaw) * Mat4::from_rotation_x(pitch);
        self.view = world.inverse();
    }
    pub fn look_at(&mut self, pos: Vec3, target: Vec3) {
        self.pos = pos;
        if (target - pos).length_squared() < 1e-8 {
            return;
        }
        self.view = Mat4::look_at_rh(pos, target, Vec3::Y);
    }
    pub fn set_proj(&mut self) {
        self.proj = Mat4::perspective_rh_gl(self.fov.to_radians(), self.aspect, 0.3, 5000.0);
    }
    /// Where a world point lands on screen: (x, y) in 0..1 from the top left, and whether it is in front.
    pub fn project(&self, p: Vec3) -> (f32, f32, bool) {
        let c = self.view_proj() * Vec4::new(p.x, p.y, p.z, 1.0);
        if c.w <= 0.0 {
            return (0.0, 0.0, false);
        }
        let n = c / c.w;
        ((n.x + 1.0) / 2.0, (1.0 - n.y) / 2.0, n.z < 1.0)
    }
}

/// The lights and fog of the scene, as uniforms.
pub struct Env {
    pub fog_color: Vec3,
    pub fog_density: f32,
    pub sky: Vec3,
    pub ground: Vec3,
    pub moon: Vec3,
    pub moon_dir: Vec3,
}

pub struct Renderer {
    pub gl: GL,
    pub std: Program,
    pub basic: Program,
    pub points: Program,
    pub line: Program,
    pub sprite: Program,
    pub sky: Program,
    pub terrain: Program,
    pub fence: Program,
    pub ruins: Program,
    bright: Program,
    blur: Program,
    out: Program,
    shapes: Vec<(Shape, Mesh)>,
    pub sky_mesh: Mesh,
    pub stars: Dynamic,
    pub star_data: Vec<f32>,
    sparks: Dynamic,
    lines: Dynamic,
    quad: Mesh,
    pub env: Env,
    hdr: bool,
    samples: i32,
    scene: Option<Target>,
    bright_t: Option<Target>,
    mips: Vec<(Target, Target)>,
    pub width: i32,
    pub height: i32,
    pub glow: bool,
}

fn quad_geom() -> Geometry {
    Geometry { pos: vec![-1.0, -1.0, 0.0, 3.0, -1.0, 0.0, -1.0, 3.0, 0.0], ..Default::default() }
}

impl Renderer {
    pub fn new(gl: GL, antialias: bool, glow: bool) -> Result<Renderer, String> {
        let p = |vs: &str, fs: &str, attrs: &[(&str, u32)]| Program::new(&gl, &format!("{}{}", HEAD, vs), &format!("{}{}", HEAD, fs), attrs);
        let pn = [("aPos", ATTR_POS), ("aNormal", ATTR_NORMAL)];
        let std = p(STD_VS, STD_FS, &pn)?;
        let basic = p(BASIC_VS, BASIC_FS, &[("aPos", ATTR_POS)])?;
        let points = p(POINTS_VS, POINTS_FS, &[("aPos", ATTR_POS), ("aColor", ATTR_A2)])?;
        let line = p(LINE_VS, LINE_FS, &[("aPos", ATTR_POS), ("aColor", ATTR_A2)])?;
        let sprite = p(SPRITE_VS, SPRITE_FS, &[("aPos", ATTR_POS)])?;
        let sky = p(SKY_VS, SKY_FS, &[("aPos", ATTR_POS)])?;
        let terrain = p(TERRAIN_VS, TERRAIN_FS, &[("aPos", ATTR_POS), ("aNormal", ATTR_NORMAL), ("aWet", ATTR_A2)])?;
        let fence = p(FENCE_VS, FENCE_FS, &[("aPos", ATTR_POS), ("aUv", ATTR_A2)])?;
        let ruins = p(
            RUINS_VS,
            RUINS_FS,
            &[("aPos", ATTR_POS), ("aNormal", ATTR_NORMAL), ("aFuv", ATTR_A2), ("aFsz", ATTR_A3), ("aTint", ATTR_A4)],
        )?;
        let bright = p(QUAD_VS, BRIGHT_FS, &[("aPos", ATTR_POS)])?;
        let blur = p(QUAD_VS, BLUR_FS, &[("aPos", ATTR_POS)])?;
        let out = p(QUAD_VS, OUT_FS, &[("aPos", ATTR_POS)])?;
        let tri = GL::TRIANGLES;
        let mk = |g: Geometry| Mesh::new(&gl, &g, tri);
        let shapes = vec![
            (Shape::Cyl20, mk(geom::cylinder(1.0, 1.0, 1.0, 20, false))),
            (Shape::CylOpen20, mk(geom::cylinder(1.0, 1.0, 1.0, 20, true))),
            (Shape::Circle20, mk(geom::circle(1.0, 20))),
            (Shape::Octa, mk(geom::octahedron(1.0))),
            (Shape::BeamCyl, mk(geom::translate(geom::cylinder(1.0, 1.0, 1.0, 8, true), 0.0, 0.5, 0.0))),
            (Shape::SpotRing, mk(geom::ring(2.6, 3.4, 32))),
            (Shape::AuraRing, mk(geom::ring(1.8, 2.3, 28))),
            (Shape::Sphere16, mk(geom::sphere(1.0, 16, 12))),
            (Shape::Sphere20, mk(geom::sphere(1.0, 20, 14))),
            (Shape::Sphere10, mk(geom::sphere(1.0, 10, 8))),
            (Shape::Capsule, mk(geom::capsule(1.15, 2.4, 4, 12))),
            (Shape::Torus, mk(geom::torus(1.2, 0.11, 6, 24))),
            (Shape::Cube, mk(geom::cuboid(1.0, 1.0, 1.0))),
            (Shape::Cone10, mk(geom::cylinder(0.0, 1.0, 1.0, 10, false))),
            (Shape::Cone8, mk(geom::cylinder(0.0, 1.0, 1.0, 8, false))),
            (Shape::Cyl10, mk(geom::cylinder(1.0, 1.0, 1.0, 10, false))),
            (Shape::Cyl18, mk(geom::cylinder(1.0, 1.0, 1.0, 18, false))),
            (Shape::Muzzle, mk(geom::cylinder(0.06, 0.08, 0.12, 10, false))),
            (
                Shape::Quad,
                mk(Geometry { pos: vec![-0.5, -0.5, 0.0, 0.5, -0.5, 0.0, 0.5, 0.5, 0.0, -0.5, 0.5, 0.0], index: vec![0, 1, 2, 0, 2, 3], ..Default::default() }),
            ),
        ];
        let sky_mesh = mk(geom::sphere(3000.0, 32, 16));
        let stars = Dynamic::new(&gl, &[(ATTR_POS, 3), (ATTR_A2, 4)]);
        let sparks = Dynamic::new(&gl, &[(ATTR_POS, 3), (ATTR_A2, 4)]);
        let lines = Dynamic::new(&gl, &[(ATTR_POS, 3), (ATTR_A2, 4)]);
        let quad = mk(quad_geom());
        let hdr = gl.get_extension("EXT_color_buffer_float").ok().flatten().is_some();
        gl.get_extension("OES_texture_float_linear").ok();
        let max_samples = gl.get_parameter(GL::MAX_SAMPLES).ok().and_then(|v| v.as_f64()).unwrap_or(0.0) as i32;
        let samples = if antialias { max_samples.min(4) } else { 0 };
        let moon_dir = Vec3::new(-400.0, 800.0, 300.0).normalize();
        let env = Env {
            fog_color: hexu(0x0d0620),
            fog_density: 0.0011,
            sky: hexu(0x6a5cff) * 1.1,
            ground: hexu(0x120820) * 1.1,
            moon: hexu(0xbfd8ff) * 1.3,
            moon_dir,
        };
        // stars: 1400 points on the upper part of a big sphere round the camera
        let mut star_data = Vec::with_capacity(1400 * 7);
        for _ in 0..1400 {
            let u = js_sys::Math::random() * 2.0 - 1.0;
            let a = js_sys::Math::random() * std::f64::consts::PI * 2.0;
            let y = u.abs() * 0.95 + 0.05;
            let r = (1.0 - y * y).sqrt();
            star_data.extend_from_slice(&[(a.cos() * r * 2800.0) as f32, (y * 2800.0) as f32, (a.sin() * r * 2800.0) as f32, 1.0, 1.0, 1.0, 0.9]);
        }
        Ok(Renderer {
            gl,
            std,
            basic,
            points,
            line,
            sprite,
            sky,
            terrain,
            fence,
            ruins,
            bright,
            blur,
            out,
            shapes,
            sky_mesh,
            stars,
            star_data,
            sparks,
            lines,
            quad,
            env,
            hdr,
            samples,
            scene: None,
            bright_t: None,
            mips: Vec::new(),
            width: 0,
            height: 0,
            glow,
        })
    }

    pub fn mesh(&self, s: Shape) -> &Mesh {
        &self.shapes.iter().find(|(k, _)| *k == s).expect("shape").1
    }

    /// Size the buffers to the canvas (in device pixels).
    pub fn resize(&mut self, w: i32, h: i32) {
        let (w, h) = (w.max(1), h.max(1));
        if w == self.width && h == self.height && self.scene.is_some() {
            return;
        }
        let gl = &self.gl;
        if let Some(t) = self.scene.take() {
            t.delete(gl);
        }
        if let Some(t) = self.bright_t.take() {
            t.delete(gl);
        }
        for (a, b) in self.mips.drain(..) {
            a.delete(gl);
            b.delete(gl);
        }
        let mut scene = Target::new(gl, w, h, self.hdr, true, self.samples);
        if !scene.ok(gl) {
            scene.delete(gl);
            self.samples = 0;
            scene = Target::new(gl, w, h, self.hdr, true, 0);
            if !scene.ok(gl) {
                scene.delete(gl);
                self.hdr = false;
                scene = Target::new(gl, w, h, false, true, 0);
            }
        }
        self.scene = Some(scene);
        let (mut rx, mut ry) = (((w as f32) / 2.0).round() as i32, ((h as f32) / 2.0).round() as i32);
        self.bright_t = Some(Target::new(gl, rx.max(1), ry.max(1), self.hdr, false, 0));
        for _ in 0..5 {
            self.mips.push((Target::new(gl, rx.max(1), ry.max(1), self.hdr, false, 0), Target::new(gl, rx.max(1), ry.max(1), self.hdr, false, 0)));
            rx = ((rx as f32) / 2.0).round() as i32;
            ry = ((ry as f32) / 2.0).round() as i32;
        }
        self.width = w;
        self.height = h;
    }

    fn set_blend(&self, b: Blend) {
        let gl = &self.gl;
        match b {
            Blend::Opaque => gl.disable(GL::BLEND),
            Blend::Alpha => {
                gl.enable(GL::BLEND);
                gl.blend_func_separate(GL::SRC_ALPHA, GL::ONE_MINUS_SRC_ALPHA, GL::ONE, GL::ONE_MINUS_SRC_ALPHA);
            }
            Blend::Add => {
                gl.enable(GL::BLEND);
                gl.blend_func(GL::SRC_ALPHA, GL::ONE);
            }
        }
    }

    fn draw_item(&self, it: &Item, cam: &Camera, vp: &Mat4) {
        let gl = &self.gl;
        match it.mat {
            Mat::Std { color, emissive, metal, rough } => {
                let p = &self.std;
                p.use_(gl);
                p.m4(gl, "uModel", &it.model);
                p.m4(gl, "uViewProj", vp);
                p.m4(gl, "uView", &cam.view);
                p.v3(gl, "uColor", color);
                p.v3(gl, "uEmissive", emissive);
                p.f1(gl, "uMetal", metal);
                p.f1(gl, "uRough", rough);
                p.v3(gl, "uCam", cam.pos);
                p.v3(gl, "uSky", self.env.sky);
                p.v3(gl, "uGround", self.env.ground);
                p.v3(gl, "uMoon", self.env.moon);
                p.v3(gl, "uMoonDir", self.env.moon_dir);
                p.v3(gl, "uFogColor", self.env.fog_color);
                p.f1(gl, "uFogDensity", self.env.fog_density);
                self.set_blend(Blend::Opaque);
                gl.depth_mask(true);
                gl.enable(GL::CULL_FACE);
            }
            Mat::Basic { color, opacity, blend, fog, double, depth_write } => {
                let p = &self.basic;
                p.use_(gl);
                p.m4(gl, "uModel", &it.model);
                p.m4(gl, "uViewProj", vp);
                p.m4(gl, "uView", &cam.view);
                p.v3(gl, "uColor", color);
                p.f1(gl, "uOpacity", opacity);
                p.f1(gl, "uFog", if fog { 1.0 } else { 0.0 });
                p.v3(gl, "uFogColor", self.env.fog_color);
                p.f1(gl, "uFogDensity", self.env.fog_density);
                self.set_blend(blend);
                gl.depth_mask(depth_write);
                if double {
                    gl.disable(GL::CULL_FACE);
                } else {
                    gl.enable(GL::CULL_FACE);
                }
            }
        }
        self.mesh(it.shape).draw(gl);
    }

    /// Draw a frame: `world` draws the fixed scenery (opaque) and `world_late` the see-through scenery.
    pub fn render(
        &mut self,
        cam: &Camera,
        frame: &mut Frame,
        world: &dyn Fn(&Renderer, &Camera),
        world_late: &dyn Fn(&Renderer, &Camera),
        pixel_ratio: f32,
    ) {
        let gl = self.gl.clone();
        let Some(scene) = self.scene.as_ref() else { return };
        let vp = cam.view_proj();
        scene.bind(&gl);
        let fc = self.env.fog_color;
        gl.clear_color(fc.x, fc.y, fc.z, 1.0);
        gl.depth_mask(true);
        gl.clear(GL::COLOR_BUFFER_BIT | GL::DEPTH_BUFFER_BIT);
        gl.enable(GL::DEPTH_TEST);
        gl.depth_func(GL::LEQUAL);
        gl.front_face(GL::CCW);
        gl.cull_face(GL::BACK);
        // the sky and the stars first, writing no depth: everything else draws over them
        gl.depth_mask(false);
        gl.disable(GL::CULL_FACE);
        self.set_blend(Blend::Opaque);
        self.sky.use_(&gl);
        self.sky.m4(&gl, "uViewProj", &vp);
        self.sky.v3(&gl, "uCam", cam.pos);
        self.sky_mesh.draw(&gl);
        self.set_blend(Blend::Alpha);
        self.points.use_(&gl);
        let star_view = cam.view * Mat4::from_translation(cam.pos);
        self.points.m4(&gl, "uViewProj", &(cam.proj * star_view));
        self.points.m4(&gl, "uView", &star_view);
        self.points.f1(&gl, "uSize", 1.6 * pixel_ratio);
        self.points.f1(&gl, "uAtten", 0.0);
        self.points.f1(&gl, "uFog", 0.0);
        self.stars.draw(&gl, &self.star_data, GL::POINTS);
        // opaque things
        gl.depth_mask(true);
        self.set_blend(Blend::Opaque);
        world(self, cam);
        for it in frame.items.iter().filter(|i| !i.mat.transparent()) {
            self.draw_item(it, cam, &vp);
        }
        // see-through things, far to near
        world_late(self, cam);
        let mut tr: Vec<(f32, &Item)> = frame
            .items
            .iter()
            .filter(|i| i.mat.transparent())
            .map(|i| ((cam.view * i.model.w_axis).z, i))
            .collect();
        tr.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        for (_, it) in tr {
            self.draw_item(it, cam, &vp);
        }
        // trails and sparks
        gl.depth_mask(false);
        self.set_blend(Blend::Add);
        self.line.use_(&gl);
        self.line.m4(&gl, "uViewProj", &vp);
        for t in &frame.trails {
            self.lines.draw(&gl, t, GL::LINE_STRIP);
        }
        self.points.use_(&gl);
        self.points.m4(&gl, "uViewProj", &vp);
        self.points.m4(&gl, "uView", &cam.view);
        self.points.f1(&gl, "uSize", 0.7 * pixel_ratio);
        self.points.f1(&gl, "uScale", self.height as f32 / 2.0 / pixel_ratio);
        self.points.f1(&gl, "uAtten", 1.0);
        self.points.f1(&gl, "uFog", 1.0);
        self.points.v3(&gl, "uFogColor", self.env.fog_color);
        self.points.f1(&gl, "uFogDensity", self.env.fog_density);
        self.sparks.draw(&gl, &frame.sparks, GL::POINTS);
        // name tags
        self.set_blend(Blend::Alpha);
        gl.disable(GL::CULL_FACE);
        self.sprite.use_(&gl);
        self.sprite.m4(&gl, "uView", &cam.view);
        self.sprite.m4(&gl, "uProj", &cam.proj);
        self.sprite.i1(&gl, "uTex", 0);
        let mut sp: Vec<&Sprite> = frame.sprites.iter().collect();
        sp.sort_by(|a, b| {
            let da = (a.center - cam.pos).length_squared();
            let db = (b.center - cam.pos).length_squared();
            db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
        });
        gl.active_texture(GL::TEXTURE0);
        for s in sp {
            gl.bind_texture(GL::TEXTURE_2D, Some(&s.tex));
            self.sprite.v3(&gl, "uCenter", s.center);
            self.sprite.f2(&gl, "uScale", s.scale.0, s.scale.1);
            self.mesh(Shape::Quad).draw(&gl);
        }
        // the view gun, over everything
        gl.disable(GL::DEPTH_TEST);
        for it in &frame.overlay {
            self.draw_item(it, cam, &vp);
        }
        gl.enable(GL::DEPTH_TEST);
        gl.depth_mask(true);
        self.set_blend(Blend::Opaque);
        let scene = self.scene.as_ref().unwrap();
        scene.resolve(&gl);
        self.post();
    }

    fn post(&self) {
        let gl = &self.gl;
        let scene = self.scene.as_ref().unwrap();
        gl.disable(GL::DEPTH_TEST);
        gl.disable(GL::CULL_FACE);
        gl.disable(GL::BLEND);
        gl.active_texture(GL::TEXTURE0);
        let radius = 0.35f32;
        let factors: Vec<f32> = [1.0f32, 0.8, 0.6, 0.4, 0.2].iter().map(|f| f + (1.2 - f - f) * radius).collect();
        if self.glow {
            let bt = self.bright_t.as_ref().unwrap();
            bt.bind(gl);
            self.bright.use_(gl);
            self.bright.i1(gl, "uTex", 0);
            self.bright.f1(gl, "uThreshold", 0.85);
            gl.bind_texture(GL::TEXTURE_2D, Some(&scene.tex));
            self.quad.draw(gl);
            let mut input = &bt.tex;
            self.blur.use_(gl);
            self.blur.i1(gl, "uTex", 0);
            for (i, (h, v)) in self.mips.iter().enumerate() {
                let k = [3, 5, 7, 9, 11][i];
                let mut coef = [0f32; 11];
                for (j, c) in coef.iter_mut().enumerate().take(k) {
                    let jf = j as f32;
                    *c = 0.39894 * (-0.5 * jf * jf / (k * k) as f32).exp() / k as f32;
                }
                self.blur.i1(gl, "uRadius", k as i32);
                self.blur.fv(gl, "uCoef", &coef);
                self.blur.f2(gl, "uInvSize", 1.0 / h.w as f32, 1.0 / h.h as f32);
                h.bind(gl);
                self.blur.f2(gl, "uDir", 1.0, 0.0);
                gl.bind_texture(GL::TEXTURE_2D, Some(input));
                self.quad.draw(gl);
                v.bind(gl);
                self.blur.f2(gl, "uDir", 0.0, 1.0);
                gl.bind_texture(GL::TEXTURE_2D, Some(&h.tex));
                self.quad.draw(gl);
                input = &v.tex;
            }
        }
        gl.bind_framebuffer(GL::FRAMEBUFFER, None);
        gl.viewport(0, 0, self.width, self.height);
        self.out.use_(gl);
        let names = ["uB0", "uB1", "uB2", "uB3", "uB4"];
        self.out.i1(gl, "uScene", 0);
        gl.active_texture(GL::TEXTURE0);
        gl.bind_texture(GL::TEXTURE_2D, Some(&scene.tex));
        for (i, n) in names.iter().enumerate() {
            self.out.i1(gl, n, i as i32 + 1);
            gl.active_texture(GL::TEXTURE1 + i as u32);
            gl.bind_texture(GL::TEXTURE_2D, self.mips.get(i).map(|m| &m.1.tex));
        }
        let strength = 0.55f32;
        let sum: f32 = factors.iter().sum();
        // three.js blends the bloom on with its own alpha (strength × the factors), which a float buffer keeps above 1
        let alpha = if self.hdr { strength * sum } else { (strength * sum).min(1.0) };
        self.out.f1(gl, "uBloom", if self.glow { 1.0 } else { 0.0 });
        self.out.f1(gl, "uBloomStrength", strength);
        self.out.f1(gl, "uBloomAlpha", alpha);
        self.out.fv(gl, "uF", &factors);
        self.out.f1(gl, "uExposure", 1.05);
        self.quad.draw(gl);
        gl.active_texture(GL::TEXTURE0);
        gl.enable(GL::DEPTH_TEST);
    }
}
