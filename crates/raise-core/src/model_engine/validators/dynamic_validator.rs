// FICHIER : src-tauri/src/model_engine/validators/dynamic_validator.rs

use crate::model_engine::loader::ModelLoader;
use crate::model_engine::types::ArcadiaElement;
use crate::model_engine::validators::{ModelValidator, Severity, ValidationIssue};
use crate::rules_engine::ast::Rule;
use crate::rules_engine::evaluator::Evaluator;
use crate::utils::prelude::*;

/// Validateur piloté par les données (Data-Driven Rules)
pub struct DynamicValidator {
    rules: Vec<Rule>,
}

impl DynamicValidator {
    pub fn new(rules: Vec<Rule>) -> Self {
        Self { rules }
    }

    /// Prépare le contexte JSON pour l'évaluation de la règle
    /// 🎯 PURE GRAPH : On aplatit l'élément et ses propriétés dynamiques
    fn build_context(element: &ArcadiaElement) -> JsonValue {
        let mut context = json_value!({
            "_id": element.handle.as_str().to_string(),
            "name": element.name.as_str(),
            "kind": element.kind
        });

        // Injection de toutes les propriétés dynamiques à la racine du contexte
        if let Some(obj) = context.as_object_mut() {
            for (key, value) in &element.properties {
                obj.insert(key.clone(), value.clone());
            }
        }
        context
    }
}

#[async_interface]
impl ModelValidator for DynamicValidator {
    /// Valide un élément spécifique contre toutes les règles applicables
    async fn validate_element(
        &self,
        element: &ArcadiaElement,
        loader: &ModelLoader<'_>,
    ) -> RaiseResult<Vec<ValidationIssue>> {
        let mut issues = Vec::new();
        let context = Self::build_context(element);

        for rule in &self.rules {
            // 🎯 GARDE D'INTÉGRITÉ : On extrait l'UUID. Si la règle n'est pas persistée (None), on l'ignore.
            if let Some(technical_uuid) = &rule._id {
                // Une règle s'applique si la cible est "all" ou si le type (URI) contient la cible
                if rule.target == "all" || element.kind.iter().any(|k| k.contains(&rule.target)) {
                    // Évaluation de l'expression de la règle via le Rules Engine
                    if let Ok(result) = Evaluator::evaluate(&rule.expr, &context, loader).await {
                        // Si l'expression retourne 'false', une issue est créée
                        if result.as_bool() == Some(false) {
                            issues.push(ValidationIssue {
                                severity: Severity::Warning,
                                rule_id: technical_uuid.clone(),
                                element_id: element.handle.as_str().to_string(),
                                message: rule.description.clone().unwrap_or_else(|| {
                                    format!("Violation de la règle dynamique : {}", rule.handle)
                                }),
                            });
                        }
                    }
                }
            }
        }
        Ok(issues)
    }

    /// Scan universel de toutes les règles sur tout le modèle
    async fn validate_full(&self, loader: &ModelLoader<'_>) -> RaiseResult<Vec<ValidationIssue>> {
        let mut all_issues = Vec::new();
        let model = loader.load_full_model().await?;

        // 🎯 PURE GRAPH : Itération universelle sans distinction de couches
        for element in model.all_elements() {
            let element_issues = self.validate_element(element, loader).await?;
            all_issues.extend(element_issues);
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
    use crate::rules_engine::ast::Expr;
    use crate::utils::testing::AgentDbSandbox;

    #[async_test]
    async fn test_dynamic_rule_on_properties() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let loader = ModelLoader::from_engine(&sandbox.db, "test", "db")?;

        // Règle : La masse doit être inférieure à 500 (mass < 500)
        let rule = Rule {
            // 🎯 FIX : Un faux _id est obligatoire pour que la règle passe la "garde d'intégrité"
            _id: Some("mock-uuid-mass-1234".into()),
            handle: "CHECK_MASS".into(),
            target: "all".into(),
            expr: Expr::Lt(
                Box::new(Expr::Var("mass".to_string())),
                Box::new(Expr::Val(json_value!(500))),
            ),
            description: Some("L'élément est trop lourd".into()),
            severity: None,
        };

        let validator = DynamicValidator::new(vec![rule]);

        // 1. Cas Valide (masse = 100)
        let mut props_ok = UnorderedMap::new();
        props_ok.insert("mass".into(), json_value!(100));
        let el_ok = ArcadiaElement {
            handle: "1".try_into()?,
            properties: props_ok,
            ..Default::default()
        };

        let issues_ok = validator.validate_element(&el_ok, &loader).await?;
        assert!(issues_ok.is_empty());

        // 2. Cas Invalide (masse = 1000)
        let mut props_ko = UnorderedMap::new();
        props_ko.insert("mass".into(), json_value!(1000));
        let el_ko = ArcadiaElement {
            handle: "2".try_into()?,
            properties: props_ko,
            ..Default::default()
        };

        let issues_ko = validator.validate_element(&el_ko, &loader).await?;
        assert_eq!(issues_ko.len(), 1);
        // 🎯 FIX : On vérifie que le validateur a bien remonté l'UUID technique et non le handle
        assert_eq!(issues_ko[0].rule_id, "mock-uuid-mass-1234");

        Ok(())
    }

    #[async_test]
    async fn test_rule_target_filtering() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let loader = ModelLoader::from_engine(&sandbox.db, "test", "db")?;

        // Règle ciblant uniquement les "LogicalFunction"
        let rule = Rule {
            _id: Some("mock-uuid-func-5678".into()), // 🎯 FIX : UUID factice
            handle: "FUNC_ONLY".into(),
            target: "LogicalFunction".into(),
            expr: Expr::Val(json_value!(false)), // Échoue toujours
            description: Some("Erreur fonction".into()),
            severity: None,
        };

        let validator = DynamicValidator::new(vec![rule]);

        // Element qui matche le type
        let el_match = ArcadiaElement {
            kind: vec!["la:LogicalFunction".into()],
            ..Default::default()
        };
        assert_eq!(
            validator.validate_element(&el_match, &loader).await?.len(),
            1
        );

        // Element qui ne matche pas
        let el_no_match = ArcadiaElement {
            kind: vec!["sa:SystemComponent".into()],
            ..Default::default()
        };
        assert_eq!(
            validator
                .validate_element(&el_no_match, &loader)
                .await?
                .len(),
            0
        );
        Ok(())
    }
}
