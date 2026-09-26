"""Fine-tune the coach model on a TPU (Kaggle TPU v3-8, free quota separate
from the GPU one): bf16 LoRA with PyTorch/XLA SPMD. Every base weight matrix
is sharded across the 8 cores (FSDP style) and each batch is split across
them; only the LoRA parameters train (in float32). Produces the same adapter
as finetune.py, so serving and evaluation are unchanged.

    pip install transformers peft datasets   # torch_xla ships with the TPU image
    python finetune_tpu.py --data sft.jsonl --base Qwen/Qwen3-4B --out coach-tpu

Prompts are rendered exactly as at inference (chat template, thinking off)
and only the completion is trained on. Every example is padded to --max-len:
XLA recompiles for each new shape, so one static shape keeps it fast.

A hand-written loop rather than transformers' Trainer: its XLA FSDPv2 path
builds the LR scheduler before the optimizer exists (transformers 5.12).
"""

import argparse
import json
import math
import os
import random

import numpy as np
import torch
import torch_xla
import torch_xla.core.xla_model as xm
import torch_xla.distributed.spmd as xs
import torch_xla.runtime as xr
from torch_xla.utils.checkpoint import checkpoint as xla_checkpoint

xr.use_spmd()

from peft import LoraConfig, get_peft_model, get_peft_model_state_dict  # noqa: E402
from safetensors.torch import save_file  # noqa: E402
from transformers import AutoModelForCausalLM, AutoTokenizer, get_cosine_schedule_with_warmup  # noqa: E402


def encode(path, tokenizer, max_len):
    rows = [json.loads(l) for l in open(path) if l.strip()]
    out, skipped = [], 0
    for r in rows:
        msgs = r["messages"]
        prompt = tokenizer.apply_chat_template(
            msgs[:-1], tokenize=False, add_generation_prompt=True, enable_thinking=False
        )
        p = tokenizer(prompt, add_special_tokens=False)["input_ids"]
        c = tokenizer(msgs[-1]["content"] + tokenizer.eos_token, add_special_tokens=False)["input_ids"]
        if len(p) + len(c) > max_len:
            skipped += 1
            continue
        pad = max_len - len(p) - len(c)
        out.append(
            (
                p + c + [tokenizer.pad_token_id] * pad,
                [1] * (len(p) + len(c)) + [0] * pad,
                [-100] * len(p) + c + [-100] * pad,  # loss on the completion only
            )
        )
    print(f"{len(out)} examples ({skipped} longer than {max_len} skipped)")
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--base", default="Qwen/Qwen3-4B")
    ap.add_argument("--out", default="coach-tpu")
    ap.add_argument("--max-len", type=int, default=3072)
    ap.add_argument("--epochs", type=float, default=3)
    ap.add_argument("--lr", type=float, default=2e-4)
    ap.add_argument("--rank", type=int, default=16)
    ap.add_argument("--batch", type=int, default=8, help="global batch (one sequence per core)")
    ap.add_argument("--accum", type=int, default=2)
    a = ap.parse_args()

    n_dev = xr.global_runtime_device_count()
    mesh = xs.Mesh(np.arange(n_dev), (n_dev, 1), ("fsdp", "model"))
    device = torch_xla.device()

    tokenizer = AutoTokenizer.from_pretrained(a.base)
    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token
    data = encode(a.data, tokenizer, a.max_len)

    model = AutoModelForCausalLM.from_pretrained(a.base, torch_dtype=torch.bfloat16)
    # torch.utils.checkpoint looks up a `torch.xla` device module that torch_xla
    # doesn't provide; use torch_xla's own checkpoint for the decoder layers.
    model.gradient_checkpointing_enable()
    model._set_gradient_checkpointing(enable=True, gradient_checkpointing_func=xla_checkpoint)
    model.enable_input_require_grads()
    model = get_peft_model(
        model,
        LoraConfig(
            r=a.rank,
            lora_alpha=a.rank,
            lora_dropout=0.0,
            target_modules=["q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj"],
            task_type="CAUSAL_LM",
        ),
    )
    for p in model.parameters():
        if p.requires_grad:
            p.data = p.data.float()
    model.print_trainable_parameters()
    model.to(device)
    # FSDP-style: shard the big frozen matrices along their first dim.
    for name, p in model.named_parameters():
        if not p.requires_grad and p.dim() == 2 and p.shape[0] % n_dev == 0:
            xs.mark_sharding(p, mesh, ("fsdp", None))

    trainable = [p for p in model.parameters() if p.requires_grad]
    opt = torch.optim.AdamW(trainable, lr=a.lr, weight_decay=0.0)
    steps_per_epoch = len(data) // a.batch // a.accum
    total = max(1, int(steps_per_epoch * a.epochs))
    sched = get_cosine_schedule_with_warmup(opt, max(1, int(total * 0.03)), total)
    print(f"{n_dev} devices, {total} optimizer steps")

    model.train()
    step, micro = 0, 0
    rng = random.Random(0)
    while step < total:
        order = list(range(len(data)))
        rng.shuffle(order)
        for i in range(0, len(order) - a.batch + 1, a.batch):
            ids, mask, labels = (torch.tensor([data[j][k] for j in order[i : i + a.batch]]) for k in range(3))
            ids, mask, labels = ids.to(device), mask.to(device), labels.to(device)
            for t in (ids, mask, labels):
                xs.mark_sharding(t, mesh, ("fsdp", None))
            loss = model(input_ids=ids, attention_mask=mask, labels=labels).loss / a.accum
            loss.backward()
            micro += 1
            if micro % a.accum == 0:
                torch.nn.utils.clip_grad_norm_(trainable, 1.0)
                opt.step()
                sched.step()
                opt.zero_grad()
                step += 1
                xm.mark_step()
                if step % 10 == 0 or step == 1:
                    print(f"step {step}/{total} loss {loss.item() * a.accum:.4f} lr {sched.get_last_lr()[0]:.2e}", flush=True)
                if step >= total:
                    break
            else:
                xm.mark_step()

    lora_dir = f"{a.out}/lora"
    os.makedirs(lora_dir, exist_ok=True)
    state = {k: v.detach().to("cpu").to(torch.bfloat16).contiguous() for k, v in get_peft_model_state_dict(model).items()}
    save_file(state, f"{lora_dir}/adapter_model.safetensors")
    model.peft_config["default"].save_pretrained(lora_dir)
    tokenizer.save_pretrained(lora_dir)
    print(f"saved {lora_dir} ({len(state)} tensors)")


if __name__ == "__main__":
    main()
