//! # 2.9" 4-color identification: GDEY029F51H init
//!
//! Runs the Good Display GDEY029F51H demo init (JD79667, 384x168, Good Display product 464) on whatever 2.9" JD7966x
//! panel is attached, sends a test frame sized for 168 x 384 and refreshes once.
//!
//! This exists to tell two products apart that can share a rear sticker.
//! `GDEY029F51H` (JD79667, 168 x 384) and `GDEY029F52` (JD79661, 128 x 296) are different
//! Good Display products, and the `R70H` read (`03 02 01` in both datasheets) cannot separate
//! them. Run both `epd_diag_029_f52` and `epd_diag_029_f51h` on the same panel, with a few
//! minutes between them, and see which one draws the test frame cleanly.
//!
//! A correct match shows a black border hugging all four edges, a red square in the top-left
//! corner, and black, yellow and red bands across the middle third. A wrong match shows a
//! sheared or shifted picture, a border that is cut off, or noise.
//!
//! One refresh per run. Do not put this in a loop.
//!
//! ## Hardware
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** ePaper Driver Board for XIAO (24-pin FPC)
//!
//! ```bash
//! cargo run --release --example epd_diag_029_f51h
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
            name: "GDEY029F51H",
            source_bits: 168,
            gate_bits: 384,
            psr0: 0x0F,
            ticks: 0,
        },
        peripherals,
    )
}
