# Sorabbyngo Super Device

> **Treat every Sorabbyngo node as one logical computer.**

Inspired by OpenHarmony's distributed device virtualization model, Super Device pools hardware resources across all connected Sorabbyngo nodes — laptops, phones, servers, IoT devices, edge nodes — and presents them as a single unified machine to agents, apps, and users.

---

## What it does

A byngox agent that needs GPU compute gets the **best available GPU across all nodes** — local or remote — transparently, without knowing which physical machine it's running on.

A phone's camera becomes `/sys/bus/sorabbyngo/devices/vcam0` on your laptop.  
A server's GPU becomes `/sys/bus/sorabbyngo/devices/vgpu0` on your watch.  
A tablet's screen becomes a virtual display for any app on any node.

This is the foundation of the **Agentic Internet** — 9 billion people connected to 900 billion agents, all sharing hardware fluidly across device boundaries.

---

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                   sorabbyngo-superdevice                │
│                                                         │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐ │
│  │ SuperDevice │  │    HMAF     │  │   StarShield    │ │
│  │    Pool     │  │Orchestrator │  │   Zero Trust    │ │
│  │             │  │             │  │                 │ │
│  │ vgpu0       │  │ 8 agents    │  │ NodeID-rooted   │ │
│  │ vcam0       │  │ cross-node  │  │ PIN auth        │ │
│  │ vsensor0    │  │ migration   │  │ continuous      │ │
│  │ vstorage0   │  │ context sync│  │ re-verification │ │
│  └──────┬──────┘  └──────┬──────┘  └────────┬────────┘ │
│         └────────────────┴──────────────────┘          │
│                          │                              │
│              ┌───────────▼───────────┐                  │
│              │   DSoftBus transport  │                  │
│              │   (byngonet)          │                  │
│              │   4ms latency         │                  │
│              │   PIN-authenticated   │                  │
│              └───────────┬───────────┘                  │
└──────────────────────────┼──────────────────────────────┘
                           │
        ┌──────────────────┼──────────────────┐
        ▼                  ▼                  ▼
   Node A (laptop)    Node B (phone)    Node C (server)
   GPU, display       camera, IMU       GPU, storage
```

---

## Components

### `superdevice.rs` — Hardware pool
Discovers, registers, and allocates virtual devices across all nodes.

| Virtual Device | Kernel path | Source |
|----------------|-------------|--------|
| `VirtualDeviceType::Gpu` | `/sys/bus/sorabbyngo/devices/vgpu<n>` | Any node with GPU |
| `VirtualDeviceType::Camera` | `/sys/bus/sorabbyngo/devices/vcam<n>` | Phone, webcam nodes |
| `VirtualDeviceType::Microphone` | `/sys/bus/sorabbyngo/devices/vmic<n>` | Any audio-capable node |
| `VirtualDeviceType::Display` | `/sys/bus/sorabbyngo/devices/vdisplay<n>` | Any screen node |
| `VirtualDeviceType::Sensor` | `/sys/bus/sorabbyngo/devices/vsensor<n>` | IMU, GPS, env sensors |
| `VirtualDeviceType::Storage` | `/sys/bus/sorabbyngo/devices/vstorage<n>` | Remote filesystems |
| `VirtualDeviceType::Inference` | `/sys/bus/sorabbyngo/devices/vinference<n>` | AI inference endpoints |

**Allocation policy:** local-first, then lowest-latency remote. An agent always gets the fastest available device.

---

### `hmaf.rs` — Cross-device agent framework
Extends byngox's 8-agent swarm to run across node boundaries.

**Agent mobility states:**

| State | Meaning |
|-------|---------|
| `Local` | Running on this node |
| `Migrating { to_node }` | Being transferred — state serialized, in flight |
| `Remote { on_node }` | Running on another node, context accessible here |
| `Shadow { of_node }` | Lightweight context mirror of a remote agent |

**Migration triggers:**
- User does a PUSH gesture (Iron Man snap transfer)
- Resource optimizer finds a better GPU/CPU on another node
- Load balancer detects this node is overloaded
- Power saver consolidates agents to fewer nodes

**The 8 byngox agents and where they run best:**

| Agent | Preferred node | Why |
|-------|---------------|-----|
| `health-agent` | Watch / phone | Closest to sensors |
| `drug-agent` | Server | Heavy data processing |
| `context-agent` | Laptop | Closest to user |
| `network-agent` | Edge node | Closest to network |
| `learning-agent` | Server | GPU-intensive training |
| `population-agent` | Server | Large dataset analysis |
| `security-agent` | Dedicated sentinel node | Isolated execution |
| `inference-agent` | Best GPU node | Dynamic allocation |

---

### `starshield.rs` — Zero Trust security
Every device operation is authenticated. No implicit trust — even between nodes on the same LAN.

**Trust levels:**

| Level | How achieved | What it unlocks |
|-------|-------------|-----------------|
| `Unknown` | Just appeared on network | Discovery announce only |
| `Verified` | PIN handshake confirmed | Channel open, file transfer |
| `Attested` | Hardware TPM attestation | App state transfer, GPU access |
| `Trusted` | Long-term user approval | Agent migration, camera access |

**Zero Trust policy (default):**

```
discovery.announce  → Unknown    (anyone can announce)
channel.open        → Verified   (PIN required)
snap.file           → Verified   (PIN required)
snap.app_state      → Attested   (hardware proof required)
hmaf.migrate        → Trusted    (explicit user trust)
superdevice.gpu     → Attested   (hardware proof required)
superdevice.camera  → Trusted    (explicit user trust)
```

**Re-verification:** Every node must re-verify within 5 minutes or access is revoked automatically.

---

## Integration map

```
sorabbyngo-superdevice
    │
    ├── byngonet ──────── DSoftBus transport, auto-discovery, 4ms latency
    ├── byngox ─────────── 8-agent swarm, HMAF migration targets
    ├── capd ───────────── capability tokens for each virtual device access
    ├── domain-mode ────── GPU mode changes on domain switch (gaming→exclusive)
    ├── hal ────────────── hardware event feed, device plug/unplug detection
    ├── sorabbyngo-security─ StarShield policy enforcement, audit log
    └── wayland-shell ──── gesture events (PUSH/PULL/SWIPE) trigger migrations
