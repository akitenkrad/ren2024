//! CRSEC の規範ライフサイクルを socsim の 6-phase ループへ翻訳したメカニズム群．
//!
//! 二層アーキテクチャの **境界** がここにある．下層（決定論的 socsim コア）は会話・
//! 観察相手の選択を `ctx.rng`（ChaCha20）と `network` 近傍で行い，上層（非決定的 LLM
//! レイヤ）は [`CrsecClient`]（キャッシュ付き Ollama→OpenAI フォールバック）越しに
//! 規範の創出・伝播・評価・遵守を行う．
//!
//! | Mechanism | Phase | 役割 |
//! |-----------|-------|------|
//! | [`ResetInteractions`] | PreStep | 当ステップの会話・観察ログをクリア |
//! | [`CreationMechanism`] | Environment | 起業家が初期規範を創出（DB 空のとき; LLM） |
//! | [`ComplianceMechanism`] | Decision | 適格規範に沿った行動生成・遵守判定（LLM） |
//! | [`SpreadingMechanism`] | Interaction | 衝突検出+会話判断+規範識別（統合 LLM 呼び出し） |
//! | [`EvaluationMechanism`] | PostStep | 4 サニティ検査（統合 LLM）→ 昇格 / 長期統合 |
//! | [`ConvergenceMechanism`] | PostStep | 適格集合が K ステップ不変なら request_stop |
//!
//! # LLM 呼び出し予算（1 エージェント / 1 ラウンド）
//!
//! - 創出: 起業家かつ DB 空のときのみ 1 回（定常では 0）．
//! - 遵守: 適格規範を持つとき 1 回．
//! - 伝播: 近傍 1 名との会話 1 回 + 観察 1 回（統合呼び出しなので各 1 回）．
//! - 評価: 未評価候補ごとに 1 回（統合 4 検査）．
//!
//! 伝播の «衝突検出 + 会話判断 + 規範識別» と評価の «4 サニティ検査» はそれぞれ
//! **1 回の構造化 LLM 呼び出し** に統合してプロンプト数を抑える（[`crate::prompts`]）．
//!
//! LLM クライアントと呼び出しメタデータは `Rc<RefCell<…>>` で共有し，run ドライバ
//! が実行後にキャッシュ保存・メタデータ集計に使う．

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use rand::seq::SliceRandom;

use socsim_core::{AgentId, Mechanism, Phase, Result, SocsimError, StepContext};
use socsim_llm::MetadataCollector;

use crate::config::LlmSettings;
use crate::llm::{llm_config, CrsecClient};
use crate::norm::{NormType, PersonalNorm};
use crate::parse;
use crate::prompts;
use crate::world::{canonical_key, Canonicalizer, CrsecWorld, InteractionEvent};

/// 共有 LLM クライアント（run ドライバとメカニズムで共有）．
pub type SharedClient = Rc<RefCell<CrsecClient>>;
/// 共有メタデータコレクタ（cache-hit 率などを run 後に集計）．
pub type SharedMetadata = Rc<RefCell<MetadataCollector>>;
/// 共有 canonicalizer（規範同定の方式; rule = 決定論 / llm = 意味判定）．
pub type SharedCanonicalizer = Rc<Canonicalizer<'static>>;

/// 当ラウンドの遵守者数・衝突数を scratch へ渡す key（run ドライバが読む）．
pub const SCRATCH_COMPLIED: &str = "complied";
pub const SCRATCH_CONFLICTS: &str = "conflicts";
pub const SCRATCH_CONVERGED: &str = "converged";

/// 共有シーンの記述（プロンプトに渡すコンテキスト; CRSEC の Smallville「Hobbs Café」相当）．
const SCENE: &str = "a shared community café where members gather, talk, and observe one another";

/// LLM を呼び出し本文を取り出すヘルパ（メタデータも記録する）．
fn complete(
    client: &SharedClient,
    metadata: &SharedMetadata,
    settings: &LlmSettings,
    prompt: &str,
) -> Result<String> {
    let mut c = client.borrow_mut();
    let resp = c
        .complete(prompt, &llm_config(settings))
        .map_err(|e| SocsimError::Mechanism(format!("LLM call failed: {e}")))?;
    metadata.borrow_mut().record(resp.metadata.clone());
    Ok(resp.text)
}

