//! The game: one arena, everyone in it. Clients move themselves and tell the server where they are; the server
//! decides who a laser hits (rewinding everyone to what the shooter saw), keeps score, runs rounds, power-ups,
//! missiles, drones and bots, and sends a snapshot of everyone 20 times a second.

use std::collections::{BTreeMap, VecDeque};
use std::sync::OnceLock;
use std::time::Instant;

use lasertag_shared::protocol::{Public, Rules, ServerMsg, SnapMissile, SnapPlayer, Spot};
use lasertag_shared::{dist3, py_mod, round_to, Ground, Solids, EYE, PALETTE};
use std::f64::consts::{PI, TAU};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::arena::Arena;
use crate::pyrandom::PyRandom;

pub const MAX_HP: i32 = 3;
pub const RESPAWN_S: f64 = 3.0;
pub const SHIELD_S: f64 = 2.5;
pub const FIRE_CD: f64 = 0.50;
pub const RAPID_CD: f64 = 0.20;
pub const HEAT_SHOT: f64 = 0.20; // each shot heats the gun this much (1 = overheated)
pub const HEAT_SHOT_RAPID: f64 = 0.08;
pub const HEAT_COOL: f64 = 0.20; // per second
pub const OVERHEAT_S: f64 = 2.0; // locked out this long once it overheats
pub const RANGE: f64 = 650.0;
pub const HIT_R: f64 = 2.0;
pub const BODY: (f64, f64) = (1.4, 5.2); // capsule spine, ft above the feet
pub const REGEN_AFTER: f64 = 4.0;
pub const REGEN_EVERY: f64 = 2.5;
pub const POWER_S: f64 = 12.0;
pub const PICKUP_RESPAWN: f64 = 20.0;
pub const MAX_HUMANS: usize = 16;
pub const BOT_NAMES: [&str; 12] = [
    "Crawdad", "Heron", "Steelhead", "Otter", "Beaver", "Kingfisher", "Salamander", "Mudpuppy", "Dragonfly", "Bullfrog",
    "Coyote", "Raccoon",
];
pub const MISSILE_CD: f64 = 12.0; // seconds between missiles
pub const MISSILE_SPEED: f64 = 95.0; // ft/s
pub const MISSILE_LIFE: f64 = 8.0; // seconds of flight before it blows up by itself
pub const BLAST_R: f64 = 18.0; // ft
pub const MISSILE_DAMAGE: i32 = 2; // bars a blast takes within MISSILE_FULL_R of it
pub const MISSILE_FULL_R: f64 = 10.0; // ft; from here out to BLAST_R it takes 1 bar
pub const MISSILE_HIT_R: f64 = 3.0; // ft; a laser this close to a missile (or drone) shoots it down
pub const DRONE_SPEED: f64 = 40.0; // ft/s, the most a drone can fly
pub const DRONE_LIFE: f64 = 20.0; // seconds of battery
pub const DRONE_BLAST_R: f64 = 20.0; // ft
pub const DRONE_RINGS: [(f64, i32); 3] = [(8.0, 3), (14.0, 2), (DRONE_BLAST_R, 1)]; // within 8 ft it tags you out
pub const FX_SHIELD: u32 = 1;

static T0: OnceLock<Instant> = OnceLock::new();

/// Seconds since the server started.
pub fn clock() -> f64 {
    T0.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// A connection's outgoing queue. If it fills up (a client that stopped reading), the connection is dropped.
pub type Conn = mpsc::Sender<Message>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Missile,
    Drone,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Missile => "missile",
            Kind::Drone => "drone",
        }
    }
}

pub struct Missile {
    pub p: [f64; 3],
    pub d: [f64; 3],
    pub t0: f64,
    pub t: f64,
    pub h: VecDeque<(f64, [f64; 3])>,
    pub k: Kind,
}

