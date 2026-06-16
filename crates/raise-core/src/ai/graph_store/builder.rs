// FICHIER : crates/raise-core/src/ai/graph_store/builder.rs

use crate::ai::graph_store::adjacency::GraphAdjacency;
use crate::ai::world_model::perception::encoder::HybridEncoder;
use crate::json_db::collections::manager::CollectionsManager;
use crate::model_engine::types::{ArcadiaElement, NameType};
use crate::utils::prelude::*;

pub struct SoftwareGraphBuilder;

impl SoftwareGraphBuilder {
    /// 🎯 Construit le graphe topologique et sémantique complet du code source.
    /// Retourne l'adjacence creuse (COO) et la matrice de caractéristiques [N, 32].
    pub async fn build_code_graph(
        manager: &CollectionsManager<'_>,
        hybrid_encoder: &HybridEncoder,
        device: &ComputeHardware,
    ) -> RaiseResult<(GraphAdjacency, NeuralTensor)> {
        // La chaîne ADN de votre architecture logicielle
        let collections = vec![
            "dapps",
            "services",
            "components",
            "modules",
            "code_elements",
        ];

        let mut uri_map = UnorderedMap::new();
        let mut uri_vec = Vec::new();
        let mut documents = Vec::new();

        // ==========================================
        // ÉTAPE 1 : DÉCOUVERTE DES NŒUDS
        // ==========================================
        for col in &collections {
            if let Ok(docs) = manager.list_all(col).await {
                for doc in docs {
                    // Stratégie Zéro Dette : On priorise le handle (sémantique) puis le _id (physique)
                    let id = doc
                        .get("handle")
                        .and_then(|v| v.as_str())
                        .or_else(|| doc.get("_id").and_then(|v| v.as_str()))
                        .unwrap_or_default();

                    if !id.is_empty() {
                        let uri = format!("{}:{}", col, id); // ex: code_elements:fn_missing_file_context
                        uri_map.insert(uri.clone(), uri_vec.len());
                        uri_vec.push(uri);
                        documents.push(doc);
                    }
                }
            }
        }

        let n = uri_vec.len();
        if n == 0 {
            raise_error!(
                "ERR_GRAPH_BUILDER_EMPTY",
                error = "Aucun élément logiciel trouvé dans la base de données."
            );
        }

        crate::user_info!(
            "MSG_SOFTWARE_GRAPH_START",
            json_value!({ "nodes": n, "action": "building_topology" })
        );

        // ==========================================
        // ÉTAPE 2 : DÉCOUVERTE DES ARÊTES (TOPOLOGIE)
        // ==========================================
        let mut src_indices = Vec::new();
        let mut dst_indices = Vec::new();

        // A. Self-loops obligatoires pour le GNN
        for i in 0..n {
            src_indices.push(i as u32);
            dst_indices.push(i as u32);
        }

        // B. Analyse des liens documentaires
        for (i, doc) in documents.iter().enumerate() {
            // 1. Liens hiérarchiques (Composition)
            let parent_keys = [
                "module_id",
                "module_handle",
                "parent_id",
                "component_id",
                "service_id",
                "dapp_id",
            ];
            for key in parent_keys {
                if let Some(parent_ref) = doc.get(key).and_then(|v| v.as_str()) {
                    for col in &collections {
                        let possible_uri = format!("{}:{}", col, parent_ref);
                        if let Some(&j) = uri_map.get(&possible_uri) {
                            // Lien bidirectionnel car l'enfant appartient au parent, et le parent contient l'enfant
                            src_indices.push(i as u32);
                            dst_indices.push(j as u32);
                            src_indices.push(j as u32);
                            dst_indices.push(i as u32);
                            break;
                        }
                    }
                }
            }

            // 2. Liens transverses (Dépendances d'exécution)
            if let Some(deps) = doc.get("dependencies").and_then(|v| v.as_array()) {
                for dep in deps {
                    if let Some(dep_ref) = dep.as_str() {
                        for col in &collections {
                            let possible_uri = format!("{}:{}", col, dep_ref);
                            if let Some(&j) = uri_map.get(&possible_uri) {
                                // Lien directionnel : i utilise j
                                src_indices.push(i as u32);
                                dst_indices.push(j as u32);
                                break;
                            }
                        }
                    }
                }
            }
        }

        // ==========================================
        // ÉTAPE 3 : ENCODAGE HYBRIDE (SÉMANTIQUE + STRUCTURE)
        // ==========================================
        let mut feature_tensors = Vec::with_capacity(n);

        for doc in &documents {
            // Mapping à la volée vers l'ontologie Arcadia pour notre HybridEncoder
            let kind = match doc.get("element_type").and_then(|v| v.as_str()) {
                Some(t) if t.eq_ignore_ascii_case("function") => {
                    "https://raise.io/ontology/arcadia/pa#PhysicalFunction"
                }
                _ => "https://raise.io/ontology/arcadia/pa#PhysicalComponent",
            };

            let element = ArcadiaElement {
                id: doc
                    .get("_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                name: NameType::default(),
                kind: kind.to_string(),
                properties: UnorderedMap::new(),
            };

            // Récupération de l'embedding NLP (S'il est absent, on retourne un vecteur zéro)
            let nlp_vec: Vec<f32> = doc
                .get("nlp_embedding")
                .and_then(|v| crate::utils::data::json::deserialize_from_value(v.clone()).ok())
                .unwrap_or_else(|| vec![0.0f32; 384]);

            // Encodage Zéro Dette [1, 32]
            let feat_tensor = hybrid_encoder.encode_hybrid(&element, &nlp_vec, device)?;
            feature_tensors.push(feat_tensor);
        }

        // ==========================================
        // ÉTAPE 4 : ASSEMBLAGE DES TENSEURS (GPU/CPU ISOLÉ)
        // ==========================================
        let device_clone = device.clone();
        let edges_count = src_indices.len();

        let tensor_result = os::execute_native_inference(move || {
            let t_src = match NeuralTensor::new(src_indices, &device_clone) {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_GNN_BUILDER_SRC", error = e.to_string()),
            };
            let t_dst = match NeuralTensor::new(dst_indices, &device_clone) {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_GNN_BUILDER_DST", error = e.to_string()),
            };

            let feat_refs: Vec<&NeuralTensor> = feature_tensors.iter().collect();
            // Concaténation sur la dimension Batch (0) -> Matrice finale [N, 32]
            let t_features = match NeuralTensor::cat(&feat_refs, 0) {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_GNN_BUILDER_CAT", error = e.to_string()),
            };

            Ok((t_src, t_dst, t_features))
        })
        .await;

        let (edge_src, edge_dst, features) = match tensor_result {
            Ok(res) => res,
            Err(e) => return Err(e),
        };

        crate::user_success!(
            "MSG_SOFTWARE_GRAPH_READY",
            json_value!({ "nodes": n, "edges": edges_count, "features_shape": format!("{:?}", features.dims()) })
        );

        let adjacency = GraphAdjacency {
            uri_to_index: uri_map,
            index_to_uri: uri_vec,
            edge_src,
            edge_dst,
        };

        Ok((adjacency, features))
    }
}

// =========================================================================
// TESTS UNITAIRES (Validation Topologique et Hybride)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::data::config::AppConfig;
    use crate::utils::testing::{AgentDbSandbox, DbSandbox};

