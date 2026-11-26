//! Creek Laser Tag: the game server.
//!
//! Run: lasertag-server /run/lasertag/ws.sock   (or host:port for a local test; 127.0.0.1:8803 by default)
//!
//! Environment: LASERTAG_WEB (where terrain.json and terrain.bin are), LASERTAG_ROUND_S, LASERTAG_INTERMISSION_S,
//! LASERTAG_BOTS (bots top the arena up to this many fighters; 0, the default, means players only).

mod arena;
mod bots;
mod game;
mod pyrandom;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use lasertag_shared::protocol::ServerMsg;
use lasertag_shared::{py_mod, Ground, GroundMeta, PALETTE};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

use game::{clock, Game, Kind, MAX_HUMANS};

type Shared = Arc<Mutex<Game>>;

fn lock(g: &Shared) -> MutexGuard<'_, Game> {
    g.lock().unwrap_or_else(|e| e.into_inner())
}

fn env_num(k: &str, d: f64) -> f64 {
    std::env::var(k).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(d)
}

/// Python's `str.isprintable()`, near enough: no control or format characters, no separators but the space.
fn printable(c: char) -> bool {
    if c == ' ' {
        return true;
    }
    if c.is_control() || c.is_whitespace() {
        return false;
    }
    !matches!(c as u32, 0xAD | 0x600..=0x605 | 0x61C | 0x6DD | 0x70F | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E
        | 0x2060..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB | 0xE000..=0xF8FF | 0xF0000..=0x10FFFF)
}

fn clean_name(v: Option<&Value>, fallback: &str) -> String {
    let s = match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    let s: String = s.chars().filter(|&c| printable(c)).collect();
    let s: String = s.trim().chars().take(16).collect();
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

/// Python's `float(v)`: numbers, numeric strings and booleans.
fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(*b as u8 as f64),
        Value::String(s) => {
            let t = s.trim().to_ascii_lowercase();
            match t.as_str() {
                "inf" | "+inf" | "infinity" | "+infinity" => Some(f64::INFINITY),
                "-inf" | "-infinity" => Some(f64::NEG_INFINITY),
                "nan" | "+nan" | "-nan" => Some(f64::NAN),
                _ => t.replace('_', "").parse().ok(),
            }
        }
        _ => None,
    }
}

/// `tuple(float(v) for v in x)[:3]`, which must come out exactly 3 long.
fn vec3(v: Option<&Value>, default: [f64; 3]) -> Option<[f64; 3]> {
    let Some(v) = v else { return Some(default) };
    let items: Vec<f64> = match v {
        Value::Array(a) => a.iter().map(num).collect::<Option<Vec<f64>>>()?,
        Value::String(s) => s.chars().map(|c| num(&Value::String(c.to_string()))).collect::<Option<Vec<f64>>>()?,
        _ => return None,
    };
    if items.len() < 3 {
        return None;
    }
    Some([items[0], items[1], items[2]])
}

fn finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
}

