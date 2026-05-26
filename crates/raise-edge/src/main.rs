// FICHIER : crates/raise-edge/src/main.rs

// Importations strictes et exclusives depuis la façade réseau du noyau
use raise_core::utils::core::error::RaiseResult;
use raise_core::utils::io::os::run_edge_node;
use raise_core::utils::network::server::{get, new_http_router, start_network_api_async};

fn main() -> RaiseResult<()> {
    println!("⚙️ Démarrage du moteur R.A.I.S.E...");

    // On délègue toute la mécanique asynchrone au noyau
    run_edge_node(async {
        println!("🚀 Agent Edge Online !");

        let app = new_http_router().route("/health", get(|| async { "Système Opérationnel\n" }));

        start_network_api_async("0.0.0.0", 3000, app).await?;

        Ok(())
    })
}
