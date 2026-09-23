import { describe, expect, it } from 'vitest';
import { performance, rolling } from './progress';

describe('progress helpers match chess_core::rating', () => {
  it('rolling and performance', () => {
    const shrunk = Math.round(1280.145 + 0.22304 * 1200);
    const [r, m] = rolling([...Array(5).fill(400), ...Array(20).fill(shrunk)])!;
    expect(r).toBeGreaterThanOrEqual(1175);
    expect(r).toBeLessThanOrEqual(1225);
    expect(m).toBeGreaterThan(100);
    expect(rolling([])).toBeNull();
    expect(performance([[1500, 1], [1500, 0], [1600, 0.5]])).toBe(1533);
    expect(performance([[1500, 1]])).toBe(1900);
  });
});
