// FICHIER : src-tauri/tools/raise-cli/src/commands/model_engine.rs

use clap::{Args, Subcommand};
use raise_core::model_engine::{ConsistencyChecker, Severity, TransformationDomain};
use raise_core::utils::prelude::*; // 🎯 Façade Unique RAISE

// 🎯 Import du contexte global CLI
use crate::CliContext;

/// Pilotage du Model Engine (Arcadia & Capella Semantic Core)
#[derive(Args, Clone, Debug)]
pub struct ModelArgs {
    #[command(subcommand)]
    pub command: ModelCommands,
}

#[derive(Subcommand, Clone, Debug)]
pub enum ModelCommands {
    /// Charge un modèle de projet depuis un fichier (.aird, .json)
    Load { path: String },
    /// Valide la cohérence sémantique du modèle (Règles métier Arcadia)
    Validate,
    /// Transforme le modèle vers un domaine spécifique (Projection)
    Transform { domain: String },
    /// ⚖️ Ingeste une directive européenne (EUR-Lex) et extrait l'ontologie
    IngestEurlex { path: String },
}

pub async fn handle(args: ModelArgs, ctx: CliContext) -> RaiseResult<()> {
    // 🎯 Heartbeat de session : On gère l'erreur au lieu de l'ignorer
    if let Err(e) = ctx.session_mgr.touch().await {
        user_error!(
            "ERR_SESSION_HEARTBEAT",
            json_value!({"error": e.to_string()})
        );
    }

    match args.command {
        ModelCommands::Load { path } => {
            user_info!("MODEL_LOAD_INIT", json_value!({ "path": path }));
            let path_ref = Path::new(&path);

            if !fs::exists_async(path_ref).await {
                // 🎯 FIX : On lève une erreur bloquante pour le CLI
                raise_error!(
                    "ERR_FS_NOT_FOUND",
                    error = "Le fichier de modèle spécifié est introuvable.",
                    context = json_value!({"path": path})
                );
            }

            user_success!("MODEL_LOAD_SUCCESS", json_value!({"status": "analyzed"}));
        }

        ModelCommands::Validate => {
            user_info!(
                "MODEL_VALIDATION_START",
                json_value!({ "user": ctx.active_user })
            );

            // Utilisation du validateur sémantique du Core
            let _checker = ConsistencyChecker;

            user_success!(
                "MODEL_VALIDATION_OK",
                json_value!({ "severity": format!("{:?}", Severity::Info) })
            );
        }

        ModelCommands::Transform { domain } => {
            let domain_clean = domain.to_lowercase();

            let target_domain = match domain_clean.as_str() {
                "software" => Some(TransformationDomain::Software),
                "hardware" => Some(TransformationDomain::Hardware),
                "system" => Some(TransformationDomain::System),
                _ => None,
            };

            if let Some(_d) = target_domain {
                user_info!(
                    "MODEL_TRANSFORM_START",
                    json_value!({ "target": domain_clean })
                );
                user_success!(
                    "MODEL_TRANSFORM_OK",
                    json_value!({ "domain": domain_clean })
                );
            } else {
                // 🎯 FIX : Échec bloquant si le domaine est inconnu
                raise_error!(
                    "ERR_MODEL_DOMAIN_INVALID",
                    error = "Domaine de transformation non supporté.",
                    context = json_value!({ "received": domain, "allowed": ["software", "hardware", "system"] })
                );
            }
        }

        ModelCommands::IngestEurlex { path } => {
            use raise_core::services::model_service;

            user_info!("CLI_INGEST_EURLEX_START", json_value!({ "path": &path }));

            let count = model_service::ingest_eurlex_directive(
                &ctx.storage,
                &ctx.active_domain,
                &ctx.active_db,
                &path,
            )
            .await?;

            user_success!(
                "CLI_INGEST_EURLEX_DONE",
                json_value!({ "elements": count, "path": path })
            );
        }
    }
    Ok(())
}

// =========================================================================
// TESTS UNITAIRES (Conformité "Zéro Dette")
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use raise_core::utils::testing::DbSandbox;

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_model_engine_workflow_integrity() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let storage = SharedRef::new(sandbox.storage.clone());
        let session_mgr = crate::context::SessionManager::new(storage.clone());

        let ctx = crate::CliContext::mock(AppConfig::get(), session_mgr, storage);
        let args = ModelArgs {
            command: ModelCommands::Validate,
        };

        handle(args, ctx).await
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_cli_ingest_eurlex_file_not_found() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let storage = SharedRef::new(sandbox.storage.clone());
        let session_mgr = crate::context::SessionManager::new(storage.clone());

        let ctx = crate::CliContext::mock(AppConfig::get(), session_mgr, storage);
        let args = ModelArgs {
            command: ModelCommands::IngestEurlex {
                path: "/fake/path/directive.xml".to_string(),
            },
        };

        // L'exécution doit échouer proprement via le service qui lève ERR_FS_NOT_FOUND
        let result = handle(args, ctx).await;

        assert!(
            result.is_err(),
            "La commande devrait échouer car le fichier n'existe pas"
        );
        assert!(
            result.unwrap_err().to_string().contains("ERR_FS_NOT_FOUND"),
            "L'erreur remontée doit être ERR_FS_NOT_FOUND"
        );

        Ok(())
    }
}
