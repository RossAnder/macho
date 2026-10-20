# Benchmarks

Wall-clock time to print one fetch, from `scripts/bench.sh` on 2026-10-20. Each
tool runs with its default configuration and its output discarded; the second
fastfetch row limits it to the fields macho prints, for a like-for-like comparison.

- **Machine:** AMD Ryzen 7 3700X 8-Core Processor, 16 threads, governor `powersave`
- **System:** Arch Linux, Linux 7.2.9-2-cachyos
- **Method:** `hyperfine --shell=none --warmup 20 --min-runs 50`
- **Versions:** macho 0.1.0 (release build), fastfetch 2.70.0, microfetch 1.2.0, macchina 6.4.0, pfetch, neofetch 7.1.0

| Tool | Mean [ms] | Min [ms] | Relative | Instructions |
|:---|---:|---:|---:|---:|
| microfetch | 0.16 ± 0.02 | 0.14 | 1.0× | 0.04 M |
| **macho** | 0.16 ± 0.01 | 0.15 | 1.0× | 0.04 M |
| fastfetch (macho's fields) | 25.4 ± 0.5 | 24.8 | 158.1× | 90.40 M |
| macchina | 26.6 ± 0.5 | 25.7 | 165.2× | 193.11 M |
| fastfetch | 30.0 ± 0.6 | 29.1 | 186.2× | 99.91 M |
| pfetch | 62.1 ± 0.5 | 61.2 | 386.1× | 247.76 M |
| neofetch | 155.6 ± 0.8 | 153.7 | 967.2× | 509.65 M |

Timings move with the machine, the package count and the page cache, so read the
ratios rather than the absolutes; instructions retired (from perf counters, where
hyperfine can read them) barely move between runs. Run `scripts/bench.sh` for your own.
