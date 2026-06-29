// FICHIER : src-tauri/src/model_engine/validators/compliance_validator.rs

use crate::model_engine::loader::ModelLoader;
use crate::model_engine::types::ArcadiaElement;
use crate::model_engine::validators::{ModelValidator, Severity, ValidationIssue};
use crate::utils::prelude::*;

/// Validateur de conformité méthodologique (Best Practices Arcadia)
#[derive(Default)]
pub struct ComplianceValidator;

impl ComplianceValidator {
    pub fn new() -> Self {
        Self
    }

    /// Analyse un seul élément pour vérifier la qualité des données de base
    fn check_quality(&self, element: &ArcadiaElement) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        let name = element.name.as_str().trim();

        // 1. Règle de Nommage (RULE_NAMING)
        if name.is_empty() || name == "Unnamed" || name == "Sans nom" || name.starts_with("Copy of")
        {
            issues.push(ValidationIssue {
                severity: Severity::Warning,
                element_id: element.handle.as_str().to_string(),
                message: format!("L'élément possède un nom générique ou vide : '{}'.", name),
                rule_id: "RULE_NAMING".to_string(),
            });
        }

        // 2. Règle de Documentation (RULE_DOC)
        // 🎯 PURE GRAPH : On cherche le champ "description" dans la map properties
        let has_description = element
            .properties
            .get("description")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);

        if !has_description {
            issues.push(ValidationIssue {
                severity: Severity::Info,
                element_id: element.handle.as_str().to_string(),
                message: format!("Documentation manquante pour l'élément '{}'.", name),
                rule_id: "RULE_DOC".to_string(),
            });
        }

        issues
    }
}

#[async_interface]
impl ModelValidator for ComplianceValidator {
    /// Validation d'un élément isolé
    async fn validate_element(
        &self,
        element: &ArcadiaElement,
        _loader: &ModelLoader<'_>,
    ) -> RaiseResult<Vec<ValidationIssue>> {
        Ok(self.check_quality(element))
    }

    /// Validation complète du modèle chargé
    async fn validate_full(&self, loader: &ModelLoader<'_>) -> RaiseResult<Vec<ValidationIssue>> {
        let mut all_issues = Vec::new();

        // On charge le snapshot complet du modèle
        if let Ok(model) = loader.load_full_model().await {
            // 🎯 PURE GRAPH : On utilise l'itérateur universel sur toutes les couches
            for element in model.all_elements() {
                all_issues.extend(self.check_quality(element));
            }
        }

        Ok(all_issues)
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_engine::types::ProjectModel;

    /// Helper pour créer un élément conforme aux types Pure Graph
    fn mock_element(
        id: &str,
        name: &str,
        description: Option<&str>,
    ) -> RaiseResult<ArcadiaElement> {
        let mut properties = UnorderedMap::new();
        if let Some(desc) = description {
            properties.insert("description".to_string(), json_value!(desc));
        }

        Ok(ArcadiaElement {
            handle: id.try_into()?,
            name: I18nString::Single(name.to_string()),
            kind: vec!["sa:SystemFunction".to_string()],
            properties,
            ..Default::default()
        })
    }

    #[test]
    fn test_quality_check_valid_element() -> RaiseResult<()> {
        let validator = ComplianceValidator::new();
        let el = mock_element("1", "Vérifier Pression", Some("Analyse les capteurs"))?;

        let issues = validator.check_quality(&el);
        assert!(
            issues.is_empty(),
            "Un élément bien rempli ne doit produire aucune issue"
        );

        Ok(())
    }

    #[test]
    fn test_quality_check_naming_warning() -> RaiseResult<()> {
        let validator = ComplianceValidator::new();

        // Cas : Nom par défaut
        let el_unnamed = mock_element("2", "Unnamed", Some("Desc"))?;
        let issues = validator.check_quality(&el_unnamed);
        assert!(issues.iter().any(|i| i.rule_id == "RULE_NAMING"));

        // Cas : Nom vide
        let el_empty = mock_element("3", "", Some("Desc"))?;
        let issues_empty = validator.check_quality(&el_empty);
        assert!(issues_empty.iter().any(|i| i.rule_id == "RULE_NAMING"));

        Ok(())
    }

    #[test]
    fn test_quality_check_documentation_info() -> RaiseResult<()> {
        let validator = ComplianceValidator::new();

        // Cas : Description manquante
        let el_no_doc = mock_element("4", "Action", None)?;
        let issues = validator.check_quality(&el_no_doc);
        assert!(issues.iter().any(|i| i.rule_id == "RULE_DOC"));
        assert_eq!(issues[0].severity, Severity::Info);

        Ok(())
    }

    #[test]
    fn test_all_elements_scan_integration() -> RaiseResult<()> {
        let validator = ComplianceValidator::new();
        let mut model = ProjectModel::default();

        // On mélange des éléments valides et invalides dans différentes couches
        model.add_element("oa", "actors", mock_element("A1", "Unnamed", None)?); // 2 erreurs
        model.add_element("sa", "functions", mock_element("F1", "Valid", Some("Doc"))?); // 0 erreur

        // Simulation manuelle de ce que ferait validate_full (puisque loader requiert le FS)
        let mut total_issues = 0;
        for el in model.all_elements() {
            total_issues += validator.check_quality(el).len();
        }

        assert_eq!(
            total_issues, 2,
            "Seul l'acteur sans nom et sans doc doit être relevé"
        );

        Ok(())
    }
}
