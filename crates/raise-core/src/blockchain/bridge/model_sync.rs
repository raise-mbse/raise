// src-tauri/src/blockchain/bridge/model_sync.rs

use crate::blockchain::storage::commit::{MentisCommit, Mutation, MutationOp};
use crate::model_engine::types::{ArcadiaElement, ProjectModel};
use crate::utils::prelude::*;
use crate::AppState;

/// Synchroniseur responsable de la mise à jour du modèle symbolique en mémoire (Pure Graph).
pub struct ModelSync<'a> {
    app_state: &'a AppState,
}

impl<'a> ModelSync<'a> {
    pub fn new(app_state: &'a AppState) -> Self {
        Self { app_state }
    }

    /// Applique les mutations d'un commit au ProjectModel global en mémoire.
    pub async fn sync_commit(&self, commit: &MentisCommit) -> RaiseResult<()> {
        let mut model_guard = self.app_state.model.lock().await;

        for mutation in &commit.mutations {
            self.apply_mutation(&mut model_guard, mutation)?;
        }
        Ok(())
    }

    /// Applique une mutation individuelle sur le graphe en mémoire.
    fn apply_mutation(&self, model: &mut ProjectModel, mutation: &Mutation) -> RaiseResult<()> {
        match mutation.operation {
            MutationOp::Create | MutationOp::Update => {
                // 🎯 RUST-FIRST : Match explicite sur la désérialisation
                match json::deserialize_from_value::<ArcadiaElement>(mutation.payload.clone()) {
                    Ok(element) => {
                        self.upsert_element(model, element)?;
                    }
                    Err(e) => {
                        // 🎯 FIX MACRO : Utilisation directe, sans `return Err()`
                        raise_error!(
                            "ERR_SYNC_PAYLOAD_INVALID",
                            error = format!("Impossible de désérialiser ArcadiaElement : {}", e),
                            context = json_value!({
                                "element_id": mutation.element_id,
                                "action": "deserialize_mutation_payload"
                            })
                        );
                    }
                }
            }
            MutationOp::Delete => {
                self.delete_element(model, &mutation.element_id)?;
            }
        }
        Ok(())
    }

