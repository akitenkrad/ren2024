# Visualization

[English](visualization.md) | [日本語](visualization.ja.md)

The Python package `crsec-tools` (module `crsec_tools`) reads the Rust outputs and renders figures. Install with `uv sync` at the workspace root, then run via `uv run crsec-tools <subcommand>`. Dependencies: matplotlib, pandas, numpy, networkx, `runvault[read]`.

Resolving a run directory is left to `runvault path`: nothing here globs `results/` hoping to land on the newest directory. Omitting `--results-dir` / `--sweep-dir` takes the most recent run. **Figures go outside the run** (`<results-root>/crsec/figures/<run_slug>/`): `manifest.csv` is settled by `finish()`, so anything added afterwards would mix a file with no hash into the record.

A pre-migration `results/<timestamp>/` still reads as before if you pass it to `--results-dir`.

## `visualize`

Reads `metrics.csv` from a `run` output directory.

```bash
uv run crsec-tools visualize
uv run crsec-tools visualize --results-dir "$(runvault path --experiment crsec --latest --subcommand run --standalone)"
```

`--standalone` keeps a sweep or reproduce child from being picked instead. Produces, under `<results-root>/crsec/figures/<run_slug>/`:

- `emergence_curves.png` — adoption rate & compliance rate over rounds (the paper's Fig. 2 shape). Expect both to climb toward 1.0; the dashed line marks the 0.9 emergence threshold.
- `conflicts_timeseries.png` — social-conflict count per round. Expect an early rise then a decline as the shared norm settles.
- `distinct_norms.png` — number of distinct canonical norms and the count of qualified-norm holders over rounds (diversity / convergence).

## `visualize-sweep`

Rebuilds the one-row-per-(cell × trial) table from a sweep parent's children (the sweep table is not on disk).

```bash
uv run crsec-tools visualize-sweep
uv run crsec-tools visualize-sweep --sweep-dir "$(runvault path --experiment crsec --latest --subcommand sweep)"
```

Produces, under `<results-root>/crsec/figures/<run_slug>/`:

- `sweep_time_to_emergence_heatmap.png` — mean time-to-emergence over the population × WS-β grid (a run that never emerged has no such metric at all, so it comes back as NaN and drops out of the mean).
- `sweep_adoption_heatmap.png` — mean final adoption rate over the grid.
- `sweep_curves.png` — final adoption vs β (one line per population) and mean time-to-emergence vs population.

## `show-experiment-settings`

Prints a run directory's `config.json` (conditions under `parameters`), its run-scope metrics (convergence, final step, time-to-emergence), the `llm` block of `run.json` (provider, model, temperature) and the cache-hit rate. Whether it was a `run`, a `sweep` or a `reproduce` comes from `run.json`'s `subcommand`.

```bash
uv run crsec-tools show-experiment-settings
uv run crsec-tools show-experiment-settings --subcommand sweep
uv run crsec-tools show-experiment-settings --results-dir "$(runvault path --experiment crsec --latest --subcommand reproduce)" --json
```

## `reproduce`

Reproduces the paper's headline findings end-to-end and draws the figures. Reads the **parent run** of a `crsec reproduce` (its across-trial `scope=sweep` metrics and its `reference.csv`) and the **representative run** (the child with `replicate_index = 0`); pass `--run` to launch the Rust binary first (add `--mock` to stay offline). See the [CLI](cli.md) for the `reproduce` subcommand.

```bash
uv run crsec-tools reproduce --run --mock           # reproduce offline, then draw figures
uv run crsec-tools reproduce --run --mock --quick   # lightweight smoke check
uv run crsec-tools reproduce                          # visualise the most recent parent run
uv run crsec-tools reproduce --json                   # print the summary as JSON
```

Produces, under `<results-root>/crsec/figures/<run_slug>/`:

- `emergence_trajectory.png` — the representative run's adoption / compliance, the distinct-norm count (consolidation), and the conflict series (rise-then-fall), in three stacked panels.
- `descriptive_vs_injunctive.png` — the per-type adoption trajectories (injunctive vs descriptive, with each type's time-to-emergence marked) and the per-type distinct-norm counts. This is the descriptive-vs-injunctive deep dive: injunctive norms emerge before descriptive ones (Fact 7).

The console prints the aggregates and the **difference from the paper's values** (with the source from `reference.csv`). The banded PASS/OFF stays on the Rust side's `crsec reproduce` console: putting the same threshold in both Python and Rust would leave room for the two to disagree.

## Interpreting the outputs

Because the local model differs from the paper's, read the figures **qualitatively**: a rising adoption curve, a rise-then-fall conflict series, and a collapse to a small number of distinct (canonical) norms together indicate that a shared social norm has emerged. In the type-split view, the injunctive curve crossing the emergence threshold before the descriptive one is the ordering effect (Fact 7).

---
*This file was generated by Claude Code.*
