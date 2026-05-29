// FICHIER : src-tauri/src/utils/mod.rs

// =========================================================================
//  RAISE UTILS V2.0 - Clean AI-Ready Foundation Layer (Strict Mode)
// =========================================================================

// --- 1. DÉCLARATION DES SOUS-DOMAINES PHYSIQUES ---
// Ces modules contiennent l'implémentation réelle.
pub mod context;
pub mod core;
pub mod data;
pub mod inference;
pub mod io;
pub mod network;
pub mod prelude;

#[cfg(any(test, debug_assertions))]
pub mod testing;

// =========================================================================
// 2. EXPORTS SÉMANTIQUES (L'ESSENCE DE RAISE)
// =========================================================================
// On n'expose ici que les piliers du framework. Tout le reste passe par le prelude.

// --- 🛡️ GESTION DES ERREURS ---
// Centralisation des types de retour pour toute la crate.
pub use core::error::{anyhow, AnyResult, AppError, Context, RaiseResult};

// --- 📦 DATA & CONFIGURATION ---
pub use data::config::AppConfig;
pub use data::json;

// --- 🎯 ALIAS SÉMANTIQUES DE CONCURRENCE ---
// On masque les types 'std' derrière le vocabulaire RAISE défini dans core.
pub use core::{InitGuard, SharedRef, StaticCell};

// --- 🧬 ALIAS SÉMANTIQUES DE DOMAINE ---
// On masque 'uuid' et 'chrono' derrière le vocabulaire métier.
pub use core::{LocalClock, LocalTimestamp, UniqueId, UtcClock, UtcTimestamp};

// =========================================================================
// 3. RÉ-EXPORTS TECHNIQUES (OUTILS TIERS)
// =========================================================================
// On ne ré-exporte que les crate racines nécessaires aux macros ou au moteur.

pub use tokio;
pub use tracing;
// Les macros #[macro_export] (raise_error!, json_value!, user_info!)
// sont automatiquement rattachées à la racine de la crate.
