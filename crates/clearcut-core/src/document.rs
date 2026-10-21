// Written by Kone Tshivhinda & Claude
//! Where an uploaded document is in the pipeline (ADR-0006).
//!
//! There is deliberately no `failed` status: a document ends `Done` or
//! `NeedsReview` (ADR-0005), so a person always has somewhere to look.

use serde::{Deserialize, Serialize};

/// The text forms are stored in the database and sent in API responses, so
/// renaming one is a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentStatus {
    /// Stored, not yet being worked on.
    Received,
    /// A job is working on it.
    Processing,
    /// A person needs to look at it.
    NeedsReview,
    /// Read, validated and turned into events.
    Done,
}

impl DocumentStatus {
    pub const ALL: [DocumentStatus; 4] = [
        DocumentStatus::Received,
        DocumentStatus::Processing,
        DocumentStatus::NeedsReview,
        DocumentStatus::Done,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            DocumentStatus::Received => "received",
            DocumentStatus::Processing => "processing",
            DocumentStatus::NeedsReview => "needs_review",
            DocumentStatus::Done => "done",
        }
    }

    /// Parses exactly one status name: no trimming, no case folding.
    pub fn parse(text: &str) -> Option<DocumentStatus> {
        DocumentStatus::ALL
            .into_iter()
            .find(|status| status.as_str() == text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_statuses_and_their_stored_names_are_exactly_the_agreed_list() {
        // ADR-0006. Stored in the database and sent in API responses.
        let expected = [
            (DocumentStatus::Received, "received"),
            (DocumentStatus::Processing, "processing"),
            (DocumentStatus::NeedsReview, "needs_review"),
            (DocumentStatus::Done, "done"),
        ];
        assert_eq!(DocumentStatus::ALL.len(), expected.len());
        for (status, name) in expected {
            assert!(DocumentStatus::ALL.contains(&status));
            assert_eq!(status.as_str(), name);
            assert_eq!(DocumentStatus::parse(name), Some(status));
            assert_eq!(serde_json::to_string(&status).unwrap(), format!("\"{name}\""));
        }
    }

    #[test]
    fn there_is_no_failed_status_and_parsing_is_exact() {
        for bad in ["failed", "", "Done", " done", "done ", "needs-review"] {
            assert_eq!(DocumentStatus::parse(bad), None, "{bad:?}");
        }
    }
}
