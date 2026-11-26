//! The fixed scenery: the ground, the laser fence, the ruins and their broken glass, and the pillars and power-up
//! spots (drawn each frame from the arena the server sends).

use glam::{Mat4, Vec3};
use lasertag_shared::protocol::Spot;
use lasertag_shared::solids::{Pillar, WallBox, Window};
use lasertag_shared::{Ground, PALETTE};
use web_sys::WebGl2RenderingContext as GL;

use crate::gl::{Geometry, Mesh, ATTR_A2, ATTR_A3, ATTR_A4};
use crate::render::{hex, hexu, Blend, Camera, Frame, Mat, Renderer, Shape};

pub struct World {
    terrain: Mesh,
    fence: Mesh,
    ruins: Option<Mesh>,
    glass: Option<Mesh>,
    pub time: f32,
}

pub fn power_color(kind: &str) -> &'static str {
    match kind {
        "rapid" => "#ffe23d",
        "over" => "#ff4d6d",
        "speed" => "#7dff3d",
        _ => "#b48cff",
    }
}

impl World {
    pub fn new(r: &Renderer, g: &Ground) -> World {
        let gl = &r.gl;
        let n = g.n;
        // the ground
        let mut geo = Geometry::default();
        let mut wet = Vec::with_capacity(n * n);
        for j in 0..n {
            for i in 0..n {
                let k = j * n + i;
                geo.pos.extend_from_slice(&[(i as f64 * g.cell) as f32, g.h[k] as f32, (j as f64 * g.cell) as f32]);
                wet.push(g.wet[k] as f32);
            }
        }
        // smooth normals from the neighbours (as computeVertexNormals gives on a grid)
        for j in 0..n {
            for i in 0..n {
                let hgt = |a: usize, b: usize| g.h[b.min(n - 1) * n + a.min(n - 1)] as f32;
                let (il, ir) = (i.saturating_sub(1), (i + 1).min(n - 1));
                let (jd, ju) = (j.saturating_sub(1), (j + 1).min(n - 1));
                let dx = (hgt(ir, j) - hgt(il, j)) / ((ir - il) as f32 * g.cell as f32);
                let dz = (hgt(i, ju) - hgt(i, jd)) / ((ju - jd) as f32 * g.cell as f32);
                let nn = Vec3::new(-dx, 1.0, -dz).normalize();
                geo.normal.extend_from_slice(&[nn.x, nn.y, nn.z]);
            }
        }
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                let a = (j * n + i) as u32;
                let (b, c) = (a + 1, a + n as u32);
                let d = c + 1;
                geo.index.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        geo.extra.push((ATTR_A2, 1, wet));
        let terrain = Mesh::new(gl, &geo, GL::TRIANGLES);
        // the laser fence round the edge
        let s = 160;
        let size = g.size;
        let mut edge: Vec<(f64, f64)> = Vec::new();
        for k in 0..s {
            edge.push((k as f64 / s as f64 * size, 0.0));
        }
        for k in 0..s {
            edge.push((size, k as f64 / s as f64 * size));
        }
        for k in 0..s {
            edge.push((size - k as f64 / s as f64 * size, size));
        }
        for k in 0..=s {
            edge.push((0.0, size - k as f64 / s as f64 * size));
        }
        let mut fg = Geometry::default();
        let mut uv = Vec::new();
        for (i, &(x, z)) in edge.iter().enumerate() {
            let gh = g.at(x, z);
            fg.pos.extend_from_slice(&[x as f32, (gh - 2.0) as f32, z as f32, x as f32, (gh + 30.0) as f32, z as f32]);
            uv.extend_from_slice(&[i as f32, 0.0, i as f32, 1.0]);
            if i > 0 {
                let a = ((i - 1) * 2) as u32;
                fg.index.extend_from_slice(&[a, a + 1, a + 2, a + 1, a + 3, a + 2]);
            }
        }
        fg.extra.push((ATTR_A2, 2, uv));
        let fence = Mesh::new(gl, &fg, GL::TRIANGLES);
        World { terrain, fence, ruins: None, glass: None, time: 0.0 }
    }