```

---

## Quick start

```bash
# Build
cargo build --release

# Run (as root — needs sysfs access for device registration)
sudo ./target/release/superdevice

# Check the device pool
cat /sys/kernel/sorabbyngo/superdevice/pool

# Allocate a GPU to an agent manually
echo "inference-agent" > /sys/kernel/sorabbyngo/superdevice/allocate/gpu

# List available virtual devices
cat /sys/kernel/sorabbyngo/superdevice/devices
```

---

## Capability node bitmask

When a node announces itself via DSoftBus discovery, it broadcasts a `u64` capability bitmask:

```rust
pub const CAP_GPU_COMPUTE:  u64 = 1 << 0;
pub const CAP_GPU_RENDER:   u64 = 1 << 1;
pub const CAP_CAMERA:       u64 = 1 << 2;
pub const CAP_MICROPHONE:   u64 = 1 << 3;
pub const CAP_DISPLAY:      u64 = 1 << 4;
pub const CAP_SENSOR_IMU:   u64 = 1 << 5;
pub const CAP_STORAGE:      u64 = 1 << 6;
pub const CAP_AI_INFERENCE: u64 = 1 << 7;
```

A laptop with GPU + display + storage would broadcast `0b01010011 = 0x53`.  
The Super Device pool reads this and registers the appropriate virtual devices automatically.

---

## Roadmap

- [ ] Kernel driver: expose virtual devices under `/sys/bus/sorabbyngo/`
- [ ] RDMA transport for GPU memory sharing (sub-1ms latency)
- [ ] Live agent migration without state serialization (memory page transfer)
- [ ] Hardware attestation via TPM 2.0
- [ ] Power-aware allocation (prefer plugged-in nodes for heavy workloads)
- [ ] Bandwidth-aware migration (don't migrate if link is congested)
- [ ] Integration with sorabbyngofs for distributed file access
- [ ] wayland-shell gesture API for user-triggered migrations

---

## Related repos

| Repo | Role |
|------|------|
| [`byngonet`](https://github.com/sorabbyngo-os/byngonet) | DSoftBus transport + discovery |
| [`byngox`](https://github.com/sorabbyngo-os/byngox) | 8-agent swarm engine |
| [`sorabbyngo-security`](https://github.com/sorabbyngo-os/sorabbyngo-security) | StarShield + PKI |
| [`capd`](https://github.com/sorabbyngo-os/capd) | Capability token daemon |
| [`hal`](https://github.com/sorabbyngo-os/hal) | Hardware abstraction + sensor bridge |
| [`wayland-shell`](https://github.com/sorabbyngo-os/wayland-shell) | Gesture recognition + HUD |

---

*Sorabbyngo OS — Africa-first AI Operating System*  
*Super Device: one body, many minds, all hardware shared.*
