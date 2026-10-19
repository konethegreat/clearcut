// Written by Kone Tshivhinda & Claude
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// Invoice lifecycle. Transitions are explicit so every change is auditable.
///
/// ```text
/// Draft -> PendingApproval -> Approved -> Sent -> PartiallyPaid -> Paid
///   |            |              |         |  \________^            
///   +------------+--------------+---------+--> Void   Sent -> Overdue -> (Paid | PartiallyPaid | Void)
/// ```
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    Draft,
    PendingApproval,
    Approved,
    Sent,
    PartiallyPaid,
    Overdue,
    Paid,
    Void,
}

impl InvoiceStatus {
    pub fn can_transition_to(self, to: InvoiceStatus) -> bool {
        use InvoiceStatus::*;
        matches!(
            (self, to),
            (Draft, PendingApproval)
                | (Draft, Approved) // auto-approve under policy threshold
                | (PendingApproval, Approved)
                | (PendingApproval, Draft) // rejected back for edits
                | (Approved, Sent)
                | (Sent, PartiallyPaid)
                | (Sent, Paid)
                | (Sent, Overdue)
                | (PartiallyPaid, Paid)
                | (PartiallyPaid, Overdue)
                | (Overdue, PartiallyPaid)
                | (Overdue, Paid)
                | (Draft | PendingApproval | Approved | Sent | Overdue, Void)
        )
    }

    pub fn transition(self, to: InvoiceStatus) -> Result<InvoiceStatus> {
        if self.can_transition_to(to) {
            Ok(to)
        } else {
            Err(Error::InvalidTransition { from: self, to })
        }
    }

    /// Once sent, an invoice is immutable: corrections go through credit/debit notes.
    pub fn is_editable(self) -> bool {
        matches!(self, InvoiceStatus::Draft)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use InvoiceStatus::*;

    #[test]
    fn happy_path() {
        let s = Draft
            .transition(PendingApproval)
            .unwrap()
            .transition(Approved)
            .unwrap()
            .transition(Sent)
            .unwrap()
            .transition(Paid)
            .unwrap();
        assert_eq!(s, Paid);
    }

    #[test]
    fn cannot_skip_approval_to_sent() {
        assert!(Draft.transition(Sent).is_err());
    }

    #[test]
    fn paid_is_terminal() {
        assert!(Paid.transition(Void).is_err());
        assert!(Paid.transition(Sent).is_err());
    }

    #[test]
    fn only_drafts_are_editable() {
        assert!(Draft.is_editable());
        assert!(!Sent.is_editable());
    }
}
