//! # Good Display GDEM0154Z90 1.54" Tri-Color E-Paper Example (`epdsi`)
//!
//! Port of the Raspberry Pi Pico 2 example from
//! [`rust-rpico2-discovery`](https://github.com/melastmohican/rust-rpico2-discovery)
//! to the Seeed Studio XIAO ESP32-C3 on the ePaper Driver Board for XIAO.
//!
//! All the `epdsi` and `embedded-graphics` code is unchanged from the RP2350 version;
//! only the board bring-up in `main` differs.
//!
//! Demonstrates:
//! 1. **Phase 1**: Full tri-colour refresh — border, header, black and red swatches,
//!    Ferris in red, Rust in black, footer labels.
//! 2. **Phase 2**: Five partial *window* updates of a 200x60 status band, writing both
//!    colour planes each time so the red progress bar survives.
//!
//! ## This example is slow, and that is correct
//!
//! Tri-colour panels have **no fast waveform**. The red pigment is a heavier particle
//! that needs the full OTP waveform to migrate, so *every* refresh takes roughly **14
//! seconds** — including the "partial" ones, which are partial only in the sense that
//! the RAM window is narrowed. Total runtime is around **90 seconds**.
//!
//! Do not interrupt it. E-paper retains the last write, and a run cut off mid-update
//! leaves the panel latched, which makes the *next* run look broken.
//!
//! [`Ssd1681RefreshMode::Partial`] is deliberately never selected: trigger `0x22 = 0xFC`
//! picks the controller's built-in fast LUT, which only exists for monochrome panels. On
//! a BWR panel it is slow *and* discards red.
//!
//! ## Both planes must be written
//!
//! The red channel is stored inverted relative to black/white: its buffer starts at
//! `0x00` and `BinaryColor::Off` sets a bit. A band update writes **both** planes for
//! the region — writing only black/white would leave stale red behind.
//!
//! ## Hardware
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** [ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html) (24-pin FPC)
//! - **Display:** Good Display GDEM0154Z90, 1.54" tri-colour (black/white/red), 200x200
//!
//! ## Wiring
//!
//! Fixed by the driver board; nothing to wire by hand.
//!
//! | Signal | XIAO pin | ESP32-C3 GPIO |
//! |--------|----------|---------------|
//! | RST    | D0       | GPIO2         |
//! | CS     | D1       | GPIO3         |
//! | BUSY   | D2       | GPIO4         |
//! | DC     | D3       | GPIO5         |
//! | SCK    | D8       | GPIO8         |
//! | MOSI   | D10      | GPIO10        |
//!
//! MISO is unused: e-paper is write-only and `SpiBusWrapper` never reads.
//!
//! ## Run
//!
//! ```bash
//! cargo run --release --example ssd1681_gdem0154z90_epd
//! ```

#![no_std]
#![no_main]

use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::mono_font::ascii::FONT_10X20;
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

/// 200 x 200 / 8 = 5,000 bytes per colour plane.
const PLANE_BYTES: usize = (GDEM0154Z90::WIDTH as usize * GDEM0154Z90::HEIGHT as usize) / 8;

/// Bottom status band. The Rust logo ends at y = 139 (75 + 64), so starting at 140 leaves
/// the header and logos painted in Phase 1 untouched.
const BAND_Y: u32 = 140;
const BAND_H: u32 = 60;
const BAND_BYTES: usize = (GDEM0154Z90::WIDTH as usize * BAND_H as usize) / 8;

