//! The messages, as JSON objects with a `t` field saying which one. The shape is the same as the original
//! Python server and JavaScript client used, so an old page left open still works with the new server.

use serde::{Deserialize, Serialize};

use crate::solids::{Pillar, WallBox, Window};

/// A power-up spot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    pub i: u32,
    pub kind: String,
    pub x: f64,
    pub z: f64,
    pub y: f64,
    pub up: bool,
    #[serde(default)]
    pub back: f64,
}

/// The numbers the client needs to play by the server's rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rules {
    pub hp: i32,
    pub cd: f64,
    pub rapid: f64,
    pub range: f64,
    pub eye: f64,
    pub heat: f64,
    #[serde(rename = "heatRapid")]
    pub heat_rapid: f64,
    pub cool: f64,
    pub lock: f64,
    pub respawn: f64,
    pub mcd: f64,
    pub mspeed: f64,
    pub mlife: f64,
    pub blast: f64,
    pub dspeed: f64,
    pub dlife: f64,
    pub dblast: f64,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            hp: 3,
            cd: 0.5,
            rapid: 0.2,
            range: 650.0,
            eye: 5.4,
            heat: 0.2,
            heat_rapid: 0.08,
            cool: 0.2,
            lock: 2.0,
            respawn: 3.0,
            mcd: 12.0,
            mspeed: 95.0,
            mlife: 8.0,
            blast: 18.0,
            dspeed: 40.0,
            dlife: 20.0,
            dblast: 20.0,
        }
    }
}

/// A player as the scoreboard shows them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Public {
    pub id: u32,
    pub name: String,
    pub color: String,
    pub bot: bool,
    pub score: i32,
    pub tagged: i32,
    pub best: i32,
}

/// One player in a snapshot: id, x, y, z, yaw, pitch, hp, alive (1/0), flags (1 shield, 2 rapid, 4 overcharge, 8 speed).
pub type SnapPlayer = (u32, f64, f64, f64, f64, f64, i32, u8, u32);
/// One missile or drone in a snapshot: owner id, x, y, z, heading dx, dy, dz, drone (1/0).
pub type SnapMissile = (u32, f64, f64, f64, f64, f64, f64, u8);

/// Server to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome {
        id: u32,
        ts: f64,
        pillars: Vec<Pillar>,
        spots: Vec<Spot>,
        boxes: Vec<WallBox>,
        windows: Vec<Window>,
        left: f64,
        inter: f64,
        no: u32,
        rules: Rules,
    },
    Full {},
    Roster { players: Vec<Public> },
    Left { id: u32 },
    Feed { msg: String },
    Spawn { id: u32, p: [f64; 3], yaw: f64 },
    Beam { by: u32, o: [f64; 3], e: [f64; 3], hit: u32, over: u8, mhit: u32 },
    Boom {
        by: u32,
        k: String,
        p: [f64; 3],
        fizzle: u8,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        down: Option<u32>,
    },
    Launch { by: u32, k: String, p: [f64; 3] },
    Deflect { id: u32, by: u32 },
    Hurt { id: u32, by: u32, hp: i32, w: String },
    Tag { v: u32, k: u32, streak: i32, w: String },
    Pick { i: u32, id: u32, kind: String, until: f64 },
    Spot { i: u32, up: bool },
    Snap { ts: f64, p: Vec<SnapPlayer>, m: Vec<SnapMissile> },
    RoundEnd { winner: u32, board: Vec<Public>, next: f64 },
    Round { left: f64, no: u32, spots: Vec<Spot> },
    Pong {
        #[serde(default)]
        c: serde_json::Value,
        ts: f64,
    },
}

impl ServerMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("messages always serialise")
    }
}

/// Client to server. The server reads these leniently from raw JSON (as the original did); the client builds them
/// with these types.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    Join { name: String, color: String },
    St { p: [f64; 3], yaw: f64, pitch: f64 },
    Fire { o: [f64; 3], d: [f64; 3], rt: f64 },
    Launch {
        d: [f64; 3],
        #[serde(skip_serializing_if = "Option::is_none")]
        k: Option<String>,
    },
    Ms { p: [f64; 3], d: [f64; 3] },
    Boom { p: [f64; 3] },
    Ping { c: f64 },
}

impl ClientMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("messages always serialise")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_match_the_original() {
        let m = ServerMsg::Boom { by: 3, k: "missile".into(), p: [1.0, 2.5, 3.0], fizzle: 0, down: None };
        assert_eq!(m.to_json(), r#"{"t":"boom","by":3,"k":"missile","p":[1.0,2.5,3.0],"fizzle":0}"#);
        let m = ServerMsg::RoundEnd { winner: 0, board: vec![], next: 12.0 };
        assert!(m.to_json().starts_with(r#"{"t":"round_end""#));
        let m: ServerMsg = serde_json::from_str(r#"{"t":"full"}"#).unwrap();
        assert!(matches!(m, ServerMsg::Full {}));
        let m: ServerMsg = serde_json::from_str(
            r#"{"t":"snap","ts":1.5,"p":[[1,2,3,4,0.5,0.1,3,1,0]],"m":[[1,2,3,4,0,0,-1,0]]}"#,
        )
        .unwrap();
        assert!(matches!(m, ServerMsg::Snap { .. }));
        assert_eq!(ClientMsg::Launch { d: [0.0, 0.0, -1.0], k: None }.to_json(), r#"{"t":"launch","d":[0.0,0.0,-1.0]}"#);
    }
}
