//! Solid things standing on the ground: round pillars and the boxes ruined walls are made of.

use serde::{Deserialize, Serialize};

/// A round pillar: centre (x, z), radius, base and top heights. Sent as `[x, z, r, base, top]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pillar(pub f64, pub f64, pub f64, pub f64, pub f64);

impl Pillar {
    pub fn x(&self) -> f64 { self.0 }
    pub fn z(&self) -> f64 { self.1 }
    pub fn r(&self) -> f64 { self.2 }
    pub fn base(&self) -> f64 { self.3 }
    pub fn top(&self) -> f64 { self.4 }
}

/// A wall piece: a box turned `angle` about the vertical, `[cx, cy, cz, hx, hy, hz, angle, tint]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WallBox(pub f64, pub f64, pub f64, pub f64, pub f64, pub f64, pub f64, pub u32);

/// A window opening with broken glass, `[cx, cy, cz, half width, half height, angle]` (only for the picture).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Window(pub f64, pub f64, pub f64, pub f64, pub f64, pub f64);

/// A wall box ready for collision tests: the turn kept as cos and sin.
#[derive(Debug, Clone, Copy)]
pub struct Bx {
    pub cx: f64,
    pub cy: f64,
    pub cz: f64,
    pub hx: f64,
    pub hy: f64,
    pub hz: f64,
    pub c: f64,
    pub s: f64,
    pub top: f64,
    pub bot: f64,
    pub r2: f64,
    pub tint: u32,
}

pub type Boxes = Vec<Bx>;

pub fn boxes_from(list: &[WallBox]) -> Boxes {
    list.iter()
        .map(|&WallBox(cx, cy, cz, hx, hy, hz, a, tint)| Bx {
            cx,
            cy,
            cz,
            hx,
            hy,
            hz,
            c: crate::dmath::cos(a),
            s: crate::dmath::sin(a),
            top: cy + hy,
            bot: cy - hy,
            r2: (hx + hz + 3.0).powi(2),
            tint,
        })
        .collect()
}

/// Everything solid in the arena.
#[derive(Default, Clone)]
pub struct Solids {
    pub pillars: Vec<Pillar>,
    pub boxes: Boxes,
    pub size: f64,
}

impl Solids {
    pub fn new(pillars: Vec<Pillar>, boxes: &[WallBox], size: f64) -> Solids {
        Solids { pillars, boxes: boxes_from(boxes), size }
    }

    /// Whether (x, z) is inside a wall's footprint, grown by `margin`.
    pub fn in_box(&self, x: f64, z: f64, margin: f64) -> bool {
        self.boxes.iter().any(|b| {
            let (px, pz) = (x - b.cx, z - b.cz);
            (b.c * px - b.s * pz).abs() < b.hx + margin && (b.s * px + b.c * pz).abs() < b.hz + margin
        })
    }

    /// Nearest distance along the ray into a wall box, or None.
    pub fn ray_boxes(&self, o: [f64; 3], d: [f64; 3], tmax: f64) -> Option<f64> {
        let mut best: Option<f64> = None;
        for b in &self.boxes {
            let (px, py, pz) = (o[0] - b.cx, o[1] - b.cy, o[2] - b.cz);
            if px * px + pz * pz > (tmax + b.hx + b.hz).powi(2) {
                continue;
            }
            let (lx, lz) = (b.c * px - b.s * pz, b.s * px + b.c * pz);
            let (dx, dz) = (b.c * d[0] - b.s * d[2], b.s * d[0] + b.c * d[2]);
            let (mut tn, mut tf) = (-1e18f64, 1e18f64);
            let mut ok = true;
            for (p, dd, h) in [(lx, dx, b.hx), (py, d[1], b.hy), (lz, dz, b.hz)] {
                if dd.abs() < 1e-9 {
                    if p.abs() > h {
                        ok = false;
                        break;
                    }
                    continue;
                }
                let (mut t1, mut t2) = ((-h - p) / dd, (h - p) / dd);
                if t1 > t2 {
                    std::mem::swap(&mut t1, &mut t2);
                }
                tn = tn.max(t1);
                tf = tf.min(t2);
                if tn > tf {
                    ok = false;
                    break;
                }
            }
            if !ok || tf < 0.0 {
                continue;
            }
            let t = tn.max(0.0);
            if t < tmax && best.map_or(true, |bb| t < bb) {
                best = Some(t);
            }
        }
        best
    }