// --------------------------------------------------------------------------- //
// PreStep: ResetInteractions
// --------------------------------------------------------------------------- //

/// 当ステップの会話・観察ログと集計バッファを初期化する（`PreStep`）．
pub struct ResetInteractions;

impl Mechanism<CrsecWorld> for ResetInteractions {
    fn name(&self) -> &str {
        "reset_interactions"
    }
    fn phases(&self) -> &'static [Phase] {
        &[Phase::PreStep]
    }
    fn apply(&mut self, _phase: Phase, ctx: &mut StepContext<'_, CrsecWorld>) -> Result<()> {
        ctx.world.interactions.clear();
        ctx.scratch.insert(SCRATCH_COMPLIED, 0usize);
        ctx.scratch.insert(SCRATCH_CONFLICTS, 0usize);
        Ok(())
    }
}

// --------------------------------------------------------------------------- //
// Environment: CreationMechanism
// --------------------------------------------------------------------------- //

/// シーン/コンテキスト設定 + 規範起業家が初期個人規範を生成する（`Environment`）．
///
/// DB が空の起業家のみ `CreateNorm(G)` を 1 回呼ぶ（定常では呼ばれない）．生成規範は
/// 即座に適格（起業家の内在規範）として DB に入る．
pub struct CreationMechanism {
    client: SharedClient,
    metadata: SharedMetadata,
    settings: LlmSettings,
}

impl CreationMechanism {
    pub fn new(client: SharedClient, metadata: SharedMetadata, settings: LlmSettings) -> Self {
        CreationMechanism {
            client,
            metadata,
            settings,
        }
    }
}

impl Mechanism<CrsecWorld> for CreationMechanism {
    fn name(&self) -> &str {
        "creation"
    }
    fn phases(&self) -> &'static [Phase] {
        &[Phase::Environment]
    }
    fn apply(&mut self, _phase: Phase, ctx: &mut StepContext<'_, CrsecWorld>) -> Result<()> {
        // スケジューラ順で走査（決定論; 順序効果は scheduler に委ねる）．
        for &id in ctx.agent_order {
            let profile = match ctx.world.agents.get(&id) {
                Some(p) => p.clone(),
                None => continue,
            };
            let db_empty = ctx
                .world
                .norm_db
                .get(&id)
                .map(|v| v.is_empty())
                .unwrap_or(true);
            if profile.is_entrepreneur && db_empty {
                let prompt = prompts::create_norm_prompt(&profile, SCENE);
                let text = complete(&self.client, &self.metadata, &self.settings, &prompt)?;
                if let Some(norm) = parse::created_norm(&text) {
                    ctx.world.norm_db.entry(id).or_default().push(norm);
                }
            }
        }
        Ok(())
    }
}

// --------------------------------------------------------------------------- //
// Decision: ComplianceMechanism
// --------------------------------------------------------------------------- //

/// 各エージェントが適格規範集合に沿って行動を生成し，遵守判定を行う（`Decision`）．
///
/// 適格規範を持つエージェントのみ `GenerateNormativePlan(C, P)` を 1 回呼ぶ．遵守
/// （COMPLY=yes）したエージェント数を scratch へ積む（compliance_rate の母数）．
pub struct ComplianceMechanism {
    client: SharedClient,
    metadata: SharedMetadata,
    settings: LlmSettings,
}

impl ComplianceMechanism {
    pub fn new(client: SharedClient, metadata: SharedMetadata, settings: LlmSettings) -> Self {
        ComplianceMechanism {
            client,
            metadata,
            settings,
        }
    }
}

impl Mechanism<CrsecWorld> for ComplianceMechanism {
    fn name(&self) -> &str {
        "compliance"
    }
    fn phases(&self) -> &'static [Phase] {
        &[Phase::Decision]
    }
    fn apply(&mut self, _phase: Phase, ctx: &mut StepContext<'_, CrsecWorld>) -> Result<()> {
        let mut complied = 0usize;
        for &id in ctx.agent_order {
            let profile = match ctx.world.agents.get(&id) {
                Some(p) => p.clone(),
                None => continue,
            };
            let qualified: Vec<PersonalNorm> =
                ctx.world.qualified_norms(id).into_iter().cloned().collect();
            if qualified.is_empty() {
                continue;
            }
            let refs: Vec<&PersonalNorm> = qualified.iter().collect();
            let prompt = prompts::compliance_prompt(&profile, &refs, SCENE);
            let text = complete(&self.client, &self.metadata, &self.settings, &prompt)?;
            if parse::yes(&text, "COMPLY") {
                complied += 1;
            }
        }
        ctx.scratch.insert(SCRATCH_COMPLIED, complied);
        Ok(())
    }
}

