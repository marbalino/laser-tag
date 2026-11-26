//! The game in the browser: your own movement, shooting, missiles and drones; the picture of everyone else from
//! the server's snapshots (drawn 100 ms behind, smoothly); and the effects, sounds and HUD. The server decides who
//! gets tagged.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::f64::consts::PI;

use glam::{Mat4, Vec3};
use lasertag_shared::protocol::{ClientMsg, Rules, ServerMsg, Spot};
use lasertag_shared::solids::Pillar;
use lasertag_shared::{round_to, Ground, Solids, EYE};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext as GL, WebGlTexture, WebSocket};

use crate::audio::Sfx;
use crate::dom::{self, el, now_s};
use crate::hud::{self, n, t, Hud, Mini, MiniPerson, Part, Row};
use crate::render::{hex, hexu, Blend, Camera, Frame, Mat, Renderer, Shape, Sprite};
use crate::world::{self, power_color, World};

pub const GRAV: f64 = 64.0;
pub const JUMP1: f64 = 22.0;
pub const JUMP2: f64 = 20.0;
pub const INTERP: f64 = 0.1;
pub const FOV: f64 = 75.0;
pub const FOV_ZOOM: f64 = 25.0;

fn power_name(k: &str) -> &'static str {
    match k {
        "rapid" => "Rapid",
        "over" => "Overcharge",
        "speed" => "Speed",
        _ => "Drone",
    }
}

fn v3(a: [f64; 3]) -> Vec3 {
    Vec3::new(a[0] as f32, a[1] as f32, a[2] as f32)
}

fn r2(v: [f64; 3]) -> [f64; 3] {
    v.map(|x| round_to(x, 2))
}

fn r4(v: [f64; 3]) -> [f64; 3] {
    v.map(|x| round_to(x, 4))
}

fn hyp3(a: f64, b: f64, c: f64) -> f64 {
    (a * a + b * b + c * c).sqrt()
}

pub fn dir_of(yaw: f64, pitch: f64) -> [f64; 3] {
    [yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos()]
}

fn lerp_angle(a: f64, b: f64, f: f64) -> f64 {
    let d = (b - a + PI * 3.0).rem_euclid(PI * 2.0) - PI;
    a + d * f
}

#[derive(Clone, Copy)]
struct Sample {
    ts: f64,
    p: [f64; 3],
    yaw: f64,
    pitch: f64,
}

pub struct Person {
    pub id: u32,
    pub name: String,
    pub color: String,
    pub bot: bool,
    pub score: i32,
    pub tagged: i32,
    pub best: i32,
    has_mesh: bool,
    key: String,
    tex: Option<WebGlTexture>,
    buf: VecDeque<Sample>,
    pub alive: bool,
    pub hp: i32,
    pub flags: u32,
    pub pos: [f64; 3],
    yaw: f64,
    pitch: f64,
    upper_y: f64,
    pub visible: bool,
}

impl Person {
    fn new(id: u32) -> Person {
        Person {
            id,
            name: "?".into(),
            color: "#ffffff".into(),
            bot: false,
            score: 0,
            tagged: 0,
            best: 0,
            has_mesh: false,
            key: String::new(),
            tex: None,
            buf: VecDeque::new(),
            alive: false,
            hp: 3,
            flags: 0,
            pos: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            upper_y: 4.4,
            visible: false,
        }
    }
    /// Shown on screen right now.
    fn seen(&self) -> bool {
        self.has_mesh && self.visible && self.alive
    }
}

#[derive(Default)]
pub struct Powers {
    pub rapid: f64,
    pub over: f64,
    pub speed: f64,
}

impl Powers {
    fn get(&self, k: &str) -> f64 {
        match k {
            "rapid" => self.rapid,
            "over" => self.over,
            "speed" => self.speed,
            _ => 0.0,
        }
    }
    fn set(&mut self, k: &str, v: f64) {
        match k {
            "rapid" => self.rapid = v,
            "over" => self.over = v,
            "speed" => self.speed = v,
            _ => {}
        }
    }
}

#[derive(Default)]
pub struct Me {
    pub id: u32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub vx: f64,
    pub vy: f64,
    pub vz: f64,
    pub yaw: f64,
    pub pitch: f64,
    pub on_ground: bool,
    pub jumps: u8,
    pub hp: i32,
    pub alive: bool,
    pub flags: u32,
    pub last_fire: f64,
    pub dead_at: f64,
    pub killer: u32,
    pub bob: f64,
    pub power_until: Powers,
    pub drones: i32,
}

#[derive(Default)]
struct Heat {
    v: f64,
    t: f64,
    lock_until: f64,
    warned: bool,
}

/// Your own missile or drone, which you fly (and become).
#[derive(Default)]
pub struct Ms {
    pub active: bool,
    pub drone: bool,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    vx: f64,
    vy: f64,
    vz: f64,
    pub yaw: f64,
    pub pitch: f64,
    t0: f64,
    send_t: f64,
    pub ready_at: f64,
    lock: Option<u32>,
    end_at: f64,
    cam: [f64; 3],
}

#[derive(Clone, Copy)]
struct MSample {
    ts: f64,
    p: [f64; 3],
    d: [f64; 3],
}

/// Someone else's missile or drone in the air.
struct RMis {
    buf: VecDeque<MSample>,
    drone: bool,
    color: String,
    pos: [f64; 3],
    dir: [f64; 3],
    trail: Vec<[f32; 3]>,
}

struct BeamFx {
    a: Vec3,
    b: Vec3,
    color: Vec3,
    thick: f32,
    life: f64,
}

struct Burst {
    pts: Vec<([f64; 3], [f64; 3])>,
    color: Vec3,
    life: f64,
    max: f64,
}

struct Blast {
    p: Vec3,
    color: Vec3,
    r: f64,
    op: f32,
    life: f64,
}

#[derive(Default)]
pub struct TouchState {
    pub mx: f64,
    pub mz: f64,
    pub stick_id: Option<i32>,
    pub look_id: Option<i32>,
    pub fire_id: Option<i32>,
    pub lx: f64,
    pub ly: f64,
    pub fx: f64,
    pub fy: f64,
    pub cx: f64,
    pub cy: f64,
}

pub struct App {
    pub canvas: HtmlCanvasElement,
    pub r: Renderer,
    world: World,
    pub ground: Ground,
    solids: Solids,
    pillars: Vec<Pillar>,
    spots: Vec<Spot>,
    mini: Mini,
    pub hud: Hud,
    pub sfx: Sfx,
    pub cam: Camera,
    pub touch_mode: bool,
    pub glow_on: bool,
    pub sens: f64,
    pub me: Me,
    pub people: BTreeMap<u32, Person>,
    ws: Option<WebSocket>,
    ws_cbs: Vec<Closure<dyn FnMut(JsValue)>>,
    joined: bool,
    rules: Rules,
    offset: Option<f64>,
    round_ends: f64,
    inter_until: f64,
    pub playing: bool,
    pub paused: bool,
    pub keys: HashSet<String>,
    pub firing: bool,
    pub zoom_held: bool,
    pub zoom_toggle: bool,
    zoom: f64,
    heat: Heat,
    pub touch: TouchState,
    pub ms: Ms,
    rmis: BTreeMap<u32, RMis>,
    beams: Vec<BeamFx>,
    bursts: Vec<Burst>,
    blasts: Vec<Blast>,
    shake: f64,
    last: f64,
    send_t: f64,
    mini_t: f64,
    hud_t: f64,
    shown_rt: Option<f64>,
    gun_kick: f64,
    gun_visible: bool,
    pub color: String,
    pixel_ratio: f64,
}

fn ws_url() -> String {
    let loc = dom::window().location();
    let proto = if loc.protocol().unwrap_or_default() == "https:" { "wss://" } else { "ws://" };
    let host = loc.host().unwrap_or_default();
    let path = loc.pathname().unwrap_or_default();
    let dir = match path.rfind('/') {
        Some(i) => &path[..=i],
        None => "/",
    };
    format!("{}{}{}ws", proto, host, dir)
}

impl App {
    pub fn new(canvas: HtmlCanvasElement, ground: Ground, touch_mode: bool) -> Result<App, String> {
        let gl = crate::gl::context(&canvas)?;
        let glow_on = dom::load("glow", !touch_mode);
        let r = Renderer::new(gl, !touch_mode, glow_on)?;
        let world = World::new(&r, &ground);
        let mini = Mini::new(&ground);
        let size = ground.size;
        let cam = Camera { pos: Vec3::ZERO, view: Mat4::IDENTITY, proj: Mat4::IDENTITY, fov: FOV as f32, aspect: 1.0 };
        let me = Me { x: size / 2.0, z: size / 2.0, hp: 3, last_fire: -9.0, ..Default::default() };
        let mut app = App {
            canvas,
            r,
            world,
            solids: Solids { pillars: Vec::new(), boxes: Vec::new(), size },
            ground,
            pillars: Vec::new(),
            spots: Vec::new(),
            mini,
            hud: Hud::new(),
            sfx: Sfx::default(),
            cam,
            touch_mode,
            glow_on,
            sens: dom::load("sens", 1.0),
            me,
            people: BTreeMap::new(),
            ws: None,
            ws_cbs: Vec::new(),
            joined: false,
            rules: Rules::default(),
            offset: None,
            round_ends: 0.0,
            inter_until: 0.0,
            playing: false,
            paused: false,
            keys: HashSet::new(),
            firing: false,
            zoom_held: false,
            zoom_toggle: false,
            zoom: 0.0,
            heat: Heat::default(),
            touch: TouchState::default(),
            ms: Ms::default(),
            rmis: BTreeMap::new(),
            beams: Vec::new(),
            bursts: Vec::new(),
            blasts: Vec::new(),
            shake: 0.0,
            last: now_s(),
            send_t: 0.0,
            mini_t: 0.0,
            hud_t: 0.0,
            shown_rt: None,
            gun_kick: 0.0,
            gun_visible: true,
            color: String::new(),
            pixel_ratio: 1.0,
        };
        app.resize();
        Ok(app)
    }

    pub fn size(&self) -> f64 {
        self.ground.size
    }

    fn server_now(&self) -> f64 {
        now_s() + self.offset.unwrap_or(0.0)
    }

    pub fn resize(&mut self) {
        let (w, h) = dom::inner_size();
        let pr = dom::device_pixel_ratio().min(if self.glow_on { 1.5 } else { 1.25 });
        self.pixel_ratio = pr;
        let (pw, ph) = ((w * pr).round() as u32, (h * pr).round() as u32);
        self.canvas.set_width(pw.max(1));
        self.canvas.set_height(ph.max(1));
        self.r.resize(pw as i32, ph as i32);
        self.cam.aspect = (w / h.max(1.0)) as f32;
        self.cam.set_proj();
    }

