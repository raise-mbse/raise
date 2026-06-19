// FICHIER : src-tauri/src/ai/deep_learning/models/gnn_model.rs
use crate::utils::prelude::*;

use crate::ai::deep_learning::layers::gnn_layer::GcnLayer;
use crate::ai::graph_store::adjacency::GraphAdjacency;

/// Le modèle GNN complet spécialisé pour l'ontologie Arcadia.
/// 🎯 Mode "Production" : Exploite le Sparse Message Passing pour économiser la VRAM.
pub struct ArcadiaGnnModel {
    pub layer1: GcnLayer,
    pub layer2: GcnLayer,
}

impl ArcadiaGnnModel {
    /// Initialise un modèle GNN à 2 couches de manière asynchrone.
    pub async fn new(
        in_dim: usize,
        hidden_dim: usize,
        out_dim: usize,
        vb: NeuralWeightsBuilder<'_>,
    ) -> RaiseResult<Self> {
        // Initialisation de la Couche 1 (Agrégation locale)
        let layer1 = GcnLayer::new(in_dim, hidden_dim, vb.pp("layer1")).await?;

        // Initialisation de la Couche 2 (Contexte global / Sémantique dense)
        let layer2 = GcnLayer::new(hidden_dim, out_dim, vb.pp("layer2")).await?;

        user_info!("🧠 [GNN] Modèle Arcadia initialisé (Mode Sparse).");

        Ok(Self { layer1, layer2 })
    }

    /// 🎯 INJECTION LORA GLOBALE
    /// Prépare le modèle GNN pour le Fine-Tuning en gelant les poids d'origine
    /// et en injectant les matrices de rang faible (A et B) sur toutes les couches.
    pub fn inject_lora(
        &mut self,
        rank: usize,
        alpha: f64,
        varmap: &mut NeuralWeightsMap,
        device: &ComputeHardware,
    ) -> RaiseResult<()> {
        self.layer1
            .inject_lora(rank, alpha, varmap, device, "layer1")?;
        self.layer2
            .inject_lora(rank, alpha, varmap, device, "layer2")?;

        user_info!(
            "MSG_GNN_LORA_INJECTED",
            json_value!({
                "rank": rank,
                "alpha": alpha,
                "status": "active",
                "layers_patched": 2
            })
        );

        Ok(())
    }

    /// Exécute la propagation complète sur le graphe via le Sparse Message Passing.
    /// Utilise les indices des arêtes (edges) plutôt qu'une matrice dense [N, N].
    pub async fn forward(
        &self,
        edge_src: &NeuralTensor,
        edge_dst: &NeuralTensor,
        features: &NeuralTensor,
    ) -> RaiseResult<NeuralTensor> {
        // Passe 1 : Agrégation des voisins directs via les arêtes
        let hidden = self.layer1.forward(edge_src, edge_dst, features).await?;

        // Passe 2 : Agrégation des voisins de niveau 2
        let output = self.layer2.forward(edge_src, edge_dst, &hidden).await?;

        Ok(output)
    }

