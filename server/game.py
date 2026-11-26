"""Creek Laser Tag: the game server.

One arena, everyone in it. Clients move themselves and tell the server where they are; the server decides who a laser
hits (rewinding everyone to what the shooter saw), keeps score, runs rounds, power-ups and bots, and sends a snapshot
of everyone 20 times a second.

Run: python game.py /run/lasertag/ws.sock   (or host:port for a local test)
"""

import asyncio
import json
import math
import os
import random
import struct
import sys
import time
from collections import deque

import websockets

HERE = os.path.dirname(os.path.abspath(__file__))
WEB = os.environ.get("LASERTAG_WEB", os.path.join(HERE, "..", "web", "laser-tag"))

TICK = 1 / 20
ROUND_S = int(os.environ.get("LASERTAG_ROUND_S", 300))
INTERMISSION_S = int(os.environ.get("LASERTAG_INTERMISSION_S", 12))
MAX_HP = 3
RESPAWN_S = 3.0
SHIELD_S = 2.5
FIRE_CD = 0.50
RAPID_CD = 0.20
HEAT_SHOT = 0.20        # each shot heats the gun this much (1 = overheated)
HEAT_SHOT_RAPID = 0.08
HEAT_COOL = 0.20        # per second
OVERHEAT_S = 2.0        # locked out this long once it overheats
RANGE = 650.0
EYE = 5.4
HIT_R = 2.0
BODY = (1.4, 5.2)  # capsule spine, ft above the feet
REGEN_AFTER = 4.0
REGEN_EVERY = 2.5
POWER_S = 12.0
PICKUP_RESPAWN = 20.0
MAX_HUMANS = 16
FILL_TO = int(os.environ.get("LASERTAG_BOTS", 0))  # bots top the arena up to this many fighters; 0 = players only (PvP)
PALETTE = ["#ff3df2", "#22f0ff", "#ffe23d", "#7dff3d", "#ff8a1f", "#a66bff", "#ff4d6d", "#3dffb4"]
BOT_NAMES = ["Crawdad", "Heron", "Steelhead", "Otter", "Beaver", "Kingfisher", "Salamander", "Mudpuppy", "Dragonfly",
             "Bullfrog", "Coyote", "Raccoon"]
POWERS = {"rapid": 2, "over": 4, "speed": 8}
MISSILE_CD = 12.0       # seconds between missiles
MISSILE_SPEED = 95.0    # ft/s
MISSILE_LIFE = 8.0      # seconds of flight before it blows up by itself
BLAST_R = 18.0          # ft
MISSILE_DAMAGE = 2      # bars of health a blast takes within MISSILE_FULL_R of it
MISSILE_FULL_R = 10.0   # ft; from here out to BLAST_R it takes 1 bar
MISSILE_HIT_R = 3.0     # ft; a laser this close to a missile (or drone) shoots it down
DRONE_SPEED = 40.0      # ft/s, the most a drone can fly
DRONE_LIFE = 20.0       # seconds of battery
DRONE_BLAST_R = 20.0    # ft
DRONE_RINGS = ((8.0, 3), (14.0, 2), (DRONE_BLAST_R, 1))  # within 8 ft it tags you out, then 2 bars, then 1
SPOT_KINDS = ["rapid", "over", "speed", "drone"]  # a drone pick-up is one drone to fly, not a timed power
FX_SHIELD = 1

now = time.monotonic
T0 = now()


def clock():
    return now() - T0


# ---------------------------------------------------------------- the ground
class Ground:
    def __init__(self):
        meta = json.load(open(os.path.join(WEB, "terrain.json")))
        self.n, self.cell, self.size = meta["n"], meta["cell"], meta["size"]
        raw = open(os.path.join(WEB, "terrain.bin"), "rb").read()
        n2 = self.n * self.n
        self.h = [c / 100 for c in struct.unpack("<%dH" % n2, raw[: 2 * n2])]
        bits = raw[2 * n2:]
        self.wet = [(bits[i >> 3] >> (7 - (i & 7))) & 1 for i in range(n2)]

    def at(self, x, z):
        n, c = self.n, self.cell
        fx = min(max(x / c, 0), n - 1.001)
        fz = min(max(z / c, 0), n - 1.001)
        i, j = int(fx), int(fz)
        u, v = fx - i, fz - j
        h = self.h
        a, b = h[j * n + i], h[j * n + i + 1]
        cc, d = h[(j + 1) * n + i], h[(j + 1) * n + i + 1]
        return (a * (1 - u) + b * u) * (1 - v) + (cc * (1 - u) + d * u) * v

    def water(self, x, z):
        i = min(max(int(round(x / self.cell)), 0), self.n - 1)
        j = min(max(int(round(z / self.cell)), 0), self.n - 1)
        return self.wet[j * self.n + i]

    def slope(self, x, z):
        e = 4
        return math.hypot(self.at(x + e, z) - self.at(x - e, z), self.at(x, z + e) - self.at(x, z - e)) / (2 * e)

    def ray(self, o, d, tmax):
        """Distance along the ray to where it goes into the ground, or None."""
        step = 2.0
        t = step
        while t < tmax:
            x, y, z = o[0] + d[0] * t, o[1] + d[1] * t, o[2] + d[2] * t
            if x < 0 or z < 0 or x > self.size or z > self.size:
                return None
            if y < self.at(x, z):
                # back off to refine
                lo, hi = t - step, t
                for _ in range(6):
                    m = (lo + hi) / 2
                    if o[1] + d[1] * m < self.at(o[0] + d[0] * m, o[2] + d[2] * m):
                        hi = m
                    else:
                        lo = m
                return hi
            t += step
        return None


G = Ground()
SIZE = G.size


