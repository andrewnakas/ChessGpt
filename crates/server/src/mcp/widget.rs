//! The MCP App (interactive board) shown inside Claude and ChatGPT. One
//! self-contained HTML document: no network access, no external scripts, so
//! it works under both hosts' default content-security policies.

pub const MIME: &str = "text/html;profile=mcp-app";

pub fn html(base: &str) -> String {
    TEMPLATE.replace("__BASE__", &base.replace('"', ""))
}

const TEMPLATE: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>chessgpt</title>
<style>
  :root { --light:#f0d9b5; --dark:#b58863; --text:#1d1b18; --muted:#6b665d; --bg:#fff; --line:#dedad2;
          --best:#15781b; --bad:#c33; --accent:#b7791f; font-family: system-ui, -apple-system, "Segoe UI", sans-serif; }
  @media (prefers-color-scheme: dark) { :root { --text:#e9e6df; --muted:#a39e94; --bg:#211f1c; --line:#3a3733; } }
  body { margin:0; background:transparent; color:var(--text); font-size:14px; }
  .wrap { display:flex; gap:14px; padding:10px; flex-wrap:wrap; align-items:flex-start; }
  .board { width:min(340px, 92vw); flex:none; }
  svg { display:block; width:100%; height:auto; border-radius:6px; }
  .side { flex:1; min-width:220px; display:flex; flex-direction:column; gap:6px; }
  .title { font-weight:700; font-size:15px; }
  .muted { color:var(--muted); }
  .line { display:flex; gap:8px; align-items:baseline; font-family: ui-monospace, Consolas, monospace; font-size:13px; }
  .score { font-weight:700; min-width:48px; }
  .moment { border:1px solid var(--line); border-radius:8px; padding:6px 8px; cursor:pointer; background:var(--bg); text-align:left; color:inherit; font:inherit; }
  .moment.on { border-color:var(--accent); box-shadow:0 0 0 1px var(--accent); }
  .cls { font-weight:700; }
  .blunder, .missed { color:var(--bad); } .mistake { color:#d9820a; } .inaccuracy { color:#3b82c4; }
  .open { margin-top:4px; align-self:flex-start; background:var(--accent); color:#fff; border:none; border-radius:8px; padding:7px 12px; font-weight:600; cursor:pointer; }
  .list { display:flex; flex-direction:column; gap:5px; max-height:300px; overflow:auto; }
  .empty { padding:14px; }
  .piece { font-family: "Segoe UI Symbol","DejaVu Sans","Noto Sans Symbols 2","Apple Symbols",serif; }
</style>
</head>
<body>
<div id="app" class="empty muted">Waiting for chessgpt…</div>
<script>
(() => {
  const BASE = "__BASE__";
  const GLYPH = { K:"♔", Q:"♕", R:"♖", B:"♗", N:"♘", P:"♙", k:"♚", q:"♛", r:"♜", b:"♝", n:"♞", p:"♟" };
  let nextId = 1; const pending = {};
  let data = null; let selected = 0;

  function send(msg) { window.parent.postMessage(msg, "*"); }
  function request(method, params) {
    const id = nextId++;
    send({ jsonrpc: "2.0", id, method, params });
    return new Promise((res) => { pending[id] = res; setTimeout(() => { if (pending[id]) { delete pending[id]; res(null); } }, 4000); });
  }
  function notify(method, params) { send({ jsonrpc: "2.0", method, params }); }

  function openLink(url) {
    if (!url) return;
    if (window.openai && window.openai.openExternal) { window.openai.openExternal({ href: url }); return; }
    request("ui/open-link", { url }).then((r) => { if (r === null) window.open(url, "_blank", "noopener"); });
  }

  function parseFen(fen) {
    const rows = (fen || "").split(" ")[0].split("/");
    const b = [];
    for (const row of rows) { const r = []; for (const ch of row) { if (/\d/.test(ch)) for (let i = 0; i < +ch; i++) r.push(null); else r.push(ch); } b.push(r); }
    return b.length === 8 ? b : null;
  }
  function sq(s) { return { x: s.charCodeAt(0) - 97, y: 8 - +s[1] }; }

  function boardSvg(fen, arrows, flip) {
    const b = parseFen(fen); if (!b) return "";
    let s = '<svg viewBox="0 0 80 80" xmlns="http://www.w3.org/2000/svg">';
    s += '<defs>' + ["best","bad","alt"].map(k => `<marker id="h-${k}" markerWidth="4" markerHeight="4" refX="2.2" refY="2" orient="auto"><path d="M0,0 L4,2 L0,4 z" fill="var(--${k === "alt" ? "accent" : k})"/></marker>`).join("") + '</defs>';
    for (let y = 0; y < 8; y++) for (let x = 0; x < 8; x++) {
      const light = (x + y) % 2 === 0;
      s += `<rect x="${x*10}" y="${y*10}" width="10" height="10" fill="var(--${light ? "light" : "dark"})"/>`;
    }
    for (let y = 0; y < 8; y++) for (let x = 0; x < 8; x++) {
      const p = b[y][x]; if (!p) continue;
      const dx = flip ? 7 - x : x, dy = flip ? 7 - y : y;
      const white = p === p.toUpperCase();
      s += `<text class="piece" x="${dx*10+5}" y="${dy*10+8.3}" font-size="9" text-anchor="middle" fill="${white ? "#fff" : "#111"}" stroke="${white ? "#222" : "#000"}" stroke-width="${white ? 0.35 : 0.1}">${GLYPH[p.toLowerCase()]}</text>`;
    }
    for (const a of arrows || []) {
      if (!a.uci || a.uci.length < 4) continue;
      let f = sq(a.uci.slice(0,2)), t = sq(a.uci.slice(2,4));
      if (flip) { f = { x: 7-f.x, y: 7-f.y }; t = { x: 7-t.x, y: 7-t.y }; }
      const col = a.kind === "alt" ? "var(--accent)" : `var(--${a.kind})`;
      s += `<line x1="${f.x*10+5}" y1="${f.y*10+5}" x2="${t.x*10+5}" y2="${t.y*10+5}" stroke="${col}" stroke-width="1.6" stroke-opacity="0.8" stroke-linecap="round" marker-end="url(#h-${a.kind})"/>`;
    }
    for (let i = 0; i < 8; i++) {
      const file = String.fromCharCode(97 + (flip ? 7 - i : i)), rank = flip ? i + 1 : 8 - i;
      s += `<text x="${i*10+9.2}" y="79.3" font-size="2.4" text-anchor="end" fill="${(i+7)%2===0 ? "var(--dark)" : "var(--light)"}">${file}</text>`;
      s += `<text x="0.6" y="${i*10+2.8}" font-size="2.4" fill="${i%2===0 ? "var(--dark)" : "var(--light)"}">${rank}</text>`;
    }
    return s + "</svg>";
  }

  function fmt(score) {
    if (!score) return "";
    if (score.kind === "mate") return score.value > 0 ? "#" + score.value : "#-" + (-score.value);
    const v = score.value / 100; return (v > 0 ? "+" : "") + v.toFixed(1);
  }
  const esc = (t) => String(t ?? "").replace(/[&<>"]/g, (c) => ({ "&":"&amp;", "<":"&lt;", ">":"&gt;", '"':"&quot;" }[c]));

  function render() {
    const el = document.getElementById("app");
    if (!data) return;
    el.className = "";
    const flip = data.user_side === "black";
    const openBtn = data.url ? `<button class="open" id="open">Open on chessgpt</button>` : "";
    let html = "";
    if (data.kind === "analyse_position") {
      const lines = data.lines || [];
      const arrows = lines.map((l, i) => ({ uci: (l.uci || [])[0], kind: i === 0 ? "best" : "alt" }));
      html = `<div class="wrap"><div class="board">${boardSvg(data.fen, arrows, false)}</div><div class="side">
        <div class="title">Stockfish${data.depth ? " · depth " + data.depth : ""}</div>
        ${lines.map((l) => `<div class="line"><span class="score">${fmt(l.score)}</span><span>${esc((l.san || []).slice(0, 10).join(" "))}</span></div>`).join("")}
        ${openBtn}</div></div>`;
    } else if (data.kind === "play_line") {
      const cont = (data.continuation || [])[0];
      html = `<div class="wrap"><div class="board">${boardSvg(data.fen, [], false)}</div><div class="side">
        <div class="title">Line checked ${data.score ? "· " + fmt(data.score) : ""}</div>
        <div class="line">${esc((data.san || []).join(" "))}</div>
        ${data.continuation && data.continuation.length ? `<div class="muted">Best continuation: ${esc(data.continuation.join(" "))}</div>` : ""}
        ${openBtn}</div></div>`;
      void cont;
    } else if (data.kind === "game") {
      const ms = data.key_moments || [];
      const m = ms[selected];
      const arrows = m ? [{ uci: m.uci, kind: "bad" }, { uci: m.best_uci, kind: "best" }] : [];
      const acc = (a) => a == null ? "–" : a.toFixed(1) + "%";
      html = `<div class="wrap"><div class="board">${boardSvg(m ? m.fen_before : data.fen, arrows, flip)}</div><div class="side">
        <div class="title">${esc(data.white)} vs ${esc(data.black)} <span class="muted">${esc(data.result || "")}</span></div>
        <div class="muted">${esc(data.opening || "")}</div>
        <div>Accuracy: White ${acc(data.white_accuracy)} · Black ${acc(data.black_accuracy)}</div>
        <div class="list">${ms.map((k, i) => `<button class="moment ${i === selected ? "on" : ""}" data-i="${i}">
          <b>${esc(k.move)}</b> <span class="cls ${esc((k.classification || "").split(" ")[0])}">${esc(k.classification)}</span>
          ${k.best_move && k.best_move !== k.san ? `<div class="muted">Better: ${esc((k.best_line || []).slice(0, 6).join(" "))}</div>` : ""}</button>`).join("") || '<div class="muted">No key moments: a clean game.</div>'}</div>
        ${openBtn}</div></div>`;
    } else if (data.kind === "games") {
      html = `<div class="wrap"><div class="side"><div class="title">Your games</div><div class="list">
        ${(data.games || []).map((g) => `<button class="moment" data-url="${esc(g.url)}"><b>${esc(g.white)} vs ${esc(g.black)}</b> ${esc(g.result)}<div class="muted">${esc(g.opening || "")}</div></button>`).join("")}
        </div>${openBtn}</div></div>`;
    } else {
      html = `<div class="wrap"><div class="side"><div class="title">chessgpt</div>${openBtn}</div></div>`;
    }
    el.innerHTML = html;
    const b = document.getElementById("open"); if (b) b.onclick = () => openLink(data.url);
    el.querySelectorAll("[data-i]").forEach((n) => n.onclick = () => { selected = +n.dataset.i; render(); });
    el.querySelectorAll("[data-url]").forEach((n) => n.onclick = () => openLink(n.dataset.url));
    notify("ui/notifications/size-changed", { width: document.body.scrollWidth, height: document.body.scrollHeight });
  }

  function accept(result) {
    const sc = result && (result.structuredContent || result);
    if (!sc || typeof sc !== "object" || !sc.kind) return;
    data = sc; selected = 0; render();
  }

  window.addEventListener("message", (ev) => {
    const m = ev.data; if (!m || m.jsonrpc !== "2.0") return;
    if (m.id != null && pending[m.id]) { const r = pending[m.id]; delete pending[m.id]; r(m.result ?? m.error ?? {}); return; }
    if (m.method === "ui/notifications/tool-result") accept(m.params);
  });

  // ChatGPT's window.openai globals, when present.
  function fromOpenai() { if (window.openai && window.openai.toolOutput) accept(window.openai.toolOutput); }
  window.addEventListener("openai:set_globals", fromOpenai);

  request("ui/initialize", {
    protocolVersion: "2026-01-26",
    appInfo: { name: "chessgpt-board", version: "1" },
    appCapabilities: {}
  }).then(() => notify("ui/notifications/initialized", {}));
  fromOpenai();
  void BASE;
})();
</script>
</body>
</html>
"##;
