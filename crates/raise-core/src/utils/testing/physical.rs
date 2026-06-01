// FICHIER : crates/raise-core/src/utils/testing/physical.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::StorageEngine;
use crate::kernel::environment::NodeEnvironment;
use crate::utils::core::error::RaiseResult;
use crate::utils::core::SharedRef;
use crate::utils::data::config::AppConfig;
use crate::utils::data::json::{self, json_value, JsonValue};
use crate::utils::io::fs;
use crate::{raise_error, user_info, user_success};

/// Environnement de test frappant le vrai disque dur physique de la machine.
/// Utilise le principe de l'espace de noms (Namespace) pour isoler les données.
pub struct PhysicalTestSandbox {
    pub storage: SharedRef<StorageEngine>,
    pub domain: String,
    pub sim_db: String,
}

impl PhysicalTestSandbox {
    /// Amorce l'environnement de test sur le disque réel.
    /// Ex: boot("_system", "raise_core") va créer et monter "sim_raise_core".
    pub async fn boot(domain: &str, target_db: &str) -> RaiseResult<Self> {
        // 1. Amorçage garanti du système physique principal
        let node_env = match NodeEnvironment::boot_physical_node().await {
            Ok(env) => env,
            Err(e) => raise_error!("ERR_TEST_PHYSICAL_BOOT", error = e),
        };

        let sim_db = format!("sim_{}", target_db);
        let config = AppConfig::get();

        let domain_root = match config.get_path("PATH_RAISE_DOMAIN") {
            Some(p) => p,
            None => raise_error!("ERR_TEST_NO_DOMAIN", error = "PATH_RAISE_DOMAIN manquant"),
        };

        // 2. Amorçage Miroir (Mirror Bootstrapping)
        let source_db_path = domain_root.join(domain).join(target_db);
        let sim_db_path = domain_root.join(domain).join(&sim_db);

        // Si la base de simulation n'existe pas, on clone l'ADN de la base réelle
        if !fs::exists_async(&sim_db_path).await {
            user_info!("TEST_MIRROR_START", json_value!({"source": target_db, "target": &sim_db}));

            match fs::create_dir_all_async(&sim_db_path).await {
                Ok(_) => (),
                Err(e) => raise_error!("ERR_TEST_MKDIR_SIM", error = e),
            }

            // A. Clonage de l'Index Système
            let source_index = source_db_path.join("_system.json");
            let sim_index = sim_db_path.join("_system.json");
            
            if fs::exists_async(&source_index).await {
                match fs::copy_async(&source_index, &sim_index).await {
                    Ok(_) => (),
                    Err(e) => raise_error!("ERR_TEST_COPY_INDEX", error = e),
                }
                
                // Mutation génétique : on assigne le rôle de simulation à l'index cloné
                let content = match fs::read_to_string_async(&sim_index).await {
                    Ok(c) => c,
                    Err(e) => raise_error!("ERR_TEST_READ_INDEX", error = e),
                };
                let mut index_doc: JsonValue = match json::deserialize_from_str(&content) {
                    Ok(d) => d,
                    Err(e) => raise_error!("ERR_TEST_PARSE_INDEX", error = e),
                };
                if let Some(obj) = index_doc.as_object_mut() {
                    obj.insert("db_role".to_string(), json_value!("simulation"));
                    obj.insert("name".to_string(), json_value!(&sim_db));
                    obj.insert("database".to_string(), json_value!(&sim_db));
                }
                match fs::write_json_atomic_async(&sim_index, &index_doc).await {
                    Ok(_) => (),
                    Err(e) => raise_error!("ERR_TEST_WRITE_INDEX", error = e),
                }
            }

            // B. Clonage Strict des Schémas de Validation
            let source_schemas = source_db_path.join("schemas");
            let sim_schemas = sim_db_path.join("schemas");
            if fs::exists_async(&source_schemas).await {
                match fs::copy_dir_recursive(&source_schemas, &sim_schemas).await {
                    Ok(_) => (),
                    Err(e) => raise_error!("ERR_TEST_COPY_SCHEMAS", error = e),
                }
            }

            // C. Clonage des Ontologies et Métadonnées Métier
            let source_onto = source_db_path.join("collections").join("_ontologies");
            let sim_onto = sim_db_path.join("collections").join("_ontologies");
            if fs::exists_async(&source_onto).await {
                match fs::copy_dir_recursive(&source_onto, &sim_onto).await {
                    Ok(_) => (),
                    Err(e) => raise_error!("ERR_TEST_COPY_ONTO", error = e),
                }
            }

            user_success!("TEST_MIRROR_COMPLETE", json_value!({"sim_db": &sim_db}));
        }

        Ok(Self {
            storage: node_env.storage,
            domain: domain.to_string(),
            sim_db,
        })
    }

