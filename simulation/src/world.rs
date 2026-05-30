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

    /// [`qualified_canonical_set`](Self::qualified_canonical_set) の canonicalizer 版．
    ///
    /// rule モードの [`Canonicalizer`] を渡すと `qualified_canonical_set` と **バイト
    /// 等価**（同じ `canonical_key` 委譲）．llm モードでは LLM 意味判定で束ねた集合を返す．
    pub fn qualified_canonical_set_with(&self, canon: &Canonicalizer<'_>) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        for norms in self.norm_db.values() {
            for n in norms.iter().filter(|n| n.qualified()) {
                let k = canon.canonicalize(&n.content);
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

// --------------------------------------------------------------------------- //
// Canonicalizer（規範同定の方式を抽象化; rule = 決定論 / llm = 意味判定）
// --------------------------------------------------------------------------- //

use std::cell::RefCell;

use crate::config::CanonicalMode;

/// 規範記述を canonical key へ束ねる戦略（`--canonical-mode {rule|llm}`）．
///
/// - `Rule`（既定）: [`canonical_key`] への純粋な委譲．LLM を一切呼ばず，二層決定論の
///   下層に閉じる．`canonicalize` の出力は `canonical_key(content)` と **バイト等価**．
/// - `Llm`: LLM が «二つの規範表現が同じ規範か» を判定する意味的同定（キャッシュ付き）．
///   既出の代表規範のリストを保持し，新しい記述がどれかと同義なら **その代表の
///   rule-key を再利用** する（語彙が重ならないパラフレーズも 1 規範に束ねられる）．
///   どれとも異なれば新たな代表として登録する．rule 既定の挙動には一切影響しない．
///
/// «二つの規範記述 a, b が同じ規範か» を返す判定器（llm モードのみ呼ばれる）．
pub type NormJudge<'a> = Box<dyn Fn(&str, &str) -> bool + 'a>;

/// `Llm` モードは内部に «代表記述のリスト» を可変状態として持つため `RefCell` で包む．
/// 判定 LLM 呼び出しはクロージャ（`judge`）として注入する（テストでは scripted mock，
/// 本番ではキャッシュ付き live クライアント）．rule モードでは `judge` を一切呼ばない．
pub struct Canonicalizer<'a> {
    mode: CanonicalMode,
    /// 既出の代表 `(代表記述, その rule-key)`（llm モードのみ使用）．
    registry: RefCell<Vec<(String, String)>>,
    /// «a と b は同じ規範か» を返す判定器（llm モードのみ呼ばれる）．
    judge: Option<NormJudge<'a>>,
}

impl<'a> Canonicalizer<'a> {
    /// 規則ベース（決定論）の canonicalizer を作る（LLM 不使用; 既定）．
    pub fn rule() -> Self {
        Canonicalizer {
            mode: CanonicalMode::Deterministic,
            registry: RefCell::new(Vec::new()),
            judge: None,
        }
    }

    /// LLM 判定ベースの canonicalizer を作る．`judge(a, b)` が «同じ規範» を返す．
    pub fn llm(judge: impl Fn(&str, &str) -> bool + 'a) -> Self {
        Canonicalizer {
            mode: CanonicalMode::Llm,
            registry: RefCell::new(Vec::new()),
            judge: Some(Box::new(judge)),
        }
    }

    /// 設定の [`CanonicalMode`] に応じて構築する（llm 時は `judge` を使用）．
    pub fn from_mode(mode: CanonicalMode, judge: impl Fn(&str, &str) -> bool + 'a) -> Self {
        match mode {
            CanonicalMode::Deterministic => Self::rule(),
            CanonicalMode::Llm => Self::llm(judge),
        }
    }

    /// モードラベル（"deterministic" / "llm"）．
    pub fn mode(&self) -> CanonicalMode {
        self.mode
    }

    /// 記述 `content` の canonical key を返す．
    ///
    /// rule モードでは `canonical_key(content)` をそのまま返す（バイト等価）．llm モード
    /// では，既出代表のいずれかと «同じ規範» と判定されればその代表の rule-key を返し，
    /// どれとも異なれば content 自身を新代表として登録しその rule-key を返す．
    pub fn canonicalize(&self, content: &str) -> String {
        if self.mode == CanonicalMode::Deterministic {
            return canonical_key(content);
        }
        let own_key = canonical_key(content);
        // まず rule-key 一致（自明な同義）を高速判定する（LLM 不要）．
        {
            let reg = self.registry.borrow();
            for (_rep, key) in reg.iter() {
                if *key == own_key {
                    return key.clone();
                }
            }
        }
        // 次に LLM 判定で既出代表との同義を確かめる．
        if let Some(judge) = &self.judge {
            let reps: Vec<(String, String)> = self.registry.borrow().clone();
            for (rep, key) in reps.iter() {
                if judge(rep, content) {
                    return key.clone();
                }
            }
        }
        // どれとも異なる → 新代表として登録（rule-key をキーに使う）．
        self.registry
            .borrow_mut()
            .push((content.to_string(), own_key.clone()));
        own_key
    }
}

#[cfg(test)]
mod canonicalizer_tests {
    use super::*;

    #[test]
    fn rule_mode_is_byte_identical_to_canonical_key() {
        let c = Canonicalizer::rule();
        for s in [
            "People should not smoke indoors.",
            "keep quiet in the library",
            "the a is to",
        ] {
            assert_eq!(c.canonicalize(s), canonical_key(s));
        }
    }

    #[test]
    fn llm_mode_merges_lexically_disjoint_paraphrases() {
        // 語彙が重ならない 2 記述を «同じ規範» と判定する mock．
        let c = Canonicalizer::llm(|a: &str, b: &str| {
            let smoke =
                |s: &str| s.contains("smok") || s.contains("cigarette") || s.contains("tobacco");
            smoke(a) && smoke(b)
        });
        let k1 = c.canonicalize("no smoking indoors");
        // 語彙が全く異なるが同義 → 同じ代表 key に束ねられる．
        let k2 = c.canonicalize("please refrain from cigarettes inside");
        assert_eq!(k1, k2);
        // 無関係な規範は別 key．
        let k3 = c.canonicalize("keep the library quiet");
        assert_ne!(k1, k3);
    }

    #[test]
    fn llm_mode_falls_back_to_rule_key_on_exact_match() {
        // judge が常に false でも rule-key 一致は束ねる（LLM コスト節約）．
        let c = Canonicalizer::llm(|_a: &str, _b: &str| false);
        let k1 = c.canonicalize("no smoking indoors");
        let k2 = c.canonicalize("indoors, no smoking");
        assert_eq!(k1, k2);
    }
}