impl Missile {
    /// Where it was at server time rt (the shooter sees it a little in the past, like everyone).
    fn at_time(&self, rt: f64) -> [f64; 3] {
        let h = &self.h;
        if rt >= h[h.len() - 1].0 {
            return self.p;
        }
        for k in (1..h.len()).rev() {
            let (t1, t0) = (h[k].0, h[k - 1].0);
            if t0 <= rt && rt <= t1 {
                let f = (rt - t0) / (t1 - t0).max(1e-6);
                return [0, 1, 2].map(|i| h[k - 1].1[i] * (1.0 - f) + h[k].1[i] * f);
            }
        }
        h[0].1
    }
}

#[derive(Default, Clone, Copy)]
pub struct Powers {
    pub rapid: f64,
    pub over: f64,
    pub speed: f64,
}

impl Powers {
    fn set(&mut self, kind: &str, until: f64) {
        match kind {
            "rapid" => self.rapid = until,
            "over" => self.over = until,
            "speed" => self.speed = until,
            _ => {}
        }
    }
}

/// A bot's state of mind.
pub struct Ai {
    pub target: Option<u32>,
    pub recheck: f64,
    pub wp: Option<(f64, f64)>,
    pub wp_until: f64,
    pub strafe: f64,
    pub strafe_t: f64,
    pub next_fire: f64,
    pub stuck_t: f64,
    pub lastpos: (f64, f64),
    pub skill: f64,
}

pub struct Player {
    pub id: u32,
    pub name: String,
    pub color: String,
    pub conn: Option<Conn>,
    pub bot: bool,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f64,
    pub pitch: f64,
    pub hp: i32,
    pub alive: bool,
    pub respawn_at: f64,
    pub shield_until: f64,
    pub powers: Powers,
    pub score: i32,
    pub tags_taken: i32,
    pub streak: i32,
    pub best_streak: i32,
    pub last_fire: f64,
    pub heat: f64,
    pub heat_t: f64,
    pub locked_until: f64,
    pub missile: Option<Missile>,
    pub missile_ready: f64,
    pub drones: i32,
    pub last_hit: f64,
    pub last_regen: f64,
    pub hist: VecDeque<(f64, f64, f64, f64)>,
    pub ai: Option<Ai>,
    pub threat: Option<u32>,
}

impl Player {
    fn flags(&self, t: f64) -> u32 {
        let mut f = if t < self.shield_until { FX_SHIELD } else { 0 };
        if self.powers.rapid > t {
            f |= 2;
        }
        if self.powers.over > t {
            f |= 4;
        }
        if self.powers.speed > t {
            f |= 8;
        }
        f
    }

    /// Where this player stood at server time rt (for judging a shot the way the shooter saw it).
    fn at_time(&self, rt: f64) -> [f64; 3] {
        let h = &self.hist;
        if h.is_empty() || rt >= h[h.len() - 1].0 {
            return [self.x, self.y, self.z];
        }
        for k in (1..h.len()).rev() {
            let (t1, t0) = (h[k].0, h[k - 1].0);
            if t0 <= rt && rt <= t1 {
                let f = (rt - t0) / (t1 - t0).max(1e-6);
                let a = [h[k - 1].1, h[k - 1].2, h[k - 1].3];
                let b = [h[k].1, h[k].2, h[k].3];
                return [0, 1, 2].map(|i| a[i] * (1.0 - f) + b[i] * f);
            }
        }
        [h[0].1, h[0].2, h[0].3]
    }

    pub fn public(&self) -> Public {
        Public {
            id: self.id,
            name: self.name.clone(),
            color: self.color.clone(),
            bot: self.bot,
            score: self.score,
            tagged: self.tags_taken,
            best: self.best_streak,
        }
    }
}

