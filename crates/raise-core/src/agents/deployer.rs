// FICHIER : crates/raise-core/src/agents/deployer.rs

use crate::blockchain::storage::commit::{MentisCommit, MutationOp};
// 🎯 ALIGNEMENT STRICT : 100% des outils (fs, ProcessCommand, UnixFilePermissions, etc.) viennent d'ici.
use crate::utils::prelude::*;

pub struct EdgeDeployer;

impl EdgeDeployer {
    /// Méthode appelée par le moteur P2P chaque fois qu'un nouveau bloc Mentis est validé par le quorum.
    pub async fn process_new_commit(
        commit: &MentisCommit,
        _local_node_id: &str,
    ) -> RaiseResult<()> {
        for mutation in &commit.mutations {
            if mutation.operation == MutationOp::Delete {
                continue;
            }

            let payload = &mutation.payload;

            let types = payload["@type"].as_array();
            let is_binary =
                types.is_some_and(|t| t.iter().any(|v| v.as_str() == Some("raise:BinaryElement")));

            if !is_binary {
                continue;
            }

            let arch = payload["target_architecture"].as_str().unwrap_or("");
            if arch != "aarch64-unknown-linux-gnu" {
                user_debug!(
                    "DEPLOYER",
                    json_value!({"msg": "Ignoré : Architecture non compatible", "arch": arch})
                );
                continue;
            }

            if let Some(ctx) = payload.get("execution_context") {
                let target_path = ctx["deploy_path"].as_str().unwrap_or("");
                let requires_chmod = ctx["requires_chmod_x"].as_bool().unwrap_or(false);

                if target_path.is_empty() {
                    continue;
                }

                user_info!(
                    "DEPLOYER",
                    json_value!({"msg": "Artefact validé reçu. Début du déploiement.", "path": target_path})
                );

                if let Some(storage) = payload.get("storage") {
                    if storage["encoding"].as_str() == Some("base64") {
                        let b64_payload = storage["payload_or_uri"].as_str().unwrap_or("");

                        match decode_base64(b64_payload) {
                            Ok(binary_data) => {
                                // 1. Écriture asynchrone sur le disque
                                if let Err(e) = fs::write_async(target_path, &binary_data).await {
                                    user_error!(
                                        "ERR_EDGE_WRITE",
                                        json_value!({"error": e.to_string(), "path": target_path})
                                    );
                                    continue; // On passe au contrat suivant, sans crasher l'agent
                                }

                                // 2. Application asynchrone des droits d'exécution Unix
                                if requires_chmod {
                                    // Utilisation d'un match pour ne pas utiliser `?` qui ferait un return Err() caché
                                    match fs::get_permissions_async(target_path).await {
                                        Ok(mut perms) => {
                                            perms.set_mode(0o755); // Nécessite UnixFilePermissions
                                            if let Err(e) =
                                                fs::set_permissions_async(target_path, perms).await
                                            {
                                                user_warn!(
                                                    "ERR_EDGE_CHMOD",
                                                    json_value!({"error": e.to_string(), "path": target_path})
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            user_warn!(
                                                "ERR_EDGE_CHMOD_READ",
                                                json_value!({"error": e.to_string(), "path": target_path})
                                            );
                                        }
                                    }
                                }

                                user_success!(
                                    "INF_EDGE_DEPLOYED",
                                    json_value!({"path": target_path, "size": binary_data.len()})
                                );

                                // Exécution automatique si c'est le binaire edge
                                if target_path.ends_with("raise-edge") {
                                    Self::restart_service(target_path).await;
                                }
                            }
                            Err(e) => {
                                // 🎯 FIX : On loggue l'erreur dans l'audit, et on passe à la suite au lieu d'avorter !
                                user_error!(
                                    "ERR_EDGE_DECODE",
                                    json_value!({"error": e.to_string()})
                                );
                                continue;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Redémarre le binaire fraîchement déployé
    async fn restart_service(path: &str) {
        user_info!("DEPLOYER", json_value!({"msg": "Lancement du service..."}));
        // 🎯 ALIGNEMENT : Utilisation de ProcessCommand au lieu de std::process::Command
        let _ = ProcessCommand::new("nohup").arg(path).arg("&").spawn();
    }
}

// =========================================================================
// TESTS UNITAIRES DE SÉCURITÉ ET D'INTÉGRITÉ
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blockchain::crypto::signing::KeyPair;
    use crate::blockchain::storage::commit::Mutation;

    /// Helper : Construit un MentisCommit fictif signé
    fn create_mock_commit(mutations: Vec<Mutation>) -> MentisCommit {
        let keys = KeyPair::generate();
        MentisCommit::new(mutations, None, &keys)
    }

    /// Helper : Crée un chemin temporaire unique et sûr
    fn get_temp_path(filename: &str) -> String {
        // 🎯 ALIGNEMENT : Utilisation de os_temp_dir() au lieu de std::env::temp_dir()
        let mut path = os_temp_dir();
        path.push(format!("{}_{}", "raise_test", filename));
        path.to_string_lossy().to_string()
    }

    /// TEST 1 : Flux nominal (Succès du déploiement)
    #[async_test]
    #[serial_test::serial]
    async fn test_deployer_success_workflow() {
        let test_path = get_temp_path("success_bin");
        // 🎯 ALIGNEMENT : Façade RAISE (remove_file_sync avec conversion en Path)
        let _ = fs::remove_file_sync(Path::new(&test_path));

        let base64_data = "Q09OVEVOVV9CSU5BSVJF";

        let payload = json_value!({
            "@type": ["raise:BinaryElement", "pa:PhysicalArtifact"],
            "target_architecture": "aarch64-unknown-linux-gnu",
            "storage": {
                "encoding": "base64",
                "payload_or_uri": base64_data
            },
            "execution_context": {
                "deploy_path": test_path.clone(),
                "requires_chmod_x": true
            }
        });

        let commit = create_mock_commit(vec![Mutation {
            element_id: "urn:test:artifact:1".into(),
            operation: MutationOp::Create,
            payload,
        }]);

        let result = EdgeDeployer::process_new_commit(&commit, "node_condorcet_pi").await;
        assert!(
            result.is_ok(),
            "Le déploiement ne doit remonter aucune erreur fatale."
        );

        // 🎯 ALIGNEMENT : Utilisation de metadata_sync
        assert!(
            fs::metadata_sync(&test_path).is_ok(),
            "Le fichier doit avoir été écrit sur le disque."
        );

        // 🎯 ALIGNEMENT : Utilisation de read_to_string_sync
        let content = fs::read_to_string_sync(Path::new(&test_path)).unwrap();
        assert_eq!(
            content, "CONTENU_BINAIRE",
            "Le base64 doit être décodé correctement."
        );

        // 🎯 ALIGNEMENT : Utilisation de get_permissions_sync au lieu de metadata().unwrap().permissions()
        let perms = fs::get_permissions_sync(&test_path).unwrap();
        assert_eq!(
            perms.mode() & 0o777,
            0o755,
            "Les droits d'exécution doivent avoir été appliqués."
        );

        let _ = fs::remove_file_sync(Path::new(&test_path));
    }

    /// TEST 2 : Rejet d'une architecture non compatible (ex: Workstation x86_64)
    #[async_test]
    #[serial_test::serial]
    async fn test_deployer_ignores_wrong_architecture() {
        let test_path = get_temp_path("wrong_arch_bin");
        let _ = fs::remove_file_sync(Path::new(&test_path));

        let payload = json_value!({
            "@type": ["raise:BinaryElement"],
            "target_architecture": "x86_64-unknown-linux-gnu",
            "storage": {
                "encoding": "base64",
                "payload_or_uri": "Q09OVEVOVV9CSU5BSVJF"
            },
            "execution_context": {
                "deploy_path": test_path.clone(),
                "requires_chmod_x": true
            }
        });

        let commit = create_mock_commit(vec![Mutation {
            element_id: "urn:test:artifact:2".into(),
            operation: MutationOp::Create,
            payload,
        }]);

        let result = EdgeDeployer::process_new_commit(&commit, "node_condorcet_pi").await;
        assert!(result.is_ok());

        assert!(
            fs::metadata_sync(&test_path).is_err(),
            "L'agent doit ignorer les binaires qui ne sont pas compilés pour aarch64."
        );
    }

    /// TEST 3 : Ignorer les données non-binaires (ex: Documentation ou Paramètres)
    #[async_test]
    #[serial_test::serial]
    async fn test_deployer_ignores_non_binary_elements() {
        let test_path = get_temp_path("doc_bin");
        let _ = fs::remove_file_sync(Path::new(&test_path));

        let payload = json_value!({
            "@type": ["raise:DocElement"],
            "target_architecture": "aarch64-unknown-linux-gnu",
            "storage": {
                "encoding": "base64",
                "payload_or_uri": "Q09OVEVOVV9CSU5BSVJF"
            },
            "execution_context": {
                "deploy_path": test_path.clone()
            }
        });

        let commit = create_mock_commit(vec![Mutation {
            element_id: "urn:test:artifact:3".into(),
            operation: MutationOp::Create,
            payload,
        }]);

        let result = EdgeDeployer::process_new_commit(&commit, "node_condorcet_pi").await;
        assert!(result.is_ok());

        assert!(
            fs::metadata_sync(&test_path).is_err(),
            "L'agent ne doit écrire QUE les éléments de type raise:BinaryElement."
        );
    }

    /// TEST 4 : Base64 corrompu (Ne doit pas crasher l'agent)
    #[async_test]
    #[serial_test::serial]
    async fn test_deployer_handles_invalid_base64_gracefully() {
        let test_path = get_temp_path("corrupted_bin");
        let _ = fs::remove_file_sync(Path::new(&test_path));

        let payload = json_value!({
            "@type": ["raise:BinaryElement"],
            "target_architecture": "aarch64-unknown-linux-gnu",
            "storage": {
                "encoding": "base64",
                "payload_or_uri": "!!! INVALID BASE 64 !!!"
            },
            "execution_context": {
                "deploy_path": test_path.clone()
            }
        });

        let commit = create_mock_commit(vec![Mutation {
            element_id: "urn:test:artifact:4".into(),
            operation: MutationOp::Create,
            payload,
        }]);

        let result = EdgeDeployer::process_new_commit(&commit, "node_condorcet_pi").await;
        assert!(result.is_ok());

        assert!(
            fs::metadata_sync(&test_path).is_err(),
            "Aucun fichier ne doit être créé si le décodage échoue."
        );
    }
}
