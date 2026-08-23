//! # SSD1680 partial-refresh bisect
//!
//! Full refresh works on this panel (≈3.9 s, `epd_diag_213`), but the full example
//! stalls on the first Phase 2 partial update. This bisects why, changing one variable
//! at a time and timing `epd.refresh()` itself — the same call the example makes.
//!
//! - **A**: full frame, `Full` mode  — baseline, expect ≈3900 ms.
//! - **B**: full frame, `Partial` mode — is the fast LUT the problem?
//! - **C**: band y=50..249, `Partial` mode — is the *windowed* write the problem?
//!
//! Each test draws a distinct pattern, so the panel says as much as the log:
//! A = horizontal stripes, B = inverted stripes, C = stripes only below y=50.
//!
//! If B completes quickly and C stalls, the fault is the banded window. If B stalls,
//! the band is irrelevant. A stall shows as ~60 s or ~180 s, since `Ssd1680Controller`
//! runs three stages each bounded by a 60 s busy wait.
//!
//! ```bash
//! cargo run --release --example epd_diag_partial
//! ```

#![no_std]
#![no_main]

use embedded_hal::delay::DelayNs;
use embedded_hal_bus::spi::ExclusiveDevice;
use epdsi::prelude::*;
use esp_backtrace as _;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::{Instant, Rate};

esp_bootloader_esp_idf::esp_app_desc!();

const STRIDE: usize = GDEM0213B74::WIDTH.div_ceil(8) as usize;
const FRAME_BYTES: usize = STRIDE * GDEM0213B74::HEIGHT as usize;
const BAND_Y: u32 = 50;
const BAND_H: u32 = 200;
const BAND_BYTES: usize = STRIDE * BAND_H as usize;

static mut BUF: [u8; FRAME_BYTES] = [0xFFu8; FRAME_BYTES];

/// Horizontal stripes, `phase` selecting which bands are black.
fn stripes(buf: &mut [u8], rows: usize, phase: usize) {
    buf.fill(0xFF);
    for row in 0..rows {
        if (row / 20) % 2 == phase {
            for b in 0..STRIDE {
                buf[row * STRIDE + b] = 0x00;
            }
        }
    }
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(200);

    esp_println::println!("=== SSD1680 partial-refresh bisect ===");

    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    let busy = Input::new(
        peripherals.GPIO4,
        InputConfig::default().with_pull(Pull::Down),
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
    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Ssd1680Controller::new(GDEM0213B74::WIDTH, GDEM0213B74::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEM0213B74>::new(controller).build(epd_bus);

    esp_println::println!("init...");
    epd.init(&mut delay).unwrap();

    // SAFETY: single-threaded, only reference taken.
    let buf: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(BUF) };

    // --- A: full frame, Full mode. Baseline. ---
    esp_println::println!("--- A: full frame, Full mode --- (panel: horizontal stripes)");
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Full);
    stripes(&mut buf[..], GDEM0213B74::HEIGHT as usize, 0);

    epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::BlackWhite, &buf[..]).unwrap();
    epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::RedYellow, &buf[..]).unwrap();

    let t = Instant::now();
    epd.refresh(&mut delay).unwrap();
    esp_println::println!("  A: {} ms  (expect ~3900)", t.elapsed().as_millis());
    delay.delay_ms(3000);

    // --- B: full frame, Partial mode. ---
    esp_println::println!("--- B: full frame, Partial mode --- (panel: stripes invert)");
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Partial);
    stripes(&mut buf[..], GDEM0213B74::HEIGHT as usize, 1);

    epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::BlackWhite, &buf[..]).unwrap();

    let t = Instant::now();
    epd.refresh(&mut delay).unwrap();
    esp_println::println!(
        "  B: {} ms  (fast LUT should be well under 3900; ~60000 or ~180000 = stalled)",
        t.elapsed().as_millis()
    );
    delay.delay_ms(3000);

    // Keep the previous-image RAM in step so C diffs against what is on the panel.
    epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::RedYellow, &buf[..]).unwrap();

    // --- C: banded, Partial mode. ---
    esp_println::println!("--- C: band y=50..249, Partial mode --- (panel: lower part inverts)");
    stripes(&mut buf[..BAND_BYTES], BAND_H as usize, 0);

    epd.set_window(0, BAND_Y, GDEM0213B74::WIDTH - 1, BAND_Y + BAND_H - 1)
        .unwrap();
    epd.set_cursor(0, BAND_Y).unwrap();
    epd.write_frame(ColorChannel::BlackWhite, &buf[..BAND_BYTES])
        .unwrap();

    let t = Instant::now();
    epd.refresh(&mut delay).unwrap();
    esp_println::println!("  C: {} ms", t.elapsed().as_millis());

    esp_println::println!("=== done ===");
    loop {
        delay.delay_ms(1000);
    }
}
