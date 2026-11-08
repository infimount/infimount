//! Pure bounded Agents API session-event outcome monitor.
//!
//! No HTTP, auth, session creation, tool call, artifact import or publication.
//! A completed root turn is UNVERIFIED until the trusted caller independently
//! inspects saved tool/item results and verifies downloaded output bytes.
//! The official SDK distinguishes root turns using turn.subagent_id == null.
use infimount_core::CoreError;
use serde_json::Value;

pub const MAX_OPENAI_HOSTED_EVENT_JSON_BYTES: usize = 1024 * 1024;
pub const MAX_OPENAI_HOSTED_STREAM_EVENTS: usize = 50_000;

/// Even a completed root turn is not authorization to import or publish files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedSessionObservedOutcome {
    Pending,
    RequiresAction,
    RootTurnCompletedUnverified,
    RootTurnFailed,
    RootTurnCancelled,
    SessionFailed,
    StreamInterrupted,
}

#[derive(Debug)]
pub struct HostedSessionEventGate {
    state: HostedSessionObservedOutcome,
    event_count: usize,
    stream_closed: bool,
}

impl Default for HostedSessionEventGate {
    fn default() -> Self {
        Self {
            state: HostedSessionObservedOutcome::Pending,
            event_count: 0,
            stream_closed: false,
        }
    }
}

