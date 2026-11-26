//! Shapes, built the way three.js builds them (same vertex order and winding), so front faces face out.

use crate::gl::Geometry;
use std::f32::consts::PI;

fn push3(v: &mut Vec<f32>, a: f32, b: f32, c: f32) {
    v.extend_from_slice(&[a, b, c]);
}

/// A cylinder (a cone when one radius is 0) along y, centred on the origin.
pub fn cylinder(rt: f32, rb: f32, h: f32, segs: u32, open: bool) -> Geometry {
    let mut g = Geometry::default();
    let half = h / 2.0;
    let slope = (rb - rt) / h;
    let mut rows: Vec<Vec<u32>> = Vec::new();
    for y in 0..=1u32 {
        let v = y as f32;
        let r = v * (rb - rt) + rt;
        let mut row = Vec::new();
        for x in 0..=segs {
            let th = x as f32 / segs as f32 * PI * 2.0;
            let (s, c) = th.sin_cos();
            push3(&mut g.pos, r * s, -v * h + half, r * c);
            let n = glam::Vec3::new(s, slope, c).normalize();
            push3(&mut g.normal, n.x, n.y, n.z);
            row.push((g.pos.len() / 3 - 1) as u32);
        }
        rows.push(row);
    }
    for x in 0..segs as usize {
        let (a, b, c, d) = (rows[0][x], rows[1][x], rows[1][x + 1], rows[0][x + 1]);
        g.index.extend_from_slice(&[a, b, d, b, c, d]);
    }
    if !open {
        for top in [true, false] {
            let r = if top { rt } else { rb };
            if r <= 0.0 {
                continue;
            }
            let sign = if top { 1.0 } else { -1.0 };
            let cstart = (g.pos.len() / 3) as u32;
            for _ in 1..=segs {
                push3(&mut g.pos, 0.0, half * sign, 0.0);
                push3(&mut g.normal, 0.0, sign, 0.0);
            }
            let cend = (g.pos.len() / 3) as u32;
            for x in 0..=segs {
                let th = x as f32 / segs as f32 * PI * 2.0;
                push3(&mut g.pos, r * th.sin(), half * sign, r * th.cos());
                push3(&mut g.normal, 0.0, sign, 0.0);
            }
            for x in 0..segs {
                let (c, i) = (cstart + x, cend + x);
                if top {
                    g.index.extend_from_slice(&[i, i + 1, c]);
                } else {
                    g.index.extend_from_slice(&[i + 1, i, c]);
                }
            }
        }
    }
    g
}

/// Rows of a sphere-like surface of revolution: (polar angle, y offset) per row.
fn revolve(r: f32, wsegs: u32, rows: &[(f32, f32)]) -> Geometry {
    let mut g = Geometry::default();
    for &(theta, yoff) in rows {
        for ix in 0..=wsegs {
            let phi = ix as f32 / wsegs as f32 * PI * 2.0;
            let n = glam::Vec3::new(-phi.cos() * theta.sin(), theta.cos(), phi.sin() * theta.sin());
            push3(&mut g.pos, n.x * r, n.y * r + yoff, n.z * r);
            push3(&mut g.normal, n.x, n.y, n.z);
        }
    }
    let w = wsegs + 1;
    let nrows = rows.len() as u32;
    for iy in 0..nrows - 1 {
        for ix in 0..wsegs {
            let a = iy * w + ix + 1;
            let b = iy * w + ix;
            let c = (iy + 1) * w + ix;
            let d = (iy + 1) * w + ix + 1;
            if iy != 0 {
                g.index.extend_from_slice(&[a, b, d]);
            }
            if iy != nrows - 2 {
                g.index.extend_from_slice(&[b, c, d]);
            }
        }
    }
    g
}

pub fn sphere(r: f32, wsegs: u32, hsegs: u32) -> Geometry {
    let rows: Vec<(f32, f32)> = (0..=hsegs).map(|iy| (iy as f32 / hsegs as f32 * PI, 0.0)).collect();
    revolve(r, wsegs, &rows)
}

/// A capsule: a cylinder `length` long with half-spheres on both ends, along y.
pub fn capsule(r: f32, length: f32, cap_segs: u32, radial: u32) -> Geometry {
    let n = cap_segs * 2;
    let mut rows = Vec::new();
    for i in 0..=n {
        rows.push((i as f32 / n as f32 * PI / 2.0, length / 2.0));
    }
    for i in 0..=n {
        rows.push((PI / 2.0 + i as f32 / n as f32 * PI / 2.0, -length / 2.0));
    }
    let mut g = revolve(r, radial, &rows);
    // the band between the hemispheres is a real face: revolve() only skips the poles, so nothing to add
    g.index.shrink_to_fit();
    g
}

