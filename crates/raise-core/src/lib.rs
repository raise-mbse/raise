// FICHIER : crates/raise-core/src/lib.rs

pub mod ai;
pub mod blockchain;
pub mod code_generator;
pub mod genetics;
pub mod json_db;
pub mod kernel;
pub mod model_engine;
pub mod plugins;
pub mod rules_engine;
pub mod spatial_engine;
pub mod traceability;
pub mod utils;
pub mod workflow_engine;

pub mod services;

use crate::model_engine::types::ProjectModel;
use crate::utils::prelude::*;

pub struct AppState {
    pub model: SharedRef<AsyncMutex<ProjectModel>>,
}

/// 🚀 BOOTSTRAP UNIFIÉ (Zéro Dette)
/// Prépare tous les moteurs système (WAL, i18n, Sémantique, Rules Engine)
pub async fn bootstrap_core(
    manager: &json_db::collections::manager::CollectionsManager<'_>,
) -> RaiseResult<()> {
    let config = AppConfig::get();

    // 1. 🛡️ MOTEUR DE RÉSILIENCE (WAL Crash Recovery)
    // On accède directement aux champs publics du manager
    match json_db::transactions::wal::recover_pending_transactions(
        &manager.storage.config,
        &manager.space,
        &manager.db,
        manager.storage,
    )
    .await
    {
        Ok(count) if count > 0 => {
            user_warn!("WRN_DB_CRASH_RECOVERED", json_value!({"recovered": count}));
        }
        Ok(_) => {
            // ✅ AJOUT : Confirmation visuelle que l'état disque est propre
            user_success!("RESILIENCE_ENGINE_READY", json_value!({"status": "clean"}));
        }
        Err(e) => {
            // La résilience est critique : on bloque le boot si elle échoue
            raise_error!("ERR_DB_RECOVERY_FATAL", error = e.to_string());
        }
    }

    // 2. 🌍 LOCALISATION (i18n)
    // 🎯 RÉSILIENCE : On loggue un warning mais on ne bloque pas le démarrage (Mode dégradé)
    if let Err(e) = crate::utils::context::init_i18n(&config.core.language).await {
        user_warn!(
            "WRN_I18N_LOAD_FAILED",
            json_value!({"error": e.to_string(), "lang": config.core.language})
        );
    } else {
        // ✅ AJOUT : Confirmation visuelle de la langue chargée
        user_success!(
            "I18N_READY",
            json_value!({"language": config.core.language})
        );
    }

    // 3. 🧠 INITIALISATION SÉMANTIQUE (Ontologie)
    // 🎯 RÉSILIENCE : On permet le démarrage sans sémantique pour la maintenance
    if let Err(e) = json_db::jsonld::VocabularyRegistry::init_from_db(manager).await {
        user_warn!(
            "WRN_SEMANTIC_BOOT_DEGRADED",
            json_value!({"error": e.to_string()})
        );
    } else {
        // ✅ AJOUT : Confirmation visuelle de l'index sémantique
        user_success!("SEMANTIC_ENGINE_READY", json_value!({"mode": "in-index"}));
    }

    // 4. 🎯 MOTEUR DE RÈGLES
    // Cette étape crée la collection '_system_rules' si absente
    rules_engine::initialize_rules_engine(manager).await?;
    user_success!("RULES_ENGINE_READY");

    Ok(())
}

// =========================================================================
// TESTS UNITAIRES (Bootstrap & Intégrité Core)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::utils::testing::DbSandbox;

    // Import explicite de l'attribut macro pour le compilateur
    use crate::utils::prelude::async_test;

    /// 🎯 TEST ROBUSTE : Validation de la séquence complète de démarrage
    #[async_test]
    async fn test_bootstrap_core_full_flow() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let config = AppConfig::get();

        let manager = CollectionsManager::new(
            &sandbox.storage,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        DbSandbox::mock_db(&manager).await?;

        // 🎯 ALIGNEMENT DEMANDÉ : Utilisation du schéma générique pour i18n
        let generic_schema = "db://_system/_system/schemas/v1/db/generic.schema.json";
        manager.create_collection("locales", generic_schema).await?;

        // On injecte la langue pour valider le chargement sans erreur
        manager
            .insert_raw(
                "locales",
                &json_value!({
                    "_id": "en",
                    "handle": "en",
                    "values": { "I18N_READY": "Ready" }
                }),
            )
            .await?;

        // 🎯 SEEDING ONTOLOGIE : Simulation d'une ontologie minimale
        {
            let lock = manager
                .storage
                .get_index_lock(&manager.space, &manager.db)?;
            let guard = lock.lock().await;
            let mut tx = manager.begin_system_tx(&guard).await?;
            tx.document["ontologies"]["raise"] = json_value!({
                "uri": "db://_system/bootstrap/ontology/raise.jsonld",
                "version": "1.0"
            });
            tx.commit().await?;
        }

        // ACTION : Exécution du Bootstrap (Doit maintenant renvoyer Ok)
        bootstrap_core(&manager).await?;

        // ASSERTIONS
        let collections = manager.list_collections().await?;
        assert!(
            collections.contains(&"_system_rules".to_string()),
            "La collection '_system_rules' aurait dû être créée par le bootstrap."
        );

        Ok(())
    }
}