// --------------------------------------------------------------------------- //
// Interaction: SpreadingMechanism（伝播の中核）
// --------------------------------------------------------------------------- //

/// 送信者が衝突検出→会話判断→発話し，受信者・観察者が規範的情報を識別する
/// （`Interaction`）．識別規範は **未適格**（s_act=F, s_val=F）で受信者 DB に格納し，
/// 評価フェーズで昇格を待つ．会話・観察の相手は `network` 近傍から `ctx.rng` で選ぶ．
///
/// 統合呼び出し（衝突検出 + 会話判断 + 規範識別を 1 回）でプロンプト数を抑える．
/// 識別された未適格規範は «同一ステップ内の他者の識別に波及させない» ため，まず
/// バッファに溜めてから一括適用する（更新セマンティクスのスナップショット）．
pub struct SpreadingMechanism {
    client: SharedClient,
    metadata: SharedMetadata,
    settings: LlmSettings,
    canon: SharedCanonicalizer,
}

impl SpreadingMechanism {
    pub fn new(
        client: SharedClient,
        metadata: SharedMetadata,
        settings: LlmSettings,
        canon: SharedCanonicalizer,
    ) -> Self {
        SpreadingMechanism {
            client,
            metadata,
            settings,
            canon,
        }
    }

    /// 送信者 `sender` と相手 `receiver` の 1 回の相互作用を処理する．
    /// 戻り値: (衝突したか, 受信者に追加される未適格規範 or None)．
    fn interact(
        &self,
        ctx: &mut StepContext<'_, CrsecWorld>,
        sender: AgentId,
        receiver: AgentId,
        is_observation: bool,
    ) -> Result<(bool, Option<(AgentId, PersonalNorm)>)> {
        let sp = ctx
            .world
            .agents
            .get(&sender)
            .expect("sender exists")
            .clone();
        let rp = ctx
            .world
            .agents
            .get(&receiver)
            .expect("receiver exists")
            .clone();
        let sender_qualified: Vec<PersonalNorm> = ctx
            .world
            .qualified_norms(sender)
            .into_iter()
            .cloned()
            .collect();
        let refs: Vec<&PersonalNorm> = sender_qualified.iter().collect();

        let prompt = prompts::spreading_prompt(&sp, &rp, &refs, is_observation, SCENE);
        let text = complete(&self.client, &self.metadata, &self.settings, &prompt)?;

        let conflict = parse::yes(&text, "CONFLICT");
        let talk = is_observation || parse::yes(&text, "TALK");

        // 会話/観察が成立し，かつ規範が識別されたときのみ受信者へ伝える．
        let identified = if talk {
            parse::identified_norm(&text).map(|n| (receiver, n))
        } else {
            None
        };

        ctx.world.interactions.push(InteractionEvent {
            sender,
            receiver,
            is_observation,
            conflict,
            identified_content: identified.as_ref().map(|(_, n)| n.content.clone()),
        });

        Ok((conflict, identified))
    }
}