# ---------------------------------------------------------------- the arena: pillars and power-up spots
# ---------------------------------------------------------------- ruins: walls with windows, as boxes
BOXES = []    # [cx, cy, cz, hx, hy, hz, angle, tint]: a box turned `angle` about the vertical
WINDOWS = []  # [cx, cy, cz, half width, half height, angle]: an opening with broken glass, for the picture


def add_wall(x0, z0, x1, z1, base, H, T, openings, rnd, tint, ruin=True):
    """A wall from (x0, z0) to (x1, z1), standing on `base`, H tall and T thick, with openings
    [(along0, along1, y0, y1, glass)] cut out of it. A ruined wall has a ragged top and some lintels gone."""
    L = math.hypot(x1 - x0, z1 - z0)
    ux, uz = (x1 - x0) / L, (z1 - z0) / L
    a = math.atan2(-uz, ux)

    def box(s0, s1, y0, y1):
        if s1 - s0 < 0.3 or y1 - y0 < 0.3:
            return
        sm = (s0 + s1) / 2
        BOXES.append([round(x0 + ux * sm, 2), round(base + (y0 + y1) / 2, 2), round(z0 + uz * sm, 2),
                      round((s1 - s0) / 2, 2), round((y1 - y0) / 2, 2), round(T / 2, 2), round(a, 4), tint])

    def solid(s0, s1):
        x = s0
        while x < s1 - 0.01:
            e = min(s1, x + rnd.uniform(3, 6)) if ruin else s1
            if s1 - e < 1.5:
                e = s1
            box(x, e, 0, H - (rnd.choice([0, 0, 0, 0.6, 1.5, 3, 5]) if ruin else 0))
            x = e

    cur = 0.0
    for s0, s1, y0, y1, glass in sorted(openings):
        solid(cur, s0)
        box(s0, s1, 0, y0)
        if not (ruin and rnd.random() < 0.22):
            box(s0, s1, y1, H - (rnd.choice([0, 0, 0.8]) if ruin else 0))
        if glass:
            sm = (s0 + s1) / 2
            WINDOWS.append([round(x0 + ux * sm, 2), round(base + (y0 + y1) / 2, 2), round(z0 + uz * sm, 2),
                            round((s1 - s0) / 2, 2), round((y1 - y0) / 2, 2), round(a, 4)])
        cur = s1
    solid(cur, L)


def openings_for(L, rnd, door=False):
    n = max(0, int((L - 3) / 8))
    out = []
    for k in range(n):
        c = (k + 0.5) * L / n
        out.append([c - 2, c + 2, 3.5, 7.0, rnd.random() < 0.9])
    if door:
        if out:
            k = min(range(len(out)), key=lambda i: abs((out[i][0] + out[i][1]) / 2 - L / 2))
            c = (out[k][0] + out[k][1]) / 2
            out[k] = [c - 3, c + 3, 0.0, 8.0, False]
        elif L > 8:
            out.append([L / 2 - 3, L / 2 + 3, 0.0, 8.0, False])
    return [tuple(o) for o in out]


def ground_span(pts):
    hs = [G.at(x, z) for x, z in pts]
    return min(hs), max(hs), any(G.water(x, z) for x, z in pts)


def build_structures(rnd, taken):
    tints = [0, 1, 2, 3, 4, 5, 6, 7]

    def free(x, z, r, gap):
        return all(math.hypot(x - a, z - b) > r + c + gap for a, b, c in taken) and 40 < x < SIZE - 40 and 40 < z < SIZE - 40

    # buildings: four ruined walls round a room, a door in one
    made, tries = 0, 0
    while made < 9 and tries < 3000:
        tries += 1
        bx, bz = rnd.uniform(60, SIZE - 60), rnd.uniform(60, SIZE - 60)
        W, D = rnd.choice([22, 26, 30, 36, 40]), rnd.choice([18, 22, 26, 30])
        A = rnd.uniform(0, math.pi)
        R = math.hypot(W, D) / 2
        if not free(bx, bz, R, 35):
            continue
        ca, sa = math.cos(A), math.sin(A)

        def w(lx, lz):
            return bx + ca * lx + sa * lz, bz - sa * lx + ca * lz

        pts = [w(i * W / 2, j * D / 2) for i in (-1, 0, 1) for j in (-1, 0, 1)]
        lo, hi, wet = ground_span(pts)
        if wet or hi - lo > 4:
            continue
        base, H, T, tint = lo - 0.6, rnd.choice([10, 11, 12, 13]) + (hi - lo), 1.2, rnd.choice(tints)
        door = rnd.randrange(4)
        walls = [((-W / 2, -D / 2 + T / 2), (W / 2, -D / 2 + T / 2)), ((W / 2, D / 2 - T / 2), (-W / 2, D / 2 - T / 2)),
                 ((-W / 2 + T / 2, D / 2 - T), (-W / 2 + T / 2, -D / 2 + T)), ((W / 2 - T / 2, -D / 2 + T), (W / 2 - T / 2, D / 2 - T))]
        for k, (p0, p1) in enumerate(walls):
            (x0, z0), (x1, z1) = w(*p0), w(*p1)
            L = math.hypot(x1 - x0, z1 - z0)
            add_wall(x0, z0, x1, z1, base, H, T, openings_for(L, rnd, door=(k == door)), rnd, tint)
        taken.append((bx, bz, R))
        made += 1
    # lone broken walls with a window or two, and low barriers to crouch behind
    for kind, count in (("wall", 16), ("low", 14)):
        made, tries = 0, 0
        while made < count and tries < 3000:
            tries += 1
            cx, cz = rnd.uniform(50, SIZE - 50), rnd.uniform(50, SIZE - 50)
            L = rnd.uniform(16, 30) if kind == "wall" else rnd.uniform(10, 16)
            A = rnd.uniform(0, math.pi)
            if not free(cx, cz, L / 2, 18):
                continue
            x0, z0 = cx - math.cos(A) * L / 2, cz - math.sin(A) * L / 2
            x1, z1 = cx + math.cos(A) * L / 2, cz + math.sin(A) * L / 2
            pts = [(x0 + (x1 - x0) * f, z0 + (z1 - z0) * f) for f in (0, 0.25, 0.5, 0.75, 1)]
            lo, hi, wet = ground_span(pts)
            if wet or hi - lo > 3:
                continue
            if kind == "wall":
                add_wall(x0, z0, x1, z1, lo - 0.6, rnd.uniform(8, 10) + (hi - lo), 1.2, openings_for(L, rnd), rnd, rnd.choice(tints))
            else:
                add_wall(x0, z0, x1, z1, lo - 0.6, 4.0 + (hi - lo), 1.6, [], rnd, rnd.choice(tints), ruin=False)
            taken.append((cx, cz, L / 2))
            made += 1


