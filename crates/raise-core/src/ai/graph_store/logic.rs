// FICHIER : src-tauri/src/ai/graph_store/logic.rs
use crate::ai::protocols::ontology::OntologyRuleEngine;
use crate::utils::prelude::*;

pub struct ArcadiaLogic {
    /// 🎯 FORMAT SPARSE (COO) : Remplace la matrice dense [N, N]
    ///
    /// Au lieu d'allouer N×N flottants (400 Mo pour N=10 000),
    /// on ne stocke que les paires (i, j) qui violent une règle Arcadia.
    /// La grande majorité des paires est légale → densité de violation ≈ 0.
    ///
    /// `violation_src[k]` et `violation_dst[k]` forment la k-ième paire interdite.
    pub violation_src: NeuralTensor, // [V] indices sources des violations (u32)
    pub violation_dst: NeuralTensor, // [V] indices cibles des violations (u32)
    pub violation_count: usize,      // Nombre de paires interdites détectées
}

impl ArcadiaLogic {
    /// 🎯 CONSTRUCTION SPARSE DE LA LISTE DE VIOLATIONS
    ///
    /// Complexité mémoire : O(V) avec V << N²
    /// (V = nombre de paires violant le Cycle en V Arcadia)
    pub fn build_violation_matrix(
        index_to_uri: &[String],
        device: &ComputeHardware,
    ) -> RaiseResult<Self> {
        let n = index_to_uri.len();
        if n == 0 {
            raise_error!("ERR_GNN_LOGIC_EMPTY", error = "Index d'URIs vide.");
        }

        // 🎯 Pré-extraction des types pour éviter de re-splitter N² fois
        let types: Vec<&str> = index_to_uri
            .iter()
            .map(|uri| Self::extract_type(uri))
            .collect();

        let mut src_violations: Vec<u32> = Vec::new();
        let mut dst_violations: Vec<u32> = Vec::new();

        // Scan sparse : on ne visite que les paires (type_src, type_dst) interdites.
        for i in 0..n {
            for j in 0..n {
                // 🎯 FIX ZÉRO DETTE : Délégation de la validation à l'OntologyRuleEngine
                if OntologyRuleEngine::is_violation(types[i], types[j]) {
                    src_violations.push(i as u32);
                    dst_violations.push(j as u32);
                }
            }
        }

        let violation_count = src_violations.len();

        user_info!(
            "MSG_GNN_LOGIC_SPARSE",
            json_value!({
                "nodes": n,
                "violations": violation_count,
                "density_percent": format!("{:.4}", violation_count as f32 / (n * n) as f32 * 100.0)
            })
        );

        // Cas dégénéré : aucune violation possible dans ce graphe
        // (ex: graphe purement PA ou OA — on retourne des tenseurs vides valides)
        if violation_count == 0 {
            let empty_src = match NeuralTensor::new(Vec::<u32>::new(), device) {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_GNN_LOGIC_EMPTY_TENSOR", error = e.to_string()),
            };
            let empty_dst = match NeuralTensor::new(Vec::<u32>::new(), device) {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_GNN_LOGIC_EMPTY_TENSOR", error = e.to_string()),
            };
            return Ok(Self {
                violation_src: empty_src,
                violation_dst: empty_dst,
                violation_count: 0,
            });
        }

        // Construction directe : les Vec<u32> sont déjà prêts en mémoire CPU,
        // pas d'inférence ni de blocage — pas besoin d'execute_native_inference ici.
        let violation_src = match NeuralTensor::new(src_violations, device) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_GNN_LOGIC_TENSOR_SRC", error = e.to_string()),
        };
        let violation_dst = match NeuralTensor::new(dst_violations, device) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_GNN_LOGIC_TENSOR_DST", error = e.to_string()),
        };

        Ok(Self {
            violation_src,
            violation_dst,
            violation_count,
        })
    }

    /// 🎯 CALCUL DE LA PERTE LOGIQUE (VERSION SPARSE)
    ///
    /// Au lieu de `predictions * violation_matrix` (produit N×N),
    /// on extrait uniquement les scores aux positions interdites via index_select,
    /// puis on somme. Complexité : O(V) avec V << N².
    ///
    /// `predictions` : tenseur [N, N] des scores de lien prédits par le GNN.
    pub fn compute_logic_loss(
        &self,
        predictions: &NeuralTensor,
        lambda: f32,
    ) -> RaiseResult<NeuralTensor> {
        // Cas court-circuit : aucune violation possible dans ce graphe
        if self.violation_count == 0 {
            let device = predictions.device();
            let zero = match NeuralTensor::new(&[0.0f32], device) {
                Ok(t) => match t.reshape(&[]) {
                    Ok(r) => r,
                    Err(e) => raise_error!("ERR_GNN_LOGIC_ZERO_RESHAPE", error = e.to_string()),
                },
                Err(e) => raise_error!("ERR_GNN_LOGIC_ZERO_ALLOC", error = e.to_string()),
            };
            return Ok(zero);
        }

        let n = predictions.dims()[0];

        // 1. Aplatissement de [N, N] → [N²] pour pouvoir faire un index_select 1D
        let flat_predictions = match predictions.reshape(&[n * n]) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_GNN_LOGIC_RESHAPE_FLAT", error = e.to_string()),
        };

        // 2. Conversion des paires (i, j) → indices 1D dans la matrice aplatie
        //    index = i * N + j
        //    Calcul en Rust pur : mul_scalar n'existe pas dans candle 0.10.2.
        let flat_idx_vec: Vec<u32> = {
            let src_vec = match self.violation_src.to_vec1::<u32>() {
                Ok(v) => v,
                Err(e) => raise_error!("ERR_GNN_LOGIC_IDX_READ_SRC", error = e.to_string()),
            };
            let dst_vec = match self.violation_dst.to_vec1::<u32>() {
                Ok(v) => v,
                Err(e) => raise_error!("ERR_GNN_LOGIC_IDX_READ_DST", error = e.to_string()),
            };
            src_vec
                .into_iter()
                .zip(dst_vec)
                .map(|(i, j)| i * (n as u32) + j)
                .collect()
        };

        let flat_indices = match NeuralTensor::new(flat_idx_vec, predictions.device()) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_GNN_LOGIC_INDEX_CALC", error = e.to_string()),
        };

        // 3. Extraction sparse : on récupère uniquement les V scores interdits [V]
        let forbidden_scores = match flat_predictions.index_select(&flat_indices, 0) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_GNN_LOGIC_INDEX_SELECT", error = e.to_string()),
        };

        // 4. Somme des activations interdites → scalaire
        let total_violation = match forbidden_scores.sum_all() {
            Ok(s) => s,
            Err(e) => raise_error!("ERR_GNN_LOGIC_SUM_FAILED", error = e.to_string()),
        };

        // 5. Pondération par lambda
        let device = total_violation.device();
        let lambda_tensor = match NeuralTensor::new(&[lambda], device) {
            Ok(t) => match t.reshape(&[]) {
                Ok(r) => r,
                Err(e) => raise_error!("ERR_GNN_LOGIC_RESHAPE", error = e.to_string()),
            },
            Err(e) => raise_error!("ERR_GNN_LOGIC_LAMBDA_ALLOC", error = e.to_string()),
        };

        match total_violation.mul(&lambda_tensor) {
            Ok(l) => Ok(l),
            Err(e) => raise_error!(
                "ERR_GNN_LOGIC_SCALE_FAILED",
                error = e.to_string(),
                context = json_value!({ "action": "COMPUTE_LOGIC_LOSS", "violations": self.violation_count })
            ),
        }
    }

    fn extract_type(uri: &str) -> &str {
        uri.split(':').next().unwrap_or("unknown")
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    /// Vérifie que la perte est identique à l'ancienne implémentation dense.
    /// Scénario : 2 nœuds (la:C1, pa:P1) → 1 violation (la→pa)
    /// Score prédit sur ce lien : 0.9 → perte attendue = 0.9 * 100.0 = 90.0
    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_logic_loss_calcul_sparse_equivalent_dense() -> RaiseResult<()> {
        let device = ComputeHardware::Cpu;
        let uris = vec!["la:C1".to_string(), "pa:P1".to_string()];
        let logic = ArcadiaLogic::build_violation_matrix(&uris, &device)?;

        // 1 violation détectée : (la → pa) = index (0, 1)
        assert_eq!(logic.violation_count, 1, "Une seule violation attendue.");

        // Matrice de prédiction [2, 2] :
        // [0.0, 0.9]  ← score sur (la→pa) = 0.9  (lien interdit)
        // [0.0, 0.0]
        let pred_raw = vec![0.0f32, 0.9, 0.0, 0.0];
        let predictions = match NeuralTensor::from_vec(pred_raw, (2, 2), &device) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_TEST_TENSOR", error = e.to_string()),
        };

        let loss_tensor = logic.compute_logic_loss(&predictions, 100.0)?;

        let loss_value = match loss_tensor.to_scalar::<f32>() {
            Ok(v) => v,
            Err(e) => raise_error!("ERR_TEST_SCALAR", error = e.to_string()),
        };

        assert!(
            (loss_value - 90.0).abs() < 0.001,
            "Perte attendue : 90.0, obtenue : {}",
            loss_value
        );
        Ok(())
    }

    /// Vérifie que la perte est nulle quand aucune violation n'est possible.
    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_logic_loss_zero_sur_graphe_sans_violation() -> RaiseResult<()> {
        let device = ComputeHardware::Cpu;
        // Graphe purement OA : aucune paire interdite entre nœuds OA
        let uris = vec!["oa:A1".to_string(), "oa:A2".to_string()];
        let logic = ArcadiaLogic::build_violation_matrix(&uris, &device)?;

        assert_eq!(logic.violation_count, 0);

        let predictions = match NeuralTensor::from_vec(vec![0.9f32, 0.8, 0.7, 0.6], (2, 2), &device)
        {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_TEST_TENSOR", error = e.to_string()),
        };

        let loss_tensor = logic.compute_logic_loss(&predictions, 100.0)?;
        let loss_value = match loss_tensor.to_scalar::<f32>() {
            Ok(v) => v,
            Err(e) => raise_error!("ERR_TEST_SCALAR", error = e.to_string()),
        };

        assert!(
            (loss_value - 0.0).abs() < 0.001,
            "Perte attendue : 0.0 sur graphe OA pur."
        );
        Ok(())
    }

    /// Vérifie la règle manquante (sa → pa).
    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_logic_regle_sa_pa_detectee() -> RaiseResult<()> {
        let device = ComputeHardware::Cpu;
        let uris = vec!["sa:S1".to_string(), "pa:P1".to_string()];
        let logic = ArcadiaLogic::build_violation_matrix(&uris, &device)?;

        assert_eq!(
            logic.violation_count, 1,
            "La violation (sa→pa) doit être détectée."
        );
        Ok(())
    }

    /// Vérifie le comportement sur un graphe vide.
    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_logic_erreur_sur_graphe_vide() -> RaiseResult<()> {
        let device = ComputeHardware::Cpu;
        let result = ArcadiaLogic::build_violation_matrix(&[], &device);

        match result {
            Err(AppError::Structured(err)) => assert_eq!(err.code, "ERR_GNN_LOGIC_EMPTY"),
            _ => panic!("Devrait lever ERR_GNN_LOGIC_EMPTY sur graphe vide."),
        }
        Ok(())
    }
}