impl Mechanism<CrsecWorld> for SpreadingMechanism {
    fn name(&self) -> &str {
        "spreading"
    }
    fn phases(&self) -> &'static [Phase] {
        &[Phase::Interaction]
    }
    fn apply(&mut self, _phase: Phase, ctx: &mut StepContext<'_, CrsecWorld>) -> Result<()> {
        let mut conflicts = 0usize;
        // 識別規範は同一ステップ内では波及させずバッファへ溜める（スナップショット）．
        let mut pending: Vec<(AgentId, PersonalNorm)> = Vec::new();

        let order: Vec<AgentId> = ctx.agent_order.to_vec();
        for sender in order {
            // 近傍から会話相手・観察相手を選ぶ（決定論; socsim コア層 rng）．
            let mut neighbors: Vec<AgentId> = ctx
                .world
                .network
                .neighbors(sender)
                .into_iter()
                .filter(|&id| id != sender)
                .collect();
            if neighbors.is_empty() {
                continue;
            }
            neighbors.sort(); // 決定論のため shuffle 前にソート安定化．
            neighbors.shuffle(ctx.rng);

            // 会話相手 1 名．
            let convo = neighbors[0];
            let (c1, id1) = self.interact(ctx, sender, convo, false)?;
            if c1 {
                conflicts += 1;
            }
            if let Some(p) = id1 {
                pending.push(p);
            }

            // 観察相手 1 名（別の近傍がいれば）．
            if neighbors.len() >= 2 {
                let obs = neighbors[1];
                let (c2, id2) = self.interact(ctx, sender, obs, true)?;
                if c2 {
                    conflicts += 1;
                }
                if let Some(p) = id2 {
                    pending.push(p);
                }
            }
        }

        // バッファを一括適用: 既に（適格/未適格を問わず）同一 canonical の規範を
        // 持つ受信者には重複追加しない．canonical 同定は `--canonical-mode` に従う
        // （rule = 決定論的 canonical_key / llm = LLM 意味判定）．rule では
        // [`canonical_key`] と完全に同一の束ね方になる（バイト等価）．
        for (receiver, norm) in pending {
            let entry = ctx.world.norm_db.entry(receiver).or_default();
            let key = self.canon.canonicalize(&norm.content);
            let exists = entry
                .iter()
                .any(|n| self.canon.canonicalize(&n.content) == key);
            if !exists {
                entry.push(norm);
            }
        }

        ctx.scratch.insert(SCRATCH_CONFLICTS, conflicts);
        Ok(())
    }
}

// --------------------------------------------------------------------------- //
// PostStep: EvaluationMechanism
// --------------------------------------------------------------------------- //

/// 即時評価（整合性・重複・型・衝突の 4 サニティ検査を統合呼び出し）→ 適格へ昇格．
/// 有用性合計が θ 超なら長期統合（抽象規範生成 + 元規範の非活性化）（`PostStep`）．
pub struct EvaluationMechanism {
    client: SharedClient,
    metadata: SharedMetadata,
    settings: LlmSettings,
}

impl EvaluationMechanism {
    pub fn new(client: SharedClient, metadata: SharedMetadata, settings: LlmSettings) -> Self {
        EvaluationMechanism {
            client,
            metadata,
            settings,
        }
    }
}

impl Mechanism<CrsecWorld> for EvaluationMechanism {
    fn name(&self) -> &str {
        "evaluation"
    }
    fn phases(&self) -> &'static [Phase] {
        &[Phase::PostStep]
    }
    fn apply(&mut self, _phase: Phase, ctx: &mut StepContext<'_, CrsecWorld>) -> Result<()> {
        let synth_threshold = ctx.world.synth_threshold;
        for &id in ctx.agent_order {
            let profile = match ctx.world.agents.get(&id) {
                Some(p) => p.clone(),
                None => continue,
            };

            // 未適格候補のインデックスを集める（評価対象）．
            let candidate_idx: Vec<usize> = ctx
                .world
                .norm_db
                .get(&id)
                .map(|v| {
                    v.iter()
                        .enumerate()
                        .filter(|(_, n)| !n.qualified())
                        .map(|(i, _)| i)
                        .collect()
                })
                .unwrap_or_default();

            for ci in candidate_idx {
                let candidate = ctx.world.norm_db.get(&id).and_then(|v| v.get(ci)).cloned();
                let candidate = match candidate {
                    Some(c) => c,
                    None => continue,
                };
                let existing: Vec<PersonalNorm> =
                    ctx.world.qualified_norms(id).into_iter().cloned().collect();
                let refs: Vec<&PersonalNorm> = existing.iter().collect();
                let prompt = prompts::evaluation_prompt(&profile, &candidate, &refs);
                let text = complete(&self.client, &self.metadata, &self.settings, &prompt)?;
                if parse::promote_decision(&text) {
                    if let Some(v) = ctx.world.norm_db.get_mut(&id) {
                        if let Some(n) = v.get_mut(ci) {
                            n.promote();
                        }
                    }
                }
            }

            // 長期統合（基本 θ 規則）: 適格規範群の合計有用性が θ 超なら，最も有用な
            // 2 件を抽象規範へ統合し元規範を非活性化する（Phase 3 で LLM 抽象化へ拡張）．
            self.maybe_synthesize(ctx, id, synth_threshold);
        }
        Ok(())
    }
}

