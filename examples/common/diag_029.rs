//! Shared body of the two `epd_diag_029_*` identification examples.
//!
//! Runs Good Display's plain `EPD_init()` for the 2.9" JD7966x glass, writes a test frame,
//! refreshes once and puts the panel to sleep. The two Good Display demos
//! (`GDEY029F51H` and `GDEY029F52`) send identical bytes except for the `TRES` resolution
//! command, so the only thing a [`Profile`] changes is the raster size.

use embedded_hal::delay::DelayNs;
use embedded_hal_bus::spi::ExclusiveDevice;
use epdsi::prelude::*;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::peripherals::Peripherals;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::Rate;

/// Raster geometry that `TRES` announces to the IC.
pub struct Profile {
    pub name: &'static str,
    /// `Source_BITS` in the vendor demo: pixels along a RAM row.
    pub source_bits: u16,
    /// `Gate_BITS` in the vendor demo: number of RAM rows.
    pub gate_bits: u16,
    /// First PSR (`0x00`) data byte. The vendor demos send `0x0F`; bits 7-6 are `RES[1:0]`.
    pub psr0: u8,
    /// Number of small black ticks drawn near the origin corner, so a photo shows which run
    /// made it. A stale picture from an earlier run has a different tick count.
    pub ticks: u32,
}

/// Large enough for the bigger raster, 168 x 384 at 2 bits per pixel.
static mut FRAME_BUF: [u8; 16128] = [0x55; 16128];

/// Vendor colour codes: black 00, white 01, yellow 10, red 11.
const BLACK: u8 = 0b00;
const WHITE: u8 = 0b01;
const YELLOW: u8 = 0b10;
const RED: u8 = 0b11;

fn pixel(profile: &Profile, x: u32, y: u32) -> u8 {
    let w = profile.source_bits as u32;
    let h = profile.gate_bits as u32;
    if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
        return BLACK;
    }
    // Red square in the top-left corner shows where RAM origin lands.
    if x < 24 && y < 24 {
        return RED;
    }
    // Run marker: `ticks` black blocks, 10 px tall with 4 px gaps, stacked along the long edge.
    if profile.ticks > 0 && (4..12).contains(&x) {
        let slot = y.wrapping_sub(40) / 14;
        if y >= 40 && slot < profile.ticks && (y - 40) % 14 < 10 {
            return BLACK;
        }
    }
    // Three bands across the middle third: black, yellow, red.
    if y >= h / 3 && y < 2 * h / 3 {
        return match x * 3 / w {
            0 => BLACK,
            1 => YELLOW,
            _ => RED,
        };
    }
    WHITE
}

pub fn run(profile: Profile, peripherals: Peripherals) -> ! {
    let mut delay = Delay::new();
    // Startup hold so `espflash monitor` can attach before the panel is touched.
    delay.delay_ms(8000);

    esp_println::println!("=== diag: 2.9\" JD7966x init as {} ===", profile.name);
    esp_println::println!("PSR: {:02X} 29, ticks: {}", profile.psr0, profile.ticks);
    esp_println::println!(
        "TRES: source {} gate {} ({} bytes per frame)",
        profile.source_bits,
        profile.gate_bits,
        profile.source_bits as usize * profile.gate_bits as usize / 4
    );
    esp_println::println!("One refresh only, do not loop this on the panel.");

    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    // Active-LOW BUSY, pulled up so an unpowered panel reads idle.
    let busy = Input::new(
        peripherals.GPIO4,
        InputConfig::default().with_pull(Pull::Up),
    );
    let spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(4))
            .with_mode(Mode::_0),
    )
    .expect("SPI2 config")
    .with_sck(peripherals.GPIO8)
    .with_mosi(peripherals.GPIO10);
    let spi_device = ExclusiveDevice::new_no_delay(spi, cs).expect("SpiDevice");
    let mut bus = SpiBusWrapper::new(spi_device, dc, rst, busy);

    // Vendor reset: at least 20 ms before, 40 ms low, 50 ms high. `hard_reset` uses one
    // duration for both phases, so 50 ms covers both minimums.
    delay.delay_ms(20);
    bus.hard_reset(&mut delay, 50).unwrap();
    bus.wait_busy_with_delay(&mut delay, false).unwrap();

    // Vendor `EPD_init()`, byte for byte.
    bus.send_command_with_data(0x4D, &[0x78]).unwrap();
    bus.send_command_with_data(0x00, &[profile.psr0, 0x29])
        .unwrap(); // PSR
    bus.send_command_with_data(0x01, &[0x07, 0x00]).unwrap(); // PWR
    bus.send_command_with_data(0x03, &[0x10, 0x54, 0x44])
        .unwrap(); // POFS
    bus.send_command_with_data(0x06, &[0x05, 0x00, 0x3F, 0x0A, 0x25, 0x12, 0x1A])
        .unwrap(); // BTST_P
    bus.send_command_with_data(0x50, &[0x37]).unwrap(); // CDI
    bus.send_command_with_data(0x60, &[0x02, 0x02]).unwrap(); // TCON
    bus.send_command_with_data(
        0x61,
        &[
            (profile.source_bits >> 8) as u8,
            profile.source_bits as u8,
            (profile.gate_bits >> 8) as u8,
            profile.gate_bits as u8,
        ],
    )
    .unwrap(); // TRES
    bus.send_command_with_data(0xE7, &[0x1C]).unwrap();
    bus.send_command_with_data(0xE3, &[0x22]).unwrap();
    bus.send_command_with_data(0xB4, &[0xD0]).unwrap();
    bus.send_command_with_data(0xB5, &[0x03]).unwrap();
    bus.send_command_with_data(0xE9, &[0x01]).unwrap();
    bus.send_command_with_data(0x30, &[0x08]).unwrap();
    bus.send_command(0x04).unwrap(); // PON
    bus.wait_busy_with_delay(&mut delay, false).unwrap();
    esp_println::println!("init done");

    // SAFETY: single-threaded example, and this is the only reference taken to FRAME_BUF.
    let frame: &mut [u8; 16128] = unsafe { &mut *core::ptr::addr_of_mut!(FRAME_BUF) };
    let w = profile.source_bits as u32;
    let h = profile.gate_bits as u32;
    let len = (w * h / 4) as usize;
    for y in 0..h {
        for xb in 0..(w / 4) {
            let mut byte = 0u8;
            for p in 0..4 {
                byte = (byte << 2) | pixel(&profile, xb * 4 + p, y);
            }
            frame[(y * (w / 4) + xb) as usize] = byte;
        }
    }

    bus.send_command(0x10).unwrap();
    for chunk in frame[..len].chunks(256) {
        bus.send_data(chunk).unwrap();
    }
    esp_println::println!("frame sent ({} bytes), refreshing", len);

    bus.send_command_with_data(0x12, &[0x00]).unwrap();
    bus.wait_busy_with_delay(&mut delay, false).unwrap();
    esp_println::println!("refresh done");

    bus.send_command_with_data(0x02, &[0x00]).unwrap();
    bus.wait_busy_with_delay(&mut delay, false).unwrap();
    bus.send_command_with_data(0x07, &[0xA5]).unwrap();
    esp_println::println!("asleep. Read the panel: border and red corner square at the top-left,");
    esp_println::println!("black/yellow/red bands across the middle third, white elsewhere.");

    loop {
        delay.delay_ms(1000);
    }
}
