# Kaggle jobs

Free Kaggle compute for the coach model. Push a job with
`kaggle kernels push -p tools/train/kaggle/<job>` (add `--accelerator NvidiaTeslaT4`
for the teachers, `--accelerator Tpu1VmV38` for `train-tpu`).

| job | what it does | input |
|---|---|---|
| `teacher`, `teacher-b` | teacher answers for distillation (vLLM, 2×T4); two run at once | datasets `chessgpt-teacher-prompts`, `-b`: `prompts.jsonl` from `chessgpt-lab export-prompts` plus `tools/train/teacher_vllm.py` |
| `train-tpu` | LoRA fine-tune of Qwen3-4B on a TPU v3-8 (about 1 h) | dataset `chessgpt-sft`: `sft.jsonl` plus `tools/train/finetune_tpu.py` |
| `export-mlc` | bakes an adapter into WebLLM's Qwen3-4B q4f16_1 build (checked byte-for-byte) | output of `train-tpu` |
| `moments` | builds a moment pool from a month of Lichess games with evals | none (downloads) |

Data lives in the private Hugging Face dataset `nakas/chessgpt-data`, with a working copy in
`~/chessgpt-data` on the Oracle server; nothing should live only on a laptop.
