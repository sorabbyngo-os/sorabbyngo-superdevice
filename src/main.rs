mod superdevice;
mod hmaf;
mod starshield;

use superdevice::{SuperDevice, VirtualDeviceType};
use hmaf::HmafOrchestrator;
use starshield::TrustStore;

fn main() {
    println!("╔══════════════════════════════════════════╗");
    println!("║  Sorabbyngo Super Device — starting up   ║");
    println!("╚══════════════════════════════════════════╝");

    let mut trust       = TrustStore::new();
    let node_id         = trust.local_node_id();
    let mut superdevice = SuperDevice::new(node_id);
    let mut hmaf        = HmafOrchestrator::new(node_id);

    superdevice.register_local_hardware();

    // Register all 8 byngox agents
    for agent in &["health","drug","context","network",
                   "learning","population","security","inference"] {
        hmaf.register_agent(&format!("{}-agent", agent), agent);
    }

    println!("[superdevice] {}", superdevice.pool_summary());
    hmaf.print_status();
    trust.print_status();

    // Allocate GPU to inference agent
    if let Some(id) = superdevice.allocate_best(
        &VirtualDeviceType::Gpu, "inference-agent") {
        println!("[superdevice] inference-agent got GPU device id={}", id);
    }

    // Keep running — in production: listen on Unix socket for allocation requests
    loop { std::thread::sleep(std::time::Duration::from_secs(10)); }
}
