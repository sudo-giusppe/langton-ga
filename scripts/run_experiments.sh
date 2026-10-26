#!/usr/bin/env bash
# Reproduce every experiment reported in the paper.
#
# Usage:  ./scripts/run_experiments.sh
# Optional environment variables:
#   SEEDS="1 2 3 4 5"   BIN=./target/release/langton-ga   OUT=results
#
# The script overwrites results/, so run it from the repository root.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN="${BIN:-./target/release/langton-ga}"
OUT="${OUT:-results}"
SEEDS="${SEEDS:-1 2 3 4 5}"
NSEEDS=$(wc -w <<< "$SEEDS")
WEAVER="0.20,0.20,0.50,0.10"
BALANCED="0.50,0.25,0.15,0.10"
STEPS=10000
GENS=80

if [[ ! -x "$BIN" ]]; then
  echo "error: $BIN not found; run 'cargo build --release' first" >&2
  exit 1
fi

rm -rf "$OUT"
mkdir -p "$OUT"/{baseline,single,distributed,portfolio,gallery}

echo "== 1/6 baseline: classic Langton ant =="
"$BIN" analyze --rule classic --steps 120000 --out "$OUT/baseline/classic_metrics.csv"
"$BIN" sim --rule classic --steps 120000 --trace "$OUT/baseline/trace.csv" --trace-every 50 \
  --render "$OUT/baseline/classic_120k.png" --scale 3 --max-cells 500 > "$OUT/baseline/classic_120k.txt"
"$BIN" render --rule classic --steps 12000 --out "$OUT/baseline/classic_12k.png" --scale 4 --max-cells 300

echo "== 2/6 single-island convergence (${NSEEDS} seeds x 2 profiles) =="
for profile in weaver balanced; do
  case "$profile" in
    weaver) W="$WEAVER" ;;
    *) W="$BALANCED" ;;
  esac
  for seed in $SEEDS; do
    "$BIN" evolve --seed "$seed" --pop 64 --gens "$GENS" --steps "$STEPS" \
      --states 2 --colors 3 --threads 4 --weights "$W" \
      --out "$OUT/single/${profile}_${seed}.csv" \
      --best-out "$OUT/single/${profile}_${seed}_best.txt" \
      --render "$OUT/single/${profile}_${seed}.png" --render-steps 30000 --scale 2 \
      > "$OUT/single/${profile}_${seed}.log"
    echo "  $profile seed $seed -> $(grep '^fitness=' "$OUT/single/${profile}_${seed}_best.txt")"
  done
done

echo "== 3/6 sequential baseline (1 population, 1 thread, ${NSEEDS} seeds) =="
# Seeds run one after another: wall time is a reported metric, so overlapping
# runs would contend for cores and inflate the sequential baseline.
for seed in $SEEDS; do
  "$BIN" evolve --seed "$seed" --pop 256 --gens "$GENS" --steps "$STEPS" \
    --states 2 --colors 3 --threads 1 --weights "$WEAVER" \
    --out "$OUT/distributed/seq_${seed}.csv" \
    --best-out "$OUT/distributed/seq_${seed}_best.txt" \
    > "$OUT/distributed/seq_${seed}.log"
  echo "  seq seed $seed -> $(grep '^fitness=' "$OUT/distributed/seq_${seed}_best.txt")"
done

echo "== 4/6 panmictic (1 population, 4 threads, ${NSEEDS} seeds) =="
for seed in $SEEDS; do
  "$BIN" evolve --seed "$seed" --pop 256 --gens "$GENS" --steps "$STEPS" \
    --states 2 --colors 3 --threads 4 --weights "$WEAVER" \
    --out "$OUT/distributed/pan_${seed}.csv" \
    --best-out "$OUT/distributed/pan_${seed}_best.txt" \
    > "$OUT/distributed/pan_${seed}.log"
  echo "  pan seed $seed -> $(grep '^fitness=' "$OUT/distributed/pan_${seed}_best.txt")"
done