    /// Helper pour initialiser un HybridEncoder factice pour les tests
    fn setup_test_encoder(device: &ComputeHardware) -> RaiseResult<HybridEncoder> {
        let varmap = NeuralWeightsMap::new();
        let vb = NeuralWeightsBuilder::from_varmap(&varmap, ComputeType::F32, device);
        HybridEncoder::new(384, 16, vb)
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_software_graph_builder_full_flow() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        // 🎯 FIX ZÉRO DETTE : On utilise un espace de travail isolé ("test_workspace")
        // pour ne pas aspirer les composants natifs de l'AgentDbSandbox.
        let manager = CollectionsManager::new(&sandbox.db, "test_workspace", "master");

        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );

        DbSandbox::mock_db(&manager).await?;
        // Création des collections cibles
        manager.create_collection("modules", &schema_uri).await?;
        manager
            .create_collection("code_elements", &schema_uri)
            .await?;

        // 1. Insertion d'un Module
        manager
            .insert_raw(
                "modules",
                &json_value!({
                    "_id": "mod_1",
                    "handle": "mod_assets",
                    "element_type": "Module"
                }),
            )
            .await?;

        // 2. Insertion d'une Fonction ENFANT du Module
        manager
            .insert_raw(
                "code_elements",
                &json_value!({
                    "_id": "fn_1",
                    "handle": "fn_resolve",
                    "module_handle": "mod_assets", // Lien de parenté fort
                    "element_type": "Function",
                    "nlp_embedding": vec![0.5f32; 384] // Mock de la partie Sémantique
                }),
            )
            .await?;

