// FICHIER : src-tauri/src/traceability/reporting/trace_matrix.rs

use crate::traceability::tracer::Tracer;
use crate::utils::prelude::*;

#[derive(Debug, Serializable, Deserializable, PartialEq)]
pub struct TraceabilityMatrix {
    pub rows: Vec<TraceRow>,
}

#[derive(Debug, Serializable, Deserializable, PartialEq)]
pub struct TraceRow {
    pub source_id: String,
    pub source_name: String,
    pub target_ids: Vec<String>,
    pub target_names: Vec<String>,
    pub coverage_status: String, // "Covered", "Uncovered"
}

pub struct MatrixGenerator;

impl MatrixGenerator {
    /// 🎯 GÉNÉRATEUR UNIVERSEL : Produit une matrice de traçabilité entre deux types sémantiques.
    /// Exemple : source_kind="SystemFunction", target_kind="LogicalComponent"
    pub fn generate_coverage(
        tracer: &Tracer,
        docs: &UnorderedMap<String, JsonValue>,
        source_kind: &str,
    ) -> RaiseResult<TraceabilityMatrix> {
        let mut rows = Vec::new();

        if source_kind.is_empty() {
            raise_error!(
                "ERR_REPORT_INVALID_KIND",
                error = "Le type source (kind) ne peut pas être vide pour une matrice."
            );
        }

        for (id, doc) in docs {
            // 1. Filtrage sémantique de la source (SA, LA, etc.)
            let kind = doc.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let type_iri = doc.get("@type").and_then(|v| v.as_str()).unwrap_or("");

            if kind == source_kind || type_iri.contains(source_kind) {
                // 2. Identification des cibles via le Tracer (Downstream)
                let downstream_ids = tracer.get_downstream_ids(id);

                let mut target_names = Vec::new();
                for tid in &downstream_ids {
                    let name = docs
                        .get(tid)
                        .and_then(|d| d.get("name").and_then(|n| n.as_str()))
                        .unwrap_or(tid);
                    target_names.push(name.to_string());
                }

                // 3. Calcul du statut
                let status = if downstream_ids.is_empty() {
                    "Uncovered".to_string()
                } else {
                    "Covered".to_string()
                };

                let source_name = doc.get("name").and_then(|n| n.as_str()).unwrap_or(id);

                rows.push(TraceRow {
                    source_id: id.clone(),
                    source_name: source_name.to_string(),
                    target_ids: downstream_ids,
                    target_names,
                    coverage_status: status,
                });
            }
        }

        Ok(TraceabilityMatrix { rows })
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
    async fn test_matrix_coverage_logic_robustness() -> RaiseResult<()> {
        let _sandbox = init_test_env().await?;

        let mut docs: UnorderedMap<String, JsonValue> = UnorderedMap::new();

        // Setup : Une fonction liée et une orpheline
        docs.insert(
            "F1".to_string(),
            json_value!({
                "handle": "F1", "kind": "Function", "name": "Engine Control", "allocatedTo": "C1"
            }),
        );
        docs.insert(
            "F2".to_string(),
            json_value!({
                "handle": "F2", "kind": "Function", "name": "Radio Control"
            }),
        );
        docs.insert(
            "C1".to_string(),
            json_value!({
                "handle": "C1", "kind": "Component", "name": "ECU"
            }),
        );

        // 🎯 Injection via from_json_list pour l'isolation
        let tracer = Tracer::from_json_list(docs.values().cloned().collect())?;

        let matrix = MatrixGenerator::generate_coverage(&tracer, &docs, "Function")?;

        assert_eq!(matrix.rows.len(), 2);

        // Vérification de la ligne couverte
        let row_f1 = matrix.rows.iter().find(|r| r.source_id == "F1").unwrap();
        assert_eq!(row_f1.coverage_status, "Covered");
        assert_eq!(row_f1.target_names, vec!["ECU".to_string()]);

        // Vérification de la ligne orpheline
        let row_f2 = matrix.rows.iter().find(|r| r.source_id == "F2").unwrap();
        assert_eq!(row_f2.coverage_status, "Uncovered");
        assert!(row_f2.target_ids.is_empty());

        Ok(())
    }

    #[async_test]
    async fn test_matrix_serialization_integrity() -> RaiseResult<()> {
        let _sandbox = init_test_env().await?;

        let matrix = TraceabilityMatrix {
            rows: vec![TraceRow {
                source_id: "S".into(),
                source_name: "Source".into(),
                target_ids: vec!["T".into()],
                target_names: vec!["Target".into()],
                coverage_status: "Covered".into(),
            }],
        };
        let serialized = json::serialize_to_string(&matrix).unwrap();
        let deserialized: TraceabilityMatrix = json::deserialize_from_str(&serialized).unwrap();
        assert_eq!(matrix, deserialized);
        Ok(())
    }
}
