//! Offline, bounded audit of saved OpenAI Agents API root-turn items.
//!
//! A completed root turn does NOT establish that its tools succeeded.
//! No network transport, credentials, user consent, artifact download, import,
//! or publication is implemented here. The trusted caller must bind every
//! response to its authenticated session and exact root turn.
use std::collections::HashSet;

use infimount_core::CoreError;
use serde::Deserialize;
use serde_json::Value;

use crate::openai_hosted_events::HostedSessionObservedOutcome;

pub const MAX_HOSTED_ITEM_PAGE_JSON_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_HOSTED_ITEM_PAGES: usize = 50;
pub const MAX_HOSTED_ITEMS: usize = 5_000;
pub const MAX_HOSTED_ITEMS_PER_PAGE: usize = 100;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedItemPage {
    object: String,
    data: Vec<Value>,
    first_id: Option<String>,
    last_id: Option<String>,
    has_more: bool,
}

/// A structural, offline assessment, NOT proof of provider or artifact success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostedSavedItemAuditSummary {
    pub pages: usize,
    pub items: usize,
    pub command_executions: usize,
}

/// Incremental list auditor. No model-provided bytes or outputs are retained.
/// A malformed page permanently poisons this auditor.
pub struct HostedSavedItemAudit {
    expected_root_turn_id: String,
    next_after: Option<String>,
    pages: usize,
    items: usize,
    command_executions: usize,
    item_ids: HashSet<String>,
    exhausted: bool,
    failed: bool,
}

impl HostedSavedItemAudit {
    pub fn new(expected_root_turn_id: &str) -> Result<Self, CoreError> {
        if !safe_opaque_id(expected_root_turn_id) {
            return Err(audit_error());
        }
        Ok(Self {
            expected_root_turn_id: expected_root_turn_id.to_owned(),
            next_after: None,
            pages: 0,
            items: 0,
            command_executions: 0,
            item_ids: HashSet::new(),
            exhausted: false,
            failed: false,
        })
    }

    /// The next request MUST use this value as the provider's after query.
    /// The trusted transport must also pin order=asc, session and turn.
    pub fn next_after(&self) -> Option<&str> {
        self.next_after.as_deref()
    }

    /// requested_after is the cursor actually used to fetch this page.
    /// The JSON is untrusted even when the transport is authenticated.
    pub fn observe_page(
        &mut self,
        page_json: &[u8],
        requested_after: Option<&str>,
    ) -> Result<(), CoreError> {
        if self.failed
            || self.exhausted
            || self.pages >= MAX_HOSTED_ITEM_PAGES
            || self.next_after.as_deref() != requested_after
            || page_json.is_empty()
            || page_json.len() > MAX_HOSTED_ITEM_PAGE_JSON_BYTES
        {
            return self.reject();
        }
        let page: SavedItemPage = match serde_json::from_slice(page_json) {
            Ok(page) => page,
            Err(_) => return self.reject(),
        };
        if page.object != "list"
            || page.data.is_empty()
            || page.data.len() > MAX_HOSTED_ITEMS_PER_PAGE
            || self.items.saturating_add(page.data.len()) > MAX_HOSTED_ITEMS
        {
            return self.reject();
        }

        let mut page_ids = HashSet::new();
        let mut new_commands = 0_usize;
        for (index, item) in page.data.iter().enumerate() {
            let Some(id) = item.get("id").and_then(Value::as_str) else {
                return self.reject();
            };
            if !safe_opaque_id(id)
                || self.item_ids.contains(id)
                || !page_ids.insert(id.to_owned())
                || item.get("turn_id").and_then(Value::as_str)
                    != Some(self.expected_root_turn_id.as_str())
            {
                return self.reject();
            }
            if index == 0 && page.first_id.as_deref() != Some(id) {
                return self.reject();
            }
            if index == page.data.len() - 1 && page.last_id.as_deref() != Some(id) {
                return self.reject();
            }
            match item.get("type").and_then(Value::as_str) {
                // The constrained C2.1 adapter does not provision external
                // MCP, web search, functions, computer use or subagent tools.
                Some("message") => {
                    if item.get("status").and_then(Value::as_str) != Some("completed") {
                        return self.reject();
                    }
                }
                Some("reasoning") => {
                    if !item
                        .get("status")
                        .is_none_or(|value| value.is_null() || value.as_str() == Some("completed"))
                    {
                        return self.reject();
                    }
                }
                Some("command_execution") => {
                    if item.get("status").and_then(Value::as_str) != Some("completed")
                        || item.get("exit_code").and_then(Value::as_i64) != Some(0)
                    {
                        return self.reject();
                    }
                    new_commands += 1;
                }
                _ => return self.reject(),
            }
        }
        if page.has_more && page.last_id.as_deref().is_none_or(|id| !safe_opaque_id(id)) {
            return self.reject();
        }
        self.item_ids.extend(page_ids);
        self.items += page.data.len();
        self.command_executions += new_commands;
        self.pages += 1;
        if page.has_more {
            self.next_after = page.last_id;
        } else {
            self.next_after = None;
            self.exhausted = true;
        }
        Ok(())
    }

