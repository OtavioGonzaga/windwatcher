import { createServer } from "node:http";
import { readFileSync, existsSync, statSync } from "node:fs";
import path from "node:path";
import { basePath } from "../lib/site";
import { exportedFile } from "../lib/export-check";

const out = path.resolve("out");
const port = Number(process.env.PORT ?? 4173);
const mime: Record<string, string> = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".json": "application/json",
  ".png": "image/png",
  ".svg": "image/svg+xml",
  ".woff2": "font/woff2",
  ".txt": "text/plain",
  ".md": "text/markdown",
  ".ico": "image/x-icon",
};

createServer((request, response) => {
  if (request.method !== "GET" && request.method !== "HEAD") {
    response.writeHead(405).end();
    return;
  }
  try {
    const url = new URL(request.url ?? "/", "http://localhost");
    if (basePath && url.pathname === basePath) {
      response.writeHead(301, { Location: `${basePath}/` }).end();
      return;
    }
    const file = exportedFile(out, url);
    if (!existsSync(file) || !statSync(file).isFile()) {
      response.writeHead(404).end("Not found");
      return;
    }
    response.writeHead(200, {
      "Content-Type": `${mime[path.extname(file)] ?? "application/octet-stream"}; charset=utf-8`,
    });
    response.end(request.method === "HEAD" ? undefined : readFileSync(file));
  } catch {
    response.writeHead(404).end("Not found");
  }
}).listen(port, "127.0.0.1", () =>
  console.log(`Static export: http://127.0.0.1:${port}${basePath}/`),
);
