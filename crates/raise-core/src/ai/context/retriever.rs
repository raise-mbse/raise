// FICHIER : src-tauri/src/ai/context/retriever.rs

use crate::ai::nlp::{preprocessing, tokenizers};
use crate::model_engine::types::{ArcadiaElement, ProjectModel};

pub struct SimpleRetriever {
    model: ProjectModel,
}

impl SimpleRetriever {
    pub fn new(model: ProjectModel) -> Self {
        Self { model }
    }

    /// Récupère un élément "racine" pour servir de contexte initial.
    /// 🎯 PURE GRAPH : On utilise l'itérateur universel pour trouver le premier élément disponible.
    pub fn get_root_element(&self) -> Option<ArcadiaElement> {
        self.model.all_elements().first().cloned().cloned()
    }

    /// Cherche les éléments pertinents avec tolérance aux accents/casse
    pub fn retrieve_context(&self, query: &str) -> String {
        // 1. NORMALISATION DE LA REQUÊTE (via NLP)
        let normalized_query = preprocessing::normalize(query);
        let keywords = tokenizers::tokenize(&normalized_query);

        let mut found_elements = Vec::new();

        // 🎯 PURE GRAPH : Un seul scan universel au lieu de multiples appels à scan_layer
        for el in self.model.all_elements() {
            let raw_name = el.name.as_str();
            let name_norm = preprocessing::normalize(raw_name);

            // Récupération de la description dans les propriétés dynamiques
            let raw_desc = el
                .properties
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let desc_norm = preprocessing::normalize(raw_desc);

            // MATCHING ROBUSTE
            let matches = keywords
                .iter()
                .any(|k| k.len() > 3 && (name_norm.contains(k) || desc_norm.contains(k)));

            let ask_all = keywords.iter().any(|k| k == "liste" || k == "tous");

            if matches || ask_all {
                found_elements.push((
                    el.kind.clone(), // On utilise le type réel (URI)
                    raw_name.to_string(),
                    raw_desc.to_string(),
                ));
            }
        }

        if found_elements.is_empty() {
            return "Aucun élément spécifique du modèle n'a été trouvé.".to_string();
        }

        let mut context_str = String::from("### CONTEXTE DU PROJET (Données réelles) ###\n");
        for (kind, name, description) in found_elements {
            context_str.push_str(&format!(
                "- [{}] {} : {}\n",
                kind.join(", "),
                name,
                description
            ));
        }

        tokenizers::truncate_tokens(&context_str, 2000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::prelude::*;

    // Helper pour créer un élément factice compatible Pure Graph
    fn mock_el(name: &str) -> RaiseResult<ArcadiaElement> {
        let mut properties = UnorderedMap::new();
        properties.insert("description".to_string(), json_value!("desc"));

        Ok(ArcadiaElement {
            handle: "uuid".try_into()?,
            name: I18nString::Single(name.to_string()),
            kind: vec!["test_type".to_string()],
            properties,
            ..Default::default()
        })
    }

    #[test]
    fn test_retrieval_normalization() -> RaiseResult<()> {
        let mut model = ProjectModel::default();
        model.add_element("sa", "components", mock_el("Système Électrique")?);

        let retriever = SimpleRetriever::new(model);
        let result = retriever.retrieve_context("Je cherche le systeme electrique");

        assert!(result.contains("Système Électrique"));
        Ok(())
    }

    #[test]
    fn test_empty_search() {
        let model = ProjectModel::default();
        let retriever = SimpleRetriever::new(model);
        let result = retriever.retrieve_context("Rien");
        assert!(result.contains("Aucun élément spécifique"));
    }

    #[test]
    fn test_get_root_element() -> RaiseResult<()> {
        let mut model = ProjectModel::default();
        let retriever_empty = SimpleRetriever::new(model.clone());
        assert!(retriever_empty.get_root_element().is_none());

        model.add_element("sa", "components", mock_el("Composant Racine")?);
        let retriever_full = SimpleRetriever::new(model);

        let root = retriever_full.get_root_element();
        assert!(root.is_some());
        assert_eq!(root.unwrap().name.as_str(), "Composant Racine");
        Ok(())
    }

    #[test]
    fn test_retrieval_dynamic_elements() -> RaiseResult<()> {
        let mut model = ProjectModel::default();

        // Ajout d'une exigence via l'API dynamique
        let mut req = mock_el("Perf Constraint 10ms")?;
        req.properties.insert(
            "description".to_string(),
            json_value!("Le système doit répondre en moins de 10ms"),
        );
        model.add_element("transverse", "requirements", req);

        let retriever = SimpleRetriever::new(model);

        // Recherche sur mot clé "10ms"
        let res_req = retriever.retrieve_context("exigence 10ms");
        assert!(
            res_req.contains("Perf Constraint"),
            "Nom d'exigence manquant"
        );
        Ok(())
    }
}
