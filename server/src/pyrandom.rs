//! Python's `random.Random`: the same Mersenne Twister, seeded the same way, with the same `random()`, `uniform()`,
//! `choice()` and `randrange()`. The arena is built from seed 415, so this keeps it exactly where it always was.

pub struct PyRandom {
    mt: [u32; 624],
    idx: usize,
    gauss_next: Option<f64>,
}

impl PyRandom {
    /// `random.Random(seed)` for a non-negative integer seed.
    pub fn new(seed: u64) -> PyRandom {
        let mut key: Vec<u32> = Vec::new();
        let mut s = seed;
        loop {
            key.push((s & 0xffff_ffff) as u32);
            s >>= 32;
            if s == 0 {
                break;
            }
        }
        let mut r = PyRandom { mt: [0; 624], idx: 624, gauss_next: None };
        r.init_by_array(&key);
        r
    }

    /// Seeded from the clock, for things that only need to be unpredictable.
    pub fn from_time() -> PyRandom {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(1);
        PyRandom::new((t as u64) ^ ((t >> 64) as u64) ^ (std::process::id() as u64) << 20)
    }

    fn init_genrand(&mut self, s: u32) {
        self.mt[0] = s;
        for i in 1..624 {
            let prev = self.mt[i - 1];
            self.mt[i] = 1812433253u32.wrapping_mul(prev ^ (prev >> 30)).wrapping_add(i as u32);
        }
        self.idx = 624;
    }

    fn init_by_array(&mut self, key: &[u32]) {
        self.init_genrand(19650218);
        let n = 624usize;
        let (mut i, mut j) = (1usize, 0usize);
        let mut k = n.max(key.len());
        while k > 0 {
            let prev = self.mt[i - 1];
            self.mt[i] = (self.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1664525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= n {
                self.mt[0] = self.mt[n - 1];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
            k -= 1;
        }
        k = n - 1;
        while k > 0 {
            let prev = self.mt[i - 1];
            self.mt[i] = (self.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1566083941)).wrapping_sub(i as u32);
            i += 1;
            if i >= n {
                self.mt[0] = self.mt[n - 1];
                i = 1;
            }
            k -= 1;
        }
        self.mt[0] = 0x8000_0000;
        self.idx = 624;
    }

    fn genrand_u32(&mut self) -> u32 {
        const UPPER: u32 = 0x8000_0000;
        const LOWER: u32 = 0x7fff_ffff;
        const MATRIX_A: u32 = 0x9908_b0df;
        if self.idx >= 624 {
            for kk in 0..624 {
                let y = (self.mt[kk] & UPPER) | (self.mt[(kk + 1) % 624] & LOWER);
                let mut v = self.mt[(kk + 397) % 624] ^ (y >> 1);
                if y & 1 != 0 {
                    v ^= MATRIX_A;
                }
                self.mt[kk] = v;
            }
            self.idx = 0;
        }
        let mut y = self.mt[self.idx];
        self.idx += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        y
    }

    /// A float in [0, 1) with 53 random bits.
    pub fn random(&mut self) -> f64 {
        let a = (self.genrand_u32() >> 5) as f64;
        let b = (self.genrand_u32() >> 6) as f64;
        (a * 67108864.0 + b) * (1.0 / 9007199254740992.0)
    }

    pub fn uniform(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.random()
    }

    fn getrandbits(&mut self, k: u32) -> u32 {
        debug_assert!(k >= 1 && k <= 32);
        self.genrand_u32() >> (32 - k)
    }

    /// An integer in [0, n).
    pub fn randbelow(&mut self, n: usize) -> usize {
        assert!(n > 0);
        let k = usize::BITS - n.leading_zeros();
        loop {
            let r = self.getrandbits(k) as usize;
            if r < n {
                return r;
            }
        }
    }

    pub fn choice<T: Copy>(&mut self, seq: &[T]) -> T {
        seq[self.randbelow(seq.len())]
    }

    /// `random.randint(a, b)`: both ends included.
    pub fn randint(&mut self, a: i64, b: i64) -> i64 {
        a + self.randbelow((b - a + 1) as usize) as i64
    }

    /// `random.gauss(mu, sigma)`.
    pub fn gauss(&mut self, mu: f64, sigma: f64) -> f64 {
        let z = match self.gauss_next.take() {
            Some(z) => z,
            None => {
                let x2pi = self.random() * std::f64::consts::TAU;
                let g2rad = (-2.0 * (1.0 - self.random()).ln()).sqrt();
                self.gauss_next = Some(x2pi.sin() * g2rad);
                x2pi.cos() * g2rad
            }
        };
        mu + z * sigma
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_cpython() {
        // printed by CPython 3.12: r = random.Random(415) ...
        let mut r = PyRandom::new(415);
        assert_eq!(r.random(), 0.7752162988425123);
        assert_eq!(r.random(), 0.24665091780030424);
        assert_eq!(r.random(), 0.27759570027956904);
        assert_eq!(r.uniform(60.0, 1340.0), 337.6671115090221);
        assert_eq!(r.uniform(60.0, 1340.0), 776.6128435159537);
        let c: Vec<i32> = (0..6).map(|_| r.choice(&[22, 26, 30, 36, 40])).collect();
        assert_eq!(c, vec![26, 22, 40, 22, 36, 26]);
        let c: Vec<usize> = (0..6).map(|_| r.randbelow(4)).collect();
        assert_eq!(c, vec![1, 0, 0, 2, 1, 0]);
        assert_eq!(r.getrandbits(32), 691798127);
    }
}