def build_arena(seed=415):
    rnd = random.Random(seed)
    global BOXL
    taken = []
    build_structures(rnd, taken)
    BOXL = _boxes_local()
    pillars = []
    tries = 0
    while len(pillars) < 30 and tries < 5000:
        tries += 1
        x, z = rnd.uniform(60, SIZE - 60), rnd.uniform(60, SIZE - 60)
        r = rnd.choice([3, 4, 4, 5, 6, 8, 10])
        if G.water(x, z) or G.slope(x, z) > 0.5:
            continue
        if any(math.hypot(x - a, z - b) < c + r + 30 for a, b, c in taken):
            continue
        base = min(G.at(x + dx, z + dz) for dx in (-r, 0, r) for dz in (-r, 0, r)) - 1
        top = max(G.at(x + dx, z + dz) for dx in (-r, 0, r) for dz in (-r, 0, r)) + rnd.choice([9, 12, 14, 18, 24])
        pillars.append([round(x, 1), round(z, 1), r, round(base, 2), round(top, 2)])
        taken.append((x, z, r + 15))
    spots = []
    kinds = SPOT_KINDS
    tries = 0
    while len(spots) < 9 and tries < 5000:
        tries += 1
        x, z = rnd.uniform(120, SIZE - 120), rnd.uniform(120, SIZE - 120)
        if G.water(x, z) or G.slope(x, z) > 0.4 or in_box(x, z, 4):
            continue
        if any(math.hypot(x - p[0], z - p[1]) < p[2] + 12 for p in pillars):
            continue
        if any(math.hypot(x - s["x"], z - s["z"]) < 220 for s in spots):
            continue
        spots.append({"i": len(spots), "kind": kinds[len(spots) % len(kinds)], "x": round(x, 1), "z": round(z, 1),
                      "y": round(G.at(x, z), 2), "up": True, "back": 0.0})
    return pillars, spots


def _boxes_local():
    return [(cx, cy, cz, hx, hy, hz, math.cos(a), math.sin(a)) for cx, cy, cz, hx, hy, hz, a, _ in BOXES]


def in_box(x, z, margin):
    for cx, cy, cz, hx, hy, hz, c, s in BOXL:
        px, pz = x - cx, z - cz
        if abs(c * px - s * pz) < hx + margin and abs(s * px + c * pz) < hz + margin:
            return True
    return False


def box_ray(o, d, tmax):
    """Nearest distance along the ray into a wall box, or None."""
    best = None
    for cx, cy, cz, hx, hy, hz, c, s in BOXL:
        px, py, pz = o[0] - cx, o[1] - cy, o[2] - cz
        if px * px + pz * pz > (tmax + hx + hz) ** 2:
            continue
        lx, lz = c * px - s * pz, s * px + c * pz
        dx, dz = c * d[0] - s * d[2], s * d[0] + c * d[2]
        tn, tf = -1e18, 1e18
        ok = True
        for p, dd, h in ((lx, dx, hx), (py, d[1], hy), (lz, dz, hz)):
            if abs(dd) < 1e-9:
                if abs(p) > h:
                    ok = False
                    break
                continue
            t1, t2 = (-h - p) / dd, (h - p) / dd
            if t1 > t2:
                t1, t2 = t2, t1
            tn, tf = max(tn, t1), min(tf, t2)
            if tn > tf:
                ok = False
                break
        if not ok or tf < 0:
            continue
        t = max(tn, 0.0)
        if t < tmax and (best is None or t < best):
            best = t
    return best


BOXL = []  # BOXES with the turn as cos and sin, filled by build_arena
PILLARS, SPOTS = build_arena()


def solid_ray(o, d, tmax):
    """Nearest distance along the ray to anything solid built on the ground (pillars and walls), or None."""
    a = pillar_ray(o, d, tmax)
    b = box_ray(o, d, a if a is not None else tmax)
    return b if b is not None else a


def pillar_ray(o, d, tmax):
    """Nearest distance along the ray to a pillar, or None."""
    best = None
    for x, z, r, b, top in PILLARS:
        ox, oz = o[0] - x, o[2] - z
        a = d[0] * d[0] + d[2] * d[2]
        if a < 1e-9:
            continue
        bb = 2 * (ox * d[0] + oz * d[2])
        c = ox * ox + oz * oz - r * r
        disc = bb * bb - 4 * a * c
        if disc < 0:
            continue
        s = math.sqrt(disc)
        for t in ((-bb - s) / (2 * a), (-bb + s) / (2 * a)):
            if 0 < t < tmax and (best is None or t < best):
                y = o[1] + d[1] * t
                if b <= y <= top:
                    best = t
                    break
        if c < 0:  # inside the pillar's circle: blocked only if within its height
            if b <= o[1] <= top:
                best = 0.0
    # tops of pillars: a ray coming down onto one
    for x, z, r, b, top in PILLARS:
        if abs(d[1]) > 1e-6:
            t = (top - o[1]) / d[1]
            if 0 < t < tmax and (best is None or t < best):
                if math.hypot(o[0] + d[0] * t - x, o[2] + d[2] * t - z) <= r:
                    best = t
    return best


