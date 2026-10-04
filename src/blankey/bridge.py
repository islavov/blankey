"""`blankey mcp`: stdio MCP server for Claude Desktop that forwards to the running tray app.

Each JSON-RPC line from stdin is POSTed to the app's local HTTP endpoint and the JSON reply is
written back to stdout. Requests run on their own threads so a blocking wait_request does not
hold up other calls. If the app is not running it is started.
"""

import json
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from email.message import Message

from blankey.config import load_config
from blankey.mcp_server import HEALTH_PATH, HEALTH_TEXT
from blankey.ui.platform import launch_command

STARTUP_TIMEOUT_S = 20
REQUEST_TIMEOUT_S = 1900  # longer than wait_request's maximum


class Bridge:
    def __init__(self, url: str):
        self.url = url
        self.session_id: str | None = None
        self.protocol_version: str | None = None
        self.stdout_lock = threading.Lock()
        self.start_lock = threading.Lock()

    def write(self, message: dict) -> None:
        with self.stdout_lock:
            sys.stdout.write(json.dumps(message, ensure_ascii=False) + "\n")
            sys.stdout.flush()

    def post(self, body: bytes) -> tuple[int, Message, bytes]:
        headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
        if self.session_id:
            headers["mcp-session-id"] = self.session_id
        if self.protocol_version:
            headers["mcp-protocol-version"] = self.protocol_version
        request = urllib.request.Request(self.url, data=body, headers=headers, method="POST")
        try:
            with urllib.request.urlopen(request, timeout=REQUEST_TIMEOUT_S) as response:
                return response.status, response.headers, response.read()
        except urllib.error.HTTPError as exc:
            return exc.code, exc.headers, exc.read()

    def ensure_app(self) -> bool:
        with self.start_lock:
            if self._reachable():
                return True
            quiet = subprocess.DEVNULL
            subprocess.Popen(launch_command(), start_new_session=True, stdout=quiet, stderr=quiet)
            deadline = time.monotonic() + STARTUP_TIMEOUT_S
            while time.monotonic() < deadline:
                time.sleep(0.5)
                if self._reachable():
                    return True
            return False

    def _reachable(self) -> bool:
        return server_running(self.url)

    def handle(self, line: str) -> None:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            return
        request_id = message.get("id") if isinstance(message, dict) else None
        try:
            if not self.ensure_app():
                raise ConnectionError("Blankey is not running and could not be started")
            status, headers, body = self.post(line.encode())
            if status == 404 and self.session_id:  # app restarted: session is gone
                self.session_id = None
                status, headers, body = self.post(line.encode())
        except OSError as exc:
            if request_id is not None:
                self.write({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32000, "message": str(exc)}})
            return
        if session := headers.get("mcp-session-id"):
            self.session_id = session
        if isinstance(message, dict) and message.get("method") == "initialize":
            self.protocol_version = message.get("params", {}).get("protocolVersion")
        if not body.strip():
            return
        try:
            reply = json.loads(body)
        except json.JSONDecodeError:
            if request_id is not None:
                text = body.decode(errors="replace")[:500]
                self.write({"jsonrpc": "2.0", "id": request_id, "error": {"code": -32000, "message": text}})
            return
        for item in reply if isinstance(reply, list) else [reply]:
            self.write(item)

    def run(self) -> None:
        for line in sys.stdin:
            if line.strip():
                threading.Thread(target=self.handle, args=(line,), daemon=True).start()


def server_running(url: str) -> bool:
    """True only if Blankey itself answers; another program on the same port does not count."""
    health = url.removesuffix("/mcp") + HEALTH_PATH
    try:
        with urllib.request.urlopen(health, timeout=1) as response:
            return response.read().decode(errors="replace").strip() == HEALTH_TEXT
    except OSError:
        return False


def main() -> None:
    Bridge(load_config().mcp_url).run()
