import { describe, expect, it } from 'vitest';
import { renderMarkdown } from './markdown';

describe('renderMarkdown', () => {
  it('escapes html and renders basics', () => {
    const html = renderMarkdown('Play **Nf3** <script>\n\n- one\n- two');
    expect(html).toBe('<p>Play <strong>Nf3</strong> &lt;script&gt;</p><ul><li>one</li><li>two</li></ul>');
  });

  it('does not treat move numbers as lists', () => {
    expect(renderMarkdown('1. e4 e5 is classical')).toBe('<p>1. e4 e5 is classical</p>');
  });

  it('flags unverified moves', () => {
    expect(renderMarkdown('Try Qh5 or Qh5+', ['Qh5'])).toContain('<mark class="unverified"');
  });
});
