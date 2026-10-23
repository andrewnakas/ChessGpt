# chessgpt: batch teacher answers for distillation (see tools/train/teacher_vllm.py).
import glob, subprocess, sys
subprocess.run([sys.executable, "-m", "pip", "install", "-q", "vllm==0.30.0"], check=True)
src = glob.glob("/kaggle/input/**/teacher_vllm.py", recursive=True)[0]
prompts = glob.glob("/kaggle/input/**/prompts.jsonl", recursive=True)[0]
subprocess.run([sys.executable, src, "--prompts", prompts, "--out", "/kaggle/working/responses.jsonl", "--judge"], check=True)
