//! Building the arena: ruined buildings, lone walls and low barriers, pillars, and power-up spots, all from one
//! seed so the arena is the same every time (and the same as the original Python server built).

use lasertag_shared::protocol::Spot;
use lasertag_shared::solids::{Pillar, WallBox, Window};
use lasertag_shared::dmath::{atan2, cos, hypot, sin};
use lasertag_shared::{round_to, Ground, Solids};
use std::f64::consts::PI;

use crate::pyrandom::PyRandom;

pub const SPOT_KINDS: [&str; 4] = ["rapid", "over", "speed", "drone"];

pub struct Arena {
    pub pillars: Vec<Pillar>,
    pub spots: Vec<Spot>,
    pub boxes: Vec<WallBox>,
    pub windows: Vec<Window>,
    pub solids: Solids,
}

/// An opening cut into a wall: from `s0` to `s1` along it, `y0` to `y1` up it, with or without glass.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
struct Opening(f64, f64, f64, f64, bool);

struct Builder<'a> {
    g: &'a Ground,
    rnd: PyRandom,
    boxes: Vec<WallBox>,
    windows: Vec<Window>,
}

impl<'a> Builder<'a> {
    /// A wall from (x0, z0) to (x1, z1), standing on `base`, `h` tall and `t` thick, with openings cut out of it. A
    /// ruined wall has a ragged top and some lintels gone.
    #[allow(clippy::too_many_arguments)]
    fn add_wall(&mut self, x0: f64, z0: f64, x1: f64, z1: f64, base: f64, h: f64, t: f64, mut openings: Vec<Opening>, tint: u32, ruin: bool) {
        let l = hypot(x1 - x0, z1 - z0);
        let (ux, uz) = ((x1 - x0) / l, (z1 - z0) / l);
        let a = atan2(-uz, ux);
        let mut boxes = std::mem::take(&mut self.boxes);
        let mut add = |s0: f64, s1: f64, y0: f64, y1: f64| {
            if s1 - s0 < 0.3 || y1 - y0 < 0.3 {
                return;
            }
            let sm = (s0 + s1) / 2.0;
            boxes.push(WallBox(
                round_to(x0 + ux * sm, 2),
                round_to(base + (y0 + y1) / 2.0, 2),
                round_to(z0 + uz * sm, 2),
                round_to((s1 - s0) / 2.0, 2),
                round_to((y1 - y0) / 2.0, 2),
                round_to(t / 2.0, 2),
                round_to(a, 4),
                tint,
            ));
        };
        let rnd = &mut self.rnd;
        let solid = |rnd: &mut PyRandom, add: &mut dyn FnMut(f64, f64, f64, f64), s0: f64, s1: f64| {
            let mut x = s0;
            while x < s1 - 0.01 {
                let mut e = if ruin { s1.min(x + rnd.uniform(3.0, 6.0)) } else { s1 };
                if s1 - e < 1.5 {
                    e = s1;
                }
                let drop = if ruin { rnd.choice(&[0.0, 0.0, 0.0, 0.6, 1.5, 3.0, 5.0]) } else { 0.0 };
                add(x, e, 0.0, h - drop);
                x = e;
            }
        };
        openings.sort_by(|p, q| p.partial_cmp(q).unwrap());
        let mut cur = 0.0;
        for Opening(s0, s1, y0, y1, glass) in openings {
            solid(rnd, &mut add, cur, s0);
            add(s0, s1, 0.0, y0);
            if !(ruin && rnd.random() < 0.22) {
                let drop = if ruin { rnd.choice(&[0.0, 0.0, 0.8]) } else { 0.0 };
                add(s0, s1, y1, h - drop);
            }
            if glass {
                let sm = (s0 + s1) / 2.0;
                self.windows.push(Window(
                    round_to(x0 + ux * sm, 2),
                    round_to(base + (y0 + y1) / 2.0, 2),
                    round_to(z0 + uz * sm, 2),
                    round_to((s1 - s0) / 2.0, 2),
                    round_to((y1 - y0) / 2.0, 2),
                    round_to(a, 4),
                ));
            }
            cur = s1;
        }
        solid(rnd, &mut add, cur, l);
        drop(add);
        self.boxes = boxes;
    }

