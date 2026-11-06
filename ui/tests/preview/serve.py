# The UI preview's server: python3 -m http.server over ui/, but the browser always loads the current files.
import functools
import http.server
import os
import sys


class NoCache(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
port = int(sys.argv[1]) if len(sys.argv) > 1 else 8780
print(f"http://localhost:{port}/tests/preview/", flush=True)
http.server.ThreadingHTTPServer(("127.0.0.1", port), functools.partial(NoCache, directory=root)).serve_forever()
