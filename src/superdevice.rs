//! dsoftbus/superdevice.rs
//!
//! Sorabbyngo Super Device — OpenHarmony-inspired device pool
//!
//! Treats all Sorabbyngo nodes as one logical computer.
//! Resources (GPU, CPU, camera, sensors, storage) are pooled
//! and allocated transparently to agents and apps.
//!
//! A byngox agent asking for GPU compute gets the best available
//! GPU across ALL nodes in the pool — local or remote.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use crate::discovery::{Peer, CAP_GPU_COMPUTE, CAP_GPU_RENDER,
                        CAP_CAMERA, CAP_MICROPHONE, CAP_DISPLAY,
                        CAP_SENSOR_IMU, CAP_STORAGE, CAP_AI_INFERENCE};

// ── Virtual device types ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum VirtualDeviceType {
    Gpu,        // compute + render
    Camera,     // image/video capture
    Microphone, // audio capture
    Display,    // screen projection target
    Sensor,     // IMU, GPS, environment
    Storage,    // remote filesystem mount
    Inference,  // AI inference endpoint
}

#[derive(Debug, Clone)]
pub struct VirtualDevice {
    pub id:          u32,
    pub dev_type:    VirtualDeviceType,
    pub owner_node:  [u8; 32],   // which physical node owns this
    pub is_local:    bool,
    pub sysfs_path:  String,     // /sys/bus/sorabbyngo/devices/vdev<id>
    pub latency_ns:  u64,
    pub allocated:   bool,
    pub alloc_to:    Option<String>,  // agent name that has it
}

impl VirtualDevice {
    pub fn new(id: u32, dev_type: VirtualDeviceType,
               owner: [u8; 32], local: bool, latency: u64) -> Self {
        let type_str = match &dev_type {
            VirtualDeviceType::Gpu       => "vgpu",
            VirtualDeviceType::Camera    => "vcam",
            VirtualDeviceType::Microphone=> "vmic",
            VirtualDeviceType::Display   => "vdisplay",
            VirtualDeviceType::Sensor    => "vsensor",
            VirtualDeviceType::Storage   => "vstorage",
            VirtualDeviceType::Inference => "vinference",
        };
        Self {
            id,
            sysfs_path: format!("/sys/bus/sorabbyngo/devices/{}{}", type_str, id),
            dev_type,
            owner_node: owner,
            is_local:   local,
            latency_ns: latency,
            allocated:  false,
            alloc_to:   None,
        }
    }

    pub fn allocate(&mut self, agent: &str) -> bool {
        if self.allocated { return false; }
        self.allocated = true;
        self.alloc_to  = Some(agent.to_string());
        true
    }

    pub fn release(&mut self) {
        self.allocated = false;
        self.alloc_to  = None;
    }
}

// ── Super Device pool ──────────────────────────────────────────────────────

pub struct SuperDevice {
    local_node_id: [u8; 32],
    devices:       Arc<RwLock<Vec<VirtualDevice>>>,
    next_id:       u32,
}

impl SuperDevice {
    pub fn new(local_node_id: [u8; 32]) -> Self {
        Self {
            local_node_id,
            devices:   Arc::new(RwLock::new(Vec::new())),
            next_id:   1,
        }
    }

    /// Register local hardware as virtual devices in the pool
    pub fn register_local_hardware(&mut self) {
        // Detect local GPU
        if std::path::Path::new("/dev/dri/card0").exists() {
            self.add_device(VirtualDeviceType::Gpu, true, 0);
            println!("[superdevice] registered local GPU");
        }

        // Detect local camera
        if std::path::Path::new("/dev/video0").exists() {
            self.add_device(VirtualDeviceType::Camera, true, 0);
            println!("[superdevice] registered local camera");
        }

        // Always register local inference endpoint
        self.add_device(VirtualDeviceType::Inference, true, 0);
        println!("[superdevice] registered local inference endpoint");

        // Always register local storage
        self.add_device(VirtualDeviceType::Storage, true, 0);
    }