impl HostedSessionEventGate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observed(&self) -> HostedSessionObservedOutcome {
        self.state
    }

    /// Only for events received from an authenticated session-scoped stream.
    /// A model-supplied JSON string is not a provider event.
    pub fn observe_json(
        &mut self,
        event_json: &[u8],
    ) -> Result<HostedSessionObservedOutcome, CoreError> {
        if self.stream_closed
            || event_json.is_empty()
            || event_json.len() > MAX_OPENAI_HOSTED_EVENT_JSON_BYTES
            || self.event_count >= MAX_OPENAI_HOSTED_STREAM_EVENTS
        {
            return self.reject_event();
        }
        self.event_count += 1;

        let event: Value = match serde_json::from_slice(event_json) {
            Ok(value) => value,
            Err(_) => return self.reject_event(),
        };
        let kind = match event.get("type").and_then(Value::as_str) {
            Some(kind) => kind,
            None => return self.reject_event(),
        };

        match kind {
            "error" | "agent.session.failed" | "agent.session.environment.failed" => {
                self.state = HostedSessionObservedOutcome::SessionFailed;
            }
            "agent.session.turn.completed"
            | "agent.session.turn.failed"
            | "agent.session.turn.cancelled" => {
                // Distinguish root from subagents. A missing discriminator
                // must never be interpreted as a successful root turn.
                let subagent = match event.get("turn").and_then(|turn| turn.get("subagent_id")) {
                    Some(value) => value,
                    None => return self.reject_event(),
                };
                if !subagent.is_null() {
                    if !subagent.is_string() {
                        return self.reject_event();
                    }
                    return Ok(self.state);
                }
                // Contradictory root terminal events are not permission
                // to trust an earlier completion. Fail closed on drift.
                let observed = match kind {
                    "agent.session.turn.completed" => {
                        HostedSessionObservedOutcome::RootTurnCompletedUnverified
                    }
                    "agent.session.turn.failed" => HostedSessionObservedOutcome::RootTurnFailed,
                    _ => HostedSessionObservedOutcome::RootTurnCancelled,
                };
                if matches!(
                    self.state,
                    HostedSessionObservedOutcome::Pending
                        | HostedSessionObservedOutcome::RequiresAction
                ) {
                    self.state = observed;
                } else if self.state != observed {
                    self.state = HostedSessionObservedOutcome::SessionFailed;
                }
            }
            "agent.session.requires_action" | "agent.session.action_required" => {
                if self.state == HostedSessionObservedOutcome::Pending {
                    self.state = HostedSessionObservedOutcome::RequiresAction;
                }
            }
            "agent.session.idle" => {}
            _ => {
                // Unknown/new event types and text deltas are observational,
                // never implicit turn success.
            }
        }
        Ok(self.state)
    }

    /// Stream closure does not imply success or cancellation. If no root turn
    /// ended, the caller must recover saved session state before further work.
    pub fn finish_stream(&mut self) -> HostedSessionObservedOutcome {
        self.stream_closed = true;
        if matches!(
            self.state,
            HostedSessionObservedOutcome::Pending | HostedSessionObservedOutcome::RequiresAction
        ) {
            self.state = HostedSessionObservedOutcome::StreamInterrupted;
        }
        self.state
    }

    fn reject_event(&mut self) -> Result<HostedSessionObservedOutcome, CoreError> {
        self.state = HostedSessionObservedOutcome::SessionFailed;
        Err(CoreError::Config(
            "OpenAI-hosted session event was malformed, oversized or out of budget".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HostedSessionObservedOutcome as Outcome;

    #[test]
    fn root_completion_is_explicit_but_does_not_authorize_artifacts() {
        let mut gate = HostedSessionEventGate::new();
        gate.observe_json(br#"{"type":"agent.session.created"}"#)
            .unwrap();
        gate.observe_json(br#"{"type":"agent.session.idle"}"#)
            .unwrap();
        gate.observe_json(br#"{"type":"agent.session.turn.output_text.done","text":"done"}"#)
            .unwrap();
        assert_eq!(gate.observed(), Outcome::Pending);
        gate.observe_json(
            br#"{"type":"agent.session.turn.completed","turn":{"subagent_id":null}}"#,
        )
        .unwrap();
        assert_eq!(gate.finish_stream(), Outcome::RootTurnCompletedUnverified);
        assert!(gate
            .observe_json(br#"{"type":"agent.session.idle"}"#)
            .is_err());
        assert_eq!(gate.observed(), Outcome::SessionFailed);
    }

    #[test]
    fn subagent_completion_and_idle_do_not_finish_root_turn() {
        let mut gate = HostedSessionEventGate::new();
        gate.observe_json(
            br#"{"type":"agent.session.turn.completed","turn":{"subagent_id":"sub_123"}}"#,
        )
        .unwrap();
        gate.observe_json(br#"{"type":"agent.session.idle"}"#)
            .unwrap();
        assert_eq!(gate.finish_stream(), Outcome::StreamInterrupted);
    }

    #[test]
    fn required_action_then_stream_disconnect_is_not_success() {
        let mut gate = HostedSessionEventGate::new();
        gate.observe_json(br#"{"type":"agent.session.requires_action"}"#)
            .unwrap();
        assert_eq!(gate.observed(), Outcome::RequiresAction);
        gate.observe_json(br#"{"type":"agent.session.idle"}"#)
            .unwrap();
        assert_eq!(gate.finish_stream(), Outcome::StreamInterrupted);
    }

    #[test]
    fn root_failure_cancel_and_session_failure_are_distinct() {
        for (kind, outcome) in [
            ("agent.session.turn.failed", Outcome::RootTurnFailed),
            ("agent.session.turn.cancelled", Outcome::RootTurnCancelled),
        ] {
            let mut gate = HostedSessionEventGate::new();
            let event = format!(r#"{{"type":"{kind}","turn":{{"subagent_id":null}}}}"#);
            gate.observe_json(event.as_bytes()).unwrap();
            assert_eq!(gate.finish_stream(), outcome);
        }
        for kind in [
            "agent.session.failed",
            "agent.session.environment.failed",
            "error",
        ] {
            let mut gate = HostedSessionEventGate::new();
            let event = format!(r#"{{"type":"{kind}"}}"#);
            gate.observe_json(event.as_bytes()).unwrap();
            assert_eq!(gate.finish_stream(), Outcome::SessionFailed);
        }
    }

    #[test]
    fn contradictory_root_terminals_reject_instead_of_trusting_prior_completion() {
        let mut gate = HostedSessionEventGate::new();
        gate.observe_json(
            br#"{"type":"agent.session.turn.completed","turn":{"subagent_id":null}}"#,
        )
        .unwrap();
        gate.observe_json(br#"{"type":"agent.session.turn.failed","turn":{"subagent_id":null}}"#)
            .unwrap();
        assert_eq!(gate.finish_stream(), Outcome::SessionFailed);

        let mut gate = HostedSessionEventGate::new();
        gate.observe_json(
            br#"{"type":"agent.session.turn.cancelled","turn":{"subagent_id":null}}"#,
        )
        .unwrap();
        gate.observe_json(
            br#"{"type":"agent.session.turn.completed","turn":{"subagent_id":null}}"#,
        )
        .unwrap();
        assert_eq!(gate.finish_stream(), Outcome::SessionFailed);
    }

    #[test]
    fn global_failure_after_completion_must_revoke_it() {
        let mut gate = HostedSessionEventGate::new();
        gate.observe_json(
            br#"{"type":"agent.session.turn.completed","turn":{"subagent_id":null}}"#,
        )
        .unwrap();
        gate.observe_json(br#"{"type":"agent.session.failed"}"#)
            .unwrap();
        assert_eq!(gate.finish_stream(), Outcome::SessionFailed);
    }

    #[test]
    fn missing_root_discriminator_and_invalid_event_fail_closed() {
        for bytes in [
            &br#"{"type":"agent.session.turn.completed"}"#[..],
            &br#"{"type":"agent.session.turn.completed","turn":{}}"#[..],
            &br#"{"type":"agent.session.turn.completed","turn":{"subagent_id":false}}"#[..],
            &br#"{"type":42}"#[..],
            &br#"not json"#[..],
        ] {
            let mut gate = HostedSessionEventGate::new();
            assert!(gate.observe_json(bytes).is_err());
            assert_eq!(gate.observed(), Outcome::SessionFailed);
        }
        let mut gate = HostedSessionEventGate::new();
        assert!(gate
            .observe_json(&vec![b' '; MAX_OPENAI_HOSTED_EVENT_JSON_BYTES + 1])
            .is_err());
        assert_eq!(gate.finish_stream(), Outcome::SessionFailed);
    }

    #[test]
    fn stream_event_budget_is_bounded() {
        let mut gate = HostedSessionEventGate::new();
        for _ in 0..MAX_OPENAI_HOSTED_STREAM_EVENTS {
            gate.observe_json(br#"{"type":"agent.session.idle"}"#)
                .unwrap();
        }
        assert!(gate
            .observe_json(br#"{"type":"agent.session.idle"}"#)
            .is_err());
        assert_eq!(gate.observed(), Outcome::SessionFailed);
    }
}
