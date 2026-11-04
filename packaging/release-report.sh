#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
PROJECT_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
OUTPUT_ROOT="$PROJECT_ROOT/reports"
REPORT_PATH="$OUTPUT_ROOT/release-evidence.md"
BENCH_PATH="$OUTPUT_ROOT/benchmark.json"
OCEL_LIFECYCLE_PATH="$OUTPUT_ROOT/benchmark-ocel-lifecycle.json"
OCEL_INTERACTION_PATH="$OUTPUT_ROOT/benchmark-ocel-interaction.json"
TEST_PATH="$OUTPUT_ROOT/tests.txt"
EVENTS=${BENCHMARK_EVENTS:-1000000}
OCEL_EVENTS=${OCEL_BENCHMARK_EVENTS:-100000}

mkdir -p "$OUTPUT_ROOT"

cd "$PROJECT_ROOT"
cargo test --workspace >"$TEST_PATH" 2>&1
cargo run --release --quiet -p pm-bench -- --events "$EVENTS" --json >"$BENCH_PATH"
cargo run --release --quiet -p pm-bench -- --ocel --ocel-shape lifecycle --events "$OCEL_EVENTS" --json >"$OCEL_LIFECYCLE_PATH"
cargo run --release --quiet -p pm-bench -- --ocel --ocel-shape interaction --events "$OCEL_EVENTS" --json >"$OCEL_INTERACTION_PATH"

OS_NAME=$(uname -s)
OS_RELEASE=$(uname -r)
ARCH=$(uname -m)
CPU="unknown"
MEMORY="unknown"
if [ "$OS_NAME" = "Darwin" ]; then
  CPU=$(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo unknown)
  MEMORY_BYTES=$(sysctl -n hw.memsize 2>/dev/null || echo 0)
  if [ "$MEMORY_BYTES" -gt 0 ] 2>/dev/null; then
    MEMORY="$((MEMORY_BYTES / 1073741824)) GiB"
  fi
elif [ -r /proc/cpuinfo ]; then
  CPU=$(sed -n 's/^model name[[:space:]]*: //p' /proc/cpuinfo | head -n 1)
  if [ -r /proc/meminfo ]; then
    MEMORY_KIB=$(sed -n 's/^MemTotal:[[:space:]]*\([0-9][0-9]*\).*/\1/p' /proc/meminfo)
    MEMORY="$((MEMORY_KIB / 1048576)) GiB"
  fi
fi

RUST_VERSION=$(rustc --version)
COMMIT=$(git rev-parse HEAD)
RUN_DATE=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
TOTAL_PASSED=$(sed -n 's/^test result: ok\. \([0-9][0-9]*\) passed;.*/\1/p' "$TEST_PATH" | awk '{ total += $1 } END { print total + 0 }')

{
  echo "# Release evidence"
  echo
  echo "Generated from commit \`$COMMIT\` at $RUN_DATE."
  echo
  echo "## Environment"
  echo
  echo "- Operating system: $OS_NAME $OS_RELEASE"
  echo "- Architecture: $ARCH"
  echo "- Processor: $CPU"
  echo "- Physical memory: $MEMORY"
  echo "- Toolchain: $RUST_VERSION"
  echo
  echo "## Correctness"
  echo
  echo "The complete workspace test suite passed, including canonical ingestion fixtures,"
  echo "deterministic engine tests, generated-model discovery oracles, replay oracles,"
  echo "privacy-boundary tests, and real loopback browser-service integration tests."
  echo
  echo "Recorded result: **$TOTAL_PASSED tests passed; no failures.**"
  echo
  echo "## Performance"
  echo
  echo "The benchmark measures \`pm_query::analyze_filtered\`, the same end-to-end path"
  echo "used by the product. Generation is reported separately and is not product time."
  echo
  printf '%s\n' '```json'
  cat "$BENCH_PATH"
  printf '%s\n' '```'
  echo
  echo "### Object-centric product path"
  echo
  echo "Both shapes measure \`pm_query::analyze_ocel\` from an official-schema OCEL 2.0"
  echo "SQLite exchange file. Lifecycle stresses long native histories; interaction adds"
  echo "five event-object relationships and four object-object relationships per process instance."
  echo
  echo "#### Lifecycle shape"
  echo
  printf '%s\n' '```json'
  cat "$OCEL_LIFECYCLE_PATH"
  printf '%s\n' '```'
  echo
  echo "#### Interaction shape"
  echo
  printf '%s\n' '```json'
  cat "$OCEL_INTERACTION_PATH"
  printf '%s\n' '```'
  echo
  echo "## Interpretation limits"
  echo
  echo "This is one run on one machine, not a universal performance claim. Runtime and"
  echo "memory depend on event count, case count, activity cardinality, variant diversity,"
  echo "source format, object count, relationship density, and machine state. Peak Rust heap"
  echo "does not include SQLite/native allocations. The raw test and benchmark outputs are retained"
  echo "beside this report."
} >"$REPORT_PATH"

echo "$REPORT_PATH"
