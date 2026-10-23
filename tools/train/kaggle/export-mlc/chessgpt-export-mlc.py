# chessgpt: bake the coach LoRA into WebLLM's own Qwen3-4B q4f16_1 build.
# The LoRA only touches the attention and MLP projections, so we keep the
# official shards and rewrite just those tensors, quantized here with numpy
# (MLC's q4f16_1: symmetric int4, groups of 32 along K, 8 per uint32, fp16
# scale). The quantizer is first checked byte-for-byte against the official
# shards on the unmodified base model.
import glob, hashlib, json, os, subprocess, sys
subprocess.run([sys.executable, "-m", "pip", "install", "-q", "peft", "transformers", "accelerate", "safetensors", "huggingface_hub"], check=True)
subprocess.run([sys.executable, "-m", "pip", "uninstall", "-y", "-q", "torchao"], check=False)
import numpy as np, torch
from huggingface_hub import snapshot_download
from peft import PeftModel
from transformers import AutoModelForCausalLM

REF = snapshot_download("mlc-ai/Qwen3-4B-q4f16_1-MLC", local_dir="/kaggle/tmp/ref")
cache = json.load(open(f"{REF}/ndarray-cache.json"))
where = {p["name"]: (rec, p) for rec in cache["records"] for p in rec["records"]}

def quantize(w: np.ndarray):
    """w: [N, K] float16 -> (q_weight uint32 [N, K/8], q_scale float16 [N, K/32]).
    Matches MLC's q4f16_1 exactly (fp16 arithmetic, scale = max|w| * fp16(1/7),
    q = floor(fp16(w / scale) + 7 + 0.5))."""
    n, k = w.shape
    g = w.reshape(n, k // 32, 32).astype(np.float16)
    scale = (np.abs(g).max(axis=2) * (np.float16(1) / np.float16(7))).astype(np.float16)
    with np.errstate(divide="ignore", invalid="ignore"):
        x = ((g / scale[..., None]).astype(np.float16) + np.float16(7)).astype(np.float16)
    q = np.floor(np.nan_to_num(x.astype(np.float32), nan=7.0) + 0.5)
    q = np.clip(q, 0, 14).astype(np.uint32).reshape(n, k // 8, 8)
    packed = np.zeros((n, k // 8), dtype=np.uint32)
    for i in range(8):
        packed |= q[:, :, i] << np.uint32(4 * i)
    return packed, scale

def fused(sd, i):
    a = f"model.layers.{i}."
    t = lambda n: sd[a + n].to(torch.float16).numpy()
    return {
        a + "self_attn.c_attn": np.concatenate([t("self_attn.q_proj.weight"), t("self_attn.k_proj.weight"), t("self_attn.v_proj.weight")]),
        a + "self_attn.o_proj": t("self_attn.o_proj.weight"),
        a + "mlp.gate_up_proj": np.concatenate([t("mlp.gate_proj.weight"), t("mlp.up_proj.weight")]),
        a + "mlp.down_proj": t("mlp.down_proj.weight"),
    }

def ref_bytes(name):
    rec, p = where[name]
    with open(f"{REF}/{rec['dataPath']}", "rb") as f:
        f.seek(p["byteOffset"])
        return f.read(p["nbytes"])

base = AutoModelForCausalLM.from_pretrained("Qwen/Qwen3-4B", torch_dtype=torch.bfloat16, low_cpu_mem_usage=True)
sd = base.state_dict()
total = same = 0
for name, w in fused(sd, 0).items():
    qw, qs = quantize(w)
    for suffix, arr in [("q_weight", qw), ("q_scale", qs)]:
        ref = np.frombuffer(ref_bytes(f"{name}.{suffix}"), dtype=arr.dtype).reshape(arr.shape)
        total += arr.size
        same += int((ref == arr).sum())
print("byte check on layer 0:", same / total, flush=True)
assert same == total, "quantizer does not reproduce MLC's output"

adapter = os.path.dirname(glob.glob("/kaggle/input/**/adapter_config.json", recursive=True)[0])
print("adapter:", adapter, flush=True)
model = PeftModel.from_pretrained(base, adapter).merge_and_unload()
sd = model.state_dict()
out = "/kaggle/working/shards"
os.makedirs(out, exist_ok=True)
patched = {}
n_layers = model.config.num_hidden_layers
for i in range(n_layers):
    for name, w in fused(sd, i).items():
        qw, qs = quantize(w)
        for suffix, arr in [("q_weight", qw), ("q_scale", qs)]:
            rec, p = where[f"{name}.{suffix}"]
            assert list(arr.shape) == p["shape"] and arr.nbytes == p["nbytes"], (name, arr.shape, p)
            patched.setdefault(rec["dataPath"], []).append((p["byteOffset"], arr.tobytes()))
for path, edits in patched.items():
    data = bytearray(open(f"{REF}/{path}", "rb").read())
    for off, b in edits:
        data[off:off + len(b)] = b
    open(f"{out}/{path}", "wb").write(data)
    for rec in cache["records"]:
        if rec["dataPath"] == path:
            rec["md5sum"] = hashlib.md5(data).hexdigest()
json.dump(cache, open(f"{out}/ndarray-cache.json", "w"))
print("patched shards:", len(patched))
