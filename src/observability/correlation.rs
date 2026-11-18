//! Correlation ids for one tool call.
//!
//! A tool call is the unit a client sees, and one call can make several Discord
//! requests — resolving a channel's guild and then the operation itself. One id
//! per call ties those records together in the logs and in audit records.
//!
//! The id travels in a task-local rather than through every function signature:
//! the gate is many layers below the tool, and threading an argument through it
//! would make every caller carry a value only the audit trail uses.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::Instrument;

/// Id counter. Process-wide and monotonic, so two records with the same id in one
/// log belong to the same call.
static NEXT: AtomicU64 = AtomicU64::new(1);

tokio::task_local! {
    static CURRENT: u64;
}

/// Allocate the next id.
pub fn next_id() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The id of the tool call this task is serving, if it is serving one.
pub fn current() -> Option<u64> {
    CURRENT.try_with(|id| *id).ok()
}

/// Run `future` as one correlated tool call.
///
/// The span carries the tool name and the id, so every record emitted inside —
/// including audit records from the gate — can be traced back to the call.
pub async fn scope_tool<F: Future>(tool: &'static str, future: F) -> F::Output {
    let correlation = next_id();
    let span = tracing::info_span!("mcp.tool", tool, correlation);
    CURRENT.scope(correlation, future.instrument(span)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn each_tool_call_gets_its_own_id() {
        let first = scope_tool("test", async { current() }).await;
        let second = scope_tool("test", async { current() }).await;

        assert!(first.is_some(), "a tool call must be correlated");
        assert_ne!(first, second, "two calls must not share an id");
    }

    #[tokio::test]
    async fn work_outside_a_tool_call_is_uncorrelated() {
        assert_eq!(current(), None);
    }

    #[tokio::test]
    async fn an_id_survives_an_await_point() {
        let correlation = scope_tool("test", async {
            tokio::task::yield_now().await;
            current()
        })
        .await;

        assert!(correlation.is_some(), "the id must outlive a yield");
    }
}
