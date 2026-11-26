# Creek Laser Tag

Multiplayer neon laser tag on a 1,400 ft square of creek terrain. Everything is Rust: the game server, and the
browser game compiled to WebAssembly with its own WebGL2 renderer.

- `shared/`: what both sides use: the ground (heights and the creek), pillars and wall boxes, the ray and collision
  maths, and the messages (JSON objects with a `t` field).
- `server/`: the authoritative game server (tokio + tungstenite). It judges hits with lag-compensated rewind, and
  runs rounds, power-ups, missiles, drones and (dormant) bots. The arena is built from seed 415 with a port of
  Python's random generator, so it is the same arena the original Python server built.
- `client/`: the browser game: the renderer (ACES tone mapping and bloom, as the three.js version had), the
  world, players, effects, synthesised sound, the HUD and minimap, and mouse, keyboard and touch controls.
- `web/laser-tag/`: the page, the terrain files, and `pkg/` (the built WebAssembly, not in git).
- `deploy/`: the systemd unit and the nginx site (port 8802, WebSocket at `/laser-tag/ws`).
- `ref/`: test material: `arena.json` (the arena the Python server built), `match_test.py` (a scripted two-player
  match against a server), `browser_test.py` (Playwright: the page against a server), and the build image.

Build (needs Docker): `./build.sh` puts the server in `bin/lasertag-server` and the browser game in
`web/laser-tag/pkg/`. Tests: `cargo test` (the arena must match `ref/arena.json`).

Run: `bin/lasertag-server /run/lasertag/ws.sock`, or `bin/lasertag-server 127.0.0.1:8899` for a local test.

Environment: `LASERTAG_WEB` (where `terrain.json` and `terrain.bin` are), `LASERTAG_ROUND_S`,
`LASERTAG_INTERMISSION_S`, `LASERTAG_BOTS` (default 0: players only).

The original Python server and three.js page are at the `python-final` tag.
