// FICHIER : src-tauri/src/traceability/reporting/audit_report.rs

use crate::traceability::compliance::{
    AiGovernanceChecker, ComplianceChecker, Do178cChecker, EuAiActChecker, Iec61508Checker,
    Iso26262Checker,
};
use crate::traceability::tracer::Tracer;
use crate::utils::prelude::*;

#[derive(Debug, Serializable, Deserializable, PartialEq, Clone)]
pub struct AuditReport {
    pub project_name: String,
    pub date: String,
    pub compliance_results: Vec<JsonValue>,
    pub model_stats: ModelStats,
}

#[derive(Debug, Serializable, Deserializable, PartialEq, Default, Clone)]
pub struct ModelStats {
    pub total_elements: usize,
    pub total_functions: usize,
    pub total_components: usize,
    pub total_requirements: usize,
    pub total_scenarios: usize,
    pub total_functional_chains: usize,
}

pub struct AuditGenerator;

impl AuditGenerator {
    /// 🎯 GÉNÉRATEUR UNIVERSEL
    /// Orchestre les audits et calcule les statistiques sémantiques.
    pub fn generate(
        tracer: &Tracer,
        docs: &UnorderedMap<String, JsonValue>,
        project_name: &str,
    ) -> RaiseResult<AuditReport> {
        // 1. Enregistrement des Checkers (Extensibilité O(1))
        let checkers: Vec<Box<dyn ComplianceChecker>> = vec![
            Box::new(Do178cChecker),
            Box::new(Iso26262Checker),
            Box::new(EuAiActChecker),
            Box::new(Iec61508Checker),
            Box::new(AiGovernanceChecker),
        ];

        // 2. Exécution et sérialisation des résultats
        let mut compliance_results = Vec::new();
        for checker in checkers {
            let report = checker.check(tracer, docs)?;
            compliance_results.push(json::serialize_to_value(report)?);
        }

        // 3. Calcul des statistiques
        let model_stats = Self::calculate_stats(docs)?;

        Ok(AuditReport {
            project_name: project_name.to_string(),
            date: UtcClock::now().to_rfc3339(),
            compliance_results,
            model_stats,
        })
    }

