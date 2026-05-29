// FICHIER : src-tauri/src/utils/core/macros.rs

#[macro_export]
macro_rules! async_test {
    ($($item:item)*) => {
        #[tokio::test]
        $($item)*
    };
}

#[macro_export]
macro_rules! async_interface {
    ($($item:item)*) => {
        #[::async_trait::async_trait]
        $($item)*
    };
}

/// Affiche une info à l'utilisateur (traduite) et logue l'événement
#[macro_export]
macro_rules! user_info {
    ($key:expr) => {{
        let msg = $crate::utils::context::i18n::t($key);
        $crate::utils::tracing::info!(
            target: "user_notification",
            event_id = $key,
            "{}", msg
        );
    }};
    ($key:expr, $context:expr) => {{
        let msg = $crate::utils::context::i18n::t($key);
        $crate::utils::tracing::info!(
            target: "user_notification",
            event_id = $key,
            context = %$context,
            "{}", msg
        );
    }};
}

/// Affiche une information de trace ultra-verbeuse (mode développeur)
#[macro_export]
macro_rules! user_trace {
    ($key:expr) => {{
        let msg = $crate::utils::context::i18n::t($key);
        $crate::utils::tracing::trace!(
            target: "user_notification",
            event_id = $key,
            severity = "trace",
            "🔍 {}", msg
        );
    }};
    ($key:expr, $context:expr) => {{
        let msg = $crate::utils::context::i18n::t($key);
        $crate::utils::tracing::trace!(
            target: "user_notification",
            event_id = $key,
            severity = "trace",
            context = %$context,
            "🔍 {}", msg
        );
    }};
}

/// Affiche un succès (vert) à l'utilisateur
#[macro_export]
macro_rules! user_success {
    ($key:expr) => {{
        let msg = $crate::utils::context::i18n::t($key);
        $crate::utils::tracing::info!(
            target: "user_notification",
            event_id = $key,
            severity = "success",
            "✅ {}", msg
        );
    }};
    ($key:expr, $context:expr) => {{
        let msg = $crate::utils::context::i18n::t($key);
        $crate::utils::tracing::info!(
            target: "user_notification",
            event_id = $key,
            severity = "success",
            context = %$context,
            "✅ {}", msg
        );
    }};
}

/// Affiche un avertissement (jaune/orange) à l'utilisateur
#[macro_export]
macro_rules! user_warn {
    ($key:expr) => {{
        let msg = $crate::utils::context::i18n::t($key);
        $crate::utils::tracing::warn!(
            target: "user_notification",
            event_id = $key,
            severity = "warning",
            "⚠️ {}", msg
        );
    }};
    ($key:expr, $context:expr) => {{
        let msg = $crate::utils::context::i18n::t($key);
        $crate::utils::tracing::warn!(
            target: "user_notification",
            event_id = $key,
            severity = "warning",
            context = %$context,
            "⚠️ {}", msg
        );
    }};
}

/// Affiche une information de débogage (mode verbeux) à l'utilisateur
#[macro_export]
macro_rules! user_debug {
    ($key:expr) => {{
        let msg = $crate::utils::context::i18n::t($key);
        $crate::utils::tracing::debug!(
            target: "user_notification",
            event_id = $key,
            severity = "debug",
            "🐛 {}", msg
        );
    }};
    ($key:expr, $context:expr) => {{
        let msg = $crate::utils::context::i18n::t($key);
        $crate::utils::tracing::debug!(
            target: "user_notification",
            event_id = $key,
            severity = "debug",
            context = %$context,
            "🐛 {}", msg
        );
    }};
}

#[macro_export]
macro_rules! user_error {
    ($key:expr) => {{
        let msg = $crate::utils::context::i18n::t($key);
        $crate::utils::tracing::error!(
            target: "user_notification",
            event_id = $key,
            severity = "error",
            "❌ {}", msg
        );
    }};
    ($key:expr, $context:expr) => {{
        let msg = $crate::utils::context::i18n::t($key);
        $crate::utils::tracing::error!(
            target: "user_notification",
            event_id = $key,
            severity = "error",
            context = %$context,
            "❌ {}", msg
        );
    }};
}

/// Trace bas niveau pour les étapes de démarrage du noyau.
#[macro_export]
macro_rules! kernel_trace {
    ($action:expr, $detail:expr) => {
        eprintln!("[RAISE KERNEL] ⚙️  {:<20} | {}", $action, $detail);
    };
}

/// Erreur fatale bas niveau (Kernel Panic) déclenchée avant que le logger ne soit prêt.
#[macro_export]
macro_rules! kernel_fatal {
    ($context:expr, $file:expr, $err:expr) => {
        eprintln!("\n========================================================");
        eprintln!("🚨 [RAISE KERNEL PANIC] ÉCHEC CRITIQUE D'INITIALISATION");
        eprintln!("========================================================");
        eprintln!("📍 Phase   : {}", $context);
        eprintln!("📄 Fichier : {}", $file);
        eprintln!("🔥 Erreur  : {}", $err);
        eprintln!("========================================================\n");
    };
}

