// The repo over HTTP for the manual's browser: the UI, the fake backend and manual/ itself, never cached.

import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize, sep } from "node:path";

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
};

/** Serves `root` on a free loopback port; resolves to `{ origin, close }`. */
export function serve(root) {
  const server = createServer(async (request, response) => {
    const path = normalize(decodeURIComponent(new URL(request.url, "http://x").pathname));
    const file = join(root, path);
    if (!file.startsWith(root + sep)) return response.writeHead(403).end();
    try {
      const body = await readFile(file);
      response.writeHead(200, { "Content-Type": TYPES[extname(file)] || "application/octet-stream", "Cache-Control": "no-store" });
      response.end(body);
    } catch {
      response.writeHead(404).end();
    }
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      resolve({ origin: `http://127.0.0.1:${server.address().port}`, close: () => new Promise((done) => server.close(done)) });
    });
  });
}
