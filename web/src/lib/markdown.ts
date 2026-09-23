// Tiny, safe markdown for coach replies: paragraphs, bullet lists, **bold**,
// *italic* and `code`. Input is HTML-escaped first. Moves the verifier could
// not confirm are marked up so the reader can see them.

function escape(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function mark(out: string, moves: Set<string>, cls: string, title: string): string {
  for (const mv of moves) {
    const e = escapeRegex(escape(mv));
    out = out.replace(
      new RegExp(`(^|[^A-Za-z0-9])(${e})(?![A-Za-z0-9])`, 'g'),
      `$1<mark class="${cls}" title="${title}">$2</mark>`
    );
  }
  return out;
}

function inline(s: string, flagged: Set<string>, rejected: Set<string>): string {
  let out = escape(s)
    .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
    .replace(/(^|[^*])\*(?!\s)(.+?)\*(?!\*)/g, '$1<em>$2</em>')
    .replace(/`([^`]+)`/g, '<code>$1</code>');
  out = mark(out, flagged, 'unverified', 'Legal, but not checked by the engine');
  return mark(out, rejected, 'rejected', 'Illegal here or not supported by the engine');
}

/** Render to HTML; `flagged` (legal, unchecked) and `rejected` moves are marked. */
export function renderMarkdown(text: string, flagged: string[] = [], rejected: string[] = []): string {
  const set = new Set(flagged);
  const rej = new Set(rejected);
  const blocks: string[] = [];
  let list: string[] = [];
  let para: string[] = [];
  const flushPara = () => {
    if (para.length) blocks.push(`<p>${inline(para.join(' '), set, rej)}</p>`);
    para = [];
  };
  const flushList = () => {
    if (list.length) blocks.push(`<ul>${list.map((l) => `<li>${inline(l, set, rej)}</li>`).join('')}</ul>`);
    list = [];
  };
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    const bullet = line.match(/^(?:[-*•]|\d+\.)\s+(.*)$/);
    if (!line) {
      flushPara();
      flushList();
    } else if (bullet && !/^\d+\.\s*[a-hKQRBNO]/.test(line)) {
      flushPara();
      list.push(bullet[1]);
    } else if (/^#{1,6}\s/.test(line)) {
      flushPara();
      flushList();
      blocks.push(`<p><strong>${inline(line.replace(/^#+\s*/, ''), set, rej)}</strong></p>`);
    } else {
      flushList();
      para.push(line);
    }
  }
  flushPara();
  flushList();
  return blocks.join('');
}
