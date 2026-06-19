// FICHIER : src-tauri/src/ai/world_model/perception/mod.rs

// On déclare le sous-module contenant l'implémentation
pub mod encoder;

// On re-exporte la structure principale pour simplifier les imports ailleurs
pub use encoder::ArcadiaEncoder;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_engine::arcadia::element_kind::Layer;
    use crate::utils::prelude::*; // 🎯 FIX : Import du prélude pour accéder à ComputeHardware

    #[test]
    fn test_perception_public_api() {
        // Ce test vérifie que l'API publique est bien accessible depuis le module
        // 🎯 FIX : Injection du device CPU pour le test
        let result = ArcadiaEncoder::encode_layer(Layer::Data, &ComputeHardware::Cpu);

        assert!(
            result.is_ok(),
            "L'API ArcadiaEncoder doit être accessible via perception::ArcadiaEncoder"
        );

        let tensor = result.unwrap();
        // Vérif rapide des dimensions (1, 8 pour les Layers : 7 + Transverse)
        assert_eq!(tensor.dims(), &[1, 8]);
    }
}
