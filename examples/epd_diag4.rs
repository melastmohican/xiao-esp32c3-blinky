//! # Black / white fill confirmation (XIAO ESP32-C3 + ePaper Driver Board)
//!
//! Does exactly two things and stops: fill the panel solid BLACK, then solid WHITE, each
//! with a full-waveform refresh, reporting how long each took.
//!
//! No logos, no differential mode, no phases — the simplest possible end-to-end check
//! that init, the RAM write, the full refresh and BUSY handling all work.
//!
//! Watch the panel, not the log. The timings say whether BUSY was awaited: a real full
//! refresh on this panel is roughly 4 s, so anything under ~500 ms means the wait
//! returned early and the panel was never driven.
//!
//! ```bash
//! cargo run --release --example epd_diag4
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
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::{Instant, Rate};

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(200);

    esp_println::println!("=== diag4: black / white fill confirmation ===");

    // Pin assignments are fixed by the ePaper Driver Board for XIAO.
    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    // SSD1677 BUSY is active-HIGH.
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
    let controller = Ssd1677Controller::new(GDEQ0426T82::WIDTH, GDEQ0426T82::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEQ0426T82>::new(controller).build(epd_bus);

    esp_println::println!("init...");
    epd.init(&mut delay).unwrap();

    // Full-waveform mode throughout; the differential path is deliberately untouched.
    epd.controller_mut()
        .set_refresh_mode(Ssd1677RefreshMode::Full);

    for (label, byte) in [("BLACK", 0x00u8), ("WHITE", 0xFFu8)] {
        esp_println::println!("--- filling {} (0x{:02X}) ---", label, byte);

        // Secondary RAM is the "previous image" on this monochrome panel, not a colour
        // plane. Keep it white so a full refresh never reads it as a second plane.
        epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
            .unwrap();
        epd.set_cursor(0, 0).unwrap();
        epd.clear_frame(ColorChannel::RedYellow, 0xFF).unwrap();

        epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
            .unwrap();
        epd.set_cursor(0, 0).unwrap();
        epd.clear_frame(ColorChannel::BlackWhite, byte).unwrap();

        let start = Instant::now();
        epd.refresh(&mut delay).unwrap();
        let ms = start.elapsed().as_millis();

        if ms < 500 {
            esp_println::println!(
                "    refresh returned in {} ms  <-- TOO FAST, BUSY was not awaited",
                ms
            );
        } else {
            esp_println::println!("    refresh took {} ms  (expected ~4000 ms)", ms);
        }

        esp_println::println!(
            "    >>> LOOK AT THE PANEL: it should now be solid {}",
            label
        );
        delay.delay_ms(6000);
    }

    esp_println::println!("=== diag4 done. Panel should have gone BLACK, then WHITE. ===");
    loop {
        delay.delay_ms(1000);
    }
}
