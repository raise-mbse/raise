// FICHIER : src-tauri/src/ai/agents/mod.rs

pub mod context;
pub mod dynamic_agent;
pub mod intent_classifier;
pub mod prompt_engine;
pub mod software_agent;
pub mod tools;

pub use self::context::AgentContext;

use self::intent_classifier::EngineeringIntent;
use crate::ai::protocols::acl::AclMessage;
use crate::utils::prelude::*;

/// Représente un élément créé ou modifié par un agent
#[derive(Debug, Clone, Serializable, Deserializable)]
pub struct CreatedArtifact {
    pub id: String,
    pub name: String,
    pub layer: String,
    pub element_type: String,
    pub path: String,
}

#[derive(Debug, Clone, Serializable, Deserializable)]
pub struct AgentResult {
    pub message: String,
    pub artifacts: Vec<CreatedArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outgoing_message: Option<AclMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xai_frame: Option<crate::ai::assurance::XaiFrame>,
}

impl AgentResult {
    pub fn text(msg: String) -> Self {
        Self {
            message: msg,
            artifacts: vec![],
            outgoing_message: None,
            xai_frame: None,
        }
    }

    pub fn communicate(msg: AclMessage) -> Self {
        Self {
            message: format!("🔄 Communication sortante vers {}", msg.receiver),
            artifacts: vec![],
            outgoing_message: Some(msg),
            xai_frame: None,
        }
    }
}

#[derive(Debug, Clone, Serializable, Deserializable)]
pub struct AgentTask {
    pub id: UniqueId,
    pub task_type: String,
    pub created_at: UtcTimestamp,
}

impl FmtDisplay for AgentTask {
    fn fmt(&self, f: &mut FmtCursor<'_>) -> FmtResult {
        write!(f, "AgentTask(id: {}, type: {})", self.id, self.task_type)
    }
}

// --- STRUCTURES DE MÉMOIRE ---

#[derive(Debug, Clone, Serializable, Deserializable)]
pub struct AgentMessage {
    pub role: String,
    pub content: String,
    pub timestamp: UtcTimestamp,
}

impl AgentMessage {
    pub fn new(role: &str, content: &str) -> Self {
        Self {
            role: role.to_string(),
            content: content.to_string(),
            timestamp: UtcClock::now(),
        }
    }
}

#[derive(Debug, Clone, Serializable, Deserializable)]
pub struct AgentSession {
    #[serde(rename = "_id")]
    pub id: String,
    pub agent_id: String,
    pub created_at: UtcTimestamp,
    pub updated_at: UtcTimestamp,
    pub messages: Vec<AgentMessage>,
    pub summary: Option<String>,
}

impl AgentSession {
    pub fn new(id: &str, agent_id: &str) -> Self {
        Self {
            id: id.to_string(),
            agent_id: agent_id.to_string(),
            created_at: UtcClock::now(),
            updated_at: UtcClock::now(),
            messages: Vec::new(),
            summary: None,
        }
    }

    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push(AgentMessage::new(role, content));
        self.updated_at = UtcClock::now();
    }
}

#[async_interface]
pub trait Agent: Send + Sync {
    fn id(&self) -> &str;

    async fn process(
        &self,
        ctx: &AgentContext,
        intent: &EngineeringIntent,
    ) -> RaiseResult<Option<AgentResult>>;
}

// =========================================================================
// TESTS UNITAIRES (Standard Zéro Dette)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_struct() -> RaiseResult<()> {
        let mut session = AgentSession::new("sess_1", "agent_1");
        session.add_message("user", "Hello");
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].role, "user");
        Ok(())
    }

    #[test]
    fn test_agent_result_acl_support() -> RaiseResult<()> {
        use crate::ai::protocols::acl::{AclMessage, Performative};

        let res_text = AgentResult::text("Hello".to_string());
        assert!(res_text.outgoing_message.is_none());

        // 🎯 FIX : Ajout du 5ème argument (Ontology) positionné à None pour ce test
        let msg = AclMessage::new(Performative::Request, "sender", "receiver", "content", None);
        let res_acl = AgentResult::communicate(msg);

        assert!(res_acl.outgoing_message.is_some());

        // 🎯 FIX ZÉRO DETTE : Pas de .unwrap()
        if let Some(msg) = res_acl.outgoing_message {
            assert_eq!(msg.receiver, "receiver");
        } else {
            raise_error!(
                "ERR_TEST_ACL_INCONSISTENCY",
                error = "Le message sortant devrait être présent."
            );
        }

        Ok(())
    }
}