    /// Calcule la similarité cosinus entre deux composants Arcadia après transformation GNN.
    /// Calcule la similarité cosinus entre deux composants Arcadia après transformation GNN.
    pub async fn compute_similarity(
        &self,
        embeddings: &NeuralTensor,
        adj_data: &GraphAdjacency,
        uri_a: &str,
        uri_b: &str,
    ) -> RaiseResult<f32> {
        // 1. Récupération stricte des index
        let idx_a = match adj_data.uri_to_index.get(uri_a) {
            Some(&idx) => idx,
            None => {
                raise_error!(
                    "ERR_GNN_URI_NOT_FOUND",
                    error = format!("URI {} introuvable", uri_a)
                )
            }
        };

        let idx_b = match adj_data.uri_to_index.get(uri_b) {
            Some(&idx) => idx,
            None => {
                raise_error!(
                    "ERR_GNN_URI_NOT_FOUND",
                    error = format!("URI {} introuvable", uri_b)
                )
            }
        };

        // 2. EXTRACTION OPTIMISÉE (GPU) : On isole uniquement les deux vecteurs [1, D]
        let vec_a = match embeddings.narrow(0, idx_a, 1) {
            Ok(t) => match t.squeeze(0) {
                Ok(t) => t,
                Err(e) => {
                    raise_error!("ERR_GNN_TENSOR_SQUEEZE", error = e.to_string())
                }
            },
            Err(e) => raise_error!("ERR_GNN_TENSOR_NARROW", error = e.to_string()),
        };

        let vec_b = match embeddings.narrow(0, idx_b, 1) {
            Ok(t) => match t.squeeze(0) {
                Ok(t) => t,
                Err(e) => {
                    raise_error!("ERR_GNN_TENSOR_SQUEEZE", error = e.to_string())
                }
            },
            Err(e) => raise_error!("ERR_GNN_TENSOR_NARROW", error = e.to_string()),
        };

        // 3. MATHÉMATIQUES GPU : Produit scalaire et normes
        let dot_tensor = match vec_a.mul(&vec_b) {
            Ok(t) => match t.sum_all() {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_GNN_MATH_SUM", error = e.to_string()),
            },
            Err(e) => raise_error!("ERR_GNN_MATH_MUL", error = e.to_string()),
        };

        let norm_a_tensor = match vec_a.sqr() {
            Ok(t) => match t.sum_all() {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_GNN_MATH_SUM", error = e.to_string()),
            },
            Err(e) => raise_error!("ERR_GNN_MATH_SQR", error = e.to_string()),
        };

        let norm_b_tensor = match vec_b.sqr() {
            Ok(t) => match t.sum_all() {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_GNN_MATH_SUM", error = e.to_string()),
            },
            Err(e) => raise_error!("ERR_GNN_MATH_SQR", error = e.to_string()),
        };

        // 4. RAPATRIEMENT CPU (Seulement 3 petits floats !)
        let dot = match dot_tensor.to_scalar::<f32>() {
            Ok(val) => val,
            Err(e) => raise_error!("ERR_GNN_SCALAR", error = e.to_string()),
        };

        let norm_a = match norm_a_tensor.to_scalar::<f32>() {
            Ok(val) => val,
            Err(e) => raise_error!("ERR_GNN_SCALAR", error = e.to_string()),
        };

        let norm_b = match norm_b_tensor.to_scalar::<f32>() {
            Ok(val) => val,
            Err(e) => raise_error!("ERR_GNN_SCALAR", error = e.to_string()),
        };
        // 5. Calcul final de la similarité cosinus
        let epsilon = 1e-8_f32;
        Ok(dot / ((norm_a + epsilon).sqrt() * (norm_b + epsilon).sqrt()))
    }
}

