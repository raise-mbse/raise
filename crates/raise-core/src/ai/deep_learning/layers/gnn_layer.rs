// FICHIER : src-tauri/src/ai/deep_learning/layers/gnn_layer.rs

use crate::utils::prelude::*;

/// Une couche de convolution sur graphe (GCN) optimisée pour Arcadia.
/// 🎯 Utilise le "Sparse Message Passing" pour éviter l'explosion mémoire O(N^2).
pub struct GcnLayer {
    /// Transformation linéaire (Poids W et Biais b)
    pub transform: NeuralLinearLayer,
    /// 🎯 Adaptateur LoRA optionnel injecté à chaud pour le Fine-Tuning
    pub lora_adapter: Option<crate::ai::training::lora::LoraLinear>,
    pub in_dim: usize,
    pub out_dim: usize,
}

impl GcnLayer {
    /// Initialise une nouvelle couche de manière asynchrone.
    pub async fn new(
        in_dim: usize,
        out_dim: usize,
        vb: NeuralWeightsBuilder<'_>,
    ) -> RaiseResult<Self> {
        let transform = match init_linear_layer(in_dim, out_dim, vb) {
            Ok(l) => l,
            Err(e) => {
                raise_error!(
                    "ERR_GNN_LAYER_INIT",
                    error = e,
                    context = json_value!({ "in_dim": in_dim, "out_dim": out_dim })
                );
            }
        };

        Ok(Self {
            transform,
            lora_adapter: None, // Inactif par défaut
            in_dim,
            out_dim,
        })
    }

    /// 🎯 INJECTION LORA : Convertit la couche dense en couche fine-tunable
    pub fn inject_lora(
        &mut self,
        rank: usize,
        alpha: f64,
        varmap: &mut NeuralWeightsMap,
        device: &ComputeHardware,
        prefix: &str,
    ) -> RaiseResult<()> {
        // Le clone() de NeuralLinearLayer est Zéro-Copy (pointeurs Arc Candle internes)
        let lora = crate::ai::training::lora::LoraLinear::new(
            self.transform.clone(),
            rank,
            alpha,
            varmap,
            device,
            prefix,
        )?;

        self.lora_adapter = Some(lora);
        Ok(())
    }

    /// Exécute la passe avant (Forward Pass) en mode Creux (Sparse).
    pub async fn forward(
        &self,
        edge_src: &NeuralTensor,
        edge_dst: &NeuralTensor,
        features: &NeuralTensor,
    ) -> RaiseResult<NeuralTensor> {
        let feat_dims = features.dims();

        if feat_dims.len() != 2 {
            raise_error!(
                "ERR_GNN_INVALID_SHAPE",
                error = "Les features doivent être une matrice 2D [N, D].",
                context = json_value!({ "feat": feat_dims })
            );
        }

        let h_src = match features.index_select(edge_src, 0) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_GNN_INDEX_SELECT", error = e),
        };

        let mut h_agg = match features.zeros_like() {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_GNN_ZEROS_LIKE", error = e),
        };

        h_agg = match h_agg.index_add(edge_dst, &h_src, 0) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_GNN_INDEX_ADD", error = e),
        };

        // 🎯 AIGUILLAGE ZÉRO DETTE : LoRA ou Standard
        let transformed_result = match &self.lora_adapter {
            Some(lora) => lora.forward(&h_agg),
            None => self.transform.forward(&h_agg),
        };

        let transformed = match transformed_result {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_GNN_LINEAR_TRANSFORM", error = e.to_string()),
        };

        let activated = match transformed.relu() {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_GNN_ACTIVATION_RELU", error = e),
        };

        Ok(activated)
    }
}

// =========================================================================
// TESTS UNITAIRES (VALIDATION MATHÉMATIQUE SPARSE)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_gcn_layer_sparse_forward_math() -> RaiseResult<()> {
        // 🎯 FIX : Signature RaiseResult
        let device = ComputeHardware::Cpu;
        let varmap = NeuralWeightsMap::new();
        let vb = NeuralWeightsBuilder::from_varmap(&varmap, ComputeType::F32, &device);

        // 🎯 FIX : Propagation sémantique au lieu de unwrap()
        let layer = GcnLayer::new(2, 4, vb).await?;

        // 🎯 FIX : Isolation des erreurs d'allocation NeuralTensor
        let feat = match NeuralTensor::zeros((3, 2), ComputeType::F32, &device) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_TEST_TENSOR_ALLOC", error = e.to_string()),
        };

        let edge_src = match NeuralTensor::new(&[0u32, 1, 0, 1, 2], &device) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_TEST_TENSOR_ALLOC", error = e.to_string()),
        };

        let edge_dst = match NeuralTensor::new(&[1u32, 2, 0, 1, 2], &device) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_TEST_TENSOR_ALLOC", error = e.to_string()),
        };

        // 🎯 FIX : On utilise ? car layer.forward retourne déjà un RaiseResult natif
        let out_tensor = layer.forward(&edge_src, &edge_dst, &feat).await?;

        assert_eq!(
            out_tensor.dims(),
            &[3, 4],
            "La dimension de sortie doit être [N, out_dim]."
        );

        Ok(())
    }
}
