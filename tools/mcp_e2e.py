"""End-to-end check of chessgpt's MCP server the way Claude/ChatGPT use it:
OAuth discovery, dynamic registration, sign-in, consent, PKCE token exchange,
then MCP over Streamable HTTP in both protocol eras.

Usage: python tools/mcp_e2e.py http://127.0.0.1:18081
Needs a server started with CHESSGPT_MODE=hosted and CHESSGPT_PUBLIC_URL set to that URL.
"""

import base64
import hashlib
import http.cookiejar
import json
import re
import secrets
import sys
import urllib.error
import urllib.parse
import urllib.request

BASE = sys.argv[1].rstrip("/") if len(sys.argv) > 1 else "http://127.0.0.1:18081"
REDIRECT = "https://claude.ai/api/mcp/auth_callback"
OPERA = open("fixtures/games/opera.pgn", encoding="utf8").read()

jar = http.cookiejar.CookieJar()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):
        return None


browser = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar), NoRedirect)
plain = urllib.request.build_opener(NoRedirect)


def call(opener, method, url, body=None, headers=None, form=False):
    data = None
    h = dict(headers or {})
    if body is not None:
        if form:
            data = urllib.parse.urlencode(body).encode()
            h["content-type"] = "application/x-www-form-urlencoded"
        else:
            data = json.dumps(body).encode()
            h["content-type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=h, method=method)
    try:
        r = opener.open(req, timeout=240)
        return r.status, dict(r.headers), r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers), e.read().decode()


def ok(cond, msg):
    print(("PASS " if cond else "FAIL ") + msg)
    if not cond:
        sys.exit(1)


# 1. Discovery
s, h, b = call(plain, "POST", f"{BASE}/mcp", {"jsonrpc": "2.0", "id": 1, "method": "tools/list"},
               {"accept": "application/json, text/event-stream"})
ok(s == 401 and "resource_metadata" in h.get("www-authenticate", h.get("WWW-Authenticate", "")), "unauthenticated /mcp -> 401 with resource_metadata")
prm = json.loads(call(plain, "GET", f"{BASE}/.well-known/oauth-protected-resource/mcp")[2])
ok(prm["resource"] == f"{BASE}/mcp", "protected resource metadata names /mcp")
asm = json.loads(call(plain, "GET", f"{BASE}/.well-known/oauth-authorization-server")[2])
ok(asm["issuer"] == BASE and "S256" in asm["code_challenge_methods_supported"], "authorization server metadata")

# 2. Dynamic client registration
s, _, b = call(plain, "POST", asm["registration_endpoint"], {"client_name": "Claude", "redirect_uris": [REDIRECT]})
client_id = json.loads(b)["client_id"]
ok(s == 201, f"registered client {client_id}")

# 3. Sign up (browser session)
email = f"e2e-{secrets.token_hex(4)}@example.com"
s, _, _ = call(browser, "POST", f"{BASE}/api/auth/register", {"email": email, "password": "correct horse battery"})
ok(s == 200, "created account and session")

# 4. Authorize + consent
verifier = secrets.token_urlsafe(48)
challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
q = {"response_type": "code", "client_id": client_id, "redirect_uri": REDIRECT, "code_challenge": challenge,
     "code_challenge_method": "S256", "state": "xyz", "scope": "chess", "resource": f"{BASE}/mcp"}
s, _, page = call(browser, "GET", f"{asm['authorization_endpoint']}?{urllib.parse.urlencode(q)}")
ok(s == 200 and "Connect Claude to chessgpt" in page, "consent page shown")
csrf = re.search(r'name=csrf value="([^"]+)"', page).group(1)
s, h, _ = call(browser, "POST", asm["authorization_endpoint"], {**q, "csrf": csrf, "decision": "allow"}, form=True)
loc = h.get("location") or h.get("Location")
ok(s in (302, 303, 307) and loc.startswith(REDIRECT), "consent redirects back to Claude")
params = urllib.parse.parse_qs(urllib.parse.urlparse(loc).query)
ok(params["state"] == ["xyz"] and params["iss"] == [BASE], "state and iss returned")
code = params["code"][0]

# 5. Token exchange (with a wrong verifier first)
s, _, _ = call(plain, "POST", asm["token_endpoint"], {"grant_type": "authorization_code", "code": code, "code_verifier": "wrong" * 10,
                                                      "client_id": client_id, "redirect_uri": REDIRECT}, form=True)
ok(s == 400, "wrong PKCE verifier rejected (and the code is burned)")
s, _, page = call(browser, "GET", f"{asm['authorization_endpoint']}?{urllib.parse.urlencode(q)}")
csrf = re.search(r'name=csrf value="([^"]+)"', page).group(1)
_, h, _ = call(browser, "POST", asm["authorization_endpoint"], {**q, "csrf": csrf, "decision": "allow"}, form=True)
code = urllib.parse.parse_qs(urllib.parse.urlparse(h.get("location") or h.get("Location")).query)["code"][0]
s, _, b = call(plain, "POST", asm["token_endpoint"], {"grant_type": "authorization_code", "code": code, "code_verifier": verifier,
                                                      "client_id": client_id, "redirect_uri": REDIRECT, "resource": f"{BASE}/mcp"}, form=True)