/// Closest approach between the ray o + d s (0..tmax) and the segment a-b: (distance, s).
pub fn seg_ray_dist(o: [f64; 3], u: [f64; 3], tmax: f64, a: [f64; 3], b: [f64; 3]) -> (f64, f64) {
    let v = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let w = [o[0] - a[0], o[1] - a[1], o[2] - a[2]];
    let dot = |p: [f64; 3], q: [f64; 3]| p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
    let aa = 1.0;
    let bb = dot(u, v);
    let cc = dot(v, v);
    let dd = dot(u, w);
    let ee = dot(v, w);
    let den = aa * cc - bb * bb;
    let mut s = if den < 1e-9 { 0.0 } else { (bb * ee - cc * dd) / den };
    s = s.max(0.0).min(tmax);
    let t = ((ee + bb * s) / cc).max(0.0).min(1.0);
    s = (bb * t - dd).max(0.0).min(tmax);
    let p = [o[0] + u[0] * s, o[1] + u[1] * s, o[2] + u[2] * s];
    let q = [a[0] + v[0] * t, a[1] + v[1] * t, a[2] + v[2] * t];
    (dist3(p, q), s)
}

fn r2(v: [f64; 3]) -> [f64; 3] {
    v.map(|x| round_to(x, 2))
}

fn normalise(d: [f64; 3]) -> [f64; 3] {
    let mut l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if l == 0.0 {
        l = 1.0;
    }
    [d[0] / l, d[1] / l, d[2] / l]
}

pub struct Game {
    pub g: Ground,
    pub arena: Arena,
    pub players: BTreeMap<u32, Player>,
    pub round_ends: f64,
    pub intermission_until: f64,
    pub roster_dirty: bool,
    pub round_no: u32,
    pub next_id: u32,
    pub rng: PyRandom,
    pub round_s: f64,
    pub intermission_s: f64,
    pub fill_to: usize,
    blast: Option<&'static str>,
}

impl Game {
    pub fn new(g: Ground, arena: Arena, round_s: f64, intermission_s: f64, fill_to: usize) -> Game {
        Game {
            g,
            arena,
            players: BTreeMap::new(),
            round_ends: clock() + round_s,
            intermission_until: 0.0,
            roster_dirty: false,
            round_no: 1,
            next_id: 1,
            rng: PyRandom::from_time(),
            round_s,
            intermission_s,
            fill_to,
            blast: None,
        }
    }

    fn solids(&self) -> &Solids {
        &self.arena.solids
    }

    pub fn size(&self) -> f64 {
        self.g.size
    }

    pub fn new_player(&mut self, name: String, color: String, conn: Option<Conn>, bot: bool) -> Player {
        let id = self.next_id;
        self.next_id += 1;
        Player {
            id,
            name,
            color,
            conn,
            bot,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            hp: MAX_HP,
            alive: false,
            respawn_at: 0.0,
            shield_until: 0.0,
            powers: Powers::default(),
            score: 0,
            tags_taken: 0,
            streak: 0,
            best_streak: 0,
            last_fire: -9.0,
            heat: 0.0,
            heat_t: 0.0,
            locked_until: 0.0,
            missile: None,
            missile_ready: 0.0,
            drones: 0,
            last_hit: -9.0,
            last_regen: 0.0,
            hist: VecDeque::with_capacity(24),
            ai: None,
            threat: None,
        }
    }

    // -- messaging
    pub fn humans(&self) -> usize {
        self.players.values().filter(|p| !p.bot).count()
    }

    /// Queue a message for one player; a connection whose queue is full is dropped.
    pub fn send(&mut self, id: u32, msg: &ServerMsg) {
        if let Some(p) = self.players.get_mut(&id) {
            if let Some(c) = &p.conn {
                if c.try_send(Message::text(msg.to_json())).is_err() {
                    p.conn = None;
                }
            }
        }
    }

    pub fn broadcast(&mut self, msg: &ServerMsg) {
        let m = Message::text(msg.to_json());
        for p in self.players.values_mut() {
            if let Some(c) = &p.conn {
                if c.try_send(m.clone()).is_err() {
                    p.conn = None; // too slow or gone: closing the queue ends the connection
                }
            }
        }
    }

