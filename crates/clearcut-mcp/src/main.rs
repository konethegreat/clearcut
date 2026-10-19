// Written by Kone Tshivhinda & Claude
//! ClearCut MCP server (stdio). Exposes invoicing tools to AI agents.
//! Streamable HTTP transport and auth arrive in Phase 1 (see docs/PHASES.md).

use clearcut_core::{
    compliance_za::{self, TaxInvoiceKind},
    Invoice, InvoiceStatus, InvoiceTotals, Money,
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router, Json, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
struct ClearCut;

#[derive(Serialize, Deserialize, JsonSchema)]
struct Calculation {
    totals: InvoiceTotals,
    required_document: TaxInvoiceKind,
    display_gross: String,
}

#[derive(Deserialize, JsonSchema)]
struct GrossParams {
    /// VAT-inclusive total in ZAR cents (R5 000.00 = 500000).
    gross_minor: i64,
}

#[derive(Serialize, JsonSchema)]
struct RequiredDocument {
    kind: TaxInvoiceKind,
}

#[derive(Deserialize, JsonSchema)]
struct TransitionParams {
    from: InvoiceStatus,
    to: InvoiceStatus,
}

#[derive(Serialize, JsonSchema)]
struct TransitionResult {
    allowed: bool,
    editable_after: bool,
}

#[tool_router]
impl ClearCut {
    fn new() -> Self {
        Self
    }

    /// Calculate net, VAT and gross totals for a draft invoice. Amounts are integer minor units (cents).
    #[tool(name = "calculate_invoice")]
    async fn calculate_invoice(
        &self,
        Parameters(invoice): Parameters<Invoice>,
    ) -> Result<Json<Calculation>, String> {
        let totals = invoice.totals().map_err(|e| e.to_string())?;
        Ok(Json(Calculation {
            required_document: compliance_za::required_kind(totals.gross),
            display_gross: totals.gross.to_string(),
            totals,
        }))
    }

    /// Which South African tax invoice is required for a VAT-inclusive total: full (over R5 000), abridged, or none (R50 or less).
    #[tool(name = "za_required_tax_invoice")]
    async fn za_required_tax_invoice(
        &self,
        Parameters(p): Parameters<GrossParams>,
    ) -> Json<RequiredDocument> {
        Json(RequiredDocument {
            kind: compliance_za::required_kind(Money::zar(p.gross_minor)),
        })
    }

    /// Check whether an invoice may move between two lifecycle states.
    #[tool(name = "check_status_transition")]
    async fn check_status_transition(
        &self,
        Parameters(p): Parameters<TransitionParams>,
    ) -> Json<TransitionResult> {
        Json(TransitionResult {
            allowed: p.from.can_transition_to(p.to),
            editable_after: p.to.is_editable(),
        })
    }
}

#[tool_handler]
impl ServerHandler for ClearCut {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("clearcut", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Invoicing tools for South African VAT. Amounts are integer cents; never use floats.",
            )
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = ClearCut::new().serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
