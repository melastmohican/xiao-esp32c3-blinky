//! # Good Display GDEQ0426T82 4.26" 4-Level Grayscale (Gray4) E-Paper Example (`epdsi`)
//!
//! Companion to `ssd1677_gdeq0426t82_epd` (plain 1-bit monochrome) — same board, panel and
//! wiring, but drives the panel's **4-level grayscale** mode instead: White/Light/Dark/Black
//! instead of just White/Black.
//!
//! ## Provenance — read before trusting this on hardware
//!
//! `GDEQ0426T82::GRAY4` is **not** Good Display/Seeed material — Good Display's own spec lists this
//! as a 2-level (monochrome) panel. It is transcribed verbatim from Adafruit_EPD's
//! `ThinkInk_426_Grayscale4_GDEQ` reference driver (`ti_426_gray4_init_code` /
//! `ti_426_gray4_lut_code`), the only Gray4 reference for this controller/panel pairing.
//! **Confirmed on physical hardware** (RP2350, blocking and async, in `rust-rpico2-discovery` /
//! `rust-rpico2-embassy-examples`) rendering four distinct gray levels correctly — this ESP32-C3
//! port is the next verification.
//!
//! Unlike single-pass SSD1680 Gray4 panels (see this repo's `ssd1680_gdey0266t90_gray4_epd`),
//! `Adafruit_SSD1677::update()`'s grayscale branch is **two-pass**: a full refresh with the OTP LUT
//! (Red/Yellow plane bypassed) sets a known monochrome baseline, then the custom LUT and voltage
//! registers are reloaded, then a second refresh with the real Black/White (LSB) and Red/Yellow
//! (MSB) planes. See `epdsi`'s `Ssd1677RefreshMode::Gray4Preclear`/`Gray4` docs for the full
//! sequence — this example drives it directly with `EpdDriver` primitives (`write_frame`/`refresh`/
//! `reload_gray4_lut`), the same way the confirmed RP2350 examples do.
//!
//! ## Note on orientation
//!
//! Same portrait convention as `ssd1677_gdeq0426t82_epd`: [`DisplayRotation::Rotate270`] turns the
//! panel's native 800x480 landscape RAM into a 480x800 portrait drawing surface with the FPC
//! ribbon at the bottom.
//!
//! ## Note on buffer size
//!
//! Two 48,000-byte bit-plane buffers (96,000 bytes total) — `static mut`, not stack-allocated, for
//! the same reason as the mono example: the ESP32-C3's default task stack is far smaller than that.
//!
//! ## Hardware
//!
//! Same board, panel, carrier and wiring as `ssd1677_gdeq0426t82_epd` — see that example for the
//! full pin table. Repeated here for convenience:
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** [ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html) (24-pin FPC)
//! - **Display:** Dalian Good Display GDEQ0426T82 4.26" (800x480), driven in Gray4 mode
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
//! cargo run --release --example ssd1677_gdeq0426t82_gray4_epd
//! ```

#![no_std]
#![no_main]

use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_6X10};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle, RoundedRectangle};
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

esp_bootloader_esp_idf::esp_app_desc!();

/// Frame buffer size per bit-plane: 100 bytes per RAM row x 480 rows = 48,000 bytes.
const FRAME_BYTES: usize = GDEQ0426T82::WIDTH.div_ceil(8) as usize * GDEQ0426T82::HEIGHT as usize;

/// Visible width in the rotated portrait frame (the panel's 480 px axis).
const VIEW_W: u32 = GDEQ0426T82::HEIGHT;

/// Visible height in the rotated portrait frame (the panel's 800 px axis).
const VIEW_H: u32 = GDEQ0426T82::WIDTH;

/// This panel's two RAM planes are both inverted — see `Gray4Polarity::ADAFRUIT_SSD1677`'s doc
/// for the derivation from `ThinkInk_426_Grayscale4_GDEQ.h`.
const POLARITY: Gray4Polarity = Gray4Polarity::ADAFRUIT_SSD1677;

/// Bit-plane A (Black/White, LSB). `static mut` rather than a stack local — the ESP32-C3's
/// default task stack is far smaller than 48,000 bytes, let alone two of them.
static mut PLANE_A: [u8; FRAME_BYTES] = [0u8; FRAME_BYTES];

/// Bit-plane B (Red/Yellow, MSB).
static mut PLANE_B: [u8; FRAME_BYTES] = [0u8; FRAME_BYTES];

