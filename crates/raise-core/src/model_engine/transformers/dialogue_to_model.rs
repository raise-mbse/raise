// FICHIER : src-tauri/src/model_engine/transformers/dialogue_to_model.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::model_engine::arcadia::ArcadiaOntology;
use crate::model_engine::types::ArcadiaElement;
use crate::utils::prelude::*;

pub struct DialogueToModelTransformer;

impl DialogueToModelTransformer {
    /// Transforme une intention extraite par le LLM en un ArcadiaElement normalisé.
    pub async fn create_element_from_intent(
        manager: &CollectionsManager<'_>,
        intent: &JsonValue,
    ) -> RaiseResult<ArcadiaElement> {
        // 1. Extraction des champs de base de l'intention
        let name_str = intent
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unnamed");
        let type_str = intent
            .get("type")
            .or_else(|| intent.get("element_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");
        let layer_str = intent
            .get("layer")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        // 2. Résolution du type (URI) via le mapping ontologique dynamique
        let mapping_doc = manager
            .get_document("configs", "ref:configs:handle:ontological_mapping")
            .await?
            .unwrap_or_else(|| json_value!({}));

        // On génère un slug propre à partir du nom de l'intention
        let handle_str = intent
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unnamed")
            .to_lowercase()
            .replace(" ", "_");

        let mut type_uri = None;

        // Tentative de résolution via les dialogue_mappings définis en base
        if let Some(dialogue_map) = mapping_doc
            .get("dialogue_mappings")
            .and_then(|v| v.as_object())
        {
            if let Some(layer_map) = dialogue_map.get(layer_str).and_then(|v| v.as_object()) {
                if let Some(prefix) = layer_map.get("prefix").and_then(|v| v.as_str()) {
                    if let Some(mapped_class) = layer_map.get(type_str).and_then(|v| v.as_str()) {
                        type_uri = ArcadiaOntology::get_uri(prefix, mapped_class);
                    }
                }
            }
        }

        // Construction de l'URI finale (avec fallback automatique par couche)
        let final_uri = type_uri.unwrap_or_else(|| {
            let prefix = match layer_str.to_uppercase().as_str() {
                "OA" => "oa",
                "SA" => "sa",
                "LA" => "la",
                "PA" => "pa",
                "TRANSVERSE" => "transverse",
                _ => "sa",
            };
            // Si non trouvé dans le registre, on construit l'URI standard RAISE
            ArcadiaOntology::get_uri(prefix, type_str)
                .unwrap_or_else(|| format!("https://raise.io/{}#{}", prefix, type_str))
        });

        // 3. Gestion dynamique des propriétés (Architecture Pure Graph)
        // 🎯 On injecte l'intégralité de l'intention dans la map properties
        let mut properties = UnorderedMap::new();
        if let Some(obj) = intent.as_object() {
            for (k, v) in obj {
                // On évite de dupliquer les métadonnées de structure déjà présentes
                if !matches!(k.as_str(), "name" | "type" | "element_type" | "layer") {
                    properties.insert(k.clone(), v.clone());
                }
            }
        }

        Ok(ArcadiaElement {
            handle: handle_str.as_str().try_into()?,
            name: I18nString::Single(name_str.to_string()),
            kind: vec![final_uri],
            properties,
            ..Default::default()
        })
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::testing::AgentDbSandbox;

    #[async_test]
    async fn test_create_logical_component_from_intent() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.db, "testing", "db");

        let intent = json_value!({
            "name": "Navigation System",
            "type": "LogicalComponent",
            "layer": "LA",
            "description": "Gère le calcul de trajectoire",
            "priority": "Critical"
        });

        let result =
            DialogueToModelTransformer::create_element_from_intent(&manager, &intent).await;
        assert!(result.is_ok());
        let el = result.unwrap();

        assert_eq!(el.name.as_str(), "Navigation System");

        assert_eq!(
            el.kind,
            vec!["https://raise.io/la#LogicalComponent"] // <-- On retire "/ontology/arcadia"
        );
        // Vérification de l'aplatissement des propriétés dynamiques (Pure Graph)
        assert_eq!(
            el.properties.get("description").and_then(|v| v.as_str()),
            Some("Gère le calcul de trajectoire")
        );
        assert_eq!(
            el.properties.get("priority").and_then(|v| v.as_str()),
            Some("Critical")
        );

        Ok(())
    }

    #[async_test]
    async fn test_create_with_automatic_layer_fallback() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.db, "testing", "db");

        let intent = json_value!({
            "name": "Sensor",
            "type": "PhysicalActor"
            // Pas de couche spécifiée
        });

        let el = DialogueToModelTransformer::create_element_from_intent(&manager, &intent)
            .await
            .unwrap();

        // Le fallback par défaut est la couche SA (System Analysis)
        assert!(el.kind.iter().any(|k| k.contains("/sa#PhysicalActor")));

        Ok(())
    }
}