    pub fn roster(&self) -> ServerMsg {
        ServerMsg::Roster { players: self.players.values().map(|p| p.public()).collect() }
    }

    // -- spawning
    fn spawn_point(&mut self, who: u32) -> (f64, f64) {
        let size = self.size();
        let others: Vec<(f64, f64)> =
            self.players.values().filter(|p| p.alive && p.id != who).map(|p| (p.x, p.z)).collect();
        let mut best: Option<(f64, f64)> = None;
        let mut bestd = -1.0;
        for _ in 0..16 {
            let x = self.rng.uniform(40.0, size - 40.0);
            let z = self.rng.uniform(40.0, size - 40.0);
            if self.g.water(x, z) || self.g.slope(x, z) > 0.6 || self.solids().in_box(x, z, 2.5) {
                continue;
            }
            let (x, z) = self.solids().push_out(x, z, self.g.at(x, z));
            let dmin = others.iter().map(|&(ox, oz)| (x - ox).hypot(z - oz)).fold(1e9, f64::min);
            let dmin = dmin.min(600.0) + self.rng.uniform(0.0, 80.0);
            if dmin > bestd {
                best = Some((x, z));
                bestd = dmin;
            }
        }
        best.unwrap_or((size / 2.0, size / 2.0))
    }

    pub fn spawn(&mut self, id: u32) {
        let (x, z) = self.spawn_point(id);
        let size = self.size();
        let y = self.g.at(x, z);
        let now = clock();
        let Some(p) = self.players.get_mut(&id) else { return };
        p.x = x;
        p.z = z;
        p.y = y;
        p.yaw = py_mod((size / 2.0 - x).atan2(-(size / 2.0 - z)), TAU); // facing the middle
        p.pitch = 0.0;
        p.hp = MAX_HP;
        p.alive = true;
        p.shield_until = now + SHIELD_S;
        p.powers = Powers::default();
        p.drones = 0;
        p.hist.clear();
        p.ai = None;
        p.threat = None;
        p.missile = None;
        let msg = ServerMsg::Spawn { id, p: r2([p.x, p.y, p.z]), yaw: round_to(p.yaw, 3) };
        self.broadcast(&msg);
    }

    // -- joining and leaving
    pub fn add(&mut self, p: Player) {
        let id = p.id;
        self.players.insert(id, p);
        self.spawn(id);
        self.roster_dirty = true;
    }

    pub fn remove(&mut self, id: u32) -> Option<Player> {
        let p = self.players.remove(&id)?;
        self.roster_dirty = true;
        self.broadcast(&ServerMsg::Left { id });
        Some(p)
    }

    pub fn manage_bots(&mut self) {
        let humans = self.humans();
        let mut bots: Vec<u32> = self.players.values().filter(|p| p.bot).map(|p| p.id).collect();
        let want = if humans > 0 { self.fill_to.saturating_sub(humans) } else { 0 };
        while bots.len() < want {
            let used: Vec<String> = self.players.values().map(|p| p.name.clone()).collect();
            let name = match BOT_NAMES.iter().find(|n| !used.iter().any(|u| u == *n)) {
                Some(n) => n.to_string(),
                None => format!("Bot {}", self.rng.randint(10, 99)),
            };
            let used_c: Vec<String> = self.players.values().map(|p| p.color.clone()).collect();
            let mut best: Option<(usize, f64, &str)> = None;
            for c in PALETTE {
                let key = (used_c.iter().filter(|u| u.as_str() == c).count(), self.rng.random());
                if best.map_or(true, |b| (key.0, key.1) < (b.0, b.1)) {
                    best = Some((key.0, key.1, c));
                }
            }
            let color = best.map(|b| b.2).unwrap_or(PALETTE[0]).to_string();
            let b = self.new_player(name.clone(), color, None, true);
            let id = b.id;
            self.add(b);
            bots.push(id);
            self.broadcast(&ServerMsg::Feed { msg: format!("{} waded in", name) });
        }
        while bots.len() > want {
            if let Some(id) = bots.pop() {
                self.remove(id);
            }
        }
    }

