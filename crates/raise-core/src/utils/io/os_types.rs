// FICHIER : src-tauri/src/utils/io/os_types.rs

/// 🤖 IA NOTE : Builder pour configurer et lancer un processus externe.
pub use std::process::Command as ProcessCommand;

/// 🤖 IA NOTE : Configuration des flux d'entrée/sortie (Pipe, Inherit, Null) pour un processus.
pub use std::process::Stdio as ProcessIoConfig;

/// 🤖 IA NOTE : Représente le résultat de l'exécution d'un processus (status, stdout, stderr).
pub use std::process::Output as ProcessOutput;

/// 🤖 IA NOTE : Code de sortie d'un processus.
pub use std::process::ExitStatus as ProcessExitStatus; // 🎯 Ajout pour la complétion

/// 🤖 IA NOTE : Représente un processus en cours d'exécution.
pub use std::process::Child as ProcessChild;

/// 🤖 IA NOTE : Extensions Unix pour les permissions de fichiers.
pub use std::os::unix::fs::PermissionsExt as UnixFilePermissions;

/// 🤖 IA NOTE : Récupération du dossier temporaire de l'OS.
pub use std::env::temp_dir as os_temp_dir;
