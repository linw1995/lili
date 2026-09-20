import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = fileURLToPath(new URL("../dist/site/", import.meta.url));
const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".png": "image/png", ".webp": "image/webp", ".gif": "image/gif" };

export async function serveSite(port = 0, basePath = "/") {
  if (!/^\/(?:[a-zA-Z0-9_-]+\/)*$/.test(basePath)) throw new Error("Base path must be a slash-delimited path such as /lili/");
  const server = createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, "http://localhost").pathname);
      if (basePath !== "/" && pathname === basePath.slice(0, -1)) {
        response.writeHead(302, { Location: basePath }).end(); return;
      }
      if (!pathname.startsWith(basePath)) { response.writeHead(404).end("Not found"); return; }
      const relative = pathname.slice(basePath.length);
      const file = path.resolve(root, relative, ...(pathname.endsWith("/") ? ["index.html"] : []));
      if (!file.startsWith(root)) { response.writeHead(403).end(); return; }
      const content = await readFile(file);
      response.writeHead(200, { "Content-Type": types[path.extname(file)] ?? "application/octet-stream", "Cache-Control": "no-store" });
      response.end(content);
    } catch {
      response.writeHead(404).end("Not found");
    }
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", resolve);
  });
  return { server, url: `http://127.0.0.1:${server.address().port}${basePath}` };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const { url } = await serveSite(4173, process.env.LILI_SITE_BASE_PATH ?? "/");
  console.log(`Lili website: ${url}`);
}
