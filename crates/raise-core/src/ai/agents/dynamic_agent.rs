// FICHIER : src-tauri/src/ai/agents/dynamic_agent.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::utils::prelude::*;

use super::intent_classifier::EngineeringIntent;
use super::prompt_engine::PromptEngine;
use super::tools::{extract_json_from_llm, load_session, save_artifacts_batch, save_session};
use super::{Agent, AgentContext, AgentResult, CreatedArtifact};

use crate::ai::llm::client::LlmBackend;
use crate::utils::data::json::Clearance;

/// L'Agent Dynamique piloté par les données (Data-Driven).
pub struct DynamicAgent {
    handle: String,
}

impl DynamicAgent {
    pub fn new(handle: &str) -> Self {
        Self {
            handle: handle.to_string(),
        }
    }
}

#[async_interface]
impl Agent for DynamicAgent {
    fn id(&self) -> &str {
        &self.handle
    }

    async fn process(
        &self,
        ctx: &AgentContext,
        intent: &EngineeringIntent,
    ) -> RaiseResult<Option<AgentResult>> {
        let config = AppConfig::get();

        let sys_manager = CollectionsManager::new(
            &ctx.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        // 1. Charger la configuration de l'Agent
        let agent_doc = match sys_manager.get_document("agents", &self.handle).await {
            Ok(Some(doc)) => doc,
            Ok(None) => raise_error!(
                "ERR_AGENT_CONFIG_NOT_FOUND",
                error = format!("Agent '{}' introuvable.", self.handle),
                context =
                    json_value!({ "handle": self.handle, "mount": config.mount_points.system })
            ),
            Err(e) => raise_error!(
                "ERR_AGENT_DB_READ",
                error = e,
                context = json_value!({ "handle": self.handle })
            ),
        };

        // 2. Extraire le prompt_id
        let prompt_id = match agent_doc["base"]["neuro_profile"]["prompt_id"].as_str() {
            Some(id) => id,
            None => raise_error!(
                "ERR_AGENT_MISSING_PROMPT",
                error = "prompt_id absent du neuro_profile.",
                context = json_value!({ "agent": self.handle })
            ),
        };

        // 3. Compiler le System Prompt
        let prompt_engine = PromptEngine::new(
            ctx.db.clone(),
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let system_prompt = match prompt_engine.compile(prompt_id, None).await {
            Ok(prompt) => prompt,
            Err(e) => raise_error!(
                "ERR_AGENT_PROMPT_COMPILE",
                error = e,
                context = json_value!({ "agent": self.handle, "prompt_id": prompt_id })
            ),
        };

        // 4. Charger la session
        let mut session = match load_session(ctx).await {
            Ok(s) => s,
            Err(e) => {
                user_warn!(
                    "WARN_SESSION_LOAD",
                    json_value!({"agent": self.handle, "err": e.to_string()})
                );
                super::AgentSession::new(&ctx.session_id, self.id())
            }
        };

        let intent_text = format!("{:?}", intent);
        session.add_message("user", &intent_text);

        let history_str = session
            .messages
            .iter()
            .rev()
            .take(10)
            .rev()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let user_prompt = format!(
            "=== HISTORIQUE ===\n{}\n\n=== TÂCHE ===\n{}",
            history_str, intent_text
        );

        let agent_name = agent_doc["base"]["name"]["fr"]
            .as_str()
            .unwrap_or(&self.handle);

        user_info!(
            "SYS_INFO",
            json_value!({ "message": format!("🧠 Agent : {}", agent_name) })
        );

        // 5. Exécution neuronale
        let response = match ctx
            .llm
            .ask(
                LlmBackend::LocalLlama,
                &system_prompt,
                &user_prompt,
                Clearance::Internal,
            )
            .await
        {
            Ok(res) => res,
            Err(e) => raise_error!(
                "ERR_AGENT_LLM_EXECUTE",
                error = e,
                context = json_value!({ "agent": self.handle, "prompt_id": prompt_id })
            ),
        };

        // 6. Extraction et Persistance des Artefacts
        let clean_json = extract_json_from_llm(&response);
        session.add_message("assistant", &clean_json);

        if let Err(e) = save_session(ctx, &session).await {
            user_warn!("WARN_SESSION_SAVE", json_value!({"err": e.to_string()}));
        }

        let parsed: JsonValue = json::deserialize_from_str(&clean_json).unwrap_or(json_value!({}));
        let mut raw_docs = vec![];

        match parsed {
            JsonValue::Array(arr) => raw_docs.extend(arr),
            JsonValue::Object(obj) if !obj.is_empty() => raw_docs.push(JsonValue::Object(obj)),
            _ => {}
        }

        // 🎯 OPTIMISATION : Validation en RAM
        let mut valid_artifacts = vec![];
        for mut doc in raw_docs {
            let layer = doc["layer"].as_str().unwrap_or("").to_string();
            let element_type = doc["type"].as_str().unwrap_or("").to_string();

            if layer.is_empty() || element_type.is_empty() {
                continue;
            }

            if let Some(obj) = doc.as_object_mut() {
                if !obj.contains_key("_id") {
                    obj.insert(
                        "_id".to_string(),
                        json_value!(UniqueId::new_v4().to_string()),
                    );
                }
            }
            valid_artifacts.push(doc);
        }

        // 🎯 BATCHING : Typage explicite ajouté pour aider l'IDE avant la MAJ de tools.rs
        let artifacts: Vec<CreatedArtifact> = match save_artifacts_batch(ctx, valid_artifacts).await
        {
            Ok(arts) => arts,
            Err(e) => raise_error!(
                "ERR_AGENT_ARTIFACTS_BATCH_SAVE",
                error = e,
                context = json_value!({ "agent": self.handle })
            ),
        };

        Ok(Some(AgentResult {
            message: format!("Cycle terminé. {} artefacts persistés.", artifacts.len()),
            artifacts,
            outgoing_message: None,
            xai_frame: None,
        }))
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::llm::client::LlmClient;
    use crate::ai::world_model::NeuroSymbolicEngine;
    use crate::utils::core::error::AppError;
    use crate::utils::testing::{AgentDbSandbox, DbSandbox};

    async fn setup_test_ctx(sandbox: &AgentDbSandbox) -> RaiseResult<AgentContext> {
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let llm = match LlmClient::new(
            &manager,
            sandbox.db.clone(),
            Some(sandbox.shared_engine.clone()),
        )
        .await
        {
            Ok(c) => c,
            Err(e) => raise_error!("ERR_TEST_LLM", error = e),
        };

        let world_engine = SharedRef::new(match NeuroSymbolicEngine::bootstrap(&manager).await {
            Ok(we) => we,
            Err(e) => raise_error!("ERR_TEST_WM", error = e),
        });
        Ok(AgentContext::new(
            "test_agent",
            "sess_123",
            sandbox.db.clone(),
            llm,
            world_engine,
            sandbox.domain_root.clone(),
            sandbox.domain_root.clone(),
        )
        .await?)
    }

    #[test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    fn test_id_mapping() {
        let agent = DynamicAgent::new("agent_modeling");
        assert_eq!(agent.id(), "agent_modeling");
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_err_agent_not_found() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let ctx = setup_test_ctx(&sandbox).await?;
        let agent = DynamicAgent::new("agent_fantome");

        match agent.process(&ctx, &EngineeringIntent::Chat).await {
            Err(AppError::Structured(data)) => {
                assert_eq!(data.code, "ERR_AGENT_CONFIG_NOT_FOUND");
                Ok(())
            }
            _ => raise_error!(
                "ERR_TEST_FAIL",
                error = "Attendu: ERR_AGENT_CONFIG_NOT_FOUND"
            ),
        }
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_err_missing_prompt_id() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let ctx = setup_test_ctx(&sandbox).await?;
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let schema_uri = "db://_system/_system/schemas/v1/db/generic.schema.json";
        manager.create_collection("agents", schema_uri).await?;

        // 🎯 L'agent est inséré dans sa collection "agents"
        let doc_to_insert = json_value!({
            "_id": "invalid_agent",
            "base": { "name": {"fr": "Sans Prompt"}, "neuro_profile": {} }
        });

        manager.insert_with_schema("agents", doc_to_insert).await?;

        let agent = DynamicAgent::new("invalid_agent");
        let result = agent.process(&ctx, &EngineeringIntent::Chat).await;
        match result {
            Err(AppError::Structured(data)) => {
                assert_eq!(data.code, "ERR_AGENT_MISSING_PROMPT");
                Ok(())
            }
            _ => raise_error!("ERR_TEST_FAIL", error = "Attendu: ERR_AGENT_MISSING_PROMPT"),
        }
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_successful_execution_and_session_init() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let ctx = setup_test_ctx(&sandbox).await?;

        let sys_manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let ws_manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.modeling.domain,
            &config.mount_points.modeling.db,
        );

        DbSandbox::mock_db(&ws_manager).await?;

        let generic_uri = "db://_system/_system/schemas/v1/db/generic.schema.json";

        // 1. Initialisation des collections système et workspace
        for col in &["prompts", "agents", "session_agents"] {
            sys_manager.create_collection(col, generic_uri).await?;
        }
        ws_manager
            .create_collection("session_agents", generic_uri)
            .await?;

        // 2. Injection des données valides via la Forteresse
        sys_manager
            .insert_with_schema(
                "prompts",
                json_value!({
                    "_id": "p_test",
                    "role": "system",
                    "environment": "Environnement de test unitaire",
                    "identity": {
                        "persona": "Test"
                    },
                    "directives": ["OK"]
                }),
            )
            .await?;

        sys_manager
            .insert_with_schema(
                "agents",
                json_value!({
                    "_id": "agent_ok",
                    "base": {
                        "name": {"fr": "Agent OK"},
                        "neuro_profile": { "prompt_id": "p_test" }
                    }
                }),
            )
            .await?;

        // 3. Exécution de l'agent
        let agent = DynamicAgent::new("agent_ok");
        let result = agent.process(&ctx, &EngineeringIntent::Chat).await;

        match result {
            Ok(_) => {}
            Err(e) => raise_error!(
                "ERR_TEST_FAIL",
                error = format!("L'exécution de l'agent a échoué : {:?}", e)
            ),
        };

        // 4. Vérification de la session
        let query = crate::json_db::query::Query::new("session_agents");

        let res_sys = crate::json_db::query::QueryEngine::new(&sys_manager)
            .execute_query(query.clone())
            .await?;
        let res_ws = crate::json_db::query::QueryEngine::new(&ws_manager)
            .execute_query(query)
            .await?;

        if res_sys.documents.is_empty() && res_ws.documents.is_empty() {
            raise_error!(
                "ERR_TEST_FAIL",
                error =
                    "La collection 'session_agents' est vide partout. save_session n'a pas écrit."
            );
        }

        Ok(())
    }
}
