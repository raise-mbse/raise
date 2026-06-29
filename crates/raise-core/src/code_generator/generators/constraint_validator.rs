// FICHIER : crates/raise-core/src/code_generator/generators/constraint_validator.rs

use crate::model_engine::types::ArcadiaElement;
//use crate::raise_error;
//use crate::utils::core::error::RaiseResult;
//use crate::utils::data::json::json_value;
pub struct ConstraintValidatorGenerator;
use crate::utils::prelude::*;

impl ConstraintValidatorGenerator {
    /// Génère un module Rust de validation à partir d'une contrainte réglementaire Arcadia.
    pub fn generate_rust_validator(constraint: &ArcadiaElement) -> RaiseResult<String> {
        // 1. Validation sémantique stricte
        if !constraint.kind.iter().any(|k| k.contains("Constraint")) {
            raise_error!(
                "ERR_CODEGEN_INVALID_KIND",
                error = "L'élément source doit être une Contrainte (Constraint) pour générer un validateur.",
                context = json_value!({"kind": constraint.kind})
            );
        }

        // 2. Extraction sécurisée des seuils réglementaires
        let max_n = constraint
            .properties
            .get("max_n_limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(170);
        let max_cu = constraint
            .properties
            .get("max_cu_mg_kg")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let max_zn = constraint
            .properties
            .get("max_zn_mg_kg")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let constraint_name = constraint.name.as_str();

        // 3. Matérialisation : Génération du code Rust "Zéro Dette"
        let generated_code = format!(
            r#"// ⚠️ FICHIER GÉNÉRÉ AUTOMATIQUEMENT PAR R.A.I.S.E.
// ⚖️ SOURCE LÉGALE : {constraint_name}
// 🆔 RÉFÉRENCE MBSE : {id}

use crate::utils::core::error::RaiseResult;
use crate::raise_error;
use crate::utils::data::json::json_value;

#[derive(Debug, Clone)]
pub struct FertilizerSample {{
    pub n_content_kg: u64,
    pub cu_content_mg: u64,
    pub zn_content_mg: u64,
}}

/// Vérifie la conformité d'un échantillon face aux exigences réglementaires.
pub fn validate_renure_compliance(sample: &FertilizerSample) -> RaiseResult<()> {{
    if sample.n_content_kg > {max_n} {{
        return Err(raise_error!(
            "ERR_RENURE_LIMIT_EXCEEDED",
            error = "Limite d'azote dépassée selon la dérogation.",
            context = json_value!({{"limit_kg": {max_n}, "actual_kg": sample.n_content_kg}})
        ));
    }}
    
    if sample.cu_content_mg > {max_cu} {{
        return Err(raise_error!(
            "ERR_RENURE_CU_EXCEEDED",
            error = "Limite de Cuivre (Cu) dépassée.",
            context = json_value!({{"limit_mg": {max_cu}, "actual_mg": sample.cu_content_mg}})
        ));
    }}
    
    if sample.zn_content_mg > {max_zn} {{
        return Err(raise_error!(
            "ERR_RENURE_ZN_EXCEEDED",
            error = "Limite de Zinc (Zn) dépassée.",
            context = json_value!({{"limit_mg": {max_zn}, "actual_mg": sample.zn_content_mg}})
        ));
    }}

    Ok(())
}}
"#,
            constraint_name = constraint_name,
            id = constraint.handle.as_str(),
            max_n = max_n,
            max_cu = max_cu,
            max_zn = max_zn
        );

        Ok(generated_code)
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::data::UnorderedMap;

    #[test]
    fn test_generate_rust_validator_from_constraint() -> RaiseResult<()> {
        let mut props = UnorderedMap::new();
        props.insert("max_n_limit".to_string(), json_value!(250));
        props.insert("max_cu_mg_kg".to_string(), json_value!(300));
        props.insert("max_zn_mg_kg".to_string(), json_value!(800));

        let mock_constraint = ArcadiaElement {
            handle: "CSTR_RENURE_LIMITS".try_into()?,
            name: I18nString::Single("Seuils de Tolérance RENURE".to_string()),
            kind: vec!["transverse:Constraint".to_string()],
            properties: props,
            ..Default::default()
        };

        let source_code = ConstraintValidatorGenerator::generate_rust_validator(&mock_constraint)?;

        // Validation de la matérialisation
        assert!(source_code.contains("pub fn validate_renure_compliance"));
        assert!(source_code.contains("sample.n_content_kg > 250"));
        assert!(source_code.contains("sample.cu_content_mg > 300"));
        assert!(source_code.contains("sample.zn_content_mg > 800"));
        assert!(source_code.contains("CSTR_RENURE_LIMITS"));

        Ok(())
    }
}
