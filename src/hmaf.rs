//! dsoftbus/hmaf.rs
//!
//! Sorabbyngo HMAF — HarmonyOS Multi-Agent Framework adaptation
//!
//! Extends byngox's 8-agent swarm to run across device boundaries.
//! Agents can:
//!   - Migrate to any node in the Super Device pool
//!   - Share context through DSoftBus channels
//!   - Request remote hardware via SuperDevice allocator
//!   - Spawn sub-agents on remote nodes
//!
//! Agent mobility states:
//!   LOCAL    → running on this node
//!   MIGRATING→ being transferred to another node
//!   REMOTE   → running on another node, context shared here
//!   SHADOW   → lightweight context mirror of a remote agent

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq)]
pub enum AgentMobility {
    Local,
    Migrating { to_node: [u8; 32] },
    Remote    { on_node: [u8; 32] },
    Shadow    { of_node: [u8; 32] },
}

#[derive(Debug, Clone)]
pub struct AgentContext {
    pub agent_id:    String,
    pub agent_type:  String,     // "health", "drug", "context", "network", etc.
    pub state:       Vec<u8>,    // serialized agent state (msgpack/json)
    pub mobility:    AgentMobility,
    pub home_node:   [u8; 32],   // where this agent was spawned
    pub current_node:[u8; 32],   // where it's currently running
    pub last_sync:   Instant,
    pub capabilities_needed: Vec<String>,  // what hardware it needs
}

impl AgentContext {
    pub fn new(id: &str, agent_type: &str, home_node: [u8; 32]) -> Self {
        Self {
            agent_id:    id.to_string(),
            agent_type:  agent_type.to_string(),
            state:       Vec::new(),
            mobility:    AgentMobility::Local,
            home_node,
            current_node: home_node,
            last_sync:   Instant::now(),
            capabilities_needed: Vec::new(),
        }
    }

    pub fn is_local(&self) -> bool {
        self.mobility == AgentMobility::Local
    }

