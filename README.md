# Creek Laser Tag

Multiplayer neon laser-tag on a 1,400 ft square of creek terrain (Three.js client, Python websockets server).

- `server/game.py` – authoritative game server (hits with lag-compensated rewind, rounds, power-ups, missile/drone). Run: `python server/game.py /run/lasertag/ws.sock` (or `game.py 127.0.0.1:8899` for a local test).
- `web/laser-tag/` – static client (`index.html`, `game.js`, vendored three.js 0.160, `terrain.bin`/`terrain.json`).
- `deploy/` – systemd unit and nginx site (port 8802, WebSocket at `/laser-tag/ws`).

Env: `LASERTAG_WEB`, `LASERTAG_ROUND_S`, `LASERTAG_INTERMISSION_S`, `LASERTAG_BOTS` (default 0).

Setup: `python3 -m venv venv && venv/bin/pip install -r requirements.txt`.
