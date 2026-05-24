//! 評価指標（論文 Section 3.2 / Fig. 2）．
//!
//! 規範創発の定量化に用いる．中心は **adoption_rate**（最頻 canonical 規範を適格
//! 規範として持つエージェントの割合）と **compliance_rate**（その規範を遵守した
//! 割合），社会的衝突数 **n_conflicts**，相異なる規範数 **n_distinct_norms**．
//! `adoption_rate` は LLM のパラフレーズを吸収するため canonical key で束ねて測る
//! （[`crate::world::canonical_key`]）．

use std::collections::BTreeMap;

use serde::Serialize;
use socsim_core::AgentId;

use crate::norm::PersonalNorm;
use crate::world::canonical_key;

/// 集団の規範DBから «最頻 canonical 規範を適格として持つ割合» を返す（採用率）．
///
/// 各エージェントが適格として持つ canonical key 集合を集め，最も多くのエージェントに
/// 共有された key の保有率を返す（論文の「ある行動基準 c の採用率」の最大値）．集団が
/// 空 or 適格規範皆無なら 0．併せてその最頻 key も返す．
pub fn adoption_rate(norm_db: &BTreeMap<AgentId, Vec<PersonalNorm>>) -> (f64, Option<String>) {
    let n = norm_db.len();
    if n == 0 {
        return (0.0, None);
    }
    // canonical key → それを適格として持つエージェント数．
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for norms in norm_db.values() {
        let mut keys: Vec<String> = norms
            .iter()
            .filter(|x| x.qualified())
            .map(|x| canonical_key(&x.content))
            .collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            *counts.entry(k).or_insert(0) += 1;
        }
    }
    match counts.iter().max_by_key(|(_, &c)| c) {
        Some((key, &c)) => (c as f64 / n as f64, Some(key.clone())),
        None => (0.0, None),
    }
}

/// 遵守率: 当該ラウンドで規範を遵守したと判定されたエージェントの割合．
pub fn compliance_rate(complied: usize, n: usize) -> f64 {
    if n == 0 {
        0.0
    } else {
        complied as f64 / n as f64
    }
}

/// 集団全体の相異なる canonical 規範数（適格規範のみ; 多様性・収束の指標）．
pub fn n_distinct_norms(norm_db: &BTreeMap<AgentId, Vec<PersonalNorm>>) -> usize {
    let mut keys: Vec<String> = Vec::new();
    for norms in norm_db.values() {
        for x in norms.iter().filter(|x| x.qualified()) {
            let k = canonical_key(&x.content);
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
    }
    keys.len()
}

/// 1 ラウンド分のメトリクス（metrics.csv の 1 行）．
#[derive(Debug, Clone, Serialize)]
pub struct Metrics {
    /// ラウンド番号 t．
    pub t: usize,
    /// 採用率 ∈ [0,1]（最頻 canonical 規範の適格保有率）．
    pub adoption_rate: f64,
    /// 遵守率 ∈ [0,1]．
    pub compliance_rate: f64,
    /// 当該ラウンドに検出された社会的衝突数（`DetectConflict = T` の件数）．
    pub n_conflicts: usize,
    /// 相異なる canonical 規範数（適格のみ）．
    pub n_distinct_norms: usize,
    /// 適格規範を 1 件以上持つエージェント数．
    pub n_qualified_holders: usize,
}

impl Metrics {
    /// 集団状態からメトリクスを計算する．
    pub fn compute(
        norm_db: &BTreeMap<AgentId, Vec<PersonalNorm>>,
        complied: usize,
        n_conflicts: usize,
        t: usize,
    ) -> Self {
        let (adoption, _key) = adoption_rate(norm_db);
        let n = norm_db.len();
        let holders = norm_db
            .values()
            .filter(|norms| norms.iter().any(|x| x.qualified()))
            .count();
        Metrics {
            t,
            adoption_rate: adoption,
            compliance_rate: compliance_rate(complied, n),
            n_conflicts,
            n_distinct_norms: n_distinct_norms(norm_db),
            n_qualified_holders: holders,
        }
    }
}

/// 創発時刻の推定: 採用率が `threshold` 以上になった最初のラウンド．
///
/// `adoptions` は各ラウンドの採用率列（t 昇順）．達しなければ `None`．
pub fn time_to_emergence(adoptions: &[f64], threshold: f64) -> Option<usize> {
    adoptions.iter().position(|&a| a >= threshold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::norm::NormType;

    fn db(rows: &[(u64, &[(&str, bool)])]) -> BTreeMap<AgentId, Vec<PersonalNorm>> {
        let mut m = BTreeMap::new();
        for &(id, norms) in rows {
            let v = norms
                .iter()
                .map(|&(c, q)| PersonalNorm::new(c, 50, NormType::Injunctive, q, q))
                .collect();
            m.insert(AgentId(id), v);
        }
        m
    }

    #[test]
    fn adoption_rate_uses_canonical_key() {
        // 3 名が同じ規範を語順・冠詞・主語の揺れで適格保有 → canonical 同定で
        // 1 規範に束ねられ採用率 1.0（決定論的 keyword-set 正規化が吸収する範囲）．
        let m = db(&[
            (0, &[("No smoking indoors", true)]),
            (1, &[("indoors, no smoking", true)]),
            (2, &[("you should not be smoking indoors", true)]),
        ]);
        let (rate, key) = adoption_rate(&m);
        assert!((rate - 1.0).abs() < 1e-12, "rate={rate}");
        assert!(key.unwrap().contains("smoking"));
    }

    #[test]
    fn unqualified_norms_excluded() {
        let m = db(&[(0, &[("a rule", false)]), (1, &[("a rule", true)])]);
        let (rate, _) = adoption_rate(&m);
        assert!((rate - 0.5).abs() < 1e-12);
        assert_eq!(n_distinct_norms(&m), 1);
    }

    #[test]
    fn time_to_emergence_finds_first() {
        assert_eq!(time_to_emergence(&[0.2, 0.5, 0.9, 1.0], 0.9), Some(2));
        assert_eq!(time_to_emergence(&[0.2, 0.5], 0.9), None);
    }
}
