#!/usr/bin/env bash
# Reproduce the M0 spike. Requires: brew install llama.cpp
set -euo pipefail
cd "$(dirname "$0")"

mkdir -p models

fetch() {
  local url="$1" out="models/$2"
  [ -f "$out" ] || curl -fL --retry 3 --progress-bar -o "$out" "$url"
}

fetch "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf" \
      "qwen2.5-0.5b-instruct-q4_k_m.gguf"
fetch "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf" \
      "qwen2.5-3b-instruct-q4_k_m.gguf"
fetch "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q8_0.gguf" \
      "qwen2.5-3b-instruct-q8_0.gguf"

python3 fingerprint.py
python3 spike.py