    /// Nearest distance along the ray to a pillar (its side or its top), or None.
    pub fn ray_pillars(&self, o: [f64; 3], d: [f64; 3], tmax: f64) -> Option<f64> {
        let mut best: Option<f64> = None;
        for p in &self.pillars {
            let (x, z, r, b, top) = (p.0, p.1, p.2, p.3, p.4);
            let (ox, oz) = (o[0] - x, o[2] - z);
            let a = d[0] * d[0] + d[2] * d[2];
            if a < 1e-9 {
                continue;
            }
            let bb = 2.0 * (ox * d[0] + oz * d[2]);
            let c = ox * ox + oz * oz - r * r;
            let disc = bb * bb - 4.0 * a * c;
            if disc < 0.0 {
                continue;
            }
            let s = disc.sqrt();
            for t in [(-bb - s) / (2.0 * a), (-bb + s) / (2.0 * a)] {
                if 0.0 < t && t < tmax && best.map_or(true, |bb| t < bb) {
                    let y = o[1] + d[1] * t;
                    if b <= y && y <= top {
                        best = Some(t);
                        break;
                    }
                }
            }
            if c < 0.0 && b <= o[1] && o[1] <= top {
                // inside the pillar's circle and within its height: blocked straight away
                best = Some(0.0);
            }
        }
        // tops of pillars: a ray coming down onto one
        for p in &self.pillars {
            if d[1].abs() > 1e-6 {
                let t = (p.4 - o[1]) / d[1];
                if 0.0 < t && t < tmax && best.map_or(true, |bb| t < bb) {
                    if (o[0] + d[0] * t - p.0).hypot(o[2] + d[2] * t - p.1) <= p.2 {
                        best = Some(t);
                    }
                }
            }
        }
        best
    }

    /// Nearest distance along the ray to anything solid built on the ground, or None.
    pub fn ray(&self, o: [f64; 3], d: [f64; 3], tmax: f64) -> Option<f64> {
        let a = self.ray_pillars(o, d, tmax);
        let b = self.ray_boxes(o, d, a.unwrap_or(tmax));
        b.or(a)
    }

    /// Move a body standing at (x, z) with its feet at `y` out of any pillar or wall, and keep it in the arena.
    pub fn push_out(&self, mut x: f64, mut z: f64, y: f64) -> (f64, f64) {
        for p in &self.pillars {
            if y >= p.4 - 0.5 {
                continue;
            }
            let (mut dx, mut dz) = (x - p.0, z - p.1);
            let mut dd = dx.hypot(dz);
            let lim = p.2 + 1.6;
            if dd < lim {
                if dd < 1e-3 {
                    dx = 1.0;
                    dz = 0.0;
                    dd = 1.0;
                }
                x = p.0 + dx / dd * lim;
                z = p.1 + dz / dd * lim;
            }
        }
        const R: f64 = 1.4;
        for b in &self.boxes {
            if y >= b.top - 0.35 || y + 5.8 <= b.bot {
                continue;
            }
            let (px, pz) = (x - b.cx, z - b.cz);
            if px * px + pz * pz > b.r2 {
                continue;
            }
            let (mut lx, mut lz) = (b.c * px - b.s * pz, b.s * px + b.c * pz);
            let (qx, qz) = (lx.max(-b.hx).min(b.hx), lz.max(-b.hz).min(b.hz));
            let (ex, ez) = (lx - qx, lz - qz);
            let dd = ex.hypot(ez);
            if dd >= R {
                continue;
            }
            if dd > 1e-4 {
                lx = qx + ex / dd * R;
                lz = qz + ez / dd * R;
            } else if b.hx - lx.abs() < b.hz - lz.abs() {
                lx = (b.hx + R).copysign(if lx == 0.0 { 1.0 } else { lx });
            } else {
                lz = (b.hz + R).copysign(if lz == 0.0 { 1.0 } else { lz });
            }
            x = b.cx + b.c * lx + b.s * lz;
            z = b.cz - b.s * lx + b.c * lz;
        }
        (x.max(2.0).min(self.size - 2.0), z.max(2.0).min(self.size - 2.0))
    }

    /// What a body at (x, z) with its feet at `y` stands on: the ground, or a pillar top or wall top it is above.
    pub fn floor_at(&self, ground: f64, x: f64, z: f64, y: f64) -> f64 {
        let mut f = ground;
        for p in &self.pillars {
            if y >= p.4 - 1.0 && (x - p.0).hypot(z - p.1) <= p.2 {
                f = f.max(p.4);
            }
        }
        for b in &self.boxes {
            if y < b.top - 1.0 || b.top <= f {
                continue;
            }
            let (px, pz) = (x - b.cx, z - b.cz);
            if px * px + pz * pz > b.r2 {
                continue;
            }
            if (b.c * px - b.s * pz).abs() <= b.hx + 0.3 && (b.s * px + b.c * pz).abs() <= b.hz + 0.3 {
                f = b.top;
            }
        }
        f
    }
}
