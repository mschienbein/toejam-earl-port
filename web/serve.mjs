// Local development/acceptance server. Deliberately no COOP/COEP headers: this
// exercises the same non-isolated deployment model as GitHub Pages.
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { dirname, extname, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const ROOT = dirname(fileURLToPath(import.meta.url));
const MIME = new Map([
  ['.html', 'text/html; charset=utf-8'], ['.js', 'text/javascript; charset=utf-8'],
  ['.css', 'text/css; charset=utf-8'], ['.wasm', 'application/wasm'],
  ['.json', 'application/json'], ['.map', 'application/json'],
  ['.png', 'image/png'],
]);

export async function startServer(port = 8766, root = ROOT) {
  const base = resolve(root);
  const server = createServer(async (request, response) => {
    try {
      const url = new URL(request.url, 'http://localhost');
      const pathname = url.pathname.endsWith('/') ? `${url.pathname}index.html` : url.pathname;
      const path = resolve(base, `.${decodeURIComponent(pathname)}`);
      const type = MIME.get(extname(path));
      if (!path.startsWith(`${base}${sep}`) || !type || !['GET', 'HEAD'].includes(request.method)) {
        response.writeHead(404).end();
        return;
      }
      const bytes = await readFile(path);
      response.writeHead(200, { 'Content-Type': type, 'Content-Length': bytes.length, 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' });
      response.end(request.method === 'HEAD' ? undefined : bytes);
    } catch (error) {
      response.writeHead(error.code === 'ENOENT' ? 404 : 400).end();
    }
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, '127.0.0.1', resolve);
  });
  return server;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const server = await startServer(Number(process.env.PORT ?? 8766));
  console.log(`Serving web/ at http://127.0.0.1:${server.address().port}/ (no COOP/COEP)`);
}
