// FICHIER : src-tauri/src/model_engine/validators/ontological_validator.rs

use super::{ModelValidator, Severity, ValidationIssue};
use crate::model_engine::arcadia::ArcadiaOntology;
use crate::model_engine::loader::ModelLoader;
use crate::model_engine::types::ArcadiaElement;
use crate::utils::prelude::*;

#[derive(Default)]
pub struct OntologicalValidator;

impl OntologicalValidator {
    pub fn new() -> Self {
        Self
    }

    /// Logique de validation pilotée par les données (Data-Driven)
    pub fn check_semantics(&self, element: &ArcadiaElement) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        // 🎯 FIX V2 : On vérifie chaque type déclaré
        for k in &element.kind {
            let kind_str = k.as_str();

            if kind_str != "Unknown" && !ArcadiaOntology::is_known_type(kind_str) {
                issues.push(ValidationIssue {
                    rule_id: "ONTO-001".to_string(),
                    severity: Severity::Warning,
                    element_id: element.handle.as_str().to_string(),
                    message: format!(
                        "Sémantique inconnue ou non-mappée dans l'ontologie : '{}'",
                        kind_str
                    ),
                });
            }
        }

        issues
    }
}

#[async_interface]
impl ModelValidator for OntologicalValidator {
    async fn validate_element(
        &self,
        element: &ArcadiaElement,
        _loader: &ModelLoader<'_>,
    ) -> RaiseResult<Vec<ValidationIssue>> {
        Ok(self.check_semantics(element))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ontological_validation() -> RaiseResult<()> {
        let validator = OntologicalValidator::new();

        // 🎯 On teste avec l'état "Unknown" (Brouillon en cours de modélisation)
        // C'est la règle métier qui permet de contourner le registre strict.
        let valid_draft = ArcadiaElement {
            handle: "draft-001".try_into()?,
            name: I18nString::Single("Brouillon".to_string()),
            kind: vec!["Unknown".to_string()], // 🎯 FIX
            ..Default::default()
        };

        let issues = validator.check_semantics(&valid_draft);
        assert_eq!(
            issues.len(),
            0,
            "Un brouillon ('Unknown') ne doit pas lever d'erreur ontologique"
        );

        // 🎯 On teste avec un type inventé (qui sera rejeté par le mock vide)
        let invalid_element = ArcadiaElement {
            handle: "err-001".try_into()?,
            name: I18nString::Single("Erreur Magique".to_string()),
            kind: vec!["MagicalEntity".to_string()], // 🎯 FIX
            ..Default::default()
        };

        let issues_err = validator.check_semantics(&invalid_element);
        assert_eq!(
            issues_err.len(),
            1,
            "Une erreur doit être levée pour un type inexistant"
        );
        assert_eq!(issues_err[0].rule_id, "ONTO-001");

        Ok(())
    }
}
