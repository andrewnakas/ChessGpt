<script lang="ts">
  import { getContext } from 'svelte';
  import type { Meta } from '$lib/api/types';
  const app = getContext<{ meta: Meta | null }>('app');
</script>

<article class="card">
  <h1>About chessgpt</h1>
  <p>
    chessgpt is an open-source chess coach. Stockfish analyses your games; a language model explains the moments that
    mattered at your level. Every move the coach mentions is checked against the engine and the rules of chess, and
    each explanation shows whether it passed.
  </p>

  <h2>How moves are judged</h2>
  <p>
    Inaccuracies, mistakes, blunders and accuracy use exactly the formulas Lichess uses, so the numbers are comparable.
    chessgpt adds book, best, excellent and missed-win labels.
    <a href="https://github.com/andrewnakas/ChessGpt/blob/main/docs/classification.md">Details</a>.
  </p>

  <h2>Your data</h2>
  <p>
    Games, analyses and chats are stored on the server you are using. API keys are encrypted at rest and only sent to
    the AI provider you configured. When you ask the coach something, the position, the question and the engine's
    results are sent to that provider.
  </p>

  <h2>Source code and licenses</h2>
  <ul>
    <li>
      <a href="https://github.com/andrewnakas/ChessGpt">chessgpt</a>, version {app.meta?.version ?? '…'}: GNU Affero General
      Public License v3.0. If you use this service over a network, you are entitled to its source code at that link.
    </li>
    <li>
      <a href="https://github.com/official-stockfish/Stockfish">Stockfish</a> ({app.meta?.engine ?? 'Stockfish'}): GPL-3.0.
      It runs as a separate program; its source is at that link.
    </li>
    <li>
      <a href="https://github.com/lichess-org/chessground">chessground</a>,
      <a href="https://github.com/niklasf/chessops">chessops</a>,
      <a href="https://github.com/niklasf/shakmaty">shakmaty</a>: GPL-3.0-or-later.
    </li>
    <li><a href="https://github.com/lichess-org/chess-openings">lichess-org/chess-openings</a>: CC0.</li>
    <li>Piece set cburnett by Colin M.L. Burnett: GPL-2.0-or-later.</li>
  </ul>
</article>

<style>
  article {
    max-width: 48rem;
    padding: 1.2rem 1.6rem;
  }
  h1 {
    margin-top: 0;
    font-size: 1.5rem;
  }
  h2 {
    font-size: 1.1rem;
    margin: 1.2rem 0 0.3rem;
  }
</style>
