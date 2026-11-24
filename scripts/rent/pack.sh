#!/usr/bin/env bash
# On the rented machine: pack what a run made, to download and unpack in the repo folder on the PC (lab #38).
#     bash scripts/rent/pack.sh            -> ~/balatro-meta-results-<date>.tgz
# Holds models/policy/ (checkpoints, train logs, evaluations: they stay off git) and results/ (reports, preflight).
set -euo pipefail
cd "$(dirname "$0")/../.."
out="$HOME/balatro-meta-results-$(date -u +%Y%m%d-%H%M).tgz"
tar czf "$out" models/policy results
echo "Packed: $out ($(du -h "$out" | cut -f1))"
echo "On the PC: copy it into the repo folder and run  tar xzf $(basename "$out")"
