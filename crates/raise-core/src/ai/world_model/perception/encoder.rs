// FICHIER : src-tauri/src/ai/world_model/perception/encoder.rs

use crate::utils::prelude::*;

use crate::model_engine::arcadia::element_kind::{ArcadiaSemantics, ElementCategory, Layer};
use crate::model_engine::types::ArcadiaElement;

/// Dimensions fixes pour l'encodage One-Hot
/// OA, SA, LA, PA, EPBS, Data, Transverse, Unknown -> 8 dimensions
const LAYER_DIM: usize = 8;
/// Component, Function, Actor, Exchange, Interface, Data, Capability, Other -> 8 dimensions
const CATEGORY_DIM: usize = 8;

/// NOUVEAU : Dimension pour les métriques continues RAMI 4.0
const RAMI_DIM: usize = 4;

/// Encodeur Hybride (Stateful) : Fusionne la Topologie MBSE (One-Hot) et la Sémantique NLP (Dense).
pub struct HybridEncoder {
    /// Couche de projection pour réduire l'embedding NLP (ex: 384 -> 16)
    pub semantic_proj: NeuralLinearLayer,
    pub structural_dim: usize,
    pub semantic_dim: usize,
}

/// Structure intermédiaire pour typer les propriétés physiques issues du graphe JSON-LD
#[derive(Debug, Default)]
pub struct RamiMetrics {
    pub latency_ms: f32,
    pub throughput_tps: f32,
    pub availability: f32,
    pub security_clearance: f32,
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
            // 🎯 CORRECTION : 8 (Layer) + 8 (Category) + 4 (RAMI) = 20
            structural_dim: LAYER_DIM + CATEGORY_DIM + RAMI_DIM,
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
    /// Dimension de sortie : [1, 20] (8 + 8 + 4)
    pub fn encode_element(
        element: &ArcadiaElement,
        device: &ComputeHardware,
    ) -> RaiseResult<NeuralTensor> {
        // 1. Extraction sémantique
        let layer = element.get_layer();
        let category = element.get_category();

        // 2. Encodage individuel
        let t_layer = Self::encode_layer(layer, device)?;
        let t_cat = Self::encode_category(category, device)?;

        // 🎯 L'ÉTAPE OUBLIÉE : Encodage des métriques RAMI 4.0
        let t_rami = Self::encode_rami_metrics(element, device)?;

        // 3. Triple Concaténation (Feature Fusion)
        let t_combined = match NeuralTensor::cat(&[&t_layer, &t_cat, &t_rami], 1) {
            Ok(t) => t,
            Err(e) => raise_error!(
                "ERR_AI_ENCODER_FUSION_FAILED",
                error = e.to_string(),
                context = json_value!({
                    "layer_shape": format!("{:?}", t_layer.shape()),
                    "category_shape": format!("{:?}", t_cat.shape()),
                    "rami_shape": format!("{:?}", t_rami.shape())
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

    /// Extrait et normalise les métriques RAMI 4.0 depuis les propriétés de l'élément
    pub fn encode_rami_metrics(
        element: &ArcadiaElement,
        device: &ComputeHardware,
    ) -> RaiseResult<NeuralTensor> {
        let mut data = vec![0f32; RAMI_DIM];

        // Helper (closure) pour extraire proprement un f32 depuis un JsonValue (Nombre ou String)
        let extract_f32 = |val: &JsonValue| -> Option<f32> {
            val.as_f64()
                .map(|n| n as f32)
                .or_else(|| val.as_str().and_then(|s| s.parse::<f32>().ok()))
        };

        // 1. Extraction robuste sans le piège des guillemets
        let latency_raw = element
            .properties
            .get("rami:latency")
            .and_then(extract_f32)
            .unwrap_or(0.0);

        let throughput_raw = element
            .properties
            .get("rami:throughput")
            .and_then(extract_f32)
            .unwrap_or(0.0);

        let availability_raw = element
            .properties
            .get("rami:availability")
            .and_then(extract_f32)
            .unwrap_or(1.0); // 100% de disponibilité par défaut

        let clearance_raw = element
            .properties
            .get("rami:clearance")
            .and_then(extract_f32)
            .unwrap_or(0.0); // Niveau de sécurité 0 par défaut

        // 2. Normalisation stricte (Min-Max Scaling vers [0.0, 1.0])
        data[0] = (latency_raw / 1000.0).clamp(0.0, 1.0);
        data[1] = (throughput_raw / 10000.0).clamp(0.0, 1.0);
        data[2] = availability_raw.clamp(0.0, 1.0);
        data[3] = (clearance_raw / 5.0).clamp(0.0, 1.0);

        // 3. Création du tenseur
        match NeuralTensor::from_vec(data, (1, RAMI_DIM), device) {
            Ok(t) => Ok(t),
            Err(e) => raise_error!(
                "ERR_AI_ENCODER_RAMI_FAILED",
                error = e.to_string(),
                context = json_value!({ "action": "create_rami_tensor" })
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_engine::types::ArcadiaElement;

    // Helper pour créer un élément dummy
    fn make_element(kind: &str) -> RaiseResult<ArcadiaElement> {
        Ok(ArcadiaElement {
            handle: "test_id".try_into()?,
            name: I18nString::default(),
            kind: vec![kind.to_string()],
            properties: UnorderedMap::new(),
            ..Default::default()
        })
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
    fn test_encode_full_element() -> RaiseResult<()> {
        let el = make_element("la:LogicalComponent")?;

        let t = ArcadiaEncoder::encode_element(&el, &ComputeHardware::Cpu).unwrap();
        let vec: Vec<f32> = t.to_vec2::<f32>().unwrap()[0].clone();

        // 🎯 CORRECTION : Taille totale attendue : 8 + 8 + 4 = 20
        assert_eq!(vec.len(), LAYER_DIM + CATEGORY_DIM + RAMI_DIM);
        assert_eq!(vec[2], 1.0, "Layer index 2 (LA) doit être 1.0");
        assert_eq!(
            vec[8], 1.0,
            "Category index 0 (Component) décalé de 8 doit être 1.0"
        );
        Ok(())
    }

    #[test]
    fn test_hybrid_encoder_fusion() -> RaiseResult<()> {
        let device = ComputeHardware::Cpu;
        let varmap = NeuralWeightsMap::new();
        let vb = NeuralWeightsBuilder::from_varmap(&varmap, ComputeType::F32, &device);

        let hybrid_encoder = HybridEncoder::new(384, 16, vb).unwrap();
        let el = make_element("la:LogicalFunction")?;
        let mock_nlp_vec = vec![0.5f32; 384];

        let combined_tensor = hybrid_encoder
            .encode_hybrid(&el, &mock_nlp_vec, &device)
            .unwrap();
        let dims = combined_tensor.dims();

        // 🎯 CORRECTION : 20 (Structure) + 16 (Sémantique NLP) = 36
        assert_eq!(
            dims,
            &[1, 36],
            "Le tenseur hybride doit faire 36 dimensions."
        );
        Ok(())
    }

    #[test]
    fn test_encode_rami_metrics_nominal() -> RaiseResult<()> {
        let mut props = UnorderedMap::new();
        // Valeurs standard qui doivent être normalisées correctement
        props.insert("rami:latency".to_string(), json_value!("500.0")); // 500 / 1000 = 0.5
        props.insert("rami:throughput".to_string(), json_value!("2000.0")); // 2000 / 10000 = 0.2
        props.insert("rami:availability".to_string(), json_value!("0.99")); // 0.99
        props.insert("rami:clearance".to_string(), json_value!("2.5")); // 2.5 / 5.0 = 0.5

        let el = ArcadiaElement {
            handle: "handle:test_nominal".try_into()?,
            name: I18nString::default(),
            kind: vec!["la:LogicalComponent".to_string()],
            properties: props,
            ..Default::default()
        };

        let t = ArcadiaEncoder::encode_rami_metrics(&el, &ComputeHardware::Cpu).unwrap();
        let vec: Vec<f32> = t.to_vec2::<f32>().unwrap()[0].clone();

        assert_eq!(vec.len(), RAMI_DIM);
        assert_eq!(vec[0], 0.5, "La latence 500ms doit être normalisée à 0.5");
        assert_eq!(vec[1], 0.2, "Le débit 2000 TPS doit être normalisé à 0.2");
        assert_eq!(vec[2], 0.99, "La disponibilité doit rester à 0.99");
        assert_eq!(vec[3], 0.5, "La clearance 2.5 doit être normalisée à 0.5");
        Ok(())
    }

    #[test]
    fn test_encode_rami_metrics_defaults() -> RaiseResult<()> {
        // Un élément sans aucune propriété RAMI 4.0
        let el = ArcadiaElement {
            handle: "handle:test_defaults".try_into()?,
            name: I18nString::default(),
            kind: vec!["la:LogicalComponent".to_string()],
            properties: UnorderedMap::new(),
            ..Default::default()
        };

        let t = ArcadiaEncoder::encode_rami_metrics(&el, &ComputeHardware::Cpu).unwrap();
        let vec: Vec<f32> = t.to_vec2::<f32>().unwrap()[0].clone();

        // Vérification des replis de sécurité
        assert_eq!(vec[0], 0.0, "Latence par défaut doit être 0.0");
        assert_eq!(vec[1], 0.0, "Débit par défaut doit être 0.0");
        assert_eq!(vec[2], 1.0, "Disponibilité par défaut doit être 1.0 (100%)");
        assert_eq!(vec[3], 0.0, "Clearance par défaut doit être 0.0");
        Ok(())
    }

    #[test]
    fn test_encode_rami_metrics_clamping() -> RaiseResult<()> {
        let mut props = UnorderedMap::new();
        // Valeurs extrêmes et négatives pour tester la robustesse du clamp(0.0, 1.0)
        props.insert("rami:latency".to_string(), json_value!("5000.0")); // Dépasse 1000 -> 1.0
        props.insert("rami:throughput".to_string(), json_value!("-500.0")); // Négatif -> 0.0
        props.insert("rami:availability".to_string(), json_value!("1.5")); // Dépasse 1.0 -> 1.0
        props.insert("rami:clearance".to_string(), json_value!("10.0")); // Dépasse 5.0 -> 1.0

        let el = ArcadiaElement {
            handle: "handle:test_clamping".try_into()?,
            name: I18nString::default(),
            kind: vec!["la:LogicalComponent".to_string()],
            properties: props,
            ..Default::default()
        };

        let t = ArcadiaEncoder::encode_rami_metrics(&el, &ComputeHardware::Cpu).unwrap();
        let vec: Vec<f32> = t.to_vec2::<f32>().unwrap()[0].clone();

        assert_eq!(
            vec[0], 1.0,
            "La latence hors limite doit être plafonnée à 1.0"
        );
        assert_eq!(vec[1], 0.0, "Le débit négatif doit être planché à 0.0");
        assert_eq!(
            vec[2], 1.0,
            "La disponibilité hors limite doit être plafonnée à 1.0"
        );
        assert_eq!(
            vec[3], 1.0,
            "La clearance hors limite doit être plafonnée à 1.0"
        );
        Ok(())
    }

    #[test]
    fn test_encode_rami_metrics_parsing_errors() -> RaiseResult<()> {
        let mut props = UnorderedMap::new();
        // Injection de chaînes invalides depuis le graphe
        props.insert("rami:latency".to_string(), json_value!("not_a_number"));
        props.insert("rami:availability".to_string(), json_value!("null"));

        let el = ArcadiaElement {
            handle: "handle:test_parsing".try_into()?,
            name: I18nString::default(),
            kind: vec!["la:LogicalComponent".to_string()],
            properties: props,
            ..Default::default()
        };

        let t = ArcadiaEncoder::encode_rami_metrics(&el, &ComputeHardware::Cpu).unwrap();
        let vec: Vec<f32> = t.to_vec2::<f32>().unwrap()[0].clone();

        // Le parsing doit échouer silencieusement et appliquer les valeurs par défaut
        assert_eq!(vec[0], 0.0, "Une latence mal formattée doit retourner 0.0");
        assert_eq!(
            vec[2], 1.0,
            "Une disponibilité mal formattée doit retourner 1.0"
        );
        Ok(())
    }
}