    /// 🎯 PURE GRAPH : Insertion ou mise à jour dynamique.
    fn upsert_element(&self, model: &mut ProjectModel, element: ArcadiaElement) -> RaiseResult<()> {
        // On détermine la destination à partir du type (kind) de l'élément
        let (layer, col) = self.map_kind_to_location(&element.kind);

        // Si l'élément existe déjà quelque part, on le met à jour
        let mut found = false;
        for collections in model.layers.values_mut() {
            for vec in collections.values_mut() {
                if let Some(pos) = vec
                    .iter()
                    .position(|e| e.handle.as_str() == element.handle.as_str())
                {
                    vec[pos] = element.clone();
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }

        // Sinon, on l'ajoute dans sa couche naturelle
        if !found {
            model.add_element(layer, col, element);
        }

        Ok(())
    }

    /// 🎯 PURE GRAPH : Recherche et suppression transversale dans toutes les couches.
    /// FIX IDEMPOTENCE : Si l'élément n'existe pas, l'opération réussit silencieusement.
    fn delete_element(&self, model: &mut ProjectModel, id: &str) -> RaiseResult<()> {
        for collections in model.layers.values_mut() {
            for vec in collections.values_mut() {
                if let Some(pos) = vec.iter().position(|e| e.handle.as_str() == id) {
                    vec.remove(pos);
                    return Ok(());
                }
            }
        }

        // 🎯 FIX : On ne lève plus d'erreur stricte. L'absence de la donnée est le résultat attendu.
        user_trace!(
            "ℹ️ [ModelSync] Suppression ignorée : '{}' déjà absent de la RAM.",
            id
        );
        Ok(())
    }

    /// Helper pour router les nouveaux éléments vers les couches par défaut.
    fn map_kind_to_location(&self, kinds: &[String]) -> (&'static str, &'static str) {
        if kinds.iter().any(|k| k.contains("OperationalActor")) {
            ("oa", "actors")
        } else if kinds.iter().any(|k| k.contains("OperationalActivity")) {
            ("oa", "activities")
        } else if kinds.iter().any(|k| k.contains("SystemComponent")) {
            ("sa", "components")
        } else if kinds.iter().any(|k| k.contains("SystemFunction")) {
            ("sa", "functions")
        } else if kinds.iter().any(|k| k.contains("LogicalComponent")) {
            ("la", "components")
        } else if kinds.iter().any(|k| k.contains("PhysicalComponent")) {
            ("pa", "components")
        } else if kinds.iter().any(|k| k.contains("Requirement")) {
            ("transverse", "requirements")
        } else {
            ("others", "elements")
        }
    }

    pub fn is_ready(&self) -> bool {
        true
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_state() -> AppState {
        AppState {
            model: SharedRef::new(AsyncMutex::new(ProjectModel::default())),
        }
    }

    #[async_test]
    async fn test_upsert_new_element_pure_graph() -> RaiseResult<()> {
        let state = create_test_state();
        let sync = ModelSync::new(&state);

        let default_element = ArcadiaElement::default();
        let mut payload =
            json::serialize_to_value(&default_element).expect("Sérialisation échouée");

        // 🎯 FIX ALIASING JSON-LD : On injecte toutes les variantes possibles du champ type/id
        // pour être certains que Serde les attrape, peu importe les #[serde(rename)] de types.rs
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("id".to_string(), json_value!("urn:sa:comp1"));
            obj.insert("@id".to_string(), json_value!("urn:sa:comp1"));

            // 🎯 FIX V2 : Sérialisation d'un vecteur JSON au lieu d'une String
            obj.insert("kind".to_string(), json_value!(["SystemComponent"]));
            obj.insert("@type".to_string(), json_value!(["SystemComponent"]));
            obj.insert("type".to_string(), json_value!(["SystemComponent"]));

            obj.insert("name".to_string(), json_value!("Radar Unit"));
        }

        let mutation = Mutation {
            element_id: "urn:sa:comp1".into(),
            operation: MutationOp::Create,
            payload,
        };

        sync.apply_mutation(&mut *state.model.lock().await, &mutation)
            .unwrap();

        let model = state.model.lock().await;

        // Vérification stricte
        let components = model.get_collection("sa", "components");
        assert_eq!(
            components.len(),
            1,
            "L'élément n'a pas été routé dans la bonne couche (sa/components)"
        );
        assert_eq!(components[0].name.as_str(), "Radar Unit");
        Ok(())
    }

    #[async_test]
    async fn test_delete_element_pure_graph() -> RaiseResult<()> {
        let state = create_test_state();
        let sync = ModelSync::new(&state);

        // 🎯 FIX : On crée un élément avec le handle correspondant à l'ID de suppression
        let mut element = ArcadiaElement::default();
        element.handle = "urn:la:ecu".try_into()?;

        let mut model = state.model.lock().await;
        model.add_element("la", "components", element);

        // On s'assure qu'il est bien là avant de le supprimer !
        assert_eq!(model.get_collection("la", "components").len(), 1);
        drop(model);

        let mutation = Mutation {
            element_id: "urn:la:ecu".into(),
            operation: MutationOp::Delete,
            payload: json_value!({}),
        };

        sync.apply_mutation(&mut *state.model.lock().await, &mutation)
            .unwrap();

        let model = state.model.lock().await;
        assert!(
            model.get_collection("la", "components").is_empty(),
            "L'élément n'a pas été supprimé"
        );
        Ok(())
    }

    #[async_test]
    async fn test_delete_idempotence_pure_graph() -> RaiseResult<()> {
        let state = create_test_state();
        let sync = ModelSync::new(&state);

        let mutation = Mutation {
            element_id: "urn:ghost:404".into(),
            operation: MutationOp::Delete,
            payload: json_value!({}),
        };

        // L'élément n'existe pas en mémoire. Cela ne doit PAS renvoyer d'erreur.
        let result = sync.apply_mutation(&mut *state.model.lock().await, &mutation);
        assert!(
            result.is_ok(),
            "La suppression d'un fantôme doit être idempotente et réussir silencieusement."
        );
        Ok(())
    }

    #[async_test]
    async fn test_invalid_payload_rejection() -> RaiseResult<()> {
        let state = create_test_state();
        let sync = ModelSync::new(&state);

        // Un payload qui n'est pas compatible avec ArcadiaElement
        let mutation = Mutation {
            element_id: "urn:error:01".into(),
            operation: MutationOp::Create,
            payload: json_value!(["je", "suis", "un", "tableau"]),
        };

        let result = sync.apply_mutation(&mut *state.model.lock().await, &mutation);
        assert!(
            result.is_err(),
            "Le modèle doit rejeter un payload qui ne correspond pas à ArcadiaElement."
        );

        if let Err(e) = result {
            assert!(e.to_string().contains("ERR_SYNC_PAYLOAD_INVALID"));
        }
        Ok(())
    }
}
