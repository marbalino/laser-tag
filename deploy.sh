#!/bin/bash
# Put the latest GitHub version of Creek Laser Tag live. Run on the server, as root:
#
#   /opt/lasertag/deploy.sh          build and test origin/main, then put it live (refuses if people are playing
#                                    and the game server itself changed, because a restart drops everyone)
#   /opt/lasertag/deploy.sh --now    the same, even if people are playing
#   /opt/lasertag/deploy.sh --check  only build and test origin/main; change nothing live
#
# It builds in a separate checkout (/opt/lasertag-next), so a broken commit never touches the live game.
set -euo pipefail
LIVE=/opt/lasertag
NEXT=/opt/lasertag-next
MODE=${1:-}
cd "$LIVE"
git fetch -q origin
target=$(git rev-parse origin/main)
if [ ! -d "$NEXT/.git" ] && [ ! -f "$NEXT/.git" ]; then
  git worktree add -q --detach "$NEXT" "$target"
fi
git -C "$NEXT" checkout -q --detach "$target"
git -C "$NEXT" clean -qfd -e target -e bin -e web/laser-tag/pkg
echo "== building and testing $(git -C "$NEXT" log --oneline -1)"
"$NEXT/build.sh"
echo "== the scripted match against the new server"
port=8890
LASERTAG_WEB="$NEXT/web/laser-tag" LASERTAG_ROUND_S=25 LASERTAG_INTERMISSION_S=3 "$NEXT/bin/lasertag-server" 127.0.0.1:$port >/tmp/lasertag-deploy-test.log 2>&1 &
srv=$!
sleep 1
if ! "$LIVE/venv/bin/python" "$NEXT/ref/match_test.py" ws://127.0.0.1:$port >/tmp/lasertag-deploy-match.txt 2>&1; then
  kill $srv 2>/dev/null || true
  grep -E "FAIL|Error|Traceback" /tmp/lasertag-deploy-match.txt | head -20
  echo "== the match test failed: nothing was changed live (full output: /tmp/lasertag-deploy-match.txt)"
  exit 1
fi
kill $srv 2>/dev/null || true
echo "   $(tail -1 /tmp/lasertag-deploy-match.txt)"
if [ "$MODE" = "--check" ]; then
  echo "== check only: nothing was changed live"
  exit 0
fi
server_changed=1
cmp -s "$NEXT/bin/lasertag-server" "$LIVE/bin/lasertag-server" && server_changed=0
players=$(ss -xH state connected 2>/dev/null | grep -c "/run/lasertag/ws.sock" || true)
if [ "$server_changed" = 1 ] && [ "$players" -gt 0 ] && [ "$MODE" != "--now" ]; then
  echo "== $players connection(s) to the game right now, and the game server changed: a restart would drop them."
  echo "   Try again later, or run: $0 --now"
  exit 2
fi
echo "== going live"
git merge -q --ff-only "$target"
install -m 755 "$NEXT/bin/lasertag-server" "$LIVE/bin/lasertag-server.new" && mv -f "$LIVE/bin/lasertag-server.new" "$LIVE/bin/lasertag-server"
mkdir -p "$LIVE/web/laser-tag/pkg" && cp "$NEXT"/web/laser-tag/pkg/* "$LIVE/web/laser-tag/pkg/"
if ! cmp -s "$LIVE/deploy/lasertag.service" /etc/systemd/system/lasertag.service; then
  install -m 644 "$LIVE/deploy/lasertag.service" /etc/systemd/system/lasertag.service && systemctl daemon-reload && server_changed=1
fi
if ! cmp -s "$LIVE/deploy/nginx-lasertag.conf" /etc/nginx/sites-available/lasertag; then
  cp /etc/nginx/sites-available/lasertag /tmp/nginx-lasertag.before
  install -m 644 "$LIVE/deploy/nginx-lasertag.conf" /etc/nginx/sites-available/lasertag
  if nginx -t 2>/dev/null; then systemctl reload nginx; else
    cp /tmp/nginx-lasertag.before /etc/nginx/sites-available/lasertag; echo "   the new nginx config is broken: kept the old one"; fi
fi
if [ "$server_changed" = 1 ]; then
  systemctl restart lasertag && sleep 1 && echo "   game server restarted: $(systemctl is-active lasertag)"
else
  echo "   game server unchanged: no restart (players pick up page changes when they reload)"
fi
echo "== live at $(git log --oneline -1)"
