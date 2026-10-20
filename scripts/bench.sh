#!/usr/bin/env bash
# Times macho against the other fetch tools on PATH with hyperfine, and writes
# docs/benchmarks.md (with the machine, date and versions) plus docs/benchmarks.json,
# the summary scripts/screenshots.py charts. Tools that are not installed are skipped.
#
# usage: scripts/bench.sh
set -euo pipefail
cd "$(dirname "$0")/.."

command -v hyperfine >/dev/null || { echo "bench: hyperfine not found" >&2; exit 1; }
cargo build --release --quiet
macho=target/release/macho

names=(macho)
commands=("$macho")
versions=("macho $("$macho" --version | cut -d' ' -f2) (release build)")
add() { # add <name> <version command, "" for none, - to leave out> <command...>
    command -v "$3" >/dev/null || return 0
    names+=("$1")
    commands+=("${*:3}")
    [ "$2" = - ] && return 0
    local version
    version=$({ [ -n "$2" ] && eval "$2" 2>/dev/null; } | grep -oE '[0-9]+(\.[0-9]+)+' | head -1 || true)
    versions+=("$1${version:+ $version}")
}
# fastfetch twice: as shipped, and limited to the fields macho prints.
add fastfetch "fastfetch --version" fastfetch
add "fastfetch (macho's fields)" - fastfetch \
    --structure Title:Separator:OS:Kernel:Uptime:Packages:Shell:WM:Terminal:CPU:Memory:Break:Colors
add microfetch "microfetch --version" microfetch
add macchina "macchina --version" macchina
add pfetch "" pfetch # pfetch has no version flag
add neofetch "neofetch --version" neofetch

args=()
for i in "${!names[@]}"; do
    args+=(--command-name "${names[$i]}" "${commands[$i]}")
done

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
hyperfine --shell=none --warmup 20 --min-runs 50 --export-json "$tmp/runs.json" "${args[@]}"

export BENCH_DATE BENCH_MACHINE BENCH_SYSTEM BENCH_VERSIONS
BENCH_DATE=$(date +%F)
BENCH_MACHINE="$(sed -n 's/^model name\s*: //p' /proc/cpuinfo | head -1), $(nproc) threads, \
governor \`$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor 2>/dev/null || echo unknown)\`"
BENCH_SYSTEM="$(. /etc/os-release && echo "$PRETTY_NAME"), Linux $(uname -r)"
BENCH_VERSIONS=$(printf '%s, ' "${versions[@]}")
BENCH_VERSIONS=${BENCH_VERSIONS%, }

python3 - "$tmp/runs.json" <<'EOF'
import json, os, sys

runs = json.load(open(sys.argv[1]))["results"]
rows = sorted(
    (
        {
            "name": r.get("name", r["command"]),  # omitted when it is the command
            "mean_ms": s["time_wall_clock"]["mean"] * 1e3,
            "stddev_ms": s["time_wall_clock"]["stddev"] * 1e3,
            "min_ms": s["time_wall_clock"]["min"] * 1e3,
            "instructions": s["instructions"]["mean"] if "instructions" in s else None,
        }
        for r in runs
        for s in [r["summary"]]
    ),
    key=lambda row: row["mean_ms"],
)
meta = {k: os.environ[f"BENCH_{k.upper()}"] for k in ["date", "machine", "system", "versions"]}
with open("docs/benchmarks.json", "w") as f:
    json.dump({**meta, "results": rows}, f, indent=2)
    f.write("\n")

ms = lambda v, like=None: f"{v:.2f}" if (like or v) < 10 else f"{v:.1f}"  # σ takes its mean's precision
fastest = rows[0]["mean_ms"]
with open("docs/benchmarks.md", "w") as f:
    f.write(f"""# Benchmarks

Wall-clock time to print one fetch, from `scripts/bench.sh` on {meta['date']}. Each
tool runs with its default configuration and its output discarded; the second
fastfetch row limits it to the fields macho prints, for a like-for-like comparison.

- **Machine:** {meta['machine']}
- **System:** {meta['system']}
- **Method:** `hyperfine --shell=none --warmup 20 --min-runs 50`
- **Versions:** {meta['versions']}

| Tool | Mean [ms] | Min [ms] | Relative | Instructions |
|:---|---:|---:|---:|---:|
""")
    for r in rows:
        name = f"**{r['name']}**" if r["name"] == "macho" else r["name"]
        relative = r["mean_ms"] / fastest
        insns = f"{r['instructions'] / 1e6:.2f} M" if r["instructions"] else "–"
        f.write(
            f"| {name} | {ms(r['mean_ms'])} ± {ms(r['stddev_ms'], r['mean_ms'])} | {ms(r['min_ms'])} "
            f"| {relative:.1f}× | {insns} |\n"
        )
    f.write("""
Timings move with the machine, the package count and the page cache, so read the
ratios rather than the absolutes; instructions retired (from perf counters, where
hyperfine can read them) barely move between runs. Run `scripts/bench.sh` for your own.
""")
EOF
echo "wrote docs/benchmarks.md and docs/benchmarks.json"
