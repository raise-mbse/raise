// FICHIER : src-tauri/src/ai/training/dataset.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::model_engine::types::ArcadiaElement;
use crate::utils::prelude::*; // 🎯 Façade Unique

/// Fournit un flux continu de paires (État_t, État_t+1) pour le World Model.
pub struct ArcadiaBatchIterator<'a> {
    manager: &'a CollectionsManager<'a>,
    collections: Vec<String>,
    current_col_idx: usize,
    batch_size: usize,
}

impl<'a> ArcadiaBatchIterator<'a> {
    pub async fn new(manager: &'a CollectionsManager<'a>, batch_size: usize) -> RaiseResult<Self> {
        let collections = manager.list_collections().await.unwrap_or_default();
        Ok(Self {
            manager,
            collections,
            current_col_idx: 0,
            batch_size,
        })
    }

    /// Extrait le prochain batch d'ArcadiaElements directement depuis la DB.
    pub async fn next_batch(&mut self) -> RaiseResult<Option<Vec<ArcadiaElement>>> {
        while self.current_col_idx < self.collections.len() {
            let col_name = &self.collections[self.current_col_idx];

            // Note: En conditions réelles, on utiliserait un curseur de pagination de la DB.
            // Ici on simule le découpage pour l'exemple.
            if let Ok(docs) = self.manager.list_all(col_name).await {
                if !docs.is_empty() {
                    let mut batch = Vec::with_capacity(self.batch_size);
                    for doc in docs.into_iter().take(self.batch_size) {
                        let element = ArcadiaElement {
                            id: doc
                                .get("handle")
                                .or_else(|| doc.get("_id"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown")
                                .to_string(),
                            name: crate::model_engine::types::NameType::default(),
                            kind: "https://raise.io/ontology/arcadia/pa#PhysicalComponent"
                                .to_string(),
                            properties: UnorderedMap::new(),
                        };
                        batch.push(element);
                    }
                    self.current_col_idx += 1;
                    return Ok(Some(batch));
                }
            }
            self.current_col_idx += 1;
        }
        Ok(None) // Fin du dataset
    }
}

#[derive(Debug, Serializable, Deserializable, Clone, PartialEq)]
pub struct TrainingExample {
    pub instruction: String,
    pub input: String,
    pub output: String,
}

/// Extrait les données spécifiquement pour un domaine métier à partir du Graphe de Connaissance.
/// Cette fonction alimente le moteur d'entraînement natif en respectant les points de montage.
pub async fn extract_domain_data(
    manager: &CollectionsManager<'_>,
    domain: &str,
) -> RaiseResult<Vec<TrainingExample>> {
    let mut dataset = Vec::new();

    // 1. Récupération de la liste des collections via le manager
    // 🎯 Rigueur : Utilisation de Match...raise_error au lieu de expect/unwrap
    let collections = match manager.list_collections().await {
        Ok(c) => c,
        Err(e) => {
            raise_error!(
                "ERR_TRAINING_DATASET_LIST_FAILED",
                error = e.to_string(),
                context = json_value!({
                    "space": manager.space,
                    "action": "list_collections"
                })
            );
        }
    };

    for col in collections {
        // Filtrage sémantique par domaine (ou "all" pour le dataset global)
        if !col.contains(domain) && domain != "all" {
            continue;
        }

        // 2. Extraction du lot de documents
        let docs = match manager.list_all(&col).await {
            Ok(d) => d,
            Err(e) => {
                raise_error!(
                    "ERR_TRAINING_DATASET_FETCH_FAILED",
                    error = e.to_string(),
                    context = json_value!({ "collection": col })
                );
            }
        };

        // 3. Transformation en exemples d'entraînement (Synthetic Augmentation)
        for doc in docs {
            dataset.push(TrainingExample {
                instruction: format!("Analyser cet élément technique du domaine {}.", domain),
                input: match json::serialize_to_string(&doc) {
                    Ok(s) => s,
                    Err(_) => continue, // On ignore les documents corrompus
                },
                output: format!(
                    "L'entité appartient à la collection '{}' dans l'espace projet '{}'.",
                    col, manager.space
                ),
            });
        }
    }

    user_info!(
        "MSG_TRAINING_DATASET_READY",
        json_value!({ "domain": domain, "samples": dataset.len() })
    );

    Ok(dataset)
}

// =========================================================================
// TESTS UNITAIRES (Rigueur Façade & Résilience des Domaines)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::testing::AgentDbSandbox;

    /// Test existant : Filtrage par domaine
    #[async_test]
    #[serial_test::serial] // Sécurité : L'orchestrateur charge l'IA
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_extract_domain_data_filtering() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        // 🎯 FIX MOUNT POINTS : Utilisation du domaine système configuré
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        // 🎯 ZÉRO DETTE : On compte dynamiquement les documents initiaux de la Sandbox (ex: dapps, migrations)
        let initial_count = extract_domain_data(&manager, "all").await?.len();

        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );

        manager
            .create_collection("safety_rules", &schema_uri)
            .await?;
        manager
            .create_collection("general_info", &schema_uri)
            .await?;

        let doc = json_value!({"_id": "1", "content": "test"});
        manager.insert_raw("safety_rules", &doc).await?;
        manager.insert_raw("general_info", &doc).await?;

        let results = extract_domain_data(&manager, "safety").await?;

        assert_eq!(
            results.len(),
            1,
            "Devrait trouver uniquement la collection safety"
        );
        assert!(results[0].instruction.contains("safety"));

        let all_results = extract_domain_data(&manager, "all").await?;
        assert_eq!(
            all_results.len(),
            initial_count + 2,
            "Devrait trouver les collections initiales + les 2 ajoutées"
        );

        Ok(())
    }

    /// Test existant : Comportement sur domaine inconnu
    #[async_test]
    #[serial_test::serial] // Sécurité : L'orchestrateur charge l'IA
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_extract_empty_domain() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let results = extract_domain_data(&manager, "nonexistent").await?;
        assert!(
            results.is_empty(),
            "Le dataset devrait être vide pour un domaine inconnu"
        );
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Résilience face à une erreur de manager (Partition manquante)
    #[async_test]
    #[serial_test::serial] // Sécurité : L'orchestrateur charge l'IA
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_extract_resilience_on_invalid_mount() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;

        // On crée un manager pointant vers un espace non initialisé
        let manager = CollectionsManager::new(&sandbox.db, "ghost_partition", "void_db");

        // Le système est tellement résilient sur les lectures qu'il ne panique pas.
        // Un espace de base de données inexistant équivaut simplement à "zéro collection".
        let results = extract_domain_data(&manager, "all").await?;

        assert!(
            results.is_empty(),
            "Le système doit survivre et renvoyer un dataset vide pour une partition fantôme"
        );
        Ok(())
    }
}