impl EvaluationMechanism {
    /// θ 超のとき長期統合を行う（決定論的な基本規則; Phase 3 拡張点）．
    fn maybe_synthesize(&self, ctx: &mut StepContext<'_, CrsecWorld>, id: AgentId, theta: f64) {
        let v = match ctx.world.norm_db.get(&id) {
            Some(v) => v,
            None => return,
        };
        let qualified: Vec<(usize, u8, NormType)> = v
            .iter()
            .enumerate()
            .filter(|(_, n)| n.qualified())
            .map(|(i, n)| (i, n.utility, n.alpha))
            .collect();
        let total: f64 = qualified.iter().map(|(_, u, _)| *u as f64).sum();
        if total <= theta || qualified.len() < 2 {
            return;
        }
        // 最も有用な 2 件を統合元に選ぶ（決定論; utility 降順, index 昇順）．
        let mut sorted = qualified.clone();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let (i0, u0, t0) = sorted[0];
        let (i1, u1, _t1) = sorted[1];
        let c0 = v[i0].content.clone();
        let c1 = v[i1].content.clone();
        let abstract_content = format!(
            "In general, {} and {}",
            c0.trim_end_matches('.'),
            c1.trim_end_matches('.')
        );
        let abstract_util = ((u0 as u16 + u1 as u16) / 2).min(100) as u8;

        if let Some(v) = ctx.world.norm_db.get_mut(&id) {
            // 元規範を非活性化（s_act=false）．
            if let Some(n) = v.get_mut(i0) {
                n.deactivate();
            }
            if let Some(n) = v.get_mut(i1) {
                n.deactivate();
            }
            // 抽象規範を適格として追加（重複なら追加しない）．
            let key = canonical_key(&abstract_content);
            if !v.iter().any(|n| canonical_key(&n.content) == key) {
                v.push(PersonalNorm::created(abstract_content, abstract_util, t0));
            }
        }
    }
}

// --------------------------------------------------------------------------- //
// PostStep: ConvergenceMechanism
// --------------------------------------------------------------------------- //

/// 適格規範集合の安定化判定（`PostStep`）．
///
/// 集団全体の適格 canonical 規範集合が直近 `window` ステップ不変なら `request_stop` を
/// 呼ぶ．宣言順で EvaluationMechanism の後に置く（昇格後の集合で判定する）．
pub struct ConvergenceMechanism {
    /// 安定ウィンドウ K．
    pub window: usize,
    /// 過去の適格 canonical 集合の履歴（直近のみ保持）．
    history: Vec<Vec<String>>,
    /// 規範同定の方式（rule = 決定論 / llm = 意味判定）．
    canon: SharedCanonicalizer,
}

impl ConvergenceMechanism {
    pub fn new(window: usize, canon: SharedCanonicalizer) -> Self {
        ConvergenceMechanism {
            window: window.max(1),
            history: Vec::new(),
            canon,
        }
    }
}

impl Mechanism<CrsecWorld> for ConvergenceMechanism {
    fn name(&self) -> &str {
        "convergence"
    }
    fn phases(&self) -> &'static [Phase] {
        &[Phase::PostStep]
    }
    fn apply(&mut self, _phase: Phase, ctx: &mut StepContext<'_, CrsecWorld>) -> Result<()> {
        let current = ctx.world.qualified_canonical_set_with(&self.canon);
        self.history.push(current.clone());
        if self.history.len() > self.window {
            self.history.remove(0);
        }
        // 集合が非空かつ window 件すべて一致なら安定 → 停止．
        let stable = self.history.len() >= self.window
            && !current.is_empty()
            && self.history.iter().all(|h| *h == current);
        ctx.scratch.insert(SCRATCH_CONVERGED, stable);
        if stable {
            ctx.request_stop();
        }
        Ok(())
    }
}

/// 空の規範DBを作る（全エージェント空; init 用ヘルパ）．
pub fn empty_norm_db(ids: &[AgentId]) -> BTreeMap<AgentId, Vec<PersonalNorm>> {
    ids.iter().map(|&id| (id, Vec::new())).collect()
}
