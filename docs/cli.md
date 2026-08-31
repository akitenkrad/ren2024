# CLI

[English](cli.md) | [日本語](cli.ja.md)

The Rust binary is `crsec` (`cargo run --release -- <subcommand>`). Three subcommands are available: `run`, `sweep` and `reproduce`.

Where the output goes, and its identity, belong to [runvault](https://github.com/akitenkrad/rs-runvault). No timestamped directory and no `results/latest` symlink are made here: everything is written into the run directory `Run::start` decided (`<results-root>/crsec/<subcommand>_<timestamp>_<config_hash>_<uid>/`). `--output-dir` names the results root. A run directory's path comes from:

```bash
runvault path --experiment crsec --latest --subcommand run --standalone
runvault path --experiment crsec --latest --subcommand sweep
runvault path --experiment crsec --latest --subcommand reproduce
```
 `run` and `reproduce` accept `--mock` to drive the whole pipeline with a deterministic scripted client (no live LLM); `--canonical-mode {rule|llm}` selects the canonical-norm-identity method (the `rule` default is byte-identical to the deterministic key).

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
| `--output-dir` | results | runvault results root |

The output is one run directory (`subcommand=run`):

- `config.json` — the experiment's conditions (under `parameters`). `llm_cache_path` is only a location, so it is excluded from `config_hash`.
- `metrics.csv` — long form, `run_uid,step,step_unit,scope,name,value`. `step` is the round `t` (`step_unit=round`), `scope=run`. The per-round metrics are `adoption_rate`, `compliance_rate`, `n_conflicts`, `n_distinct_norms`, `n_qualified_holders` and the per-type `adoption_injunctive` / `adoption_descriptive` / `n_distinct_injunctive` / `n_distinct_descriptive`. The whole-run metrics carry no step: `n_units`, `converged`, `final_step`, `time_to_emergence` (**no row at all when the run never reached the threshold**), and `llm_calls` / `llm_cache_hits` / `llm_cache_hit_rate`.
- `events.jsonl` — the old `norms.csv`. The final norm database is written as `x.ren2024.norm` events, one norm per line (`unit_id` is the agent, `t` is the final round, and the fields are `norm_type`, `content`, `utility`, `s_act`, `s_val`, `qualified`).
- `run.json` — the LLM provider, model and temperature live in the `llm` block (no `run_metadata.json` is written any more).

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
| `--output-dir` | results | runvault results root |

The output is **one parent plus one child run per trial**. The parent (`subcommand=sweep`) holds the grid definition itself in `parameters` and writes no per-condition metric; being more than one simulation, it claims no `master_seed`. Each child (`subcommand=sweep-point`) is one (population × β × trial), with the same shape of `parameters`, `metrics.csv` and `events.jsonl` as a `run` started by hand — so the same condition gives the same `config_hash`. Repetitions of one cell are told apart by `replicate_index`.

The old `sweep_summary.csv` is not on disk. The same values are in the children, so `crsec_tools.sweep_summary` rebuilds the table (this is what `crsec-tools visualize-sweep` uses). One value is expressed differently: the old CSV wrote `time_to_emergence = -1` as a sentinel for "never emerged", whereas runvault writes no row at all (it comes back as NaN).

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
| `--output-dir` | results | runvault results root |

The output is **one parent plus one child run per trial**. The parent's `metrics.csv` (`subcommand=reproduce`) holds the across-trial averages (`scope=sweep`: `mean_final_adoption`, `mean_tte_injunctive`, `consolidation_gap`, …), and its `reference.csv` holds the **paper's reported values** (Fact 4: 100% of agents had adopted and adhered to the injunctive norms at the end of Day 2) with their source. Each child (`subcommand=reproduce-run`) is one trial, numbered by `replicate_index`.

The tolerance bands (`target_lo` / `target_hi`) are not the paper's claim but this implementation's own, so they are not recorded — the banded PASS/OFF stays on the console. The Python `crsec-tools reproduce` reads the parent's aggregates and `reference.csv` and reports the difference from the paper's values rather than the bands (putting the same threshold in both Python and Rust leaves room for the two to disagree). The figures are drawn from the representative run (the child with `replicate_index = 0`).

---
*This file was generated by Claude Code.*
