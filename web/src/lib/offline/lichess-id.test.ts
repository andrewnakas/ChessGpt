import { describe, expect, it } from 'vitest';
import { lichessId } from './backend';

describe('lichessId', () => {
  it('reads every Lichess game URL shape', () => {
    expect(lichessId('https://lichess.org/abcdEFGH')).toBe('abcdEFGH');
    expect(lichessId('https://lichess.org/abcdEFGHijkl')).toBe('abcdEFGH');
    // Since October 2026 game pages (and PGN Site tags) are /game/{id}.
    expect(lichessId('https://lichess.org/game/Hhli90m0')).toBe('Hhli90m0');
    expect(lichessId('https://lichess.org/game/Hhli90m0/black#33')).toBe('Hhli90m0');
    expect(lichessId('https://lichess.org/game/export/abcdEFGH')).toBe('abcdEFGH');
    expect(lichessId('https://lichess.org/embed/game/abcdEFGH?theme=brown')).toBe('abcdEFGH');
    expect(lichessId('abcdEFGH')).toBe('abcdEFGH');
    expect(lichessId('nope')).toBeNull();
  });
});
