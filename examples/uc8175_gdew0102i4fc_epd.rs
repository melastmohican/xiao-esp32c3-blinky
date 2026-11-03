//! # Good Display GDEW0102I4FC 1.02" Monochrome E-Paper Example (`epdsi`, XIAO ESP32-C3)
//!
//! XIAO ESP32-C3 wired to a Good Display `DESPI-C102` adapter, driving the **GDEW0102I4FC**
//! 1.02" Monochrome (80x128) panel with `Uc8175Controller` from `epdsi`.
//!
//! **Bench verified on 2 Nov 2026** (blocking, XIAO ESP32-C3 wired by hand to a `DESPI-C102`): the
//! full refresh rendered the Rust and Ferris picture. A half-seated FPC shows only noise, so
//! reseat it first if that happens.
//!
//! Two full refreshes (about 3 s each, this panel has no partial refresh):
//! 1. Clear to white, as the vendor demo does first.
//! 2. Header, Rust and Ferris logos.
//!
//! Then deep sleep.
//!
//! ## Pin map (same `D` positions as the XIAO MG24 sketches, wired by hand to the adapter)
//!
//! | Signal | XIAO pin | ESP32-C3 GPIO |
//! |--------|----------|---------------|
//! | RST    | D0       | GPIO2         |
//! | CS     | D1       | GPIO3         |
//! | BUSY   | D2       | GPIO4 (active-LOW) |
//! | DC     | D3       | GPIO5         |
//! | SCK    | D8       | GPIO8         |
//! | MOSI   | D10      | GPIO10        |
//!
//! ## Run
//!
//! Flash, then unplug and replug USB, then attach the monitor (see `BRINGUP.md`). The example
//! waits 5 s at start so there is time to attach.
//!
//! ```bash
//! cargo run --release --example uc8175_gdew0102i4fc_epd
//! espflash monitor
//! ```

#![no_std]
#![no_main]

use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use embedded_hal::delay::DelayNs;
use embedded_hal_bus::spi::ExclusiveDevice;
use epdsi::prelude::*;
use esp_backtrace as _;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::Rate;
use tinybmp::Bmp;

esp_bootloader_esp_idf::esp_app_desc!();

/// Row stride in bytes: 80 / 8 = 10.
const STRIDE: usize = GDEW0102I4FC::WIDTH.div_ceil(8) as usize;

/// Full frame size: 10 x 128 = 1,280 bytes, the vendor's `EPD_ARRAY`.
const FRAME_BYTES: usize = STRIDE * GDEW0102I4FC::HEIGHT as usize;

/// How long the cleared panel is held before the picture, in milliseconds.
const HOLD_MS: u32 = 1_500;

/// X of the 64 px wide logos, centered in the 80 px wide panel.
const LOGO_X: i32 = (GDEW0102I4FC::WIDTH as i32 - 64) / 2;

/// Draws the border, header, Rust logo (64 x 64) and Ferris logo (64 x 42).
fn draw_picture(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
) {
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    Rectangle::new(
        Point::new(0, 0),
        Size::new(GDEW0102I4FC::WIDTH, GDEW0102I4FC::HEIGHT),
    )
    .into_styled(stroke)
    .draw(display)
    .unwrap();

    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    Text::new("GDEW0102I4FC", Point::new(4, 12), style)
        .draw(display)
        .unwrap();
    Line::new(Point::new(0, 16), Point::new(79, 16))
        .into_styled(stroke)
        .draw(display)
        .unwrap();

    // The Rust BMP draws where it is `On`, the Ferris BMP where it is `Off`. On `PageBuffer`,
    // `On` is the black ink.
    let rust_pos = Point::new(LOGO_X, 19);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }
    let ferris_pos = Point::new(LOGO_X, 84);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(100);

    esp_println::println!("Starting GDEW0102I4FC 1.02\" Monochrome EPD example (epdsi UC8175)");

    // Hold before touching the panel so `espflash monitor` can attach after a power cycle.
    delay.delay_ms(5_000);

    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    // UC8175 BUSY is active-LOW: pull up so a missing panel reads idle, not busy forever.
    let busy = Input::new(
        peripherals.GPIO4,
        InputConfig::default().with_pull(Pull::Up),
    );

    // 4 MHz, the rate of Good Display's own XIAO sketches for this panel.
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
    let controller = Uc8175Controller::new();
    let mut epd = EpdBuilder::<_, GDEW0102I4FC>::new(controller).build(epd_bus);

    esp_println::println!("Initializing UC8175...");
    epd.init(&mut delay).unwrap();

    let full = (0, 0, GDEW0102I4FC::WIDTH - 1, GDEW0102I4FC::HEIGHT - 1);

    esp_println::println!("--- Screen 1 of 2: clear to white ---");
    epd.set_window(full.0, full.1, full.2, full.3).unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0xFF).unwrap();
    epd.clear_frame(ColorChannel::BlackWhite, 0xFF).unwrap();
    epd.refresh(&mut delay).unwrap();
    esp_println::println!("Clear done, holding briefly");
    delay.delay_ms(HOLD_MS);

    let mut buf = [0xFFu8; FRAME_BYTES];
    let mut display = PageBuffer::new(&mut buf, GDEW0102I4FC::WIDTH, GDEW0102I4FC::HEIGHT, 0);
    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();
    draw_picture(&mut display, &ferris_bmp, &rust_bmp);

    esp_println::println!("--- Screen 2 of 2: Rust and Ferris ---");
    // Old plane white first, as the vendor demo does, then the new plane.
    epd.clear_frame(ColorChannel::RedYellow, 0xFF).unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .unwrap();
    epd.refresh(&mut delay).unwrap();
    esp_println::println!("Picture done");

    esp_println::println!("--- Deep sleep ---");
    epd.sleep(&mut delay).unwrap();
    esp_println::println!("Display complete!");

    loop {
        delay.delay_ms(500);
    }
}
