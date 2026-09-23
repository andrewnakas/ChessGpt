// A sparring bot at a chosen rating: Stockfish's top lines, softened. Lower
// ratings search shallower, pick among near-best moves more loosely, and now
// and then play a plain mistake, which is what games at that level look like.
// (Stockfish's own UCI_Elo floor is about 1320, too strong for most players.)

export interface BotStyle {
  depth: number;
  /** Softmax temperature over the lines' win% (higher = looser). */
  temperature: number;
  /** Chance per move of a random legal move instead. */
  blunder: number;
}

const TABLE: [number, BotStyle][] = [
  [800, { depth: 3, temperature: 14, blunder: 0.12 }],
  [1100, { depth: 5, temperature: 9, blunder: 0.07 }],
  [1400, { depth: 7, temperature: 6, blunder: 0.04 }],
  [1700, { depth: 9, temperature: 4, blunder: 0.02 }],
  [2000, { depth: 11, temperature: 2.5, blunder: 0.008 }],
  [2300, { depth: 14, temperature: 1, blunder: 0.002 }],
  [2600, { depth: 18, temperature: 0.3, blunder: 0 }]
];

/** Style for a rating, interpolated between the table's rows. */
export function styleFor(elo: number): BotStyle {
  if (elo <= TABLE[0][0]) return TABLE[0][1];
  for (let i = 1; i < TABLE.length; i++) {
    const [e1, s1] = TABLE[i];
    if (elo === e1) return s1;
    if (elo < e1) {
      const [e0, s0] = TABLE[i - 1];
      const t = (elo - e0) / (e1 - e0);
      const mix = (a: number, b: number) => a + (b - a) * t;
      return { depth: Math.round(mix(s0.depth, s1.depth)), temperature: mix(s0.temperature, s1.temperature), blunder: mix(s0.blunder, s1.blunder) };
    }
  }
  return TABLE[TABLE.length - 1][1];
}

export interface Candidate {
  uci: string;
  /** Bot's win% after this move (0..100). */
  win: number;
}

/** Choose a move; `rand` returns [0, 1). */
export function choose(cands: Candidate[], legal: string[], style: BotStyle, rand: () => number = Math.random): string {
  if (legal.length && rand() < style.blunder) return legal[Math.floor(rand() * legal.length)];
  if (!cands.length) return legal[0];
  const best = Math.max(...cands.map((c) => c.win));
  const w = cands.map((c) => Math.exp((c.win - best) / Math.max(0.01, style.temperature)));
  let r = rand() * w.reduce((a, b) => a + b, 0);
  for (let i = 0; i < cands.length; i++) {
    r -= w[i];
    if (r <= 0) return cands[i].uci;
  }
  return cands[0].uci;
}
