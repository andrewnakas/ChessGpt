# chessgpt: build a pool of coaching moments from Lichess games with evals.
# Streams the August 2026 rated database, keeps games with [%eval] comments
# whose players sit in the same rating band (up to GAMES_PER_BAND each), then
# runs `chessgpt-lab build-set --pgn-evals` (Stockfish deep pass on the key
# moments). Stops before Kaggle's 12 h limit so the output is always saved.
import os, re, subprocess, sys, time
T0 = time.time()
def sh(c, **kw):
    print("+", c, flush=True)
    return subprocess.run(c, shell=True, check=True, **kw)

W = "/kaggle/working"
sh("curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null")
os.environ["PATH"] = os.path.expanduser("~/.cargo/bin") + ":" + os.environ["PATH"]
sh("git clone -q --depth 1 https://github.com/andrewnakas/ChessGpt /kaggle/tmp/repo")
sh("cd /kaggle/tmp/repo && cargo build -q --release -p lab")
sh("mkdir -p /kaggle/tmp/sf && cd /kaggle/tmp/sf && curl -sL -o sf.tgz https://github.com/official-stockfish/Stockfish/releases/latest/download/stockfish-linux-x86-64-universal.tar.gz && tar xzf sf.tgz")
sf = subprocess.run("find /kaggle/tmp/sf -type f -name 'stockfish*' ! -name '*.tgz' -perm -u+x | head -1", shell=True, capture_output=True, text=True).stdout.strip()
print("stockfish:", sf, flush=True)
assert sf, "no stockfish binary"
os.environ["STOCKFISH_PATH"] = sf
print("build done", int(time.time() - T0), "s", flush=True)

BANDS = [(0, "<1000"), (1000, "1000-1399"), (1400, "1400-1799"), (1800, "1800-2099"), (2100, "2100-2399"), (2400, "2400+")]
band = lambda e: [b for lo, b in BANDS if e >= lo][-1]
GAMES_PER_BAND = 1000
want = {b: GAMES_PER_BAND for _, b in BANDS}
games = []
url = "https://database.lichess.org/standard/lichess_db_standard_rated_2026-08.pgn.zst"
subprocess.run([sys.executable, "-m", "pip", "install", "-q", "zstandard"], check=True)
import io, urllib.request, zstandard
resp = urllib.request.urlopen(url)
stream = io.TextIOWrapper(zstandard.ZstdDecompressor().stream_reader(resp), encoding="utf-8", errors="replace")
cur, seen = [], 0
def flush(lines):
    text = "".join(lines)
    if "%eval" not in text:
        return
    m1 = re.search(r'\[WhiteElo "(\d+)"\]', text); m2 = re.search(r'\[BlackElo "(\d+)"\]', text)
    tc = re.search(r'\[TimeControl "(\d+)\+', text)
    if not (m1 and m2 and tc) or int(tc.group(1)) < 180:  # blitz and slower
        return
    bw, bb = band(int(m1.group(1))), band(int(m2.group(1)))
    if bw == bb and want[bw] > 0:
        want[bw] -= 1
        games.append(text)
for line in stream:
    if line.startswith("[Event ") and cur:
        seen += 1
        flush(cur); cur = []
        if not any(want.values()) or time.time() - T0 > 3 * 3600:
            break
    cur.append(line)
resp.close()
print("scanned", seen, "games; kept", len(games), "missing", want, flush=True)
# Interleave bands so a time-limited run still covers all of them.
by = {}
for g in games:
    by.setdefault(band(int(re.search(r'\[WhiteElo "(\d+)"\]', g).group(1))), []).append(g)
order = []
while any(by.values()):
    for b in list(by):
        if by[b]:
            order.append(by[b].pop())
open("/kaggle/tmp/games.pgn", "w").write("\n".join(order))
left = int(11 * 3600 - (time.time() - T0))
try:
    sh(f"cd /kaggle/tmp/repo && ./target/release/chessgpt-lab build-set --pgn-evals --depth 14 --per-game 2 --per-band 100000 --out {W}/moments-2026-08.jsonl /kaggle/tmp/games.pgn 2>&1 | tail -n 3", timeout=max(600, left))
except subprocess.TimeoutExpired:
    print("time limit reached; keeping what was built", flush=True)
n = sum(1 for _ in open(f"{W}/moments-2026-08.jsonl"))
print("moments:", n, flush=True)
