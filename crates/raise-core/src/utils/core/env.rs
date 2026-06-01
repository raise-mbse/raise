// FICHIER : crates/raise-core/src/utils/core/env.rs

use crate::utils::core::RuntimeEnv;
use crate::utils::io::fs::{self, Path, PathBuf};

/// Résout dynamiquement les chemins relatifs (~/, ./, ../) pour garantir
/// un ancrage absolu et déterministe avant leur injection dans l'OS.
fn resolve_path_value(value: &str) -> String {
    // 1. ANCRAGE UTILISATEUR (Tilde) -> /home/user/...
    if value.starts_with("~/") {
        match RuntimeEnv::var("HOME") {
            Ok(home) => {
                let clean_path = value.trim_start_matches("~/");
                return PathBuf::from(home)
                    .join(clean_path)
                    .to_string_lossy()
                    .to_string();
            }
            Err(_) => return value.to_string(), // Fallback silencieux si l'OS n'a pas de HOME
        }
    }
    // 2. ANCRAGE BINAIRE -> Résolu par rapport à l'emplacement physique de l'exécutable
    else if value.starts_with("./") || value.starts_with("../") {
        match RuntimeEnv::current_exe() {
            Ok(mut exe_path) => {
                exe_path.pop(); // Retire le nom du binaire pour obtenir son répertoire
                return exe_path.join(value).to_string_lossy().to_string();
            }
            Err(_) => return value.to_string(), // Fallback silencieux
        }
    }

    // Si ce n'est pas un chemin relatif reconnu, on le retourne tel quel
    value.to_string()
}

/// Charge manuellement un fichier .env dans l'environnement du processus.
/// Ne nécessite aucune dépendance externe.
pub fn load_local_env(env_path: &Path) {
    // Si le fichier n'existe pas, on sort silencieusement.
    let content = match fs::read_to_string_sync(env_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    for line in content.lines() {
        let line = line.trim();

        // On ignore les lignes vides et les commentaires
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // On sépare la clé de la valeur au premier '='
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let mut raw_value = value.trim();

            // Nettoyage : On retire les guillemets éventuels autour de la valeur
            if (raw_value.starts_with('"') && raw_value.ends_with('"'))
                || (raw_value.starts_with('\'') && raw_value.ends_with('\''))
            {
                raw_value = &raw_value[1..raw_value.len() - 1];
            }

            // 🎯 Interception et résolution sémantique des chemins
            let resolved_value = resolve_path_value(raw_value);

            // On injecte dans l'OS SEULEMENT si la variable n'est pas déjà définie
            if RuntimeEnv::var(key).is_err() {
                RuntimeEnv::set_var(key, &resolved_value);
            }
        }
    }
}

// ==============================================================================
// 🧪 TESTS UNITAIRES (Validation du Parseur d'Environnement)
// ==============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::io::fs::tempdir;

    #[test]
    #[serial_test::serial]
    fn test_load_local_env_parsing() {
        let dir = tempdir().expect("Impossible de créer le tempdir");
        let env_path = dir.path().join(".env");

        let env_content = "
# Ceci est un commentaire
VAR_SIMPLE=123
VAR_DOUBLE_QUOTES=\"valeur_double\"
VAR_SINGLE_QUOTES='valeur_simple'
        ";

        fs::write_sync(&env_path, env_content.as_bytes()).unwrap();

        RuntimeEnv::remove_var("VAR_SIMPLE");
        RuntimeEnv::remove_var("VAR_DOUBLE_QUOTES");
        RuntimeEnv::remove_var("VAR_SINGLE_QUOTES");

        load_local_env(&env_path);

        assert_eq!(
            RuntimeEnv::var("VAR_SIMPLE").unwrap(),
            "123",
            "Erreur sur la valeur simple"
        );
        assert_eq!(
            RuntimeEnv::var("VAR_DOUBLE_QUOTES").unwrap(),
            "valeur_double",
            "Erreur de suppression des guillemets doubles"
        );
        assert_eq!(
            RuntimeEnv::var("VAR_SINGLE_QUOTES").unwrap(),
            "valeur_simple",
            "Erreur de suppression des guillemets simples"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_load_local_env_no_overwrite() {
        let dir = tempdir().expect("Impossible de créer le tempdir");
        let env_path = dir.path().join(".env");

        let env_content = "VAR_EXISTANTE=nouvelle_valeur\n";
        fs::write_sync(&env_path, env_content.as_bytes()).unwrap();

        RuntimeEnv::set_var("VAR_EXISTANTE", "ancienne_valeur_intouchable");

        load_local_env(&env_path);

        assert_eq!(
            RuntimeEnv::var("VAR_EXISTANTE").unwrap(),
            "ancienne_valeur_intouchable",
            "Le fichier .env a écrasé une variable système existante !"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_load_local_env_missing_file_is_silent() {
        let dir = tempdir().expect("Impossible de créer le tempdir");
        let env_path = dir.path().join(".env_inexistant");

        // Ceci ne doit pas paniquer (Zéro Panique)
        load_local_env(&env_path);
    }

    #[test]
    #[serial_test::serial]
    fn test_load_local_env_path_resolution() {
        let dir = tempdir().expect("Impossible de créer le tempdir");
        let env_path = dir.path().join(".env");

        // Injection de chemins relatifs
        let env_content = "
PATH_TEST_HOME=~/data_home
PATH_TEST_EXE=./data_exe
PATH_TEST_PARENT=../data_parent
        ";
        fs::write_sync(&env_path, env_content.as_bytes()).unwrap();

        // 1. Sauvegarde et Mock de l'environnement
        let original_home = RuntimeEnv::var("HOME").ok();
        RuntimeEnv::set_var("HOME", "/mock/home");

        RuntimeEnv::remove_var("PATH_TEST_HOME");
        RuntimeEnv::remove_var("PATH_TEST_EXE");
        RuntimeEnv::remove_var("PATH_TEST_PARENT");

        // 2. Exécution du Parseur
        load_local_env(&env_path);

        // 3. Vérification de l'ancrage Tilde (~)
        let expected_home = PathBuf::from("/mock/home")
            .join("data_home")
            .to_string_lossy()
            .to_string();
        assert_eq!(
            RuntimeEnv::var("PATH_TEST_HOME").unwrap(),
            expected_home,
            "Le chemin ~/ n'a pas été résolu correctement en chemin absolu"
        );

        // 4. Vérification de l'ancrage Binaire (./ et ../)
        let exe_path = RuntimeEnv::current_exe().expect("Impossible d'obtenir current_exe");
        let exe_dir = exe_path.parent().unwrap();

        let expected_exe = exe_dir.join("./data_exe").to_string_lossy().to_string();
        assert_eq!(
            RuntimeEnv::var("PATH_TEST_EXE").unwrap(),
            expected_exe,
            "Le chemin ./ n'a pas été correctement ancré sur l'exécutable"
        );

        let expected_parent = exe_dir.join("../data_parent").to_string_lossy().to_string();
        assert_eq!(
            RuntimeEnv::var("PATH_TEST_PARENT").unwrap(),
            expected_parent,
            "Le chemin ../ n'a pas été correctement ancré sur l'exécutable"
        );

        // 5. Nettoyage de l'environnement pour les tests suivants
        if let Some(home) = original_home {
            RuntimeEnv::set_var("HOME", &home);
        } else {
            RuntimeEnv::remove_var("HOME");
        }
    }
}
