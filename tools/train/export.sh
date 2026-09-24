#!/usr/bin/env bash
# Export a merged fine-tune for the two places it runs:
#   GGUF Q4_K_M for llama.cpp (the Oracle fallback)
#   MLC q4f16_1 for WebLLM (the browser), reusing WebLLM's prebuilt model
#   library for the same architecture, so no WebGPU compile is needed.
#
#   ./export.sh coach-4b/merged coach-4b Qwen3-4B
#
# Needs: a llama.cpp checkout (LLAMA_CPP, built) and `pip install mlc-llm`
# (nightly wheel from https://mlc.ai/package/). CPU-only is fine.
set -euo pipefail
merged=$1 name=$2 base=${3:-Qwen3-4B}
out=${OUT:-export}
LLAMA_CPP=${LLAMA_CPP:-$HOME/llama.cpp}
mkdir -p "$out"

python "$LLAMA_CPP/convert_hf_to_gguf.py" "$merged" --outtype f16 --outfile "$out/$name-f16.gguf"
"$LLAMA_CPP/build/bin/llama-quantize" "$out/$name-f16.gguf" "$out/$name-Q4_K_M.gguf" Q4_K_M
rm "$out/$name-f16.gguf"

# Same chat template as WebLLM's prebuilt build of the base model.
conv=$(curl -sL "https://huggingface.co/mlc-ai/$base-q4f16_1-MLC/resolve/main/mlc-chat-config.json" \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["conv_template"]["name"])')
mlc_llm convert_weight "$merged" --quantization q4f16_1 -o "$out/$name-q4f16_1-MLC"
mlc_llm gen_config "$merged" --quantization q4f16_1 --conv-template "$conv" \
  --context-window-size 8192 -o "$out/$name-q4f16_1-MLC"

cat <<MSG
Done. Next:
  1. Upload $out/$name-q4f16_1-MLC to a Hugging Face model repo, and
     $out/$name-Q4_K_M.gguf to another (or the same) repo.
  2. On the server (.env):
       CHESSGPT_DEVICE_MODEL=$name-q4f16_1-MLC
       CHESSGPT_DEVICE_MODEL_URL=https://huggingface.co/<you>/$name-q4f16_1-MLC
       CHESSGPT_DEVICE_MODEL_LIB=https://raw.githubusercontent.com/mlc-ai/binary-mlc-llm-libs/main/web-llm-models/v0_2_84/base/$base-q4f16_1_cs1k-webgpu.wasm
       (the model_lib WebLLM @0.2.85 uses for $base-q4f16_1-MLC; check web/node_modules/@mlc-ai/web-llm)
       CHESSGPT_DEVICE_MODEL_MB=$(du -sm "$out/$name-q4f16_1-MLC" | cut -f1)
       LLAMA_HF_MODEL=<you>/<gguf repo>:Q4_K_M
  3. Run the eval gate first: cargo xtask eval against the new model.
MSG
