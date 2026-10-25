// Written by Kone Tshivhinda & Claude
//! What a document and a month may spend (ADR-0011 budgets, ADR-0018 section 6).
//!
//! The rule is the promise of ADR-0011: **a call that would break a budget does not happen.**
//! Before each call the router asks [`BudgetCheck::check`] with the call's worst case; after
//! it, it records what the call really cost. The document's cap grows with its pages (a flat
//! R5 would stop a 300-page statement before any escalation), and is never allowed to carry the
//! document past what is left of the month.

use super::Cost;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// Which limit a refused call would have broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetScope {
    Document,
    Month,
}

impl BudgetScope {
    pub fn as_str(self) -> &'static str {
        match self {
            BudgetScope::Document => "document",
            BudgetScope::Month => "month",
        }
    }
}

/// The amounts, set by the operator and visible to the tenant. **Read back from text, they are
/// checked** like amounts built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "UncheckedBudget")]
pub struct BudgetPolicy {
    per_document: Cost,
    per_page: Cost,
    per_month: Cost,
}

#[derive(Deserialize)]
struct UncheckedBudget {
    per_document: Cost,
    per_page: Cost,
    per_month: Cost,
}

impl TryFrom<UncheckedBudget> for BudgetPolicy {
    type Error = Error;

    fn try_from(raw: UncheckedBudget) -> Result<Self> {
        BudgetPolicy::new(raw.per_document, raw.per_page, raw.per_month)
    }
}

impl BudgetPolicy {
    /// Every amount must be above zero.
    pub fn new(per_document: Cost, per_page: Cost, per_month: Cost) -> Result<BudgetPolicy> {
        if per_document == Cost::ZERO || per_page == Cost::ZERO || per_month == Cost::ZERO {
            return Err(Error::InvalidPricing("a budget amount must be above zero"));
        }
        Ok(BudgetPolicy {
            per_document,
            per_page,
            per_month,
        })
    }

    pub fn per_document(&self) -> Cost {
        self.per_document
    }

    pub fn per_page(&self) -> Cost {
        self.per_page
    }

    pub fn per_month(&self) -> Cost {
        self.per_month
    }

    /// The most a document of `pages` pages may spend: the larger of the flat amount and so much
    /// a page.
    pub fn document_cap(&self, pages: u32) -> Cost {
        self.per_document
            .max(self.per_page.saturating_mul(u64::from(pages)))
    }
}

impl Default for BudgetPolicy {
    /// R5 a document (Kone, ADR-0011), R0.50 a page for longer ones (ADR-0018), R100 a month.
    fn default() -> Self {
        BudgetPolicy {
            per_document: Cost::from_rand_cents(500),
            per_page: Cost::from_rand_cents(50),
            per_month: Cost::from_rand_cents(10_000),
        }
    }
}

/// The spending of one document against its cap and the month. Plain data, so a job can keep it
/// in a checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetCheck {
    document_cap: Cost,
    month_budget: Cost,
    document_spent: Cost,
    /// What the tenant had spent this month before this document began.
    month_spent_before: Cost,
}

impl BudgetCheck {
    pub fn new(policy: &BudgetPolicy, pages: u32, month_spent_before: Cost) -> BudgetCheck {
        BudgetCheck {
            document_cap: policy.document_cap(pages),
            month_budget: policy.per_month(),
            document_spent: Cost::ZERO,
            month_spent_before,
        }
    }

    /// May a call whose worst case is `worst` be made? The document is named first when both
    /// limits would be broken. Exactly reaching a limit is allowed.
    pub fn check(&self, worst: Cost) -> std::result::Result<(), BudgetScope> {
        if self.document_spent.saturating_add(worst) > self.document_cap {
            return Err(BudgetScope::Document);
        }
        if self.month_spent().saturating_add(worst) > self.month_budget {
            return Err(BudgetScope::Month);
        }
        Ok(())
    }

    /// Adds what a call really cost.
    pub fn record(&mut self, actual: Cost) {
        self.document_spent = self.document_spent.saturating_add(actual);
    }

    pub fn document_spent(&self) -> Cost {
        self.document_spent
    }

    /// What the document may still spend under its own cap.
    pub fn document_remaining(&self) -> Cost {
        self.document_cap.saturating_sub(self.document_spent)
    }

    /// What the month still has, counting this document's spending.
    pub fn month_remaining(&self) -> Cost {
        self.month_budget.saturating_sub(self.month_spent())
    }

    fn month_spent(&self) -> Cost {
        self.month_spent_before.saturating_add(self.document_spent)
    }
}
