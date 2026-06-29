// FICHIER : src-tauri/src/model_engine/validators/mod.rs

pub mod compliance_validator;
pub mod consistency_checker;
pub mod dynamic_validator;
pub mod ontological_validator;

use crate::utils::prelude::*;

use crate::model_engine::loader::ModelLoader;
use crate::model_engine::types::ArcadiaElement;

// Re-exports pour faciliter l'usage externe
pub use compliance_validator::ComplianceValidator;
pub use consistency_checker::ConsistencyChecker;
pub use dynamic_validator::DynamicValidator;
pub use ontological_validator::OntologicalValidator;

/// Niveau de sévérité d'un problème de validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serializable, Deserializable)]
pub enum Severity {
    Error,   // Bloquant / Rouge
    Warning, // Avertissement / Jaune
    Info,    // Suggestion / Bleu
}

/// Représente un problème détecté dans le modèle.
#[derive(Debug, Clone, Serializable, Deserializable, PartialEq)]
pub struct ValidationIssue {
    pub severity: Severity,
    pub rule_id: String,
    pub element_id: String,
    pub message: String,
}

/// Trait commun que tous les validateurs doivent implémenter.
/// Refactorisé pour le Lazy Loading et la validation incrémentale.
#[async_interface]
pub trait ModelValidator: Send + Sync {
    /// Valide un élément unique (Contexte Temps Réel).
    /// Le Loader est fourni pour permettre des vérifications croisées (ex: vérifier l'existence d'une cible).
    async fn validate_element(
        &self,
        element: &ArcadiaElement,
        loader: &ModelLoader<'_>,
    ) -> RaiseResult<Vec<ValidationIssue>>;

    /// Valide l'ensemble du modèle (Batch).
    /// Utile pour les rapports CI/CD ou les vérifications globales.
    /// Par défaut, retourne vide (à implémenter si nécessaire en itérant sur le loader).
    async fn validate_full(&self, _loader: &ModelLoader<'_>) -> RaiseResult<Vec<ValidationIssue>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_db::storage::{JsonDbConfig, StorageEngine};

    // Mock d'un validateur simple pour tester le trait
    struct MockValidator;

    #[async_interface]
    impl ModelValidator for MockValidator {
        async fn validate_element(
            &self,
            element: &ArcadiaElement,
            _loader: &ModelLoader<'_>,
        ) -> RaiseResult<Vec<ValidationIssue>> {
            if element.name.as_str() == "Invalid" {
                Ok(vec![ValidationIssue {
                    severity: Severity::Error,
                    rule_id: "MOCK_RULE".to_string(),
                    element_id: element.handle.as_str().to_string(),
                    message: "Invalid name".to_string(),
                }])
            } else {
                Ok(vec![])
            }
        }
    }

    #[async_test]
    async fn test_model_validator_trait_integration() -> RaiseResult<()> {
        crate::utils::testing::mock::inject_mock_config().await;
        // 1. Setup minimal du Loader (nécessaire pour la signature)
        let dir = tempdir().unwrap();
        let config = JsonDbConfig::new(dir.path().to_path_buf());
        let storage = StorageEngine::new(config)?;
        // On utilise new_with_manager pour éviter de dépendre de l'état global Tauri
        let loader = ModelLoader::new_with_manager(
            crate::json_db::collections::manager::CollectionsManager::new(
                &storage,
                "test_space",
                "test_db",
            ),
        )?;

        // 2. Création élément
        let el = ArcadiaElement {
            handle: "1".try_into()?,
            name: I18nString::Single("Invalid".to_string()),
            kind: vec!["Test".to_string()],
            properties: UnorderedMap::new(),
            ..Default::default()
        };

        // 3. Validation
        let validator = MockValidator;
        let issues = validator.validate_element(&el, &loader).await?;

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].message, "Invalid name");

        Ok(())
    }
}