    /// Fournit un manager directement branché sur la base de simulation
    pub fn manager(&self) -> CollectionsManager<'_> {
        CollectionsManager::new(&self.storage, &self.domain, &self.sim_db)
    }

    /// Nettoie la base de simulation après l'exécution des tests (Garbage Collection)
    pub async fn teardown(&self) -> RaiseResult<()> {
        let config = AppConfig::get();
        let domain_root = config.get_path("PATH_RAISE_DOMAIN").unwrap();
        let sim_db_path = domain_root.join(&self.domain).join(&self.sim_db);
        
        if fs::exists_async(&sim_db_path).await {
            match fs::remove_dir_all_async(&sim_db_path).await {
                Ok(_) => Ok(()),
                Err(e) => raise_error!("ERR_TEST_TEARDOWN", error = e),
            }
        } else {
            Ok(())
        }
    }

 
}


// ==============================================================================
// 🧪 TESTS UNITAIRES (Validation du Harnais Physique)
// ==============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::core::async_test;

    // 🎯 On cible la base de production 'master' du domaine système 
    // qui est garantie d'exister grâce au SystemBootstrapper.
    const TEST_DOMAIN: &str = "_system";
    const TEST_SOURCE_DB: &str = "master";

    #[async_test]
    #[serial_test::serial] // 🛡️ Verrouillage séquentiel obligatoire pour les I/O physiques
    async fn test_physical_sandbox_lifecycle() -> RaiseResult<()> {
        let _ = AppConfig::init();
        
        // 1. Amorçage (Boot)
        let sandbox = PhysicalTestSandbox::boot(TEST_DOMAIN, TEST_SOURCE_DB).await?;
        
        // Vérification de la création du dossier
        let config = AppConfig::get();
        let domain_root = config.get_path("PATH_RAISE_DOMAIN").unwrap();
        let sim_db_path = domain_root.join(TEST_DOMAIN).join(&sandbox.sim_db);
        
        assert!(
            fs::exists_async(&sim_db_path).await, 
            "Le dossier physique de simulation n'a pas été créé"
        );
        
        // 2. Vérification de l'accès au manager
        let manager = sandbox.manager();
        assert_eq!(manager.db, "sim_master", "Le manager ne pointe pas vers la base de simulation");

        // 3. Nettoyage (Teardown)
        sandbox.teardown().await?;
        
        assert!(
            !fs::exists_async(&sim_db_path).await, 
            "Le dossier de simulation n'a pas été supprimé par le teardown (Fuite de données !)"
        );

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_physical_sandbox_mirror_genetics() -> RaiseResult<()> {
        let _ = AppConfig::init();
        
        let sandbox = PhysicalTestSandbox::boot(TEST_DOMAIN, TEST_SOURCE_DB).await?;
        
        let config = AppConfig::get();
        let domain_root = config.get_path("PATH_RAISE_DOMAIN").unwrap();
        let sim_db_path = domain_root.join(TEST_DOMAIN).join(&sandbox.sim_db);

        // 1. Validation de la mutation génétique de l'Index (_system.json)
        let sys_json_path = sim_db_path.join("_system.json");
        assert!(fs::exists_async(&sys_json_path).await, "Le fichier _system.json n'a pas été cloné");

        let content = fs::read_to_string_async(&sys_json_path).await?;
        let index_doc: JsonValue = crate::utils::data::json::deserialize_from_str(&content).unwrap();

        assert_eq!(
            index_doc.get("db_role").and_then(|v| v.as_str()), 
            Some("simulation"),
            "Le rôle de la base n'a pas été muté en 'simulation'"
        );
        
        assert_eq!(
            index_doc.get("name").and_then(|v| v.as_str()), 
            Some(sandbox.sim_db.as_str()),
            "Le nom de la base n'a pas été mis à jour dans l'index"
        );

        // 2. Validation du clonage des structures de gouvernance
        let schemas_path = sim_db_path.join("schemas");
        assert!(
            fs::exists_async(&schemas_path).await, 
            "Les schémas de validation n'ont pas été copiés vers la simulation"
        );

        let ontologies_path = sim_db_path.join("collections").join("_ontologies");
        assert!(
            fs::exists_async(&ontologies_path).await, 
            "Les ontologies (sémantique) n'ont pas été copiées vers la simulation"
        );

        // Nettoyage
        sandbox.teardown().await?;
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_physical_sandbox_idempotency() -> RaiseResult<()> {
        let _ = AppConfig::init();
        
        // 1. Premier Boot
        let sandbox1 = PhysicalTestSandbox::boot(TEST_DOMAIN, TEST_SOURCE_DB).await?;
        
        // 2. Deuxième Boot immédiat (Doit ignorer la création silencieusement)
        let sandbox2 = PhysicalTestSandbox::boot(TEST_DOMAIN, TEST_SOURCE_DB).await?;
        
        assert_eq!(
            sandbox1.sim_db, 
            sandbox2.sim_db,
            "Les deux instances ne pointent pas vers la même base de simulation"
        );

        // 3. Test de robustesse : le manager doit toujours pouvoir insérer un document
        let manager = sandbox2.manager();
        manager.create_collection("test_sandbox_collection", "db://_system/master/schemas/v1/db/generic.schema.json").await?;
        
        let doc = json_value!({ "_id": "test_id_123", "status": "active" });
        manager.upsert_document("test_sandbox_collection", doc).await?;

        // 4. Nettoyage (une seule fois suffit car pointent vers le même dossier)
        sandbox1.teardown().await?;
        
        Ok(())
    }
}