def push_out(x, z, y):
    for px, pz, r, b, top in PILLARS:
        if y >= top - 0.5:
            continue
        dx, dz = x - px, z - pz
        dd = math.hypot(dx, dz)
        lim = r + 1.6
        if dd < lim:
            if dd < 1e-3:
                dx, dz, dd = 1.0, 0.0, 1.0
            x, z = px + dx / dd * lim, pz + dz / dd * lim
    for cx, cy, cz, hx, hy, hz, c, s in BOXL:
        if y >= cy + hy - 0.35 or y + 5.8 <= cy - hy:
            continue
        px, pz = x - cx, z - cz
        lx, lz = c * px - s * pz, s * px + c * pz
        qx, qz = min(max(lx, -hx), hx), min(max(lz, -hz), hz)
        ex, ez = lx - qx, lz - qz
        dd = math.hypot(ex, ez)
        if dd >= 1.4:
            continue
        if dd > 1e-4:
            lx, lz = qx + ex / dd * 1.4, qz + ez / dd * 1.4
        elif hx - abs(lx) < hz - abs(lz):
            lx = math.copysign(hx + 1.4, lx or 1)
        else:
            lz = math.copysign(hz + 1.4, lz or 1)
        x, z = cx + c * lx + s * lz, cz - s * lx + c * lz
    return min(max(x, 2), SIZE - 2), min(max(z, 2), SIZE - 2)


def missile_at_time(m, rt):
    """Where a missile was at server time rt (the shooter sees it a little in the past, like everyone)."""
    h = m["h"]
    if rt >= h[-1][0]:
        return m["p"]
    for k in range(len(h) - 1, 0, -1):
        t1, t0 = h[k][0], h[k - 1][0]
        if t0 <= rt <= t1:
            f = (rt - t0) / max(t1 - t0, 1e-6)
            return tuple(h[k - 1][1][i] * (1 - f) + h[k][1][i] * f for i in range(3))
    return h[0][1]


def seg_ray_dist(o, d, tmax, a, b):
    """Closest approach between the ray o + d t (0..tmax) and segment a-b: (distance, t)."""
    u = d
    v = (b[0] - a[0], b[1] - a[1], b[2] - a[2])
    w = (o[0] - a[0], o[1] - a[1], o[2] - a[2])
    A = 1.0
    B = u[0] * v[0] + u[1] * v[1] + u[2] * v[2]
    C = v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
    D = u[0] * w[0] + u[1] * w[1] + u[2] * w[2]
    E = v[0] * w[0] + v[1] * w[1] + v[2] * w[2]
    den = A * C - B * B
    if den < 1e-9:
        s, t = 0.0, E / C
    else:
        s = (B * E - C * D) / den
        t = (A * E - B * D) / den
    s = min(max(s, 0.0), tmax)
    t = min(max((E + B * s) / C, 0.0), 1.0)
    s = min(max(B * t - D, 0.0), tmax)
    p = (o[0] + u[0] * s, o[1] + u[1] * s, o[2] + u[2] * s)
    q = (a[0] + v[0] * t, a[1] + v[1] * t, a[2] + v[2] * t)
    return math.dist(p, q), s


# ---------------------------------------------------------------- players
class Player:
    _next = 1

    def __init__(self, name, color, ws=None, bot=False):
        self.id = Player._next
        Player._next += 1
        self.name, self.color, self.ws, self.bot = name, color, ws, bot
        self.x = self.y = self.z = 0.0
        self.yaw = self.pitch = 0.0
        self.hp, self.alive = MAX_HP, False
        self.respawn_at = 0.0
        self.shield_until = 0.0
        self.powers = {}
        self.score = self.tags_taken = self.streak = self.best_streak = 0
        self.last_fire = -9.0
        self.heat = 0.0
        self.heat_t = 0.0
        self.locked_until = 0.0
        self.missile = None
        self.missile_ready = 0.0
        self.drones = 0
        self.last_hit = -9.0
        self.last_regen = 0.0
        self.hist = deque(maxlen=24)
        self.msgs = deque()
        self.ai = {}

    def flags(self, t):
        f = FX_SHIELD if t < self.shield_until else 0
        for k, bit in POWERS.items():
            if self.powers.get(k, 0) > t:
                f |= bit
        return f

    def at_time(self, rt):
        """Where this player stood at server time rt (for judging a shot the way the shooter saw it)."""
        h = self.hist
        if not h or rt >= h[-1][0]:
            return (self.x, self.y, self.z)
        for k in range(len(h) - 1, 0, -1):
            t1, t0 = h[k][0], h[k - 1][0]
            if t0 <= rt <= t1:
                f = (rt - t0) / max(t1 - t0, 1e-6)
                return tuple(h[k - 1][i + 1] * (1 - f) + h[k][i + 1] * f for i in range(3))
        return h[0][1:]

    def public(self):
        return {"id": self.id, "name": self.name, "color": self.color, "bot": self.bot, "score": self.score,
                "tagged": self.tags_taken, "best": self.best_streak}


