use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use std::net::IpAddr;

pub const CAP_GPU_COMPUTE:  u64 = 1 << 0;
pub const CAP_GPU_RENDER:   u64 = 1 << 1;
pub const CAP_CAMERA:       u64 = 1 << 2;
pub const CAP_MICROPHONE:   u64 = 1 << 3;
pub const CAP_DISPLAY:      u64 = 1 << 4;
pub const CAP_SENSOR_IMU:   u64 = 1 << 5;
pub const CAP_STORAGE:      u64 = 1 << 6;
pub const CAP_AI_INFERENCE: u64 = 1 << 7;

#[derive(Debug, Clone)]
pub struct Peer {
    pub node_id:      [u8; 32],
    pub addr:         IpAddr,
    pub capabilities: u64,
    pub last_seen:    Instant,
    pub latency_ns:   u64,
    pub pin_verified: bool,
}

impl Peer {
    pub fn has_cap(&self, cap: u64) -> bool { self.capabilities & cap != 0 }
    pub fn node_id_hex(&self) -> String {
        self.node_id.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum VirtualDeviceType {
    Gpu, Camera, Microphone, Display, Sensor, Storage, Inference,
}

#[derive(Debug, Clone)]
pub struct VirtualDevice {
    pub id:         u32,
    pub dev_type:   VirtualDeviceType,
    pub owner_node: [u8; 32],
    pub is_local:   bool,
    pub sysfs_path: String,
    pub latency_ns: u64,
    pub allocated:  bool,
    pub alloc_to:   Option<String>,
}

impl VirtualDevice {
    pub fn new(id: u32, dev_type: VirtualDeviceType,
               owner: [u8; 32], local: bool, latency: u64) -> Self {
        let type_str = match &dev_type {
            VirtualDeviceType::Gpu        => "vgpu",
            VirtualDeviceType::Camera     => "vcam",
            VirtualDeviceType::Microphone => "vmic",
            VirtualDeviceType::Display    => "vdisplay",
            VirtualDeviceType::Sensor     => "vsensor",
            VirtualDeviceType::Storage    => "vstorage",
            VirtualDeviceType::Inference  => "vinference",
        };
        Self {
            id,
            sysfs_path: format!("/sys/bus/sorabbyngo/devices/{}{}", type_str, id),
            dev_type, owner_node: owner, is_local: local,
            latency_ns: latency, allocated: false, alloc_to: None,
        }
    }

    pub fn allocate(&mut self, agent: &str) -> bool {
        if self.allocated { return false; }
        self.allocated = true;
        self.alloc_to  = Some(agent.to_string());
        true
    }

    pub fn release(&mut self) { self.allocated = false; self.alloc_to = None; }
}

pub struct SuperDevice {
    local_node_id: [u8; 32],
    devices:       Arc<RwLock<Vec<VirtualDevice>>>,
    next_id:       u32,
}

impl SuperDevice {
    pub fn new(local_node_id: [u8; 32]) -> Self {
        Self { local_node_id, devices: Arc::new(RwLock::new(Vec::new())), next_id: 1 }
    }

    pub fn register_local_hardware(&mut self) {
        if std::path::Path::new("/dev/dri/card0").exists() {
            self.add_device(VirtualDeviceType::Gpu, true, 0);
            println!("[superdevice] registered local GPU");
        }
        if std::path::Path::new("/dev/video0").exists() {
            self.add_device(VirtualDeviceType::Camera, true, 0);
            println!("[superdevice] registered local camera");
        }
        self.add_device(VirtualDeviceType::Inference, true, 0);
        self.add_device(VirtualDeviceType::Storage,   true, 0);
        println!("[superdevice] registered local inference + storage");
    }

    pub fn register_peer(&mut self, peer: &Peer) {
        let (l, n) = (peer.latency_ns, peer.node_id);
        if peer.has_cap(CAP_GPU_COMPUTE) || peer.has_cap(CAP_GPU_RENDER) {
            self.add_remote(VirtualDeviceType::Gpu, n, l);
        }
        if peer.has_cap(CAP_CAMERA)       { self.add_remote(VirtualDeviceType::Camera,     n, l); }
        if peer.has_cap(CAP_MICROPHONE)   { self.add_remote(VirtualDeviceType::Microphone,  n, l); }
        if peer.has_cap(CAP_DISPLAY)      { self.add_remote(VirtualDeviceType::Display,     n, l); }
        if peer.has_cap(CAP_SENSOR_IMU)   { self.add_remote(VirtualDeviceType::Sensor,      n, l); }
        if peer.has_cap(CAP_AI_INFERENCE) { self.add_remote(VirtualDeviceType::Inference,   n, l); }
        println!("[superdevice] peer {} caps={:#x}", peer.node_id_hex(), peer.capabilities);
    }

    pub fn allocate_best(&mut self, dev_type: &VirtualDeviceType, agent: &str) -> Option<u32> {
        let mut devs = self.devices.write().unwrap();
        let idx = devs.iter().enumerate()
            .filter(|(_, d)| &d.dev_type == dev_type && !d.allocated)
            .min_by_key(|(_, d)| if d.is_local { 0u64 } else { d.latency_ns + 1 })
            .map(|(i, _)| i)?;
        devs[idx].allocate(agent);
        let id = devs[idx].id;
        println!("[superdevice] allocated {} to '{}'", devs[idx].sysfs_path, agent);
        Some(id)
    }

    pub fn release(&mut self, device_id: u32) {
        let mut devs = self.devices.write().unwrap();
        if let Some(d) = devs.iter_mut().find(|d| d.id == device_id) {
            d.release();
            println!("[superdevice] released id={}", device_id);
        }
    }

    pub fn pool_summary(&self) -> String {
        let devs  = self.devices.read().unwrap();
        let total = devs.len();
        let free  = devs.iter().filter(|d| !d.allocated).count();
        let local = devs.iter().filter(|d| d.is_local).count();
        format!("pool: {} devices ({} local, {} remote), {} free",
            total, local, total - local, free)
    }

    fn add_device(&mut self, t: VirtualDeviceType, local: bool, lat: u64) {
        let id = self.next_id; self.next_id += 1;
        self.devices.write().unwrap().push(
            VirtualDevice::new(id, t, self.local_node_id, local, lat));
    }

    fn add_remote(&mut self, t: VirtualDeviceType, node: [u8; 32], lat: u64) {
        let id = self.next_id; self.next_id += 1;
        self.devices.write().unwrap().push(
            VirtualDevice::new(id, t, node, false, lat));
    }
}
