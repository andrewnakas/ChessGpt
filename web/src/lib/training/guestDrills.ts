// Drills for guests (no account): the server builds a set on request
// (POST /api/drills/try) and stores nothing; the set and its results live in
// this browser.
import type { DrillAttempt, DrillOverview, DrillSet, DrillSource, TechniqueMastery } from '$lib/api/types';
import { tagLabel } from '$lib/chess';
import { DRILLABLE } from './drills';

interface Stored {
  sets: DrillSet[];
  attempts: Record<string, DrillAttempt & { tag: string; at: number }>;
}

const KEY = 'chessgpt.guestDrills';
const KEEP = 20;
let memory: Stored = { sets: [], attempts: {} };

function load(): Stored {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) memory = JSON.parse(raw) as Stored;
  } catch {
    /* storage blocked: keep what this page has */
  }
  return memory;
}

function save(s: Stored) {
  memory = s;
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    /* storage blocked or full */
  }
}

const WEEK_MS = 7 * 86_400_000;
const MONTH_MS = 30 * 86_400_000;

export function overview(weakTags: string[]): DrillOverview {
  const s = load();
  const since = Date.now() - MONTH_MS;
  const techniques: TechniqueMastery[] = DRILLABLE.map((tag) => {
    const rows = Object.values(s.attempts).filter((a) => a.tag === tag && a.at >= since);
    const clean = rows.filter((a) => a.solved && a.hints_used === 0);
    const times = clean.map((a) => a.ms).sort((a, b) => a - b);
    return {
      tag,
      label: tagLabel(tag),
      attempted: rows.length,
      solved: rows.filter((a) => a.solved).length,
      clean: clean.length,
      median_ms: times.length ? times[Math.floor(times.length / 2)] : null
    };
  });
  const focus = [...weakTags.filter((t) => DRILLABLE.includes(t)), 'hanging_piece', 'fork'].filter((t, i, a) => a.indexOf(t) === i);
  return {
    techniques,
    focus: focus.slice(0, 3),
    week_done: s.sets.filter((x) => x.completed_at && x.completed_at >= Date.now() - WEEK_MS).length,
    week_goal: 5,
    recent: s.sets.slice(0, 10).map((x) => ({
      id: x.id,
      technique: x.technique,
      label: x.label,
      items: x.items.length,
      attempted: x.items.filter((i) => i.solved != null).length,
      solved: x.items.filter((i) => i.solved).length,
      created_at: x.created_at,
      completed_at: x.completed_at
    }))
  };
}

/** Which technique a guest set trains. */
export function guestTag(source: DrillSource, weakTags: string[]): string {
  if (source.kind === 'theme') return source.tag;
  return overview(weakTags).focus[0];
}

export function remember(set: DrillSet): DrillSet {
  const s = load();
  save({ ...s, sets: [set, ...s.sets.filter((x) => x.id !== set.id)].slice(0, KEEP) });
  return set;
}

export function get(id: string): DrillSet {
  const set = load().sets.find((x) => x.id === id);
  if (!set) throw new Error('This drill set is no longer in this browser.');
  return set;
}

/** Record the first attempt at an item, like the server does. */
export function attempt(setId: string, itemId: string, a: DrillAttempt): DrillSet {
  const s = load();
  const set = s.sets.find((x) => x.id === setId);
  const item = set?.items.find((i) => i.id === itemId);
  if (!set || !item) throw new Error('drill item not found');
  if (item.solved == null) {
    item.solved = a.solved;
    s.attempts[`${setId}/${itemId}`] = { ...a, tag: set.technique, at: Date.now() };
    if (set.items.every((i) => i.solved != null)) set.completed_at = Date.now();
    save(s);
  }
  return set;
}
