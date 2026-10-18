#!/usr/bin/env bash
# Cross-tool cold-extraction wall-clock comparison via hyperfine — proper
# warmup runs, statistical analysis (mean/stddev/outlier detection), not a
# hand-rolled timing loop. Each command is a fresh `tsx <script>` process
# covering the whole fixtures/ corpus, matching how each tool would actually
# be invoked once per build/CI run.
set -euo pipefail
cd "$(dirname "$0")"

if ! command -v hyperfine >/dev/null 2>&1; then
  echo "hyperfine not found — install via 'brew install hyperfine' (or see https://github.com/sharkdp/hyperfine)" >&2
  exit 1
fi

hyperfine \
  --warmup 2 \
  --min-runs 10 \
  --export-markdown bench-cold.md \
  --export-json bench-cold.json \
  -n "react-docgen" "pnpm exec tsx src/run-react-docgen.ts" \
  -n "react-docgen-typescript" "pnpm exec tsx src/run-react-docgen-typescript.ts" \
  -n "caliper-docgen" "pnpm exec tsx src/run-ours.ts"

echo
echo "Results written to apps/validate/bench-cold.md and apps/validate/bench-cold.json"