    /// Build the ruined walls (one merged mesh, each face knowing where its edges are) and their broken glass.
    pub fn build_ruins(&mut self, r: &Renderer, boxes: &[WallBox], windows: &[Window]) {
        let gl = &r.gl;
        let mut g = Geometry::default();
        let (mut fuv, mut fsz, mut tint) = (Vec::new(), Vec::new(), Vec::new());
        let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
            ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
            ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
            ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ];
        for b in boxes {
            let &WallBox(cx, cy, cz, hx, hy, hz, a, t) = b;
            let (c, sn) = (a.cos() as f32, a.sin() as f32);
            let (cx, cy, cz) = (cx as f32, cy as f32, cz as f32);
            let h = [hx as f32, hy as f32, hz as f32];
            let col = hex(PALETTE[t as usize % PALETTE.len()]);
            let w = |l: [f32; 3]| [cx + c * l[0] + sn * l[2], cy + l[1], cz - sn * l[0] + c * l[2]];
            for (n, u, v) in faces {
                let su = 2.0 * (u[0].abs() * h[0] + u[1].abs() * h[1] + u[2].abs() * h[2]);
                let sv = 2.0 * (v[0].abs() * h[0] + v[1].abs() * h[1] + v[2].abs() * h[2]);
                let base = (g.pos.len() / 3) as u32;
                for (a1, b1) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    let l = [0, 1, 2].map(|k| n[k] * h[k] + u[k] * a1 * su / 2.0 + v[k] * b1 * sv / 2.0);
                    g.pos.extend_from_slice(&w(l));
                    g.normal.extend_from_slice(&[c * n[0] + sn * n[2], n[1], -sn * n[0] + c * n[2]]);
                    fuv.extend_from_slice(&[(a1 + 1.0) / 2.0 * su, (b1 + 1.0) / 2.0 * sv]);
                    fsz.extend_from_slice(&[su, sv]);
                    tint.extend_from_slice(&[col.x, col.y, col.z]);
                }
                g.index.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
        g.extra.push((ATTR_A2, 2, fuv));
        g.extra.push((ATTR_A3, 2, fsz));
        g.extra.push((ATTR_A4, 3, tint));
        self.ruins = if boxes.is_empty() { None } else { Some(Mesh::new(gl, &g, GL::TRIANGLES)) };
        // broken glass: jagged shards hanging into each window from its edges
        let mut gg = Geometry::default();
        for (wi, &Window(cx, cy, cz, hw, hh, a)) in windows.iter().enumerate() {
            let mut seed: i64 = wi as i64 * 9301 + 49297;
            let mut rnd = || {
                seed = (seed * 233280 + 49297) % 233280;
                seed as f64 / 233280.0
            };
            let ax = [a.cos(), 0.0, -a.sin()];
            let pt = |u: f64, v: f64| [(cx + ax[0] * u) as f32, (cy + v) as f32, (cz + ax[2] * u) as f32];
            let n = 3 + (rnd() * 4.0).floor() as i32;
            for _ in 0..n {
                let side = (rnd() * 4.0).floor() as i32;
                let f0 = rnd() * 0.7;
                let f1 = f0 + 0.15 + rnd() * 0.3;
                let reach = 0.25 + rnd() * 0.5;
                let tipf = f0 + (f1 - f0) * rnd();
                let (pa, pb, pc) = match side {
                    0 => (pt(-hw + 2.0 * hw * f0, hh), pt(-hw + 2.0 * hw * f1.min(1.0), hh), pt(-hw + 2.0 * hw * tipf, hh - 2.0 * hh * reach)),
                    1 => (pt(-hw + 2.0 * hw * f0, -hh), pt(-hw + 2.0 * hw * f1.min(1.0), -hh), pt(-hw + 2.0 * hw * tipf, -hh + 2.0 * hh * reach * 0.6)),
                    2 => (pt(-hw, -hh + 2.0 * hh * f0), pt(-hw, -hh + 2.0 * hh * f1.min(1.0)), pt(-hw + 2.0 * hw * reach * 0.7, -hh + 2.0 * hh * tipf)),
                    _ => (pt(hw, -hh + 2.0 * hh * f0), pt(hw, -hh + 2.0 * hh * f1.min(1.0)), pt(hw - 2.0 * hw * reach * 0.7, -hh + 2.0 * hh * tipf)),
                };
                for p in [pa, pb, pc] {
                    gg.pos.extend_from_slice(&p);
                }
            }
        }
        self.glass = if gg.pos.is_empty() { None } else { Some(Mesh::new(gl, &gg, GL::TRIANGLES)) };
    }