/// 🚀 Macro surpuissante pour générer des erreurs structurées AI-Ready
#[macro_export]
macro_rules! build_error {
    ($key:expr, error = $err:expr, context = $ctx:expr, correlation_id = $cid:expr, user_id = $uid:expr) => {
        $crate::build_error!(@internal $key, Some($err.to_string()), $ctx, Some($cid.to_string()), Some($uid.to_string()))
    };
    ($key:expr, error = $err:expr, context = $ctx:expr) => {
        $crate::build_error!(@internal $key, Some($err.to_string()), $ctx, None::<String>, None::<String>)
    };
    ($key:expr, error = $err:expr) => {
        $crate::build_error!(@internal $key, Some($err.to_string()), $crate::utils::data::json::json_value!({}), None::<String>, None::<String>)
    };
    ($key:expr, context = $ctx:expr) => {
        $crate::build_error!(@internal $key, None::<String>, $ctx, None::<String>, None::<String>)
    };
    ($key:expr) => {
        $crate::build_error!(@internal $key, None::<String>, $crate::utils::data::json::json_value!({}), None::<String>, None::<String>)
    };

    // =========================================================================
    // LE CERVEAU (Interne)
    // =========================================================================
    (@internal $key:expr, $err:expr, $ctx:expr, $corr_id:expr, $usr_id:expr) => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str { std::any::type_name::<T>() }
        let action_name = type_name_of(f).rsplit("::").nth(1).unwrap_or("UNKNOWN_ACTION").to_uppercase();

        let mod_path = module_path!();
        let parts: Vec<&str> = mod_path.split("::").collect();
        let (service, subdomain, component) = match parts.len() {
            0 | 1 => ("core", "general", "unknown"),
            2 => (parts[1], "general", parts[1]),
            3 => (parts[1], "core", parts[2]),
            _ => (parts[1], parts[2], parts[3]),
        };

        // 🎯 Utilisation de notre alias JsonObject
        let mut ctx_map = $crate::utils::data::json::JsonObject::new();
        ctx_map.insert("action".to_string(), $crate::utils::data::json::json_value!(action_name));

        if let Some(cid) = $corr_id { ctx_map.insert("correlationId".to_string(), $crate::utils::data::json::json_value!(cid)); }
        if let Some(uid) = $usr_id { ctx_map.insert("userId".to_string(), $crate::utils::data::json::json_value!(uid)); }

        if let Some(ref e) = $err {
            ctx_map.insert("technical_error".to_string(), $crate::utils::data::json::json_value!(e));
        }

        let context_value = $ctx;
        if let $crate::utils::data::json::JsonValue::Object(user_map) = context_value {
            for (k, v) in user_map { ctx_map.insert(k, v); }
        } else {
            ctx_map.insert("data".to_string(), context_value);
        }

        let final_context = $crate::utils::data::json::JsonValue::Object(ctx_map);
        let reason_msg = $crate::utils::context::i18n::t($key);

        $crate::utils::tracing::error!(
            event = "user_error",
            key = $key,
            service = %service,
            subdomain = %subdomain,
            componentName = %component.to_uppercase(),
            action = %action_name,
            reason = %reason_msg,
            error = ?$err,
            context = %final_context,
            "❌ [{}] {}", component.to_uppercase(), reason_msg
        );

        $crate::utils::core::error::AppError::Structured(Box::new($crate::utils::core::error::StructuredData {
            service: service.to_string(),
            subdomain: subdomain.to_string(),
            component: component.to_uppercase(),
            code: $key.to_string(),
            message: reason_msg,
            context: final_context,
        }))
    }};
}

/// 🚀 Macro de DIVERGENCE (Fait un return Err)
#[macro_export]
macro_rules! raise_error {
    ($($arg:tt)*) => {
        return Err($crate::build_error!($($arg)*))
    };
}

/// 🛡️ Macro de GARDE : Valide la session et met à jour l'horodatage
#[macro_export]
macro_rules! require_session {
    ($state:expr) => {{
        match $state.get_current_session().await {
            Some(session) => {
                let _ = $state.touch().await;
                session
            }
            None => {
                return Err($crate::build_error!(
                    "ERR_UNAUTHORIZED",
                    error = "Accès refusé : aucune session active",
                    context = $crate::utils::data::json::json_value!({
                        "hint": "Vous devez appeler 'session_login' avant d'exécuter cette commande."
                    })
                ));
            }
        }
    }};
}

// ============================================================================
// TESTS UNITAIRES DES MACROS
// ============================================================================
#[cfg(test)]
mod tests {
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::utils::core::error::{AppError, RaiseResult};
    use crate::utils::data::json::json_value;
    use crate::utils::testing::mock::inject_mock_user;

