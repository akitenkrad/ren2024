# Architecture

[English](architecture.md) | [日本語](architecture.ja.md)

## Repository structure

```
ren2024/
├── simulation/                  Rust crate `crsec-simulation` (binary `crsec`)
│   ├── src/
│   │   ├── main.rs              CLI (clap): run / sweep / reproduce
│   │   ├── lib.rs               public modules
│   │   ├── config.rs            Config, Network{ws|er|ba}, CanonicalMode, LlmSettings
│   │   ├── norm.rs              PersonalNorm 5-tuple ⟨c,u,α,s_act,s_val⟩, NormType
│   │   ├── world.rs            CrsecWorld (WorldState), AgentProfile, InteractionEvent, canonical_key, Canonicalizer
│   │   ├── llm.rs              CrsecClient = CachingClient<Box<dyn LlmClient>> (Ollama→OpenAI + cache)
│   │   ├── prompts.rs          CRSEC LLM-operation prompts (KEY: value contract)
│   │   ├── parse.rs            lenient parser for the structured LLM responses
│   │   ├── mechanisms.rs       the six life-cycle mechanisms
│   │   ├── simulation.rs       init_world + run / run_mock drivers + canonicalizer wiring + output writers
│   │   ├── metrics.rs          adoption / compliance / conflicts / distinct norms (incl. per-type) / time-to-emergence
│   │   ├── reproduce.rs        the reproduce subcommand: averaged trajectory + observed-vs-paper anchors
│   │   └── reproduce_mock.rs   deterministic scripted client for offline run / reproduce
│   ├── examples/mock_smoke.rs  offline (no live LLM) smoke run
│   └── tests/integration_test.rs   mock-driven integration tests
├── tools/                       Python package `crsec-tools` (module `crsec_tools`)
│   └── src/crsec_tools/{cli,visualize,visualize_sweep,show_experiment_settings,reproduce_paper}.py
├── docs/                        bilingual docs (this directory)
└── results/                     run-time outputs (gitignored)
```

## Two-layer determinism

socsim's core is deterministic and LLM-free; LLM output is not bit-reproducible. The implementation keeps the two layers explicit:

- **Deterministic socsim core (lower layer).** From a single root seed two streams are derived: `derive_seed(root,&[0])` initialises the world (network generation, profile + entrepreneur assignment), and `derive_seed(root,&[1])` drives the engine (activation order via `RandomActivationScheduler`, conversation/observation partner sampling via `ctx.rng`). The metrics and the canonical-norm identity are pure functions of state, so they are deterministic too.
- **Non-deterministic LLM layer (upper layer).** Confined to the mechanisms via `CrsecClient = CachingClient<Box<dyn LlmClient>>`. The backend is `FallbackClient<OllamaClient, OpenAiClient>` boxed to `Box<dyn LlmClient>` (socsim-llm provides `impl LlmClient for Box<T>`, issue #26, so no local newtype is needed). `temperature=0`, a fixed seed and a `hash(prompt+model)` → response cache pseudo-determinise it.

`run_metadata.json` records the model, endpoint, temperature, seed, cache-hit rate, convergence, final step and time-to-emergence, plus a `determinism_note`.

## CRSEC life-cycle → mechanism mapping

The CRSEC norm life-cycle is translated to the socsim six-phase loop, one rule per mechanism. Declaration order = firing order within a phase.

| Mechanism | Phase | Role | LLM ops / agent / round |
|-----------|-------|------|-------------------------|
| `ResetInteractions` | PreStep | clear the round's interaction/observation log + counters | 0 |
| `CreationMechanism` | Environment | entrepreneurs create an initial norm when their DB is empty (`CreateNorm`) | ≤ 1 (only while DB empty; 0 in steady state) |
| `ComplianceMechanism` | Decision | generate a normative action consistent with the qualified set; record compliance | 1 if it holds any qualified norm |
| `SpreadingMechanism` | Interaction | for each sender, 1 conversation + 1 observation with network neighbours; **one structured call** does conflict detection + decide-to-talk + norm identification; identified norms enter the receiver DB unqualified | up to 2 (one per partner) |
| `EvaluationMechanism` | PostStep | **one structured call** runs the 4 sanity checks (consistency / duplication / type / conflict) → promote to qualified; if summed utility > θ, long-term synthesis (abstract norm + deactivate sources) | 1 per unevaluated candidate |
| `ConvergenceMechanism` | PostStep | stop when the qualified canonical set is stable for `K` rounds (`request_stop`) | 0 |

**LLM-call consolidation (documented pragmatism).** The paper lists several distinct LLM operations for spreading and evaluation. To keep calls bounded we consolidate (a) `DetectConflict` + `DecideToTalk` + `IdentifyNormativeInformation` into one spreading prompt, and (b) the four evaluation sanity checks into one evaluation prompt. The prompts ask for line-based `KEY: value` output that `parse.rs` reads leniently.

## Update semantics

One tick = one round in which every agent passes through create / comply / spread / evaluate. Norms identified during spreading are buffered and applied at the end of the Interaction phase, so a mid-round DB change does not cascade into other agents' identification within the same round (snapshot idiom). Activation order is randomised each round (order effects averaged); switch to a sequential scheduler for a fixed order.

## Canonical-norm identity

`world::Canonicalizer` decides whether two norm expressions are "the same" norm, with the method chosen by `--canonical-mode`:

- **`rule`** (default) — pure delegation to `world::canonical_key`, which reduces content to a deterministic keyword set (lowercase → split on non-alphanumerics → drop stopwords → dedup → sort → join). The adoption rate, distinct-norm count, the spreading dedup and the convergence set all bucket by this key, with **no extra LLM calls**; this path is byte-identical to the pre-`Canonicalizer` code.
- **`llm`** — a cached LLM judge (`prompts::same_norm_prompt`, parsed by `parse::same_norm`) answers "same norm?" against a registry of seen representatives, merging even lexically disjoint paraphrases. A rule-key match short-circuits the judge; the judge runs through the shared cached client, so it stays pseudo-deterministic. The canonicalizer is shared (`Rc`) by the spreading dedup and the convergence mechanism.

## Metrics

| metric | definition |
|--------|------------|
| `adoption_rate` | fraction of agents holding the most-shared canonical norm as qualified |
| `compliance_rate` | fraction of agents that complied (COMPLY=yes) this round |
| `n_conflicts` | number of conflicts detected this round (DetectConflict=T) |
| `n_distinct_norms` | number of distinct canonical norms (qualified only) |
| `adoption_injunctive` / `adoption_descriptive` | per-type adoption rate (injunctive vs descriptive) |
| `n_distinct_injunctive` / `n_distinct_descriptive` | per-type distinct canonical norm count |
| `time_to_emergence` | first round where `adoption_rate ≥ emergence_threshold` (default 0.9) |

The per-type columns drive the descriptive-vs-injunctive deep dive: the `reproduce` subcommand compares the per-type time-to-emergence (injunctive precedes descriptive) and the Python `reproduce` tool draws the two trajectories.

## Qualitative reproduction targets

The local model (`llama3.2:latest`) differs from the paper's GPT-3.5/4, so we target the **pattern**, not the numbers: adoption rises toward 1.0; the distinct-norm count consolidates; the conflict count rises early then declines; injunctive norms emerge before descriptive ones. The `reproduce` subcommand scores these as observed-vs-paper anchors and writes `reproduce_summary.json`.

## References

- Ren, S., Cui, Z., Song, R., Wang, Z., & Hu, S. (2024). Emergence of Social Norms in Generative Agent Societies: Principles and Architecture. *IJCAI-24*, 7895–7903. arXiv:2403.08251.
- Park, J. S., et al. (2023). Generative Agents: Interactive Simulacra of Human Behavior. *UIST '23*. arXiv:2304.03442.
- Cialdini, R. B., Kallgren, C. A., & Reno, R. R. (1991). A Focus Theory of Normative Conduct.
- [socsim](https://github.com/akitenkrad/rs-social-simulation-tools) — `socsim-core` / `socsim-engine` / `socsim-net` / `socsim-llm`.

---
*This file was generated by Claude Code.*
