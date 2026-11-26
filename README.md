# Creek Laser Tag

[![check](https://github.com/marbalino/laser-tag/actions/workflows/check.yml/badge.svg)](https://github.com/marbalino/laser-tag/actions/workflows/check.yml)

Multiplayer neon laser tag on a 1,400 ft square of creek terrain. Everything is Rust: the game server, and the
browser game compiled to WebAssembly with its own WebGL2 renderer.

Play: http://109.123.250.254:8802/laser-tag/ (ask the server owner for the username and password).

- [How a change gets into the game](#how-a-change-gets-into-the-game)
- [Making a change with a chat AI and the GitHub website](#making-a-change-with-a-chat-ai-and-the-github-website)
- [Where things are: what to edit for each kind of change](#where-things-are-what-to-edit-for-each-kind-of-change)
- [Rules that keep the game working](#rules-that-keep-the-game-working)
- [When the check goes red](#when-the-check-goes-red)
- [Undoing a change](#undoing-a-change)
- [For the server owner: deploying, building, running](#for-the-server-owner-deploying-building-running)
- [How it is built](#how-it-is-built)

---

## How a change gets into the game

1. **You commit to `main` on GitHub** (with the website's editor, see below).
2. **GitHub checks it, by itself, in about 5 minutes.** It compiles the server and the browser game, runs the
   tests, and plays a scripted two-player match against the new server. You see the result next to the commit:
   a yellow dot while it runs, a **green tick** when it is fine, a **red cross** when something broke
   (see [When the check goes red](#when-the-check-goes-red)).
3. **The server owner puts it live** by running `/opt/lasertag/deploy.sh` on the server. Nothing you commit
   reaches the live game until then, so a broken commit cannot break the game people are playing. Tell the owner
   when you have a green tick and want it live.

Players get page changes (graphics, controls, HUD) when they reload the page. Game-server changes (rules,
damage, rounds) need a restart, which drops everyone in the arena for a moment, so the deploy waits until nobody
is playing unless the owner says otherwise.

## Making a change with a chat AI and the GitHub website

You do not need to install anything. You need a GitHub account with write access to this repo, a chat AI
(ChatGPT, Claude, Gemini, ...) and a browser.

### A change to one file

1. **Find the file** with the table below ([Where things are](#where-things-are-what-to-edit-for-each-kind-of-change)).
2. **Open it on GitHub** and press the **copy raw file** button (two overlapping squares, top right of the file).
3. **Start the chat** with this, then paste the file after it:

   > I am changing a Rust game. Read the rules first: https://github.com/marbalino/laser-tag#rules-that-keep-the-game-working
   > (or I will paste them). Here is the file `PATH/TO/FILE.rs`. I want: DESCRIBE THE CHANGE.
   > Give me back the **whole file**, complete, with the change made, and nothing left out. Tell me if another
   > file has to change too.

   If the chat AI cannot open links, paste the [rules section](#rules-that-keep-the-game-working) into the chat
   as well.
4. **Paste the answer into GitHub:** open the file, press the **pencil** (Edit), select everything
   (Ctrl+A / Cmd+A), paste the AI's whole file, then **Commit changes...**, write one line saying what you
   changed (for example "Missiles fly 20% faster"), keep "Commit directly to the main branch", and commit.
5. **Wait for the tick** next to your commit (on the repo's front page or under **Actions**). Green: tell the
   owner it is ready to go live. Red: see [When the check goes red](#when-the-check-goes-red).

### A change to several files at once

Some changes touch two or three files (for example, anything both the server and the browser have to know about).
Make them in **one commit**, so there is never a half-done version on GitHub:

1. On the repo page press the **`.`** key (full stop). GitHub opens an editor (github.dev) in your browser.
2. Open each file from the list on the left, select all, and paste the AI's new version.
3. Click the **Source Control** icon on the far left (it shows a number), write a one-line message, and press
   **Commit & Push**.

### Big files

`client/src/game.rs` (about 2,000 lines) and `server/src/game.rs` (about 900) may be too long to paste into some
chat boxes or to get back whole. Then work by function:

1. Paste only the function(s) you need to change (each starts with `fn name(` and ends at its matching `}`),
   plus the top of the file (the `use` lines and the constants), and say which file they come from.
2. Ask: "Give me back each changed function complete, from its `fn` line to its closing `}`, and tell me
   exactly which function each one replaces. If you add a new function, tell me where to put it."
3. In the GitHub editor, find the old function (Ctrl+F for `fn name(`), select it from its `fn` line down to its
   closing `}`, and paste the new one over it.

### Tips

- **One idea per commit.** It is much easier to see what broke and to undo it.
- **Ask the AI to keep everything else exactly as it is.** AIs sometimes "tidy up" unrelated code or shorten
  files with comments like `// ... rest unchanged`. If you see that, ask for the complete file again; pasting a
  shortened file deletes the missing code.
- **Watch the tick.** A green tick means it compiles and a scripted match still plays correctly. It does not
  mean your feature feels right: ask the owner to put it live, then play it.

## Where things are: what to edit for each kind of change

Units are **feet** and **seconds**. Up is **y**. The ground is x from 0 to 1400 and z from 0 to 1400.

| I want to change... | Edit | Look for |
|---|---|---|
| Damage, health, fire rate, overheating, laser range, respawn and shield time, regeneration | `server/src/game.rs` | the constants at the top (`MAX_HP`, `FIRE_CD`, `HEAT_SHOT`, `RANGE`, `RESPAWN_S`, `SHIELD_S`, `REGEN_AFTER`...). The browser is told these numbers when it joins (`welcome`), so the server file is usually the only one to change |
| Missiles and drones (speed, life, blast size, damage, recharge) | `server/src/game.rs` | constants `MISSILE_*`, `BLAST_R`, `DRONE_*`, `DRONE_RINGS`; functions `launch`, `explode`. How you fly them: `client/src/game.rs`, functions `step_missile` (homing is there), `step_drone`, `launch_missile`, `launch_drone` |
| Who a laser hits | `server/src/game.rs`, function `fire` | `HIT_R` (how close counts), `BODY` (the body's height) |
| Running speed, sprint, jumping, gravity, swimming slowdown | `client/src/game.rs` | constants `GRAV`, `JUMP1`, `JUMP2` at the top; function `my_move` (21 = walk, 31 = sprint, 0.6 = in water); function `jump` |
| Zoom, field of view | `client/src/game.rs` | constants `FOV`, `FOV_ZOOM` |
| Round length, break between rounds | the server's settings, not code | ask the owner (environment `LASERTAG_ROUND_S`, `LASERTAG_INTERMISSION_S` in `deploy/lasertag.service`) |
| Power-ups (how long they last, what they do) | `server/src/game.rs` | `POWER_S`, `PICKUP_RESPAWN`, `Powers`, and in `tick` the pick-up code; what each one does: `fire` (rapid, over), and in the browser `my_move` (speed) |
| A new kind of power-up | several files, one commit | `server/src/arena.rs` (`SPOT_KINDS`), `server/src/game.rs` (`Powers`, `flags`, `tick`), `client/src/world.rs` (`power_color`), `client/src/game.rs` (`power_name`, `Powers`, `hud_powers`, the `Pick` case in `on_message`), `web/laser-tag/index.html` (the help text). This changes the arena test, see the rules |
| The arena: buildings, walls, pillars, where power-ups are | `server/src/arena.rs` | `build_structures` (buildings, lone walls, low walls), `build_arena` (pillars, spots). The seed (415) is in `server/src/main.rs`. This changes the arena test, see the rules |
| Maximum players | `server/src/game.rs` | `MAX_HUMANS`; also the "arena is full" text in `client/src/game.rs` |
| Bots | `server/src/bots.rs` (how they think) | they are off: the owner sets `LASERTAG_BOTS` to turn them on |
| Player colours | `shared/src/lib.rs` | `PALETTE` (also the help colours in `web/laser-tag/index.html`) |
| The ground's look: grid, contour lines, creek, fog | `client/src/shaders.rs` | `TERRAIN_FS` |
| Sky, stars | `client/src/shaders.rs` (`SKY_FS`), `client/src/render.rs` (stars in `Renderer::new`) | |
| Walls' look, fence's look | `client/src/shaders.rs` | `RUINS_FS`, `FENCE_FS` |
| Glow strength, lights, brightness | `client/src/render.rs` | `post` (bloom: `strength`, `radius`, the 0.85 threshold), `Env` in `Renderer::new` (lights, fog), `uExposure` |
| Players' look (body, visor, gun, name tag, shield bubble) | `client/src/game.rs` | `build_frame`, the "people" part |
| Missiles' and drones' look, laser beams, explosions, sparks | `client/src/game.rs` | `build_frame`; `beam`, `burst`, `explosion` |
| Pillars' and power-up spots' look | `client/src/world.rs` | `draw_pillars`, `draw_spots` |
| Your own gun on screen | `client/src/game.rs` | `gun_local` (where it sits), the end of `build_frame` (its parts) |
| Sounds | `client/src/audio.rs` | one function per sound (`pew`, `hit`, `boom`...): they are synthesised, no sound files |
| Text on screen: messages, kill feed, "TAGGED", streak calls | `client/src/game.rs` | `on_message` (one case per server message), `hud_*` functions |
| Menus, help text, HUD layout and colours, buttons | `web/laser-tag/index.html` | plain HTML and CSS: the safest file to change |
| Scoreboard, minimap | `client/src/hud.rs` (`board`, `Mini`), `client/src/game.rs` (`show_board`, `draw_mini`) | |
| Keys and mouse | `client/src/lib.rs` | `events` (keydown: Q missile, E drone, Space jump, Z zoom, Tab board); help text in `index.html` |
| Phone and tablet controls | `client/src/lib.rs` (`touch_controls`, `press`), `web/laser-tag/index.html` (button positions in the CSS, `#touch`) | |
| What the server and browser say to each other | `shared/src/protocol.rs` | `ServerMsg`, `ClientMsg`. Both sides use this file; see the rules |
| Ground heights and the creek | `web/laser-tag/terrain.bin` | binary data: ask the owner |

## Rules that keep the game working

Give these to the chat AI with every change.

1. **The language is Rust** (edition 2021). The browser part compiles to WebAssembly: no threads, no files, no
   `std::time::Instant` in `client/` (use `dom::now_s()`), and browser features come through the `web-sys`
   crate. If the AI uses a new browser type (for example `web_sys::Gamepad`), its name must also be added to the
   `features` list in `client/Cargo.toml`, or it will not compile.
2. **Do not add new crates (libraries)** unless you really need one. If you do, add it to that part's
   `Cargo.toml` (`client/`, `server/` or `shared/`) and expect the check to take longer.
3. **The server is in charge.** The server decides hits, damage, scores, deaths and power-ups. The browser only
   draws, moves you, and asks (for example "I fired from here in this direction"). Never let the browser decide
   that someone was hit or how much damage happened: anyone can change their own browser.
4. **Messages must match on both sides.** If you add or change a message (or a field) in
   `shared/src/protocol.rs`, change the server code that sends or reads it and the browser code that reads or
   sends it **in the same commit**. Messages are JSON objects whose `t` field says which one they are.
5. **Directions:** `yaw` 0 looks towards -z, and turning right increases it; `pitch` up is positive. A direction
   from yaw and pitch is `[sin(yaw) cos(pitch), sin(pitch), -cos(yaw) cos(pitch)]` (`dir_of` in the client).
6. **The arena test.** `cargo test` checks that the arena is exactly the one in `ref/arena.json`. If you change
   the arena on purpose (`server/src/arena.rs`, the seed, `SPOT_KINDS`, or the maths in `shared/src/lib.rs`'s
   `dmath`), the check will fail with "box ... differs" or "spot ... differs". Then also delete the test
   `arena_is_unchanged` near the bottom of `server/src/main.rs` in the same commit, or ask the owner to refresh
   `ref/arena.json`. Everything that decides where walls, pillars and spots go must use `dmath` (`sin`, `cos`,
   `atan2`, `hypot`), not Rust's `.sin()` and so on: those can differ in the last digit from one computer to
   another, and then two machines build two different arenas.
7. **Keep `window.__lt`** (`debug_hook` in `client/src/lib.rs`) and `debug_state`: the tests use them.
8. **Never put the site's password, or any password or key, in the repo.** It is public.
9. **Give back whole files or whole functions**, never "...rest unchanged".

## When the check goes red

1. Click the red cross, then **Details**, then the step that failed.
2. Copy the error from the log: in Rust, it starts with `error[E....]:` and shows the file and line
   (`--> client/src/game.rs:812:9`). For the match test, copy the lines starting with `FAIL`.
3. Paste it into the same chat with: "GitHub's check failed with this error. Fix it and give me the complete
   corrected file (or function)."
4. Commit the fix. Repeat until the tick is green.

What the steps mean:

- **Tests**: the arena or the shared maths changed (see rule 6).
- **Build the game server** / **Build the browser game**: it does not compile. The error says where.
- **Scripted two-player match**: it compiles, but the game plays differently: someone is not hit when they
  should be, a missile does not go off, a round does not end, and so on. `ref/match_test.py` is the script. If
  you changed a rule on purpose (for example, the fire rate), the script may need the same change: give the
  failing line and the script to the AI.

## Undoing a change

- **Your last commit, before it is live:** open the file's **History** (top right of the file), click the commit
  before yours, open the file at that version (**View file**, or the `...` menu), copy the raw file, and paste it
  back with the pencil as a new commit ("Undo: ...").
- **Something live is broken:** tell the owner. The server owner can put any earlier version back live.

---

## For the server owner: deploying, building, running

**Deploy** (on the server, as root):

```
/opt/lasertag/deploy.sh            # build + test GitHub's main, then put it live
/opt/lasertag/deploy.sh --check    # build + test only, change nothing live
/opt/lasertag/deploy.sh --now      # put it live even if people are playing (a server change drops them)
```

It builds in a separate checkout (`/opt/lasertag-next`), runs the tests and the scripted match there, and only
then copies the new server and page into `/opt/lasertag`. It restarts the game server only if the server
changed, and refuses to while people are connected unless given `--now`. It also installs changed
`deploy/lasertag.service` and `deploy/nginx-lasertag.conf` (keeping the old nginx config if the new one fails
`nginx -t`).

**Put an older version back:** `git -C /opt/lasertag log --oneline`, then check out the commit you want in
`/opt/lasertag-next`, run its `build.sh`, and copy `bin/lasertag-server` and `web/laser-tag/pkg/` across as
`deploy.sh` does. Or: revert the commit on GitHub and run `deploy.sh`.

**Build** in any checkout (needs Docker): `./build.sh` runs the tests, then puts the server in
`bin/lasertag-server` and the browser game in `web/laser-tag/pkg/` (`--no-test` skips the tests).

**Run** a private copy for testing: `LASERTAG_WEB=web/laser-tag bin/lasertag-server 127.0.0.1:8899`, then
`python3 ref/match_test.py ws://127.0.0.1:8899` (with `LASERTAG_ROUND_S=25 LASERTAG_INTERMISSION_S=3`, the
match checks the round end too). `ref/browser_test.py` drives real pages with Playwright.

**Settings** (environment of the server): `LASERTAG_WEB` (where `terrain.json` and `terrain.bin` are),
`LASERTAG_ROUND_S` (300), `LASERTAG_INTERMISSION_S` (12), `LASERTAG_BOTS` (0: players only).

**Login:** nginx basic auth in front of everything, including the WebSocket; the password file is
`/etc/nginx/lasertag.htpasswd` on the server, not in git.

## How it is built

- `shared/`: what both sides use: the ground (heights and the creek), pillars and wall boxes, the ray and
  collision maths, and the messages.
- `server/`: the authoritative game server (tokio + tungstenite). It judges hits with lag-compensated rewind (it
  rewinds everyone to what the shooter saw, up to half a second), and runs rounds, power-ups, missiles, drones
  and bots. It sends everyone a snapshot 20 times a second. The arena is built from seed 415 with a port of
  Python's random generator and machine-independent maths (`dmath`), so it is the same on every computer. It is
  the arena the original Python server built, except for one short wall whose window sits 9 ft further along.
- `client/`: the browser game. `lib.rs` starts it and wires up the page; `game.rs` is the game (state, messages,
  movement, shooting, missiles, the frame); `render.rs`, `gl.rs`, `geom.rs` and `shaders.rs` draw it (ACES tone
  mapping and bloom); `world.rs` is the fixed scenery; `hud.rs` the HUD and minimap; `audio.rs` the sounds.
  Other players are drawn 100 ms in the past, smoothly, from the snapshots.
- `web/laser-tag/`: the page (`index.html`), the terrain files, and `pkg/` (the built WebAssembly, not in git).
- `deploy/`: the systemd unit and the nginx site (port 8802, WebSocket at `/laser-tag/ws`).
- `ref/`: test material and the build image. `.github/workflows/check.yml` is GitHub's check.

The original Python server and three.js page are at the `python-final` tag.
