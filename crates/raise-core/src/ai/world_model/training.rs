// FICHIER : src-tauri/src/ai/world_model/training.rs

use crate::utils::prelude::*;

use crate::ai::world_model::engine::NeuroSymbolicEngine;

/// Le Coach du World Model.
/// Totalement découplé de la logique métier (Arcadia). Il ne manipule que des tenseurs latents.
pub struct WorldTrainer<'a> {
    engine: &'a NeuroSymbolicEngine,
    opt: NeuralOptimizerAdamW,
}

impl<'a> WorldTrainer<'a> {
    pub fn new(engine: &'a NeuroSymbolicEngine, lr: f64) -> RaiseResult<Self> {
        let vars = engine.varmap.all_vars();

        let opt = match NeuralOptimizerAdamW::new(
            vars,
            OptimizerConfigAdamW {
                lr,
                ..Default::default()
            },
        ) {
            Ok(optimizer) => optimizer,
            Err(e) => {
                raise_error!(
                    "ERR_AI_OPTIMIZER_INIT_FAILED",
                    error = e,
                    context = json_value!({
                        "learning_rate": lr,
                        "variable_count": engine.varmap.all_vars().len(),
                        "hint": "Échec de l'initialisation du NeuralOptimizerAdamW."
                    })
                )
            }
        };

        Ok(Self { engine, opt })
    }

    /// Exécute une étape d'apprentissage par renforcement (Backpropagation).
    /// * `state_t_tensor` : L'état initial fusionné [1, 32] (Structure + NLP)
    /// * `action_tensor` : L'action encodée en One-Hot [1, Action_Dim]
    /// * `target_t1_tensor` : L'état futur réel fusionné [1, 32] (Structure + NLP)
    pub fn train_step(
        &mut self,
        state_t_tensor: &NeuralTensor,
        action_tensor: &NeuralTensor,
        target_t1_tensor: &NeuralTensor,
    ) -> RaiseResult<f64> {
        // 1. Quantization de l'état initial (Discrétisation façon VQ-VAE)
        let token_t = self.engine.quantizer.tokenize(state_t_tensor)?;
        let state_quantized = self.engine.quantizer.decode(&token_t)?;

        // 2. Inférence (Prédiction de l'état futur)
        let predicted_tensor = self
            .engine
            .predictor
            .forward(&state_quantized, action_tensor)?;

        // 3. Préparation de la cible réelle (Target)
        // On passe la cible réelle dans le quantizer pour obtenir sa représentation latente exacte
        let token_t1 = self.engine.quantizer.tokenize(target_t1_tensor)?;
        let target_latent = self.engine.quantizer.decode(&token_t1)?.detach();

        // 4. Calcul de la Perte (MSE Loss)
        let diff = match predicted_tensor.sub(&target_latent) {
            Ok(d) => d,
            Err(e) => raise_error!(
                "ERR_AI_LOSS_SUB_FAILED",
                error = e,
                context = json_value!({
                    "pred_shape": format!("{:?}", predicted_tensor.shape()),
                    "target_shape": format!("{:?}", target_latent.shape())
                })
            ),
        };

        let loss = match diff.sqr() {
            Ok(s) => match s.mean_all() {
                Ok(m) => m,
                Err(e) => raise_error!("ERR_AI_LOSS_MEAN_FAILED", error = e),
            },
            Err(e) => raise_error!("ERR_AI_LOSS_SQR_FAILED", error = e),
        };

        // 5. Rétropropagation (Backprop)
        match self.opt.backward_step(&loss) {
            Ok(_) => (),
            Err(e) => raise_error!("ERR_AI_BACKPROP_FAILED", error = e),
        };

        // 6. Extraction de la valeur scalaire
        let scalar_loss = match loss.to_scalar::<f32>() {
            Ok(val) => val as f64,
            Err(e) => raise_error!("ERR_AI_LOSS_SCALAR_CONVERSION_FAILED", error = e),
        };

        Ok(scalar_loss)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::world_model::engine::WorldModelConfig;

    #[test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    fn test_training_loop_convergence_tensors() {
        // 1. Setup
        let device = ComputeHardware::Cpu;
        let varmap = NeuralWeightsMap::new();

        // Config adaptée à l'encodeur Hybride : embedding_dim = 32 (16 Struct + 16 NLP)
        let config = WorldModelConfig {
            vocab_size: 10,
            embedding_dim: 32,
            action_dim: 5,
            hidden_dim: 64,
            use_gpu: false,
        };

        let engine = NeuroSymbolicEngine::new(config.clone(), varmap).unwrap();
        let mut trainer = WorldTrainer::new(&engine, 0.05).unwrap();

        // 2. Création de tenseurs factices (Mock) pour simuler la sortie de l'HybridEncoder
        // Plus besoin de dépendre de `ArcadiaElement` !
        let state_t = NeuralTensor::randn(0.0f32, 1.0f32, (1, 32), &device).unwrap();
        let state_t1 = NeuralTensor::randn(0.0f32, 1.0f32, (1, 32), &device).unwrap();

        // Mock de l'action (One-hot pour l'index 0)
        // 🎯 FIX : Forcer le type f32 pour correspondre aux tenseurs d'état
        let action_tensor = NeuralTensor::from_vec(
            vec![1.0_f32, 0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32],
            (1, 5),
            &device,
        )
        .unwrap();

        // 3. Boucle d'entraînement
        let mut initial_loss = 0.0;
        let mut final_loss = 0.0;

        for i in 0..50 {
            let loss = trainer
                .train_step(&state_t, &action_tensor, &state_t1)
                .unwrap();

            if i == 0 {
                initial_loss = loss;
            }
            final_loss = loss;
        }

        println!("Initial Loss: {}, Final Loss: {}", initial_loss, final_loss);
        assert!(
            final_loss < initial_loss,
            "Le modèle n'a pas appris sur les tenseurs hybrides !"
        );
    }
}
