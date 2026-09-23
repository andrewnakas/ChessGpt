"""Fine-tune the coach model on distilled explanations (QLoRA, Unsloth).

Runs on a free Kaggle notebook (2x T4, 16 GB each) or any CUDA GPU with
>= 8 GB (a GTX 1080 works with --bf16 off, which is the default here).

    pip install unsloth trl datasets
    python finetune.py --data sft.jsonl --base unsloth/Qwen3.5-4B --out coach-4b

Input: the JSONL written by `chessgpt-lab datagen` ({"messages": [...]}).
Output: <out>/merged (16-bit weights for GGUF/MLC export) and <out>/lora.

Each example is split into the prompt exactly as the model sees it at
inference (chat template, thinking off) and the completion (the JSON answer),
and only the completion is trained on.
"""

import argparse
import json
import random

from datasets import Dataset
from trl import SFTConfig, SFTTrainer
from unsloth import FastLanguageModel


def load(path, tokenizer, holdout):
    rows = [json.loads(l) for l in open(path) if l.strip()]
    random.Random(0).shuffle(rows)
    out = []
    for r in rows:
        msgs = r["messages"]
        prompt = tokenizer.apply_chat_template(
            msgs[:-1], tokenize=False, add_generation_prompt=True, enable_thinking=False
        )
        completion = msgs[-1]["content"] + tokenizer.eos_token
        out.append({"prompt": prompt, "completion": completion, "id": r.get("id", "")})
    n = max(1, int(len(out) * holdout))
    return Dataset.from_list(out[n:]), Dataset.from_list(out[:n])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--base", default="unsloth/Qwen3.5-4B")
    ap.add_argument("--out", default="coach")
    ap.add_argument("--max-len", type=int, default=6144)
    ap.add_argument("--epochs", type=float, default=2)
    ap.add_argument("--lr", type=float, default=2e-4)
    ap.add_argument("--rank", type=int, default=16)
    ap.add_argument("--batch", type=int, default=1)
    ap.add_argument("--accum", type=int, default=16)
    ap.add_argument("--holdout", type=float, default=0.03)
    ap.add_argument("--bf16", action="store_true", help="Ampere or newer (not T4 / GTX 10xx)")
    a = ap.parse_args()

    model, tokenizer = FastLanguageModel.from_pretrained(a.base, max_seq_length=a.max_len, load_in_4bit=True)
    model = FastLanguageModel.get_peft_model(
        model,
        r=a.rank,
        lora_alpha=a.rank,
        lora_dropout=0,
        target_modules=["q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj"],
        use_gradient_checkpointing="unsloth",
        random_state=0,
    )
    train, evals = load(a.data, tokenizer, a.holdout)
    print(f"{len(train)} training examples, {len(evals)} held out")

    trainer = SFTTrainer(
        model=model,
        processing_class=tokenizer,
        train_dataset=train,
        eval_dataset=evals,
        args=SFTConfig(
            output_dir=f"{a.out}/checkpoints",
            max_length=a.max_len,
            completion_only_loss=True,
            per_device_train_batch_size=a.batch,
            gradient_accumulation_steps=a.accum,
            num_train_epochs=a.epochs,
            learning_rate=a.lr,
            lr_scheduler_type="cosine",
            warmup_ratio=0.03,
            logging_steps=10,
            eval_strategy="steps",
            eval_steps=100,
            save_steps=200,
            save_total_limit=2,
            bf16=a.bf16,
            fp16=not a.bf16,
            optim="adamw_8bit",
            seed=0,
            report_to="none",
        ),
    )
    trainer.train()
    model.save_pretrained(f"{a.out}/lora")
    tokenizer.save_pretrained(f"{a.out}/lora")
    model.save_pretrained_merged(f"{a.out}/merged", tokenizer, save_method="merged_16bit")
    print(f"saved {a.out}/merged")


if __name__ == "__main__":
    main()
