#![no_std]
#![no_main]

use core::time::Duration;

use uefi::boot::stall;
use uefi::prelude::*;
#[cfg(debug_assertions)]
use uefi::println;

const PCI_ADDR: u16 = 0xCF8;
const PCI_DATA: u16 = 0xCFC;

const BUS_UPSTREAM: u8 = 0x08;
const BUS_DOWNSTREAM: u8 = 0x09;
const BUS_GPU: u8 = 0x0A;

const MEM_BASE: u16 = 0xE020;
const MEM_LIMIT_ROOT: u16 = 0xE050; // 0xE050FFFF for root port (includes switch BAR)
const MEM_LIMIT_CHILD: u16 = 0xE03F; // 0xE03FFFFF for downstream bridges

const GPU_BAR0_LO: u32 = 0x3000_000C; // 0x1030000000, 64-bit pref
const GPU_BAR0_HI: u32 = 0x0000_0010;
const GPU_BAR2_LO: u32 = 0x4000_000C; // 0x1040000000, 64-bit pref  
const GPU_BAR2_HI: u32 = 0x0000_0010;
const GPU_BAR4: u32 = 0x0000_2001; // I/O @ 0x2000
const GPU_BAR5: u32 = 0xE020_0000; // MMIO @ 0xE0200000
const GPU_ROM: u32 = 0xE030_0001; // ROM @ 0xE0300000, enabled

const AUDIO_BAR0: u32 = 0xE032_0000; // MMIO @ 0xE0320000

const SWITCH_BAR0: u32 = 0xE040_0000; // MMIO @ 0xE0400000

const PREF_BASE_LO: u16 = 0x3001;
const PREF_LIMIT_LO: u16 = 0x4011;
const PREF_BASE_HI: u32 = 0x10;
const PREF_LIMIT_HI: u32 = 0x10;

const IO_BASE: u8 = 0x20;
const IO_LIMIT: u8 = 0x20;

#[inline(always)]
fn pci_addr(bus: u8, dev: u8, func: u8, reg: u16) -> u32 {
    0x8000_0000
        | ((bus as u32) << 16)
        | ((dev as u32) << 11)
        | ((func as u32) << 8)
        | ((reg as u32) & 0xFC)
}

#[inline(always)]
fn out32(port: u16, val: u32) {
    unsafe {
        core::arch::asm!("out dx, eax", in("dx") port, in("eax") val);
    }
}

#[inline(always)]
fn in32(port: u16) -> u32 {
    unsafe {
        let mut val: u32;
        core::arch::asm!("in eax, dx", out("eax") val, in("dx") port);
        val
    }
}

fn r32(bus: u8, dev: u8, func: u8, reg: u16) -> u32 {
    out32(PCI_ADDR, pci_addr(bus, dev, func, reg));
    in32(PCI_DATA)
}

fn r16(bus: u8, dev: u8, func: u8, reg: u16) -> u16 {
    ((r32(bus, dev, func, reg & 0xFC) >> ((reg & 2) * 8)) & 0xFFFF) as u16
}

fn r8(bus: u8, dev: u8, func: u8, reg: u16) -> u8 {
    out32(PCI_ADDR, pci_addr(bus, dev, func, reg & 0xFC));
    ((in32(PCI_DATA) >> ((reg & 3) * 8)) & 0xFF) as u8
}

fn w32(bus: u8, dev: u8, func: u8, reg: u16, val: u32) {
    out32(PCI_ADDR, pci_addr(bus, dev, func, reg));
    out32(PCI_DATA, val);
}

fn w16(bus: u8, dev: u8, func: u8, reg: u16, val: u16) {
    out32(PCI_ADDR, pci_addr(bus, dev, func, reg & 0xFC));
    let o = in32(PCI_DATA);
    let shift = (reg & 2) * 8;
    let mask = !(0xFFFF << shift);
    out32(PCI_DATA, (o & mask) | ((val as u32) << shift));
}

fn w8(bus: u8, dev: u8, func: u8, reg: u16, val: u8) {
    out32(PCI_ADDR, pci_addr(bus, dev, func, reg & 0xFC));
    let o = in32(PCI_DATA);
    let shift = (reg & 3) * 8;
    let mask = !(0xFF << shift);
    out32(PCI_DATA, (o & mask) | ((val as u32) << shift));
}

fn enable_cmd(bus: u8, dev: u8, func: u8) {
    let cmd = r16(bus, dev, func, 0x04);
    w16(bus, dev, func, 0x04, cmd | 0x0007); // IO | MEM | BUS MASTER
}

