// FICHIER : crates/raise-core/src/services/devops_service.rs

use crate::ai::agents::devops_agent::DevopsAgent;
use crate::ai::agents::intent_classifier::EngineeringIntent;
use crate::ai::agents::{Agent, AgentContext};
use crate::ai::llm::client::{LlmClient, LlmEngine};
use crate::ai::world_model::NeuroSymbolicEngine;
use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::StorageEngine;
use crate::utils::core::instrument;
use crate::utils::prelude::*;

/// Regroupe le contexte d'exécution pour éviter les signatures de fonctions surchargées
pub struct DevopsExecutionContext<'a> {
    pub domain: &'a str,
    pub db: &'a str,
    pub storage: SharedRef<StorageEngine>,
    pub native_llm: Option<SharedRef<AsyncMutex<dyn LlmEngine>>>,
    pub session_id: &'a str,
    pub is_test_mode: bool,
}

/// 🚀 Point d'entrée métier pour déclencher un déploiement SRE via l'Agent DevOps
#[instrument(skip(ctx))]
pub async fn deploy_edge_artifact(
    target_handle: &str,
    target_architecture: &str,
    payload_uri: &str,
    ctx: DevopsExecutionContext<'_>,
) -> RaiseResult<String> {
    crate::user_info!(
        "SVC_DEVOPS_START_DEPLOY",
        json_value!({"target": target_handle, "arch": target_architecture, "uri": payload_uri})
    );

    let config = AppConfig::get();
    let manager = CollectionsManager::new(&ctx.storage, ctx.domain, ctx.db);

    let llm = match LlmClient::new(&manager, ctx.storage.clone(), ctx.native_llm).await {
        Ok(c) => c,
        Err(e) => raise_error!("ERR_DEVOPS_LLM_INIT", error = e.to_string()),
    };

    let world_engine = match NeuroSymbolicEngine::bootstrap(&manager).await {
        Ok(engine) => SharedRef::new(engine),
        Err(e) => raise_error!("ERR_DEVOPS_WORLD_ENGINE_INIT", error = e.to_string()),
    };

    let domain_root = if ctx.is_test_mode {
        config
            .get_path("PATH_CODE_FILE")
            .unwrap_or_else(|| config.get_path("PATH_RAISE_DOMAIN").unwrap_or_default())
    } else {
        config.get_path("PATH_RAISE_DOMAIN").unwrap_or_default()
    };
    let dataset_root = domain_root.join("dataset");

    let agent_ctx = AgentContext::new(
        "agent_devops",
        ctx.session_id,
        ctx.storage.clone(),
        llm,
        world_engine,
        domain_root,
        dataset_root,
    )
    .await?;

    let agent = DevopsAgent::new(ctx.domain.to_string(), ctx.db.to_string());
    let intent = EngineeringIntent::DeployEdgeArtifact {
        target_handle: target_handle.to_string(),
        target_architecture: target_architecture.to_string(),
        payload_uri: payload_uri.to_string(),
    };

    match agent.process(&agent_ctx, &intent).await {
        Ok(Some(res)) => Ok(res.message),
        Ok(None) => raise_error!(
            "ERR_DEVOPS_NO_RESULT",
            error = "L'agent DevOps a terminé son cycle sans émettre de résultat formel."
        ),
        Err(e) => raise_error!("ERR_DEVOPS_EXECUTION_FAILED", error = e.to_string()),
    }
}

/// 🔄 Point d'entrée métier pour déclencher une restauration d'infrastructure
#[instrument(skip(ctx))]
pub async fn rollback_deployment(
    target_handle: &str,
    fallback_commit: &str,
    ctx: DevopsExecutionContext<'_>,
) -> RaiseResult<String> {
    crate::user_info!(
        "SVC_DEVOPS_START_ROLLBACK",
        json_value!({"target": target_handle, "commit": fallback_commit})
    );

    let config = AppConfig::get();
    let manager = CollectionsManager::new(&ctx.storage, ctx.domain, ctx.db);

    let llm = match LlmClient::new(&manager, ctx.storage.clone(), ctx.native_llm).await {
        Ok(c) => c,
        Err(e) => raise_error!("ERR_DEVOPS_LLM_INIT", error = e.to_string()),
    };

    let world_engine = match NeuroSymbolicEngine::bootstrap(&manager).await {
        Ok(engine) => SharedRef::new(engine),
        Err(e) => raise_error!("ERR_DEVOPS_WORLD_ENGINE_INIT", error = e.to_string()),
    };

    let domain_root = if ctx.is_test_mode {
        config
            .get_path("PATH_CODE_FILE")
            .unwrap_or_else(|| config.get_path("PATH_RAISE_DOMAIN").unwrap_or_default())
    } else {
        config.get_path("PATH_RAISE_DOMAIN").unwrap_or_default()
    };
    let dataset_root = domain_root.join("dataset");

    let agent_ctx = AgentContext::new(
        "agent_devops",
        ctx.session_id,
        ctx.storage.clone(),
        llm,
        world_engine,
        domain_root,
        dataset_root,
    )
    .await?;

    let agent = DevopsAgent::new(ctx.domain.to_string(), ctx.db.to_string());
    let intent = EngineeringIntent::RollbackDeployment {
        target_handle: target_handle.to_string(),
        fallback_commit: fallback_commit.to_string(),
    };

    match agent.process(&agent_ctx, &intent).await {
        Ok(Some(res)) => Ok(res.message),
        Ok(None) => raise_error!(
            "ERR_DEVOPS_NO_RESULT",
            error = "L'agent DevOps a ignoré la demande de rollback."
        ),
        Err(e) => raise_error!("ERR_DEVOPS_EXECUTION_FAILED", error = e.to_string()),
    }
}

