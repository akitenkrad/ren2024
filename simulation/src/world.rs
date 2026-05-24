//! CRSEC の世界状態（socsim `WorldState` 実装）．
//!
//! エージェントは **固定ノード** であり移動も空間格子も持たない．状態として変化する
//! のは「内在化した個人規範の集合（`norm_db`）」と「社会接続（`network`）」だけ．
//! 相互作用（会話・観察）の相手は `socsim-net::SocialNetwork`（無向; 会話・観察は
//! 対称）の近傍リストから引く．`#[derive(Clone)]` でスナップショット（save/resume）と
//! 比較実験に対応する．`agent_ids()` は `BTreeMap` のソート済みキーを返し決定論を
//! 保証する（socsim コア層）．

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use socsim_core::{AgentId, SimClock, WorldState};
use socsim_net::SocialNetwork;

use crate::norm::PersonalNorm;

/// 1 エージェントのプロフィール G（LLM プロンプトに渡す記述）．
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentProfile {
    /// G: 名前・性格・背景の自然言語記述（LLM プロンプト用）．
    pub description: String,
    /// 規範起業家か（初期に規範を保持し伝播を駆動する少数）．
    pub is_entrepreneur: bool,
    /// 選好（喫煙・大声・チップ等への賛否; 初期の価値観対立の源）．
    pub preferences: Vec<String>,
}

impl AgentProfile {
    /// プロフィールを作る．
    pub fn new(
        description: impl Into<String>,
        is_entrepreneur: bool,
        preferences: Vec<String>,
    ) -> Self {
        AgentProfile {
            description: description.into(),
            is_entrepreneur,
            preferences,
        }
    }
}

/// 当該ステップに発生した会話・観察の記録（PreStep でクリア）．
///
/// 伝播（[`crate::mechanisms::SpreadingMechanism`]）が生成し，評価・指標で集計する．
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InteractionEvent {
    /// 発信者（送信者 / 被観察者）．
    pub sender: AgentId,
    /// 受信者（会話相手 / 観察者）．
    pub receiver: AgentId,
    /// 観察（true）か会話（false）か．
    pub is_observation: bool,
    /// 当該イベントで衝突が検出されたか（`DetectConflict = T`）．
    pub conflict: bool,
    /// 受信者が識別した規範記述（識別されなければ None）．
    pub identified_content: Option<String>,
}

/// CRSEC の世界状態．
#[derive(Clone)]
pub struct CrsecWorld {
    /// シミュレーションクロック．
    pub clock: SimClock,
    /// 社会接続（誰が誰と会話・観察できるか）．無向の小世界網を既定とする．
    pub network: SocialNetwork,
    /// 各エージェントのプロフィール G（ソート済みキー）．
    pub agents: BTreeMap<AgentId, AgentProfile>,
    /// 各エージェントの個人規範DB（5つ組のリスト）．伝播・評価で更新される正本．
    pub norm_db: BTreeMap<AgentId, Vec<PersonalNorm>>,
    /// 当該ステップに発生した会話・観察ログ（PreStep でクリア）．
    pub interactions: Vec<InteractionEvent>,
    /// 長期統合の有用性閾値 θ．
    pub synth_threshold: f64,
}

impl CrsecWorld {
    /// 構成済みフィールドから世界状態を組み立てる（網生成・初期化は
    /// [`crate::simulation::init_world`]）．
    pub fn new(
        network: SocialNetwork,
        agents: BTreeMap<AgentId, AgentProfile>,
        norm_db: BTreeMap<AgentId, Vec<PersonalNorm>>,
        synth_threshold: f64,
        max_steps: u64,
    ) -> Self {
        CrsecWorld {
            clock: SimClock::new(max_steps),
            network,
            agents,
            norm_db,
            interactions: Vec::new(),
            synth_threshold,
        }
    }

    /// エージェント数 N．
    pub fn n(&self) -> usize {
        self.agents.len()
    }

    /// 当該エージェントの適格規範（`s_act && s_val`）の参照を列挙する．
    pub fn qualified_norms(&self, id: AgentId) -> Vec<&PersonalNorm> {
        self.norm_db
            .get(&id)
            .map(|v| v.iter().filter(|n| n.qualified()).collect())
            .unwrap_or_default()
    }

    /// 集団全体の適格規範を canonical key ごとに束ねた集合（ソート済み）を返す．
    ///
    /// 採用率・収束判定の基礎．LLM 記述の揺れを [`canonical_key`] で吸収する．
    pub fn qualified_canonical_set(&self) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        for norms in self.norm_db.values() {
            for n in norms.iter().filter(|n| n.qualified()) {
                let k = canonical_key(&n.content);
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        keys.sort();
        keys
    }
}

impl WorldState for CrsecWorld {
    fn agent_ids(&self) -> Vec<AgentId> {
        // BTreeMap のキーはソート済み．契約（sorted）を明示する．
        self.agents.keys().copied().collect()
    }

    fn clock(&self) -> &SimClock {
        &self.clock
    }

    fn clock_mut(&mut self) -> &mut SimClock {
        &mut self.clock
    }
}

// --------------------------------------------------------------------------- //
// canonical-norm-identity（決定論的・規則ベース）
// --------------------------------------------------------------------------- //

/// 英語ストップワード（canonical 正規化で除去する機能語）．
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "to", "of", "in", "on",
    "at", "for", "and", "or", "but", "with", "without", "you", "your", "we", "our", "they",
    "their", "should", "shall", "must", "ought", "do", "does", "not", "no", "people", "one",
    "everyone", "always", "never", "that", "this", "it", "its", "as", "by", "from", "when",
    "while", "than",
];

/// 規範記述 `c` を **決定論的** な canonical key へ正規化する．
///
/// 手順: 小文字化 → 英数字以外を空白へ → トークン化 → ストップワード除去 →
/// 重複除去 → 昇順ソート → スペース連結．LLM のパラフレーズ（語順・冠詞・主語の
/// 揺れ）を吸収しつつ，**追加の LLM 呼び出しを要しない**（二層決定論の下層に閉じる）．
///
/// 例: "People should not smoke indoors" と "No smoking inside, you must not"
///     はともに `key = "indoors inside smoke smoking"` 付近のキーへ寄る
///     （keyword 集合の重なりで束ねる）．
///
/// 完全一致を狙わず，content 語幹集合の一致で「同じ規範」を識別する保守的な規則．
/// より高精度な意味同定が要るなら [`crate::config::CanonicalMode::Llm`]（拡張点）．
pub fn canonical_key(content: &str) -> String {
    let lowered = content.to_ascii_lowercase();
    let mut tokens: Vec<String> = lowered
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .collect();
    tokens.sort();
    tokens.dedup();
    if tokens.is_empty() {
        // 全てストップワードに落ちた場合は元記述を素朴に正規化して退避する．
        return lowered.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    tokens.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_key_collapses_paraphrases() {
        let a = canonical_key("People should not smoke indoors.");
        let b = canonical_key("No smoking indoors!");
        // どちらも {smoke/smoking, indoors} に縮約され，キーワード集合が大きく重なる．
        assert!(a.contains("indoors"));
        assert!(b.contains("indoors"));
        assert!(a.contains("smoke"));
        assert!(b.contains("smoking"));
    }

    #[test]
    fn canonical_key_is_order_invariant() {
        assert_eq!(
            canonical_key("keep quiet library"),
            canonical_key("library quiet keep"),
        );
    }

    #[test]
    fn canonical_key_never_empty() {
        // 全てストップワードでも空文字にはしない．
        assert!(!canonical_key("the a is to").is_empty());
    }
}
