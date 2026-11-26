"""Browser test: the Rust/WebAssembly page and the old JavaScript page, both playing on the Rust server.

Run with the Playwright venv: python browser_test.py OUTDIR
Expects the new page at http://127.0.0.1:8898/laser-tag/ and the old page at http://127.0.0.1:8896/laser-tag/,
both proxying their WebSocket to a test server.
"""
import json
import math
import sys
import time

from playwright.sync_api import sync_playwright

OUT = sys.argv[1]
NEW = "http://127.0.0.1:8898/laser-tag/"
OLD = "http://127.0.0.1:8896/laser-tag/"
fails = []


def check(ok, what):
    print(("ok   " if ok else "FAIL ") + what, flush=True)
    if not ok:
        fails.append(what)


def state(page):
    return json.loads(page.evaluate("window.__lt.state()"))


with sync_playwright() as p:
    b = p.chromium.launch(args=["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist",
                                "--autoplay-policy=no-user-gesture-required"])
    ctx = b.new_context(viewport={"width": 640, "height": 400})
    ctx.set_default_timeout(150000)
    new = ctx.new_page()
    errors = []
    new.on("console", lambda m: errors.append(m.text) if m.type == "error" else None)
    new.on("pageerror", lambda e: errors.append(str(e)))
    new.goto(NEW)
    new.wait_for_function("window.__lt && window.__lt.state")
    time.sleep(1.5)
    new.screenshot(path=OUT + "/1-lobby.png")
    s = state(new)
    check(not s["playing"] and s["size"][0] > 0, "the new page starts, shows the lobby and renders (%s)" % s["size"])
    check(new.eval_on_selector_all("#swatches button", "b => b.length") == 8, "8 colour swatches")
    new.fill("#name", "Rusty")
    new.click("#play")
    new.wait_for_function("JSON.parse(window.__lt.state()).playing")
    time.sleep(1.0)
    s = state(new)
    check(s["playing"] and s["me"]["alive"] and s["me"]["id"] > 0, "Play joins the arena and you spawn")
    check(s["boxes"] == 433 and s["pillars"] == 30 and len(s["spots"]) == 9, "the arena arrives (433 wall boxes, 30 pillars, 9 spots)")
    check(new.is_hidden("#lobby") and new.is_visible("#hud"), "the lobby hides and the HUD shows")
    hp = new.eval_on_selector_all("#hp i", "x => x.length")
    check(hp == 3, "3 health bars")
    rank = new.text_content("#rank")
    check("Just you" in rank, "the top pill says you are alone: %r" % rank)

    # the old JavaScript page joins the same arena
    old = ctx.new_page()
    old_err = []
    old.on("pageerror", lambda e: old_err.append(str(e)))
    old.goto(OLD)
    old.wait_for_function("window.__lt")
    old.fill("#name", "OldBob")
    old.click("#play")
    old.wait_for_function("window.__lt.playing")
    time.sleep(1.0)
    s = state(new)
    names = sorted(x["name"] for x in s["people"])
    check(names == ["OldBob", "Rusty"], "the new page sees the old page's player: %s" % names)
    old_people = old.evaluate("[...window.__lt.people.values()].map(p => p.name).sort()")
    check(old_people == ["OldBob", "Rusty"], "the old page sees the new page's player: %s" % old_people)
    feed = new.text_content("#feed")
    check("OldBob joined" in feed, "the feed says OldBob joined")

    # stand them 40 ft apart on open ground, the new player facing the old one
    time.sleep(2.8)  # spawn shields wear off
    hit = False
    for (x, z) in [(700, 700), (400, 400), (1000, 1000), (300, 1000), (1000, 300), (600, 900), (900, 600)]:
        old.evaluate("([x, z]) => { const m = window.__lt.me; m.x = x; m.z = z - 40; m.vx = m.vz = 0; m.yaw = Math.PI; }", [x, z])
        time.sleep(0.6)
        s = state(new)
        bob = [q for q in s["people"] if q["name"] == "OldBob"][0]
        me = s["me"]
        # aim at Bob's chest from our eye
        new.evaluate("([x, z]) => window.__lt.place(x, z, 0, 0)", [x, z])
        time.sleep(0.3)
        s = state(new)
        me = s["me"]
        bob = [q for q in s["people"] if q["name"] == "OldBob"][0]
        dx, dy, dz = bob["x"] - me["x"], (bob["y"] + 3.0) - (me["y"] + 5.4), bob["z"] - me["z"]
        yaw = math.atan2(dx, -dz)
        pitch = math.atan2(dy, math.hypot(dx, dz))
        new.evaluate("([x, z, yaw, pitch]) => window.__lt.place(x, z, yaw, pitch)", [x, z, yaw, pitch])
        before = old.evaluate("window.__lt.me.hp")
        new.evaluate("window.__lt.fire(true)")
        time.sleep(1.2)
        new.evaluate("window.__lt.fire(false)")
        time.sleep(1.0)
        after = old.evaluate("window.__lt.me.hp")
        if after < before:
            hit = True
            break
        time.sleep(0.6)
    new.screenshot(path=OUT + "/2-aiming.png")
    check(hit, "the new page's laser hits the old page's player (hp %s -> %s)" % (before, after))
    old.screenshot(path=OUT + "/3-old-page-view.png")

    # a missile from the new page: you become it, it flies, it goes off
    s = state(new)
    new.evaluate("([x, z, yaw]) => window.__lt.place(x, z, yaw, 0.15)", [s["me"]["x"], s["me"]["z"], s["me"]["yaw"] + math.pi])
    time.sleep(0.3)
    new.evaluate("window.__lt.launch()")
    time.sleep(0.4)
    s = state(new)
    check(s["ms"]["active"], "Q launches a missile and you fly it")
    check(new.is_visible("#mcam"), "the missile camera overlay shows")
    new.screenshot(path=OUT + "/4-missile-cam.png")
    time.sleep(0.3)
    old_sees = old.evaluate("window.__lt.rmis.size")
    check(old_sees == 1, "the old page sees the missile in the air")
    new.evaluate("window.__lt.detonate()")
    time.sleep(0.25)
    new.screenshot(path=OUT + "/5-blast.png")
    s = state(new)
    check(not s["ms"]["active"], "clicking blows it up and you are back in your body")

    # the old page fires at the new player
    s = state(new)
    me = s["me"]
    ob = old.evaluate("({x: window.__lt.me.x, y: window.__lt.me.y, z: window.__lt.me.z})")
    dx, dy, dz = me["x"] - ob["x"], (me["y"] + 3.0) - (ob["y"] + 5.4), me["z"] - ob["z"]
    old.evaluate("([yaw, pitch]) => { window.__lt.me.yaw = yaw; window.__lt.me.pitch = pitch; }",
                 [math.atan2(dx, -dz), math.atan2(dy, math.hypot(dx, dz))])
    time.sleep(2.6)  # the new player may have a spawn shield
    hp0 = state(new)["me"]["hp"]
    old.evaluate("window.__lt.fire(true)")
    time.sleep(1.5)
    old.evaluate("window.__lt.fire(false)")
    time.sleep(0.5)
    s = state(new)
    check(s["me"]["hp"] < hp0 or not s["me"]["alive"], "the old page's laser hits the new player (hp %s -> %s)" % (hp0, s["me"]["hp"]))
    check(new.eval_on_selector_all("#hp i.gone", "x => x.length") >= 1, "a health bar empties on the HUD")

    # scoreboard
    new.keyboard.down("Tab")
    time.sleep(0.2)
    rows = new.eval_on_selector_all("#board tbody tr", "x => x.length")
    new.screenshot(path=OUT + "/6-board.png")
    new.keyboard.up("Tab")
    check(rows == 2, "Tab shows a scoreboard with 2 rows")

    # glow off and on, resizing
    new.set_viewport_size({"width": 800, "height": 600})
    time.sleep(0.5)
    check(state(new)["size"][0] in (800, 1200, 1000, 1000), "the canvas follows a resize: %s" % state(new)["size"])

    # leaving
    old.close()
    time.sleep(1.0)
    s = state(new)
    check([q["name"] for q in s["people"]] == ["Rusty"], "the old player leaving removes them")
    new.screenshot(path=OUT + "/7-alone.png")

    real = [e for e in errors if "pointer" not in e.lower() and "fullscreen" not in e.lower()]
    check(not real, "no errors on the new page: %s" % real[:5])
    check(not old_err, "no errors on the old page: %s" % old_err[:5])
    b.close()

print("FAILED: %d" % len(fails) if fails else "ALL PASSED")
sys.exit(1 if fails else 0)
