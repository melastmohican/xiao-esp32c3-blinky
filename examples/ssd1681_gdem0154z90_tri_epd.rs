//! # GDEM0154Z90 Tri-Color `PageBufferPair` Draw Target Example (`epdsi`)
//!
//! Full-parity companion to `ssd1681_gdem0154z90_epd` on the Seeed Studio XIAO ESP32-C3 — same two
//! phases, same content, same board bring-up — but drawn entirely through `PageBufferPair`/`TriColor`
//! instead of two separate `PageBuffer`s and panel-specific `BinaryColor::On`/`Off` polarity choices.
//!
//! Demonstrates:
//! 1. **Phase 1**: Full tri-colour refresh — border, header, black and red swatches,
//!    Ferris in red (Accent), Rust in black, footer labels.
//! 2. **Phase 2**: Five partial *window* updates of a 200x60 status band, writing both
//!    colour planes each time so the accent progress bar survives.
//!
//! See `ssd1681_gdem0154z90_epd` for the full narrative on refresh speed, wiring, and hardware —
//! none of that changed. What differs is only the drawing code:
//!
//! - One `page: &mut PageBufferPair` replaces the `display_bw`/`display_red` `PageBuffer` locals.
//! - `TriColor::{Black, Accent}` replaces every `BinaryColor::On`/`Off` polarity choice — no more
//!   picking `Off` "because this is the Red plane."
//! - `PageBufferPair::clear()` replaces the `band_bw.clear_byte(0xFF)` / `band_red.clear_byte(0x00)`
//!   pair before each windowed redraw.
//!
//! This example still uses `static mut` frame buffers accessed through raw pointers rather than
//! stack locals: two 5,000-byte frame buffers don't fit this board's default stack. The `static`
//! initial values now come from `PlanePolarity::SSD168X.bw_background_byte()` /
//! `.accent_background_byte()` (both `const fn`) instead of hardcoded `0xFF`/`0x00`, so the polarity
//! constant is the single source of truth for both the initial buffer contents and the
//! `clear_frame` calls below.
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
//! The accent channel is stored inverted relative to black/white on this controller
//! (`PlanePolarity::SSD168X`): its buffer starts at `0x00` and `TriColor::Accent` sets a bit. A band
//! update writes **both** planes for the region via `page.bw()`/`page.accent()` — writing only
//! black/white would leave stale red behind.
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
//! cargo run --release --example ssd1681_gdem0154z90_tri_epd
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

/// The polarity this panel needs — Black/White plane normal, accent plane inverted. Passed to
/// every `PageBufferPair::new` call rather than assumed once, since a different panel could need
/// `PlanePolarity::UC8253` instead.
const POLARITY: PlanePolarity = PlanePolarity::SSD168X;

/// 200 x 200 / 8 = 5,000 bytes per colour plane.
const PLANE_BYTES: usize = (GDEM0154Z90::WIDTH as usize * GDEM0154Z90::HEIGHT as usize) / 8;

/// Bottom status band. The Rust logo ends at y = 139 (75 + 64), so starting at 140 leaves
/// the header and logos painted in Phase 1 untouched.
const BAND_Y: u32 = 140;
const BAND_H: u32 = 60;
const BAND_BYTES: usize = (GDEM0154Z90::WIDTH as usize * BAND_H as usize) / 8;

