// FICHIER : crates/raise-core/src/ai/training/lora.rs

use crate::utils::prelude::*;

pub struct LoraLinear {
    old_linear: NeuralLinearLayer,
    pub lora_a: NeuralTensor, // Projection : [Out, Rank]
    pub lora_b: NeuralTensor, // Réduction  : [Rank, In]
    scale: f64,
}

impl LoraLinear {
    pub fn new(
        old_linear: NeuralLinearLayer,
        rank: usize,
        alpha: f64,
        varmap: &mut NeuralWeightsMap,
        device: &ComputeHardware,
        prefix: &str, // 🎯 FIX : Réception du préfixe
    ) -> RaiseResult<Self> {
        let (out_dims, in_dims) = old_linear.weight().shape().dims2()?;
        let dtype = old_linear.weight().dtype();

        // lora_a : [Out, Rank]
        // 🎯 FIX : Concaténation du préfixe pour garantir l'unicité dans la VRAM
        let lora_a = varmap.get(
            (out_dims, rank),
            &format!("{}.lora_a", prefix),
            NeuralInitStrategy::DEFAULT_KAIMING_NORMAL,
            dtype,
            device,
        )?;

        // lora_b : [Rank, In]
        // 🎯 FIX : Concaténation du préfixe
        let lora_b = varmap.get(
            (rank, in_dims),
            &format!("{}.lora_b", prefix),
            NeuralInitStrategy::ZERO,
            dtype,
            device,
        )?;

        let scale = alpha / rank as f64;

        Ok(Self {
            old_linear,
            lora_a,
            lora_b,
            scale,
        })
    }
}

impl NeuralModule for LoraLinear {
    fn forward(&self, x: &NeuralTensor) -> std::result::Result<NeuralTensor, NeuralCoreError> {
        // Calcul standard
        let standard_output = self.old_linear.forward(x)?;

        // Calcul LoRA corrigé :
        // 1. x [1, In] * lora_b^T [In, Rank] -> [1, Rank]
        // 2. [1, Rank] * lora_a^T [Rank, Out] -> [1, Out]
        let lora_output = x
            .matmul(&self.lora_b.t()?)? // Réduction vers le rang
            .matmul(&self.lora_a.t()?)?; // Projection vers la sortie

        standard_output.add(&(lora_output * self.scale)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial] // Sécurité : L'orchestrateur charge l'IA
    #[cfg_attr(not(feature = "cuda"), ignore)]
    fn test_lora_linear_forward_shape() -> RaiseResult<()> {
        let device = ComputeHardware::Cpu;
        let mut varmap = NeuralWeightsMap::new();

        // Simule une couche 10 (In) -> 20 (Out)
        let weight = NeuralTensor::zeros((20, 10), ComputeType::F32, &device)?;
        let bias = NeuralTensor::zeros(20, ComputeType::F32, &device)?;
        let linear = NeuralLinearLayer::new(weight, Some(bias));

        // 🎯 FIX : Ajout du préfixe "test_layer"
        let lora = LoraLinear::new(linear, 4, 1.0, &mut varmap, &device, "test_layer")?;

        // Input [1, 10]
        let input = NeuralTensor::ones((1, 10), ComputeType::F32, &device)?;
        let output = lora.forward(&input)?;

        // Output doit être [1, 20]
        assert_eq!(output.shape().dims(), &[1, 20]);
        Ok(())
    }
}
