// FICHIER : src-tauri/src/utils/io/os.rs

// 1. Types de base et Chemins (Provenance : io)
use crate::utils::io::fs::Path;
use crate::utils::io::io_traits::{SyncBufRead, SyncWrite};
use crate::utils::io::os_types::{ProcessCommand, ProcessIoConfig};

// 3. Core : Erreurs, Observabilité et Diagnostics
use crate::utils::core::error::RaiseResult;
use crate::utils::core::instrument; // 🎯 On passe par ta façade core !
use crate::utils::io::{stdin_raw, stdout_raw};

// 4. Macros RAISE et Locales
use crate::utils::data::json::json_value;
use crate::{raise_error, user_debug, user_warn}; // Propulsées à la racine

/// Exécute une commande système et capture sa sortie.
/// Utile pour lancer des outils comme Cargo, Git, etc.
#[instrument(skip(args), fields(cmd = cmd, cwd = ?cwd))]
pub fn exec_command_sync(cmd: &str, args: &[&str], cwd: Option<&Path>) -> RaiseResult<String> {
    user_debug!(
        "MSG_OS_EXEC_START",
        json_value!({ "cmd": cmd, "args": args })
    );

    let mut command = ProcessCommand::new(cmd);
    command.args(args);

    // Configuration du dossier courant
    if let Some(dir) = cwd {
        if !dir.exists() {
            raise_error!(
                "ERR_OS_CWD_NOT_FOUND",
                error = "Dossier d'exécution introuvable",
                context = json_value!({ "path": dir.to_string_lossy() })
            );
        }
        command.current_dir(dir);
    }

    command.stdout(ProcessIoConfig::piped());
    command.stderr(ProcessIoConfig::piped());

    match command.output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();

            if output.status.success() {
                user_debug!("MSG_OS_EXEC_SUCCESS");
                Ok(stdout)
            } else {
                user_warn!(
                    "MSG_OS_EXEC_FAILED",
                    json_value!({ "code": output.status.code() })
                );
                raise_error!(
                    "ERR_OS_COMMAND_EXIT_ERROR",
                    error = stderr.trim(),
                    context = json_value!({
                        "cmd": cmd,
                        "args": args,
                        "exit_code": output.status.code()
                    })
                );
            }
        }
        Err(e) => {
            raise_error!(
                "ERR_OS_EXEC_SPAWN",
                error = e,
                context = json_value!({ "cmd": cmd, "args": args })
            );
        }
    }
}

/// Exécute une commande système de manière asynchrone (Non-bloquante pour Tauri).
pub async fn exec_command_async(
    cmd: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> RaiseResult<String> {
    let cmd_owned = cmd.to_string();
    let args_owned: Vec<String> = args.iter().map(|&s| s.to_string()).collect();
    let cwd_owned = cwd.map(|p| p.to_path_buf());

    let join_res = tokio::task::spawn_blocking(move || {
        let args_ref: Vec<&str> = args_owned.iter().map(|s| s.as_str()).collect();
        exec_command_sync(&cmd_owned, &args_ref, cwd_owned.as_deref())
    })
    .await;

    // 🎯 Un match propre et explicite !
    match join_res {
        Ok(Ok(stdout)) => Ok(stdout),
        Ok(Err(e)) => Err(e), // Propagation propre de l'erreur métier
        Err(e) => raise_error!(
            "ERR_OS_ASYNC_PANIC",
            error = e,
            context = json_value!({ "cmd": cmd })
        ),
    }
}

/// Lance l'application asynchrone en gardant le contrôle strict sur le CPU.
pub fn run_edge_node<F>(app: F) -> RaiseResult<()>
where
    F: std::future::Future<Output = RaiseResult<()>>,
{
    // Construction manuelle du moteur Tokio
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        // STRATÉGIE EDGE : On limite Tokio à 2 threads réseau/I/O
        // pour sanctuariser les autres cœurs du Pi 5 pour Candle et la logique Rust
        .worker_threads(2)
        .build()
        .unwrap();

    // On bloque le thread principal pour faire tourner le futur asynchrone
    rt.block_on(app)
}

/// Exécute une tâche d'inférence ou de calcul intensif (CPU/GPU) sur un thread dédié.
/// 🎯 ZÉRO DETTE : Isole le runtime Tokio des opérations bloquantes pour éviter le gel de l'UI.
#[instrument(skip(task))]
pub async fn execute_native_inference<F, T>(task: F) -> RaiseResult<T>
where
    F: FnOnce() -> RaiseResult<T> + Send + 'static,
    T: Send + 'static,
{
    // On utilise le pool de threads bloquants de l'exécuteur asynchrone
    let join_res = tokio::task::spawn_blocking(task).await;

    match join_res {
        Ok(res) => res, // On propage le RaiseResult<T> de la tâche
        Err(e) => raise_error!(
            "ERR_OS_INFERENCE_THREAD_PANIC",
            error = e.to_string(),
            context = json_value!({
                "action": "execute_native_inference",
                "hint": "Le thread de calcul intensif a été interrompu ou a paniqué."
            })
        ),
    }
}

/// Passe une chaîne de caractères dans l'entrée standard (stdin) d'une commande
/// et récupère le résultat transformé (stdout).
#[instrument(skip(input), fields(cmd = cmd))]
pub fn pipe_through(cmd: &str, input: &str) -> RaiseResult<String> {
    let mut child = match ProcessCommand::new(cmd)
        .stdin(ProcessIoConfig::piped())
        .stdout(ProcessIoConfig::piped())
        .stderr(ProcessIoConfig::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => raise_error!(
            "ERR_OS_PIPE_SPAWN",
            error = e,
            context = json_value!({ "cmd": cmd })
        ),
    };

    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(input.as_bytes()) {
            raise_error!(
                "ERR_OS_PIPE_WRITE",
                error = e,
                context = json_value!({ "cmd": cmd })
            );
        }
    }

    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(e) => raise_error!(
            "ERR_OS_PIPE_WAIT",
            error = e,
            context = json_value!({ "cmd": cmd })
        ),
    };

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        raise_error!(
            "ERR_OS_PIPE_EXEC_ERROR",
            error = stderr.trim(),
            context = json_value!({ "cmd": cmd })
        );
    }
}

