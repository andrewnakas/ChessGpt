"""Generate teacher answers for distillation on a free GPU box (Kaggle 2x T4).

Free API tiers allow only ~50 long answers a day, so the teacher runs here
instead: an open-weight model (Apache-2.0) under vLLM, batched.

    # on the laptop
    cargo run -p lab -- export-prompts --set moments.jsonl --out prompts.jsonl
    # on Kaggle (GPU T4 x2), with prompts.jsonl uploaded as a dataset
    pip install vllm
    python teacher_vllm.py --prompts prompts.jsonl --out responses.jsonl
    # back on the laptop: verify with the production checks, keep the good ones
    cargo run -p lab -- ingest --set moments.jsonl --responses responses.jsonl --out sft.jsonl

The script is resumable (ids already in --out are skipped), so it can span
several 12-hour Kaggle sessions. The teacher thinks before answering (better
chess reasoning); the thinking is dropped and only the JSON answer is kept.
"""

import argparse
import json
import os
import re

from vllm import LLM, SamplingParams


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--prompts", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--model", default="Qwen/Qwen3-32B-AWQ", help="an Apache-2.0 instruct model that fits the GPUs")
    ap.add_argument("--tp", type=int, default=2, help="tensor parallel size (GPUs)")
    ap.add_argument("--max-len", type=int, default=12288)
    ap.add_argument("--max-tokens", type=int, default=4096)
    ap.add_argument("--batch", type=int, default=64, help="prompts per vLLM call (then flushed to --out)")
    ap.add_argument("--no-think", action="store_true")
    a = ap.parse_args()

    done = set()
    if os.path.exists(a.out):
        done = {json.loads(l)["id"] for l in open(a.out) if l.strip()}
    rows = [json.loads(l) for l in open(a.prompts) if l.strip()]
    rows = [r for r in rows if r["id"] not in done]
    print(f"{len(rows)} prompts to do, {len(done)} done")

    llm = LLM(
        model=a.model,
        tensor_parallel_size=a.tp,
        dtype="half",  # T4 has no bf16
        max_model_len=a.max_len,
        gpu_memory_utilization=0.92,
        enable_prefix_caching=True,  # moments from one game share the system prompt
    )
    params = SamplingParams(temperature=0.6, top_p=0.95, max_tokens=a.max_tokens)
    think = re.compile(r"<think>.*?</think>", re.S)
    with open(a.out, "a") as out:
        for i in range(0, len(rows), a.batch):
            chunk = rows[i : i + a.batch]
            results = llm.chat(
                [r["messages"] for r in chunk],
                params,
                chat_template_kwargs={"enable_thinking": not a.no_think},
            )
            for r, res in zip(chunk, results):
                text = think.sub("", res.outputs[0].text).strip()
                out.write(json.dumps({"id": r["id"], "text": text, "model": a.model}) + "\n")
            out.flush()
            print(f"{min(i + a.batch, len(rows))}/{len(rows)}")


if __name__ == "__main__":
    main()