/// One message from a player.
fn handle_msg(game: &mut Game, id: u32, m: &Value) {
    let k = m.get("t").and_then(Value::as_str).unwrap_or("");
    let size = game.size();
    match k {
        "st" => {
            let alive = game.players.get(&id).map_or(false, |p| p.alive);
            if !alive {
                return;
            }
            let Some(Value::Array(a)) = m.get("p") else { return };
            if a.len() != 3 {
                return;
            }
            let Some(xyz) = a.iter().map(num).collect::<Option<Vec<f64>>>() else { return };
            let (x, y, z) = (xyz[0], xyz[1], xyz[2]);
            if !finite(&[x, y, z]) {
                return;
            }
            let yaw = match m.get("yaw") {
                None => Some(0.0),
                Some(v) => num(v),
            };
            let pitch = match m.get("pitch") {
                None => Some(0.0),
                Some(v) => num(v),
            };
            let (Some(yaw), Some(pitch)) = (yaw, pitch) else { return };
            if !yaw.is_finite() || pitch.is_nan() {
                return;
            }
            let x = x.max(0.0).min(size);
            let z = z.max(0.0).min(size);
            let y = y.max(game.g.at(x, z) - 1.0).min(400.0);
            if let Some(p) = game.players.get_mut(&id) {
                p.x = x;
                p.z = z;
                p.y = y;
                p.yaw = py_mod(yaw, std::f64::consts::TAU);
                p.pitch = pitch.min(1.5).max(-1.5);
            }
        }
        "fire" => {
            let (Some(o), Some(d)) = (m.get("o"), m.get("d")) else { return };
            let (Some(o), Some(d)) = (vec3(Some(o), [0.0; 3]), vec3(Some(d), [0.0; 3])) else { return };
            let rt = match m.get("rt") {
                None => clock(),
                Some(v) => match num(v) {
                    Some(r) => r,
                    None => return,
                },
            };
            if finite(&o) && finite(&d) && rt.is_finite() {
                game.fire(id, o, d, rt);
            }
        }
        "launch" | "ms" | "boom" => {
            let Some(d) = vec3(m.get("d"), [0.0, 0.0, -1.0]) else { return };
            let Some(o) = vec3(m.get("p"), [0.0, 0.0, 0.0]) else { return };
            if !finite(&o) || !finite(&d) {
                return;
            }
            match k {
                "launch" => {
                    let kind = if m.get("k").and_then(Value::as_str) == Some("drone") { Kind::Drone } else { Kind::Missile };
                    game.launch(id, d, kind);
                }
                "ms" => game.missile_at(id, o, d),
                _ => game.explode(id, Some(o), false),
            }
        }
        "ping" => {
            let c = m.get("c").cloned().unwrap_or(Value::Null);
            game.send(id, &ServerMsg::Pong { c, ts: lasertag_shared::round_to(clock(), 3) });
        }
        "rename" => {
            if let Some(p) = game.players.get_mut(&id) {
                p.name = clean_name(m.get("name"), &p.name.clone());
            }
            game.roster_dirty = true;
        }
        _ => {}
    }
}

async fn next_text<S>(rx: &mut S) -> Option<String>
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match rx.next().await? {
            Ok(Message::Text(t)) => return Some(t.to_string()),
            Ok(Message::Binary(_)) => return Some(String::new()),
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => continue,
        }
    }
}