echo "== 5/6 island model (4 islands x 64, migration 0/5/20, ${NSEEDS} seeds) =="
port=8300
for mode in m0 m5 m20; do
  case "$mode" in
    m0) INTERVAL=0 ;;
    m5) INTERVAL=5 ;;
    m20) INTERVAL=20 ;;
  esac
  for seed in $SEEDS; do
    dir="$OUT/distributed/${mode}_${seed}"
    "$BIN" demo --islands 4 --gens "$GENS" --pop 64 --steps "$STEPS" --seed "$seed" \
      --states 2 --colors 3 --weights "$WEAVER" \
      --migration-interval "$INTERVAL" --migrants 2 \
      --base-port "$port" --out-dir "$dir" --timeout-secs 1200 \
      --render-scale 2 > "$OUT/distributed/${mode}_${seed}.log" 2>&1
    echo "  $mode seed $seed -> $(awk -F, 'NR>1{if($2>m)m=$2}END{print m}' "$dir/finals.csv")"
    port=$((port + 10))
    if (( port > 8800 )); then port=8300; fi
  done
done

echo "== 6/6 heterogeneous portfolio (no migration) =="
"$BIN" demo --islands 4 --gens 60 --pop 64 --steps "$STEPS" --seed 1 \
  --states 2 --colors 3 --profiles mover,builder,weaver,balanced \
  --migration-interval 0 --base-port 8900 --out-dir "$OUT/portfolio" \
  --timeout-secs 600 --render-scale 4 --render-steps 40000 \
  > "$OUT/portfolio/run.log" 2>&1

echo "== extra: gallery analyses and high-resolution renders =="
# The four portfolio specialists.
for i in 1 2 3 4; do
  hex=$(grep '^hex=' "$OUT/portfolio/island${i}_best.txt" | cut -d= -f2)
  "$BIN" analyze --states 2 --colors 3 --steps 60000 --rule "$hex" \
    --weights "$WEAVER" --out "$OUT/gallery/portfolio_${i}.csv"
done
"$BIN" render --states 2 --colors 3 --steps 40000 \
  --rule "$(grep '^hex=' "$OUT/portfolio/island3_best.txt" | cut -d= -f2)" \
  --out "$OUT/gallery/weaver.png" --scale 6 --max-cells 300
"$BIN" render --states 2 --colors 3 --steps 60000 \
  --rule "$(grep '^hex=' "$OUT/portfolio/island3_best.txt" | cut -d= -f2)" \
  --out "$OUT/gallery/weaver_60k.png" --scale 5 --max-cells 300
"$BIN" render --states 2 --colors 3 --steps 30000 \
  --rule "$(grep '^hex=' "$OUT/portfolio/island4_best.txt" | cut -d= -f2)" \
  --out "$OUT/gallery/conveyor.png" --scale 4 --max-cells 220 --pad 40
"$BIN" render --states 2 --colors 3 --steps 30000 \
  --rule "$(grep '^hex=' "$OUT/portfolio/island1_best.txt" | cut -d= -f2)" \
  --out "$OUT/gallery/mover.png" --scale 2 --max-cells 400 --pad 20
"$BIN" render --states 2 --colors 3 --steps 30000 \
  --rule "$(grep '^hex=' "$OUT/portfolio/island2_best.txt" | cut -d= -f2)" \
  --out "$OUT/gallery/builder.png" --scale 4 --max-cells 220 --pad 40
# Balanced single-island winner (a conveyor belt discovered from scratch).
# Trajectories for the gallery figure.
for pair in "1 mover" "2 builder" "3 weaver" "4 balanced"; do
  set -- $pair
  hex=$(grep '^hex=' "$OUT/portfolio/island$1_best.txt" | cut -d= -f2)
  "$BIN" sim --states 2 --colors 3 --steps 2000 --trace-every 1 \
    --rule "$hex" --trace "$OUT/gallery/traj_$2.csv" > /dev/null
done
hex=$(grep '^hex=' "$OUT/single/balanced_1_best.txt" | cut -d= -f2)
"$BIN" analyze --states 2 --colors 3 --steps 60000 --rule "$hex" \
  --weights "$BALANCED" --out "$OUT/gallery/balanced_1.csv"

echo "== extra: simulator benchmark =="
"$BIN" bench --iters 100 --steps 20000 > "$OUT/bench.txt"

echo "done. results in $OUT/"
