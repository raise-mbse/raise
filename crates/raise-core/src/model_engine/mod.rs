// FICHIER : crates/raise-core/src/model_engine/mod.rs

// 1. Modules Fondamentaux (Le cœur du moteur)
pub mod ingestion;
pub mod loader;
pub mod types;

// 2. Modules de Logique Métier (Les fonctionnalités)
pub mod arcadia; // Définitions sémantiques (OA, SA, LA, PA)
pub mod capella; // Support des fichiers .capella / .aird
pub mod dsl;
pub mod eurlex;
pub mod sysml2;
pub mod transformers; // Génération de code et conversion
pub mod validators; // Vérification de cohérence // Conformité
                    // 3. Re-exports (Façade publique pour le reste de l'app)

// Loader & Modèle
pub use loader::ModelLoader;
// 🎯 PURE GRAPH : Suppression de TransverseModel
pub use ingestion::ModelIngestionService;
pub use types::{ArcadiaElement, ProjectMeta, ProjectModel};

// Transformers (Software, Hardware, System)
pub use transformers::{
    dialogue_to_model::DialogueToModelTransformer, get_transformer, ModelTransformer,
    TransformationDomain,
};

// Validators (Règles métier)
pub use validators::{
    compliance_validator::ComplianceValidator, consistency_checker::ConsistencyChecker,
    dynamic_validator::DynamicValidator, ModelValidator, Severity, ValidationIssue,
};

// Arcadia Semantics (Couches et Catégories)
pub use arcadia::element_kind::{ArcadiaSemantics, ElementCategory, Layer};

// Capella (Import)
pub use capella::{CapellaReader, CapellaXmiParser};

pub use sysml2::{Sysml2Parser, Sysml2ToArcadiaMapper};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::prelude::*;

    #[test]
    fn test_integration_facade() -> RaiseResult<()> {
        // 1. Vérifie l'accès aux types de base
        let mut model = ProjectModel::default();

        // 🎯 PURE GRAPH : Utilisation des méthodes dynamiques pour le test
        let req = ArcadiaElement {
            handle: "REQ-1".try_into()?,
            name: I18nString::Single("Test".to_string()),
            kind: vec!["Requirement".to_string()],
            ..Default::default()
        };
        model.add_element("transverse", "requirements", req);
        assert_eq!(model.get_collection("transverse", "requirements").len(), 1);

        // 2. Vérifie l'accès à la Factory Transformer
        let transformer = get_transformer(TransformationDomain::Software);
        let dummy = json_value!({ "_id": "TEST", "name": "TestElement" });
        assert!(transformer.transform(&dummy).is_ok());

        // 3. Vérifie l'accès à l'enum Sémantique
        let layer = Layer::SystemAnalysis;
        assert_eq!(layer, Layer::SystemAnalysis);
        Ok(())
    }
}