    #[test]
    fn test_build_error_key_only() {
        let err = crate::build_error!("ERR_SIMPLE");
        let AppError::Structured(data) = err;

        assert_eq!(data.code, "ERR_SIMPLE");
        assert!(data.context.get("action").is_some());
    }

    #[test]
    fn test_build_error_with_technical_error() {
        let db_err = "Connection refused";
        let err = crate::build_error!("ERR_DB", error = db_err);

        // 🎯 ALTERNATIVE ROBUSTE : Déstructuration directe d'un motif irréfutable
        // Comme AppError n'a qu'un seul variant, on l'extrait sans match ni rattrapage.
        let AppError::Structured(data) = err;

        assert_eq!(data.code, "ERR_DB");

        // Extraction sécurisée de l'erreur technique via le contexte sémantique
        let tech_err = data
            .context
            .get("technical_error")
            .and_then(|v| v.as_str())
            .expect("La propriété technical_error est manquante ou mal formatée");

        assert_eq!(tech_err, "Connection refused");
    }

    #[test]
    fn test_build_error_with_full_context() {
        let err = crate::build_error!(
            "ERR_API",
            error = "Timeout",
            context = json_value!({"retry": true, "timeout_ms": 5000})
        );

        // 🎯 ALTERNATIVE ROBUSTE : Déstructuration directe d'un motif irréfutable
        // Comme AppError n'a qu'un seul variant, on l'extrait sans match ni rattrapage.
        let AppError::Structured(data) = err;

        assert_eq!(data.code, "ERR_API");

        // Validation des métadonnées du contexte sémantique
        assert_eq!(
            data.context.get("retry").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            data.context.get("timeout_ms").and_then(|v| v.as_i64()),
            Some(5000)
        );
        assert_eq!(
            data.context.get("technical_error").and_then(|v| v.as_str()),
            Some("Timeout")
        );
    }

    #[test]
    fn test_raise_error_control_flow() {
        fn simulate_failure() -> RaiseResult<i32> {
            crate::raise_error!("ERR_CRITICAL", error = "Crash");
            #[allow(unreachable_code)]
            Ok(42)
        }

        let result = simulate_failure();

        // 🎯 ALTERNATIVE ROBUSTE : Extraction directe du motif irréfutable
        // On utilise 'unwrap_err()' pour obtenir l'AppError, puis on déstructure
        // le variant unique 'Structured' directement.
        let err = result.expect_err("La fonction aurait dû retourner une erreur");
        let AppError::Structured(data) = err;

        assert_eq!(data.code, "ERR_CRITICAL");
        assert_eq!(data.context["technical_error"], "Crash");
    }

    #[tokio::test]
    async fn test_require_session_guard() -> RaiseResult<()> {
        use crate::utils::context::session::{Session, SessionManager};
        use crate::utils::testing::mock::AgentDbSandbox;

        async fn mock_protected_command(manager: &SessionManager) -> RaiseResult<Session> {
            let session = crate::require_session!(manager);
            Ok(session)
        }

        let sandbox = AgentDbSandbox::new().await?;
        let manager = SessionManager::new(sandbox.db.clone());

        // ====================================================================
        // 1. CAS D'ÉCHEC : Pas de session (Vérification de la Garde)
        // ====================================================================
        let err_result = mock_protected_command(&manager).await;

        // 🎯 ALTERNATIVE ROBUSTE : On attend une erreur, puis on déstructure directement.
        // Comme AppError n'a qu'un variant, le compilateur accepte cette syntaxe.
        let err = err_result.expect_err("La macro aurait dû bloquer l'accès");
        let AppError::Structured(err_data) = err;

        assert_eq!(err_data.code, "ERR_UNAUTHORIZED");
        assert_eq!(
            err_data
                .context
                .get("technical_error")
                .and_then(|v| v.as_str()),
            Some("Accès refusé : aucune session active")
        );

        // ====================================================================
        // 2. CAS DE SUCCÈS : Session active (Validation sémantique)
        // ====================================================================
        let test_user = "agent-macro";

        // 🎯 RÉSILIENCE MOUNT POINTS : Utilisation dynamique de la configuration
        let db_mgr = CollectionsManager::new(
            &sandbox.db,
            &sandbox.config.mount_points.system.domain,
            &sandbox.config.mount_points.system.db,
        );
        inject_mock_user(&db_mgr, test_user).await;

        manager
            .start_session(test_user)
            .await
            .expect("Échec démarrage session");

        let success_result = mock_protected_command(&manager).await;

        // Assertion directe sur le succès
        let session = success_result.expect("La macro a bloqué l'accès à tort");
        assert_eq!(session.user_handle, test_user);

        Ok(())
    }
}