    /// The opaque scenery.
    pub fn draw(&self, r: &Renderer, cam: &Camera) {
        let gl = &r.gl;
        let vp = cam.view_proj();
        gl.disable(GL::BLEND);
        gl.depth_mask(true);
        gl.enable(GL::CULL_FACE);
        let p = &r.terrain;
        p.use_(gl);
        p.m4(gl, "uViewProj", &vp);
        p.f1(gl, "uTime", self.time);
        p.v3(gl, "uCam", cam.pos);
        self.terrain.draw(gl);
        if let Some(m) = &self.ruins {
            let p = &r.ruins;
            p.use_(gl);
            p.m4(gl, "uViewProj", &vp);
            p.v3(gl, "uCam", cam.pos);
            m.draw(gl);
        }
    }

    /// The see-through scenery: the fence and the glass.
    pub fn draw_late(&self, r: &Renderer, cam: &Camera) {
        let gl = &r.gl;
        let vp = cam.view_proj();
        gl.enable(GL::BLEND);
        gl.blend_func(GL::SRC_ALPHA, GL::ONE);
        gl.depth_mask(false);
        gl.disable(GL::CULL_FACE);
        let p = &r.fence;
        p.use_(gl);
        p.m4(gl, "uViewProj", &vp);
        p.f1(gl, "uTime", self.time);
        self.fence.draw(gl);
        if let Some(m) = &self.glass {
            let p = &r.basic;
            p.use_(gl);
            p.m4(gl, "uModel", &Mat4::IDENTITY);
            p.m4(gl, "uViewProj", &vp);
            p.m4(gl, "uView", &cam.view);
            p.v3(gl, "uColor", hexu(0x7fe9ff) * 0.7);
            p.f1(gl, "uOpacity", 0.38);
            p.f1(gl, "uFog", 1.0);
            p.v3(gl, "uFogColor", r.env.fog_color);
            p.f1(gl, "uFogDensity", r.env.fog_density);
            m.draw(gl);
        }
        gl.depth_mask(true);
    }
}

/// Pillars: a dark column with two glowing rings and a glowing cap.
pub fn draw_pillars(f: &mut Frame, pillars: &[Pillar]) {
    let body = Mat::std(hexu(0x141026), Vec3::ZERO, 0.4, 0.55);
    for (i, p) in pillars.iter().enumerate() {
        let (x, z, r, b, top) = (p.0 as f32, p.1 as f32, p.2 as f32, p.3 as f32, p.4 as f32);
        let h = top - b;
        let c = hex(PALETTE[i % PALETTE.len()]);
        f.push(Shape::Cyl20, Mat4::from_translation(Vec3::new(x, b + h / 2.0, z)) * Mat4::from_scale(Vec3::new(r, h, r)), body);
        for y in [top - 0.6, b + (top - b) * 0.55] {
            f.push(
                Shape::CylOpen20,
                Mat4::from_translation(Vec3::new(x, y, z)) * Mat4::from_scale(Vec3::new(r + 0.08, 0.5, r + 0.08)),
                Mat::glow(c * 1.8),
            );
        }
        f.push(
            Shape::Circle20,
            Mat4::from_translation(Vec3::new(x, top + 0.02, z)) * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2) * Mat4::from_scale(Vec3::splat(r * 0.75)),
            Mat::glow(c * 0.8),
        );
    }
}

/// Power-up spots: a spinning gem, a beam of light up into the sky, and a pad.
pub fn draw_spots(f: &mut Frame, spots: &[Spot], t: f32) {
    for s in spots {
        let c = hex(power_color(&s.kind));
        let base = Mat4::from_translation(Vec3::new(s.x as f32, s.y as f32, s.z as f32));
        if s.up {
            let gy = 3.2 + (t * 2.5 + s.i as f32).sin() * 0.5;
            f.push(Shape::Octa, base * Mat4::from_translation(Vec3::new(0.0, gy, 0.0)) * Mat4::from_rotation_y(t * 2.0) * Mat4::from_scale(Vec3::splat(1.6)), Mat::glow(c * 2.5));
            f.push(
                Shape::BeamCyl,
                base * Mat4::from_scale(Vec3::new(0.35, 120.0, 0.35)),
                Mat::Basic { color: c, opacity: 0.25, blend: Blend::Add, fog: true, double: true, depth_write: false },
            );
        }
        f.push(
            Shape::SpotRing,
            base * Mat4::from_translation(Vec3::new(0.0, 0.25, 0.0)) * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2),
            Mat::Basic { color: c * 1.6, opacity: 1.0, blend: Blend::Opaque, fog: true, double: true, depth_write: true },
        );
    }
}
