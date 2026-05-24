//! 個人規範の5つ組 `n = <c, u, α, s_act, s_val>`（Ren et al. 2024, Section 2.1）．
//!
//! 規範ライフサイクル（創出→表現→伝播→評価→遵守）が更新する正本のデータ型．
//! 伝播で識別された規範は **未適格**（`s_act = false`, `s_val = false`）で格納され，
//! 評価（サニティ検査）を通過して初めて **適格**（`qualified()`）へ昇格する．
//! 社会規範の創発は，集団全体での適格規範の共有として測定する（[`crate::metrics`]）．

use serde::{Deserialize, Serialize};

/// 規範の型 α（Cialdini らの focus theory）．
///
/// - `Descriptive`（記述的）: 「皆がこうしている」という観察された規則性．
/// - `Injunctive`（命令的）: 「こうすべき」という是認・否認を伴う規則．
///
/// 論文 Fact 7 の順序効果（injunctive が descriptive より先に創発する）の解析に使う．
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NormType {
    /// 記述的規範（観察された規則性; "people tip the waiter"）．
    Descriptive,
    /// 命令的規範（是認・否認を伴う規則; "one should not smoke indoors"）．
    Injunctive,
}

impl NormType {
    /// 出力・キャッシュキー用の安定ラベル（"des" / "inj"）．
    pub fn label(&self) -> &'static str {
        match self {
            NormType::Descriptive => "des",
            NormType::Injunctive => "inj",
        }
    }

    /// LLM 出力文字列から型を推定する（規則ベース; 決定論的）．
    ///
    /// "inj" / "injunctive" / "should" / "ought" を含めば命令的，それ以外は記述的．
    pub fn parse_loose(s: &str) -> NormType {
        let l = s.trim().to_ascii_lowercase();
        if l.contains("inj") || l.contains("should") || l.contains("ought") || l.contains("must") {
            NormType::Injunctive
        } else {
            NormType::Descriptive
        }
    }
}

/// 個人規範の5つ組 `n = <c, u, α, s_act, s_val>`（論文 Section 2.1）．
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalNorm {
    /// c: 自然言語記述（例 "no smoking indoors"）．
    pub content: String,
    /// u: 有用性 ∈ `[1, 100]`．
    pub utility: u8,
    /// α: 型（記述的 / 命令的）．
    pub alpha: NormType,
    /// s_act: 活性（active）．
    pub s_act: bool,
    /// s_val: 有効（valid）．
    pub s_val: bool,
}

impl PersonalNorm {
    /// 新しい規範を作る（活性・有効を明示）．
    pub fn new(
        content: impl Into<String>,
        utility: u8,
        alpha: NormType,
        s_act: bool,
        s_val: bool,
    ) -> Self {
        PersonalNorm {
            content: content.into(),
            utility: utility.clamp(1, 100),
            alpha,
            s_act,
            s_val,
        }
    }

    /// 起業家が創出する初期規範（即座に適格 = 活性かつ有効）．
    pub fn created(content: impl Into<String>, utility: u8, alpha: NormType) -> Self {
        Self::new(content, utility, alpha, true, true)
    }

    /// 伝播で識別された規範（未適格 = 活性 false・有効 false; 評価待ち）．
    pub fn identified(content: impl Into<String>, utility: u8, alpha: NormType) -> Self {
        Self::new(content, utility, alpha, false, false)
    }

    /// 適格 = 活性 ∧ 有効（`s_act && s_val`）．社会規範の創発はこの集合で測る．
    pub fn qualified(&self) -> bool {
        self.s_act && self.s_val
    }

    /// 評価を通過させ適格へ昇格する（`s_act = true`, `s_val = true`）．
    pub fn promote(&mut self) {
        self.s_act = true;
        self.s_val = true;
    }

    /// 長期統合で抽象規範に吸収された元規範を非活性化する（`s_act = false`）．
    pub fn deactivate(&mut self) {
        self.s_act = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualified_requires_both_flags() {
        assert!(PersonalNorm::created("c", 50, NormType::Injunctive).qualified());
        assert!(!PersonalNorm::identified("c", 50, NormType::Injunctive).qualified());
        let mut n = PersonalNorm::new("c", 50, NormType::Descriptive, true, false);
        assert!(!n.qualified());
        n.promote();
        assert!(n.qualified());
    }

    #[test]
    fn utility_is_clamped() {
        assert_eq!(
            PersonalNorm::created("c", 0, NormType::Descriptive).utility,
            1
        );
        assert_eq!(
            PersonalNorm::created("c", 200, NormType::Descriptive).utility,
            100
        );
    }

    #[test]
    fn norm_type_parse_loose() {
        assert_eq!(NormType::parse_loose("Injunctive"), NormType::Injunctive);
        assert_eq!(NormType::parse_loose("one should..."), NormType::Injunctive);
        assert_eq!(NormType::parse_loose("descriptive"), NormType::Descriptive);
        assert_eq!(
            NormType::parse_loose("people tend to"),
            NormType::Descriptive
        );
    }
}
