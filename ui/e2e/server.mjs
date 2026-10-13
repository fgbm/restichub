// Serves the built UI with the production Content Security Policy.
//
// The built index.html holds the policy in a `<meta http-equiv="content-security-policy">` tag
// with the SHA-256 hashes of the inline bootstrap scripts (the SvelteKit CSP configuration).
// The server sends that policy as an HTTP header, the way the control plane does, so the test
// exercises the header and not only the meta tag.
import { createServer } from 'node:http';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { extname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../build/', import.meta.url));
const index = readFileSync(join(root, 'index.html'), 'utf8');
const meta = index.match(/http-equiv="content-security-policy"\s+content="([^"]*)"/i);
const policy = meta ? meta[1] + "; frame-ancestors 'none'; base-uri 'none'" : "default-src 'self'";

const types = {
	'.html': 'text/html',
	'.js': 'text/javascript',
	'.css': 'text/css',
	'.svg': 'image/svg+xml',
	'.json': 'application/json',
	'.woff2': 'font/woff2',
	'.png': 'image/png'
};

createServer((request, response) => {
	const url = new URL(request.url ?? '/', 'http://127.0.0.1');
	let path = join(root, decodeURIComponent(url.pathname));
	if (!existsSync(path) || statSync(path).isDirectory()) path = join(root, 'index.html');
	response.setHeader('content-type', types[extname(path)] ?? 'application/octet-stream');
	response.setHeader('content-security-policy', policy);
	response.end(readFileSync(path));
}).listen(4173, '127.0.0.1');