class Game:
    def __init__(self):
        self.players = {}
        self.round_ends = clock() + ROUND_S
        self.intermission_until = 0.0
        self.roster_dirty = False
        self.round_no = 1

    # -- messaging
    def humans(self):
        return [p for p in self.players.values() if not p.bot]

    async def send(self, p, msg):
        if p.ws is None:
            return
        try:
            await p.ws.send(msg if isinstance(msg, str) else json.dumps(msg, separators=(",", ":")))
        except websockets.ConnectionClosed:
            pass

    def broadcast(self, msg):
        data = json.dumps(msg, separators=(",", ":"))
        conns = [p.ws for p in self.players.values() if p.ws is not None]
        if conns:
            websockets.broadcast(conns, data)

    def roster(self):
        return {"t": "roster", "players": [p.public() for p in self.players.values()]}

    # -- spawning
    def spawn_point(self, who):
        others = [p for p in self.players.values() if p.alive and p is not who]
        best, bestd = None, -1
        for _ in range(16):
            x, z = random.uniform(40, SIZE - 40), random.uniform(40, SIZE - 40)
            if G.water(x, z) or G.slope(x, z) > 0.6 or in_box(x, z, 2.5):
                continue
            x, z = push_out(x, z, G.at(x, z))
            dmin = min((math.hypot(x - o.x, z - o.z) for o in others), default=1e9)
            dmin = min(dmin, 600) + random.uniform(0, 80)
            if dmin > bestd:
                best, bestd = (x, z), dmin
        if best is None:
            best = (SIZE / 2, SIZE / 2)
        return best

    def spawn(self, p):
        x, z = self.spawn_point(p)
        p.x, p.z = x, z
        p.y = G.at(x, z)
        p.yaw = math.atan2(SIZE / 2 - x, -(SIZE / 2 - z)) % (2 * math.pi)  # facing the middle
        p.pitch = 0.0
        p.hp, p.alive = MAX_HP, True
        p.shield_until = clock() + SHIELD_S
        p.powers = {}
        p.drones = 0
        p.hist.clear()
        p.ai = {}
        p.missile = None
        self.broadcast({"t": "spawn", "id": p.id, "p": [round(p.x, 2), round(p.y, 2), round(p.z, 2)],
                        "yaw": round(p.yaw, 3)})

    # -- joining and leaving
    def add(self, p):
        self.players[p.id] = p
        self.spawn(p)
        self.roster_dirty = True

    def remove(self, p):
        if self.players.pop(p.id, None):
            self.roster_dirty = True
            self.broadcast({"t": "left", "id": p.id})

    def manage_bots(self):
        humans = len(self.humans())
        bots = [p for p in self.players.values() if p.bot]
        want = max(0, FILL_TO - humans) if humans else 0
        while len(bots) < want:
            used = {p.name for p in self.players.values()}
            name = next((n for n in BOT_NAMES if n not in used), "Bot %d" % random.randint(10, 99))
            used_c = [p.color for p in self.players.values()]
            color = min(PALETTE, key=lambda c: (used_c.count(c), random.random()))
            b = Player(name, color, bot=True)
            self.add(b)
            bots.append(b)
            self.broadcast({"t": "feed", "msg": "%s waded in" % name})
        while len(bots) > want:
            b = bots.pop()
            self.remove(b)

    # -- shooting
    def fire(self, shooter, o, d, rt):
        t = clock()
        if not shooter.alive or t < self.intermission_until:
            return
        rapid = shooter.powers.get("rapid", 0) > t
        cd = RAPID_CD if rapid else FIRE_CD
        if t - shooter.last_fire < cd * 0.75 or t < shooter.locked_until:
            return
        shooter.heat = max(0.0, shooter.heat - (t - shooter.heat_t) * HEAT_COOL)
        shooter.heat_t = t
        # a little slack against the client's own count, so a fair shot is never refused
        shooter.heat += (HEAT_SHOT_RAPID if rapid else HEAT_SHOT) * 0.7
        if shooter.heat >= 1.0:
            shooter.heat = 0.0
            shooter.locked_until = t + OVERHEAT_S * 0.85
        shooter.last_fire = t
        eye = (shooter.x, shooter.y + EYE, shooter.z)
        if math.dist(o, eye) > 14:
            o = eye
        dl = math.sqrt(d[0] ** 2 + d[1] ** 2 + d[2] ** 2) or 1
        d = (d[0] / dl, d[1] / dl, d[2] / dl)
        rt = min(max(rt, t - 0.5), t)  # judge the shot as the shooter saw it, up to half a second back
        tend = RANGE
        tg = G.ray(o, d, tend)
        if tg is not None:
            tend = tg
        tp = solid_ray(o, d, tend)
        if tp is not None:
            tend = tp
        hit, hit_t = None, tend
        for p in self.players.values():
            if p is shooter or not p.alive:
                continue
            x, y, z = p.at_time(rt)
            dist, s = seg_ray_dist(o, d, hit_t, (x, y + BODY[0], z), (x, y + BODY[1], z))
            if dist < HIT_R and s < hit_t:
                hit, hit_t = p, s
        # someone else's missile in the way: the laser shoots it down
        downed, down_at = None, None
        for p in self.players.values():
            if p is shooter or not p.missile:
                continue
            c = missile_at_time(p.missile, rt)
            v = (c[0] - o[0], c[1] - o[1], c[2] - o[2])
            s = v[0] * d[0] + v[1] * d[1] + v[2] * d[2]
            if 0 < s < hit_t and math.dist(v, (d[0] * s, d[1] * s, d[2] * s)) < MISSILE_HIT_R:
                downed, down_at, hit, hit_t = p, c, None, s
        end = (o[0] + d[0] * hit_t, o[1] + d[1] * hit_t, o[2] + d[2] * hit_t)
        over = shooter.powers.get("over", 0) > t
        self.broadcast({"t": "beam", "by": shooter.id, "o": [round(v, 2) for v in o], "e": [round(v, 2) for v in end],
                        "hit": hit.id if hit else 0, "over": 1 if over else 0, "mhit": downed.id if downed else 0})
        if downed:
            self.shoot_down(downed, shooter, downed.missile["p"])
        if hit:
            self.damage(hit, shooter, MAX_HP if over else 1)

    def shoot_down(self, owner, by, at):
        """A missile shot out of the sky: it goes off where it is, harmlessly."""
        kind = owner.missile["k"]
        owner.missile = None
        if kind == "missile":
            owner.missile_ready = clock() + MISSILE_CD
        self.broadcast({"t": "boom", "by": owner.id, "k": kind, "p": [round(v, 2) for v in at], "fizzle": 0, "down": by.id})

    _blast = False

    # -- missiles: the owner flies it (their client steers and says where it is); the server judges the blast
    def launch(self, p, d, kind="missile"):
        t = clock()
        if not p.alive or p.missile or self.intermission_until:
            return
        if kind == "drone":
            if p.drones < 1:
                return
            p.drones -= 1
        elif t < p.missile_ready:
            return
        dl = math.sqrt(d[0] ** 2 + d[1] ** 2 + d[2] ** 2) or 1
        d = (d[0] / dl, d[1] / dl, d[2] / dl)
        o = (p.x + d[0] * 3, p.y + EYE + d[1] * 3, p.z + d[2] * 3)
        p.missile = {"p": o, "d": d, "t0": t, "t": t, "h": deque([(t, o)], maxlen=24), "k": kind}
        self.broadcast({"t": "launch", "by": p.id, "k": kind, "p": [round(v, 2) for v in o]})

    def missile_at(self, p, o, d):
        m, t = p.missile, clock()
        if not m:
            return
        if math.dist(o, m["p"]) > (MISSILE_SPEED if m["k"] == "missile" else DRONE_SPEED) * (t - m["t"]) * 1.8 + 10:
            return  # too far, too fast: keep the last good place
        m["p"], m["d"], m["t"] = o, d, t
        m["h"].append((t, o))

    def explode(self, p, at=None, fizzle=False):
        m = p.missile
        if not m:
            return
        p.missile = None
        kind = m["k"]
        if kind == "missile":
            p.missile_ready = clock() + MISSILE_CD  # recharges from when it goes off
        o = m["p"] if at is None or math.dist(at, m["p"]) > 25 else at
        self.broadcast({"t": "boom", "by": p.id, "k": kind, "p": [round(v, 2) for v in o], "fizzle": 1 if fizzle else 0})
        if fizzle:
            return
        reach = DRONE_BLAST_R if kind == "drone" else BLAST_R
        self._blast = "d" if kind == "drone" else "m"
        try:
            for q in list(self.players.values()):
                if not q.alive:
                    continue
                c = (q.x, q.y + 3.0, q.z)
                dist = math.dist(o, c)
                if dist >= reach:
                    continue
                if dist > 1.0:
                    dd = tuple((c[i] - o[i]) / dist for i in range(3))
                    if G.ray(o, dd, dist - 0.8) is not None or solid_ray(o, dd, dist - 0.8) is not None:
                        continue
                if kind == "drone":
                    n = next(n for r, n in DRONE_RINGS if dist < r)
                else:
                    n = MISSILE_DAMAGE if dist < MISSILE_FULL_R else 1
                self.damage(q, p, n)
        finally:
            self._blast = False

    def damage(self, victim, by, n):
        t = clock()
        if t < victim.shield_until:
            self.broadcast({"t": "deflect", "id": victim.id, "by": by.id})
            return
        victim.hp -= n
        victim.last_hit = t
        victim.last_regen = t
        if victim.hp > 0:
            self.broadcast({"t": "hurt", "id": victim.id, "by": by.id, "hp": victim.hp, "w": self._blast or "l"})
            if victim.bot:
                victim.ai["threat"] = by.id
            return
        victim.alive = False
        victim.hp = 0
        victim.respawn_at = t + RESPAWN_S
        victim.tags_taken += 1
        victim.streak = 0
        if victim is not by:
            by.score += 1
            by.streak += 1
            by.best_streak = max(by.best_streak, by.streak)
        self.roster_dirty = True
        self.broadcast({"t": "tag", "v": victim.id, "k": by.id, "streak": by.streak, "w": self._blast or "l"})
        if victim.missile:
            self.explode(victim, fizzle=True)

    # -- every tick
    def tick(self, dt):
        t = clock()
        if self.intermission_until and t >= self.intermission_until:
            self.intermission_until = 0.0
            self.new_round()
        elif not self.intermission_until and t >= self.round_ends:
            self.end_round()
        for p in list(self.players.values()):
            if not p.alive:
                if t >= p.respawn_at and not self.intermission_until:
                    self.spawn(p)
                continue
            if p.hp < MAX_HP and t - p.last_hit > REGEN_AFTER and t - p.last_regen > REGEN_EVERY:
                p.hp += 1
                p.last_regen = t
            for s in SPOTS:
                if s["up"] and abs(p.x - s["x"]) < 5 and abs(p.z - s["z"]) < 5 and abs(p.y - s["y"]) < 8:
                    s["up"], s["back"] = False, t + PICKUP_RESPAWN
                    if s["kind"] == "drone":
                        p.drones = 1
                    else:
                        p.powers[s["kind"]] = t + POWER_S
                    self.broadcast({"t": "pick", "i": s["i"], "id": p.id, "kind": s["kind"],
                                    "until": 0 if s["kind"] == "drone" else POWER_S})
        for s in SPOTS:
            if not s["up"] and t >= s["back"]:
                s["up"] = True
                self.broadcast({"t": "spot", "i": s["i"], "up": True})
        for p in list(self.players.values()):
            if p.missile and t - p.missile["t0"] > (DRONE_LIFE if p.missile["k"] == "drone" else MISSILE_LIFE) + 1.0:
                self.explode(p)
        for p in self.players.values():
            if p.bot:
                bot_think(self, p, dt, t)
            p.hist.append((t, p.x, p.y, p.z))
        if self.roster_dirty:
            self.roster_dirty = False
            self.broadcast(self.roster())
        snap = [[p.id, round(p.x, 2), round(p.y, 2), round(p.z, 2), round(p.yaw, 3), round(p.pitch, 3), p.hp,
                 1 if p.alive else 0, p.flags(t)] for p in self.players.values()]
        ms = [[p.id] + [round(v, 2) for v in p.missile["p"]] + [round(v, 3) for v in p.missile["d"]] + [1 if p.missile["k"] == "drone" else 0]
              for p in self.players.values() if p.missile]
        self.broadcast({"t": "snap", "ts": round(t, 3), "p": snap, "m": ms})

    def standings(self):
        return sorted(self.players.values(), key=lambda p: (-p.score, p.tags_taken))

    def end_round(self):
        st = self.standings()
        self.intermission_until = clock() + INTERMISSION_S
        self.broadcast({"t": "round_end", "winner": st[0].id if st and st[0].score else 0,
                        "board": [p.public() for p in st], "next": INTERMISSION_S})

    def new_round(self):
        self.round_no += 1
        self.round_ends = clock() + ROUND_S
        for p in self.players.values():
            p.score = p.tags_taken = p.streak = p.best_streak = 0
            p.missile, p.missile_ready = None, 0.0
            self.spawn(p)
        for s in SPOTS:
            s["up"] = True
        self.roster_dirty = True
        self.broadcast({"t": "round", "left": ROUND_S, "no": self.round_no, "spots": SPOTS})

    def welcome(self, p):
        t = clock()
        return {"t": "welcome", "id": p.id, "ts": round(t, 3), "pillars": PILLARS, "spots": SPOTS, "boxes": BOXES, "windows": WINDOWS,
                "left": round(max(0, self.round_ends - t), 1),
                "inter": round(max(0, self.intermission_until - t), 1), "no": self.round_no,
                "rules": {"hp": MAX_HP, "cd": FIRE_CD, "rapid": RAPID_CD, "range": RANGE, "eye": EYE, "heat": HEAT_SHOT,
                          "heatRapid": HEAT_SHOT_RAPID, "cool": HEAT_COOL, "lock": OVERHEAT_S,
                          "respawn": RESPAWN_S, "mcd": MISSILE_CD, "mspeed": MISSILE_SPEED, "mlife": MISSILE_LIFE,
                          "blast": BLAST_R, "dspeed": DRONE_SPEED, "dlife": DRONE_LIFE, "dblast": DRONE_BLAST_R}}