    // -- shooting
    pub fn fire(&mut self, sid: u32, o: [f64; 3], d: [f64; 3], rt: f64) {
        let t = clock();
        let intermission = t < self.intermission_until;
        let Some(sh) = self.players.get_mut(&sid) else { return };
        if !sh.alive || intermission {
            return;
        }
        let rapid = sh.powers.rapid > t;
        let cd = if rapid { RAPID_CD } else { FIRE_CD };
        if t - sh.last_fire < cd * 0.75 || t < sh.locked_until {
            return;
        }
        sh.heat = (sh.heat - (t - sh.heat_t) * HEAT_COOL).max(0.0);
        sh.heat_t = t;
        // a little slack against the client's own count, so a fair shot is never refused
        sh.heat += (if rapid { HEAT_SHOT_RAPID } else { HEAT_SHOT }) * 0.7;
        if sh.heat >= 1.0 {
            sh.heat = 0.0;
            sh.locked_until = t + OVERHEAT_S * 0.85;
        }
        sh.last_fire = t;
        let eye = [sh.x, sh.y + EYE, sh.z];
        let over = sh.powers.over > t;
        let o = if dist3(o, eye) > 14.0 { eye } else { o };
        let d = normalise(d);
        let rt = rt.max(t - 0.5).min(t); // judge the shot as the shooter saw it, up to half a second back
        let mut tend = RANGE;
        if let Some(tg) = self.g.ray(o, d, tend) {
            tend = tg;
        }
        if let Some(tp) = self.solids().ray(o, d, tend) {
            tend = tp;
        }
        let (mut hit, mut hit_t): (Option<u32>, f64) = (None, tend);
        for p in self.players.values() {
            if p.id == sid || !p.alive {
                continue;
            }
            let [x, y, z] = p.at_time(rt);
            let (dist, s) = seg_ray_dist(o, d, hit_t, [x, y + BODY.0, z], [x, y + BODY.1, z]);
            if dist < HIT_R && s < hit_t {
                hit = Some(p.id);
                hit_t = s;
            }
        }
        // someone else's missile in the way: the laser shoots it down
        let mut downed: Option<u32> = None;
        for p in self.players.values() {
            let Some(m) = &p.missile else { continue };
            if p.id == sid {
                continue;
            }
            let c = m.at_time(rt);
            let v = [c[0] - o[0], c[1] - o[1], c[2] - o[2]];
            let s = v[0] * d[0] + v[1] * d[1] + v[2] * d[2];
            if 0.0 < s && s < hit_t && dist3(v, [d[0] * s, d[1] * s, d[2] * s]) < MISSILE_HIT_R {
                downed = Some(p.id);
                hit = None;
                hit_t = s;
            }
        }
        let end = [o[0] + d[0] * hit_t, o[1] + d[1] * hit_t, o[2] + d[2] * hit_t];
        self.broadcast(&ServerMsg::Beam {
            by: sid,
            o: r2(o),
            e: r2(end),
            hit: hit.unwrap_or(0),
            over: over as u8,
            mhit: downed.unwrap_or(0),
        });
        if let Some(owner) = downed {
            let at = self.players[&owner].missile.as_ref().map(|m| m.p).unwrap_or(end);
            self.shoot_down(owner, sid, at);
        }
        if let Some(v) = hit {
            self.damage(v, sid, if over { MAX_HP } else { 1 });
        }
    }

