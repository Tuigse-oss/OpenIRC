#!/usr/bin/env bash
# Benchmarks of another commit, built and run here, so a comparison in
# scripts/check.sh is between two builds on the same machine at the same
# time. CI uses it with the base of a pull request (or the commit before a
# push to main) and then runs scripts/check.sh, which compares this run's
# benchmarks with the results saved here. Results from another machine, or
# from the same shared runner on another day, differ by far more than any
# change in the code (one identical run came out 40 to 126% slower).
#
#   scripts/bench-base.sh <commit> [<results file>]
#
# The results go to target/bench-baseline.txt unless another file is named.
# A commit that does not exist here (the first push, a scheduled run, a
# force-push) leaves no results, and check.sh then runs without comparing.
set -euo pipefail

commit=${1:?usage: scripts/bench-base.sh <commit> [<results file>]}
results=${2:-target/bench-baseline.txt}
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

case "$commit" in
    ""|0000000000000000000000000000000000000000)
        echo "no base commit: no results to compare with"
        exit 0
        ;;
esac
if ! git cat-file -e "$commit^{commit}" 2>/dev/null; then
    echo "base commit $commit is not here: no results to compare with"
    exit 0
fi

capped() {
    local limit="${TEST_MEMORY_MAX:-2G}"
    if systemd-run --user --scope --quiet -p MemoryMax=64M true >/dev/null 2>&1; then
        systemd-run --user --scope --quiet -p MemoryMax="$limit" -p MemorySwapMax=0 "$@"
    else
        local bytes
        bytes=$(numfmt --from=iec "$limit")
        ( ulimit -v $(( bytes / 1024 * 4 )); "$@" )
    fi
}

base=$(mktemp -d)
config=$(mktemp -d)
trap 'git worktree remove --force "$base" >/dev/null 2>&1 || true; rm -rf "$base" "$config"' EXIT
git worktree add --detach "$base" "$commit" >/dev/null
echo "benchmarks of $(git -C "$base" log -1 --format='%h %s')"
# Its own target directory: built in the shared one, the base's binary
# would stand in for this version's, which cargo then sees as up to date.
export CARGO_TARGET_DIR="$root/target/base"
(cd "$base" && cargo build --release --features bench --example bench)
mkdir -p "$(dirname "$results")"
capped env XDG_CONFIG_HOME="$config" "$CARGO_TARGET_DIR/release/examples/bench" --save "$results"
echo "saved as the results to compare with: $results"