        let device = ComputeHardware::Cpu;
        let encoder = setup_test_encoder(&device)?;

        // EXÉCUTION
        let (adj, features) =
            SoftwareGraphBuilder::build_code_graph(&manager, &encoder, &device).await?;

        // 🎯 FIX : Assertions relatives (Tolérance aux fixtures du Sandbox)
        let n_nodes = adj.index_to_uri.len();
        assert!(n_nodes >= 2, "Le graphe doit trouver au moins nos 2 nœuds.");
        assert!(
            adj.uri_to_index.contains_key("modules:mod_assets"),
            "Le module doit être indexé."
        );
        assert!(
            adj.uri_to_index.contains_key("code_elements:fn_resolve"),
            "La fonction doit être indexée."
        );

        let edge_count = adj.edge_src.dims()[0];
        // Topologie : N self-loops + 2 Arêtes (Lien Parent->Enfant bidirectionnel)
        let expected_edges = n_nodes + 2;
        assert_eq!(
            edge_count, expected_edges,
            "Il devrait y avoir N self-loops + 2 arêtes structurelles."
        );

        assert_eq!(
            features.dims(),
            &[n_nodes, 32],
            "Les features extraites doivent être une matrice [N, 32]."
        );

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_software_graph_builder_dependencies() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        // 🎯 FIX : Espace isolé
        let manager = CollectionsManager::new(&sandbox.db, "test_workspace", "master");
        DbSandbox::mock_db(&manager).await?;
        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );

        manager
            .create_collection("code_elements", &schema_uri)
            .await?;

        // Fonction A qui appelle la Fonction B
        manager
            .insert_raw(
                "code_elements",
                &json_value!({
                    "_id": "fn_A",
                    "handle": "fn_caller",
                    "element_type": "Function",
                    "dependencies": ["fn_target"] // Lien transversal
                }),
            )
            .await?;

        manager
            .insert_raw(
                "code_elements",
                &json_value!({
                    "_id": "fn_B",
                    "handle": "fn_target",
                    "element_type": "Function"
                }),
            )
            .await?;

        let device = ComputeHardware::Cpu;
        let encoder = setup_test_encoder(&device)?;

        let (adj, _) = SoftwareGraphBuilder::build_code_graph(&manager, &encoder, &device).await?;

        let n_nodes = adj.index_to_uri.len();
        let edge_count = adj.edge_src.dims()[0];

        // Topologie : N self-loops + 1 Arête directionnelle (A utilise B)
        let expected_edges = n_nodes + 1;
        assert_eq!(
            edge_count, expected_edges,
            "Il devrait y avoir N self-loops + 1 arête directionnelle."
        );

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_software_graph_builder_empty_db() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        // On cible un domaine vide
        let manager = CollectionsManager::new(&sandbox.db, "void_domain", "void_db");

        let device = ComputeHardware::Cpu;
        let encoder = setup_test_encoder(&device)?;

        let res = SoftwareGraphBuilder::build_code_graph(&manager, &encoder, &device).await;

        assert!(
            res.is_err(),
            "Le Builder doit crasher proprement si aucune collection n'existe."
        );

        let err_str = res.unwrap_err().to_string();
        assert!(
            err_str.contains("ERR_GRAPH_BUILDER_EMPTY"),
            "Le code d'erreur doit être ERR_GRAPH_BUILDER_EMPTY."
        );

        Ok(())
    }
}
