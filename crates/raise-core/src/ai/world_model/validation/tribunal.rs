// FICHIER : crates/raise-core/src/ai/world_model/validation/tribunal.rs

use crate::model_engine::types::ArcadiaElement;
use crate::utils::prelude::*; // Façade RAISE

/// Le Juge Symbolique Inflexible (Tribunal de l'AST)
/// Garde du corps déterministe du World Model.
pub struct AstTribunal;

impl AstTribunal {
    /// -----------------------------------------------------------------------
    /// BARRIÈRE 1 : Le Court-Circuit Synchrone ("Fail-Fast")
    /// Évalue statiquement l'élément AVANT de solliciter le World Model.
    /// Préserve la charge CPU/GPU en rejetant les aberrations topologiques.
    /// -----------------------------------------------------------------------
    pub fn execute_pre_clearance(element: &ArcadiaElement) -> RaiseResult<()> {
        let name = element.name.as_str();

        // 1. Extraction robuste des attributs physiques (RAMI 4.0) et de sécurité
        let extract_f32 = |val: &JsonValue| -> f32 {
            val.as_f64()
                .map(|n| n as f32)
                .or_else(|| val.as_str().and_then(|s| s.parse::<f32>().ok()))
                .unwrap_or(0.0)
        };

        let clearance = element
            .properties
            .get("rami:clearance")
            .map_or(0.0, extract_f32);

        let is_public = element
            .properties
            .get("net:public_facing")
            .and_then(|v| {
                v.as_bool()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<bool>().ok()))
            })
            .unwrap_or(false);

        // RÈGLE AST 1 : Sanctuarisation Absolue
        // Rejet immédiat si un composant critique (clearance >= 3.0) est exposé publiquement.
        if clearance >= 3.0 && is_public {
            raise_error!(
                "ERR_AST_SECURITY_VIOLATION",
                error = "Verdict AST : Violation de la sanctuarisation. Composant critique exposé.",
                context =
                    json_value!({ "handle": element.handle, "name": name, "clearance": clearance })
            );
        }

        // RÈGLE AST 2 : Cohérence Physique Mathématique
        // Un composant matériel ou logique ne peut pas avoir une masse ou une latence négative.
        let latency = element
            .properties
            .get("rami:latency")
            .map_or(0.0, extract_f32);
        if latency < 0.0 {
            raise_error!(
                "ERR_AST_PHYSICS_VIOLATION",
                error = "Verdict AST : Aberration physique. La latence ne peut être négative.",
                context = json_value!({ "handle": element.handle, "latency": latency })
            );
        }

        Ok(())
    }

    /// -----------------------------------------------------------------------
    /// BARRIÈRE 2 : Le Rejet Prédictif Impitoyable
    /// Évalue le tenseur d'état futur (t+1) généré par le World Model.
    /// -----------------------------------------------------------------------
    pub fn execute_post_verdict(predicted_state: &NeuralTensor) -> RaiseResult<()> {
        // Extraction des valeurs scalaires du tenseur (Simulation)
        let vec = match predicted_state.to_vec2::<f32>() {
            Ok(v) => v,
            Err(e) => raise_error!("ERR_AST_TENSOR_READ_FAILED", error = e.to_string()),
        };

        // En se basant sur notre encodeur, les métriques RAMI sont aux index 16 à 19
        // 16: Latency (0.0 à 1.0), 17: Throughput, 18: Availability, 19: Clearance
        if vec[0].len() < 20 {
            raise_error!(
                "ERR_AST_TENSOR_DIMENSION",
                error = "Tenseur d'état incompatible."
            );
        }

        let predicted_latency_ratio = vec[0][16];
        let predicted_availability = vec[0][18];

        // RÈGLE SYSTÉMIQUE : Saturation et Goulot d'étranglement
        // Rejet si le modèle prédit une saturation de la latence à plus de 90% de la tolérance
        if predicted_latency_ratio > 0.90 {
            raise_error!(
                "ERR_AST_BOTTLENECK_PREDICTED",
                error = "Verdict AST : Rejet Prédictif. Risque critique de goulot d'étranglement sous charge.",
                context = json_value!({ "predicted_latency_ratio": predicted_latency_ratio })
            );
        }

        // RÈGLE SYSTÉMIQUE : Effondrement de la disponibilité
        if predicted_availability < 0.95 {
            raise_error!(
                "ERR_AST_DOWNTIME_PREDICTED",
                error = "Verdict AST : Rejet Prédictif. Effondrement de la disponibilité anticipé.",
                context = json_value!({ "predicted_availability": predicted_availability })
            );
        }

        Ok(())
    }
}

