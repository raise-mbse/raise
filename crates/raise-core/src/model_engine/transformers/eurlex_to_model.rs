// FICHIER : src-tauri/src/model_engine/transformers/eurlex_to_model.rs

use crate::model_engine::eurlex::parser::EurlexParsedData;
use crate::model_engine::types::{ArcadiaElement, NameType, ProjectModel};
use crate::utils::data::UnorderedMap;
use crate::utils::prelude::*;

pub struct EurlexToModelTransformer;

impl EurlexToModelTransformer {
    /// Convertit les données brutes extraites d'EUR-Lex en un modèle de projet Arcadia.
    /// Les lois deviennent des Exigences (Requirements) et des Contraintes (Constraints) transverses.
    pub fn transform_to_model(parsed_data: &EurlexParsedData) -> RaiseResult<ProjectModel> {
        let mut model = ProjectModel::default();

        // ====================================================================
        // 1. TRAÇABILITÉ LÉGALE : L'Exigence (Requirement)
        // ====================================================================
        let mut req_props = UnorderedMap::new();
        req_props.insert(
            "description".to_string(),
            json_value!(&parsed_data.raw_text),
        );
        req_props.insert(
            "source".to_string(),
            json_value!("Directive 2026/288/RENURE"),
        );
        req_props.insert("priority".to_string(), json_value!("Mandatory"));

        let requirement = ArcadiaElement {
            id: "REQ_DIR_2026_288_RENURE".to_string(),
            name: NameType::String("Dérogation Fertilisants RENURE".to_string()),
            // Utilisation stricte de l'URI sémantique de la couche Transverse
            kind: "https://raise.io/transverse#Requirement".to_string(),
            properties: req_props,
        };

        // Ajout dans la collection dynamique du ProjectModel
        model.add_element("transverse", "requirements", requirement);

        // ====================================================================
        // 2. RÈGLE DÉTERMINISTE : La Contrainte Physique (Constraint)
        // ====================================================================
        let mut constraint_props = UnorderedMap::new();

        // Calcul métier direct : Base réglementaire (ex: 170kg) + dérogation RENURE (80kg)
        let n_limit = parsed_data.extra_n_limit.unwrap_or(0);
        let total_n_limit = 170 + n_limit;

        constraint_props.insert("max_n_limit".to_string(), json_value!(total_n_limit));

        if let Some(cu) = parsed_data.max_cu {
            constraint_props.insert("max_cu_mg_kg".to_string(), json_value!(cu));
        }
        if let Some(zn) = parsed_data.max_zn {
            constraint_props.insert("max_zn_mg_kg".to_string(), json_value!(zn));
        }

        let constraint = ArcadiaElement {
            id: "CSTR_RENURE_LIMITS".to_string(),
            name: NameType::String("Seuils de Tolérance RENURE".to_string()),
            kind: "https://raise.io/transverse#Constraint".to_string(),
            properties: constraint_props,
        };

        model.add_element("transverse", "constraints", constraint);

        Ok(model)
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_eurlex_data_to_arcadia_model() -> RaiseResult<()> {
        // 1. Mock des données extraites par le parseur
        let parsed_data = EurlexParsedData {
            raw_text: "Les fertilisants RENURE sont autorisés jusqu'à 80kg supplémentaires."
                .to_string(),
            extra_n_limit: Some(80),
            max_cu: Some(300),
            max_zn: Some(800),
        };

        // 2. Exécution du transformateur
        let model = EurlexToModelTransformer::transform_to_model(&parsed_data)?;

        // 3. Validation de l'Exigence (Requirement)
        let requirements = model.get_collection("transverse", "requirements");
        assert_eq!(requirements.len(), 1, "Une exigence doit être créée");
        let req = &requirements[0];
        assert_eq!(req.id, "REQ_DIR_2026_288_RENURE");
        assert!(req
            .properties
            .get("description")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("RENURE"));

        // 4. Validation de la Contrainte (Constraint)
        let constraints = model.get_collection("transverse", "constraints");
        assert_eq!(constraints.len(), 1, "Une contrainte doit être créée");
        let cstr = &constraints[0];
        assert_eq!(cstr.id, "CSTR_RENURE_LIMITS");

        // On vérifie que la logique métier (170 + 80) a bien été appliquée
        assert_eq!(
            cstr.properties
                .get("max_n_limit")
                .unwrap()
                .as_u64()
                .unwrap(),
            250
        );
        assert_eq!(
            cstr.properties
                .get("max_cu_mg_kg")
                .unwrap()
                .as_u64()
                .unwrap(),
            300
        );
        assert_eq!(
            cstr.properties
                .get("max_zn_mg_kg")
                .unwrap()
                .as_u64()
                .unwrap(),
            800
        );

        Ok(())
    }
}