fn find_pcie_cap(bus: u8, dev: u8, func: u8) -> Option<u8> {
    let status = r16(bus, dev, func, 0x06);
    if (status & 0x10) == 0 {
        return None;
    }
    let mut cap_ptr = r8(bus, dev, func, 0x34) & 0xFC;
    while cap_ptr != 0 {
        let cap_id = r8(bus, dev, func, cap_ptr as u16);
        if cap_id == 0x10 {
            return Some(cap_ptr);
        }
        cap_ptr = r8(bus, dev, func, (cap_ptr + 1) as u16) & 0xFC;
    }
    None
}

#[cfg(debug_assertions)]
fn debug_scan_all_buses() {
    println!("=== PCI Bus Scan ===");
    for bus in 0..=255u8 {
        for dev in 0..32u8 {
            for func in 0..8u8 {
                let vendor_device = r32(bus, dev, func, 0x00);
                let vendor_id = (vendor_device & 0xFFFF) as u16;
                if vendor_id != 0xFFFF && vendor_id != 0x0000 {
                    let device_id = (vendor_device >> 16) as u16;
                    let aspm_str = if let Some(pcie_cap) = find_pcie_cap(bus, dev, func) {
                        let link_ctrl = r16(bus, dev, func, (pcie_cap + 0x10) as u16);
                        let enabled = link_ctrl & 0x3;
                        match enabled {
                            0 => "ASPM:off",
                            1 => "ASPM:L0s",
                            2 => "ASPM:L1",
                            3 => "ASPM:L0s+L1",
                            _ => "ASPM:?",
                        }
                    } else {
                        "ASPM:N/A"
                    };
                    println!(
                        "{:02X}:{:02X}.{} {:04X}:{:04X} {}",
                        bus, dev, func, vendor_id, device_id, aspm_str
                    );
                }
                if func == 0 {
                    let header = r8(bus, dev, 0, 0x0E);
                    if (header & 0x80) == 0 {
                        break;
                    }
                }
            }
        }
    }
    println!("=== End Scan ===");
}

fn disable_aspm_root_port() {
    if let Some(pcie_cap) = find_pcie_cap(0x00, 0x02, 0x01) {
        let link_ctrl = r16(0x00, 0x02, 0x01, (pcie_cap + 0x10) as u16);
        w16(
            0x00,
            0x02,
            0x01,
            (pcie_cap + 0x10) as u16,
            link_ctrl & !0x03,
        );
    }
}

fn config_bridge_root() {
    // Root port 00:02.1 - larger window to include switch BAR
    w8(0x00, 0x02, 0x01, 0x18, 0x00); // Primary = 0
    w8(0x00, 0x02, 0x01, 0x19, BUS_UPSTREAM); // Secondary = 8
    w8(0x00, 0x02, 0x01, 0x1A, BUS_GPU); // Subordinate = 10

    w8(0x00, 0x02, 0x01, 0x1C, IO_BASE);
    w8(0x00, 0x02, 0x01, 0x1D, IO_LIMIT);

    w16(0x00, 0x02, 0x01, 0x20, MEM_BASE);
    w16(0x00, 0x02, 0x01, 0x22, MEM_LIMIT_ROOT); // Larger limit

    w16(0x00, 0x02, 0x01, 0x24, PREF_BASE_LO);
    w16(0x00, 0x02, 0x01, 0x26, PREF_LIMIT_LO);
    w32(0x00, 0x02, 0x01, 0x28, PREF_BASE_HI);
    w32(0x00, 0x02, 0x01, 0x2C, PREF_LIMIT_HI);

    enable_cmd(0x00, 0x02, 0x01);
}