    fn openings_for(&mut self, l: f64, door: bool) -> Vec<Opening> {
        let n = (((l - 3.0) / 8.0) as i64).max(0);
        let mut out: Vec<Opening> = Vec::new();
        for k in 0..n {
            let c = (k as f64 + 0.5) * l / n as f64;
            out.push(Opening(c - 2.0, c + 2.0, 3.5, 7.0, self.rnd.random() < 0.9));
        }
        if door {
            if !out.is_empty() {
                let mut k = 0;
                let key = |o: &Opening| ((o.0 + o.1) / 2.0 - l / 2.0).abs();
                for i in 1..out.len() {
                    if key(&out[i]) < key(&out[k]) {
                        k = i;
                    }
                }
                let c = (out[k].0 + out[k].1) / 2.0;
                out[k] = Opening(c - 3.0, c + 3.0, 0.0, 8.0, false);
            } else if l > 8.0 {
                out.push(Opening(l / 2.0 - 3.0, l / 2.0 + 3.0, 0.0, 8.0, false));
            }
        }
        out
    }

    fn ground_span(&self, pts: &[(f64, f64)]) -> (f64, f64, bool) {
        let hs: Vec<f64> = pts.iter().map(|&(x, z)| self.g.at(x, z)).collect();
        let lo = hs.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = hs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        (lo, hi, pts.iter().any(|&(x, z)| self.g.water(x, z)))
    }