    /// Analyse sémantique des types pour le comptage
    fn calculate_stats(docs: &UnorderedMap<String, JsonValue>) -> RaiseResult<ModelStats> {
        // 🎯 FIX CLIPPY : Initialisation atomique
        let mut stats = ModelStats {
            total_elements: docs.len(),
            ..ModelStats::default()
        };

        for (id, doc) in docs {
            let kind = doc
                .get("kind")
                .map(|v| {
                    v.as_str().ok_or_else(|| {
                        build_error!(
                            "ERR_DATATYPE_MISMATCH",
                            context = json_value!({"id": id, "field": "kind"})
                        )
                    })
                })
                .transpose()?
                .unwrap_or("");

            let type_iri = doc
                .get("@type")
                .map(|v| {
                    v.as_str().ok_or_else(|| {
                        build_error!(
                            "ERR_DATATYPE_MISMATCH",
                            context = json_value!({"id": id, "field": "@type"})
                        )
                    })
                })
                .transpose()?
                .unwrap_or("");

            // Matching robuste sur le Kind ou l'IRI JSON-LD
            if kind == "Function" || type_iri.contains("Function") {
                stats.total_functions += 1;
            } else if kind == "Component" || type_iri.contains("Component") {
                stats.total_components += 1;
            } else if kind == "Requirement" || type_iri.contains("Requirement") {
                stats.total_requirements += 1;
            } else if kind == "Scenario" || type_iri.contains("Scenario") {
                stats.total_scenarios += 1;
            } else if kind == "FunctionalChain" || type_iri.contains("FunctionalChain") {
                stats.total_functional_chains += 1;
            }
        }
        Ok(stats)
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

    async fn init_registry() -> RaiseResult<DbSandbox> {
        // 1. Création de la sandbox isolée
        let sandbox = DbSandbox::new().await?;

        // 2. Création du manager pour la sandbox
        let mgr = CollectionsManager::new(
            &sandbox.storage,
            &sandbox.config.mount_points.system.domain,
            &sandbox.config.mount_points.system.db,
        );

        // 3. Initialisation du registre
        VocabularyRegistry::init_from_db(&mgr).await?;

        Ok(sandbox)
    }

    /// 🎯 TEST 1 : Vérification de l'intégralité du rapport
    #[async_test]
    async fn test_audit_generate_full_report() -> RaiseResult<()> {
        init_registry().await?;

        let mut docs = UnorderedMap::new();
        docs.insert(
            "F1".into(),
            json_value!({ "_id": "F1", "kind": "Function", "name": "Main Controller" }),
        );

        let tracer = Tracer::from_json_list(vec![])?;
        let report = AuditGenerator::generate(&tracer, &docs, "Test Project")?;

        assert_eq!(report.project_name, "Test Project");
        // On attend 5 résultats (un par checker enregistré)
        assert_eq!(report.compliance_results.len(), 5);
        assert_eq!(report.model_stats.total_functions, 1);

        Ok(())
    }

    /// 🎯 TEST 2 : Robustesse du comptage sémantique (Stats)
    #[async_test]
    async fn test_calculate_stats_semantic_mapping() -> RaiseResult<()> {
        init_registry().await?;
        let mut docs = UnorderedMap::new();
        docs.insert("1".into(), json_value!({ "kind": "Function" }));
        docs.insert(
            "2".into(),
            json_value!({ "@type": "raise:SystemComponent" }),
        );
        docs.insert("3".into(), json_value!({ "kind": "Requirement" }));
        docs.insert("4".into(), json_value!({ "kind": "Scenario" }));
        docs.insert("5".into(), json_value!({ "kind": "FunctionalChain" }));
        // Élément inconnu (ne doit pas fausser les comptes spécifiques)
        docs.insert("6".into(), json_value!({ "kind": "Unknown" }));

        let stats = AuditGenerator::calculate_stats(&docs)?;

        assert_eq!(stats.total_elements, 6);
        assert_eq!(stats.total_functions, 1);
        assert_eq!(stats.total_components, 1);
        assert_eq!(stats.total_requirements, 1);
        assert_eq!(stats.total_scenarios, 1);
        assert_eq!(stats.total_functional_chains, 1);

        Ok(())
    }

    /// 🎯 TEST 3 : Résilience aux données JSON malformées
    #[async_test]
    async fn test_robustness_malformed_json() -> RaiseResult<()> {
        init_registry().await?;
        let mut docs_empty = UnorderedMap::new();
        // 1. Un document totalement vide ne fait pas planter, il est juste ignoré par les stats.
        docs_empty.insert("empty".into(), json_value!({}));

        let tracer = Tracer::from_json_list(vec![])?;
        let report = AuditGenerator::generate(&tracer, &docs_empty, "Empty Test")?;

        assert_eq!(report.model_stats.total_elements, 1);
        assert_eq!(report.model_stats.total_functions, 0);

        // 2. Un document avec un type illégal (ex: 'null' au lieu de string) DOIT faire échouer l'audit !
        let mut docs_corrupt = UnorderedMap::new();
        docs_corrupt.insert("null_kind".into(), json_value!({ "kind": null }));

        let result = AuditGenerator::generate(&tracer, &docs_corrupt, "Corrupt Test");

        assert!(
            result.is_err(),
            "L'audit aurait dû être bloqué par la corruption du champ 'kind'"
        );
        if let Err(e) = result {
            assert!(e.to_string().contains("ERR_DATATYPE_MISMATCH"));
        }

        Ok(())
    }

    /// 🎯 TEST 4 : Intégrité de la date ISO-8601
    #[async_test]
    async fn test_audit_date_format() -> RaiseResult<()> {
        init_registry().await?;
        let tracer = Tracer::from_json_list(vec![])?;
        let report = AuditGenerator::generate(&tracer, &UnorderedMap::new(), "Date Test")?;

        // Vérifie que la date est au format rfc3339 (contient 'T' et 'Z' ou offset)
        assert!(report.date.contains('T'));

        Ok(())
    }
}
