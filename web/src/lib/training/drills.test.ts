import { describe, expect, it } from 'vitest';
import type { DrillItem, DrillQuiz } from '$lib/api/types';
import { accepts, drillableTag, gradeChoice, gradeTap, tally } from './drills';

const quiz = (q: Partial<DrillQuiz>): DrillQuiz => ({
  question: '',
  choices: [],
  answer_choice: null,
  answer_squares: [],
  arrow_uci: null,
  explanation: '',
  ...q
});

describe('drill grading', () => {
  it('accepts any holding defence, promotions included', () => {
    expect(accepts(['e2e4', 'g1f3'], 'g1f3')).toBe(true);
    expect(accepts(['e2e4'], 'd2d4')).toBe(false);
    expect(accepts(['a7a8q'], 'a7a8')).toBe(true);
    expect(accepts(['a7a8n'], 'a7a8q')).toBe(false);
  });

  it('grades taps and choices', () => {
    expect(gradeTap(quiz({ answer_squares: ['d5', 'e4'] }), 'e4')).toBe(true);
    expect(gradeTap(quiz({ answer_squares: ['d5'] }), 'd4')).toBe(false);
    expect(gradeChoice(quiz({ answer_choice: 2 }), 2)).toBe(true);
    expect(gradeChoice(quiz({ answer_choice: 2 }), 0)).toBe(false);
  });

  it('finds a drillable tag and tallies results by kind', () => {
    expect(drillableTag(['calculation', 'fork'])).toBe('fork');
    expect(drillableTag(['tempo'])).toBeNull();
    const item = (kind: DrillItem['kind'], solved: boolean | null) => ({ kind, solved }) as DrillItem;
    expect(tally([item('spot', true), item('find', false), item('spot', false), item('find', true)])).toEqual([
      { kind: 'spot', solved: 1, total: 2 },
      { kind: 'find', solved: 1, total: 2 }
    ]);
  });
});
