//! # 2.9" 4-color GDEY029F52: PSR `8F 29` test
//!
//! Same as `epd_diag_029_f52`, which sends the vendor's PSR `0F 29` and drew cleanly twice,
//! but with PSR first byte `0x8F` (RES=10, 128x250, the byte the epdsi `Jd79661` variant sends).
//!
//! Question: does PSR matter on this panel, or does TRES set the geometry on its own? If the
//! frame draws the same as the `0F` run (black border on all four edges, red square at the
//! top-left, black, yellow and red bands across the middle third), the existing `Jd79661`
//! variant can drive the F52 as it is.
//!
//! The frame carries 1 small black tick along the long edge near the
//! origin corner, so a photo shows which run made it (1 here, none in the `0F` run).
//!
//! One refresh per run. Do not put this in a loop.
//!
//! ```bash
//! cargo run --release --example epd_diag_029_f52_psr8f
//! ```

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::main;

#[path = "common/diag_029.rs"]
mod diag_029;

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    diag_029::run(
        diag_029::Profile {
            name: "GDEY029F52 with PSR 8F",
            source_bits: 128,
            gate_bits: 296,
            psr0: 0x8F,
            ticks: 1,
        },
        peripherals,
    )
}