/// Screen 1: title/subtitle banner over a 4-band Black/Dark/Light/White swatch.
fn draw_banner(page: &mut GrayBufferPair) {
    let title_style = MonoTextStyle::new(&FONT_10X20, Gray4Color::Black);
    let dark_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Dark);
    let light_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Light);
    let black_small_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Black);
    let white_small_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::White);

    // "SSD1677 Gray4" is 13 chars at 10px = 130px, centered in the 480px width.
    Text::new("SSD1677 Gray4", Point::new(175, 40), title_style)
        .draw(page)
        .unwrap();
    // "GDEQ0426T82 800x480" is 20 chars at 6px = 120px.
    Text::new("GDEQ0426T82 800x480", Point::new(180, 62), dark_style)
        .draw(page)
        .unwrap();
    // "XIAO ESP32-C3 Gray4" is 19 chars at 6px = 114px.
    Text::new("XIAO ESP32-C3 Gray4", Point::new(183, 84), light_style)
        .draw(page)
        .unwrap();

    // Four equal bands spanning the full 480px width, one per gray level.
    const BAR_Y: i32 = 140;
    const BAR_H: u32 = 100;
    const BAR_W: u32 = VIEW_W / 4;

    let bands = [
        (0u32, Gray4Color::Black, "Black", white_small_style),
        (1u32, Gray4Color::Dark, "Dark", white_small_style),
        (2u32, Gray4Color::Light, "Light", black_small_style),
        (3u32, Gray4Color::White, "White", black_small_style),
    ];
    for (index, fill, label, label_style) in bands {
        let x = (index * BAR_W) as i32;
        Rectangle::new(Point::new(x, BAR_Y), Size::new(BAR_W, BAR_H))
            .into_styled(PrimitiveStyle::with_fill(fill))
            .draw(page)
            .unwrap();
        Text::new(label, Point::new(x + 8, BAR_Y + 56), label_style)
            .draw(page)
            .unwrap();
    }
    // Outline around the White band so its edge is visible against the page background.
    Rectangle::new(Point::new(3 * BAR_W as i32, BAR_Y), Size::new(BAR_W, BAR_H))
        .into_styled(PrimitiveStyle::with_stroke(Gray4Color::Black, 1))
        .draw(page)
        .unwrap();
}

/// Screen 2: four concentric rounded rectangles alternating gray levels, with a centered label.
fn draw_geometric(page: &mut GrayBufferPair) {
    let stroke = PrimitiveStyle::with_stroke(Gray4Color::Black, 1);
    Rectangle::new(Point::new(0, 0), Size::new(VIEW_W, VIEW_H))
        .into_styled(stroke)
        .draw(page)
        .unwrap();

    // (padding, corner radius, fill) for each nested ring, outer to inner.
    let rings: [(u32, u32, Gray4Color); 4] = [
        (20, 24, Gray4Color::Light),
        (60, 18, Gray4Color::Dark),
        (100, 12, Gray4Color::Black),
        (140, 12, Gray4Color::White),
    ];
    for (pad, radius, fill) in rings {
        let rect = Rectangle::new(
            Point::new(pad as i32, pad as i32),
            Size::new(VIEW_W - 2 * pad, VIEW_H - 2 * pad),
        );
        RoundedRectangle::with_equal_corners(rect, Size::new(radius, radius))
            .into_styled(PrimitiveStyle::with_fill(fill))
            .draw(page)
            .unwrap();
    }

    // Centered "4-Level Gray" label inside the innermost White ring.
    let inner_pad = 140i32;
    let inner_width = VIEW_W as i32 - 2 * inner_pad;
    let label_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Black);
    let label = "4-Level Gray";
    let label_width = label.len() as i32 * 6;
    Text::new(
        label,
        Point::new(
            inner_pad + (inner_width - label_width) / 2,
            VIEW_H as i32 / 2,
        ),
        label_style,
    )
    .draw(page)
    .unwrap();
}

/// Preclear pass: writes `data` to *both* the Black/White and Red/Yellow channels, so the
/// baseline OTP-LUT refresh (`Ssd1677RefreshMode::Gray4Preclear`) matches the final image's
/// black/white split.
fn write_preclear<BUS, C, P>(epd: &mut EpdDriver<BUS, C, P>, data: &[u8])
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::BlackWhite, data).unwrap();

    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::RedYellow, data).unwrap();
}

