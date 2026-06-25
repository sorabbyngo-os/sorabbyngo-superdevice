//! dsoftbus/starshield.rs
//!
//! Sorabbyngo StarShield — Zero Trust security layer
//! Inspired by HarmonyOS StarShield + Sorabbyngo NodeID
//!
//! Every node has a hardware-rooted identity (NodeID).
//! Every channel is authenticated before any data flows.
//! Every resource access is verified against a Zero Trust policy.
//!
//! Zero Trust principles enforced here:
//!   1. Never trust, always verify — even local processes
//!   2. Assume breach — every channel acts as if network is hostile
//!   3. Least privilege — nodes only see capabilities they need
//!   4. Continuous verification — re-auth on every session

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ── NodeID — hardware-rooted device identity ───────────────────────────────

#[derive(Debug, Clone)]
pub struct NodeIdentity {
    pub node_id:      [u8; 32],    // BLAKE3(TPM_seed || machine_id || boot_time)
    pub public_key:   [u8; 32],    // X25519 public key for channel encryption
    pub attestation:  [u8; 64],    // hardware attestation (TPM/secure enclave)
    pub capabilities: u64,
    pub trust_level:  TrustLevel,
    pub first_seen:   Instant,
    pub last_verified:Instant,
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub enum TrustLevel {
    Unknown   = 0,
    Verified  = 1,   // PIN-authenticated, NodeID confirmed
    Attested  = 2,   // hardware attestation verified (TPM)
    Trusted   = 3,   // long-term trusted node (user-approved)
}

impl NodeIdentity {
    pub fn new_local() -> Self {
        // In production: derive from TPM seed + /etc/machine-id
        let node_id = Self::derive_local_id();
        Self {
            node_id,
            public_key:   [0u8; 32],   // set from X25519 keygen
            attestation:  [0u8; 64],   // set from TPM attestation
            capabilities: 0,
            trust_level:  TrustLevel::Trusted,
            first_seen:   Instant::now(),
            last_verified:Instant::now(),
        }
    }

    fn derive_local_id() -> [u8; 32] {
        // Read /etc/machine-id + hostname and hash them
        let machine_id = std::fs::read_to_string("/etc/machine-id")
            .unwrap_or_else(|_| "unknown-machine".to_string());
        let hostname = std::fs::read_to_string("/etc/hostname")
            .unwrap_or_else(|_| "sorabbyngo-node".to_string());
        let combined = format!("sorabbyngo-nodeid:{}:{}", 
            machine_id.trim(), hostname.trim());
        // BLAKE3 hash (placeholder — use blake3 crate in production)
        let mut out = [0u8; 32];
        for (i, b) in combined.bytes().enumerate() {
            out[i % 32] ^= b.wrapping_add(i as u8);
        }
        out
    }

    pub fn node_id_short(&self) -> String {
        self.node_id[0..8].iter().map(|b| format!("{:02x}", b)).collect()
    }
}

// ── Zero Trust policy engine ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ZeroTrustPolicy {
    /// Minimum trust level required for each operation
    pub rules: HashMap<String, TrustLevel>,
}

impl ZeroTrustPolicy {
    pub fn default_policy() -> Self {
        let mut rules = HashMap::new();
        // Discovery: any node can announce itself (Unknown)
        rules.insert("discovery.announce".to_string(),  TrustLevel::Unknown);
        // Channel open: must be PIN-verified
        rules.insert("channel.open".to_string(),        TrustLevel::Verified);
        // File transfer: must be PIN-verified
        rules.insert("snap.file".to_string(),           TrustLevel::Verified);
        // App state transfer: must be attested
        rules.insert("snap.app_state".to_string(),      TrustLevel::Attested);
        // Agent migration: must be trusted
        rules.insert("hmaf.migrate".to_string(),        TrustLevel::Trusted);
        // GPU allocation: must be attested
        rules.insert("superdevice.gpu".to_string(),     TrustLevel::Attested);
        // Camera access: must be trusted
        rules.insert("superdevice.camera".to_string(),  TrustLevel::Trusted);
        Self { rules }
    }

