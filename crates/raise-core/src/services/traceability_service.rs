// FICHIER : src-tauri/src/services/traceability_service.rs

use crate::model_engine::types::ProjectModel;
use crate::utils::prelude::*;

use crate::traceability::{
    impact_analyzer::{ImpactAnalyzer, ImpactReport},
    reporting::{
        audit_report::{AuditGenerator, AuditReport},
        trace_matrix::{MatrixGenerator, TraceabilityMatrix},
    },
    tracer::Tracer,
};

/// Helper interne : Convertit le modèle Arcadia en index de documents JSON
/// 🎯 PURE GRAPH : On utilise l'itérateur universel pour collecter tous les éléments
fn get_model_docs(model: &ProjectModel) -> UnorderedMap<String, JsonValue> {
    let mut docs = UnorderedMap::new();

    for e in model.all_elements() {
        if let Ok(val) = json::serialize_to_value(e) {
            docs.insert(e.handle.as_str().to_string(), val);
        }
    }

    docs
}

pub async fn analyze_impact(
    model: &ProjectModel,
    element_id: &str,
    depth: usize,
) -> RaiseResult<ImpactReport> {
    // Utilisation du constructeur de rétro-compatibilité (qui utilise all_elements désormais)
    let tracer = Tracer::from_legacy_model(model)?;

    let analyzer = ImpactAnalyzer::new(tracer);
    let report = analyzer.analyze(element_id, depth)?;

    Ok(report)
}

pub async fn run_compliance_audit(model: &ProjectModel) -> RaiseResult<AuditReport> {
    // Préparation des données pour le générateur universel
    let docs = get_model_docs(model);
    let tracer = Tracer::from_json_list(docs.values().cloned().collect())?;

    // AuditGenerator prend désormais 3 arguments
    let report = AuditGenerator::generate(&tracer, &docs, &model.meta.name)?;

    Ok(report)
}

pub async fn get_traceability_matrix(model: &ProjectModel) -> RaiseResult<TraceabilityMatrix> {
    let docs = get_model_docs(model);
    let tracer = Tracer::from_json_list(docs.values().cloned().collect())?;

    // Utilisation du générateur de couverture universel
    let matrix = MatrixGenerator::generate_coverage(&tracer, &docs, "Function")?;

    Ok(matrix)
}

pub async fn get_element_neighbors(
    model: &ProjectModel,
    element_id: &str,
) -> RaiseResult<JsonValue> {
    let docs = get_model_docs(model);

    // Utilisation du nouveau Tracer
    let tracer = Tracer::from_legacy_model(model)?;

    // Récupération des IDs
    let upstream_ids = tracer.get_upstream_ids(element_id);
    let downstream_ids = tracer.get_downstream_ids(element_id);

    // Résolution des objets complets via l'index
    let mut upstream = Vec::new();
    for id in upstream_ids {
        if let Some(val) = docs.get(&id) {
            upstream.push(val.clone());
        }
    }

    let mut downstream = Vec::new();
    for id in downstream_ids {
        if let Some(val) = docs.get(&id) {
            downstream.push(val.clone());
        }
    }

    Ok(json_value!({
        "center_id": element_id,
        "upstream": upstream,
        "downstream": downstream
    }))
}
