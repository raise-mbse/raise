// FICHIER : crates/raise-core/src/model_engine/capella/model_reader.rs

use super::xmi_parser::CapellaXmiParser;
use crate::model_engine::types::{ProjectMeta, ProjectModel};
use crate::utils::prelude::*;

/// Service de lecture pour les fichiers au format Capella (.capella)
pub struct CapellaReader;

impl CapellaReader {
    /// Lit un fichier .capella et retourne un ProjectModel complet en architecture Pure Graph
    pub fn read_model(path: &Path) -> RaiseResult<ProjectModel> {
        let mut model = ProjectModel::default();

        // 1. Parsing du XMI (Structure Sémantique) via le parser dédié
        // Le parser remplit dynamiquement les 'layers' du modèle.
        CapellaXmiParser::parse_file(path, &mut model)?;

        // 2. Extraction du nom de fichier pour les métadonnées
        let filename = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("unknown.capella");

        // 3. Finalisation des métadonnées du projet
        // 🎯 FIX : Retrait de 'loaded_at' qui n'est plus dans ProjectMeta (types.rs)
        model.meta = ProjectMeta {
            name: filename.to_string(),
            element_count: Self::count_elements(&model),
        };

        Ok(model)
    }

    /// Compte le nombre total d'éléments dans le modèle de manière dynamique
    fn count_elements(model: &ProjectModel) -> usize {
        // 🎯 PURE GRAPH : On utilise l'itérateur universel all_elements()
        // pour compter tous les éléments sans connaître les couches à l'avance.
        model.all_elements().len()
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_engine::types::ArcadiaElement;

    /// Helper pour créer un élément factice compatible Pure Graph
    fn create_dummy(id: &str, kind: &str) -> RaiseResult<ArcadiaElement> {
        Ok(ArcadiaElement {
            handle: id.try_into()?,
            name: I18nString::default(),
            kind: vec![kind.into()],
            // 🎯 FIX : Pas de champ description statique, tout est dans properties
            properties: UnorderedMap::new(),
            ..Default::default()
        })
    }

    #[test]
    fn test_element_counting_logic() -> RaiseResult<()> {
        let mut model = ProjectModel::default();

        // Ajout d'éléments dans diverses couches via l'API dynamique
        model.add_element("sa", "functions", create_dummy("F1", "SystemFunction")?);
        model.add_element("la", "components", create_dummy("C1", "LogicalComponent")?);

        // Ajout dans la couche Transverse
        model.add_element(
            "transverse",
            "requirements",
            create_dummy("R1", "Requirement")?,
        );

        // Vérification du comptage universel
        let count = CapellaReader::count_elements(&model);
        assert_eq!(
            count, 3,
            "Le compteur doit inclure toutes les couches dynamiques"
        );
        Ok(())
    }

    #[test]
    fn test_empty_model_metadata() -> RaiseResult<()> {
        let model = ProjectModel::default();
        assert_eq!(CapellaReader::count_elements(&model), 0);
        Ok(())
    }
}