    pub fn check(&self, operation: &str, node: &NodeIdentity) -> bool {
        match self.rules.get(operation) {
            None          => false,  // deny unknown operations
            Some(required) => node.trust_level >= *required,
        }
    }

    pub fn add_rule(&mut self, op: &str, level: TrustLevel) {
        self.rules.insert(op.to_string(), level);
    }
}

// ── Trust store ────────────────────────────────────────────────────────────

pub struct TrustStore {
    local:  NodeIdentity,
    peers:  HashMap<[u8; 32], NodeIdentity>,
    policy: ZeroTrustPolicy,
    audit:  Vec<TrustAuditEntry>,
}

#[derive(Debug, Clone)]
pub struct TrustAuditEntry {
    pub timestamp_ns: u64,
    pub node_id:      [u8; 32],
    pub operation:    String,
    pub allowed:      bool,
    pub trust_level:  TrustLevel,
}

impl TrustStore {
    pub fn new() -> Self {
        let local = NodeIdentity::new_local();
        println!("[starshield] NodeID: {}", local.node_id_short());
        Self {
            local,
            peers:  HashMap::new(),
            policy: ZeroTrustPolicy::default_policy(),
            audit:  Vec::new(),
        }
    }

    pub fn local_node_id(&self) -> [u8; 32] {
        self.local.node_id
    }

    /// Upgrade a peer's trust level after PIN verification
    pub fn verify_peer(&mut self, node_id: [u8; 32], pin_ok: bool) {
        let entry = self.peers.entry(node_id).or_insert_with(|| {
            NodeIdentity {
                node_id,
                public_key:   [0u8; 32],
                attestation:  [0u8; 64],
                capabilities: 0,
                trust_level:  TrustLevel::Unknown,
                first_seen:   Instant::now(),
                last_verified:Instant::now(),
            }
        });

        if pin_ok {
            entry.trust_level  = TrustLevel::Verified;
            entry.last_verified = Instant::now();
            println!("[starshield] peer {:?} -> Verified",
                &node_id[0..4]);
        }
    }

    /// Check if an operation is allowed for a peer node
    pub fn check(&mut self, node_id: &[u8; 32], operation: &str) -> bool {
        let node = match self.peers.get(node_id) {
            Some(n) => n.clone(),
            None    => {
                self.audit_entry(node_id, operation, false, TrustLevel::Unknown);
                return false;
            }
        };

        // Re-verification required if not seen in 5 minutes
        if node.last_verified.elapsed() > Duration::from_secs(300) {
            println!("[starshield] peer {:?} needs re-verification",
                &node_id[0..4]);
            self.audit_entry(node_id, operation, false, node.trust_level.clone());
            return false;
        }

        let allowed = self.policy.check(operation, &node);
        self.audit_entry(node_id, operation, allowed, node.trust_level.clone());

        if !allowed {
            println!("[starshield] DENY node={:?} op={}",
                &node_id[0..4], operation);
        }

        allowed
    }

    pub fn audit_log(&self) -> &[TrustAuditEntry] {
        &self.audit
    }

    pub fn print_status(&self) {
        println!("[starshield] local={} peers={}",
            self.local.node_id_short(), self.peers.len());
        for (id, node) in &self.peers {
            println!("  peer {:?} trust={:?}",
                &id[0..4], node.trust_level);
        }
    }

    fn audit_entry(&mut self, node_id: &[u8; 32],
                   op: &str, allowed: bool, level: TrustLevel) {
        self.audit.push(TrustAuditEntry {
            timestamp_ns: now_ns(),
            node_id:      *node_id,
            operation:    op.to_string(),
            allowed,
            trust_level:  level,
        });
    }
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