    /// A structural gate only. Even a positive summary MUST NOT trigger an
    /// artifact import or publication without separate trusted byte checks.
    pub fn finish(
        &self,
        outcome: HostedSessionObservedOutcome,
    ) -> Result<HostedSavedItemAuditSummary, CoreError> {
        if self.failed
            || !self.exhausted
            || self.pages == 0
            || self.items == 0
            || outcome != HostedSessionObservedOutcome::RootTurnCompletedUnverified
        {
            return Err(audit_error());
        }
        Ok(HostedSavedItemAuditSummary {
            pages: self.pages,
            items: self.items,
            command_executions: self.command_executions,
        })
    }

    fn reject(&mut self) -> Result<(), CoreError> {
        self.failed = true;
        Err(audit_error())
    }
}

fn safe_opaque_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn audit_error() -> CoreError {
    CoreError::Config(
        "OpenAI-hosted saved turn items did not pass the bounded offline audit".into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TURN: &str = "turn_123";
    const ROOT: HostedSessionObservedOutcome =
        HostedSessionObservedOutcome::RootTurnCompletedUnverified;

    fn message(id: &str) -> Value {
        json!({
            "id": id,
            "turn_id": TURN,
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "phase": "final_answer",
            "content": []
        })
    }

    fn command(id: &str, code: Value) -> Value {
        json!({
            "id": id,
            "turn_id": TURN,
            "type": "command_execution",
            "status": "completed",
            "exit_code": code,
            "command": "printf ok",
            "output": "ok"
        })
    }

    fn page(items: Vec<Value>, more: bool) -> Vec<u8> {
        let first = items.first().and_then(|i| i.get("id")).cloned();
        let last = items.last().and_then(|i| i.get("id")).cloned();
        serde_json::to_vec(&json!({
            "object": "list",
            "data": items,
            "first_id": first,
            "last_id": last,
            "has_more": more
        }))
        .unwrap()
    }

    #[test]
    fn completed_items_are_only_an_offline_audit_not_a_publish_grant() {
        let mut audit = HostedSavedItemAudit::new(TURN).unwrap();
        assert!(audit.finish(ROOT).is_err());
        audit
            .observe_page(
                &page(vec![command("item_1", json!(0)), message("item_2")], false),
                None,
            )
            .unwrap();
        assert_eq!(
            audit.finish(ROOT).unwrap(),
            HostedSavedItemAuditSummary {
                pages: 1,
                items: 2,
                command_executions: 1
            }
        );
        assert!(audit.finish(HostedSessionObservedOutcome::Pending).is_err());
        assert!(audit
            .observe_page(&page(vec![message("item_3")], false), None)
            .is_err());
        assert!(audit.finish(ROOT).is_err());
    }

    #[test]
    fn cursor_chain_requires_all_pages_and_exact_requested_after() {
        let mut audit = HostedSavedItemAudit::new(TURN).unwrap();
        audit
            .observe_page(&page(vec![message("item_a")], true), None)
            .unwrap();
        assert_eq!(audit.next_after(), Some("item_a"));
        assert!(audit.finish(ROOT).is_err());
        audit
            .observe_page(&page(vec![message("item_b")], false), Some("item_a"))
            .unwrap();
        assert_eq!(audit.finish(ROOT).unwrap().pages, 2);
    }

    #[test]
    fn wrong_cursor_and_duplicate_ids_poison_the_audit() {
        let mut audit = HostedSavedItemAudit::new(TURN).unwrap();
        audit
            .observe_page(&page(vec![message("item_a")], true), None)
            .unwrap();
        assert!(audit
            .observe_page(&page(vec![message("item_b")], false), None)
            .is_err());
        assert!(audit
            .observe_page(&page(vec![message("item_b")], false), Some("item_a"))
            .is_err());

        let mut audit = HostedSavedItemAudit::new(TURN).unwrap();
        audit
            .observe_page(&page(vec![message("item_a")], true), None)
            .unwrap();
        assert!(audit
            .observe_page(&page(vec![message("item_a")], false), Some("item_a"))
            .is_err());
        assert!(audit.finish(ROOT).is_err());
    }

    #[test]
    fn failed_incomplete_or_nonzero_command_is_rejected() {
        for status in ["failed", "incomplete", "in_progress"] {
            let mut cmd = command("item_1", json!(0));
            cmd["status"] = json!(status);
            let mut audit = HostedSavedItemAudit::new(TURN).unwrap();
            assert!(audit.observe_page(&page(vec![cmd], false), None).is_err());
        }
        for code in [json!(1), json!(-1), Value::Null] {
            let mut audit = HostedSavedItemAudit::new(TURN).unwrap();
            assert!(audit
                .observe_page(&page(vec![command("item_1", code)], false), None)
                .is_err());
        }
    }

    #[test]
    fn denied_capabilities_unknown_items_and_turn_drift_reject() {
        for kind in [
            "mcp_call",
            "function_call",
            "computer_use_call",
            "web_search_call",
            "create_subagent_call",
            "future_tool",
        ] {
            let item = json!({
                "id": "item_1",
                "turn_id": TURN,
                "type": kind,
                "status": "completed"
            });
            let mut audit = HostedSavedItemAudit::new(TURN).unwrap();
            assert!(audit.observe_page(&page(vec![item], false), None).is_err());
        }
        let mut item = message("item_1");
        item["turn_id"] = json!("turn_other");
        let mut audit = HostedSavedItemAudit::new(TURN).unwrap();
        assert!(audit.observe_page(&page(vec![item], false), None).is_err());
    }

    #[test]
    fn reasoning_null_status_allowed_but_unfinished_message_is_not() {
        let reasoning = json!({
            "id": "item_1",
            "turn_id": TURN,
            "type": "reasoning",
            "status": null,
            "summary": []
        });
        let mut audit = HostedSavedItemAudit::new(TURN).unwrap();
        audit
            .observe_page(&page(vec![reasoning], false), None)
            .unwrap();
        assert_eq!(audit.finish(ROOT).unwrap().items, 1);
        let mut item = message("item_2");
        item["status"] = json!("in_progress");
        let mut audit = HostedSavedItemAudit::new(TURN).unwrap();
        assert!(audit.observe_page(&page(vec![item], false), None).is_err());
    }

    #[test]
    fn missing_or_lied_about_page_boundaries_are_rejected() {
        for bad in [
            json!({"object":"list", "data":[], "first_id":null, "last_id":null, "has_more":false}),
            json!({
                "object":"list", "data":[message("item_1")],
                "first_id":"bad", "last_id":"item_1", "has_more":false
            }),
            json!({
                "object":"list", "data":[message("item_1")],
                "first_id":"item_1", "last_id":"bad", "has_more":false
            }),
            json!({
                "object":"not_list", "data":[message("item_1")],
                "first_id":"item_1", "last_id":"item_1", "has_more":false
            }),
        ] {
            let mut audit = HostedSavedItemAudit::new(TURN).unwrap();
            assert!(audit
                .observe_page(&serde_json::to_vec(&bad).unwrap(), None)
                .is_err());
        }
    }

    #[test]
    fn bounds_and_invalid_identity_fail_closed() {
        for bad in ["", "turn/evil", "turn space", "a:b"] {
            assert!(HostedSavedItemAudit::new(bad).is_err());
        }
        let mut audit = HostedSavedItemAudit::new(TURN).unwrap();
        assert!(audit
            .observe_page(&vec![b' '; MAX_HOSTED_ITEM_PAGE_JSON_BYTES + 1], None)
            .is_err());
        assert!(audit.finish(ROOT).is_err());
        let mut audit = HostedSavedItemAudit::new(TURN).unwrap();
        let too_many = (0..=MAX_HOSTED_ITEMS_PER_PAGE)
            .map(|i| message(&format!("item_{i}")))
            .collect();
        assert!(audit.observe_page(&page(too_many, false), None).is_err());
    }
}