    /// A missile shot out of the sky: it goes off where it is, harmlessly.
    fn shoot_down(&mut self, owner: u32, by: u32, at: [f64; 3]) {
        let Some(p) = self.players.get_mut(&owner) else { return };
        let Some(m) = p.missile.take() else { return };
        if m.k == Kind::Missile {
            p.missile_ready = clock() + MISSILE_CD;
        }
        self.broadcast(&ServerMsg::Boom { by: owner, k: m.k.name().into(), p: r2(at), fizzle: 0, down: Some(by) });
    }

    // -- missiles: the owner flies it (their client steers and says where it is); the server judges the blast
    pub fn launch(&mut self, id: u32, d: [f64; 3], kind: Kind) {
        let t = clock();
        let inter = self.intermission_until != 0.0;
        let Some(p) = self.players.get_mut(&id) else { return };
        if !p.alive || p.missile.is_some() || inter {
            return;
        }
        if kind == Kind::Drone {
            if p.drones < 1 {
                return;
            }
            p.drones -= 1;
        } else if t < p.missile_ready {
            return;
        }
        let d = normalise(d);
        let o = [p.x + d[0] * 3.0, p.y + EYE + d[1] * 3.0, p.z + d[2] * 3.0];
        let mut h = VecDeque::with_capacity(24);
        h.push_back((t, o));
        p.missile = Some(Missile { p: o, d, t0: t, t, h, k: kind });
        self.broadcast(&ServerMsg::Launch { by: id, k: kind.name().into(), p: r2(o) });
    }

    pub fn missile_at(&mut self, id: u32, o: [f64; 3], d: [f64; 3]) {
        let t = clock();
        let Some(p) = self.players.get_mut(&id) else { return };
        let Some(m) = &mut p.missile else { return };
        let speed = if m.k == Kind::Missile { MISSILE_SPEED } else { DRONE_SPEED };
        if dist3(o, m.p) > speed * (t - m.t) * 1.8 + 10.0 {
            return; // too far, too fast: keep the last good place
        }
        m.p = o;
        m.d = d;
        m.t = t;
        if m.h.len() == 24 {
            m.h.pop_front();
        }
        m.h.push_back((t, o));
    }

    pub fn explode(&mut self, id: u32, at: Option<[f64; 3]>, fizzle: bool) {
        let Some(p) = self.players.get_mut(&id) else { return };
        let Some(m) = p.missile.take() else { return };
        let kind = m.k;
        if kind == Kind::Missile {
            p.missile_ready = clock() + MISSILE_CD; // recharges from when it goes off
        }
        let o = match at {
            Some(a) if dist3(a, m.p) <= 25.0 => a,
            _ => m.p,
        };
        self.broadcast(&ServerMsg::Boom { by: id, k: kind.name().into(), p: r2(o), fizzle: fizzle as u8, down: None });
        if fizzle {
            return;
        }
        let reach = if kind == Kind::Drone { DRONE_BLAST_R } else { BLAST_R };
        self.blast = Some(if kind == Kind::Drone { "d" } else { "m" });
        let ids: Vec<u32> = self.players.keys().copied().collect();
        for qid in ids {
            let Some(q) = self.players.get(&qid) else { continue };
            if !q.alive {
                continue;
            }
            let c = [q.x, q.y + 3.0, q.z];
            let dist = dist3(o, c);
            if dist >= reach {
                continue;
            }
            if dist > 1.0 {
                let dd = [0, 1, 2].map(|i| (c[i] - o[i]) / dist);
                if self.g.ray(o, dd, dist - 0.8).is_some() || self.solids().ray(o, dd, dist - 0.8).is_some() {
                    continue;
                }
            }
            let n = if kind == Kind::Drone {
                DRONE_RINGS.iter().find(|(r, _)| dist < *r).map(|(_, n)| *n).unwrap_or(1)
            } else if dist < MISSILE_FULL_R {
                MISSILE_DAMAGE
            } else {
                1
            };
            self.damage(qid, id, n);
        }
        self.blast = None;
    }

