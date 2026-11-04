#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"
export CARGO_TARGET_DIR="$repo_root/target/coverage"
report_dir="$repo_root/target/quality"
baseline="$repo_root/.github/crap-baseline.json"
helper="$repo_root/.github/scripts/crap_report.py"

require_version() {
    local expected="$1" actual
    shift
    actual="$("$@" --version)"
    if [[ "$actual" != "$expected" ]]; then
        echo "Expected $expected; found $actual. See CONTRIBUTING.md for installation." >&2
        return 1
    fi
}

clear_reports() {
    rm -f -- "$report_dir/crap.json" "$report_dir/crap-delta.json" "$report_dir/summary.md"
}

case "${1:-}" in
    coverage)
        mkdir -p "$report_dir"
        clear_reports
        rm -f -- "$report_dir/lcov.info" "$report_dir/coverage-run.json"
        require_version 'cargo-llvm-cov 0.9.1' cargo llvm-cov
        # Capture before eval so a failure in show-env cannot be hidden by source.
        coverage_env="$(cargo llvm-cov show-env --sh)"
        eval "$coverage_env"
        cargo llvm-cov clean --workspace --locked
        # Plain cargo test preserves the existing selection, including doctests.
        cargo test --workspace --locked
        python3 -m unittest discover -s performance -p 'test_*.py'
        python3 -m unittest discover -s .github/scripts -p 'test_*.py'
        cargo test -p cytiq-cli --example performance_support --locked
        cargo llvm-cov report --lcov --output-path "$report_dir/lcov.info"
        # This marker is written only after every test and coverage export passed.
        python3 "$helper" record "$report_dir"
        ;;
    report|baseline)
        clear_reports
        if [[ ! -s "$report_dir/lcov.info" || ! -s "$report_dir/coverage-run.json" ]]; then
            echo 'No successful coverage run found. Run just coverage first.' >&2
            exit 1
        fi
        require_version 'cargo-crap 0.9.0' cargo crap
        cargo crap --workspace --lcov "$report_dir/lcov.info" \
            --format json --sort file --output "$report_dir/crap.json"
        if [[ "$1" == report ]]; then
            if [[ ! -s "$baseline" ]]; then
                echo 'Missing .github/crap-baseline.json; restore the reviewed Linux CI baseline.' >&2
                exit 1
            fi
            cargo crap --workspace --lcov "$report_dir/lcov.info" --baseline "$baseline" \
                --format json --sort file --output "$report_dir/crap-delta.json"
            python3 "$helper" summary "$report_dir" --baseline "$baseline"
        else
            # Bootstrap a candidate without changing the checked-in baseline.
            python3 "$helper" summary "$report_dir"
        fi
        cat "$report_dir/summary.md"
        if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
            cat "$report_dir/summary.md" >> "$GITHUB_STEP_SUMMARY"
        fi
        ;;
    *)
        echo 'Usage: bash .github/scripts/crap.sh {coverage|report|baseline}' >&2
        exit 2
        ;;
esac