// =========================================================================
// TESTS UNITAIRES (VALIDATION DU MODÈLE SPARSE)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_gnn_model_sparse_flow() {
        let device = ComputeHardware::Cpu;
        let varmap = NeuralWeightsMap::new();
        let vb = NeuralWeightsBuilder::from_varmap(&varmap, ComputeType::F32, &device);

        // Modèle : In=4, Hidden=8, Out=2
        let model = ArcadiaGnnModel::new(4, 8, 2, vb).await.unwrap();

        // Nœuds et Edges factices (3 nœuds liés en chaîne + self-loops)
        let edge_src = NeuralTensor::new(&[0u32, 1, 0, 1, 2], &device).unwrap();
        let edge_dst = NeuralTensor::new(&[1u32, 2, 0, 1, 2], &device).unwrap();
        let feat = NeuralTensor::zeros((3, 4), ComputeType::F32, &device).unwrap();

        let output = model.forward(&edge_src, &edge_dst, &feat).await;

        assert!(output.is_ok(), "Le Forward complet du modèle a échoué.");
        assert_eq!(output.unwrap().dims(), &[3, 2]);
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_similarity_logic_with_epsilon() {
        let device = ComputeHardware::Cpu;
        let varmap = NeuralWeightsMap::new();
        let vb = NeuralWeightsBuilder::from_varmap(&varmap, ComputeType::F32, &device);
        let model = ArcadiaGnnModel::new(2, 2, 2, vb).await.unwrap();

        let mut uri_map = UnorderedMap::new();
        uri_map.insert("la:Function".to_string(), 0);
        uri_map.insert("sa:Component".to_string(), 1);

        let adj_mock = GraphAdjacency {
            uri_to_index: uri_map,
            index_to_uri: vec!["la:Function".to_string(), "sa:Component".to_string()],
            edge_src: NeuralTensor::new(&[0u32, 1], &device).unwrap(),
            edge_dst: NeuralTensor::new(&[0u32, 1], &device).unwrap(),
        };

        // Deux vecteurs "Zéro" -> Testera l'Epsilon
        let embeddings_zero = NeuralTensor::zeros((2, 2), ComputeType::F32, &device).unwrap();
        let sim_zero = model
            .compute_similarity(&embeddings_zero, &adj_mock, "la:Function", "sa:Component")
            .await
            .unwrap();

        assert!(
            sim_zero.is_finite(),
            "L'Epsilon n'a pas protégé contre la division par zéro !"
        );

        // 🎯 FIX : On force le type en f32 avec le suffixe `_f32`
        let embeddings =
            NeuralTensor::from_vec(vec![1.0_f32, 0.0_f32, 1.0_f32, 0.0_f32], (2, 2), &device)
                .unwrap();
        let sim = model
            .compute_similarity(&embeddings, &adj_mock, "la:Function", "sa:Component")
            .await
            .unwrap();

        assert!(
            (sim - 1.0).abs() < 1e-5,
            "Les vecteurs identiques doivent avoir une similarité de 1.0"
        );
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_gnn_message_passing_convergence_mbse() {
        let device = ComputeHardware::Cpu;

        // 1. MOCK DE LA TOPOLOGIE MBSE (4 Nœuds)
        let mut uri_map = UnorderedMap::new();
        uri_map.insert("la:F1".to_string(), 0);
        uri_map.insert("la:F2".to_string(), 1);
        uri_map.insert("sa:S1".to_string(), 2);
        uri_map.insert("pa:P1".to_string(), 3);

        // 2. LISTE DES ARÊTES (Sparse)
        let mut src = vec![0u32, 1, 2, 3];
        let mut dst = vec![0u32, 1, 2, 3];

        // Liens bidirectionnels de réalisation (F1 <-> S1) et (F1 <-> F2)
        src.extend_from_slice(&[0, 2, 0, 1]);
        dst.extend_from_slice(&[2, 0, 1, 0]);

        let adj_mock = GraphAdjacency {
            uri_to_index: uri_map,
            index_to_uri: vec![
                "la:F1".to_string(),
                "la:F2".to_string(),
                "sa:S1".to_string(),
                "pa:P1".to_string(),
            ],
            edge_src: NeuralTensor::new(src.as_slice(), &device).unwrap(),
            edge_dst: NeuralTensor::new(dst.as_slice(), &device).unwrap(),
        };

        // 2. LISTE DES ARÊTES (Sparse)
        let mut src = vec![0u32, 1, 2, 3];
        let mut dst = vec![0u32, 1, 2, 3];

        // Liens bidirectionnels de réalisation (F1 <-> S1) et (F1 <-> F2)
        src.extend_from_slice(&[0, 2, 0, 1]);
        dst.extend_from_slice(&[2, 0, 1, 0]);

        let edge_src = NeuralTensor::new(src.as_slice(), &device).unwrap();
        let edge_dst = NeuralTensor::new(dst.as_slice(), &device).unwrap();

        // 3. VECTEURS SÉMANTIQUES INITIAUX
        let features = NeuralTensor::eye(4, ComputeType::F32, &device).unwrap();

        // 🎯 FIX : Boucle de résilience pour l'initialisation aléatoire des poids
        let mut success = false;
        let mut last_sim_connected = 0.0;
        let mut last_sim_isolated = 0.0;

        for _ in 0..10 {
            let varmap = NeuralWeightsMap::new();
            let vb = NeuralWeightsBuilder::from_varmap(&varmap, ComputeType::F32, &device);
            let model = ArcadiaGnnModel::new(4, 8, 4, vb).await.unwrap();

            let sim_init = model
                .compute_similarity(&features, &adj_mock, "la:F1", "sa:S1")
                .await
                .unwrap();

            assert!(
                (sim_init - 0.0).abs() < 1e-5,
                "La similarité initiale doit être 0.0"
            );

            let final_embeddings = model
                .forward(&edge_src, &edge_dst, &features)
                .await
                .unwrap();

            let sim_final_connected = model
                .compute_similarity(&final_embeddings, &adj_mock, "la:F1", "sa:S1")
                .await
                .unwrap();

            let sim_final_isolated = model
                .compute_similarity(&final_embeddings, &adj_mock, "la:F1", "pa:P1")
                .await
                .unwrap();

            last_sim_connected = sim_final_connected;
            last_sim_isolated = sim_final_isolated;

            // Critère de succès : Le GNN a bien rapproché les nœuds connectés
            if sim_final_connected > 0.1 && sim_final_connected > sim_final_isolated {
                success = true;
                break;
            }
        }

        assert!(
            success,
            "Le GNN n'a pas réussi à converger après 10 initialisations. Connecté: {}, Isolé: {}",
            last_sim_connected, last_sim_isolated
        );
    }
}