# ---------------------------------------------------------------- bots
def los(a, b):
    o = (a.x, a.y + EYE, a.z)
    tx, ty, tz = b.x, b.y + 3.5, b.z
    d = (tx - o[0], ty - o[1], tz - o[2])
    L = math.sqrt(d[0] ** 2 + d[1] ** 2 + d[2] ** 2)
    if L < 1e-3:
        return True
    d = (d[0] / L, d[1] / L, d[2] / L)
    return G.ray(o, d, L) is None and solid_ray(o, d, L) is None


def bot_think(game, b, dt, t):
    if not b.alive or game.intermission_until:
        return
    ai = b.ai
    if not ai:
        ai.update(target=None, recheck=0, wp=None, strafe=1, strafe_t=0, next_fire=t + random.uniform(0.6, 1.4),
                  stuck=0, lastpos=(b.x, b.z), skill=random.uniform(0.75, 1.25))
    # pick a target now and then
    if t >= ai["recheck"]:
        ai["recheck"] = t + random.uniform(0.35, 0.7)
        cands = []
        for p in game.players.values():
            if p is b or not p.alive:
                continue
            dist = math.hypot(p.x - b.x, p.z - b.z)
            if dist > 480:
                continue
            bias = -120 if ai.get("threat") == p.id else 0
            bias += -40 if not p.bot else 0
            cands.append((dist + bias, p))
        cands.sort(key=lambda c: c[0])
        ai["target"] = None
        for _, p in cands[:3]:
            if los(b, p):
                ai["target"] = p.id
                break
    tgt = game.players.get(ai["target"]) if ai["target"] else None
    if tgt and not tgt.alive:
        tgt = ai["target"] = None
    speed = 19.0 * (1.5 if b.powers.get("speed", 0) > t else 1.0)
    if G.water(b.x, b.z):
        speed *= 0.6
    mx = mz = 0.0
    if tgt:
        dx, dz = tgt.x - b.x, tgt.z - b.z
        dist = math.hypot(dx, dz) or 1
        want_yaw = math.atan2(dx, -dz)
        want_pitch = math.atan2((tgt.y + 3.5) - (b.y + EYE), dist)
        dy = (want_yaw - b.yaw + math.pi) % (2 * math.pi) - math.pi
        turn = 3.2 * ai["skill"] * dt
        b.yaw += max(-turn, min(turn, dy))
        b.pitch += (want_pitch - b.pitch) * min(1, 6 * dt)
        if t > ai["strafe_t"]:
            ai["strafe"], ai["strafe_t"] = random.choice((-1, 1)), t + random.uniform(0.6, 1.8)
        fwd = 1 if dist > 170 else (-1 if dist < 60 else 0)
        ux, uz = dx / dist, dz / dist
        mx = ux * fwd + (-uz) * ai["strafe"] * 0.8
        mz = uz * fwd + ux * ai["strafe"] * 0.8
        if abs(dy) < 0.12 and t >= ai["next_fire"]:
            cd = RAPID_CD * 2.2 if b.powers.get("rapid", 0) > t else random.uniform(0.6, 1.05)
            ai["next_fire"] = t + cd / ai["skill"]
            err = (0.03 + dist / 9000) / ai["skill"]
            yaw = b.yaw + random.gauss(0, err)
            pit = b.pitch + random.gauss(0, err * 0.7)
            d = (math.sin(yaw) * math.cos(pit), math.sin(pit), -math.cos(yaw) * math.cos(pit))
            game.fire(b, (b.x, b.y + EYE, b.z), d, t)
    else:
        wp = ai["wp"]
        if wp is None or math.hypot(wp[0] - b.x, wp[1] - b.z) < 12 or t > ai.get("wp_until", 0):
            ups = [s for s in SPOTS if s["up"]]
            if ups and random.random() < 0.5:
                s = random.choice(ups)
                wp = (s["x"], s["z"])
            else:
                wp = (random.uniform(60, SIZE - 60), random.uniform(60, SIZE - 60))
            ai["wp"], ai["wp_until"] = wp, t + 25
        dx, dz = wp[0] - b.x, wp[1] - b.z
        dist = math.hypot(dx, dz) or 1
        mx, mz = dx / dist, dz / dist
        want_yaw = math.atan2(mx, -mz)
        dy = (want_yaw - b.yaw + math.pi) % (2 * math.pi) - math.pi
        b.yaw += max(-3 * dt, min(3 * dt, dy))
        b.pitch *= 0.9
    m = math.hypot(mx, mz)
    if m > 1e-6:
        nx, nz = b.x + mx / m * speed * dt, b.z + mz / m * speed * dt
        if G.at(nx, nz) - b.y > 2.2 * speed * dt + 0.5:  # too steep to walk straight up: slide sideways
            nx, nz = b.x + (-mz / m) * speed * dt * 0.7, b.z + (mx / m) * speed * dt * 0.7
        b.x, b.z = push_out(nx, nz, b.y)
        b.y = G.at(b.x, b.z)
    if t > ai.get("stuck_t", 0):
        ai["stuck_t"] = t + 2.0
        if math.hypot(b.x - ai["lastpos"][0], b.z - ai["lastpos"][1]) < 6 and m > 0:
            ai["wp"] = None
            ai["strafe"] = -ai["strafe"]
        ai["lastpos"] = (b.x, b.z)


