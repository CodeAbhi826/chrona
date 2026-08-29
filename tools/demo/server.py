#!/usr/bin/env python3
"""Chrona demo web server — a thin HTTP bridge onto the real daemon.

Browsers cannot speak Unix sockets, so this server:
  * serves the demo dashboard (tools/demo/web/*),
  * forwards /api/<cmd> requests to chronad's local Unix-socket JSON API
    (GET query params or a POST JSON body become the `args` object),
  * serves repo assets (logo, bundled Inter font).

Every number you see in the demo dashboard is computed by the real Rust
daemon from real SQLite rows — this server only relays bytes.

Usage:
    python3 tools/demo/server.py --socket /tmp/chrona.sock [--port 3000]
"""

import argparse
import json
import os
import socket as syssock
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
WEB = os.path.join(HERE, "web")

GET_CMDS = {"status", "day", "week", "month", "app", "rules", "goals", "settings.get", "export", "ping"}
POST_CMDS = {"goal.set", "goal.del", "rule.add", "rule.del", "settings.set", "pause.set"}
INT_KEYS = {"offset", "days", "id", "limit_seconds", "priority"}

MIME = {
    ".html": "text/html; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".js": "application/javascript; charset=utf-8",
    ".svg": "image/svg+xml",
    ".ttf": "font/ttf",
    ".png": "image/png",
}


def rpc(path, cmd, args):
    try:
        s = syssock.socket(syssock.AF_UNIX, syssock.SOCK_STREAM)
        s.settimeout(5)
        s.connect(path)
        s.sendall((json.dumps({"id": 1, "cmd": cmd, "args": args}) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
        s.close()
        return buf.decode().strip()
    except Exception as e:  # daemon down
        return json.dumps({"ok": False, "error": f"daemon unreachable: {e}"})


class Handler(BaseHTTPRequestHandler):
    sock_path = "/tmp/chrona.sock"

    def log_message(self, fmt, *a):  # quieter logs
        pass

    def _send(self, code, body, ctype="application/json; charset=utf-8", cache="no-store"):
        if isinstance(body, str):
            body = body.encode()
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", cache)
        self.end_headers()
        self.wfile.write(body)

    def _file(self, abspath, cache="no-store"):
        try:
            with open(abspath, "rb") as f:
                body = f.read()
        except OSError:
            return self._send(404, "not found", "text/plain")
        ext = os.path.splitext(abspath)[1]
        self._send(200, body, MIME.get(ext, "application/octet-stream"), cache)

    def do_GET(self):
        u = urlparse(self.path)
        if u.path in ("/", "/index.html"):
            return self._file(os.path.join(WEB, "index.html"))
        # static demo assets (style.css, app.js, extras.js, …) — safe basename
        # lookup inside web/, no traversal.
        if "/" not in u.path[1:] and os.path.splitext(u.path)[1] in (".css", ".js"):
            cand = os.path.join(WEB, os.path.basename(u.path))
            if os.path.isfile(cand):
                return self._file(cand)
        if u.path in ("/logo.svg", "/favicon.svg"):
            return self._file(os.path.join(REPO, "assets", "logo.svg"), cache="public, max-age=3600")
        # bundled app icons (web demo only) — safe basename inside web/icons/
        if u.path.startswith("/icons/"):
            name = os.path.basename(u.path[1:])
            if os.path.splitext(name)[1] in (".svg", ".png"):
                cand = os.path.join(WEB, "icons", name)
                if os.path.isfile(cand):
                    return self._file(cand, cache="public, max-age=86400")
        if u.path == "/inter.ttf":
            return self._file(
                os.path.join(REPO, "assets", "fonts", "Inter.ttf"), cache="public, max-age=86400"
            )

        if u.path.startswith("/api/"):
            cmd = u.path[len("/api/"):]
            if cmd not in GET_CMDS:
                return self._send(403, json.dumps({"ok": False, "error": "command not allowed over HTTP"}))
            args = {}
            for k, v in parse_qs(u.query).items():
                val = v[0]
                if k in INT_KEYS:
                    try:
                        val = int(val)
                    except ValueError:
                        pass
                args[k] = val
            return self._send(200, rpc(self.sock_path, cmd, args))

        return self._send(404, "not found", "text/plain")

    def do_POST(self):
        u = urlparse(self.path)
        if not u.path.startswith("/api/"):
            return self._send(404, "not found", "text/plain")
        cmd = u.path[len("/api/"):]
        if cmd not in POST_CMDS:
            return self._send(403, json.dumps({"ok": False, "error": "command not allowed over HTTP"}))
        try:
            length = int(self.headers.get("Content-Length", 0))
            args = json.loads(self.rfile.read(length) or b"{}")
            if not isinstance(args, dict):
                raise ValueError
        except Exception:
            return self._send(400, json.dumps({"ok": False, "error": "body must be a JSON object"}))
        return self._send(200, rpc(self.sock_path, cmd, args))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--socket", default=os.environ.get("CHRONA_SOCK", "/tmp/chrona.sock"))
    ap.add_argument("--port", type=int, default=int(os.environ.get("PORT", "3000")))
    ap.add_argument("--host", default="0.0.0.0")
    args = ap.parse_args()
    Handler.sock_path = args.socket
    print(f"chrona demo ui on http://{args.host}:{args.port} (daemon socket: {args.socket})")
    srv = ThreadingHTTPServer((args.host, args.port), Handler)
    srv.daemon_threads = True
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        sys.exit(0)


if __name__ == "__main__":
    main()
