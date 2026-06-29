// FICHIER : src-tauri/src/ai/context/tests.rs

use crate::ai::context::retriever::SimpleRetriever;
use crate::model_engine::types::{ArcadiaElement, ProjectModel};
use crate::utils::prelude::*;

/// Helper pour créer un élément factice compatible avec l'architecture Pure Graph
fn mock_element(name: &str, desc: &str) -> RaiseResult<ArcadiaElement> {
    // 🎯 FIX : La description est maintenant une propriété dynamique
    let mut props = UnorderedMap::new();
    props.insert("description".to_string(), json_value!(desc));

    Ok(ArcadiaElement {
        handle: format!("uuid-{}", name.replace(' ', "_"))
            .as_str()
            .try_into()?,
        name: I18nString::Single(name.to_string()),
        kind: vec!["mock:Type".to_string()],
        properties: props,
        ..Default::default()
    })
}

#[test]
fn test_retriever_finds_relevant_info() -> RaiseResult<()> {
    // 1. Préparer un modèle "Pure Graph"
    let mut model = ProjectModel::default();

    // 🎯 FIX : Utilisation de add_element(layer, collection, element) au lieu des champs statiques
    model.add_element(
        "oa",
        "actors",
        mock_element("Pilote de Drone", "Responsable du vol et de la sécurité.")?,
    );

    model.add_element(
        "sa",
        "functions",
        mock_element("Alimenter Secteur", "Fournit l'énergie au système.")?,
    );

    // 2. Instancier le retriever
    let retriever = SimpleRetriever::new(model);

    // 3. Test de recherche
    let query = "Qui est responsable du vol ?";
    let context = retriever.retrieve_context(query);

    // 4. Assertions
    assert!(
        context.contains("Pilote de Drone"),
        "Le contexte doit contenir l'acteur trouvé"
    );
    assert!(
        context.contains("Responsable du vol"),
        "Le contexte doit contenir la description extraite des properties"
    );
    Ok(())
}

#[test]
fn test_retriever_empty_search() {
    let model = ProjectModel::default();
    let retriever = SimpleRetriever::new(model);

    let context = retriever.retrieve_context("Rien à voir");
    assert!(
        context.contains("Aucun élément spécifique"),
        "Doit gérer proprement l'absence de résultats"
    );
}