async fn handler<S>(stream: S, shared: Shared)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut cfg = WebSocketConfig::default();
    cfg.max_message_size = Some(4096);
    cfg.max_frame_size = Some(4096);
    let Ok(ws) = tokio_tungstenite::accept_async_with_config(stream, Some(cfg)).await else { return };
    let (mut sink, mut rx) = ws.split();
    // the first message has to be a join, within 30 s
    let Ok(Some(first)) = tokio::time::timeout(Duration::from_secs(30), next_text(&mut rx)).await else { return };
    let Ok(msg) = serde_json::from_str::<Value>(&first) else { return };
    if msg.get("t").and_then(Value::as_str) != Some("join") {
        return;
    }
    let full = lock(&shared).humans() >= MAX_HUMANS;
    if full {
        let _ = sink.send(Message::text(ServerMsg::Full {}.to_json())).await;
        let _ = sink.close().await;
        return;
    }
    let (tx, mut out) = mpsc::channel::<Message>(1024);
    let id = {
        let mut g = lock(&shared);
        let color = match msg.get("color").and_then(Value::as_str) {
            Some(c) if PALETTE.contains(&c) => c.to_string(),
            _ => PALETTE[g.rng.randbelow(PALETTE.len())].to_string(),
        };
        let fallback = format!("Player {}", g.next_id);
        let p = g.new_player(clean_name(msg.get("name"), &fallback), color, Some(tx), false);
        let (id, name) = (p.id, p.name.clone());
        let welcome = g.welcome(id);
        if let Some(c) = &p.conn {
            let _ = c.try_send(Message::text(welcome.to_json()));
        }
        g.add(p);
        g.manage_bots();
        g.broadcast(&ServerMsg::Feed { msg: format!("{} joined", name) });
        let roster = g.roster();
        g.send(id, &roster);
        id
    };
    // writing: everything queued for this player, and a ping every 20 s
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(20));
        ping.tick().await;
        loop {
            tokio::select! {
                m = out.recv() => match m {
                    Some(m) => if sink.send(m).await.is_err() { break },
                    None => break,
                },
                _ = ping.tick() => if sink.send(Message::Ping(Vec::new().into())).await.is_err() { break },
            }
        }
        let _ = sink.close().await;
    });
    // reading: 60 messages a second at most (bursts of 80), and a client silent for 45 s is gone
    let mut budget = 80.0f64;
    let mut last = Instant::now();
    loop {
        let item = tokio::time::timeout(Duration::from_secs(45), rx.next()).await;
        let raw = match item {
            Ok(Some(Ok(Message::Text(t)))) => Some(t),
            Ok(Some(Ok(Message::Binary(_)))) => None,
            Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
            Ok(Some(Ok(_))) => continue, // ping, pong
        };
        if writer.is_finished() {
            break; // its queue was dropped (too slow) or the socket failed
        }
        let n = Instant::now();
        budget = (budget + n.duration_since(last).as_secs_f64() * 60.0).min(80.0);
        last = n;
        if budget < 1.0 {
            continue;
        }
        budget -= 1.0;
        let Some(raw) = raw else { continue };
        if raw.chars().count() > 600 {
            continue;
        }
        let Ok(m) = serde_json::from_str::<Value>(raw.as_str()) else { continue };
        if !m.is_object() {
            continue;
        }
        let mut g = lock(&shared);
        if !g.players.contains_key(&id) {
            break;
        }
        handle_msg(&mut g, id, &m);
    }
    {
        let mut g = lock(&shared);
        if let Some(p) = g.remove(id) {
            g.broadcast(&ServerMsg::Feed { msg: format!("{} left", p.name) });
        }
        g.manage_bots();
    }
    let _ = writer.await;
}

async fn tick_loop(shared: Shared) {
    let mut iv = tokio::time::interval(Duration::from_millis(50));
    iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last = Instant::now();
    loop {
        iv.tick().await;
        let t = Instant::now();
        let dt = t.duration_since(last).as_secs_f64().min(0.25);
        last = t;
        let s = shared.clone();
        // a bug in one tick must not stop the game
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lock(&s).tick(dt)));
        if let Err(e) = r {
            let what = e.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| e.downcast_ref::<String>().cloned());
            eprintln!("tick error: {}", what.unwrap_or_else(|| "panic".into()));
        }
    }
}

