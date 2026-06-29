// FICHIER : src-tauri/src/model_engine/validators/consistency_checker.rs

use super::{ModelValidator, Severity, ValidationIssue};
use crate::json_db::jsonld::vocabulary::VocabularyRegistry;
use crate::model_engine::loader::ModelLoader;
use crate::model_engine::types::ArcadiaElement;
use crate::utils::prelude::*; // 🎯 Façade Unique RAISE

#[derive(Default)]
pub struct ConsistencyChecker;

impl ConsistencyChecker {
    pub fn new() -> Self {
        Self
    }

    /// Vérifie la logique locale (ID, Nom, Domaine des propriétés)
    /// Respecte strictement la façade sémantique RAISE.
    pub fn check_local_logic(&self, element: &ArcadiaElement) -> RaiseResult<Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        let name = element.name.as_str();

        // 1. Vérification de l'ID technique
        if element.handle.as_str().trim().is_empty() {
            issues.push(ValidationIssue {
                severity: Severity::Error,
                rule_id: "SYS_001".to_string(),
                element_id: "unknown".to_string(),
                message: format!("L'élément '{}' n'a pas d'identifiant unique (UUID).", name),
            });
        }

        // 2. Vérification du nom par défaut
        if name.trim().is_empty() || name == "Sans nom" {
            issues.push(ValidationIssue {
                severity: Severity::Warning,
                rule_id: "SYS_002".to_string(),
                element_id: element.handle.as_str().to_string(),
                message: "L'élément n'a pas de nom descriptif.".to_string(),
            });
        }

        // 3. Validation sémantique du domaine (Ontologie) via Registry global
        let registry = VocabularyRegistry::global()?;
        for prop_key in element.properties.keys() {
            if let Some(prop_def) = registry.get_property(prop_key) {
                if let Some(domain_iri) = &prop_def.domain {
                    if !registry.is_subtype_of(&element.kind, domain_iri) {
                        issues.push(ValidationIssue {
                            severity: Severity::Error,
                            rule_id: "SEM_001".to_string(),
                            element_id: element.handle.as_str().to_string(),
                            message: format!(
                                "Violation de domaine : '{}' ne peut pas s'appliquer à un '{}' (Attendu: {}).",
                                prop_def.label, element.kind.join(", "), domain_iri // 🎯 FIX format
                            ),
                        });
                    }
                }
            } else {
                user_warn!(
                    "WARN_UNKNOWN_PROPERTY",
                    json_value!({
                        "error": format!("Propriété inconnue : {}", prop_key),
                        "property_key": prop_key,
                        "element_id": element.handle.as_str().to_string(),
                        "action": "check_local_logic"
                    })
                );
            }
        }

