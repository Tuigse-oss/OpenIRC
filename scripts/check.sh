#!/usr/bin/env bash
# Every check a build of OpenIRC must pass, in one command:
#
#   scripts/check.sh                 everything
#   scripts/check.sh --quick         skip the Windows type-check and benchmarks
#   scripts/check.sh --save-bench    keep this run's benchmarks as the new
#                                    baseline instead of comparing them with
#                                    the old one (after a hardware change or
#                                    an intended slowdown); every other check
#                                    still runs, and so do the benchmarks
#
# The checks, in order: formatting, clippy (Linux, with and without the
# bench feature, then the Windows target when it is installed), the tests,
# RustSec through cargo audit and cargo deny, and the benchmarks, which fail
# the run when one is slower than the baseline by more than BENCH_TOLERANCE
# percent (default 50). The comparison fails, too, when the baseline cannot
# be read or does not list exactly the benchmarks that ran. Results go to
# target/ in the source tree, whatever CARGO_TARGET_DIR says.
#
# Offline machines: set CARGO_NET_OFFLINE=true and the advisory databases
# already on disk are used instead of fetching them.
#
# The tests and benchmarks run with a memory cap, so a test that runs away
# is stopped by itself instead of filling the machine and taking other
# programs down with it. Where systemd can make a scope, that scope may use
# at most TEST_MEMORY_MAX of memory in all (default 2G; the tests need
# about 250 MB) and no swap. Elsewhere each process may reserve at most
# four times TEST_MEMORY_MAX of address space (ulimit -v): a looser, per
# process limit, since programs reserve more than they use.
set -euo pipefail
cd "$(dirname "$0")/.."

quick=false
save_bench=false
for arg in "$@"; do
    case "$arg" in
        --quick) quick=true ;;
        --save-bench) save_bench=true ;;
        *) echo "unknown option $arg" >&2; exit 2 ;;
    esac
done

offline=false
[ "${CARGO_NET_OFFLINE:-}" = "true" ] && offline=true
started=$(date +%s)
step() { printf '\n== %s\n' "$*"; }

step "format"
cargo fmt --check

step "clippy"
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --features bench -- -D warnings

if ! $quick; then
    windows=x86_64-pc-windows-gnu
    if rustup target list --installed 2>/dev/null | grep -qx "$windows"; then
        step "clippy for Windows"
        # Only a type-check: the host C compiler stands in for the
        # Windows one, which the C parts of the TLS library need.
        export CC_x86_64_pc_windows_gnu="${CC_x86_64_pc_windows_gnu:-gcc}"
        export AR_x86_64_pc_windows_gnu="${AR_x86_64_pc_windows_gnu:-ar}"
        cargo clippy --all-targets --target "$windows" --target-dir target/windows -- -D warnings
        cargo clippy --all-targets --features bench --target "$windows" --target-dir target/windows -- -D warnings
    else
        step "clippy for Windows: skipped (rustup target add $windows to run it)"
    fi
fi

# Run a command with the memory cap described above.
capped() {
    local limit="${TEST_MEMORY_MAX:-2G}"
    if systemd-run --user --scope --quiet -p MemoryMax=64M true >/dev/null 2>&1; then
        systemd-run --user --scope --quiet -p MemoryMax="$limit" -p MemorySwapMax=0 "$@"
    else
        # Without systemd: a limit on address space, which is looser (a
        # program reserves more than it uses) but still stops a runaway.
        local bytes
        bytes=$(numfmt --from=iec "$limit")
        ( ulimit -v $(( bytes / 1024 * 4 )); "$@" )
    fi
}

step "tests"
cargo test --no-run
capped cargo test

step "RustSec: cargo audit"
if $offline; then
    cargo audit --no-fetch
else
    cargo audit
fi

step "RustSec, licenses and sources: cargo deny"
if $offline; then
    cargo deny --offline check --hide-inclusion-graph
else
    cargo deny check --hide-inclusion-graph
fi

if ! $quick; then
    step "benchmarks"
    baseline=target/bench-baseline.txt
    latest=target/bench-latest.txt
    mkdir -p target
    config=$(mktemp -d)
    trap 'rm -rf "$config"' EXIT
    compare=()
    if [ -f "$baseline" ] && ! $save_bench; then
        compare=(--compare "$baseline" --tolerance "${BENCH_TOLERANCE:-50}")
    elif [ ! -f "$baseline" ]; then
        echo "no baseline yet: this run's results become it, without a comparison"
    fi
    cargo build --release --features bench --example bench
    capped env XDG_CONFIG_HOME="$config" cargo run --release --features bench --example bench -- \
        --save "$latest" "${compare[@]}"
    if $save_bench || [ ! -f "$baseline" ]; then
        cp "$latest" "$baseline"
        echo "benchmarks saved as the baseline in $baseline"
    fi
fi

printf '\nAll checks passed in %ss.\n' "$(( $(date +%s) - started ))"
