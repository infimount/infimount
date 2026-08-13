#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

TIME=(time)
if [[ -x /usr/bin/time ]]; then
  TIME=(/usr/bin/time -v)
fi

# Keep compiler memory/time out of the measured benchmark process.
cargo test -p infimount_core --release --no-run >/dev/null

run_metric() {
  local test_name=$1
  echo "== PR09 benchmark: $test_name =="
  "${TIME[@]}" cargo test -p infimount_core "$test_name" --release -- --ignored --nocapture
}

# Reproducible local listing scenarios. Test output records entry counts, first-page
# latency, and total elapsed time; GNU time adds peak RSS and wall-clock metrics.
run_metric benchmark_paginated_listing
run_metric benchmark_recursive_listing_100k

# This opt-in scenario writes roughly 3 GiB of temporary local data while proving
# a 1 GiB upload/download round trip remains streaming and byte-count verified.
if [[ "${PR09_FULL:-0}" == "1" ]]; then
  run_metric benchmark_streaming_one_gib_local_round_trip
else
  echo "Skipping 1 GiB local round trip (set PR09_FULL=1 to run)."
fi

cat <<'EOF'
Simulator-only evidence (S3 request counts, 200 ms injected latency, and remote
cancellation latency) must be recorded by release CI with its simulator/proxy
endpoints. This local harness intentionally does not fabricate remote metrics.
EOF
