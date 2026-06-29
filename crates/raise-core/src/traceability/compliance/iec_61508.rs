// FICHIER : src-tauri/src/traceability/compliance/iec_61508.rs

use super::{ComplianceChecker, ComplianceReport, Violation};
use crate::traceability::tracer::Tracer;
use crate::utils::prelude::*;

pub struct Iec61508Checker;

impl ComplianceChecker for Iec61508Checker {
    fn name(&self) -> &str {
        "IEC-61508 (Industrial Safety)"
    }

    /// 🎯 Version robuste : Audit de la certification SIL pour les systèmes industriels
    fn check(
        &self,
        _tracer: &Tracer,
        docs: &UnorderedMap<String, JsonValue>,
    ) -> RaiseResult<ComplianceReport> {
        let mut violations = Vec::new();
        let mut checked_count = 0;

        for (id, doc) in docs {
            // 🎯 Identification sémantique du domaine industriel
            let is_industrial = doc
                .get("domain")
                .and_then(|v| v.as_str())
                .map(|s| s == "Industrial")
                .unwrap_or(false);

            if is_industrial {
                checked_count += 1;

                let name = doc.get("name").and_then(|n| n.as_str()).ok_or_else(|| {
                    build_error!(
                        "ERR_COMPLIANCE_DATA_INCOMPLETE",
                        context = json_value!({ "id": id, "field": "name" })
                    )
                })?;

                // 🎯 RÈGLE : Présence obligatoire du niveau SIL (Safety Integrity Level)
                let has_sil = doc.get("sil").is_some();

                if !has_sil {
                    violations.push(Violation {
                        element_id: Some(id.clone()),
                        rule_id: "IEC61508-SIL-MISSING".to_string(),
                        description: format!(
                            "Le système industriel '{}' ne possède pas de certification SIL",
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
    async fn test_iec61508_sil_validation() -> RaiseResult<()> {
        let _sandbox = init_test_env().await?;
        let mut docs: UnorderedMap<String, JsonValue> = UnorderedMap::new();

        // 1. Système conforme (Domaine Industriel + SIL défini)
        docs.insert(
            "Turbine_01".to_string(),
            json_value!({
                "_id": "Turbine_01",
                "domain": "Industrial",
                "name": "Gas Turbine Control",
                "sil": 3
            }),
        );

        // 2. Système non conforme (Domaine Industriel mais SIL manquant)
        docs.insert(
            "Conveyor_02".to_string(),
            json_value!({
                "_id": "Conveyor_02",
                "domain": "Industrial",
                "name": "Main Conveyor Belt"
            }),
        );

        // 3. Élément ignoré (Domaine différent)
        docs.insert(
            "Office_PC".to_string(),
            json_value!({
                "_id": "Office_PC",
                "domain": "Corporate"
            }),
        );

        // 🎯 Injection du graphe via from_json_list (Isolant total pour le test)
        let tracer = Tracer::from_json_list(docs.values().cloned().collect())?;
        let checker = Iec61508Checker;
        let report = checker.check(&tracer, &docs)?;

        assert_eq!(report.rules_checked, 2); // Turbine + Conveyor
        assert_eq!(report.violations.len(), 1); // Conveyor est fautif
        assert_eq!(
            report.violations[0].element_id,
            Some("Conveyor_02".to_string())
        );
        assert!(report.violations[0].description.contains("SIL"));

        Ok(())
    }

    #[async_test]
    async fn test_iec61508_empty_scope() -> RaiseResult<()> {
        let _sandbox = init_test_env().await?;

        let docs = UnorderedMap::new();
        let tracer = Tracer::from_json_list(vec![])?;
        let checker = Iec61508Checker;

        let report = checker.check(&tracer, &docs)?;

        assert!(report.passed);
        assert_eq!(report.rules_checked, 0);

        Ok(())
    }
}