/// Final pass: writes the real Black/White (LSB) and Red/Yellow (MSB) planes.
fn write_real_planes<BUS, C, P>(epd: &mut EpdDriver<BUS, C, P>, plane_a: &[u8], plane_b: &[u8])
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::BlackWhite, plane_a).unwrap();

    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::RedYellow, plane_b).unwrap();
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();

    delay.delay_ms(100);
    esp_println::println!(
        "Starting GDEQ0426T82 4.26\" 4-Level Grayscale (Gray4) EPD example (epdsi SSD1677)"
    );

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
    .expect("failed to configure SPI2")
    .with_sck(peripherals.GPIO8)
    .with_mosi(peripherals.GPIO10);

    let spi_device = ExclusiveDevice::new_no_delay(spi, cs).expect("failed to build SpiDevice");

    // `for_panel` picks up GDEQ0426T82's dimensions; `.with_gray4` layers on the Adafruit_EPD-
    // sourced register bundle. Start on `Gray4Preclear` — the first pass of every screen below.
    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Ssd1677Controller::for_panel::<GDEQ0426T82>()
        .with_gray4(GDEQ0426T82::GRAY4)
        .with_refresh_mode(Ssd1677RefreshMode::Gray4Preclear);
    let mut epd = EpdBuilder::<_, GDEQ0426T82>::new(controller).build(epd_bus);

    esp_println::println!("Initializing SSD1677 epdsi EPD driver (Gray4)...");
    epd.init(&mut delay).unwrap();

    // SAFETY: single-threaded example, and these are the only references taken to the buffers.
    let plane_a: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(PLANE_A) };
    let plane_b: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(PLANE_B) };

    esp_println::println!("--- Screen 1: Banner & 4-Level Swatch ---");
    {
        let mut page = GrayBufferPair::new(
            plane_a,
            plane_b,
            GDEQ0426T82::WIDTH,
            GDEQ0426T82::HEIGHT,
            0,
            POLARITY,
        );
        page.set_rotation(DisplayRotation::Rotate270);
        page.clear();
        draw_banner(&mut page);

        esp_println::println!("Preclear pass (mono baseline, expect several seconds)...");
        write_preclear(&mut epd, page.plane_a().as_slice());
        epd.refresh(&mut delay).unwrap();

        // The preclear refresh's OTP LUT load overwrote the custom LUT/voltage registers
        // uploaded during init — reload them before the real Gray4 refresh.
        {
            let (bus, controller) = epd.split_mut();
            controller.reload_gray4_lut(bus).unwrap();
        }
        epd.controller_mut()
            .set_refresh_mode(Ssd1677RefreshMode::Gray4);

        esp_println::println!("Gray4 pass (real image, expect several seconds)...");
        write_real_planes(
            &mut epd,
            page.plane_a().as_slice(),
            page.plane_b().as_slice(),
        );
        epd.refresh(&mut delay).unwrap();
    }

    delay.delay_ms(8000);

    esp_println::println!("--- Screen 2: Concentric Geometric Grayscale Test Pattern ---");
    epd.controller_mut()
        .set_refresh_mode(Ssd1677RefreshMode::Gray4Preclear);
    {
        // SAFETY: single-threaded example, and these are the only references taken to the buffers.
        let plane_a: &'static mut [u8; FRAME_BYTES] =
            unsafe { &mut *core::ptr::addr_of_mut!(PLANE_A) };
        let plane_b: &'static mut [u8; FRAME_BYTES] =
            unsafe { &mut *core::ptr::addr_of_mut!(PLANE_B) };

        let mut page = GrayBufferPair::new(
            plane_a,
            plane_b,
            GDEQ0426T82::WIDTH,
            GDEQ0426T82::HEIGHT,
            0,
            POLARITY,
        );
        page.set_rotation(DisplayRotation::Rotate270);
        page.clear();
        draw_geometric(&mut page);

        esp_println::println!("Preclear pass (mono baseline, expect several seconds)...");
        write_preclear(&mut epd, page.plane_a().as_slice());
        epd.refresh(&mut delay).unwrap();

        {
            let (bus, controller) = epd.split_mut();
            controller.reload_gray4_lut(bus).unwrap();
        }
        epd.controller_mut()
            .set_refresh_mode(Ssd1677RefreshMode::Gray4);

        esp_println::println!("Gray4 pass (real image, expect several seconds)...");
        write_real_planes(
            &mut epd,
            page.plane_a().as_slice(),
            page.plane_b().as_slice(),
        );
        epd.refresh(&mut delay).unwrap();
    }

    esp_println::println!("Display complete!");

    loop {
        delay.delay_ms(1000);
    }
}