    /// Register a remote peer's capabilities as virtual devices
    pub fn register_peer(&mut self, peer: &Peer) {
        let latency = peer.latency_ns;
        let node_id = peer.node_id;

        if peer.has_cap(CAP_GPU_COMPUTE) || peer.has_cap(CAP_GPU_RENDER) {
            self.add_remote_device(VirtualDeviceType::Gpu, node_id, latency);
            println!("[superdevice] registered remote GPU from {}",
                &peer.node_id_hex()[..8]);
        }
        if peer.has_cap(CAP_CAMERA) {
            self.add_remote_device(VirtualDeviceType::Camera, node_id, latency);
        }
        if peer.has_cap(CAP_MICROPHONE) {
            self.add_remote_device(VirtualDeviceType::Microphone, node_id, latency);
        }
        if peer.has_cap(CAP_DISPLAY) {
            self.add_remote_device(VirtualDeviceType::Display, node_id, latency);
        }
        if peer.has_cap(CAP_SENSOR_IMU) {
            self.add_remote_device(VirtualDeviceType::Sensor, node_id, latency);
        }
        if peer.has_cap(CAP_AI_INFERENCE) {
            self.add_remote_device(VirtualDeviceType::Inference, node_id, latency);
        }
    }

    /// Allocate the best available device of a given type to an agent.
    /// Prefers local, then lowest latency remote.
    pub fn allocate_best(
        &mut self,
        dev_type: &VirtualDeviceType,
        agent:    &str,
    ) -> Option<u32> {
        let mut devs = self.devices.write().unwrap();

        // Find best: local first, then lowest latency, then unallocated
        let best_idx = devs.iter()
            .enumerate()
            .filter(|(_, d)| &d.dev_type == dev_type && !d.allocated)
            .min_by_key(|(_, d)| {
                if d.is_local { 0u64 } else { d.latency_ns + 1 }
            })
            .map(|(i, _)| i);

        if let Some(idx) = best_idx {
            devs[idx].allocate(agent);
            let id = devs[idx].id;
            println!("[superdevice] allocated {} (id={}) to agent '{}'",
                devs[idx].sysfs_path, id, agent);
            Some(id)
        } else {
            println!("[superdevice] no available {:?} for agent '{}'",
                dev_type, agent);
            None
        }
    }

    pub fn release(&mut self, device_id: u32) {
        let mut devs = self.devices.write().unwrap();
        if let Some(dev) = devs.iter_mut().find(|d| d.id == device_id) {
            println!("[superdevice] released {} (id={})",
                dev.sysfs_path, device_id);
            dev.release();
        }
    }

    pub fn list_available(&self) -> Vec<(u32, String, bool, u64)> {
        self.devices.read().unwrap()
            .iter()
            .filter(|d| !d.allocated)
            .map(|d| (d.id, d.sysfs_path.clone(), d.is_local, d.latency_ns))
            .collect()
    }

    pub fn pool_summary(&self) -> String {
        let devs = self.devices.read().unwrap();
        let total    = devs.len();
        let free     = devs.iter().filter(|d| !d.allocated).count();
        let local    = devs.iter().filter(|d| d.is_local).count();
        let remote   = total - local;
        format!("pool: {} devices ({} local, {} remote), {} free",
            total, local, remote, free)
    }

    fn add_device(&mut self, dev_type: VirtualDeviceType, local: bool, latency: u64) {
        let id  = self.next_id;
        self.next_id += 1;
        let dev = VirtualDevice::new(id, dev_type, self.local_node_id, local, latency);
        self.devices.write().unwrap().push(dev);
    }

    fn add_remote_device(&mut self, dev_type: VirtualDeviceType,
                         node: [u8; 32], latency: u64) {
        let id  = self.next_id;
        self.next_id += 1;
        let dev = VirtualDevice::new(id, dev_type, node, false, latency);
        self.devices.write().unwrap().push(dev);
    }
}
