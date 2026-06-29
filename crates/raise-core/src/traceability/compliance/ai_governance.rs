// FICHIER : src-tauri/src/traceability/compliance/ai_governance.rs

use super::{ComplianceChecker, ComplianceReport, Violation};
use crate::traceability::tracer::Tracer;
use crate::utils::prelude::*; // 🎯 FIX : Import explicite de UnorderedMap

pub struct AiGovernanceChecker;

impl ComplianceChecker for AiGovernanceChecker {
    fn name(&self) -> &str {
        "RAISE AI Governance"
    }

    fn check(
        &self,
        tracer: &Tracer,
        docs: &UnorderedMap<String, JsonValue>,
    ) -> RaiseResult<ComplianceReport> {
        let mut violations = Vec::new();
        let mut checked_count = 0;

        for (id, doc) in docs {
            // Détection du modèle IA
            let is_ai = doc.get("nature").and_then(|v| v.as_str()) == Some("AI_Model");

            if is_ai {
                checked_count += 1;
                let name = doc.get("name").and_then(|n| n.as_str()).unwrap_or(id);

                // 🎯 RECHERCHE DE PREUVES (Reverse Links) via Tracer
                let evidence_ids = tracer.get_upstream_ids(id);

                // FIX : Type annotation explicite pour aider l'inférence de type
                let has_quality = evidence_ids.iter().any(|eid: &String| {
                    docs.get(eid)
                        .map(|d: &JsonValue| {
                            d.get("kind").and_then(|k| k.as_str()) == Some("QualityReport")
                        })
                        .unwrap_or(false)
                });

                let has_xai = evidence_ids.iter().any(|eid: &String| {
                    docs.get(eid)
                        .map(|d: &JsonValue| {
                            d.get("kind").and_then(|k| k.as_str()) == Some("XaiFrame")
                        })
                        .unwrap_or(false)
                });

                // 🎯 VALIDATION DES RÈGLES
                if !has_quality {
                    violations.push(Violation {
                        element_id: Some(id.clone()),
                        rule_id: "AI-GOV-QR".to_string(),
                        description: format!(
                            "Le modèle IA '{}' manque d'un QualityReport validé",
                            name
                        ),
                        severity: "Critical".to_string(), // 🎯 FIX : .to_string()
                    });
                }

                if !has_xai {
                    violations.push(Violation {
                        element_id: Some(id.clone()),
                        rule_id: "AI-GOV-XAI".to_string(),
                        description: format!(
                            "Le modèle IA '{}' manque d'une trame d'explicabilité (XAI)",
                            name
                        ),
                        severity: "High".to_string(), // 🎯 FIX : .to_string()
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
    async fn test_audit_ai_model_full_compliance() -> RaiseResult<()> {
        let _sandbox = init_test_env().await?;
        let mut docs: UnorderedMap<String, JsonValue> = UnorderedMap::new(); // 🎯 FIX : Type explicite

        // Setup : Modèle IA + Ses deux preuves
        docs.insert(
            "AI_1".to_string(),
            json_value!({ "handle": "AI_1", "nature": "AI_Model", "name": "Boreas" }),
        );
        docs.insert(
            "QR_1".to_string(),
            json_value!({ "handle": "QR_1", "kind": "QualityReport", "model_id": "AI_1" }),
        );
        docs.insert(
            "XAI_1".to_string(),
            json_value!({ "handle": "XAI_1", "kind": "XaiFrame", "model_id": "AI_1" }),
        );

        let tracer = Tracer::from_json_list(docs.values().cloned().collect())?;
        let checker = AiGovernanceChecker;

        let report = checker.check(&tracer, &docs)?;

        assert!(report.passed);
        assert_eq!(report.rules_checked, 1);
        assert!(report.violations.is_empty());

        Ok(())
    }

    #[async_test]
    async fn test_audit_ai_model_missing_everything() -> RaiseResult<()> {
        let _sandbox = init_test_env().await?;
        let mut docs: UnorderedMap<String, JsonValue> = UnorderedMap::new();
        // Setup : Modèle IA tout seul
        docs.insert(
            "AI_EMPTY".to_string(),
            json_value!({ "handle": "AI_EMPTY", "nature": "AI_Model" }),
        );

        let tracer = Tracer::from_json_list(docs.values().cloned().collect())?;
        let checker = AiGovernanceChecker;

        let report = checker.check(&tracer, &docs)?;

        assert!(!report.passed);
        assert_eq!(report.violations.len(), 2); // Doit manquer QR et XAI

        let has_qr_violation = report.violations.iter().any(|v| v.rule_id == "AI-GOV-QR");
        let has_xai_violation = report.violations.iter().any(|v| v.rule_id == "AI-GOV-XAI");

        assert!(has_qr_violation);
        assert!(has_xai_violation);

        Ok(())
    }
}