/// Black/white plane. Background byte comes from `PlanePolarity::SSD168X`, which is `0xFF` (white).
static mut BW_BUF: [u8; PLANE_BYTES] = [POLARITY.bw_background_byte(); PLANE_BYTES];
/// Accent (red) plane, stored inverted: background byte from `PlanePolarity::SSD168X` is `0x00`.
static mut RED_BUF: [u8; PLANE_BYTES] = [POLARITY.accent_background_byte(); PLANE_BYTES];

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(100);

    esp_println::println!(
        "Starting GDEM0154Z90 1.54\" Tri-Color EPD example (epdsi SSD1681, PageBufferPair)"
    );
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

    epd.clear_frame(ColorChannel::BlackWhite, POLARITY.bw_background_byte())
        .unwrap();
    epd.clear_frame(ColorChannel::RedYellow, POLARITY.accent_background_byte())
        .unwrap();

    // SAFETY: single-threaded example, and these are the only references taken.
    let bw_buf: &'static mut [u8; PLANE_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(BW_BUF) };
    let red_buf: &'static mut [u8; PLANE_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(RED_BUF) };

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    let stroke = PrimitiveStyle::with_stroke(TriColor::Black, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, TriColor::Black);

    esp_println::println!("--- Phase 1: Full Tri-Color Refresh ---");

    // Scoped so the full-frame borrows end before Phase 2 re-borrows the prefixes as
    // smaller sub-region buffers.
    {
        let mut page = PageBufferPair::new(
            &mut bw_buf[..],
            &mut red_buf[..],
            GDEM0154Z90::WIDTH,
            GDEM0154Z90::HEIGHT,
            0,
            POLARITY,
        );

        // Outer border (Black)
        Rectangle::new(
            Point::new(0, 0),
            Size::new(GDEM0154Z90::WIDTH, GDEM0154Z90::HEIGHT),
        )
        .into_styled(stroke)
        .draw(&mut page)
        .unwrap();

        Text::new("GDEM0154Z90 1.54\"", Point::new(10, 18), text_style)
            .draw(&mut page)
            .unwrap();

        Line::new(Point::new(10, 25), Point::new(190, 25))
            .into_styled(stroke)
            .draw(&mut page)
            .unwrap();

        // Subtitle: "Tri-Color" in Black, "BWR" in Accent.
        Text::new("Tri-Color ", Point::new(10, 42), text_style)
            .draw(&mut page)
            .unwrap();
        Text::new(
            "BWR",
            Point::new(110, 42),
            MonoTextStyle::new(&FONT_10X20, TriColor::Accent),
        )
        .draw(&mut page)
        .unwrap();

        Rectangle::new(Point::new(10, 50), Size::new(180, 16))
            .into_styled(stroke)
            .draw(&mut page)
            .unwrap();

        // Black swatch.
        Rectangle::new(Point::new(12, 52), Size::new(84, 12))
            .into_styled(PrimitiveStyle::with_fill(TriColor::Black))
            .draw(&mut page)
            .unwrap();

        // Accent swatch.
        Rectangle::new(Point::new(104, 52), Size::new(84, 12))
            .into_styled(PrimitiveStyle::with_fill(TriColor::Accent))
            .draw(&mut page)
            .unwrap();

        // Ferris in Accent, left.
        let ferris_pos = Point::new(20, 75);
        for pixel in ferris_bmp.pixels() {
            if pixel.1 == BinaryColor::Off {
                Pixel(pixel.0 + ferris_pos, TriColor::Accent)
                    .draw(&mut page)
                    .unwrap();
            }
        }

        // Rust logo in Black, right.
        let rust_pos = Point::new(115, 75);
        for pixel in rust_bmp.pixels() {
            if pixel.1 == BinaryColor::On {
                Pixel(pixel.0 + rust_pos, TriColor::Black)
                    .draw(&mut page)
                    .unwrap();
            }
        }

        Text::new("XIAO ESP32-C3", Point::new(10, 165), text_style)
            .draw(&mut page)
            .unwrap();

        Text::new("epdsi PageBufferPair", Point::new(10, 185), text_style)
            .draw(&mut page)
            .unwrap();

        // Each RAM write starts from the window origin, so reset window + cursor before
        // both channels.
        esp_println::println!("Sending Black/White frame (5,000 bytes)...");
        epd.set_window(0, 0, GDEM0154Z90::WIDTH - 1, GDEM0154Z90::HEIGHT - 1)
            .unwrap();
        epd.set_cursor(0, 0).unwrap();
        epd.write_frame(ColorChannel::BlackWhite, page.bw().as_slice())
            .unwrap();

        esp_println::println!("Sending Red frame (5,000 bytes)...");
        epd.set_window(0, 0, GDEM0154Z90::WIDTH - 1, GDEM0154Z90::HEIGHT - 1)
            .unwrap();
        epd.set_cursor(0, 0).unwrap();
        epd.write_frame(ColorChannel::RedYellow, page.accent().as_slice())
            .unwrap();

        esp_println::println!("Refreshing (full waveform, ~14 s — do not interrupt)...");
        epd.refresh(&mut delay).unwrap();
    }

    delay.delay_ms(2000);

    esp_println::println!("--- Phase 2: Partial Window Tri-Color Refresh ---");

    for count in 1..=5u32 {
        // Sub-region buffers: 200 x 60 / 8 = 1,500 bytes of each full-frame array.
        let mut band = PageBufferPair::new(
            &mut bw_buf[..BAND_BYTES],
            &mut red_buf[..BAND_BYTES],
            GDEM0154Z90::WIDTH,
            BAND_H,
            BAND_Y,
            POLARITY,
        );
        band.clear();

        let mut count_buf = [0u8; 32];
        let count_str =
            format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap();
        Text::new(count_str, Point::new(10, 157), text_style)
            .draw(&mut band)
            .unwrap();

        Rectangle::new(Point::new(10, 164), Size::new(180, 14))
            .into_styled(stroke)
            .draw(&mut band)
            .unwrap();

        // Progress bar fill in Accent — proves the accent plane survives a windowed update.
        Rectangle::new(Point::new(12, 166), Size::new(count * 35, 10))
            .into_styled(PrimitiveStyle::with_fill(TriColor::Accent))
            .draw(&mut band)
            .unwrap();

        Text::new("Partial window", Point::new(10, 195), text_style)
            .draw(&mut band)
            .unwrap();

        // Restrict controller RAM to the band, then write BOTH planes for that region.
        epd.set_window(0, BAND_Y, GDEM0154Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .unwrap();
        epd.set_cursor(0, BAND_Y).unwrap();
        epd.write_frame(ColorChannel::BlackWhite, band.bw().as_slice())
            .unwrap();

        epd.set_window(0, BAND_Y, GDEM0154Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
            .unwrap();
        epd.set_cursor(0, BAND_Y).unwrap();
        epd.write_frame(ColorChannel::RedYellow, band.accent().as_slice())
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