# ---------------------------------------------------------------- connections
GAME = Game()


def clean_name(s, fallback):
    s = "".join(ch for ch in str(s or "") if ch.isprintable()).strip()[:16]
    return s or fallback


async def handler(ws):
    p = None
    try:
        first = await asyncio.wait_for(ws.recv(), timeout=30)
        msg = json.loads(first)
        if msg.get("t") != "join":
            return
        if len(GAME.humans()) >= MAX_HUMANS:
            await ws.send(json.dumps({"t": "full"}))
            return
        color = msg.get("color") if msg.get("color") in PALETTE else random.choice(PALETTE)
        p = Player(clean_name(msg.get("name"), "Player %d" % (Player._next)), color, ws=ws)
        await ws.send(json.dumps(GAME.welcome(p), separators=(",", ":")))
        GAME.add(p)
        GAME.manage_bots()
        GAME.broadcast({"t": "feed", "msg": "%s joined" % p.name})
        await ws.send(json.dumps(GAME.roster(), separators=(",", ":")))
        budget, last = 80.0, now()
        async for raw in ws:
            n = now()
            budget = min(80.0, budget + (n - last) * 60)
            last = n
            if budget < 1:
                continue
            budget -= 1
            if not isinstance(raw, str) or len(raw) > 600:
                continue
            try:
                m = json.loads(raw)
            except ValueError:
                continue
            k = m.get("t")
            if k == "st" and p.alive:
                try:
                    x, y, z = (float(v) for v in m["p"])
                    if not all(map(math.isfinite, (x, y, z))):
                        continue
                    p.x, p.z = min(max(x, 0), SIZE), min(max(z, 0), SIZE)
                    p.y = min(max(y, G.at(p.x, p.z) - 1), 400)
                    p.yaw, p.pitch = float(m.get("yaw", 0)) % (2 * math.pi), max(-1.5, min(1.5, float(m.get("pitch", 0))))
                except (KeyError, TypeError, ValueError):
                    continue
            elif k == "fire":
                try:
                    o = tuple(float(v) for v in m["o"])[:3]
                    d = tuple(float(v) for v in m["d"])[:3]
                    rt = float(m.get("rt", clock()))
                    if len(o) == 3 and len(d) == 3 and all(map(math.isfinite, o + d + (rt,))):
                        GAME.fire(p, o, d, rt)
                except (KeyError, TypeError, ValueError):
                    continue
            elif k in ("launch", "ms", "boom"):
                try:
                    d = tuple(float(v) for v in m.get("d", (0, 0, -1)))[:3]
                    o = tuple(float(v) for v in m.get("p", (0, 0, 0)))[:3]
                    if len(d) != 3 or len(o) != 3 or not all(map(math.isfinite, o + d)):
                        continue
                except (TypeError, ValueError):
                    continue
                if k == "launch":
                    GAME.launch(p, d, "drone" if m.get("k") == "drone" else "missile")
                elif k == "ms":
                    GAME.missile_at(p, o, d)
                else:
                    GAME.explode(p, o)
            elif k == "ping":
                await ws.send(json.dumps({"t": "pong", "c": m.get("c"), "ts": round(clock(), 3)}))
            elif k == "rename":
                p.name = clean_name(m.get("name"), p.name)
                GAME.roster_dirty = True
    except (asyncio.TimeoutError, websockets.ConnectionClosed, ValueError):
        pass
    finally:
        if p is not None:
            GAME.remove(p)
            GAME.broadcast({"t": "feed", "msg": "%s left" % p.name})
            GAME.manage_bots()


async def loop():
    last = now()
    while True:
        t = now()
        dt = min(t - last, 0.25)
        last = t
        try:
            GAME.tick(dt)
        except Exception as e:  # a bug in one tick must not stop the game
            print("tick error:", repr(e), file=sys.stderr, flush=True)
        await asyncio.sleep(max(0.0, TICK - (now() - t)))


async def main():
    where = sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1:8803"
    kw = dict(max_size=4096, ping_interval=20, ping_timeout=20, compression=None)
    if where.startswith("/"):
        if os.path.exists(where):
            os.unlink(where)
        server = await websockets.unix_serve(handler, where, **kw)
        os.chmod(where, 0o666)
    else:
        host, _, port = where.rpartition(":")
        server = await websockets.serve(handler, host or "127.0.0.1", int(port), **kw)
    print("laser tag on", where, "pillars", len(PILLARS), "spots", len(SPOTS), flush=True)
    async with server:
        await loop()


if __name__ == "__main__":
    asyncio.run(main())
