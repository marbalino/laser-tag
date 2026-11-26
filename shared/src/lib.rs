//! Creek Laser Tag: what the game server and the browser game share.
//!
//! The ground (heights and the creek), the solid things built on it (pillars and ruined walls), the ray and
//! collision maths both sides use, and the messages they send each other.

pub mod ground;
pub mod protocol;
pub mod solids;

pub use ground::{Ground, GroundMeta};
pub use solids::{Boxes, Pillar, Solids, WallBox};

/// Eye height above the feet, in feet.
pub const EYE: f64 = 5.4;
/// The colours a player can pick.
pub const PALETTE: [&str; 8] = ["#ff3df2", "#22f0ff", "#ffe23d", "#7dff3d", "#ff8a1f", "#a66bff", "#ff4d6d", "#3dffb4"];

/// Round to `places` decimals the way Python's `round()` does (correctly rounded, ties to even), so numbers on
/// the wire and the arena layout match the original server exactly.
pub fn round_to(x: f64, places: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let s = format!("{:.*}", places, x);
    let v: f64 = s.parse().unwrap_or(x);
    if v == 0.0 {
        0.0
    } else {
        v
    }
}

/// Distance between two points.
pub fn dist3(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (x, y, z) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    (x * x + y * y + z * z).sqrt()
}

/// Python's `%` for floats: the result takes the sign of the divisor.
pub fn py_mod(a: f64, b: f64) -> f64 {
    let r = a % b;
    if r != 0.0 && (r < 0.0) != (b < 0.0) {
        r + b
    } else {
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_like_python() {
        assert_eq!(round_to(0.125, 2), 0.12);
        assert_eq!(round_to(2.675, 2), 2.67);
        assert_eq!(round_to(-1.005, 2), -1.0);
        assert_eq!(round_to(-0.001, 2), 0.0);
        assert_eq!(round_to(12.3456, 3), 12.346);
    }

    #[test]
    fn mod_like_python() {
        assert!((py_mod(-1.0, std::f64::consts::TAU) - (std::f64::consts::TAU - 1.0)).abs() < 1e-12);
        assert_eq!(py_mod(7.0, 4.0), 3.0);
    }
}
