//! The ground: a square height grid with a bit mask of where the creek is.

use serde::Deserialize;

/// `terrain.json`: the grid size, the cell size and the side of the square, in feet.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct GroundMeta {
    pub n: usize,
    pub cell: f64,
    pub size: f64,
}

pub struct Ground {
    pub n: usize,
    pub cell: f64,
    pub size: f64,
    /// Heights in feet, row by row (z rows, x columns).
    pub h: Vec<f64>,
    /// 1 where the creek is.
    pub wet: Vec<u8>,
}

impl Ground {
    /// From `terrain.json` and `terrain.bin`: little-endian u16 heights in hundredths of a foot, then a packed bit mask
    /// of the water, high bit first.
    pub fn from_bytes(meta: GroundMeta, raw: &[u8]) -> Result<Ground, String> {
        let n2 = meta.n * meta.n;
        if raw.len() < 2 * n2 + (n2 + 7) / 8 {
            return Err(format!("terrain.bin is {} bytes, too short for a {}x{} grid", raw.len(), meta.n, meta.n));
        }
        let h = (0..n2).map(|i| u16::from_le_bytes([raw[2 * i], raw[2 * i + 1]]) as f64 / 100.0).collect();
        let bits = &raw[2 * n2..];
        let wet = (0..n2).map(|i| (bits[i >> 3] >> (7 - (i & 7))) & 1).collect();
        Ok(Ground { n: meta.n, cell: meta.cell, size: meta.size, h, wet })
    }

    /// Ground height at (x, z), interpolated across the cell.
    pub fn at(&self, x: f64, z: f64) -> f64 {
        let n = self.n;
        let fx = (x / self.cell).max(0.0).min(n as f64 - 1.001);
        let fz = (z / self.cell).max(0.0).min(n as f64 - 1.001);
        let (i, j) = (fx as usize, fz as usize);
        let (u, v) = (fx - i as f64, fz - j as f64);
        let h = &self.h;
        let (a, b) = (h[j * n + i], h[j * n + i + 1]);
        let (c, d) = (h[(j + 1) * n + i], h[(j + 1) * n + i + 1]);
        (a * (1.0 - u) + b * u) * (1.0 - v) + (c * (1.0 - u) + d * u) * v
    }

    fn cell_of(&self, x: f64, z: f64) -> usize {
        let n = self.n as f64;
        // Python's round() and JavaScript's Math.round() differ only at exact halves, which a float position on a
        // 4 ft grid practically never lands on; this follows Python (ties to even).
        let i = round_half_even(x / self.cell).max(0.0).min(n - 1.0) as usize;
        let j = round_half_even(z / self.cell).max(0.0).min(n - 1.0) as usize;
        j * self.n + i
    }

    /// Whether (x, z) is in the creek.
    pub fn water(&self, x: f64, z: f64) -> bool {
        self.wet[self.cell_of(x, z)] != 0
    }

    /// How steep the ground is at (x, z) (rise over run).
    pub fn slope(&self, x: f64, z: f64) -> f64 {
        let e = 4.0;
        let a = self.at(x + e, z) - self.at(x - e, z);
        let b = self.at(x, z + e) - self.at(x, z - e);
        crate::dmath::hypot(a, b) / (2.0 * e)
    }

    /// Distance along the ray to where it goes into the ground, or None.
    pub fn ray(&self, o: [f64; 3], d: [f64; 3], tmax: f64) -> Option<f64> {
        let step = 2.0;
        let mut t = step;
        while t < tmax {
            let (x, y, z) = (o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t);
            if x < 0.0 || z < 0.0 || x > self.size || z > self.size {
                return None;
            }
            if y < self.at(x, z) {
                let (mut lo, mut hi) = (t - step, t);
                for _ in 0..6 {
                    let m = (lo + hi) / 2.0;
                    if o[1] + d[1] * m < self.at(o[0] + d[0] * m, o[2] + d[2] * m) {
                        hi = m;
                    } else {
                        lo = m;
                    }
                }
                return Some(hi);
            }
            t += step;
        }
        None
    }
}

fn round_half_even(v: f64) -> f64 {
    let r = v.round();
    if (v - v.trunc()).abs() == 0.5 {
        2.0 * (v / 2.0).round()
    } else {
        r
    }
}
