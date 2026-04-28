import { createReadStream } from "node:fs";
import { access, readFile, stat } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, join, normalize } from "node:path";

const ROOT = new URL("../out/", import.meta.url);
const DEFAULT_PORT = Number(process.env.PORT || 3001);

const MIME_TYPES = {
  ".css": "text/css; charset=utf-8",
  ".html": "text/html; charset=utf-8",
  ".ico": "image/x-icon",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".map": "application/json; charset=utf-8",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".txt": "text/plain; charset=utf-8",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
};

function sendJson(res, statusCode, body) {
  const payload = JSON.stringify(body);
  res.writeHead(statusCode, {
    "Content-Length": Buffer.byteLength(payload),
    "Content-Type": "application/json; charset=utf-8",
  });
  res.end(payload);
}

function contentType(pathname) {
  return MIME_TYPES[extname(pathname)] ?? "application/octet-stream";
}

function isSafeRelative(pathname) {
  const normalized = normalize(pathname).replace(/^(\.\.(\/|\\|$))+/, "");
  return normalized && !normalized.startsWith("..");
}

async function resolveAssetPath(urlPath) {
  const decoded = decodeURIComponent(urlPath);
  const cleanPath = decoded.replace(/^\/+/, "");

  if (cleanPath === "" || cleanPath === "/") {
    return join(ROOT.pathname, "index.html");
  }

  const relative = cleanPath.endsWith("/") ? `${cleanPath}index.html` : cleanPath;
  if (!isSafeRelative(relative)) {
    return null;
  }

  const candidate = join(ROOT.pathname, relative);
  try {
    const info = await stat(candidate);
    if (info.isFile()) {
      return candidate;
    }
  } catch {}

  const htmlFallback = join(ROOT.pathname, cleanPath, "index.html");
  try {
    const info = await stat(htmlFallback);
    if (info.isFile()) {
      return htmlFallback;
    }
  } catch {}

  return join(ROOT.pathname, "404.html");
}

const server = createServer(async (req, res) => {
  if (!req.url) {
    sendJson(res, 400, { error: "Missing URL" });
    return;
  }

  const requestUrl = new URL(req.url, `http://${req.headers.host || "localhost"}`);
  const assetPath = await resolveAssetPath(requestUrl.pathname);
  if (!assetPath) {
    sendJson(res, 400, { error: "Invalid path" });
    return;
  }

  try {
    await access(assetPath);
  } catch {
    sendJson(res, 404, { error: "Static export not found. Run `npm run build` first." });
    return;
  }

  const headers = {
    "Cache-Control": "no-cache",
    "Content-Type": contentType(assetPath),
    "X-Content-Type-Options": "nosniff",
  };

  if (req.method === "HEAD") {
    res.writeHead(200, headers);
    res.end();
    return;
  }

  const stream = createReadStream(assetPath);
  stream.on("error", async () => {
    const body = await readFile(join(ROOT.pathname, "404.html"), "utf8").catch(() => "Not found");
    res.writeHead(404, { "Content-Type": "text/html; charset=utf-8" });
    res.end(body);
  });

  res.writeHead(assetPath.endsWith("404.html") ? 404 : 200, headers);
  stream.pipe(res);
});

server.listen(DEFAULT_PORT, () => {
  console.log(`Static export server listening on http://localhost:${DEFAULT_PORT}`);
});
