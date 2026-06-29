// FICHIER : src-tauri/src/traceability/compliance/do_178c.rs

use super::{ComplianceChecker, ComplianceReport, Violation};
use crate::traceability::tracer::Tracer;
use crate::utils::prelude::*;

pub struct Do178cChecker;

impl ComplianceChecker for Do178cChecker {
    fn name(&self) -> &str {
        "DO-178C (Software Traceability)"
    }

    /// 🎯 Version robuste : Audit de la traçabilité SA -> LA
    fn check(
        &self,
        tracer: &Tracer,
        docs: &UnorderedMap<String, JsonValue>,
    ) -> RaiseResult<ComplianceReport> {
        let mut violations = Vec::new();
        let mut checked_count = 0;

        for (id, doc) in docs {
            // 🎯 Identification sémantique : Est-ce une fonction système (SA) ?
            let is_system_function = doc
                .get("@type")
                .and_then(|t| t.as_str())
                .map(|t| t.contains("SystemFunction"))
                .unwrap_or(false)
                || doc.get("kind").and_then(|k| k.as_str()) == Some("Function");

            if is_system_function {
                checked_count += 1;

                // 🎯 Vérification de la traçabilité aval (Downstream) en O(1)
                let downstream_ids = tracer.get_downstream_ids(id);

                if downstream_ids.is_empty() {
                    let name = doc.get("name").and_then(|n| n.as_str()).ok_or_else(|| {
                        build_error!(
                            "ERR_COMPLIANCE_DATA_INCOMPLETE",
                            context = json_value!({ "id": id, "field": "name" })
                        )
                    })?;
                    violations.push(Violation {
                        element_id: Some(id.clone()),
                        rule_id: "DO178-TRACE-01".to_string(),
                        description: format!(
                            "La fonction '{}' n'est pas allouée à un composant logiciel (LA)",
                            name
                        ),
                        severity: "High".to_string(),
                    });
                }
            }
        }

        Ok(ComplianceReport {
            standard: self.name().to_string(),
            passed: violations.is_empty(),
            rules_checked: checked_count,
            violations,
        })
    }
}

// =========================================================================
// TESTS UNITAIRES HYPER ROBUSTES
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::json_db::jsonld::VocabularyRegistry;
    use crate::utils::testing::mock::DbSandbox;

    async fn init_test_env() -> RaiseResult<DbSandbox> {
        let sandbox = DbSandbox::new().await?;
        let mgr = CollectionsManager::new(
            &sandbox.storage,
            &sandbox.config.mount_points.system.domain,
            &sandbox.config.mount_points.system.db,
        );
        VocabularyRegistry::init_from_db(&mgr).await?;
        Ok(sandbox)
    }

    #[async_test]
    async fn test_do178_traceability_logic() -> RaiseResult<()> {
        let _sandbox = init_test_env().await?;
        let mut docs: UnorderedMap<String, JsonValue> = UnorderedMap::new();

        // 🎯 FIX : Utilisation de "handle" pour correspondre au Tracer actuel
        docs.insert(
            "F1".to_string(),
            json_value!({
                "handle": "F1",
                "kind": "Function",
                "name": "Engine Controller",
                "properties": { "allocatedTo": "C1" } // 🎯 Le Tracer cherche dans "properties"
            }),
        );

        docs.insert(
            "F2".to_string(),
            json_value!({
                "handle": "F2",
                "kind": "Function",
                "name": "Radio Controller"
            }),
        );

        docs.insert(
            "C1".to_string(),
            json_value!({
                "handle": "C1",
                "kind": "Component",
                "name": "ECU"
            }),
        );

        let tracer = Tracer::from_json_list(docs.values().cloned().collect())?;
        let checker = Do178cChecker;

        let report = checker.check(&tracer, &docs)?;

        assert_eq!(report.rules_checked, 2);
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].element_id, Some("F2".to_string()));

        Ok(())
    }

    #[async_test]
    async fn test_do178_empty_model() -> RaiseResult<()> {
        let _sandbox = init_test_env().await?;
        let docs = UnorderedMap::new();
        let tracer = Tracer::from_json_list(vec![])?;
        let checker = Do178cChecker;

        let report = checker.check(&tracer, &docs)?;

        assert!(report.passed);
        assert_eq!(report.rules_checked, 0);

        Ok(())
    }
}
