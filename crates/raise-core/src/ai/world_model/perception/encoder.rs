// FICHIER : src-tauri/src/ai/world_model/perception/encoder.rs

use crate::utils::prelude::*;

use crate::model_engine::arcadia::element_kind::{ArcadiaSemantics, ElementCategory, Layer};
use crate::model_engine::types::ArcadiaElement;

/// Dimensions fixes pour l'encodage One-Hot
/// OA, SA, LA, PA, EPBS, Data, Transverse, Unknown -> 8 dimensions
const LAYER_DIM: usize = 8;
/// Component, Function, Actor, Exchange, Interface, Data, Capability, Other -> 8 dimensions
const CATEGORY_DIM: usize = 8;

/// Encodeur Hybride (Stateful) : Fusionne la Topologie MBSE (One-Hot) et la Sémantique NLP (Dense).
pub struct HybridEncoder {
    /// Couche de projection pour réduire l'embedding NLP (ex: 384 -> 16)
    pub semantic_proj: NeuralLinearLayer,
    pub structural_dim: usize,
    pub semantic_dim: usize,
}

impl HybridEncoder {
    /// Initialise l'encodeur hybride avec ses poids entraînables.
    /// * `nlp_dim` : Dimension de sortie du modèle FastEmbed (ex: 384 pour BGE-Small).
    /// * `semantic_dim` : Dimension compressée cible (ex: 16, pour équilibrer avec les 16 dims structurelles).
    pub fn new(
        nlp_dim: usize,
        semantic_dim: usize,
        vb: NeuralWeightsBuilder<'_>,
    ) -> RaiseResult<Self> {
        let semantic_proj = match init_linear_layer(nlp_dim, semantic_dim, vb.pp("semantic_proj")) {
            Ok(layer) => layer,
            Err(e) => raise_error!(
                "ERR_AI_HYBRID_ENCODER_INIT",
                error = e.to_string(),
                context = json_value!({ "nlp_dim": nlp_dim, "semantic_dim": semantic_dim })
            ),
        };

        Ok(Self {
            semantic_proj,
            structural_dim: LAYER_DIM + CATEGORY_DIM, // 16
            semantic_dim,
        })
    }

    /// Encode un élément en fusionnant sa nature structurelle et son sens NLP.
    /// Retourne un tenseur de dimension [1, structural_dim + semantic_dim].
    pub fn encode_hybrid(
        &self,
        element: &ArcadiaElement,
        nlp_embedding: &[f32], // Le vecteur issu de fast.rs (FastEmbedEngine)
        device: &ComputeHardware,
    ) -> RaiseResult<NeuralTensor> {
        // 1. Perception Structurelle Pure (Zéro Dette) -> [1, 16]
        let t_structure = ArcadiaEncoder::encode_element(element, device)?;

        // 2. Conversion du vecteur NLP en Tenseur -> [1, 384]
        let nlp_dim = nlp_embedding.len();
        let t_nlp = match NeuralTensor::from_vec(nlp_embedding.to_vec(), (1, nlp_dim), device) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_AI_HYBRID_TENSOR_ALLOC", error = e.to_string()),
        };

        // 3. Projection Sémantique -> [1, 16]
        let t_semantic_raw = match self.semantic_proj.forward(&t_nlp) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_AI_HYBRID_PROJECTION", error = e.to_string()),
        };

        // 4. Activation Non-Linéaire (GELU) pour extraire les features complexes
        let t_semantic = match NeuralActivation::Gelu.forward(&t_semantic_raw) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_AI_HYBRID_ACTIVATION", error = e.to_string()),
        };

        // 5. Fusion finale (Concaténation sur la dimension 1) -> [1, 32]
        match NeuralTensor::cat(&[&t_structure, &t_semantic], 1) {
            Ok(t_combined) => Ok(t_combined),
            Err(e) => raise_error!(
                "ERR_AI_HYBRID_FUSION",
                error = e.to_string(),
                context = json_value!({
                    "struct_shape": format!("{:?}", t_structure.shape()),
                    "semantic_shape": format!("{:?}", t_semantic.shape())
                })
            ),
        }
    }
}

/// Encodeur sans état (Stateless) pour transformer les concepts Arcadia en Tenseurs.
pub struct ArcadiaEncoder;

impl ArcadiaEncoder {
    /// Encode la couche (Layer) en vecteur One-Hot [1, 8]
    pub fn encode_layer(layer: Layer, device: &ComputeHardware) -> RaiseResult<NeuralTensor> {
        let index = match layer {
            Layer::OperationalAnalysis => 0,
            Layer::SystemAnalysis => 1,
            Layer::LogicalArchitecture => 2,
            Layer::PhysicalArchitecture => 3,
            Layer::EPBS => 4,
            Layer::Data => 5,
            Layer::Transverse => 6,
            Layer::Unknown => 7,
        };

        Self::one_hot(index, LAYER_DIM, device)
    }

