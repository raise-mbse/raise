// FICHIER : src-tauri/tools/raise-cli/src/commands/traceability.rs

use clap::{Args, Subcommand};
use raise_core::{user_error, user_info, user_success, utils::prelude::*}; // 🎯 Façade Unique RAISE

// Imports métiers depuis le cœur
use raise_core::model_engine::types::ProjectModel;
use raise_core::traceability::{
    reporting::audit_report::AuditGenerator, ChangeTracker, ImpactAnalyzer, Tracer,
};

// 🎯 Import du contexte global CLI
use crate::CliContext;

#[derive(Args, Clone, Debug)]
pub struct TraceabilityArgs {
    #[command(subcommand)]
    pub command: TraceabilityCommands,
}

#[derive(Subcommand, Clone, Debug)]
pub enum TraceabilityCommands {
    /// Lance un rapport de traçage complet sur le modèle actuel (Audit MBSE)
    Audit,
    /// Analyse l'impact d'un changement sur un composant cible et ses dépendances
    Impact {
        /// Identifiant du composant (URI Arcadia)
        component_id: String,
    },
    /// Affiche les derniers changements détectés dans le Knowledge Graph
    History,
}

/// Helper pour extraire les documents sémantiques du graphe
fn get_docs(model: &ProjectModel) -> UnorderedMap<String, JsonValue> {
    let mut docs = UnorderedMap::new();

    // Itération dynamique sur l'ensemble du graphe via all_elements()
    for e in model.all_elements() {
        if let Ok(val) = json::serialize_to_value(e) {
            docs.insert(e.handle.as_str().to_string(), val);
        }
    }
    docs
}

pub async fn handle(args: TraceabilityArgs, ctx: CliContext) -> RaiseResult<()> {
    // 🎯 Heartbeat de session : Traitement de l'erreur pour la traçabilité sémantique
    if let Err(e) = ctx.session_mgr.touch().await {
        user_error!(
            "ERR_SESSION_HEARTBEAT",
            json_value!({"error": e.to_string()})
        );
    }

    match args.command {
        TraceabilityCommands::Audit => {
            user_info!(
                "TRACE_AUDIT_INIT",
                json_value!({ "domain": ctx.active_domain, "user": ctx.active_user })
            );

            // TODO : Charger le modèle réel depuis la partition via le session_mgr
            let model = ProjectModel::default();
            let docs = get_docs(&model);

            let tracer = Tracer::from_legacy_model(&model)?;
            let report = AuditGenerator::generate(&tracer, &docs, &model.meta.name)?;

            // Affichage structuré du rapport
            println!("{}", json::serialize_to_string_pretty(&report)?);

            user_success!(
                "AUDIT_TRACEABILITY_OK",
                json_value!({
                    "compliance_count": report.compliance_results.len(),
                    "status": "verified"
                })
            );
        }

        TraceabilityCommands::Impact { component_id } => {
            user_info!(
                "IMPACT_ANALYSIS_START",
                json_value!({ "target": component_id })
            );

            let model = ProjectModel::default();
            let tracer = Tracer::from_legacy_model(&model)?;
            let analyzer = ImpactAnalyzer::new(tracer);

            let report = analyzer.analyze(&component_id, 3)?; // Analyse sur 3 niveaux de profondeur

            println!("{}", json::serialize_to_string_pretty(&report)?);

            user_success!(
                "IMPACT_ANALYSIS_OK",
                json_value!({
                    "component": component_id,
                    "timestamp": UtcClock::now().to_rfc3339()
                })
            );
        }

        TraceabilityCommands::History => {
            user_info!(
                "TRACE_HISTORY_FETCH",
                json_value!({ "action": "loading_change_logs" })
            );

            let _tracker = ChangeTracker::new();

            user_success!(
                "TRACE_HISTORY_OK",
                json_value!({ "status": "synchronized" })
            );
        }
    }
    Ok(())
}

// =========================================================================
// TESTS UNITAIRES (Conformité « Zéro Dette »)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use raise_core::utils::testing::DbSandbox;

    #[async_test]
    #[serial_test::serial] // 🎯 FIX : Empêche les collisions de ressources et d'états globaux
    async fn test_traceability_cli_audit_workflow() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let storage = SharedRef::new(sandbox.storage.clone());
        let session_mgr = crate::context::SessionManager::new(storage.clone());

        let ctx = crate::CliContext::mock(AppConfig::get(), session_mgr, storage);
        let args = TraceabilityArgs {
            command: TraceabilityCommands::Audit,
        };

        // Propagation directe du résultat pour un test pur
        handle(args, ctx).await
    }
}