    pub fn damage(&mut self, vid: u32, by: u32, n: i32) {
        let t = clock();
        let w = self.blast.unwrap_or("l").to_string();
        let Some(v) = self.players.get_mut(&vid) else { return };
        if t < v.shield_until {
            self.broadcast(&ServerMsg::Deflect { id: vid, by });
            return;
        }
        v.hp -= n;
        v.last_hit = t;
        v.last_regen = t;
        if v.hp > 0 {
            let hp = v.hp;
            if v.bot {
                v.threat = Some(by);
            }
            self.broadcast(&ServerMsg::Hurt { id: vid, by, hp, w });
            return;
        }
        v.alive = false;
        v.hp = 0;
        v.respawn_at = t + RESPAWN_S;
        v.tags_taken += 1;
        v.streak = 0;
        let had_missile = v.missile.is_some();
        let mut streak = 0;
        if let Some(k) = self.players.get_mut(&by) {
            if vid != by {
                k.score += 1;
                k.streak += 1;
                k.best_streak = k.best_streak.max(k.streak);
            }
            streak = k.streak;
        }
        self.roster_dirty = true;
        self.broadcast(&ServerMsg::Tag { v: vid, k: by, streak, w });
        if had_missile {
            self.explode(vid, None, true);
        }
    }

    // -- every tick
    pub fn tick(&mut self, dt: f64) {
        let t = clock();
        if self.intermission_until != 0.0 && t >= self.intermission_until {
            self.intermission_until = 0.0;
            self.new_round();
        } else if self.intermission_until == 0.0 && t >= self.round_ends {
            self.end_round();
        }
        let ids: Vec<u32> = self.players.keys().copied().collect();
        for &id in &ids {
            let Some(p) = self.players.get_mut(&id) else { continue };
            if !p.alive {
                if t >= p.respawn_at && self.intermission_until == 0.0 {
                    self.spawn(id);
                }
                continue;
            }
            if p.hp < MAX_HP && t - p.last_hit > REGEN_AFTER && t - p.last_regen > REGEN_EVERY {
                p.hp += 1;
                p.last_regen = t;
            }
            let (px, py, pz) = (p.x, p.y, p.z);
            for si in 0..self.arena.spots.len() {
                let s = &mut self.arena.spots[si];
                if s.up && (px - s.x).abs() < 5.0 && (pz - s.z).abs() < 5.0 && (py - s.y).abs() < 8.0 {
                    s.up = false;
                    s.back = t + PICKUP_RESPAWN;
                    let (i, kind) = (s.i, s.kind.clone());
                    let p = self.players.get_mut(&id).unwrap();
                    if kind == "drone" {
                        p.drones = 1;
                    } else {
                        p.powers.set(&kind, t + POWER_S);
                    }
                    let until = if kind == "drone" { 0.0 } else { POWER_S };
                    self.broadcast(&ServerMsg::Pick { i, id, kind, until });
                }
            }
        }
        for si in 0..self.arena.spots.len() {
            let s = &mut self.arena.spots[si];
            if !s.up && t >= s.back {
                s.up = true;
                let i = s.i;
                self.broadcast(&ServerMsg::Spot { i, up: true });
            }
        }
        for &id in &ids {
            let expired = match self.players.get(&id).and_then(|p| p.missile.as_ref()) {
                Some(m) => t - m.t0 > (if m.k == Kind::Drone { DRONE_LIFE } else { MISSILE_LIFE }) + 1.0,
                None => false,
            };
            if expired {
                self.explode(id, None, false);
            }
        }
        for &id in &ids {
            let Some(p) = self.players.get(&id) else { continue };
            if p.bot {
                crate::bots::think(self, id, dt, t);
            }
            if let Some(p) = self.players.get_mut(&id) {
                if p.hist.len() == 24 {
                    p.hist.pop_front();
                }
                p.hist.push_back((t, p.x, p.y, p.z));
            }
        }
        if self.roster_dirty {
            self.roster_dirty = false;
            let r = self.roster();
            self.broadcast(&r);
        }
        let snap: Vec<SnapPlayer> = self
            .players
            .values()
            .map(|p| {
                (
                    p.id,
                    round_to(p.x, 2),
                    round_to(p.y, 2),
                    round_to(p.z, 2),
                    round_to(p.yaw, 3),
                    round_to(p.pitch, 3),
                    p.hp,
                    p.alive as u8,
                    p.flags(t),
                )
            })
            .collect();
        let ms: Vec<SnapMissile> = self
            .players
            .values()
            .filter_map(|p| {
                p.missile.as_ref().map(|m| {
                    (
                        p.id,
                        round_to(m.p[0], 2),
                        round_to(m.p[1], 2),
                        round_to(m.p[2], 2),
                        round_to(m.d[0], 3),
                        round_to(m.d[1], 3),
                        round_to(m.d[2], 3),
                        (m.k == Kind::Drone) as u8,
                    )
                })
            })
            .collect();
        self.broadcast(&ServerMsg::Snap { ts: round_to(t, 3), p: snap, m: ms });
    }