/// Force l'affichage immédiat sur la console (flush stdout).
#[instrument]
pub fn flush_stdout() -> RaiseResult<()> {
    if let Err(e) = stdout_raw().flush() {
        raise_error!("ERR_OS_STDOUT_FLUSH", error = e);
    }
    Ok(())
}

/// Lit une ligne depuis l'entrée standard (stdin) de manière synchrone.
#[instrument]
pub fn read_stdin_line() -> RaiseResult<String> {
    let mut input = String::new();
    let stdin = stdin_raw();
    let mut handle = stdin.lock();

    match handle.read_line(&mut input) {
        Ok(_) => Ok(input.trim().to_string()),
        Err(e) => {
            raise_error!(
                "ERR_OS_STDIN_READ",
                error = e,
                context = json_value!({ "source": "stdin" })
            );
        }
    }
}

pub fn prompt(message: &str) -> RaiseResult<String> {
    print!("{}", message);
    flush_stdout()?;
    read_stdin_line()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::core::error::AppError;

    #[test]
    fn test_exec_command_success() {
        let res = exec_command_sync("cargo", &["--version"], None);
        assert!(res.is_ok());
    }

    #[test]
    fn test_exec_command_not_found() {
        let res = exec_command_sync("commande_qui_n_existe_pas_12345", &[], None);
        assert!(res.is_err());

        let AppError::Structured(data) = res.unwrap_err();
        assert_eq!(data.code, "ERR_OS_EXEC_SPAWN");
    }

    #[test]
    fn test_exec_command_failure_status() {
        let res = exec_command_sync("cargo", &["build", "--manifest-path", "ghost.toml"], None);
        assert!(res.is_err());

        let AppError::Structured(data) = res.unwrap_err();
        assert_eq!(data.code, "ERR_OS_COMMAND_EXIT_ERROR");
    }

    // =========================================================================
    // TESTS ASYNCHRONES (RAISE v2.1)
    // =========================================================================

    #[tokio::test]
    async fn test_exec_command_async_success() {
        // 🎯 On utilise .await pour dépiler la Future
        let res = exec_command_async("cargo", &["--version"], None).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_exec_command_async_not_found() {
        let res = exec_command_async("commande_qui_n_existe_pas_async", &[], None).await;
        assert!(res.is_err());

        let AppError::Structured(data) = res.unwrap_err();
        // Vérification de la propagation de l'erreur via la tâche bloquante
        assert_eq!(data.code, "ERR_OS_EXEC_SPAWN");
    }

    #[tokio::test]
    async fn test_exec_command_async_failure_status() {
        let res = exec_command_async(
            "cargo",
            &["build", "--manifest-path", "ghost_async.toml"],
            None,
        )
        .await;
        assert!(res.is_err());

        let AppError::Structured(data) = res.unwrap_err();
        assert_eq!(data.code, "ERR_OS_COMMAND_EXIT_ERROR");
    }
}
