#!/usr/bin/env bash
# Downloads Qwen3-Embedding-0.6B into a Hugging Face cache layout that lampl-embed loads offline. Usage: fetch-model.sh <dir>
set -euo pipefail
dir=${1:?usage: fetch-model.sh <dir>}
repo=Qwen/Qwen3-Embedding-0.6B
rev=$(curl -fsSL "https://huggingface.co/api/models/${repo}" | sed -n 's/.*"sha":"\([0-9a-f]\{40\}\)".*/\1/p' | head -1)
[ -n "$rev" ] || { echo "could not resolve ${repo} revision" >&2; exit 1; }
base="${dir}/models--${repo//\//--}"
mkdir -p "${base}/refs" "${base}/snapshots/${rev}"
printf '%s' "$rev" > "${base}/refs/main"
for f in config.json tokenizer.json model.safetensors; do
  curl -fL --retry 3 -C - -o "${base}/snapshots/${rev}/${f}" "https://huggingface.co/${repo}/resolve/${rev}/${f}"
done
echo "model ${repo}@${rev} in ${dir}"