fn config_bridge_upstream() {
    // Upstream switch 08:00.0 - window for downstream only
    if (r32(BUS_UPSTREAM, 0, 0, 0x00) & 0xFFFF) != 0x1002 {
        return;
    }

    // Assign switch's own BAR first
    w32(BUS_UPSTREAM, 0, 0, 0x10, SWITCH_BAR0);

    w8(BUS_UPSTREAM, 0, 0, 0x18, BUS_UPSTREAM);
    w8(BUS_UPSTREAM, 0, 0, 0x19, BUS_DOWNSTREAM);
    w8(BUS_UPSTREAM, 0, 0, 0x1A, BUS_GPU);

    w8(BUS_UPSTREAM, 0, 0, 0x1C, IO_BASE);
    w8(BUS_UPSTREAM, 0, 0, 0x1D, IO_LIMIT);

    w16(BUS_UPSTREAM, 0, 0, 0x20, MEM_BASE);
    w16(BUS_UPSTREAM, 0, 0, 0x22, MEM_LIMIT_CHILD); // Smaller limit, excludes own BAR

    w16(BUS_UPSTREAM, 0, 0, 0x24, PREF_BASE_LO);
    w16(BUS_UPSTREAM, 0, 0, 0x26, PREF_LIMIT_LO);
    w32(BUS_UPSTREAM, 0, 0, 0x28, PREF_BASE_HI);
    w32(BUS_UPSTREAM, 0, 0, 0x2C, PREF_LIMIT_HI);

    enable_cmd(BUS_UPSTREAM, 0, 0);
}

fn config_bridge_downstream() {
    // Downstream switch 09:00.0
    if (r32(BUS_DOWNSTREAM, 0, 0, 0x00) & 0xFFFF) != 0x1002 {
        return;
    }

    w8(BUS_DOWNSTREAM, 0, 0, 0x18, BUS_DOWNSTREAM);
    w8(BUS_DOWNSTREAM, 0, 0, 0x19, BUS_GPU);
    w8(BUS_DOWNSTREAM, 0, 0, 0x1A, BUS_GPU);

    w8(BUS_DOWNSTREAM, 0, 0, 0x1C, IO_BASE);
    w8(BUS_DOWNSTREAM, 0, 0, 0x1D, IO_LIMIT);

    w16(BUS_DOWNSTREAM, 0, 0, 0x20, MEM_BASE);
    w16(BUS_DOWNSTREAM, 0, 0, 0x22, MEM_LIMIT_CHILD); // Same as upstream child window

    w16(BUS_DOWNSTREAM, 0, 0, 0x24, PREF_BASE_LO);
    w16(BUS_DOWNSTREAM, 0, 0, 0x26, PREF_LIMIT_LO);
    w32(BUS_DOWNSTREAM, 0, 0, 0x28, PREF_BASE_HI);
    w32(BUS_DOWNSTREAM, 0, 0, 0x2C, PREF_LIMIT_HI);

    enable_cmd(BUS_DOWNSTREAM, 0, 0);
}

fn config_bridges() {
    config_bridge_root();
    stall(Duration::from_millis(20));

    config_bridge_upstream();
    stall(Duration::from_millis(20));

    config_bridge_downstream();
    stall(Duration::from_millis(20));
}

fn config_gpu_bars() {
    let bus = BUS_GPU;

    // BAR0: 256M @ 0x1030000000 (64-bit pref)
    w32(bus, 0, 0, 0x10, GPU_BAR0_LO);
    w32(bus, 0, 0, 0x14, GPU_BAR0_HI);

    // BAR2: 2M @ 0x1040000000 (64-bit pref)
    w32(bus, 0, 0, 0x18, GPU_BAR2_LO);
    w32(bus, 0, 0, 0x1C, GPU_BAR2_HI);

    // BAR4: I/O @ 0x2000
    w32(bus, 0, 0, 0x20, GPU_BAR4);

    // BAR5: 1M MMIO @ 0xE0200000
    w32(bus, 0, 0, 0x24, GPU_BAR5);

    // Expansion ROM @ 0xE0300000 (enable)
    w32(bus, 0, 0, 0x30, GPU_ROM);

    enable_cmd(bus, 0, 0);

    //Audio function (01)
    w32(bus, 0, 1, 0x10, AUDIO_BAR0);
    let cmd = r16(bus, 0, 1, 0x04);
    w16(bus, 0, 1, 0x04, cmd | 0x0006); // MEM + BUS_MASTER

    enable_cmd(bus, 0, 1);
    stall(Duration::from_millis(20));
}

fn config_upstream_switch() {
    // Assign BAR0 to upstream switch
    w32(BUS_UPSTREAM, 0, 0, 0x10, SWITCH_BAR0);
    stall(Duration::from_millis(20));
}

#[entry]
fn main() -> Status {
    uefi::helpers::init().unwrap();

    config_bridges();
    config_upstream_switch();
    config_gpu_bars();

    disable_aspm_root_port();

    #[cfg(debug_assertions)]
    {
        debug_scan_all_buses();
        stall(Duration::from_secs(7));
    }
    Status::SUCCESS
}