/// Black/white plane. `0xFF` is white.
static mut BW_BUF: [u8; PLANE_BYTES] = [0xFFu8; PLANE_BYTES];
/// Red plane, stored inverted: `0x00` is "no red".
static mut RED_BUF: [u8; PLANE_BYTES] = [0x00u8; PLANE_BYTES];

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(100);

    esp_println::println!("Starting GDEM0154Z90 1.54\" Tri-Color EPD example (epdsi SSD1681)");
    esp_println::println!("Every refresh takes ~14 s on a tri-colour panel. Total ~90 s.");
    esp_println::println!("Do not interrupt it.");

    // Pin assignments are fixed by the ePaper Driver Board for XIAO.
    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    // SSD1681 BUSY is active-HIGH, so pull down: a floating line reads "idle".
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
    let controller = Ssd1681Controller::new(GDEM0154Z90::WIDTH, GDEM0154Z90::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEM0154Z90>::new(controller).build(epd_bus);

    esp_println::println!("Initializing SSD1681 epdsi EPD driver...");
    epd.init(&mut delay).unwrap();

    epd.clear_frame(ColorChannel::BlackWhite, 0xFF).unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0x00).unwrap();

    // SAFETY: single-threaded example, and these are the only references taken.
    let bw_buf: &'static mut [u8; PLANE_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(BW_BUF) };
    let red_buf: &'static mut [u8; PLANE_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(RED_BUF) };

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    let style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    esp_println::println!("--- Phase 1: Full Tri-Color Refresh ---");

    // Scoped so the full-frame borrows end before Phase 2 re-borrows the prefixes as
    // smaller sub-region buffers.
    {
        let mut display_bw =
            PageBuffer::new(&mut bw_buf[..], GDEM0154Z90::WIDTH, GDEM0154Z90::HEIGHT, 0);
        let mut display_red =
            PageBuffer::new(&mut red_buf[..], GDEM0154Z90::WIDTH, GDEM0154Z90::HEIGHT, 0);

        Rectangle::new(
            Point::new(0, 0),
            Size::new(GDEM0154Z90::WIDTH, GDEM0154Z90::HEIGHT),
        )
        .into_styled(style)
        .draw(&mut display_bw)
        .unwrap();

        Text::new("GDEM0154Z90 1.54\"", Point::new(10, 18), text_style)
            .draw(&mut display_bw)
            .unwrap();

        Line::new(Point::new(10, 25), Point::new(190, 25))
            .into_styled(style)
            .draw(&mut display_bw)
            .unwrap();

        // Subtitle: "Tri-Color" in black, "BWR" in red.
        Text::new("Tri-Color ", Point::new(10, 42), text_style)
            .draw(&mut display_bw)
            .unwrap();
        Text::new("BWR", Point::new(110, 42), text_style)
            .draw(&mut display_red)
            .unwrap();

        Rectangle::new(Point::new(10, 50), Size::new(180, 16))
            .into_styled(style)
            .draw(&mut display_bw)
            .unwrap();

        // Black swatch on the B/W plane.
        Rectangle::new(Point::new(12, 52), Size::new(84, 12))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut display_bw)
            .unwrap();

        // Red swatch on the red plane. `Off` sets a bit in the 0x00-based buffer.
        Rectangle::new(Point::new(104, 52), Size::new(84, 12))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut display_red)
            .unwrap();

        // Ferris in red, left.
        let ferris_pos = Point::new(20, 75);
        for pixel in ferris_bmp.pixels() {
            if pixel.1 == BinaryColor::Off {
                Pixel(pixel.0 + ferris_pos, BinaryColor::Off)
                    .draw(&mut display_red)
                    .unwrap();
            }
        }

        // Rust logo in black, right.
        let rust_pos = Point::new(115, 75);
        for pixel in rust_bmp.pixels() {
            if pixel.1 == BinaryColor::On {
                Pixel(pixel.0 + rust_pos, BinaryColor::On)
                    .draw(&mut display_bw)
                    .unwrap();
            }
        }

        Text::new("XIAO ESP32-C3", Point::new(10, 165), text_style)
            .draw(&mut display_bw)
            .unwrap();

        Text::new("epdsi SSD1681", Point::new(10, 185), text_style)
            .draw(&mut display_bw)
            .unwrap();

        // Each RAM write starts from the window origin, so reset window + cursor before
        // both channels.
        esp_println::println!("Sending Black/White frame (5,000 bytes)...");
        epd.set_window(0, 0, GDEM0154Z90::WIDTH - 1, GDEM0154Z90::HEIGHT - 1)
            .unwrap();
        epd.set_cursor(0, 0).unwrap();
        epd.write_frame(ColorChannel::BlackWhite, display_bw.as_slice())
            .unwrap();

        esp_println::println!("Sending Red frame (5,000 bytes)...");
        epd.set_window(0, 0, GDEM0154Z90::WIDTH - 1, GDEM0154Z90::HEIGHT - 1)
            .unwrap();
        epd.set_cursor(0, 0).unwrap();
        epd.write_frame(ColorChannel::RedYellow, display_red.as_slice())
            .unwrap();

        esp_println::println!("Refreshing (full waveform, ~14 s — do not interrupt)...");
        epd.refresh(&mut delay).unwrap();
    }

    delay.delay_ms(2000);

    esp_println::println!("--- Phase 2: Partial Window Tri-Color Refresh ---");

    for count in 1..=5u32 {
        // Sub-region buffers: 200 x 60 / 8 = 1,500 bytes of each full-frame array.
        let mut band_bw = PageBuffer::new(
            &mut bw_buf[..BAND_BYTES],
            GDEM0154Z90::WIDTH,
            BAND_H,
            BAND_Y,
        );
        let mut band_red = PageBuffer::new(
            &mut red_buf[..BAND_BYTES],
            GDEM0154Z90::WIDTH,
            BAND_H,
            BAND_Y,
        );

        band_bw.clear_byte(0xFF);
        band_red.clear_byte(0x00);

        let mut count_buf = [0u8; 32];
        let count_str =
            format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap();
        Text::new(count_str, Point::new(10, 157), text_style)
            .draw(&mut band_bw)
            .unwrap();

        Rectangle::new(Point::new(10, 164), Size::new(180, 14))
            .into_styled(style)
            .draw(&mut band_bw)
            .unwrap();

        // Progress bar fill in red — proves the red plane survives a windowed update.
        Rectangle::new(Point::new(12, 166), Size::new(count * 35, 10))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::Off))
            .draw(&mut band_red)
            .unwrap();

        Text::new("Partial window", Point::new(10, 195), text_style)
            .draw(&mut band_bw)
            .unwrap();

        // Restrict controller RAM to the band, then write BOTH planes for that region.
        epd.set_window(0, BAND_Y, GDEM0154Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .unwrap();
        epd.set_cursor(0, BAND_Y).unwrap();
        epd.write_frame(ColorChannel::BlackWhite, band_bw.as_slice())
            .unwrap();

        epd.set_window(0, BAND_Y, GDEM0154Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .unwrap();
        epd.set_cursor(0, BAND_Y).unwrap();
        epd.write_frame(ColorChannel::RedYellow, band_red.as_slice())
            .unwrap();

        esp_println::println!(
            "Refreshing band y={}..{} (Update #{}, ~14 s)...",
            BAND_Y,
            BAND_Y + BAND_H - 1,
            count
        );
        epd.refresh(&mut delay).unwrap();

        delay.delay_ms(1000);
    }

    // Restore the full-frame RAM window for any subsequent updates.
    epd.set_window(0, 0, GDEM0154Z90::WIDTH - 1, GDEM0154Z90::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();

    esp_println::println!("Display complete!");

    loop {
        delay.delay_ms(1000);
    }
}