        Ok(issues)
    }

    /// Vérifie la validité des relations (Range de l'ontologie)
    async fn check_relationships(
        &self,
        element: &ArcadiaElement,
        loader: &ModelLoader<'_>,
    ) -> RaiseResult<Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        let registry = VocabularyRegistry::global()?;

        for (prop_key, prop_val) in &element.properties {
            if let Some(prop_def) = registry.get_property(prop_key) {
                if let Some(range_iri) = &prop_def.range {
                    let target_ids = match prop_val {
                        JsonValue::String(s) => vec![s.clone()],
                        JsonValue::Array(arr) => arr
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect(),
                        _ => vec![],
                    };

                    for target_id in target_ids {
                        // On ne vérifie que les références qui ressemblent à des IDs ou URIs
                        if target_id.starts_with("http") || target_id.len() > 20 {
                            match loader.get_element(&target_id).await {
                                Ok(target_el) => {
                                    if !registry.is_subtype_of(&target_el.kind, range_iri) {
                                        issues.push(ValidationIssue {
                                            severity: Severity::Warning,
                                            rule_id: "SEM_002".to_string(),
                                            element_id: element.handle.as_str().to_string(),
                                            message: format!(
                                                "Relation invalide : La cible '{}' est de type '{}', attendu '{}' pour la propriété '{}'.",
                                                target_el.name.as_str(), target_el.kind.join(", "), range_iri, prop_def.label // 🎯 FIX format
                                            ),
                                        });
                                    }
                                }
                                Err(_) => {
                                    // Résilience : La cible est peut-être hors-scope ou non indexée
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(issues)
    }
}

#[async_interface]
impl ModelValidator for ConsistencyChecker {
    async fn validate_element(
        &self,
        element: &ArcadiaElement,
        loader: &ModelLoader<'_>,
    ) -> RaiseResult<Vec<ValidationIssue>> {
        let mut issues = self.check_local_logic(element)?;

        if !element.properties.is_empty() {
            let rel_issues = self.check_relationships(element, loader).await?;
            issues.extend(rel_issues);
        }

        Ok(issues)
    }

    /// 🎯 SCAN UNIVERSEL : Parcourt dynamiquement tout le modèle chargé.
    /// Utilise les points de montage pour la résilience de chargement.
    async fn validate_full(&self, loader: &ModelLoader<'_>) -> RaiseResult<Vec<ValidationIssue>> {
        let mut all_issues = Vec::new();

        let model = loader.load_full_model().await?;

        for el in model.all_elements() {
            let element_issues = self.validate_element(el, loader).await?;
            // ✅ extend() sur un Vec aplatit proprement les issues dans all_issues.
            all_issues.extend(element_issues);
        }

        Ok(all_issues)
    }
}

// =========================================================================
// TESTS UNITAIRES (Conformité & Résilience Mount Points)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::utils::testing::{AgentDbSandbox, DbSandbox};

    async fn inject_mock_mapping(manager: &CollectionsManager<'_>) -> RaiseResult<()> {
        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            manager.space, manager.db
        );
        manager.create_collection("configs", &schema_uri).await?;

        manager
            .upsert_document(
                "configs",
                json_value!({
                    "_id": "ref:configs:handle:ontological_mapping",
                    "search_spaces": [ { "layer": "oa", "collection": "actors" } ]
                }),
            )
            .await?;
        Ok(())
    }

    #[async_test]
    async fn test_consistency_local_logic() -> RaiseResult<()> {
        // 1. Amorçage ultra-léger du registre sémantique en mémoire (Mock)
        crate::utils::testing::mock::inject_mock_config().await;
        let checker = ConsistencyChecker::new();
        let el = ArcadiaElement {
            handle: "UUID-OK".try_into()?,
            name: I18nString::Single("ValidName".to_string()),
            kind: vec!["la:LogicalComponent".to_string()],
            ..Default::default()
        };
        let issues = checker.check_local_logic(&el)?;
        assert!(
            issues.is_empty(),
            "Un élément valide ne devrait pas générer d'issues."
        );

        Ok(())
    }

    #[async_test]
    async fn test_consistency_full_scan_dynamic() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        // 🎯 RÉSILIENCE MOUNT POINTS : Utilisation dynamique de la config système
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );
        inject_mock_mapping(&manager).await?;

        let oa_mgr = CollectionsManager::new(&sandbox.db, &config.mount_points.system.domain, "oa");
        DbSandbox::mock_db(&oa_mgr).await?;

        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            manager.space, manager.db
        );
        oa_mgr.create_collection("actors", &schema_uri).await?;

        // Insertion d'un élément avec nom vide dans la couche OA
        oa_mgr
            .insert_raw(
                "actors",
                &json_value!({
                    "_id": "ACT-EMPTY", "name": "", "type": "OperationalActor"
                }),
            )
            .await?;

        let loader = ModelLoader::new_with_manager(manager)?;
        let checker = ConsistencyChecker::new();

        let issues = checker.validate_full(&loader).await?;

        // Vérification de la détection
        let found = issues
            .iter()
            .any(|i| i.element_id == "ACT-EMPTY" && i.rule_id == "SYS_002");
        assert!(
            found,
            "Le checker doit trouver l'erreur via le scan universel"
        );

        Ok(())
    }

    ///  Résilience face à un loader défaillant (Mount Point corrompu)
    #[async_test]
    async fn test_resilience_loader_failure() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        // Manager pointant sur une partition inexistante
        let manager = CollectionsManager::new(&sandbox.db, "ghost_partition", "void_db");
        let loader = ModelLoader::new_with_manager(manager)?;
        let checker = ConsistencyChecker::new();

        // Le scan ne doit pas paniquer mais renvoyer une liste vide ou loguer une erreur
        let issues = checker.validate_full(&loader).await?;
        assert!(issues.is_empty());
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Inférence des domaines système configurés
    #[async_test]
    async fn test_mount_point_resolution_validator() -> RaiseResult<()> {
        let config = AppConfig::get();
        // On s'assure que le validateur peut s'appuyer sur les points de montage SSOT
        assert!(!config.mount_points.system.domain.is_empty());
        assert!(!config.mount_points.system.db.is_empty());
        Ok(())
    }
}