// =========================================================================
// TESTS UNITAIRES DU TRIBUNAL
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    fn mock_element(clearance: f32, is_public: bool, latency: f32) -> RaiseResult<ArcadiaElement> {
        let mut props = UnorderedMap::new();
        props.insert("rami:clearance".into(), json_value!(clearance));
        props.insert("net:public_facing".into(), json_value!(is_public));
        props.insert("rami:latency".into(), json_value!(latency));

        Ok(ArcadiaElement {
            handle: "handle:test".try_into()?,
            name: I18nString::Single("TestComponent".into()),
            kind: vec!["la:LogicalComponent".into()],
            properties: props,
            ..Default::default()
        })
    }

    #[test]
    fn test_pre_clearance_fail_fast_security() -> RaiseResult<()> {
        // Un composant critique (4.0) exposé publiquement (true)
        let element = mock_element(4.0, true, 50.0)?;
        let result = AstTribunal::execute_pre_clearance(&element);

        assert!(result.is_err());
        if let Err(AppError::Structured(e)) = result {
            assert_eq!(e.code, "ERR_AST_SECURITY_VIOLATION");
        } else {
            panic!("Mauvais type d'erreur retourné");
        }
        Ok(())
    }

    #[test]
    fn test_pre_clearance_fail_fast_physics() -> RaiseResult<()> {
        // Une latence négative (aberration)
        let element = mock_element(1.0, false, -10.0)?;
        let result = AstTribunal::execute_pre_clearance(&element);

        assert!(result.is_err());
        if let Err(AppError::Structured(e)) = result {
            assert_eq!(e.code, "ERR_AST_PHYSICS_VIOLATION");
        } else {
            panic!("Mauvais type d'erreur retourné");
        }
        Ok(())
    }

    #[test]
    fn test_pre_clearance_success() -> RaiseResult<()> {
        // Composant standard, légal
        let element = mock_element(2.0, false, 50.0)?;
        let result = AstTribunal::execute_pre_clearance(&element);
        assert!(result.is_ok(), "Le tribunal aurait dû valider cet élément");
        Ok(())
    }

    #[test]
    fn test_post_verdict_bottleneck_prediction() -> RaiseResult<()> {
        // Tenseur fictif de dimension 20
        let mut data = vec![0.0f32; 20];
        data[16] = 0.95; // Latency ratio = 95% (Dépasse le seuil de 0.90)
        data[18] = 0.99; // Availability OK

        // Simulation d'une exécution CPU
        let tensor = NeuralTensor::from_vec(data, (1, 20), &ComputeHardware::Cpu).unwrap();

        let result = AstTribunal::execute_post_verdict(&tensor);
        assert!(result.is_err());
        if let Err(AppError::Structured(e)) = result {
            assert_eq!(e.code, "ERR_AST_BOTTLENECK_PREDICTED");
        } else {
            panic!("Mauvais type d'erreur retourné");
        }
        Ok(())
    }

    #[test]
    fn test_post_verdict_success() -> RaiseResult<()> {
        // Tenseur fictif de dimension 20
        let mut data = vec![0.0f32; 20];
        data[16] = 0.50; // Latency ratio = 50% (Sain)
        data[18] = 0.99; // Availability = 99% (Sain)

        let tensor = NeuralTensor::from_vec(data, (1, 20), &ComputeHardware::Cpu).unwrap();

        let result = AstTribunal::execute_post_verdict(&tensor);
        assert!(
            result.is_ok(),
            "Le verdict prédictif aurait dû être favorable"
        );
        Ok(())
    }
}
