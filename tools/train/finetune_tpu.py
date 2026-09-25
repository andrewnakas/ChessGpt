"""Fine-tune the coach model on a TPU (Kaggle TPU v3-8, free quota separate
from the GPU one): bf16 LoRA with PyTorch/XLA SPMD FSDPv2, the Hugging Face
recipe for TPUs. Produces the same LoRA adapter as finetune.py, so serving
and evaluation are unchanged.

    pip install transformers peft trl datasets   # torch_xla ships with the TPU image
    python finetune_tpu.py --data sft.jsonl --base Qwen/Qwen3-4B --out coach-tpu

Prompts are rendered exactly as at inference (chat template, thinking off)
and only the completion is trained on. Every batch is padded to --max-len:
XLA recompiles for each new shape, so fixed shapes keep it fast.
"""

import argparse
import json
import os
import random

import torch
import torch_xla.runtime as xr

xr.use_spmd()

from datasets import Dataset  # noqa: E402
from peft import LoraConfig  # noqa: E402
from transformers import AutoModelForCausalLM, AutoTokenizer  # noqa: E402
from trl import SFTConfig, SFTTrainer  # noqa: E402


def load(path, tokenizer):
    rows = [json.loads(l) for l in open(path) if l.strip()]
    random.Random(0).shuffle(rows)
    out = []
    for r in rows:
        msgs = r["messages"]
        prompt = tokenizer.apply_chat_template(
            msgs[:-1], tokenize=False, add_generation_prompt=True, enable_thinking=False
        )
        out.append({"prompt": prompt, "completion": msgs[-1]["content"] + tokenizer.eos_token})
    return Dataset.from_list(out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--base", default="Qwen/Qwen3-4B")
    ap.add_argument("--out", default="coach-tpu")
    ap.add_argument("--max-len", type=int, default=3072)
    ap.add_argument("--epochs", type=float, default=3)
    ap.add_argument("--lr", type=float, default=2e-4)
    ap.add_argument("--rank", type=int, default=16)
    ap.add_argument("--batch", type=int, default=8, help="global batch across the 8 cores")
    ap.add_argument("--accum", type=int, default=2)
    a = ap.parse_args()

    tokenizer = AutoTokenizer.from_pretrained(a.base)
    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token
    model = AutoModelForCausalLM.from_pretrained(a.base, torch_dtype=torch.bfloat16)
    layer_cls = type(model.model.layers[0]).__name__
    train = load(a.data, tokenizer)
    print(f"{len(train)} examples; wrapping {layer_cls}")

    trainer = SFTTrainer(
        model=model,
        processing_class=tokenizer,
        train_dataset=train,
        peft_config=LoraConfig(
            r=a.rank,
            lora_alpha=a.rank,
            lora_dropout=0.0,
            target_modules=["q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj"],
            task_type="CAUSAL_LM",
        ),
        args=SFTConfig(
            output_dir=f"{a.out}/checkpoints",
            max_length=a.max_len,
            completion_only_loss=True,
            pad_to_multiple_of=a.max_len,  # one static shape for XLA
            per_device_train_batch_size=a.batch,
            gradient_accumulation_steps=a.accum,
            num_train_epochs=a.epochs,
            learning_rate=a.lr,
            lr_scheduler_type="cosine",
            warmup_ratio=0.03,
            logging_steps=10,
            save_strategy="no",
            eval_strategy="no",
            bf16=True,
            optim="adafactor",
            dataloader_drop_last=True,
            report_to="none",
            seed=0,
            fsdp="full_shard",
            fsdp_config={
                "fsdp_transformer_layer_cls_to_wrap": [layer_cls],
                "xla": True,
                "xla_fsdp_v2": True,
                "xla_fsdp_grad_ckpt": True,
            },
        ),
    )
    trainer.train()
    os.makedirs(f"{a.out}/lora", exist_ok=True)
    trainer.save_model(f"{a.out}/lora")
    tokenizer.save_pretrained(f"{a.out}/lora")
    print(f"saved {a.out}/lora")


if __name__ == "__main__":
    main()
