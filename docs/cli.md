# CLI

[English](cli.md) | [日本語](cli.ja.md)

The Rust binary is `crsec` (`cargo run --release -- <subcommand>`). Three subcommands are available: `run`, `sweep` and `reproduce`. `run` and `reproduce` accept `--mock` to drive the whole pipeline with a deterministic scripted client (no live LLM); `--canonical-mode {rule|llm}` selects the canonical-norm-identity method (the `rule` default is byte-identical to the deterministic key).

## LLM environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `OLLAMA_HOST` | `http://localhost:11434` | Ollama endpoint (tried first) |
| `OLLAMA_MODEL` | `llama3.2:latest` | Ollama model |
| `OPENAI_API_KEY` | (unset) | enables the OpenAI fallback |
| `OPENAI_MODEL` | `gpt-4o-mini` | OpenAI model |

Provider order is **Ollama first → OpenAI fallback**. With no live backend reachable, use the offline mock smoke (`cargo run --release --example mock_smoke -- results`).

## `run`

Run one society through the norm life-cycle.

```bash
cargo run --release -- run \
    --population 10 --entrepreneurs 3 \
    --network ws --ws-k 4 --ws-beta 0.1 \
    --rounds 48 --synth-threshold 200 --seed 42
```

| Flag | Default | Meaning |
|------|---------|---------|
| `--population` | 10 | number of agents N |
| `--entrepreneurs` | 3 | number of norm entrepreneurs |
| `--network` | ws | topology: `ws` / `er` / `ba` |
| `--ws-k` | 4 | WS initial degree k (even) |
| `--ws-beta` | 0.1 | WS rewiring probability β |
| `--er-p` | 0.3 | ER edge probability p |
| `--ba-m` | 2 | BA edges per new node m |
| `--rounds` | 48 | number of rounds T |
| `--synth-threshold` | 200 | long-term synthesis utility threshold θ |
| `--convergence-window` | 3 | stop when the qualified set is stable for K rounds |
| `--emergence-threshold` | 0.9 | adoption-rate threshold for time-to-emergence |
| `--canonical-mode` | deterministic | norm identity: `deterministic` (alias `rule`) / `llm` |
| `--mock` | false | drive with a deterministic scripted client (no live LLM) |
| `--seed` | random | socsim core seed |
| `--temperature` | 0.0 | LLM generation temperature |
| `--llm-seed` | 0 | LLM backend seed |
| `--cache-path` | `.llm_cache/cache.json` | prompt→response cache file |
| `--output-dir` | results | output base directory |

Outputs `results/{timestamp}/`: `config.json`, `metrics.csv`, `norms.csv`, `run_metadata.json`, plus a `results/latest` symlink. `metrics.csv` includes the per-type columns `adoption_injunctive` / `adoption_descriptive` and `n_distinct_injunctive` / `n_distinct_descriptive`.

With `--canonical-mode llm`, an LLM judge (cached, `temperature=0`) decides whether two norm expressions denote the same norm, also merging lexically disjoint paraphrases; a rule-key match short-circuits the judge. The `rule` path is unchanged.

## `sweep`

Sweep population × WS-β, multiple independent runs per cell.

```bash
cargo run --release -- sweep \
    --population-values 6,10,20 \
    --ws-beta-min 0.0 --ws-beta-max 0.5 --ws-beta-step 0.1 \
    --rounds 48 --runs 3 --seed 42
```

| Flag | Default | Meaning |
|------|---------|---------|
| `--population-values` | 6,10,20 | comma-separated population list |
| `--ws-beta-min` / `--ws-beta-max` / `--ws-beta-step` | 0.0 / 0.5 / 0.1 | WS-β grid |
| `--network` | ws | topology (single, fixed) |
| `--entrepreneurs` | 3 | entrepreneurs (fixed) |
| `--ws-k` | 4 | WS degree k |
| `--runs` | 3 | independent runs per cell |
| `--rounds` | 48 | rounds T |
| `--seed` | 42 | base seed (each run derives an independent seed) |
| `--cache-path` | `.llm_cache/cache.json` | shared cache across the sweep |
| `--output-dir` | results | output base directory |

Outputs `results/{timestamp}_sweep/`: `sweep_summary.csv`, `sweep_config.json`, plus a `results/latest` symlink.

## `reproduce`

Reproduce the paper's headline findings: norm **emergence** (adoption rises high), **consolidation** (the distinct-norm count contracts from its peak), social conflict **rise-then-fall**, and **Fact 7** (injunctive norms emerge before descriptive ones). Runs the standard setting over several seeds, averages the trajectory, and scores observed-vs-paper anchors.

```bash
# Offline (no live LLM): deterministic scripted mock
cargo run --release -- reproduce --mock

# Live (Ollama→OpenAI), three seeds
cargo run --release -- reproduce --population 12 --runs 3 --rounds 48 --seed 42
```

| Flag | Default | Meaning |
|------|---------|---------|
| `--population` | 12 | number of agents N |
| `--entrepreneurs` | 3 | number of norm entrepreneurs |
| `--network` | ws | topology: `ws` / `er` / `ba` |
| `--ws-k` | 4 | WS initial degree k |
| `--ws-beta` | 0.1 | WS rewiring probability β |
| `--rounds` | 48 | rounds T |
| `--runs` | 3 | independent runs (averaged) |
| `--emergence-threshold` | 0.9 | adoption-rate threshold for time-to-emergence |
| `--canonical-mode` | deterministic | norm identity: `deterministic` (alias `rule`) / `llm` |
| `--mock` | false | drive with a deterministic scripted client (no live LLM) |
| `--quick` | false | shrink N and rounds (a smoke check, not for paper-value validation) |
| `--temperature` / `--llm-seed` / `--cache-path` | 0.0 / 0 / `.llm_cache/cache.json` | LLM settings (live only) |
| `--seed` | 42 | base seed (each run derives an independent seed) |
| `--output-dir` | results | output base directory |

Outputs `results/reproduce_{timestamp}/`: `reproduce_summary.json` (the averaged cell, the observed-vs-paper anchors and the pass count) and `metrics.csv` (the representative run's trajectory). The Python `crsec-tools reproduce` reads these to draw `figures/emergence_trajectory.png` and `figures/descriptive_vs_injunctive.png`.

---
*This file was generated by Claude Code.*
