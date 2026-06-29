// FICHIER : src-tauri/src/model_engine/transformers/mod.rs

use crate::model_engine::types::{ArcadiaElement, ProjectModel};
use crate::utils::prelude::*;
pub mod dialogue_to_model;

pub mod eurlex_to_model;
pub use eurlex_to_model::EurlexToModelTransformer;

/// Configuration pour piloter la transformation sémantique
#[derive(Clone)]
pub struct TransformerConfig {
    pub domain: String,
    pub relation_key: String, // ex: "ownedFunctionalAllocation"
    pub target_layer: String, // ex: "la"
    pub target_col: String,   // ex: "functions"
}

pub trait ModelTransformer: Send + Sync {
    fn transform(&self, element: &JsonValue) -> RaiseResult<JsonValue>;

    fn transform_with_context(
        &self,
        element: &ArcadiaElement,
        model: &ProjectModel,
    ) -> RaiseResult<JsonValue>;
}

pub struct UniversalTransformer {
    config: TransformerConfig,
}

impl UniversalTransformer {
    pub fn new(config: TransformerConfig) -> Self {
        Self { config }
    }
}

impl ModelTransformer for UniversalTransformer {
    fn transform(&self, element: &JsonValue) -> RaiseResult<JsonValue> {
        Ok(json_value!({
            "domain": self.config.domain,
            "entity": element
        }))
    }

    fn transform_with_context(
        &self,
        element: &ArcadiaElement,
        model: &ProjectModel,
    ) -> RaiseResult<JsonValue> {
        let mut related_data = Vec::new();

        // 🎯 NAVIGATION GÉNÉRIQUE
        let targets = model.get_collection(&self.config.target_layer, &self.config.target_col);

        if let Some(links) = element
            .properties
            .get(&self.config.relation_key)
            .and_then(|v| v.as_array())
        {
            for link in links {
                if let Some(id) = link.get("id").and_then(|v| v.as_str()) {
                    if let Some(found) = targets.iter().find(|t| t.handle.as_str() == id) {
                        related_data.push(json_value!({
                            "name": found.name.as_str(),
                            "kind": found.kind,
                            "id":found.handle.as_str().to_string()
                        }));
                    }
                }
            }
        }

        Ok(json_value!({
            "domain": self.config.domain,
            "entity": {
                "name": element.name.as_str(),
                "kind": element.kind,
                "relations": related_data
            },
            "generated_at": UtcClock::now().to_rfc3339()
        }))
    }
}

pub enum TransformationDomain {
    Software,
    Hardware,
    System,
}

/// 🎯 FACTORY : Centralise la création des transformers avec leurs configs
pub fn get_transformer(domain: TransformationDomain) -> Box<dyn ModelTransformer> {
    let config = match domain {
        TransformationDomain::Software => TransformerConfig {
            domain: "software".into(),
            relation_key: "ownedFunctionalAllocation".into(),
            target_layer: "la".into(),
            target_col: "functions".into(),
        },
        TransformationDomain::Hardware => TransformerConfig {
            domain: "hardware".into(),
            relation_key: "ownedPhysicalPorts".into(),
            target_layer: "pa".into(),
            target_col: "links".into(),
        },
        TransformationDomain::System => TransformerConfig {
            domain: "system".into(),
            relation_key: "parts".into(),
            target_layer: "oa".into(),
            target_col: "entities".into(),
        },
    };
    Box::new(UniversalTransformer::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_universal_transformer_logic() -> RaiseResult<()> {
        let mut model = ProjectModel::default();
        let config = TransformerConfig {
            domain: "test".into(),
            relation_key: "links".into(),
            target_layer: "layer1".into(),
            target_col: "col1".into(),
        };
        let transformer = UniversalTransformer::new(config);

        // Ajout d'une cible
        let target = ArcadiaElement {
            handle: "T1".try_into()?,
            name: I18nString::Single("Target".into()),
            ..Default::default()
        };
        model.add_element("layer1", "col1", target);

        // Élément source avec lien
        let mut source = ArcadiaElement {
            handle: "S1".try_into()?,
            ..Default::default()
        };
        source
            .properties
            .insert("links".into(), json_value!([{"id": "T1"}]));

        let result = transformer.transform_with_context(&source, &model).unwrap();
        assert_eq!(result["domain"], "test");
        assert_eq!(result["entity"]["relations"][0]["name"], "Target");
        Ok(())
    }
}