    pub fn set_glow(&mut self, on: bool) {
        self.glow_on = on;
        self.r.glow = on;
        dom::save("glow", &on);
        self.resize();
    }

    // ------------------------------------------------------------------ network
    pub fn send(&self, m: &ClientMsg) {
        if let Some(ws) = &self.ws {
            if ws.ready_state() == 1 {
                let _ = ws.send_with_str(&m.to_json());
            }
        }
    }

    fn drop_socket(&mut self) {
        if let Some(ws) = self.ws.take() {
            ws.set_onopen(None);
            ws.set_onmessage(None);
            ws.set_onerror(None);
            ws.set_onclose(None);
            let _ = ws.close();
        }
        self.ws_cbs.clear();
    }

    pub fn connect(&mut self, name: String, color: String) {
        self.drop_socket();
        self.joined = false;
        let ws = match WebSocket::new(&ws_url()) {
            Ok(ws) => ws,
            Err(_) => return self.join_failed("Could not reach the game server."),
        };
        let open = Closure::<dyn FnMut(JsValue)>::new(move |_| {
            crate::with(|a| a.send(&ClientMsg::Join { name: name.clone(), color: color.clone() }));
        });
        let msg = Closure::<dyn FnMut(JsValue)>::new(move |e: JsValue| {
            let Ok(e) = e.dyn_into::<web_sys::MessageEvent>() else { return };
            let Some(s) = e.data().as_string() else { return };
            match serde_json::from_str::<ServerMsg>(&s) {
                Ok(m) => {
                    crate::with(|a| a.on_message(m));
                }
                Err(err) => dom::log(&format!("bad message: {} {}", err, &s[..s.len().min(120)])),
            }
        });
        let err = Closure::<dyn FnMut(JsValue)>::new(move |_| {
            crate::with(|a| {
                if !a.joined {
                    a.join_failed("Could not reach the game server.");
                }
            });
        });
        let close = Closure::<dyn FnMut(JsValue)>::new(move |_| {
            crate::with(|a| {
                if a.joined && a.playing {
                    a.disconnected();
                } else if !a.joined {
                    a.join_failed("Could not reach the game server.");
                }
            });
        });
        ws.set_onopen(Some(open.as_ref().unchecked_ref()));
        ws.set_onmessage(Some(msg.as_ref().unchecked_ref()));
        ws.set_onerror(Some(err.as_ref().unchecked_ref()));
        ws.set_onclose(Some(close.as_ref().unchecked_ref()));
        self.ws_cbs = vec![open, msg, err, close];
        self.ws = Some(ws);
    }

    fn join_failed(&mut self, why: &str) {
        if self.joined {
            return;
        }
        let err = el("err");
        if err.text_content().unwrap_or_default().is_empty() || why.contains("full") {
            err.set_text_content(Some(why));
        }
        let play: web_sys::HtmlButtonElement = el("play").unchecked_into();
        play.set_disabled(false);
        if let Some(ws) = &self.ws {
            ws.set_onclose(None);
            ws.set_onerror(None);
        }
    }

    /// The server said welcome: into the arena.
    fn joined_ok(&mut self) {
        self.joined = true;
        self.playing = true;
        self.paused = false;
        dom::hidden("lobby", true);
        dom::hidden("hud", false);
        if self.touch_mode {
            self.set_touch_mode(true);
            let de = dom::doc().document_element();
            if let Some(de) = de {
                let _ = de.request_fullscreen();
            }
        } else {
            self.lock();
        }
        self.hud_hp();
        self.hud_top();
    }

    pub fn disconnected(&mut self) {
        self.playing = false;
        dom::doc().exit_pointer_lock();
        el("err").set_text_content(Some("Lost connection to the game. Press Play to rejoin."));
        dom::hidden("hud", true);
        dom::hidden("touch", true);
        dom::hidden("pause", true);
        dom::hidden("lobby", false);
        let play: web_sys::HtmlButtonElement = el("play").unchecked_into();
        play.set_disabled(false);
        for (_, p) in std::mem::take(&mut self.people) {
            if let Some(t) = p.tex {
                self.r.gl.delete_texture(Some(&t));
            }
        }
        self.rmis.clear();
        self.ms.active = false;
        self.ms.end_at = 0.0;
        dom::hidden("mcam", true);
        dom::hidden("lockbox", true);
    }

    pub fn leave(&mut self) {
        self.playing = false;
        self.drop_socket();
        self.disconnected();
        el("err").set_text_content(Some(""));
    }

    pub fn play(&mut self) {
        let input: web_sys::HtmlInputElement = el("name").unchecked_into();
        let mut name: String = input.value().trim().chars().take(16).collect();
        if name.is_empty() {
            name = "Player".into();
        }
        dom::save("name", &name);
        dom::save("color", &self.color);
        el("err").set_text_content(Some(""));
        let play: web_sys::HtmlButtonElement = el("play").unchecked_into();
        play.set_disabled(true);
        self.sfx.wake();
        let color = self.color.clone();
        self.connect(name, color);
    }

    fn person(people: &mut BTreeMap<u32, Person>, id: u32) -> &mut Person {
        people.entry(id).or_insert_with(|| Person::new(id))
    }

