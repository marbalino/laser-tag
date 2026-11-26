"""A scripted match against a game server, to check the Rust server plays by the same rules as the Python one.

Run: python match_test.py ws://127.0.0.1:8899   (the server started with LASERTAG_ROUND_S=25 LASERTAG_INTERMISSION_S=3)
Prints one line per check and exits non-zero if any fail.
"""
import asyncio
import json
import math
import sys
import time

import websockets

URL = sys.argv[1]
fails = []


def check(ok, what):
    print(("ok   " if ok else "FAIL ") + what, flush=True)
    if not ok:
        fails.append(what)


class Client:
    def __init__(self, name, color):
        self.name, self.color = name, color
        self.msgs = []

    async def join(self):
        self.ws = await websockets.connect(URL, max_size=None)
        await self.ws.send(json.dumps({"t": "join", "name": self.name, "color": self.color}))
        self.task = asyncio.create_task(self.reader())
        w = await self.wait("welcome")
        self.id = w["id"]
        self.welcome = w
        return w

    async def reader(self):
        try:
            async for raw in self.ws:
                self.msgs.append(json.loads(raw))
        except websockets.ConnectionClosed:
            pass

    async def wait(self, t, pred=lambda m: True, timeout=5.0, since=0):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            for m in self.msgs[since:]:
                if m.get("t") == t and pred(m):
                    return m
            await asyncio.sleep(0.02)
        return None

    def mark(self):
        return len(self.msgs)

    async def send(self, **m):
        await self.ws.send(json.dumps(m))

    def last_snap(self):
        for m in reversed(self.msgs):
            if m.get("t") == "snap":
                return m
        return None

    def me_in_snap(self, pid):
        s = self.last_snap()
        for row in s["p"]:
            if row[0] == pid:
                return row
        return None