    /// Encode la catégorie fonctionnelle en vecteur One-Hot [1, 8]
    pub fn encode_category(
        category: ElementCategory,
        device: &ComputeHardware,
    ) -> RaiseResult<NeuralTensor> {
        let index = match category {
            ElementCategory::Component => 0,
            ElementCategory::Function => 1,
            ElementCategory::Actor => 2,
            ElementCategory::Exchange => 3,
            ElementCategory::Interface => 4,
            ElementCategory::Data => 5,
            ElementCategory::Capability => 6,
            ElementCategory::Other => 7,
        };

        Self::one_hot(index, CATEGORY_DIM, device)
    }

    /// Encode un élément complet (Concaténation Layer + Category)
    /// Dimension de sortie : [1, 16] (8 + 8)
    pub fn encode_element(
        element: &ArcadiaElement,
        device: &ComputeHardware,
    ) -> RaiseResult<NeuralTensor> {
        // 1. Extraction sémantique
        let layer = element.get_layer();
        let category = element.get_category();

        // 2. Encodage individuel (délégué aux sous-fonctions RAISE-safe)
        let t_layer = Self::encode_layer(layer, device)?;
        let t_cat = Self::encode_category(category, device)?;

        // 3. Concaténation (Feature Fusion)
        let t_combined = match NeuralTensor::cat(&[&t_layer, &t_cat], 1) {
            Ok(t) => t,
            Err(e) => raise_error!(
                "ERR_AI_ENCODER_FUSION_FAILED",
                error = e,
                context = json_value!({
                    "layer_shape": format!("{:?}", t_layer.shape()),
                    "category_shape": format!("{:?}", t_cat.shape()),
                    "action": "concatenate_features"
                })
            ),
        };

        Ok(t_combined)
    }

    /// Helper pour générer un vecteur One-Hot
    fn one_hot(index: usize, size: usize, device: &ComputeHardware) -> RaiseResult<NeuralTensor> {
        let mut data = vec![0f32; size];
        if index < size {
            data[index] = 1.0;
        }

        // 🎯 FIX ABSOLU : Le CPU n'est plus codé en dur !
        match NeuralTensor::from_vec(data, (1, size), device) {
            Ok(t) => Ok(t),
            Err(e) => raise_error!(
                "ERR_AI_ENCODER_ONE_HOT_FAILED",
                error = e,
                context = json_value!({
                    "index": index,
                    "size": size,
                    "hint": "Échec de la création du vecteur One-Hot."
                })
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_engine::types::{ArcadiaElement, NameType};

    // Helper pour créer un élément dummy
    fn make_element(kind: &str) -> ArcadiaElement {
        ArcadiaElement {
            id: "test_id".to_string(),
            name: NameType::default(),
            kind: kind.to_string(),
            properties: UnorderedMap::new(),
        }
    }

    #[test]
    fn test_encode_layer_sa() {
        // 🎯 FIX : Ajout du device manquant pour le test
        let t = ArcadiaEncoder::encode_layer(Layer::SystemAnalysis, &ComputeHardware::Cpu).unwrap();
        let vec: Vec<f32> = t.to_vec2::<f32>().unwrap()[0].clone();

        assert_eq!(vec.len(), LAYER_DIM);
        assert_eq!(vec[1], 1.0);
        assert_eq!(vec[0], 0.0);
    }

    #[test]
    fn test_encode_category_function() {
        // 🎯 FIX : Ajout du device manquant pour le test
        let t = ArcadiaEncoder::encode_category(ElementCategory::Function, &ComputeHardware::Cpu)
            .unwrap();
        let vec: Vec<f32> = t.to_vec2::<f32>().unwrap()[0].clone();

        assert_eq!(vec.len(), CATEGORY_DIM);
        assert_eq!(vec[1], 1.0);
    }

    #[test]
    fn test_encode_full_element() {
        let el = make_element("https://raise.io/ontology/arcadia/la#LogicalComponent");

        // 🎯 FIX : Ajout du device manquant pour le test
        let t = ArcadiaEncoder::encode_element(&el, &ComputeHardware::Cpu).unwrap();
        let vec: Vec<f32> = t.to_vec2::<f32>().unwrap()[0].clone();

        // Taille totale attendue : 8 + 8 = 16
        assert_eq!(vec.len(), LAYER_DIM + CATEGORY_DIM);
        assert_eq!(vec[2], 1.0, "Layer index 2 (LA) doit être 1.0");
        assert_eq!(
            vec[8], 1.0,
            "Category index 0 (Component) décalé de 8 doit être 1.0"
        );
    }

    #[test]
    fn test_hybrid_encoder_fusion() {
        let device = ComputeHardware::Cpu;
        let varmap = NeuralWeightsMap::new();
        let vb = NeuralWeightsBuilder::from_varmap(&varmap, ComputeType::F32, &device);

        let hybrid_encoder = HybridEncoder::new(384, 16, vb).unwrap();
        let el = make_element("https://raise.io/ontology/arcadia/la#LogicalFunction");
        let mock_nlp_vec = vec![0.5f32; 384];

        let combined_tensor = hybrid_encoder
            .encode_hybrid(&el, &mock_nlp_vec, &device)
            .unwrap();
        let dims = combined_tensor.dims();

        assert_eq!(
            dims,
            &[1, 32],
            "Le tenseur hybride doit faire 32 dimensions."
        );
    }
}
