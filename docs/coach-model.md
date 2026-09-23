# The free coach model

chessgpt's coach can run without paying for a model API. A small open model
(Qwen3.5-4B by default, Apache-2.0) runs **in the visitor's browser** over
WebGPU. Browsers without WebGPU fall back to the same kind of model on the
server's CPU (llama.cpp). A site key for a frontier model still works and is
used as the fallback when set.

A 4B model can't calculate chess the way a frontier model half-does, so the
design is to have it **narrate facts the program has already verified**:

- **Motifs.** `chess-core/src/motifs.rs` detects forks, pins, skewers,
  discovered attacks, hanging and trapped pieces, back-rank and mate threats,
  promotions and pawn-structure features. It checks them against the Lichess
  puzzle database's theme tags (`fixtures/puzzles/`, test
  `lichess_puzzle_recall`). It also turns the engine's lines into plain facts
  ("10... cxb5 wins the loose knight on b5; net result: Black is a pawn up").
  The prompt (`explain.v2`) hands these to the model as MOTIFS.
- **Verification.** Every move the model writes is still checked against the
  engine (`coach/src/verify.rs`), with one correction round, then stripped.
  JSON output is grammar-constrained (WebLLM's xgrammar, llama.cpp's
  `json_schema`).
- **Chat.** Small models call tools badly, so their turns start with the
  engine lines and the game already fetched (`chat::wants_prefetch`). Tools
  travel through the prompt (`llm::prompted`) for models without native tool
  calls.

## How the browser model is wired

1. `/api/meta` offers `device_model` (from `CHESSGPT_DEVICE_MODEL`).
2. The user turns it on in Settings. `web/src/lib/llm/device.svelte.ts` loads
   it with WebLLM in a worker; the weights are cached by the browser. The tab
   then holds `GET /api/llm/device?model=…` open.
3. While a tab is connected, `AppState::provider()` returns a `relay::Relay`.
   Coach requests go to the tab as OpenAI-style bodies; the tab answers with
   `POST /api/llm/relay/{id}`. If the tab is gone or takes longer than 180 s,
   the rest of the job uses the server's model (`CHESSGPT_LLM_*`), if any.
4. Tokens served by browsers don't count against `CHESSGPT_LLM_DAILY_TOKENS`.

## Server CPU fallback

`deploy/oracle/docker-compose.yml` has a `llama` service (llama.cpp,
`unsloth/Qwen3.5-4B-GGUF:Q4_K_M`). To turn it on, in `~/chessgpt/.env`:

    COMPOSE_PROFILES=llm
    CHESSGPT_LLM_PROVIDER=openai_compatible
    CHESSGPT_LLM_BASE_URL=http://llama:8080/v1
    CHESSGPT_LLM_MODEL=coach
    ENGINE_WORKERS=1

On the Always Free A1 shape, first resize to 4 OCPU / 24 GB (the free
maximum): `oci compute instance update --instance-id <id> --shape-config
'{"ocpus": 4, "memoryInGBs": 24}'`. Expect a few tokens per second, one
request at a time. It's a fallback, not the main path.

## Measuring quality: the eval harness

    cargo xtask eval-set fixtures/eval/games.pgn   # once: key moments + Stockfish
    LAB_LLM_PROVIDER=openai_compatible LAB_LLM_BASE_URL=http://localhost:8080/v1 \
    LAB_LLM_MODEL=coach LAB_JUDGE_PROVIDER=openrouter LAB_JUDGE_MODEL=<free large model> \
    LAB_JUDGE_API_KEY=… cargo xtask eval --limit 100

`fixtures/eval/games.pgn` holds 240 Lichess games, 40 in each of six rating
bands. The report (`target/eval/<model>.json`) gives, overall and per band:

- JSON validity and first-try rate
- verifier-clean rate
- motif coverage: does the answer name the tactic the program found?
- an LLM-judge score (accuracy, insight, level, takeaway)

Gate for shipping a model: at least 98% verifier-clean, at least 99% valid
JSON, and a judge score of at least 90% of the teacher's.

## Making it better: distillation (all free)

1. **Moments.** Take Lichess games with `[%eval]` comments (CC0), so only the
   key moments need Stockfish:

       cargo run -p lab -- build-set --pgn-evals --per-band 3000 --out moments.jsonl games.pgn

2. **Teacher answers.** Use a large open-weight model on a free tier
   (OpenRouter `:free` models, Groq, Cerebras). Check that the model's license
   and the host's terms allow training on outputs. The run is resumable, so
   it can be spread across days of free quota:

       LAB_TEACHER_PROVIDER=openrouter LAB_TEACHER_MODEL=… LAB_TEACHER_API_KEY=… \
       LAB_JUDGE_PROVIDER=… cargo run -p lab -- datagen --set moments.jsonl --out sft.jsonl --per-minute 15

   Only answers that verify on the first try, name the moment's motif and
   grade at least 4/5 are kept (rejects go to `sft.jsonl.rejected`). The
   messages are byte-for-byte what the student model sees at inference.

3. **Fine-tune.** `tools/train/finetune.py` does QLoRA with Unsloth on a free
   Kaggle notebook (2x T4) or a GTX 1080 / RTX 4050.

4. **Export.** `tools/train/export.sh` writes GGUF Q4_K_M for llama.cpp and
   MLC q4f16_1 for WebLLM. The MLC build reuses WebLLM's prebuilt library, so
   there is no WebGPU compile. Upload both to Hugging Face and point
   `CHESSGPT_DEVICE_MODEL*` / `LLAMA_HF_MODEL` at them.

5. **Gate.** Run `cargo xtask eval` on the new model before switching the
   site to it.