tok = json.loads(b)
ok(s == 200 and tok["token_type"] == "Bearer", "access token issued")
s, _, b = call(plain, "POST", asm["token_endpoint"], {"grant_type": "refresh_token", "refresh_token": tok["refresh_token"], "client_id": client_id}, form=True)
tok = json.loads(b)
ok(s == 200, "refresh token rotated")
AUTH = {"authorization": f"Bearer {tok['access_token']}", "accept": "application/json, text/event-stream"}


def rpc(method, params=None, modern=False, rid=[0]):
    rid[0] += 1
    body = {"jsonrpc": "2.0", "id": rid[0], "method": method}
    hdr = dict(AUTH)
    p = dict(params or {})
    if modern:
        p["_meta"] = {"io.modelcontextprotocol/protocolVersion": "2026-07-28",
                      "io.modelcontextprotocol/clientCapabilities": {"extensions": {"io.modelcontextprotocol/ui": {"mimeTypes": ["text/html;profile=mcp-app"]}}},
                      "io.modelcontextprotocol/clientInfo": {"name": "e2e", "version": "1"}}
        hdr["mcp-protocol-version"] = "2026-07-28"
        hdr["mcp-method"] = method
        if "name" in p:
            hdr["mcp-name"] = p["name"]
        elif "uri" in p:
            hdr["mcp-name"] = p["uri"]
    if p:
        body["params"] = p
    s, h, b = call(plain, "POST", f"{BASE}/mcp", body, hdr)
    if b.startswith("event:") or b.startswith("data:"):
        b = next(l[5:].strip() for l in b.splitlines() if l.startswith("data:") and '"id"' in l)
    try:
        return s, json.loads(b)
    except Exception:
        return s, {"raw": b}


# 6a. Legacy era
s, r = rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "e2e", "version": "1"}})
ok(s == 200 and r["result"]["serverInfo"]["name"] == "chessgpt", f"legacy initialize -> {r['result']['protocolVersion']}")
call(plain, "POST", f"{BASE}/mcp", {"jsonrpc": "2.0", "method": "notifications/initialized"}, AUTH)
s, r = rpc("tools/list")
names = [t["name"] for t in r["result"]["tools"]]
ok({"analyze_position", "check_line", "analyze_game", "game_report", "my_games", "my_weaknesses"} <= set(names), f"tools/list: {', '.join(names)}")
ok(all("annotations" in t and ("readOnlyHint" in t["annotations"]) for t in r["result"]["tools"]), "every tool has readOnlyHint")
ap = next(t for t in r["result"]["tools"] if t["name"] == "analyze_position")
ok(ap.get("_meta", {}).get("ui", {}).get("resourceUri", "").startswith("ui://"), "analyze_position declares the board widget")

# 6b. Modern era
s, r = rpc("server/discover", modern=True)
ok(s == 200 and "2026-07-28" in r.get("result", {}).get("supportedVersions", []), f"server/discover (2026-07-28): {r.get('result', r).get('supportedVersions') if isinstance(r.get('result'), dict) else r}")

s, r = rpc("tools/call", {"name": "analyze_position", "arguments": {"fen": "r2qkb1r/pp2nppp/3p4/2pNN1B1/2BnP3/3P4/PPP2PPP/R2bK2R w KQkq - 1 10", "lines": 2}}, modern=True)
sc = r["result"].get("structuredContent", {})
ok(sc.get("lines", [{}])[0].get("san", [""])[0] == "Nf6+", "analyze_position finds the mate (Nf6+)")
ok("chessgpt" in r["result"]["content"][0]["text"] and sc.get("url", "").startswith(BASE), "result links back to chessgpt")

s, r = rpc("tools/call", {"name": "check_line", "arguments": {"fen": "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", "moves": ["e4", "e5", "Ke3"]}})
ok(r["result"].get("isError") is True and "Ke3" in r["result"]["content"][0]["text"], "check_line rejects an illegal move")

s, r = rpc("tools/call", {"name": "analyze_game", "arguments": {"pgn": OPERA, "my_color": "black", "my_rating": 1300}}, modern=True)
sc = r["result"].get("structuredContent", {})
ok(sc.get("kind") == "game" and sc.get("key_moments"), f"analyze_game: accuracy {sc.get('white_accuracy')} / {sc.get('black_accuracy')}, {len(sc.get('key_moments', []))} key moments")
ok(sc["url"].startswith(f"{BASE}/analyse/"), f"game link: {sc['url']}")
gid = sc["game_id"]

s, r = rpc("tools/call", {"name": "my_games", "arguments": {}})
ok(any(g["game_id"] == gid for g in r["result"]["structuredContent"]["games"]), "game is in the user's library")

s, r = rpc("tools/call", {"name": "my_weaknesses", "arguments": {}})
ok("phase" in r["result"]["content"][0]["text"].lower(), "my_weaknesses: " + r["result"]["content"][0]["text"].replace("\n", " | "))

s, r = rpc("resources/read", {"uri": ap["_meta"]["ui"]["resourceUri"]}, modern=True)
c = r["result"]["contents"][0]
ok(c["mimeType"] == "text/html;profile=mcp-app" and "ui/initialize" in c["text"], "board widget served as an MCP App")

# 7. The same account sees the game on the website
s, _, b = call(browser, "GET", f"{BASE}/api/games/{gid}")
ok(s == 200 and json.loads(b)["analysis"]["status"] == "done", "website shows the analysed game for the signed-in user")
s, _, _ = call(plain, "GET", f"{BASE}/api/games/{gid}")
ok(s == 401, "not visible without signing in")
print("ALL PASSED")
