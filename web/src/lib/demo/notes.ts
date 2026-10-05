// Coach notes for the home-page demo: Morphy vs Duke Karl / Count Isouard,
// Paris 1858 (the Opera Game). Keyed by ply; the engine data is in opera.json
// (Stockfish 19, depth 20, judged by the same code as the site's reviews).

export interface DemoNote {
  /** Shown when the move has just been played. */
  text: string;
  /** Before this ply the viewer is asked to find the move. */
  quiz?: string;
}

export const NOTES: Record<number, DemoNote> = {
  6: {
    text: 'A natural developing move, but the pin on f3 commits Black: after 4.dxe5 the bishop has to take on f3, or Black loses a pawn.'
  },
  8: {
    text: 'Giving up the bishop for the knight hands White the bishop pair and a lead in development. Stockfish preferred 4...Nc6, keeping the tension.'
  },
  12: {
    text: 'Normal-looking development, but it walks into a double attack. 6...Qf6 covered f7 first. Engines call this the first real mistake.'
  },
  13: {
    quiz: 'White to move. Black has left two weak points. Find the move that hits both.',
    text: 'A double attack: the queen and the c4 bishop both hit f7, and the queen also eyes b7. Black can only cover one cheaply.'
  },
  14: { text: 'Defends f7, but the queen now blocks the f8 bishop, and Black’s king is stuck in the centre.' },
  15: {
    text: 'Morphy turns down the b7 pawn (8.Qxb7 Qb4+ trades queens). Development first: every white piece is heading for the action.'
  },
  18: {
    text: 'Black tries to chase the bishop, but opening lines while your king is in the centre and your pieces are at home is asking for trouble. 9...Kd8 was safer.'
  },
  19: {
    quiz: 'White to move. Black just pushed a pawn at your bishop. Find the sacrifice that rips the position open.',
    text: 'A piece sacrifice for two pawns and open lines to the king. Black’s f8 bishop and h8 rook still haven’t moved, so the material barely matters.'
  },
  21: { text: 'Check, and after 11...Nbd7 the knight is pinned to the king. Pins are the theme of the rest of the game.' },
  23: { text: 'Castling with tempo: the rook arrives on the d-file at once, aimed at the pinned knight.' },
  25: {
    quiz: 'White to move. The d7 knight is pinned. Find the move that removes a defender.',
    text: 'Morphy gives a rook for the knight to remove a defender. After 13...Rxd7 14.Rd1 the new d7 rook is pinned too.'
  },
  27: { text: 'More pressure on the pin: the b5 bishop and the d1 rook both bear down on d7.' },
  28: { text: 'Black offers a queen trade to break the pin, but the back rank is about to give way.' },
  31: {
    quiz: 'White to move, and there is a forced mate in two. Find the move that made this game famous.',
    text: 'The queen sacrifice! It deflects the d7 knight: it must take on b8, and that opens d8.'
  },
  33: {
    text: 'Back-rank mate. The rook mates on d8, guarded by the g5 bishop, and Black’s own pieces box in the king. Every White piece took part; Black’s h8 rook and f8 bishop never moved.'
  }
};