    /// Make (or remake, when the name or colour changed) a person's figure and name tag.
    fn ensure_mesh(&mut self, id: u32) {
        if id == self.me.id {
            return;
        }
        let gl = self.r.gl.clone();
        let Some(p) = self.people.get_mut(&id) else { return };
        let key = format!("{}{}", p.name, p.color);
        if p.has_mesh && p.key == key {
            return;
        }
        if let Some(t) = p.tex.take() {
            gl.delete_texture(Some(&t));
        }
        let c = hud::label_canvas(&p.name, &p.color);
        let tex = gl.create_texture();
        gl.bind_texture(GL::TEXTURE_2D, tex.as_ref());
        let _ = gl.tex_image_2d_with_u32_and_u32_and_html_canvas_element(GL::TEXTURE_2D, 0, GL::SRGB8_ALPHA8 as i32, GL::RGBA, GL::UNSIGNED_BYTE, &c);
        gl.generate_mipmap(GL::TEXTURE_2D);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MIN_FILTER, GL::LINEAR_MIPMAP_LINEAR as i32);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_MAG_FILTER, GL::LINEAR as i32);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_S, GL::CLAMP_TO_EDGE as i32);
        gl.tex_parameteri(GL::TEXTURE_2D, GL::TEXTURE_WRAP_T, GL::CLAMP_TO_EDGE as i32);
        p.tex = tex;
        p.key = key;
        p.has_mesh = true;
        p.visible = false;
    }

    fn drop_person(&mut self, id: u32) {
        if let Some(p) = self.people.remove(&id) {
            if let Some(t) = p.tex {
                self.r.gl.delete_texture(Some(&t));
            }
        }
    }

    fn sound_pan(&self, x: f64, z: f64) -> (f64, f64) {
        let d = (x - self.me.x).hypot(z - self.me.z);
        let pan = ((x - self.me.x).atan2(-(z - self.me.z)) - self.me.yaw).sin();
        (d, pan)
    }

    fn my_color(&self) -> String {
        self.people.get(&self.me.id).map(|p| p.color.clone()).unwrap_or_else(|| "#22f0ff".into())
    }

    fn buzz(&self, pattern: &[u32]) {
        if !self.touch_mode {
            return;
        }
        let nav = dom::window().navigator();
        if pattern.len() == 1 {
            nav.vibrate_with_duration(pattern[0]);
        } else {
            let arr = js_sys::Array::new();
            for p in pattern {
                arr.push(&JsValue::from(*p));
            }
            nav.vibrate_with_pattern(&arr);
        }
    }

    pub fn on_message(&mut self, m: ServerMsg) {
        match m {
            ServerMsg::Welcome { id, ts, pillars, spots, boxes, windows, left, inter, rules, .. } => {
                self.me.id = id;
                self.rules = rules;
                self.offset = Some(ts - now_s());
                self.solids = Solids::new(pillars.clone(), &boxes, self.size());
                self.pillars = pillars;
                self.spots = spots;
                self.world.build_ruins(&self.r, &boxes, &windows);
                self.round_ends = now_s() + left;
                self.inter_until = if inter > 0.0 { now_s() + inter } else { 0.0 };
                self.joined_ok();
            }
            ServerMsg::Full {} => self.join_failed("The arena is full (16 players). Try again in a bit."),
            ServerMsg::Roster { players } => {
                let mut seen = HashSet::new();
                for r in players {
                    let p = App::person(&mut self.people, r.id);
                    p.name = r.name;
                    p.color = r.color;
                    p.bot = r.bot;
                    p.score = r.score;
                    p.tagged = r.tagged;
                    p.best = r.best;
                    seen.insert(r.id);
                    self.ensure_mesh(r.id);
                }
                let gone: Vec<u32> = self.people.keys().filter(|k| !seen.contains(k)).copied().collect();
                for id in gone {
                    self.drop_person(id);
                }
                self.hud_top();
                if !dom::is_hidden("board") {
                    self.show_board(true);
                }
            }
            ServerMsg::Left { id } => self.drop_person(id),
            ServerMsg::Snap { ts, p, m } => {
                let sample = ts - now_s();
                match self.offset {
                    Some(o) if sample <= o => self.offset = Some(o + (sample - o) * 0.002),
                    _ => self.offset = Some(sample),
                }
                for (id, x, y, z, yaw, pitch, hp, alive, flags) in p {
                    let me_id = self.me.id;
                    let pp = App::person(&mut self.people, id);
                    pp.hp = hp;
                    pp.flags = flags;
                    if id == me_id {
                        if hp != self.me.hp {
                            self.me.hp = hp;
                            self.hud_hp();
                        }
                        self.me.flags = flags;
                        continue;
                    }
                    pp.alive = alive != 0;
                    pp.buf.push_back(Sample { ts, p: [x, y, z], yaw, pitch });
                    if pp.buf.len() > 40 {
                        pp.buf.pop_front();
                    }
                }
                let mut live = HashSet::new();
                for (id, x, y, z, dx, dy, dz, kd) in m {
                    if id == self.me.id {
                        continue;
                    }
                    live.insert(id);
                    let drone = kd != 0;
                    if self.rmis.get(&id).map_or(false, |r| r.drone != drone) {
                        self.rmis.remove(&id);
                    }
                    let col = self.people.get(&id).map(|p| p.color.clone()).unwrap_or_else(|| "#fff".into());
                    let r = self.rmis.entry(id).or_insert_with(|| RMis {
                        buf: VecDeque::new(),
                        drone,
                        color: col,
                        pos: [x, y, z],
                        dir: [dx, dy, dz],
                        trail: Vec::new(),
                    });
                    r.buf.push_back(MSample { ts, p: [x, y, z], d: [dx, dy, dz] });
                    if r.buf.len() > 30 {
                        r.buf.pop_front();
                    }
                }
                self.rmis.retain(|id, _| live.contains(id));
            }
            ServerMsg::Spawn { id, p, yaw } => {
                if id == self.me.id {
                    let me = &mut self.me;
                    me.x = p[0];
                    me.y = p[1];
                    me.z = p[2];
                    me.yaw = yaw;
                    me.pitch = 0.0;
                    me.vx = 0.0;
                    me.vy = 0.0;
                    me.vz = 0.0;
                    me.alive = true;
                    me.hp = self.rules.hp;
                    me.power_until = Powers::default();
                    me.drones = 0;
                    self.heat.v = 0.0;
                    self.heat.lock_until = 0.0;
                    self.hud_hp();
                    if self.playing && self.inter_until == 0.0 {
                        self.hud.center("GO!", "", 700.0, Some("#7dff3d"));
                    }
                } else {
                    let rt = self.server_now() - 1.0;
                    let pp = App::person(&mut self.people, id);
                    pp.buf.clear();
                    pp.alive = true;
                    pp.buf.push_back(Sample { ts: rt, p, yaw, pitch: 0.0 });
                    pp.pos = p;
                }
            }
            ServerMsg::Beam { by, o, e, hit, over, mhit } => {
                let color = self.people.get(&by).map(|p| p.color.clone()).unwrap_or_else(|| "#ffffff".into());
                if by == self.me.id {
                    if hit != 0 {
                        let big = over != 0 || self.people.get(&hit).map_or(false, |v| v.hp <= 1);
                        self.hitmark(big);
                        self.sfx.hit();
                    }
                    if mhit != 0 {
                        self.hitmark(true);
                        self.sfx.hit();
                        self.hud.center("MISSILE DOWN!", "", 1300.0, Some("#7dff3d"));
                    }
                    return;
                }
                let from = match self.people.get(&by) {
                    Some(p) if p.has_mesh && p.visible && p.alive => self.tip_of(p),
                    _ => o,
                };
                self.beam(from, e, &color, if over != 0 { 2.2 } else { 1.0 });
                self.burst(e, &color, 10, 14.0, 0.35);
                let (d, pan) = self.sound_pan(from[0], from[2]);
                self.sfx.pew((0.22f64.min(14.0 / d.max(1.0) * 0.22) + 0.02) as f32, pan as f32, over != 0);
            }
            ServerMsg::Hurt { id, by, hp, w } => {
                if id == self.me.id {
                    self.me.hp = hp;
                    self.hud_hp();
                    self.hud_hurt(by);
                    self.sfx.hurt();
                    self.buzz(&[60]);
                }
                if by == self.me.id && (w == "m" || w == "d") && id != self.me.id {
                    self.hitmark(false);
                    self.sfx.hit();
                }
            }
            ServerMsg::Deflect { id, by } => {
                if by == self.me.id {
                    self.sfx.deflect();
                }
                if let Some(p) = self.people.get(&id) {
                    if p.has_mesh {
                        let pos = [p.pos[0], p.pos[1] + 3.0, p.pos[2]];
                        self.burst(pos, "#66e6ff", 12, 18.0, 0.4);
                    }
                }
            }
            ServerMsg::Tag { v, k, streak, w } => {
                let vp = self.people.get(&v).map(|p| (p.name.clone(), p.color.clone(), p.pos, p.has_mesh));
                let kp = self.people.get(&k).map(|p| (p.name.clone(), p.color.clone()));
                if let (Some(vv), Some(kk)) = (&vp, &kp) {
                    if v == k {
                        self.hud.feed(vec![n(&vv.0, &vv.1), t(" blew themselves up")]);
                    } else {
                        let verb = match w.as_str() {
                            "m" => " blasted ",
                            "d" => " drone-bombed ",
                            _ => " tagged ",
                        };
                        self.hud.feed(vec![n(&kk.0, &kk.1), t(verb), n(&vv.0, &vv.1)]);
                    }
                }
                if v == self.me.id {
                    if self.ms.active {
                        self.end_missile();
                    }
                    self.me.alive = false;
                    self.me.hp = 0;
                    self.me.dead_at = now_s();
                    self.me.killer = k;
                    self.hud_hp();
                    self.firing = false;
                    self.sfx.tagged();
                    self.buzz(&[80, 60, 160]);
                    let c = self.my_color();
                    self.burst([self.me.x, self.me.y + 3.0, self.me.z], &c, 40, 30.0, 0.9);
                    let sub = match &kp {
                        Some(kk) => format!("by {} · back in {}s", kk.0, self.rules.respawn.round()),
                        None => String::new(),
                    };
                    self.hud.center("TAGGED", &sub, 2600.0, Some("#ff4d6d"));
                } else if let Some(vv) = &vp {
                    if vv.3 {
                        self.burst([vv.2[0], vv.2[1] + 3.0, vv.2[2]], &vv.1.clone(), 40, 30.0, 0.9);
                    }
                    if let Some(p) = self.people.get_mut(&v) {
                        p.alive = false;
                        p.visible = false;
                    }
                }
                if k == self.me.id && v != self.me.id {
                    self.sfx.scored();
                    let call = match streak {
                        2 => "Double tag!",
                        3 => "On a roll!",
                        5 => "Unstoppable!",
                        7 => "Creek legend!",
                        10 => "Absolutely untaggable!",
                        _ => "",
                    };
                    let who = vp.as_ref().map(|v| v.0.clone()).unwrap_or_else(|| "someone".into());
                    self.hud.center(&format!("You tagged {}!", who), call, 1600.0, Some("#7dff3d"));
                } else if let Some(kk) = &kp {
                    if matches!(streak, 3 | 5 | 7 | 10) {
                        self.hud.feed(vec![n(&kk.0, &kk.1), t(&format!(" is on a {}-tag streak", streak))]);
                    }
                }
            }
            ServerMsg::Pick { i, id, kind, until } => {
                self.set_spot(i, false);
                if id == self.me.id && kind == "drone" {
                    self.me.drones = 1;
                    self.sfx.pick();
                    let sub = if self.touch_mode { "Tap DRONE to fly it" } else { "Press E to fly it" };
                    self.hud.center("DRONE!", sub, 1800.0, Some(power_color("drone")));
                } else if id == self.me.id {
                    let at = self.server_now() + until;
                    self.me.power_until.set(&kind, at);
                    self.sfx.pick();
                    let sub = match kind.as_str() {
                        "rapid" => "Fire fast",
                        "over" => "One hit tags",
                        "speed" => "Run fast",
                        _ => "",
                    };
                    self.hud.center(&power_name(&kind).to_uppercase(), sub, 1200.0, Some(power_color(&kind)));
                } else if let Some(p) = self.people.get(&id) {
                    let parts = vec![n(&p.name, &p.color), t(" grabbed "), n(power_name(&kind), power_color(&kind))];
                    self.hud.feed(parts);
                }
            }
            ServerMsg::Spot { i, up } => self.set_spot(i, up),
            ServerMsg::Launch { by, k, p } => {
                if by == self.me.id {
                    return;
                }
                let (d, pan) = self.sound_pan(p[0], p[2]);
                self.sfx.launch((0.35f64.min(20.0 / d.max(1.0) * 0.35) + 0.03) as f32, pan as f32);
                if let Some(pp) = self.people.get(&by) {
                    let parts = vec![n(&pp.name, &pp.color), t(if k == "drone" { " sent out a drone" } else { " launched a missile" })];
                    self.hud.feed(parts);
                }
            }
            ServerMsg::Boom { by, k, p, fizzle, down } => {
                self.rmis.remove(&by);
                let r = if k == "drone" { self.rules.dblast } else { self.rules.blast };
                if let Some(down) = down.filter(|d| *d != 0) {
                    // shot out of the sky: a harmless bang in mid-air
                    let owner = self.people.get(&by).map(|p| (p.name.clone(), p.color.clone()));
                    let shooter = self.people.get(&down).map(|p| (p.name.clone(), p.color.clone()));
                    if by == self.me.id && self.ms.active {
                        self.ms.x = p[0];
                        self.ms.y = p[1];
                        self.ms.z = p[2];
                        self.end_missile();
                    }
                    let col = shooter.as_ref().map(|s| s.1.clone()).unwrap_or_else(|| "#fff".into());
                    self.explosion(p, &col, true, r);
                    if let (Some(o), Some(s)) = (&owner, &shooter) {
                        let what = if k == "drone" { "'s drone" } else { "'s missile" };
                        self.hud.feed(vec![n(&s.0, &s.1), t(" shot down "), n(&o.0, &o.1), t(what)]);
                    }
                    if by == self.me.id {
                        let sub = shooter.map(|s| format!("by {}", s.0)).unwrap_or_default();
                        self.hud.center(if k == "drone" { "DRONE SHOT DOWN" } else { "MISSILE SHOT DOWN" }, &sub, 1800.0, Some("#ff4d6d"));
                    }
                    return;
                }
                if by == self.me.id {
                    if self.ms.active {
                        self.end_missile();
                    }
                    if fizzle == 0 {
                        return;
                    }
                }
                let col = self.people.get(&by).map(|p| p.color.clone()).unwrap_or_else(|| "#fff".into());
                self.explosion(p, &col, fizzle == 0, r);
            }
            ServerMsg::Feed { msg } => self.hud.feed(vec![Part::T(msg)]),
            ServerMsg::RoundEnd { winner, board, next } => {
                self.inter_until = now_s() + next;
                self.firing = false;
                let w = board.iter().find(|r| r.id == winner);
                let win = el("roundend").query_selector(".win").ok().flatten();
                if let Some(win) = win {
                    let win: web_sys::HtmlElement = win.unchecked_into();
                    let text = match w {
                        Some(w) if w.id == self.me.id => "You win the round!".to_string(),
                        Some(w) => format!("{} wins the round!", w.name),
                        None => "Nobody tagged anybody.".into(),
                    };
                    win.set_text_content(Some(&text));
                    dom::style(&win, "color", w.map(|w| w.color.as_str()).unwrap_or(""));
                }
                let rows: Vec<Row> = board
                    .iter()
                    .map(|r| Row { id: r.id, name: r.name.clone(), color: r.color.clone(), bot: r.bot, cols: vec![r.score, r.tagged] })
                    .collect();
                if let Some(tb) = el("roundend").query_selector("tbody").ok().flatten() {
                    hud::board(&tb, &rows, self.me.id);
                }
                dom::hidden("roundend", false);
            }
            ServerMsg::Round { left, no, spots } => {
                self.inter_until = 0.0;
                self.round_ends = now_s() + left;
                dom::hidden("roundend", true);
                for s in spots {
                    self.set_spot(s.i, s.up);
                }
                self.hud.center(&format!("ROUND {}", no), "Fight!", 1500.0, None);
            }
            ServerMsg::Pong { .. } => {}
        }
    }

    fn set_spot(&mut self, i: u32, up: bool) {
        if let Some(s) = self.spots.iter_mut().find(|s| s.i == i) {
            s.up = up;
        }
    }

    // ------------------------------------------------------------------ hud
    fn hitmark(&mut self, big: bool) {
        let h = el("hitmark");
        dom::class(&h, "big", big);
        self.hud.pulse(h, "on");
    }

    fn hud_hurt(&mut self, from: u32) {
        self.hud.pulse(el("vig"), "on");
        let Some(p) = self.people.get(&from) else { return };
        if !p.has_mesh {
            return;
        }
        let a = (p.pos[0] - self.me.x).atan2(-(p.pos[2] - self.me.z)) - self.me.yaw;
        if let Some(i) = el("dir").first_element_child() {
            let i: web_sys::HtmlElement = i.unchecked_into();
            dom::style(&i, "transform", &format!("rotate({}rad)", a));
            self.hud.pulse(i, "on");
        }
    }

    pub fn hud_hp(&self) {
        let h = el("hp");
        h.set_inner_html("");
        for k in 0..self.rules.hp {
            let i = dom::make("i");
            if k >= self.me.hp {
                i.set_class_name("gone");
            }
            let _ = h.append_with_node_1(&i);
        }
    }

    fn hud_powers(&self) {
        let w = el("powers");
        w.set_inner_html("");
        let t = self.server_now();
        let pill = |bg: &str, text: &str| {
            let s = dom::make("span");
            s.set_class_name("pw");
            dom::style(&s, "background", bg);
            s.set_text_content(Some(text));
            let _ = w.append_with_node_1(&s);
        };
        for k in ["rapid", "over", "speed"] {
            let until = self.me.power_until.get(k);
            if until > t {
                pill(power_color(k), &format!("{} {}", power_name(k), (until - t).ceil()));
            }
        }
        if self.me.flags & 1 != 0 {
            pill("#66e6ff", "Shield");
        }
        let flying_drone = self.ms.active && self.ms.drone;
        if self.me.drones > 0 || flying_drone {
            let txt = if flying_drone { "Drone flying" } else if self.touch_mode { "Drone ready" } else { "Drone ready · E" };
            pill(power_color("drone"), txt);
        }
        dom::hidden("tdrone", !(self.me.drones > 0 || flying_drone));
        let left = self.ms.ready_at - now_s();
        let flying = self.ms.active && !self.ms.drone;
        let bg = if flying { "#ffb347" } else if left > 0.0 { "rgba(255,255,255,.25)" } else { "#ffb347" };
        let txt = if flying {
            "Missile flying".to_string()
        } else if left > 0.0 {
            format!("Missile {}s", left.ceil())
        } else if self.touch_mode {
            "Missile ready".into()
        } else {
            "Missile ready · Q".into()
        };
        pill(bg, &txt);
    }

    fn ranks(&self) -> Vec<&Person> {
        let mut r: Vec<&Person> = self.people.values().collect();
        r.sort_by(|a, b| b.score.cmp(&a.score).then(a.tagged.cmp(&b.tagged)));
        r
    }

    pub fn hud_top(&self) {
        let r = self.ranks();
        let mine = self.people.get(&self.me.id);
        let text = match mine {
            None => "—".to_string(),
            Some(_) if r.len() < 2 => "Just you so far · share the link".to_string(),
            Some(m) => {
                let i = r.iter().position(|p| p.id == self.me.id).unwrap_or(0);
                format!("{} tag{} · {} of {}", m.score, if m.score == 1 { "" } else { "s" }, hud::ordinal(i + 1), r.len())
            }
        };
        dom::text("rank", &text);
        dom::text("me", mine.map(|m| m.name.as_str()).unwrap_or(""));
    }

    pub fn show_board(&self, on: bool) {
        let b = el("board");
        dom::class(&b, "hidden", !on);
        if on {
            let rows: Vec<Row> = self
                .ranks()
                .iter()
                .map(|p| Row { id: p.id, name: p.name.clone(), color: p.color.clone(), bot: p.bot, cols: vec![p.score, p.tagged, p.best] })
                .collect();
            if let Some(tb) = b.query_selector("tbody").ok().flatten() {
                hud::board(&tb, &rows, self.me.id);
            }
        }
    }

    pub fn set_touch_mode(&mut self, on: bool) {
        self.touch_mode = on;
        if let Some(b) = dom::doc().body() {
            dom::class(&b, "touching", on);
        }
        dom::hidden("touch", !(on && self.playing));
        dom::hidden("keyhelp", on);
        dom::hidden("touchhelp", !on);
        if on && dom::doc().pointer_lock_element().is_some() {
            dom::doc().exit_pointer_lock();
        }
    }

    /// Capture the mouse (asking for raw movement where the browser allows it).
    pub fn lock(&self) {
        if self.touch_mode {
            return;
        }
        let c = self.canvas.clone();
        let f = js_sys::Reflect::get(&c, &"requestPointerLock".into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok());
        let Some(f) = f else { return };
        let opts = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&opts, &"unadjustedMovement".into(), &true.into());
        match f.call1(&c, &opts) {
            Ok(r) => {
                if let Ok(p) = r.dyn_into::<js_sys::Promise>() {
                    let c2 = c.clone();
                    let cb = Closure::once(move |_: JsValue| c2.request_pointer_lock());
                    let _ = p.catch(&cb);
                    cb.forget();
                }
            }
            Err(_) => c.request_pointer_lock(),
        }
    }

    // ------------------------------------------------------------------ effects
    fn beam(&mut self, a: [f64; 3], b: [f64; 3], color: &str, thick: f32) {
        let (a, b) = (v3(a), v3(b));
        if (b - a).length() < 0.1 {
            return;
        }
        self.beams.push(BeamFx { a, b, color: hex(color), thick, life: 0.28 });
    }

    fn burst(&mut self, p: [f64; 3], color: &str, n: usize, speed: f64, life: f64) {
        let mut pts = Vec::with_capacity(n);
        for _ in 0..n {
            let a = dom::random() * PI * 2.0;
            let u = dom::random() * 2.0 - 1.0;
            let r = (1.0 - u * u).sqrt();
            let s = speed * (0.3 + dom::random() * 0.7);
            pts.push((p, [a.cos() * r * s, u.abs() * s * 0.8 + 4.0, a.sin() * r * s]));
        }
        self.bursts.push(Burst { pts, color: hex(color) * 3.0, life, max: life });
    }

    fn explosion(&mut self, p: [f64; 3], color: &str, big: bool, radius: f64) {
        let r = if big { if radius > 0.0 { radius } else { 18.0 } } else { 3.0 };
        for (c, k, op) in [("#ffb347", 2.5f32, 0.9f32), (color, 1.6, 0.45)] {
            let rr = if c == color { r } else { r * 0.7 };
            self.blasts.push(Blast { p: v3(p), color: hex(c) * k, r: rr, op, life: 0.5 });
        }
        self.burst(p, "#ffb347", if big { 70 } else { 12 }, if big { 55.0 } else { 12.0 }, if big { 1.1 } else { 0.5 });
        if big {
            self.burst(p, color, 40, 40.0, 0.9);
        }
        let d = hyp3(p[0] - self.me.x, p[1] - self.me.y, p[2] - self.me.z);
        let pan = ((p[0] - self.me.x).atan2(-(p[2] - self.me.z)) - self.me.yaw).sin();
        if big {
            self.sfx.boom((0.8f64.min(40.0 / d.max(1.0) * 0.8) + 0.05) as f32, pan as f32);
            self.shake = self.shake.max((1.0 - d / 90.0).max(0.0));
        } else {
            self.sfx.fizzle();
        }
    }

    fn step_fx(&mut self, dt: f64) {
        self.blasts.retain_mut(|b| {
            b.life -= dt;
            b.life > 0.0
        });
        self.beams.retain_mut(|b| {
            b.life -= dt;
            b.life > 0.0
        });
        self.bursts.retain_mut(|s| {
            s.life -= dt;
            if s.life <= 0.0 {
                return false;
            }
            for (p, v) in s.pts.iter_mut() {
                v[1] -= 40.0 * dt;
                p[0] += v[0] * dt;
                p[1] += v[1] * dt;
                p[2] += v[2] * dt;
            }
            true
        });
    }

    // ------------------------------------------------------------------ input actions
    pub fn look(&mut self, dx: f64, dy: f64) {
        if self.ms.active {
            self.ms.yaw += dx * self.sens * 0.8;
            self.ms.pitch = (self.ms.pitch - dy * self.sens * 0.8).clamp(-1.45, 1.45);
            return;
        }
        let zs = self.cam.fov as f64 / FOV;
        self.me.yaw += dx * self.sens * zs;
        self.me.pitch = (self.me.pitch - dy * self.sens * zs).clamp(-1.45, 1.45);
    }

    pub fn jump(&mut self) {
        if !self.me.alive || self.ms.active {
            return;
        }
        if self.me.on_ground {
            self.me.vy = JUMP1;
            self.me.on_ground = false;
            self.me.jumps = 1;
            self.sfx.jump();
        } else if self.me.jumps < 2 {
            self.me.vy = JUMP2;
            self.me.jumps = 2;
            self.sfx.jump();
            self.burst([self.me.x, self.me.y, self.me.z], "#22f0ff", 10, 10.0, 0.35);
        }
    }

    pub fn start_firing(&mut self) {
        self.sfx.wake();
        self.firing = true;
        self.heat.warned = false;
    }

    fn heat_now(&self, t: f64) -> f64 {
        if t < self.heat.lock_until {
            return (self.heat.lock_until - t) / self.rules.lock;
        }
        (self.heat.v - (t - self.heat.t) * self.rules.cool).max(0.0)
    }

    fn ground(&self, x: f64, z: f64) -> f64 {
        self.ground.at(x, z)
    }

    fn my_move(&mut self, dt: f64) {
        let me = &self.me;
        let fwd = [me.yaw.sin(), -me.yaw.cos()];
        let right = [me.yaw.cos(), me.yaw.sin()];
        let k = |c: &str| self.keys.contains(c);
        let (mut ix, mut iz) = (0.0, 0.0);
        if k("KeyW") || k("ArrowUp") {
            iz += 1.0;
        }
        if k("KeyS") || k("ArrowDown") {
            iz -= 1.0;
        }
        if k("KeyD") || k("ArrowRight") {
            ix += 1.0;
        }
        if k("KeyA") || k("ArrowLeft") {
            ix -= 1.0;
        }
        if self.touch_mode {
            ix += self.touch.mx;
            iz -= self.touch.mz;
        }
        if self.ms.active {
            ix = 0.0;
            iz = 0.0;
        }
        let (mut wx, mut wz) = (fwd[0] * iz + right[0] * ix, fwd[1] * iz + right[1] * ix);
        let wl = wx.hypot(wz);
        if wl > 1.0 {
            wx /= wl;
            wz /= wl;
        }
        let t = self.server_now();
        let in_water = self.ground.water(me.x, me.z) && me.on_ground;
        let sprint = k("ShiftLeft") || k("ShiftRight") || (self.touch_mode && self.touch.mx.hypot(self.touch.mz) > 0.9);
        let sp = (if sprint { 31.0 } else { 21.0 }) * (if me.power_until.speed > t { 1.5 } else { 1.0 }) * (if in_water { 0.6 } else { 1.0 });
        let kk = (dt * if me.on_ground { 12.0 } else { 2.5 }).min(1.0);
        let mut me_ = std::mem::take(&mut self.me);
        me_.vx += (wx * sp - me_.vx) * kk;
        me_.vz += (wz * sp - me_.vz) * kk;
        let (mut nx, mut nz) = (me_.x + me_.vx * dt, me_.z + me_.vz * dt);
        let step = (nx - me_.x).hypot(nz - me_.z);
        if me_.on_ground && self.ground(nx, nz) - me_.y > 0.5f64.max(step * 1.5) {
            nx = me_.x;
            nz = me_.z;
            me_.vx *= 0.3;
            me_.vz *= 0.3;
        }
        let (px, pz) = self.solids.push_out(nx, nz, me_.y);
        nx = px;
        nz = pz;
        me_.vy -= GRAV * dt;
        let mut ny = me_.y + me_.vy * dt;
        let fl = self.solids.floor_at(self.ground(nx, nz), nx, nz, me_.y);
        if ny <= fl || (me_.on_ground && me_.vy <= 0.0 && ny - fl < 1.2f64.max(step * 1.6)) {
            ny = fl;
            me_.vy = 0.0;
            me_.on_ground = true;
            me_.jumps = 0;
        } else {
            me_.on_ground = false;
        }
        me_.x = nx;
        me_.z = nz;
        me_.y = ny;
        me_.bob += if me_.on_ground { me_.vx.hypot(me_.vz) * dt * 0.45 } else { 0.0 };
        let splash = in_water && me_.vx.hypot(me_.vz) > 8.0 && dom::random() < dt * 8.0;
        self.me = me_;
        if splash {
            let p = [self.me.x + (dom::random() - 0.5) * 2.0, self.me.y + 0.2, self.me.z + (dom::random() - 0.5) * 2.0];
            self.burst(p, "#22f0ff", 6, 7.0, 0.4);
        }
    }

    /// The view gun's transform in camera space.
    fn gun_local(&self) -> Mat4 {
        let gk = (self.cam.aspect as f64).min(1.0);
        let bob_y = (self.me.bob * 2.0).sin() * 0.12;
        let scale = 0.5 * (0.55 + 0.45 * gk);
        let pos = Vec3::new(((0.36 + self.me.bob.sin() * 0.015) * gk) as f32, (-0.34 + bob_y * 0.2) as f32, (-0.95 + self.gun_kick * 0.06) as f32);
        Mat4::from_translation(pos) * Mat4::from_scale(Vec3::splat(scale as f32))
    }

    fn eye_camera_world(&self) -> Mat4 {
        let bob_y = (self.me.bob * 2.0).sin() * 0.12;
        Mat4::from_translation(Vec3::new(self.me.x as f32, (self.me.y + EYE + bob_y) as f32, self.me.z as f32))
            * Mat4::from_rotation_y(-self.me.yaw as f32)
            * Mat4::from_rotation_x(self.me.pitch as f32)
    }

    fn try_fire(&mut self) {
        if !self.firing || !self.me.alive || self.paused || self.inter_until != 0.0 || self.ms.active {
            return;
        }
        let t = now_s();
        let snow = self.server_now();
        let rapid = self.me.power_until.rapid > snow;
        let cd = if rapid { self.rules.rapid } else { self.rules.cd };
        if t < self.heat.lock_until {
            if !self.heat.warned {
                self.heat.warned = true;
                self.sfx.dry();
            }
            return;
        }
        if t - self.me.last_fire < cd {
            return;
        }
        self.me.last_fire = t;
        self.heat.v = self.heat_now(t) + if rapid { self.rules.heat_rapid } else { self.rules.heat };
        self.heat.t = t;
        if self.heat.v >= 1.0 {
            self.heat.v = 0.0;
            self.heat.lock_until = t + self.rules.lock;
            self.heat.warned = true;
            self.sfx.overheat();
        }
        let eye = [self.me.x, self.me.y + EYE, self.me.z];
        let d = dir_of(self.me.yaw, self.me.pitch);
        let mut tend = self.rules.range;
        if let Some(tg) = self.ground.ray(eye, d, tend) {
            tend = tg;
        }
        if let Some(tp) = self.solids.ray(eye, d, tend) {
            tend = tp;
        }
        // the people you see (where they are drawn)
        for p in self.people.values() {
            if p.id == self.me.id || !p.seen() {
                continue;
            }
            let c = p.pos;
            let (a1, b1) = (c[1] + 1.4, c[1] + 5.2);
            let (wx, wz) = (eye[0] - c[0], eye[2] - c[2]);
            let dh = d[0].hypot(d[2]);
            if dh < 1e-4 {
                continue;
            }
            let s = -(wx * d[0] + wz * d[2]) / (dh * dh);
            if s < 0.0 || s > tend {
                continue;
            }
            let (px, pz, py) = (eye[0] + d[0] * s - c[0], eye[2] + d[2] * s - c[2], eye[1] + d[1] * s);
            if px.hypot(pz) < 2.0 && py > a1 - 2.0 && py < b1 + 2.0 {
                tend = s;
            }
        }
        for r in self.rmis.values() {
            let c = r.pos;
            let v = [c[0] - eye[0], c[1] - eye[1], c[2] - eye[2]];
            let s = v[0] * d[0] + v[1] * d[1] + v[2] * d[2];
            if s > 0.0 && s < tend && hyp3(v[0] - d[0] * s, v[1] - d[1] * s, v[2] - d[2] * s) < 3.0 {
                tend = s;
            }
        }
        let end = [eye[0] + d[0] * tend, eye[1] + d[1] * tend, eye[2] + d[2] * tend];
        let muzzle = (self.eye_camera_world() * self.gun_local()).transform_point3(Vec3::new(0.0, 0.02, -0.56));
        let over = self.me.power_until.over > snow;
        let col = self.my_color();
        self.beam([muzzle.x as f64, muzzle.y as f64, muzzle.z as f64], end, &col, if over { 2.2 } else { 1.0 });
        self.burst(end, &col, 10, 14.0, 0.35);
        self.sfx.pew(0.22, 0.0, over);
        self.gun_kick = 1.0;
        let rt = self.shown_rt.unwrap_or(snow - INTERP);
        self.send(&ClientMsg::Fire { o: r2(eye), d: r4(d), rt: round_to(rt, 3) });
    }

    pub fn launch_missile(&mut self) {
        let t = now_s();
        if !self.playing || !self.me.alive || self.ms.active || self.inter_until != 0.0 || t < self.ms.ready_at {
            if self.playing && self.me.alive && t < self.ms.ready_at {
                self.sfx.dry();
            }
            return;
        }
        let d = dir_of(self.me.yaw, self.me.pitch);
        let ready_at = self.ms.ready_at;
        self.ms = Ms {
            active: true,
            drone: false,
            x: self.me.x + d[0] * 3.0,
            y: self.me.y + EYE + d[1] * 3.0,
            z: self.me.z + d[2] * 3.0,
            yaw: self.me.yaw,
            pitch: self.me.pitch,
            t0: t,
            ready_at,
            ..Default::default()
        };
        self.firing = false;
        self.zoom_held = false;
        self.zoom_toggle = false;
        self.me.vx = 0.0;
        self.me.vz = 0.0;
        self.send(&ClientMsg::Launch { d: r4(d), k: None });
        self.sfx.launch(0.35, 0.0);
        dom::text("mlbl", if self.touch_mode { "MISSILE · DRAG TO STEER · FIRE: BOOM" } else { "MISSILE · STEER · CLICK OR Q TO DETONATE" });
        let mc = el("mcam");
        dom::class(&mc, "hidden", false);
        dom::class(&mc, "drone", false);
    }

    pub fn launch_drone(&mut self) {
        let t = now_s();
        if !self.playing || !self.me.alive || self.ms.active || self.inter_until != 0.0 || self.me.drones == 0 {
            if self.playing && self.me.alive && self.me.drones == 0 && !self.ms.active {
                self.sfx.dry();
            }
            return;
        }
        let d = dir_of(self.me.yaw, 0.0);
        let ready_at = self.ms.ready_at;
        self.ms = Ms {
            active: true,
            drone: true,
            x: self.me.x + d[0] * 3.0,
            y: self.me.y + EYE + 2.0,
            z: self.me.z + d[2] * 3.0,
            yaw: self.me.yaw,
            pitch: self.me.pitch * 0.5,
            t0: t,
            ready_at,
            ..Default::default()
        };
        self.me.drones = 0;
        self.firing = false;
        self.zoom_held = false;
        self.zoom_toggle = false;
        self.me.vx = 0.0;
        self.me.vz = 0.0;
        self.send(&ClientMsg::Launch { d: r4(d), k: Some("drone".into()) });
        self.sfx.drone_out();
        dom::text("mlbl", if self.touch_mode { "DRONE · STICK TO FLY · FIRE: BOOM" } else { "DRONE · WASD + SPACE/SHIFT · CLICK OR E: BOOM" });
        let mc = el("mcam");
        dom::class(&mc, "hidden", false);
        dom::class(&mc, "drone", true);
        dom::hidden("lockbox", true);
    }

    fn end_missile(&mut self) {
        if !self.ms.drone {
            self.ms.ready_at = now_s() + self.rules.mcd;
        }
        self.ms.active = false;
        self.ms.end_at = now_s() + 0.8;
        self.ms.cam = [self.ms.x, self.ms.y, self.ms.z];
        dom::hidden("mcam", true);
        dom::hidden("lockbox", true);
    }

    pub fn detonate(&mut self, at: Option<[f64; 3]>) {
        if !self.ms.active {
            return;
        }
        let p = at.unwrap_or([self.ms.x, self.ms.y, self.ms.z]);
        self.send(&ClientMsg::Boom { p: r2(p) });
        let col = self.my_color();
        let r = if self.ms.drone { self.rules.dblast } else { self.rules.blast };
        self.explosion(p, &col, true, r);
        self.ms.x = p[0];
        self.ms.y = p[1];
        self.ms.z = p[2];
        self.end_missile();
    }

    fn clear_to(&self, o: [f64; 3], c: [f64; 3], dist: f64) -> bool {
        let d = [(c[0] - o[0]) / dist, (c[1] - o[1]) / dist, (c[2] - o[2]) / dist];
        self.ground.ray(o, d, dist).is_none() && self.solids.ray(o, d, dist).is_none()
    }

    fn step_drone(&mut self, dt: f64) {
        let t = now_s();
        let life = self.rules.dlife;
        if t - self.ms.t0 > life {
            self.detonate(None);
            return;
        }
        let k = |c: &str| self.keys.contains(c);
        let (mut f, mut sr, mut up) = (0.0, 0.0, 0.0);
        if k("KeyW") || k("ArrowUp") {
            f += 1.0;
        }
        if k("KeyS") || k("ArrowDown") {
            f -= 1.0;
        }
        if k("KeyD") || k("ArrowRight") {
            sr += 1.0;
        }
        if k("KeyA") || k("ArrowLeft") {
            sr -= 1.0;
        }
        if k("Space") {
            up += 1.0;
        }
        if k("ShiftLeft") || k("ShiftRight") || k("KeyC") {
            up -= 1.0;
        }
        if self.touch_mode {
            f -= self.touch.mz;
            sr += self.touch.mx;
        }
        let ms = &self.ms;
        let d = dir_of(ms.yaw, ms.pitch);
        let r = [ms.yaw.cos(), 0.0, ms.yaw.sin()];
        let mut w = [d[0] * f + r[0] * sr, d[1] * f + up, d[2] * f + r[2] * sr];
        let wl = hyp3(w[0], w[1], w[2]);
        if wl > 1.0 {
            w = w.map(|v| v / wl);
        }
        let sp = self.rules.dspeed * 0.95;
        let kk = (dt * 4.0).min(1.0);
        let o = [ms.x, ms.y, ms.z];
        let (mut vx, mut vy, mut vz) = (ms.vx, ms.vy, ms.vz);
        vx += (w[0] * sp - vx) * kk;
        vy += (w[1] * sp - vy) * kk;
        vz += (w[2] * sp - vz) * kk;
        let step = [vx * dt, vy * dt, vz * dt];
        let len = hyp3(step[0], step[1], step[2]);
        let (mut x, mut y, mut z) = (ms.x, ms.y, ms.z);
        if len > 1e-4 {
            let u = step.map(|v| v / len);
            if self.solids.ray(o, u, len + 1.6).is_some() || self.ground.ray(o, u, len + 1.6).is_some() {
                // bump off walls
                vx *= -0.3;
                vy *= -0.3;
                vz *= -0.3;
            } else {
                x += step[0];
                y += step[1];
                z += step[2];
            }
        }
        let size = self.size();
        x = x.clamp(3.0, size - 3.0);
        z = z.clamp(3.0, size - 3.0);
        let g = self.ground(x, z);
        y = y.max(g + 1.5).min(g + 220.0);
        let ms = &mut self.ms;
        ms.x = x;
        ms.y = y;
        ms.z = z;
        ms.vx = vx;
        ms.vy = vy;
        ms.vz = vz;
        // fly into someone and it goes off
        let bump = self.people.values().any(|p| p.id != self.me.id && p.seen() && hyp3(p.pos[0] - x, p.pos[1] + 3.0 - y, p.pos[2] - z) < 3.8);
        if bump {
            self.detonate(None);
            return;
        }
        self.ms.send_t -= dt;
        if self.ms.send_t <= 0.0 {
            self.ms.send_t = 0.05;
            let h = dir_of(self.ms.yaw, 0.0);
            self.send(&ClientMsg::Ms { p: r2([x, y, z]), d: r4(h) });
        }
        dom::style(&el("mfuel"), "--f", &format!("{}", (1.0 - (t - self.ms.t0) / life).max(0.0)));
        if dom::random() < dt * 6.0 {
            self.sfx.drone_hum();
        }
    }

    fn step_missile(&mut self, dt: f64) {
        let t = now_s();
        let ms = &self.ms;
        let o = [ms.x, ms.y, ms.z];
        let size = self.size();
        if t - ms.t0 > self.rules.mlife || ms.y < self.ground(ms.x, ms.z) || ms.x < 0.0 || ms.z < 0.0 || ms.x > size || ms.z > size || ms.y > 700.0 {
            self.detonate(None);
            return;
        }
        // homing: someone in front of it, in the open, pulls it round gently (you can still steer away)
        let mut d = dir_of(ms.yaw, ms.pitch);
        let mut best = 0.45;
        let mut lock: Option<(u32, [f64; 3], f64)> = None;
        for p in self.people.values() {
            if p.id == self.me.id || !p.seen() {
                continue;
            }
            let c = [p.pos[0], p.pos[1] + 3.0, p.pos[2]];
            let v = [c[0] - o[0], c[1] - o[1], c[2] - o[2]];
            let dist = hyp3(v[0], v[1], v[2]);
            if !(1.0..=420.0).contains(&dist) {
                continue;
            }
            let ang = ((v[0] * d[0] + v[1] * d[1] + v[2] * d[2]) / dist).clamp(-1.0, 1.0).acos();
            if ang < best && self.clear_to(o, c, dist) {
                best = ang;
                lock = Some((p.id, v, dist));
            }
        }
        if let Some(l) = lock {
            if self.ms.lock != Some(l.0) {
                self.sfx.lock();
            }
        }
        self.ms.lock = lock.map(|l| l.0);
        if let Some((_, v, _)) = lock {
            let ty = v[0].atan2(-v[2]);
            let tp = v[1].atan2(v[0].hypot(v[2]));
            let k = 1.3 * dt;
            let dy = (ty - self.ms.yaw + PI * 3.0).rem_euclid(PI * 2.0) - PI;
            self.ms.yaw += dy.clamp(-k, k);
            self.ms.pitch += (tp - self.ms.pitch).clamp(-k, k);
            d = dir_of(self.ms.yaw, self.ms.pitch);
        }
        // fly, and blow up on whatever it meets first
        let step = self.rules.mspeed * dt;
        let mut hit: Option<f64> = None;
        if let Some(tg) = self.ground.ray(o, d, step + 1.0) {
            hit = Some(tg);
        }
        if let Some(ts) = self.solids.ray(o, d, hit.unwrap_or(step + 1.0)) {
            hit = Some(ts);
        }
        for p in self.people.values() {
            if p.id == self.me.id || !p.seen() {
                continue;
            }
            let c = [p.pos[0] - o[0], p.pos[1] + 3.0 - o[1], p.pos[2] - o[2]];
            let s = (c[0] * d[0] + c[1] * d[1] + c[2] * d[2]).clamp(0.0, step + 1.0);
            if hyp3(c[0] - d[0] * s, c[1] - d[1] * s, c[2] - d[2] * s) < 3.2 && hit.map_or(true, |h| s < h) {
                hit = Some(s);
            }
        }
        if let Some(h) = hit {
            if h <= step + 1.0 {
                let b = (h - 0.6).max(0.0);
                self.detonate(Some([o[0] + d[0] * b, o[1] + d[1] * b, o[2] + d[2] * b]));
                return;
            }
        }
        self.ms.x += d[0] * step;
        self.ms.y += d[1] * step;
        self.ms.z += d[2] * step;
        self.ms.send_t -= dt;
        if self.ms.send_t <= 0.0 {
            self.ms.send_t = 0.05;
            self.send(&ClientMsg::Ms { p: r2([self.ms.x, self.ms.y, self.ms.z]), d: r4(d) });
        }
        // the missile camera's dials
        dom::style(&el("mfuel"), "--f", &format!("{}", (1.0 - (t - self.ms.t0) / self.rules.mlife).max(0.0)));
        let lb = el("lockbox");
        match lock.and_then(|l| self.people.get(&l.0).map(|p| (p, l.2))) {
            Some((p, dist)) => {
                let (sx, sy, front) = self.cam.project(Vec3::new(p.pos[0] as f32, (p.pos[1] + 3.0) as f32, p.pos[2] as f32));
                if front {
                    let (w, h) = dom::inner_size();
                    dom::class(&lb, "hidden", false);
                    dom::style(&lb, "left", &format!("{}px", sx as f64 * w));
                    dom::style(&lb, "top", &format!("{}px", sy as f64 * h));
                    let _ = lb.set_attribute("data-name", &format!("{} · {} ft", p.name, dist.round()));
                }
            }
            None => dom::class(&lb, "hidden", true),
        }
    }

    fn draw_missiles(&mut self) {
        let rt = self.server_now() - INTERP;
        for r in self.rmis.values_mut() {
            let b = &r.buf;
            if b.is_empty() {
                continue;
            }
            let (mut s0, mut s1) = (b[0], b[b.len() - 1]);
            for i in (1..b.len()).rev() {
                if b[i - 1].ts <= rt {
                    s0 = b[i - 1];
                    s1 = b[i];
                    break;
                }
            }
            let f = if s1.ts > s0.ts { ((rt - s0.ts) / (s1.ts - s0.ts)).clamp(0.0, 1.0) } else { 1.0 };
            r.pos = [0, 1, 2].map(|i| s0.p[i] + (s1.p[i] - s0.p[i]) * f);
            r.dir = s1.d;
            if !r.drone {
                let p = [r.pos[0] as f32, r.pos[1] as f32, r.pos[2] as f32];
                if r.trail.is_empty() {
                    r.trail = vec![p; 48];
                }
                r.trail.pop();
                r.trail.insert(0, p);
            }
        }
    }

    fn draw_people(&mut self, dt: f64) {
        let rt = self.server_now() - INTERP;
        let t = now_s();
        self.shown_rt = Some(rt);
        let me_id = self.me.id;
        for p in self.people.values_mut() {
            if p.id == me_id || !p.has_mesh {
                continue;
            }
            let b = &p.buf;
            if b.is_empty() {
                p.visible = false;
                continue;
            }
            let (mut s0, mut s1) = (b[0], b[b.len() - 1]);
            for i in (1..b.len()).rev() {
                if b[i - 1].ts <= rt {
                    s0 = b[i - 1];
                    s1 = b[i];
                    break;
                }
            }
            let f = if s1.ts > s0.ts { ((rt - s0.ts) / (s1.ts - s0.ts)).clamp(0.0, 1.0) } else { 1.0 };
            let prev = p.pos;
            p.pos = [0, 1, 2].map(|i| s0.p[i] + (s1.p[i] - s0.p[i]) * f);
            let moving = hyp3(p.pos[0] - prev[0], p.pos[1] - prev[1], p.pos[2] - prev[2]) / dt.max(1e-3);
            p.yaw = lerp_angle(s0.yaw, s1.yaw, f);
            p.pitch = s0.pitch + (s1.pitch - s0.pitch) * f;
            p.upper_y = 4.4 + if moving > 3.0 && moving < 60.0 { (t * 9.0).sin().abs() * 0.25 } else { 0.0 };
            p.visible = p.alive;
        }
    }

    fn avatar_frame(p: &Person) -> (Mat4, Mat4) {
        let g = Mat4::from_translation(v3(p.pos)) * Mat4::from_rotation_y(-p.yaw as f32);
        let u = g * Mat4::from_translation(Vec3::new(0.0, p.upper_y as f32, 0.0)) * Mat4::from_rotation_x(p.pitch as f32);
        (g, u)
    }

    /// Where a player's gun tip is (beams start there).
    fn tip_of(&self, p: &Person) -> [f64; 3] {
        let (_, u) = App::avatar_frame(p);
        let v = u.transform_point3(Vec3::new(0.95, -0.4, -2.0));
        [v.x as f64, v.y as f64, v.z as f64]
    }

    // ------------------------------------------------------------------ the picture
    fn build_frame(&self, t: f64) -> Frame {
        let mut f = Frame::default();
        let tf = t as f32;
        world::draw_pillars(&mut f, &self.pillars);
        world::draw_spots(&mut f, &self.spots, tf);
        // people
        for p in self.people.values() {
            if p.id == self.me.id || !p.has_mesh || !p.visible {
                continue;
            }
            let c = hex(&p.color);
            let suit = Mat::std(hexu(0x1a1830), c * 0.18, 0.35, 0.5);
            let glow = Mat::glow(c * 1.7);
            let (g, u) = App::avatar_frame(p);
            let tr = |x: f32, y: f32, z: f32| Mat4::from_translation(Vec3::new(x, y, z));
            f.push(Shape::Capsule, g * tr(0.0, 2.6, 0.0), suit);
            for y in [2.0, 3.3] {
                f.push(Shape::Torus, g * tr(0.0, y, 0.0) * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2), glow);
            }
            f.push(Shape::Sphere16, u * tr(0.0, 1.0, 0.0) * Mat4::from_scale(Vec3::splat(0.85)), suit);
            f.push(Shape::Cube, u * tr(0.0, 1.05, -0.62) * Mat4::from_scale(Vec3::new(1.25, 0.32, 0.5)), glow);
            f.push(Shape::Cube, u * tr(0.95, -0.4, -1.0) * Mat4::from_scale(Vec3::new(0.3, 0.3, 1.9)), suit);
            f.push(Shape::Cube, u * tr(0.95, -0.4, -2.0) * Mat4::from_scale(Vec3::new(0.34, 0.34, 0.25)), glow);
            if p.flags & 1 != 0 {
                f.push(Shape::Sphere20, g * tr(0.0, 3.0, 0.0) * Mat4::from_scale(Vec3::splat(3.4)), Mat::add(hexu(0x66e6ff), 0.12 + 0.08 * (tf * 12.0).sin()));
            }
            let pw = if p.flags & 2 != 0 {
                Some("#ffe23d")
            } else if p.flags & 4 != 0 {
                Some("#ff4d6d")
            } else if p.flags & 8 != 0 {
                Some("#7dff3d")
            } else {
                None
            };
            if let Some(pw) = pw {
                f.push(
                    Shape::AuraRing,
                    g * tr(0.0, 0.2, 0.0) * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2) * Mat4::from_rotation_z(tf * 3.0),
                    Mat::Basic { color: hex(pw) * 2.0, opacity: 0.8, blend: Blend::Add, fog: true, double: true, depth_write: false },
                );
            }
            if let Some(tex) = &p.tex {
                f.sprites.push(Sprite { tex: tex.clone(), center: v3(p.pos) + Vec3::new(0.0, 8.3, 0.0), scale: (9.0, 2.25) });
            }
        }
        // other people's missiles and drones
        let flame_c = hex("#ffb347");
        for r in self.rmis.values() {
            let c = hex(&r.color);
            let pos = v3(r.pos);
            if r.drone {
                let yaw = r.dir[0].atan2(-r.dir[2]) as f32;
                let g = Mat4::from_translation(pos) * Mat4::from_rotation_y(-yaw) * Mat4::from_scale(Vec3::splat(1.3));
                let dark = Mat::std(hexu(0x2c2848), Vec3::ZERO, 0.5, 0.45);
                let tr = |x: f32, y: f32, z: f32| Mat4::from_translation(Vec3::new(x, y, z));
                f.push(Shape::Cube, g * Mat4::from_scale(Vec3::new(1.6, 0.55, 1.6)), dark);
                f.push(Shape::Cube, g * Mat4::from_scale(Vec3::new(1.7, 0.14, 1.7)), Mat::glow(c * 2.0));
                if (tf * 3.0).rem_euclid(1.0) < 0.6 {
                    f.push(Shape::Sphere10, g * tr(0.0, -0.15, -0.85) * Mat4::from_scale(Vec3::splat(0.28)), Mat::glow(hex("#ff2a4a") * 2.5));
                }
                for (i, (x, z)) in [(1.0f32, 1.0f32), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)].iter().enumerate() {
                    f.push(Shape::Cube, g * tr(x * 0.75, 0.1, z * 0.75) * Mat4::from_rotation_y(x.atan2(*z)) * Mat4::from_scale(Vec3::new(0.18, 0.14, 2.2)), dark);
                    let spin = |k: usize| Mat4::from_rotation_y(tf * 40.0 + k as f32);
                    f.push(
                        Shape::Cyl18,
                        g * tr(x * 1.5, 0.3, z * 1.5) * spin(i * 2) * Mat4::from_scale(Vec3::new(0.95, 0.05, 0.95)),
                        Mat::add(c * 1.2, 0.45),
                    );
                    f.push(Shape::Cube, g * tr(x * 1.5, 0.33, z * 1.5) * spin(i * 2 + 1) * Mat4::from_scale(Vec3::new(1.8, 0.04, 0.16)), Mat::glow(Vec3::ONE));
                }
                continue;
            }
            let dir = v3(r.dir);
            let fwd = if dir.length_squared() > 1e-8 { dir.normalize() } else { Vec3::NEG_Z };
            let xa = Vec3::Y.cross(fwd);
            let xa = if xa.length_squared() < 1e-8 { Vec3::X } else { xa.normalize() };
            let ya = fwd.cross(xa);
            let rot = Mat4::from_cols(xa.extend(0.0), ya.extend(0.0), fwd.extend(0.0), glam::Vec4::W);
            let g = Mat4::from_translation(pos) * rot * Mat4::from_scale(Vec3::splat(1.5));
            let rx = |a: f32| Mat4::from_rotation_x(a);
            f.push(Shape::Cyl10, g * rx(std::f32::consts::FRAC_PI_2) * Mat4::from_scale(Vec3::new(0.35, 2.4, 0.35)), Mat::std(hexu(0x3a3560), Vec3::ZERO, 0.5, 0.4));
            f.push(Shape::Cone10, g * Mat4::from_translation(Vec3::new(0.0, 0.0, 1.65)) * rx(std::f32::consts::FRAC_PI_2) * Mat4::from_scale(Vec3::new(0.35, 0.9, 0.35)), Mat::glow(c * 2.0));
            let fl = 0.8 + dom::random() as f32 * 0.5;
            f.push(
                Shape::Cone8,
                g * Mat4::from_translation(Vec3::new(0.0, 0.0, -2.3)) * rx(-std::f32::consts::FRAC_PI_2) * Mat4::from_scale(Vec3::new(0.32 * fl, 2.2 * fl, 0.32 * fl)),
                Mat::add(flame_c * 3.0, 0.9),
            );
            let tc = flame_c * 1.6;
            let mut trail = Vec::with_capacity(48 * 7);
            for p in &r.trail {
                trail.extend_from_slice(&[p[0], p[1], p[2], tc.x, tc.y, tc.z, 0.7]);
            }
            f.trails.push(trail);
        }
        // beams, blasts, sparks
        for b in &self.beams {
            let d = b.b - b.a;
            let len = d.length();
            let rot = glam::Quat::from_rotation_arc(Vec3::Y, d / len);
            for (w, k, op) in [(0.06 * b.thick, 3.0, 1.0f32), (0.25 * b.thick, 1.0, 0.3)] {
                let m = Mat4::from_scale_rotation_translation(Vec3::new(w, len, w), rot, b.a);
                f.push(
                    Shape::BeamCyl,
                    m,
                    Mat::Basic { color: b.color * k, opacity: op * (b.life / 0.28) as f32, blend: Blend::Add, fog: false, double: false, depth_write: false },
                );
            }
        }
        for b in &self.blasts {
            let fr = 1.0 - b.life / 0.5;
            let s = (0.5 + b.r * fr.sqrt()) as f32;
            f.push(Shape::Sphere20, Mat4::from_translation(b.p) * Mat4::from_scale(Vec3::splat(s)), Mat::add(b.color, b.op * ((1.0 - fr) * (1.0 - fr)) as f32));
        }
        for s in &self.bursts {
            let a = (s.life / s.max) as f32;
            for (p, _) in &s.pts {
                f.sparks.extend_from_slice(&[p[0] as f32, p[1] as f32, p[2] as f32, s.color.x, s.color.y, s.color.z, a]);
            }
        }
        // the view gun, in front of the camera
        if self.gun_visible {
            let cw = self.cam.world() * self.gun_local();
            let pw = if self.me.power_until.rapid > self.server_now() {
                "#ffe23d".to_string()
            } else if self.me.power_until.over > self.server_now() {
                "#ff4d6d".to_string()
            } else {
                self.my_color()
            };
            let glow = Mat::glow(hex(&pw) * 1.1);
            let dark = Mat::std(hexu(0x5a5488), hexu(0x241c44), 0.3, 0.4);
            let tr = |x: f32, y: f32, z: f32| Mat4::from_translation(Vec3::new(x, y, z));
            f.overlay.push(crate::render::Item { shape: Shape::Cube, model: cw * Mat4::from_scale(Vec3::new(0.16, 0.2, 0.9)), mat: dark });
            f.overlay.push(crate::render::Item {
                shape: Shape::Cube,
                model: cw * tr(0.0, -0.2, 0.25) * Mat4::from_rotation_x(0.3) * Mat4::from_scale(Vec3::new(0.12, 0.28, 0.2)),
                mat: dark,
            });
            f.overlay.push(crate::render::Item { shape: Shape::Cube, model: cw * tr(0.0, 0.11, -0.05) * Mat4::from_scale(Vec3::new(0.17, 0.04, 0.7)), mat: glow });
            f.overlay.push(crate::render::Item {
                shape: Shape::Muzzle,
                model: cw * tr(0.0, 0.02, -0.5) * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2),
                mat: glow,
            });
        }
        f
    }

    fn draw_mini(&self) {
        let pillars: Vec<(f64, f64, f64)> = self.pillars.iter().map(|p| (p.0, p.1, p.2)).collect();
        let spots: Vec<(f64, f64, &str)> = self.spots.iter().filter(|s| s.up).map(|s| (s.x, s.z, power_color(&s.kind))).collect();
        let people: Vec<MiniPerson> = self
            .people
            .values()
            .filter(|p| p.id != self.me.id && p.alive && p.has_mesh)
            .map(|p| MiniPerson { x: p.pos[0], z: p.pos[2], name: &p.name, color: &p.color })
            .collect();
        let col = self.people.get(&self.me.id).map(|p| p.color.as_str()).unwrap_or("#fff");
        let half = (self.cam.fov as f64) * PI / 360.0 * self.cam.aspect as f64;
        let me = if self.me.alive { Some((self.me.x, self.me.z, self.me.yaw, col, half)) } else { None };
        self.mini.draw(self.size(), &pillars, &spots, &people, me);
    }

    pub fn frame(&mut self) {
        let t = now_s();
        let dt = (t - self.last).min(0.05);
        self.last = t;
        self.world.time = t as f32;
        // zoom, the scope, the heat bar
        {
            let want = if self.playing && self.me.alive && !self.paused && !self.ms.active && (self.zoom_held || self.zoom_toggle) { 1.0 } else { 0.0 };
            self.zoom += (want - self.zoom) * (dt * 14.0).min(1.0);
            if (want - self.zoom).abs() < 0.002 {
                self.zoom = want;
            }
            let fov = FOV + (FOV_ZOOM - FOV) * self.zoom;
            if (self.cam.fov as f64 - fov).abs() > 0.01 {
                self.cam.fov = fov as f32;
                self.cam.set_proj();
            }
            dom::style(&el("scope"), "opacity", &format!("{}", (self.zoom * 1.4 - 0.4).max(0.0)));
            self.gun_visible = self.me.alive && self.zoom < 0.6 && !self.ms.active && self.ms.end_at == 0.0;
            let h = self.heat_now(t);
            let locked = t < self.heat.lock_until;
            let bar = el("heat");
            dom::style(&bar, "--h", &format!("{}", h));
            dom::class(&bar, "locked", locked);
            dom::class(&bar, "hot", !locked && h > 0.7);
            dom::style(&bar, "opacity", if self.playing && self.me.alive && (h > 0.01 || locked) { "1" } else { "0" });
        }
        if self.playing {
            if self.me.alive {
                if !self.paused || self.touch_mode {
                    self.my_move(dt);
                }
                self.try_fire();
                let bob_y = (self.me.bob * 2.0).sin() * 0.12;
                self.cam.set_euler(Vec3::new(self.me.x as f32, (self.me.y + EYE + bob_y) as f32, self.me.z as f32), self.me.pitch as f32, self.me.yaw as f32);
                if self.ms.active {
                    if !self.paused || self.touch_mode {
                        if self.ms.drone {
                            self.step_drone(dt);
                        } else {
                            self.step_missile(dt);
                        }
                    }
                    if self.ms.active {
                        self.cam.set_euler(Vec3::new(self.ms.x as f32, self.ms.y as f32, self.ms.z as f32), self.ms.pitch as f32, self.ms.yaw as f32);
                        self.gun_visible = false;
                    }
                } else if self.ms.end_at != 0.0 {
                    if t < self.ms.end_at {
                        let d = dir_of(self.ms.yaw, self.ms.pitch);
                        let c = self.ms.cam;
                        self.cam.look_at(
                            Vec3::new((c[0] - d[0] * 10.0) as f32, (c[1] - d[1] * 10.0 + 3.0) as f32, (c[2] - d[2] * 10.0) as f32),
                            v3(c),
                        );
                        self.gun_visible = false;
                    } else {
                        self.ms.end_at = 0.0;
                    }
                }
                self.gun_kick = (self.gun_kick - dt * 10.0).max(0.0);
                self.send_t -= dt;
                if self.send_t <= 0.0 {
                    self.send_t = 0.05;
                    let me = &self.me;
                    self.send(&ClientMsg::St { p: r2([me.x, me.y, me.z]), yaw: round_to(me.yaw, 3), pitch: round_to(me.pitch, 3) });
                }
            } else {
                // tagged: float up and look at who did it
                let up = ((t - self.me.dead_at) / 1.2).min(1.0);
                let pos = Vec3::new(self.me.x as f32, (self.me.y + EYE + up * 22.0) as f32, (self.me.z + up * 10.0) as f32);
                let target = match self.people.get(&self.me.killer) {
                    Some(k) if k.seen() => Vec3::new(k.pos[0] as f32, (k.pos[1] + 3.0) as f32, k.pos[2] as f32),
                    _ => v3([self.me.x, self.me.y, self.me.z]),
                };
                self.cam.look_at(pos, target);
            }
            self.draw_people(dt);
            self.draw_missiles();
            if self.shake > 0.0 {
                let (sx, sy) = ((dom::random() - 0.5) * self.shake * 1.2, (dom::random() - 0.5) * self.shake * 1.2);
                let off = Vec3::new(sx as f32, sy as f32, 0.0);
                self.cam.view *= Mat4::from_translation(-off);
                self.cam.pos += off;
                self.shake = (self.shake - dt * 2.5).max(0.0);
            }
            self.hud_t -= dt;
            if self.hud_t <= 0.0 {
                self.hud_t = 0.25;
                let left = if self.inter_until != 0.0 { self.inter_until - t } else { self.round_ends - t };
                let s = left.ceil().max(0.0) as i64;
                let timer = el("timer");
                let txt = format!("{}{}:{:02}", if self.inter_until != 0.0 { "Next round " } else { "" }, s / 60, s % 60);
                timer.set_text_content(Some(&txt));
                dom::style(&timer, "color", if self.inter_until == 0.0 && left < 30.0 { "#ff4d6d" } else { "" });
                if self.inter_until != 0.0 {
                    dom::text("nextround", &format!("Next round in {}s", s));
                }
                self.hud_powers();
                self.hud_top();
            }
            self.mini_t -= dt;
            if self.mini_t <= 0.0 {
                self.mini_t = 0.05;
                self.draw_mini();
            }
        } else {
            // lobby: a slow flight over the arena
            let a = t * 0.05;
            let s = self.size();
            self.cam.look_at(
                Vec3::new((s / 2.0 + a.sin() * 640.0) as f32, 150.0, (s / 2.0 + a.cos() * 640.0) as f32),
                Vec3::new((s / 2.0) as f32, 30.0, (s / 2.0) as f32),
            );
            self.draw_people(dt);
            self.gun_visible = false;
        }
        self.step_fx(dt);
        self.hud.step();
        let mut frame = self.build_frame(t);
        let world = &self.world;
        let pr = self.pixel_ratio as f32;
        let cam = &self.cam;
        self.r.render(cam, &mut frame, &|r, c| world.draw(r, c), &|r, c| world.draw_late(r, c), pr);
    }

    /// For the browser tests: a summary of the state.
    pub fn debug_state(&self) -> String {
        serde_json::json!({
            "playing": self.playing,
            "paused": self.paused,
            "touchMode": self.touch_mode,
            "me": { "id": self.me.id, "x": self.me.x, "y": self.me.y, "z": self.me.z, "yaw": self.me.yaw, "alive": self.me.alive, "hp": self.me.hp },
            "people": self.people.values().map(|p| serde_json::json!({"id": p.id, "name": p.name, "alive": p.alive, "visible": p.visible, "x": p.pos[0], "y": p.pos[1], "z": p.pos[2]})).collect::<Vec<_>>(),
            "rmis": self.rmis.len(),
            "ms": { "active": self.ms.active, "drone": self.ms.drone, "x": self.ms.x, "y": self.ms.y, "z": self.ms.z },
            "spots": self.spots.iter().map(|s| serde_json::json!({"i": s.i, "kind": s.kind, "up": s.up, "x": s.x, "z": s.z})).collect::<Vec<_>>(),
            "boxes": self.solids.boxes.len(),
            "pillars": self.pillars.len(),
            "beams": self.beams.len(),
            "sparks": self.bursts.len(),
            "glow": self.glow_on,
            "size": [self.r.width, self.r.height],
        })
        .to_string()
    }

    /// For the browser tests: put yourself somewhere and look somewhere.
    pub fn debug_place(&mut self, x: f64, z: f64, yaw: f64, pitch: f64) {
        self.me.x = x;
        self.me.z = z;
        self.me.y = self.ground(x, z);
        self.me.yaw = yaw;
        self.me.pitch = pitch;
        self.me.vx = 0.0;
        self.me.vz = 0.0;
    }
}
