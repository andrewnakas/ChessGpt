// chessgpt at the edge. The web app is served from Cloudflare directly, so it
// is up even when the server isn't; server paths are proxied to the Oracle
// origin through a Cloudflare Tunnel. When the origin is down the app falls
// back to browser mode (Stockfish in the visitor's browser).

const PROXIED = ['/api/', '/mcp', '/oauth/', '/.well-known/', '/auth/'];
// Cloudflare's own "origin unreachable" statuses; chessgpt itself never sends these.
const ORIGIN_DOWN = new Set([502, 504, 520, 521, 522, 523, 524, 530]);

function proxied(path) {
  return PROXIED.some((p) => path === p.replace(/\/$/, '') || path.startsWith(p));
}

function offline(url) {
  const msg = 'The chessgpt server is offline right now. Analysis still works in your browser.';
  if (url.pathname.startsWith('/api/') || url.pathname === '/mcp' || url.pathname.startsWith('/.well-known/')) {
    return new Response(JSON.stringify({ error: msg, offline: true }), {
      status: 503,
      headers: { 'content-type': 'application/json', 'retry-after': '60', 'cache-control': 'no-store' }
    });
  }
  const html = `<!doctype html><meta charset=utf-8><meta name=viewport content="width=device-width,initial-scale=1">
<title>chessgpt</title><body style="font-family:system-ui;max-width:32rem;margin:15vh auto;padding:0 1rem">
<h1>chessgpt is catching its breath</h1><p>${msg}</p><p><a href="/">Open chessgpt</a></p></body>`;
  return new Response(html, { status: 503, headers: { 'content-type': 'text/html; charset=utf-8', 'retry-after': '60' } });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (!proxied(url.pathname)) return env.ASSETS.fetch(request);

    const target = new URL(url.pathname + url.search, env.ORIGIN);
    const headers = new Headers(request.headers);
    headers.set('x-forwarded-host', url.host);
    headers.set('x-forwarded-proto', 'https');
    try {
      const res = await fetch(target, {
        method: request.method,
        headers,
        body: request.method === 'GET' || request.method === 'HEAD' ? undefined : request.body,
        redirect: 'manual'
      });
      if (ORIGIN_DOWN.has(res.status)) return offline(url);
      return res;
    } catch {
      return offline(url);
    }
  }
};