    pub fn serialize_for_migration(&self) -> Vec<u8> {
        // In production: use msgpack or protobuf
        // Here: simple length-prefixed format
        let mut out = Vec::new();
        let id_bytes = self.agent_id.as_bytes();
        out.extend_from_slice(&(id_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(id_bytes);
        let type_bytes = self.agent_type.as_bytes();
        out.extend_from_slice(&(type_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(type_bytes);
        out.extend_from_slice(&(self.state.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.state);
        out.extend_from_slice(&self.home_node);
        out
    }
}

// ── HMAF orchestrator ──────────────────────────────────────────────────────

pub struct HmafOrchestrator {
    local_node_id: [u8; 32],
    agents:        HashMap<String, AgentContext>,
    migration_log: Vec<MigrationRecord>,
}

#[derive(Debug, Clone)]
pub struct MigrationRecord {
    pub agent_id:   String,
    pub from_node:  [u8; 32],
    pub to_node:    [u8; 32],
    pub reason:     MigrationReason,
    pub timestamp:  u64,
    pub duration_ms:u64,
}

#[derive(Debug, Clone)]
pub enum MigrationReason {
    ResourceAvailability,  // better GPU/CPU on target node
    Latency,               // target node is closer to data source
    LoadBalance,           // this node is overloaded
    UserRequested,         // snap gesture threw the agent
    PowerSaving,           // consolidate to fewer nodes
}

impl HmafOrchestrator {
    pub fn new(node_id: [u8; 32]) -> Self {
        Self {
            local_node_id: node_id,
            agents:        HashMap::new(),
            migration_log: Vec::new(),
        }
    }

    /// Register a byngox agent with the HMAF layer
    pub fn register_agent(&mut self, id: &str, agent_type: &str) {
        let ctx = AgentContext::new(id, agent_type, self.local_node_id);
        println!("[hmaf] registered agent '{}' type={}", id, agent_type);
        self.agents.insert(id.to_string(), ctx);
    }

    /// Migrate an agent to a remote node
    /// Called when: user does a PUSH gesture, resource optimizer decides,
    ///              or a snap transfer includes app state
    pub fn migrate_agent(
        &mut self,
        agent_id:  &str,
        to_node:   [u8; 32],
        reason:    MigrationReason,
    ) -> Result<Vec<u8>, String> {
        let ctx = self.agents.get_mut(agent_id)
            .ok_or(format!("agent '{}' not found", agent_id))?;

        if !ctx.is_local() {
            return Err(format!("agent '{}' is not local", agent_id));
        }

        println!("[hmaf] migrating '{}' to node {:?} ({:?})",
            agent_id, &to_node[0..4], reason);

        let start = Instant::now();
        let payload = ctx.serialize_for_migration();

        // Mark as migrating
        ctx.mobility     = AgentMobility::Migrating { to_node };
        ctx.current_node = to_node;

        let duration_ms = start.elapsed().as_millis() as u64;

        // Record migration
        self.migration_log.push(MigrationRecord {
            agent_id:    agent_id.to_string(),
            from_node:   self.local_node_id,
            to_node,
            reason,
            timestamp:   now_ns(),
            duration_ms,
        });

        // Leave a shadow context locally
        if let Some(ctx) = self.agents.get_mut(agent_id) {
            ctx.mobility = AgentMobility::Shadow { of_node: to_node };
        }

        println!("[hmaf] migration payload: {} bytes, {}ms",
            payload.len(), duration_ms);

        Ok(payload)  // caller sends this via StreamChannel to target node
    }

    /// Receive a migrated agent from another node
    pub fn receive_agent(&mut self, payload: &[u8], from_node: [u8; 32])
        -> Result<String, String>
    {
        if payload.len() < 6 {
            return Err("payload too short".to_string());
        }

        let id_len  = u16::from_le_bytes(payload[0..2].try_into().unwrap()) as usize;
        let agent_id = String::from_utf8_lossy(&payload[2..2 + id_len]).to_string();

        let type_off = 2 + id_len;
        let type_len = u16::from_le_bytes(
            payload[type_off..type_off+2].try_into().unwrap()) as usize;
        let agent_type = String::from_utf8_lossy(
            &payload[type_off+2..type_off+2+type_len]).to_string();

        let state_off = type_off + 2 + type_len;
        let state_len = u32::from_le_bytes(
            payload[state_off..state_off+4].try_into().unwrap()) as usize;
        let state = payload[state_off+4..state_off+4+state_len].to_vec();

        let mut ctx = AgentContext::new(&agent_id, &agent_type, from_node);
        ctx.state    = state;
        ctx.mobility = AgentMobility::Local;
        ctx.current_node = self.local_node_id;

        println!("[hmaf] received agent '{}' from node {:?}",
            agent_id, &from_node[0..4]);

        self.agents.insert(agent_id.clone(), ctx);
        Ok(agent_id)
    }

    /// Sync agent context to all shadow holders
    pub fn sync_context(&mut self, agent_id: &str, new_state: Vec<u8>) {
        if let Some(ctx) = self.agents.get_mut(agent_id) {
            ctx.state     = new_state;
            ctx.last_sync = Instant::now();
            // In production: broadcast state update to shadow nodes via DSoftBus
            println!("[hmaf] synced context for '{}'", agent_id);
        }
    }

    /// Auto-migration decision: find agents that would run better on another node
    pub fn evaluate_migrations(
        &self,
        available_nodes: &[([u8; 32], u64, u64)], // (node_id, gpu_caps, latency_ns)
    ) -> Vec<(String, [u8; 32], MigrationReason)> {
        let mut suggestions = Vec::new();

        for (agent_id, ctx) in &self.agents {
            if !ctx.is_local() { continue; }

            // If agent needs GPU inference and a remote node has better GPU
            if ctx.capabilities_needed.contains(&"gpu_inference".to_string()) {
                if let Some(&(node, _, latency)) = available_nodes.iter()
                    .filter(|(n, caps, _)| *n != self.local_node_id && *caps > 0)
                    .min_by_key(|(_, _, lat)| *lat)
                {
                    if latency < 2_000_000 {  // < 2ms — worth migrating
                        suggestions.push((
                            agent_id.clone(),
                            node,
                            MigrationReason::ResourceAvailability,
                        ));
                    }
                }
            }
        }

        suggestions
    }

    pub fn agent_count(&self) -> usize { self.agents.len() }

    pub fn migration_count(&self) -> usize { self.migration_log.len() }

    pub fn print_status(&self) {
        println!("[hmaf] {} agents, {} migrations",
            self.agents.len(), self.migration_log.len());
        for (id, ctx) in &self.agents {
            println!("  {} ({}) — {:?}", id, ctx.agent_type, ctx.mobility);
        }
    }
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