fn load_ground(web: &PathBuf) -> Ground {
    let meta: GroundMeta = serde_json::from_slice(&std::fs::read(web.join("terrain.json")).expect("read terrain.json"))
        .expect("parse terrain.json");
    let raw = std::fs::read(web.join("terrain.bin")).expect("read terrain.bin");
    Ground::from_bytes(meta, &raw).expect("terrain.bin")
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let _ = clock();
    let web: PathBuf = std::env::var("LASERTAG_WEB").map(PathBuf::from).unwrap_or_else(|_| {
        let exe = std::env::current_exe().unwrap_or_default();
        exe.parent().map(|p| p.join("../web/laser-tag")).unwrap_or_else(|| PathBuf::from("web/laser-tag"))
    });
    let g = load_ground(&web);
    let arena = arena::build_arena(&g, 415);
    let (np, ns) = (arena.pillars.len(), arena.spots.len());
    let game = Game::new(
        g,
        arena,
        env_num("LASERTAG_ROUND_S", 300.0).trunc(),
        env_num("LASERTAG_INTERMISSION_S", 12.0).trunc(),
        env_num("LASERTAG_BOTS", 0.0).max(0.0) as usize,
    );
    let shared: Shared = Arc::new(Mutex::new(game));
    let where_ = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:8803".into());
    tokio::spawn(tick_loop(shared.clone()));
    println!("laser tag on {} pillars {} spots {}", where_, np, ns);
    if where_.starts_with('/') {
        let _ = std::fs::remove_file(&where_);
        let listener = tokio::net::UnixListener::bind(&where_).expect("bind unix socket");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&where_, std::fs::Permissions::from_mode(0o666)).expect("chmod socket");
        }
        loop {
            if let Ok((s, _)) = listener.accept().await {
                tokio::spawn(handler(s, shared.clone()));
            }
        }
    } else {
        let (host, port) = where_.rsplit_once(':').unwrap_or(("127.0.0.1", where_.as_str()));
        let host = if host.is_empty() { "127.0.0.1" } else { host };
        let listener = tokio::net::TcpListener::bind((host, port.parse::<u16>().expect("port"))).await.expect("bind");
        loop {
            if let Ok((s, _)) = listener.accept().await {
                let _ = s.set_nodelay(true);
                tokio::spawn(handler(s, shared.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web/laser-tag")
    }

    /// The arena must come out exactly as ref/arena.json, on every machine. If you change the arena on purpose,
    /// regenerate it: `cargo test -p lasertag-server dump_arena -- --ignored` writes target/arena.json; copy that
    /// over ref/arena.json (or delete this test).
    #[test]
    fn arena_is_unchanged() {
        let g = load_ground(&web());
        let a = arena::build_arena(&g, 415);
        let want: Value =
            serde_json::from_slice(&std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../ref/arena.json")).unwrap())
                .unwrap();
        let same = |got: Value, want: &Value, what: &str| {
            let (ga, wa) = (got.as_array().unwrap(), want.as_array().unwrap());
            assert_eq!(ga.len(), wa.len(), "{} count", what);
            for (i, (x, y)) in ga.iter().zip(wa).enumerate() {
                let close = |p: &Value, q: &Value| match (p, q) {
                    (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
                    _ => p == q,
                };
                let ok = match (x, y) {
                    (Value::Array(p), Value::Array(q)) => p.len() == q.len() && p.iter().zip(q).all(|(a, b)| close(a, b)),
                    (Value::Object(p), Value::Object(q)) => p.len() == q.len() && p.iter().all(|(k, a)| q.get(k).map_or(false, |b| close(a, b))),
                    _ => false,
                };
                assert!(ok, "{} {} differs: {} vs {}", what, i, x, y);
            }
        };
        same(serde_json::to_value(&a.boxes).unwrap(), &want["boxes"], "box");
        same(serde_json::to_value(&a.windows).unwrap(), &want["windows"], "window");
        same(serde_json::to_value(&a.pillars).unwrap(), &want["pillars"], "pillar");
        same(serde_json::to_value(&a.spots).unwrap(), &want["spots"], "spot");
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name(Some(&Value::String("  Hi\u{0007}there  ".into())), "x"), "Hithere");
        assert_eq!(clean_name(Some(&Value::String("a".repeat(30))), "x"), "a".repeat(16));
        assert_eq!(clean_name(None, "Player 3"), "Player 3");
        assert_eq!(clean_name(Some(&Value::String("\u{200b}".into())), "P"), "P");
    }

    #[test]
    fn hit_maths() {
        // a ray straight through a standing body's spine
        let (d, s) = game::seg_ray_dist([0.0, 3.0, 10.0], [0.0, 0.0, -1.0], 100.0, [0.0, 1.4, 0.0], [0.0, 5.2, 0.0]);
        assert!(d < 1e-9 && (s - 10.0).abs() < 1e-9);
    }
}

/// Writes the arena to target/arena.json (run with --ignored), to refresh ref/arena.json after an arena change.
#[cfg(test)]
mod dump {
    #[test]
    #[ignore]
    fn dump_arena() {
        let g = super::load_ground(&std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web/laser-tag"));
        let a = super::arena::build_arena(&g, 415);
        let v = serde_json::json!({"pillars": a.pillars, "spots": a.spots, "boxes": a.boxes, "windows": a.windows});
        std::fs::write(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/arena.json"), v.to_string()).unwrap();
    }
}
