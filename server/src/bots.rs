//! Bots: they top the arena up to LASERTAG_BOTS fighters (0 by default, so normally there are none).

use lasertag_shared::EYE;
use std::f64::consts::PI;

use crate::game::{wrap_angle, Ai, Game, RAPID_CD};

fn los(game: &Game, a: u32, b: u32) -> bool {
    let (Some(a), Some(b)) = (game.players.get(&a), game.players.get(&b)) else { return false };
    let o = [a.x, a.y + EYE, a.z];
    let d = [b.x - o[0], b.y + 3.5 - o[1], b.z - o[2]];
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if l < 1e-3 {
        return true;
    }
    let d = [d[0] / l, d[1] / l, d[2] / l];
    game.g.ray(o, d, l).is_none() && game.arena.solids.ray(o, d, l).is_none()
}

pub fn think(game: &mut Game, id: u32, dt: f64, t: f64) {
    let size = game.size();
    let inter = game.intermission_until != 0.0;
    let Some(b) = game.players.get(&id) else { return };
    if !b.alive || inter {
        return;
    }
    let (bx, bz) = (b.x, b.z);
    if b.ai.is_none() {
        let next_fire = t + game.rng.uniform(0.6, 1.4);
        let skill = game.rng.uniform(0.75, 1.25);
        game.players.get_mut(&id).unwrap().ai = Some(Ai {
            target: None,
            recheck: 0.0,
            wp: None,
            wp_until: 0.0,
            strafe: 1.0,
            strafe_t: 0.0,
            next_fire,
            stuck_t: 0.0,
            lastpos: (bx, bz),
            skill,
        });
    }
    // pick a target now and then
    let recheck = game.players[&id].ai.as_ref().unwrap().recheck;
    if t >= recheck {
        let next = t + game.rng.uniform(0.35, 0.7);
        let threat = game.players[&id].threat;
        let mut cands: Vec<(f64, u32)> = Vec::new();
        for p in game.players.values() {
            if p.id == id || !p.alive {
                continue;
            }
            let dist = (p.x - bx).hypot(p.z - bz);
            if dist > 480.0 {
                continue;
            }
            let mut bias = if threat == Some(p.id) { -120.0 } else { 0.0 };
            bias += if !p.bot { -40.0 } else { 0.0 };
            cands.push((dist + bias, p.id));
        }
        cands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut target = None;
        for &(_, pid) in cands.iter().take(3) {
            if los(game, id, pid) {
                target = Some(pid);
                break;
            }
        }
        let ai = game.players.get_mut(&id).unwrap().ai.as_mut().unwrap();
        ai.recheck = next;
        ai.target = target;
    }
    let target = game.players[&id].ai.as_ref().unwrap().target;
    let mut tgt = target.and_then(|tid| game.players.get(&tid)).map(|p| (p.id, p.x, p.y, p.z, p.alive));
    if let Some((_, _, _, _, false)) = tgt {
        tgt = None;
        game.players.get_mut(&id).unwrap().ai.as_mut().unwrap().target = None;
    }
    let b = &game.players[&id];
    let mut speed = 19.0 * if b.powers.speed > t { 1.5 } else { 1.0 };
    if game.g.water(b.x, b.z) {
        speed *= 0.6;
    }
    let (mx, mz);
    if let Some((_, tx, ty, tz, _)) = tgt {
        let (dx, dz) = (tx - b.x, tz - b.z);
        let mut dist = dx.hypot(dz);
        if dist == 0.0 {
            dist = 1.0;
        }
        let want_yaw = dx.atan2(-dz);
        let want_pitch = ((ty + 3.5) - (b.y + EYE)).atan2(dist);
        let dy = wrap_angle(want_yaw - b.yaw);
        let rapid = b.powers.rapid > t;
        let strafe_due = t > b.ai.as_ref().unwrap().strafe_t;
        let (new_strafe, strafe_until) = if strafe_due {
            (if game.rng.randbelow(2) == 0 { -1.0 } else { 1.0 }, Some(t + game.rng.uniform(0.6, 1.8)))
        } else {
            (0.0, None)
        };
        let bm = game.players.get_mut(&id).unwrap();
        let ai = bm.ai.as_mut().unwrap();
        let turn = 3.2 * ai.skill * dt;
        bm.yaw += dy.max(-turn).min(turn);
        bm.pitch += (want_pitch - bm.pitch) * (6.0 * dt).min(1.0);
        if let Some(u) = strafe_until {
            ai.strafe = new_strafe;
            ai.strafe_t = u;
        }
        let fwd = if dist > 170.0 { 1.0 } else if dist < 60.0 { -1.0 } else { 0.0 };
        let (ux, uz) = (dx / dist, dz / dist);
        mx = ux * fwd + (-uz) * ai.strafe * 0.8;
        mz = uz * fwd + ux * ai.strafe * 0.8;
        if dy.abs() < 0.12 && t >= ai.next_fire {
            let skill = ai.skill;
            let cd = if rapid { RAPID_CD * 2.2 } else { game.rng.uniform(0.6, 1.05) };
            let bm = game.players.get_mut(&id).unwrap();
            bm.ai.as_mut().unwrap().next_fire = t + cd / skill;
            let err = (0.03 + dist / 9000.0) / skill;
            let (yaw0, pitch0, x, y, z) = (bm.yaw, bm.pitch, bm.x, bm.y, bm.z);
            let yaw = yaw0 + game.rng.gauss(0.0, err);
            let pit = pitch0 + game.rng.gauss(0.0, err * 0.7);
            let d = [yaw.sin() * pit.cos(), pit.sin(), -yaw.cos() * pit.cos()];
            game.fire(id, [x, y + EYE, z], d, t);
        }
    } else {
        let ai = b.ai.as_ref().unwrap();
        let mut wp = ai.wp;
        let renew = match wp {
            None => true,
            Some((wx, wz)) => (wx - b.x).hypot(wz - b.z) < 12.0 || t > ai.wp_until,
        };
        if renew {
            let ups: Vec<(f64, f64)> = game.spots().iter().filter(|s| s.up).map(|s| (s.x, s.z)).collect();
            if !ups.is_empty() && game.rng.random() < 0.5 {
                wp = Some(ups[game.rng.randbelow(ups.len())]);
            } else {
                wp = Some((game.rng.uniform(60.0, size - 60.0), game.rng.uniform(60.0, size - 60.0)));
            }
            let ai = game.players.get_mut(&id).unwrap().ai.as_mut().unwrap();
            ai.wp = wp;
            ai.wp_until = t + 25.0;
        }
        let (wx, wz) = wp.unwrap();
        let bm = game.players.get_mut(&id).unwrap();
        let (dx, dz) = (wx - bm.x, wz - bm.z);
        let mut dist = dx.hypot(dz);
        if dist == 0.0 {
            dist = 1.0;
        }
        mx = dx / dist;
        mz = dz / dist;
        let want_yaw = mx.atan2(-mz);
        let dy = wrap_angle(want_yaw - bm.yaw);
        bm.yaw += dy.max(-3.0 * dt).min(3.0 * dt);
        bm.pitch *= 0.9;
    }
    let m = mx.hypot(mz);
    if m > 1e-6 {
        let b = &game.players[&id];
        let (mut nx, mut nz) = (b.x + mx / m * speed * dt, b.z + mz / m * speed * dt);
        if game.g.at(nx, nz) - b.y > 2.2 * speed * dt + 0.5 {
            // too steep to walk straight up: slide sideways
            nx = b.x + (-mz / m) * speed * dt * 0.7;
            nz = b.z + (mx / m) * speed * dt * 0.7;
        }
        let (x, z) = game.arena.solids.push_out(nx, nz, b.y);
        let y = game.g.at(x, z);
        let bm = game.players.get_mut(&id).unwrap();
        bm.x = x;
        bm.z = z;
        bm.y = y;
    }
    let bm = game.players.get_mut(&id).unwrap();
    let (x, z) = (bm.x, bm.z);
    let ai = bm.ai.as_mut().unwrap();
    if t > ai.stuck_t {
        ai.stuck_t = t + 2.0;
        if (x - ai.lastpos.0).hypot(z - ai.lastpos.1) < 6.0 && m > 0.0 {
            ai.wp = None;
            ai.strafe = -ai.strafe;
        }
        ai.lastpos = (x, z);
    }
    let _ = PI;
}
