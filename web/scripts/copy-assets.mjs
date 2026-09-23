// Copies the browser engine and the opening book into static/ (browser mode).
import { copyFileSync, mkdirSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const web = join(dirname(fileURLToPath(import.meta.url)), '..');
const eng = join(web, 'static/engine');
mkdirSync(eng, { recursive: true });
for (const f of ['stockfish-19-lite-single.js', 'stockfish-19-lite-single.wasm']) {
  copyFileSync(join(web, 'node_modules/stockfish/bin', f), join(eng, f));
}
copyFileSync(join(web, 'node_modules/stockfish/Copying.txt'), join(eng, 'COPYING.txt'));
const open = join(web, 'static/openings');
mkdirSync(open, { recursive: true });
const src = join(web, '../crates/chess-core/data/openings');
for (const f of readdirSync(src)) copyFileSync(join(src, f), join(open, f));
console.log('copied engine and openings into static/');