    fn standings(&self) -> Vec<Public> {
        let mut st: Vec<&Player> = self.players.values().collect();
        st.sort_by_key(|p| (-p.score, p.tags_taken)); // stable, like Python's sorted
        st.iter().map(|p| p.public()).collect()
    }

    fn end_round(&mut self) {
        let st = self.standings();
        self.intermission_until = clock() + self.intermission_s;
        let winner = st.first().filter(|p| p.score != 0).map(|p| p.id).unwrap_or(0);
        self.broadcast(&ServerMsg::RoundEnd { winner, board: st, next: self.intermission_s });
    }

    fn new_round(&mut self) {
        self.round_no += 1;
        self.round_ends = clock() + self.round_s;
        let ids: Vec<u32> = self.players.keys().copied().collect();
        for id in ids {
            if let Some(p) = self.players.get_mut(&id) {
                p.score = 0;
                p.tags_taken = 0;
                p.streak = 0;
                p.best_streak = 0;
                p.missile = None;
                p.missile_ready = 0.0;
            }
            self.spawn(id);
        }
        for s in &mut self.arena.spots {
            s.up = true;
        }
        self.roster_dirty = true;
        let msg = ServerMsg::Round { left: self.round_s, no: self.round_no, spots: self.arena.spots.clone() };
        self.broadcast(&msg);
    }

    pub fn welcome(&self, id: u32) -> ServerMsg {
        let t = clock();
        ServerMsg::Welcome {
            id,
            ts: round_to(t, 3),
            pillars: self.arena.pillars.clone(),
            spots: self.arena.spots.clone(),
            boxes: self.arena.boxes.clone(),
            windows: self.arena.windows.clone(),
            left: round_to((self.round_ends - t).max(0.0), 1),
            inter: round_to((self.intermission_until - t).max(0.0), 1),
            no: self.round_no,
            rules: Rules {
                hp: MAX_HP,
                cd: FIRE_CD,
                rapid: RAPID_CD,
                range: RANGE,
                eye: EYE,
                heat: HEAT_SHOT,
                heat_rapid: HEAT_SHOT_RAPID,
                cool: HEAT_COOL,
                lock: OVERHEAT_S,
                respawn: RESPAWN_S,
                mcd: MISSILE_CD,
                mspeed: MISSILE_SPEED,
                mlife: MISSILE_LIFE,
                blast: BLAST_R,
                dspeed: DRONE_SPEED,
                dlife: DRONE_LIFE,
                dblast: DRONE_BLAST_R,
            },
        }
    }

    pub fn spots(&self) -> &[Spot] {
        &self.arena.spots
    }
}

/// `(a - b + π) mod 2π - π`: the shortest turn from b to a.
pub fn wrap_angle(a: f64) -> f64 {
    py_mod(a + PI, TAU) - PI
}