    fn build_structures(&mut self, taken: &mut Vec<(f64, f64, f64)>) {
        let size = self.g.size;
        let tints: [u32; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
        let free = |taken: &Vec<(f64, f64, f64)>, x: f64, z: f64, r: f64, gap: f64| {
            taken.iter().all(|&(a, b, c)| hypot(x - a, z - b) > r + c + gap)
                && 40.0 < x
                && x < size - 40.0
                && 40.0 < z
                && z < size - 40.0
        };
        // buildings: four ruined walls round a room, a door in one
        let (mut made, mut tries) = (0, 0);
        while made < 9 && tries < 3000 {
            tries += 1;
            let bx = self.rnd.uniform(60.0, size - 60.0);
            let bz = self.rnd.uniform(60.0, size - 60.0);
            let w: f64 = self.rnd.choice(&[22.0, 26.0, 30.0, 36.0, 40.0]);
            let d: f64 = self.rnd.choice(&[18.0, 22.0, 26.0, 30.0]);
            let a = self.rnd.uniform(0.0, PI);
            let r = hypot(w, d) / 2.0;
            if !free(taken, bx, bz, r, 35.0) {
                continue;
            }
            let (ca, sa) = (cos(a), sin(a));
            let wp = |lx: f64, lz: f64| (bx + ca * lx + sa * lz, bz - sa * lx + ca * lz);
            let mut pts = Vec::new();
            for i in [-1.0, 0.0, 1.0] {
                for j in [-1.0, 0.0, 1.0] {
                    pts.push(wp(i * w / 2.0, j * d / 2.0));
                }
            }
            let (lo, hi, wet) = self.ground_span(&pts);
            if wet || hi - lo > 4.0 {
                continue;
            }
            let base = lo - 0.6;
            let h = self.rnd.choice(&[10.0, 11.0, 12.0, 13.0]) + (hi - lo);
            let t = 1.2;
            let tint = self.rnd.choice(&tints);
            let door = self.rnd.randbelow(4);
            let walls = [
                ((-w / 2.0, -d / 2.0 + t / 2.0), (w / 2.0, -d / 2.0 + t / 2.0)),
                ((w / 2.0, d / 2.0 - t / 2.0), (-w / 2.0, d / 2.0 - t / 2.0)),
                ((-w / 2.0 + t / 2.0, d / 2.0 - t), (-w / 2.0 + t / 2.0, -d / 2.0 + t)),
                ((w / 2.0 - t / 2.0, -d / 2.0 + t), (w / 2.0 - t / 2.0, d / 2.0 - t)),
            ];
            for (k, (p0, p1)) in walls.iter().enumerate() {
                let (x0, z0) = wp(p0.0, p0.1);
                let (x1, z1) = wp(p1.0, p1.1);
                let l = hypot(x1 - x0, z1 - z0);
                let ops = self.openings_for(l, k == door);
                self.add_wall(x0, z0, x1, z1, base, h, t, ops, tint, true);
            }
            taken.push((bx, bz, r));
            made += 1;
        }
        // lone broken walls with a window or two, and low barriers to crouch behind
        for (kind, count) in [("wall", 16), ("low", 14)] {
            let (mut made, mut tries) = (0, 0);
            while made < count && tries < 3000 {
                tries += 1;
                let cx = self.rnd.uniform(50.0, size - 50.0);
                let cz = self.rnd.uniform(50.0, size - 50.0);
                let l = if kind == "wall" { self.rnd.uniform(16.0, 30.0) } else { self.rnd.uniform(10.0, 16.0) };
                let a = self.rnd.uniform(0.0, PI);
                if !free(taken, cx, cz, l / 2.0, 18.0) {
                    continue;
                }
                let (x0, z0) = (cx - cos(a) * l / 2.0, cz - sin(a) * l / 2.0);
                let (x1, z1) = (cx + cos(a) * l / 2.0, cz + sin(a) * l / 2.0);
                let pts: Vec<(f64, f64)> =
                    [0.0, 0.25, 0.5, 0.75, 1.0].iter().map(|&f| (x0 + (x1 - x0) * f, z0 + (z1 - z0) * f)).collect();
                let (lo, hi, wet) = self.ground_span(&pts);
                if wet || hi - lo > 3.0 {
                    continue;
                }
                if kind == "wall" {
                    let h = self.rnd.uniform(8.0, 10.0) + (hi - lo);
                    let ops = self.openings_for(l, false);
                    let tint = self.rnd.choice(&tints);
                    self.add_wall(x0, z0, x1, z1, lo - 0.6, h, 1.2, ops, tint, true);
                } else {
                    let tint = self.rnd.choice(&tints);
                    self.add_wall(x0, z0, x1, z1, lo - 0.6, 4.0 + (hi - lo), 1.6, Vec::new(), tint, false);
                }
                taken.push((cx, cz, l / 2.0));
                made += 1;
            }
        }
    }
}

pub fn build_arena(g: &Ground, seed: u64) -> Arena {
    let size = g.size;
    let mut b = Builder { g, rnd: PyRandom::new(seed), boxes: Vec::new(), windows: Vec::new() };
    let mut taken = Vec::new();
    b.build_structures(&mut taken);
    let walls_only = Solids::new(Vec::new(), &b.boxes, size);
    let rnd = &mut b.rnd;
    let mut pillars: Vec<Pillar> = Vec::new();
    let mut tries = 0;
    while pillars.len() < 30 && tries < 5000 {
        tries += 1;
        let x = rnd.uniform(60.0, size - 60.0);
        let z = rnd.uniform(60.0, size - 60.0);
        let r = rnd.choice(&[3.0, 4.0, 4.0, 5.0, 6.0, 8.0, 10.0]);
        if g.water(x, z) || g.slope(x, z) > 0.5 {
            continue;
        }
        if taken.iter().any(|&(a, bb, c)| hypot(x - a, z - bb) < c + r + 30.0) {
            continue;
        }
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for dx in [-r, 0.0, r] {
            for dz in [-r, 0.0, r] {
                let h = g.at(x + dx, z + dz);
                lo = lo.min(h);
                hi = hi.max(h);
            }
        }
        let base = lo - 1.0;
        let top = hi + rnd.choice(&[9.0, 12.0, 14.0, 18.0, 24.0]);
        pillars.push(Pillar(round_to(x, 1), round_to(z, 1), r, round_to(base, 2), round_to(top, 2)));
        taken.push((x, z, r + 15.0));
    }
    let mut spots: Vec<Spot> = Vec::new();
    tries = 0;
    while spots.len() < 9 && tries < 5000 {
        tries += 1;
        let x = rnd.uniform(120.0, size - 120.0);
        let z = rnd.uniform(120.0, size - 120.0);
        if g.water(x, z) || g.slope(x, z) > 0.4 || walls_only.in_box(x, z, 4.0) {
            continue;
        }
        if pillars.iter().any(|p| hypot(x - p.0, z - p.1) < p.2 + 12.0) {
            continue;
        }
        if spots.iter().any(|s| hypot(x - s.x, z - s.z) < 220.0) {
            continue;
        }
        spots.push(Spot {
            i: spots.len() as u32,
            kind: SPOT_KINDS[spots.len() % SPOT_KINDS.len()].to_string(),
            x: round_to(x, 1),
            z: round_to(z, 1),
            y: round_to(g.at(x, z), 2),
            up: true,
            back: 0.0,
        });
    }
    let solids = Solids::new(pillars.clone(), &b.boxes, size);
    Arena { pillars, spots, boxes: b.boxes, windows: b.windows, solids }
}