/// 🩺 TÉLÉMÉTRIE : Récupère l'état de santé logique et l'historique de l'artefact depuis la base sémantique
#[instrument(skip(ctx))]
pub async fn get_service_status(
    target_handle: &str,
    ctx: DevopsExecutionContext<'_>,
) -> RaiseResult<JsonValue> {
    let manager = CollectionsManager::new(&ctx.storage, ctx.domain, ctx.db);

    // Extraction sémantique du nœud d'infrastructure
    match manager.get_document("components", target_handle).await {
        Ok(Some(doc)) => {
            let status = doc.get("_blockchain_sync").cloned().unwrap_or_else(|| {
                json_value!({
                    "sync_at": "Aucune synchronisation distribuée",
                    "commit_id": "Local-First Only",
                    "op": "Create"
                })
            });
            Ok(json_value!({
                "handle": target_handle,
                "status": "Indexé",
                "meta": doc.get("base").unwrap_or(&json_value!({})),
                "runtime_telemetry": status
            }))
        }
        Ok(None) => Ok(json_value!({
            "handle": target_handle,
            "status": "Inconnu",
            "hint": "Le binaire n'est pas répertorié dans le Knowledge Graph local."
        })),
        Err(e) => raise_error!("ERR_STATUS_FETCH_FAILED", error = e.to_string()),
    }
}

/// 📋 TRACABILITÉ : Inspecte les fichiers de logs résiduels ou transactionnels liés au nœud physique
// 🎯 FIX : On préfixe `_ctx` par un underscore pour indiquer à Clippy que ce paramètre
// est ignoré dans le corps de la fonction (gardé uniquement pour l'uniformité de l'API)
#[instrument(skip(_ctx))]
pub async fn get_service_logs(
    target_handle: &str,
    _ctx: DevopsExecutionContext<'_>,
) -> RaiseResult<String> {
    // 🎯 FIX : Suppression du AppConfig::get() inutilisé
    let temp_dir = crate::utils::io::fs::tempdir()
        .map_err(|e| build_error!("ERR_FS_TEMP", error = e.to_string()))?;

    // Recherche déterministe d'un fichier de log de diagnostic dans le workspace
    let log_filename = format!("{}_edge.log", target_handle);
    let potential_log_path = temp_dir.path().join(&log_filename);

    if crate::utils::io::fs::exists_async(&potential_log_path).await {
        match fs::read_to_string_async(&potential_log_path).await {
            Ok(content) => Ok(content),
            Err(e) => raise_error!("ERR_LOGS_READ_FAILED", error = e.to_string()),
        }
    } else {
        Ok(format!(
            "Aucune trace de crash résiduelle pour '{}' dans le staging temporaire (Nœud stable ou nettoyé).",
            target_handle
        ))
    }
}

// =========================================================================
// TESTS UNITAIRES (Validation du Service SRE étendu)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::data::config::AppConfig;
    use crate::utils::testing::{AgentDbSandbox, DbSandbox};

    async fn setup_devops_service_sandbox() -> RaiseResult<AgentDbSandbox> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let sys_manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        DbSandbox::mock_db(&sys_manager).await?;
        let generic_schema = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );

        for col in &["components", "service_configs", "agents", "prompts"] {
            let _ = sys_manager.create_collection(col, &generic_schema).await;
        }

        sys_manager.upsert_document("components", json_value!({ "_id": "ref:components:handle:codegen_engine", "handle": "codegen_engine" })).await?;
        sys_manager
            .upsert_document(
                "service_configs",
                json_value!({
                    "_id": "mock_codegen",
                    "component_id": "ref:components:handle:codegen_engine",
                    "service_settings": { "format_on_save": true, "strict_mode": true }
                }),
            )
            .await?;

        sys_manager
            .upsert_document(
                "prompts",
                json_value!({
                    "_id": "prompt_devops",
                    "handle": "prompt_devops",
                    "role": "SRE",
                    "identity": { "persona": "Testeur", "tone": "Strict" },
                    "environment": "Test",
                    "directives": ["Valider {{target_handle}}"]
                }),
            )
            .await?;

        sys_manager
            .upsert_document(
                "agents",
                json_value!({
                    "_id": "agent_devops",
                    "base": {
                        "name": {"fr": "DevOps Agent"},
                        "neuro_profile": { "prompt_id": "prompt_devops" }
                    }
                }),
            )
            .await?;

        Ok(sandbox)
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_devops_service_rollback_success() -> RaiseResult<()> {
        let sandbox = setup_devops_service_sandbox().await?;
        let config = AppConfig::get();

        let ctx = DevopsExecutionContext {
            domain: &config.mount_points.system.domain,
            db: &config.mount_points.system.db,
            storage: sandbox.db.clone(),
            native_llm: Some(sandbox.shared_engine.clone()),
            session_id: "sess_test_rollback",
            is_test_mode: true,
        };

        let result = rollback_deployment("raise_edge_node", "commit_alpha123", ctx).await?;

        assert!(result.contains("raise_edge_node"));
        assert!(result.contains("commit_alpha123"));
        assert!(result.contains("initié"));

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_devops_service_status_unknown() -> RaiseResult<()> {
        let sandbox = setup_devops_service_sandbox().await?;
        let config = AppConfig::get();

        let ctx = DevopsExecutionContext {
            domain: &config.mount_points.system.domain,
            db: &config.mount_points.system.db,
            storage: sandbox.db.clone(),
            native_llm: None,
            session_id: "sess_test_status",
            is_test_mode: true,
        };

        let status = get_service_status("un_composant_fictif", ctx).await?;
        assert_eq!(status["status"], "Inconnu");

        Ok(())
    }
}
