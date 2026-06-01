// FICHIER : crates/raise-core/src/kernel/assets.rs

use crate::utils::prelude::*; // 🎯 Façade Unique RAISE (Tolérance Zéro)

pub struct AssetResolver;

impl AssetResolver {
    /// Résout un fichier avec la logique stricte de fallback (Domaine/DB -> Shared System).
    /// Nomenclature `_sync` car l'opération bloque le thread courant pour sonder le système de fichiers.
    pub fn resolve_ai_file_sync(
        primary_base_path: &Path,
        asset_category_path: &str, // ex: "ai-assets/models"
        filename: &str,
    ) -> Option<PathBuf> {
        let config = AppConfig::get();

        // 1. Test du chemin primaire (Spécifique au domaine/db en cours)
        let primary = primary_base_path.join(filename);
        if fs::exists_sync(&primary) {
            return Some(primary);
        }

        // 2. Test du chemin partagé (Global _system)
        let raise_domain_path = config
            .get_path("PATH_RAISE_DOMAIN")
            .unwrap_or_else(|| PathBuf::from("./raise_domain"));

        let shared = raise_domain_path
            .join("_system")
            .join(asset_category_path)
            .join(filename);

        if fs::exists_sync(&shared) {
            return Some(shared);
        }

        // 3. Introuvable
        None
    }

    /// Génère un contexte d'erreur JSON standardisé pour les logs en cas d'échec
    pub fn missing_file_context(
        primary_base_path: &Path,
        asset_category_path: &str,
        filename: &str,
    ) -> JsonValue {
        let raise_domain_path = AppConfig::get()
            .get_path("PATH_RAISE_DOMAIN")
            .unwrap_or_else(|| PathBuf::from("./raise_domain"));

        json_value!({
            "filename": filename,
            "checked_primary": primary_base_path.join(filename).to_string_lossy(),
            "checked_shared": raise_domain_path.join("_system").join(asset_category_path).join(filename).to_string_lossy()
        })
    }
}

// =========================================================================
// TESTS UNITAIRES (Validation Stricte de la Cascade & Façade FS)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::testing::AgentDbSandbox;

    /// Helper local qui respecte ABSOLUMENT la façade Raise
    fn touch_test_file(path: &Path) -> RaiseResult<()> {
        if let Some(parent) = path.parent() {
            fs::ensure_dir_sync(parent)?;
        }
        fs::write_sync(path, b"dummy data")?;
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_resolver_priority_primary_over_shared() -> RaiseResult<()> {
        let _sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        let raise_domain_path = config.get_path("PATH_RAISE_DOMAIN").unwrap();
        let primary_path = raise_domain_path
            .join("test_domain")
            .join("test_db")
            .join("ai-assets");
        let category = "ai-assets/models";
        let filename = "qwen_test.gguf";

        let expected_primary = primary_path.join(filename);
        let expected_shared = raise_domain_path
            .join("_system")
            .join(category)
            .join(filename);

        // On crée les DEUX fichiers
        touch_test_file(&expected_primary)?;
        touch_test_file(&expected_shared)?;

        // Exécution via la façade
        let resolved = AssetResolver::resolve_ai_file_sync(&primary_path, category, filename);

        assert!(resolved.is_some(), "Le fichier devrait être résolu");
        assert_eq!(
            resolved.unwrap(),
            expected_primary,
            "Le résolveur DOIT prioriser le chemin primaire"
        );

        // Nettoyage via la façade
        let _ = fs::remove_file_sync(&expected_primary);
        let _ = fs::remove_file_sync(&expected_shared);
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_resolver_fallback_to_shared() -> RaiseResult<()> {
        let _sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        let raise_domain_path = config.get_path("PATH_RAISE_DOMAIN").unwrap();
        let primary_path = raise_domain_path
            .join("test_domain")
            .join("test_db")
            .join("ai-assets");
        let category = "ai-assets/models";
        let filename = "whisper_test.bin";

        let expected_primary = primary_path.join(filename);
        let expected_shared = raise_domain_path
            .join("_system")
            .join(category)
            .join(filename);

        if fs::exists_sync(&expected_primary) {
            let _ = fs::remove_file_sync(&expected_primary);
        }
        touch_test_file(&expected_shared)?;

        let resolved = AssetResolver::resolve_ai_file_sync(&primary_path, category, filename);

        assert!(
            resolved.is_some(),
            "Le fichier devrait être résolu via le fallback"
        );
        assert_eq!(
            resolved.unwrap(),
            expected_shared,
            "Le résolveur aurait dû basculer sur le chemin partagé"
        );

        let _ = fs::remove_file_sync(&expected_shared);
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_resolver_file_not_found() -> RaiseResult<()> {
        let _sandbox = AgentDbSandbox::new().await?;
        let raise_domain_path = AppConfig::get().get_path("PATH_RAISE_DOMAIN").unwrap();

        let primary_path = raise_domain_path.join("ghost_domain").join("ghost_db");
        let category = "ai-assets/ghost";
        let filename = "ghost_model.gguf";

        let resolved = AssetResolver::resolve_ai_file_sync(&primary_path, category, filename);

        assert!(
            resolved.is_none(),
            "Le résolveur doit retourner None si le fichier n'existe nulle part"
        );
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_resolver_missing_context_format() -> RaiseResult<()> {
        let _sandbox = AgentDbSandbox::new().await?;

        let primary_path = PathBuf::from("/mock/primary/path");
        let category = "ai-assets/models";
        let filename = "test.json";

        let context = AssetResolver::missing_file_context(&primary_path, category, filename);

        assert_eq!(context["filename"], "test.json");
        assert!(context["checked_primary"]
            .as_str()
            .unwrap()
            .contains("/mock/primary/path/test.json"));
        assert!(context["checked_shared"]
            .as_str()
            .unwrap()
            .contains("_system/ai-assets/models/test.json"));
        Ok(())
    }
}