/// A torus round the z axis (in the xy plane).
pub fn torus(radius: f32, tube: f32, radial: u32, tubular: u32) -> Geometry {
    let mut g = Geometry::default();
    for j in 0..=radial {
        for i in 0..=tubular {
            let u = i as f32 / tubular as f32 * PI * 2.0;
            let v = j as f32 / radial as f32 * PI * 2.0;
            let p = glam::Vec3::new((radius + tube * v.cos()) * u.cos(), (radius + tube * v.cos()) * u.sin(), tube * v.sin());
            let c = glam::Vec3::new(radius * u.cos(), radius * u.sin(), 0.0);
            let n = (p - c).normalize();
            push3(&mut g.pos, p.x, p.y, p.z);
            push3(&mut g.normal, n.x, n.y, n.z);
        }
    }
    let t = tubular + 1;
    for j in 1..=radial {
        for i in 1..=tubular {
            let a = t * j + i - 1;
            let b = t * (j - 1) + i - 1;
            let c = t * (j - 1) + i;
            let d = t * j + i;
            g.index.extend_from_slice(&[a, b, d, b, c, d]);
        }
    }
    g
}

/// A flat ring in the xy plane, facing +z.
pub fn ring(inner: f32, outer: f32, segs: u32) -> Geometry {
    let mut g = Geometry::default();
    for r in [inner, outer] {
        for i in 0..=segs {
            let a = i as f32 / segs as f32 * PI * 2.0;
            push3(&mut g.pos, r * a.cos(), r * a.sin(), 0.0);
            push3(&mut g.normal, 0.0, 0.0, 1.0);
        }
    }
    for i in 0..segs {
        let (a, b, c, d) = (i, i + segs + 1, i + segs + 2, i + 1);
        g.index.extend_from_slice(&[a, b, d, b, c, d]);
    }
    g
}

/// A flat disc in the xy plane, facing +z.
pub fn circle(r: f32, segs: u32) -> Geometry {
    let mut g = Geometry::default();
    push3(&mut g.pos, 0.0, 0.0, 0.0);
    push3(&mut g.normal, 0.0, 0.0, 1.0);
    for s in 0..=segs {
        let a = s as f32 / segs as f32 * PI * 2.0;
        push3(&mut g.pos, r * a.cos(), r * a.sin(), 0.0);
        push3(&mut g.normal, 0.0, 0.0, 1.0);
    }
    for i in 1..=segs {
        g.index.extend_from_slice(&[i, i + 1, 0]);
    }
    g
}

/// A box w × h × d centred on the origin.
pub fn cuboid(w: f32, h: f32, d: f32) -> Geometry {
    let mut g = Geometry::default();
    let hs = [w / 2.0, h / 2.0, d / 2.0];
    // normal, u, v with u × v = normal
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ];
    for (n, u, v) in faces {
        let base = (g.pos.len() / 3) as u32;
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            for k in 0..3 {
                g.pos.push(n[k] * hs[k] + (u[k] * a + v[k] * b) * hs[k]);
            }
            push3(&mut g.normal, n[0], n[1], n[2]);
        }
        g.index.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    g
}

pub fn octahedron(r: f32) -> Geometry {
    let v: [[f32; 3]; 6] = [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]];
    let idx = [0, 2, 4, 0, 4, 3, 0, 3, 5, 0, 5, 2, 1, 2, 5, 1, 5, 3, 1, 3, 4, 1, 4, 2];
    let mut g = Geometry::default();
    for tri in idx.chunks(3) {
        let (a, b, c) = (glam::Vec3::from(v[tri[0]]), glam::Vec3::from(v[tri[1]]), glam::Vec3::from(v[tri[2]]));
        let n = (b - a).cross(c - a).normalize();
        for p in [a, b, c] {
            push3(&mut g.pos, p.x * r, p.y * r, p.z * r);
            push3(&mut g.normal, n.x, n.y, n.z);
        }
    }
    g
}

/// Move every vertex.
pub fn translate(mut g: Geometry, dx: f32, dy: f32, dz: f32) -> Geometry {
    for p in g.pos.chunks_mut(3) {
        p[0] += dx;
        p[1] += dy;
        p[2] += dz;
    }
    g
}
