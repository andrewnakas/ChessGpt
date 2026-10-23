# chessgpt: LoRA fine-tune on a TPU v3-8 (see tools/train/finetune_tpu.py).
import glob, subprocess, sys
subprocess.run([sys.executable, "-m", "pip", "install", "-q", "transformers", "peft", "safetensors"], check=True)
subprocess.run([sys.executable, "-c", "import torch, torch_xla, transformers, peft; print(torch.__version__, torch_xla.__version__, transformers.__version__, peft.__version__)"], check=True)
src = glob.glob("/kaggle/input/**/finetune_tpu.py", recursive=True)[0]
data = glob.glob("/kaggle/input/**/sft.jsonl", recursive=True)[0]
subprocess.run([sys.executable, src, "--data", data, "--out", "/kaggle/working/coach", "--epochs", "3"], check=True)