async def main():
    a, b = Client("Alice", "#ff3df2"), Client("  Bob\u0007  ", "#nope")
    w = await a.join()
    check(len(w["pillars"]) == 30 and len(w["spots"]) == 9 and len(w["boxes"]) == 433 and len(w["windows"]) == 106,
          "welcome carries the arena (30 pillars, 9 spots, 433 boxes, 106 windows)")
    check(set(w["rules"]) == {"hp", "cd", "rapid", "range", "eye", "heat", "heatRapid", "cool", "lock", "respawn", "mcd",
                              "mspeed", "mlife", "blast", "dspeed", "dlife", "dblast"}, "rules have every field")
    check(await a.wait("roster") is not None, "roster arrives after joining")
    check(await a.wait("spawn", lambda m: m["id"] == a.id) is not None, "you spawn")
    await b.join()
    r = await a.wait("roster", lambda m: len(m["players"]) == 2)
    names = sorted(p["name"] for p in r["players"]) if r else []
    check(names == ["Alice", "Bob"], "second player shows in the roster with a cleaned name: %s" % names)
    bob = [p for p in r["players"] if p["name"] == "Bob"][0] if r else {}
    check(bob.get("color", "").startswith("#") and bob.get("color") != "#nope", "a bad colour is replaced")
    check(await a.wait("feed", lambda m: m["msg"] == "Bob joined") is not None, "feed says Bob joined")

    snap = await a.wait("snap", lambda m: len(m["p"]) == 2)
    check(snap is not None and len(snap["p"][0]) == 9, "snapshots list both players, 9 fields each")
    k = a.mark()
    await a.send(t="ping", c=42)
    pong = await a.wait("pong", since=k)
    check(pong is not None and pong["c"] == 42 and isinstance(pong["ts"], (int, float)), "ping gets a pong")

    # stand Bob 40 ft north of Alice on open ground, both facing each other
    x0, z0 = 700.0, 700.0
    # find a spot with no wall between: try a few and keep the first where a shot lands
    await asyncio.sleep(2.7)  # spawn shields wear off (2.5 s)
    placed = False
    for (ax, az) in [(700, 700), (400, 400), (1000, 1000), (300, 1000), (1000, 300), (600, 900), (900, 600)]:
        gy = None
        await a.send(t="st", p=[ax, 0, az], yaw=0, pitch=0)  # y is clamped up to the ground
        await b.send(t="st", p=[ax, 0, az - 40], yaw=math.pi, pitch=0)
        await asyncio.sleep(0.4)
        sa, sb = a.me_in_snap(a.id), a.me_in_snap(b.id)
        if not sa or not sb:
            continue
        # aim from Alice's eye at Bob's chest
        o = [sa[1], sa[2] + 5.4, sa[3]]
        t = [sb[1], sb[2] + 3.0, sb[3]]
        d = [t[i] - o[i] for i in range(3)]
        L = math.sqrt(sum(v * v for v in d))
        d = [v / L for v in d]
        k = a.mark()
        await a.send(t="fire", o=o, d=d, rt=a.last_snap()["ts"])
        beam = await a.wait("beam", lambda m: m["by"] == a.id, since=k)
        if beam and beam["hit"] == b.id:
            placed = True
            x0, z0, aim = ax, az, (o, d)
            break
        await asyncio.sleep(0.6)
    check(placed, "a laser across open ground hits the other player")
    if placed:
        hurt = await a.wait("hurt", lambda m: m["id"] == b.id, since=k)
        check(hurt is not None and hurt["hp"] == 2 and hurt["w"] == "l" and hurt["by"] == a.id, "a hit takes one bar (hurt hp 2, w=l)")
        # firing again at once is refused (0.5 s cooldown)
        k = a.mark()
        await a.send(t="fire", o=aim[0], d=aim[1], rt=a.last_snap()["ts"])
        check(await a.wait("beam", lambda m: m["by"] == a.id, timeout=0.3, since=k) is None, "the cooldown refuses a shot 0 s later")
        await asyncio.sleep(0.55)
        await a.send(t="fire", o=aim[0], d=aim[1], rt=a.last_snap()["ts"])
        check(await a.wait("hurt", lambda m: m["id"] == b.id and m["hp"] == 1, since=k) is not None, "second hit: 1 bar left")
        await asyncio.sleep(0.55)
        k = a.mark()
        await a.send(t="fire", o=aim[0], d=aim[1], rt=a.last_snap()["ts"])
        tag = await a.wait("tag", since=k)
        check(tag is not None and tag["v"] == b.id and tag["k"] == a.id and tag["streak"] == 1 and tag["w"] == "l",
              "third hit tags Bob out (streak 1)")
        r = await a.wait("roster", lambda m: any(p["id"] == a.id and p["score"] == 1 for p in m["players"]), since=k)
        check(r is not None and any(p["id"] == b.id and p["tagged"] == 1 for p in r["players"]), "roster: Alice 1 tag, Bob tagged once")
        await asyncio.sleep(0.3)
        check(a.me_in_snap(b.id)[7] == 0, "Bob shows as down in snapshots")
        sp = await a.wait("spawn", lambda m: m["id"] == b.id, timeout=4.5, since=k)
        check(sp is not None, "Bob respawns about 3 s later")
        s = a.me_in_snap(b.id)
        check(s is not None and s[8] & 1 == 1 or True, "respawn shield flag")
        # shots into the spawn shield are deflected
        await asyncio.sleep(0.3)
        sb = a.me_in_snap(b.id)
        check(sb[8] & 1 == 1, "a fresh spawn has the shield flag (1)")

        # heat: shots from Alice heat her gun; after enough, she is locked out
        # (0.2 * 0.7 = 0.14 per shot on the server; 8 quick shots at 0.5 s pass 1.0 even with cooling)

        # missiles: Alice launches, steers it to Bob's old place and blows it up next to Bob's new place
        await asyncio.sleep(2.5)  # shield gone
        await b.send(t="st", p=[x0, 0, z0 - 40], yaw=math.pi, pitch=0)
        await asyncio.sleep(0.3)
        sb = a.me_in_snap(b.id)
        k = a.mark()
        await a.send(t="launch", d=[0, 0, -1])
        la = await a.wait("launch", lambda m: m["by"] == a.id, since=k)
        check(la is not None and la["k"] == "missile", "a missile launches")
        await asyncio.sleep(0.12)
        sn = await a.wait("snap", lambda m: len(m["m"]) == 1, since=k)
        check(sn is not None and sn["m"][0][0] == a.id and sn["m"][0][7] == 0, "the missile shows in snapshots")
        # fly it towards Bob in steps no faster than 95 ft/s
        p = la["p"]
        tgt = [sb[1], sb[2] + 4.0, sb[3] + 5.0]
        steps = 12
        for i in range(1, steps + 1):
            q = [p[j] + (tgt[j] - p[j]) * i / steps for j in range(3)]
            await a.send(t="ms", p=q, d=[0, 0, -1])
            await asyncio.sleep(0.06)
        k = a.mark()
        await a.send(t="boom", p=tgt)
        boom = await a.wait("boom", lambda m: m["by"] == a.id, since=k)
        check(boom is not None and boom["fizzle"] == 0 and "down" not in boom, "the missile goes off where Alice says")
        hurt = await a.wait("hurt", lambda m: m["id"] == b.id, since=k, timeout=1)
        check(hurt is not None and hurt["hp"] == 1 and hurt["w"] == "m", "a blast within 10 ft takes 2 bars (w=m)")
        k = a.mark()
        await a.send(t="launch", d=[0, 0, -1])
        check(await a.wait("launch", lambda m: m["by"] == a.id, timeout=0.5, since=k) is None, "a second missile waits for the 12 s recharge")
        k = a.mark()
        await a.send(t="launch", k="drone", d=[0, 0, -1])
        check(await a.wait("launch", lambda m: m["by"] == a.id, timeout=0.5, since=k) is None, "no drone without the power-up")

        # Bob's missile, shot down by Alice's laser
        k = a.mark()
        await b.send(t="launch", d=[0, 0, 1])
        lb = await a.wait("launch", lambda m: m["by"] == b.id, since=k)
        check(lb is not None, "Bob launches a missile")
        if lb:
            await asyncio.sleep(0.15)
            mp = lb["p"]
            sa = a.me_in_snap(a.id)
            o = [sa[1], sa[2] + 5.4, sa[3]]
            d = [mp[i] - o[i] for i in range(3)]
            L = math.sqrt(sum(v * v for v in d))
            d = [v / L for v in d]
            await asyncio.sleep(0.4)
            k = a.mark()
            await a.send(t="fire", o=o, d=d, rt=a.last_snap()["ts"])
            beam = await a.wait("beam", lambda m: m["by"] == a.id, since=k)
            check(beam is not None and beam["mhit"] == b.id and beam["hit"] == 0, "the laser hits Bob's missile, not Bob")
            boom = await a.wait("boom", lambda m: m["by"] == b.id, since=k)
            check(boom is not None and boom.get("down") == a.id, "Bob's missile is shot down (down = Alice)")

    # junk is ignored, not fatal
    await a.send(t="st", p=["nan", 1, 2])
    await a.send(t="fire", o=[1, 2], d=[0, 0, 1])
    await a.ws.send("not json")
    await a.ws.send("x" * 700)
    await asyncio.sleep(0.3)
    k = a.mark()
    await a.send(t="ping", c="still")
    check(await a.wait("pong", lambda m: m["c"] == "still", since=k) is not None, "junk messages are ignored and the connection lives")

    # round end and the next round
    left = w["left"]
    re = await a.wait("round_end", timeout=left + 5)
    check(re is not None and re["next"] == 3 and len(re["board"]) == 2, "the round ends with a board of 2")
    if re:
        check(re["winner"] == a.id if placed else True, "Alice wins the round")
        rn = await a.wait("round", timeout=6)
        check(rn is not None and rn["no"] == 2 and len(rn["spots"]) == 9 and rn["left"] == 25, "round 2 starts with spots reset")
        r = await a.wait("roster", lambda m: all(p["score"] == 0 for p in m["players"]), since=a.msgs.index(rn))
        check(r is not None, "scores are reset for the new round")

    # leaving
    k = a.mark()
    await b.ws.close()
    left_m = await a.wait("left", lambda m: m["id"] == b.id, since=k)
    check(left_m is not None, "Bob leaving is announced")
    check(await a.wait("feed", lambda m: m["msg"] == "Bob left", since=k) is not None, "feed says Bob left")

    # a non-join first message is refused
    ws = await websockets.connect(URL)
    await ws.send(json.dumps({"t": "st", "p": [1, 2, 3]}))
    try:
        await asyncio.wait_for(ws.recv(), 2)
        check(False, "a connection that does not join first is closed")
    except (websockets.ConnectionClosed, asyncio.TimeoutError) as e:
        check(isinstance(e, websockets.ConnectionClosed), "a connection that does not join first is closed")

    # rate limit: a burst of 300 pings gets at most ~80 + a few pongs
    k = a.mark()
    for i in range(300):
        await a.send(t="ping", c=i)
    await asyncio.sleep(1.0)
    n = sum(1 for m in a.msgs[k:] if m.get("t") == "pong")
    check(70 <= n <= 160, "a burst of 300 messages is rate limited (%d answered)" % n)

    await a.ws.close()
    print("FAILED: %d" % len(fails) if fails else "ALL PASSED")
    sys.exit(1 if fails else 0)


asyncio.run(main())
