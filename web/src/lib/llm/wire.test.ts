import { describe, expect, it } from 'vitest';
import { stripThinking, toWebLlm } from './wire';

describe('wire', () => {
  it('maps json_schema to WebLLM json_object and forces non-streaming', () => {
    const out = toWebLlm(
      {
        model: 'x',
        stream: true,
        max_tokens: 16000,
        messages: [{ role: 'system', content: 's' }, { role: 'user', content: 'u' }],
        response_format: { type: 'json_schema', json_schema: { schema: { type: 'object' } } }
      },
      'Qwen3.5-4B-q4f16_1-MLC'
    );
    expect(out.model).toBe('Qwen3.5-4B-q4f16_1-MLC');
    expect(out.stream).toBe(false);
    expect(out.max_tokens).toBe(2048);
    expect(out.response_format).toEqual({ type: 'json_object', schema: '{"type":"object"}' });
  });

  it('strips thinking blocks', () => {
    expect(stripThinking('<think>\nhmm\n</think>\n\n{"a":1}')).toBe('{"a":1}');
  });
});
