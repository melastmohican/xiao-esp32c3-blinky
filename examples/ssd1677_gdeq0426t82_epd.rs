//! # Good Display GDEQ0426T82 4.26" Monochrome E-Paper Example (`epdsi`)
//!
//! Port of the Raspberry Pi Pico 2 example from
//! [`rust-rpico2-discovery`](https://github.com/melastmohican/rust-rpico2-discovery)
//! to the Seeed Studio XIAO ESP32-C3 on the ePaper Driver Board for XIAO.
//!
//! Everything from `FRAME_BYTES` down to `draw_frame` is byte-for-byte identical to the
//! RP2350 version — the `epdsi` panel, controller, `PageBuffer` and `embedded-graphics`
//! code is HAL-agnostic. Only the board bring-up in `main` differs.
//!
//! Demonstrates:
//! 1. **Phase 1**: Full monochrome refresh — header bar, 4x-scaled Ferris and Rust logos
//!    stacked vertically, and text labels.
//! 2. **Phase 2**: Fast *differential* refresh loop swapping the two logos each pass and
//!    advancing a progress bar. Only changed pixels move, so the header stays steady.
//! 3. **Phase 3**: Full-waveform cleanup pass, restoring the ink density the shortened
//!    differential waveform leaves behind.
//!
//! ## Note on orientation
//!
//! The panel's native RAM layout is 800x480 landscape with the origin away from the FPC
//! ribbon. This renders **portrait, 480x800, ribbon at the bottom**, via
//! [`DisplayRotation::Rotate270`]: logical `(x, y)` maps to RAM `(y, 479 - x)`.
//!
//! ## Note on the reversed Y axis
//!
//! This panel's gates are physically wired in reverse and the SSD1677 has no
//! gate-scan-direction bit, so [`Ssd1677Controller`] flips Y in software inside
//! `set_window`/`set_cursor`. Transparent to callers, and separate from the rotation above.
//!
//! ## Note on buffer size
//!
//! 800 px is byte-aligned, so a RAM row is 100 bytes and the full frame is 48,000 bytes.
//! The ESP32-C3 has 400 KB of SRAM, but the default task stack is far smaller than 48 KB,
//! so the frame buffer is a `static mut` rather than a stack array — this is the one
//! substantive difference from the RP2350 version, where it lives on the stack.
//!
//! ## Hardware
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** [ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html) (24-pin FPC)
//! - **Display:** Dalian Good Display GDEQ0426T82 4.26" Monochrome (800x480)
//!
//! ## Wiring
//!
//! The driver board fixes these assignments; nothing to wire by hand.
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
//! cargo run --release --example ssd1677_gdeq0426t82_epd
//! ```

#![no_std]
#![no_main]

use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_10X20;
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
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::Rate;
use tinybmp::Bmp;

esp_bootloader_esp_idf::esp_app_desc!();

/// Full frame buffer size: 100 bytes per RAM row x 480 rows = 48,000 bytes.
const FRAME_BYTES: usize = GDEQ0426T82::WIDTH.div_ceil(8) as usize * GDEQ0426T82::HEIGHT as usize;

/// Visible width in the rotated portrait frame (the panel's 480 px axis).
const VIEW_W: u32 = GDEQ0426T82::HEIGHT;

/// Visible height in the rotated portrait frame (the panel's 800 px axis).
const VIEW_H: u32 = GDEQ0426T82::WIDTH;

/// Bottom Y coordinate of the header bar. Its border is shared with the content area below.
const HEADER_H: u32 = 60;

/// Integer scale factor applied to both 64 px logo bitmaps. At 4x they become 256 px wide,
/// which suits a 480 px-wide portrait frame.
const LOGO_SCALE: u32 = 4;

/// X coordinate that horizontally centres a 256 px scaled logo in the 480 px portrait frame.
const LOGO_X: i32 = (VIEW_W as i32 - 64 * LOGO_SCALE as i32) / 2;

/// Y coordinate of the top of the upper logo slot.
const LOGO_TOP_Y: i32 = 110;

/// Y coordinate both logo stacks are bottom-aligned to.
const LOGO_BOTTOM_Y: i32 = 574;

/// Draws a 1 bpp bitmap scaled up by [`LOGO_SCALE`], with each source pixel becoming a filled
/// square. `ink` selects which [`BinaryColor`] counts as set in the source: the Ferris and Rust
/// BMPs ship with opposite polarity.
fn draw_scaled(display: &mut PageBuffer, bmp: &Bmp<BinaryColor>, origin: Point, ink: BinaryColor) {
    let fill = PrimitiveStyle::with_fill(BinaryColor::On);
    let scale = LOGO_SCALE as i32;

    for pixel in bmp.pixels() {
        if pixel.1 == ink {
            Rectangle::new(
                origin + Point::new(pixel.0.x * scale, pixel.0.y * scale),
                Size::new(LOGO_SCALE, LOGO_SCALE),
            )
            .into_styled(fill)
            .draw(display)
            .unwrap();
        }
    }
}

/// Draws the Ferris and Rust logos stacked vertically, horizontally centred.
///
/// `swapped` exchanges which logo occupies the upper slot: Phase 2 flips it on every differential
/// update so the refresh is obvious at a glance. Ferris is 64x42 and Rust is 64x64 before scaling,
/// and the offsets are chosen so both arrangements span the same [`LOGO_TOP_Y`]..[`LOGO_BOTTOM_Y`].
fn draw_logos(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
    swapped: bool,
) {
    let ferris_h = (ferris_bmp.size().height * LOGO_SCALE) as i32;
    let rust_h = (rust_bmp.size().height * LOGO_SCALE) as i32;

    let (ferris_y, rust_y) = if swapped {
        (LOGO_BOTTOM_Y - ferris_h, LOGO_TOP_Y)
    } else {
        (LOGO_TOP_Y, LOGO_BOTTOM_Y - rust_h)
    };

    // The Ferris BMP has the opposite polarity to the Rust BMP, hence the differing `ink`.
    draw_scaled(
        display,
        ferris_bmp,
        Point::new(LOGO_X, ferris_y),
        BinaryColor::Off,
    );
    draw_scaled(
        display,
        rust_bmp,
        Point::new(LOGO_X, rust_y),
        BinaryColor::On,
    );
}

/// Renders the complete portrait frame: header bar, outer border, both logos, footer labels,
/// update counter, and progress bar.
fn draw_frame(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
    swapped: bool,
    count: u32,
) {
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    // Header bar. Height HEADER_H + 1 puts its bottom edge exactly on the content area's top
    // edge, so the two rectangles meet on a single line rather than doubling up.
    Rectangle::new(Point::new(0, 0), Size::new(VIEW_W, HEADER_H + 1))
        .into_styled(stroke)
        .draw(display)
        .unwrap();

    Text::new("GDEQ0426T82  4.26\"", Point::new(24, 38), text_style)
        .draw(display)
        .unwrap();

    // Content area border
    Rectangle::new(
        Point::new(0, HEADER_H as i32),
        Size::new(VIEW_W, VIEW_H - HEADER_H),
    )
    .into_styled(stroke)
    .draw(display)
    .unwrap();

    draw_logos(display, ferris_bmp, rust_bmp, swapped);

    // Separator above the footer
    Line::new(Point::new(24, 620), Point::new(455, 620))
        .into_styled(stroke)
        .draw(display)
        .unwrap();

    Text::new("XIAO ESP32-C3", Point::new(24, 656), text_style)
        .draw(display)
        .unwrap();

    Text::new("epdsi SSD1677", Point::new(24, 682), text_style)
        .draw(display)
        .unwrap();

    // Update counter label
    let mut count_buf = [0u8; 32];
    let count_str = format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap();
    Text::new(count_str, Point::new(24, 720), text_style)
        .draw(display)
        .unwrap();

    // Progress bar outline
    Rectangle::new(Point::new(24, 732), Size::new(432, 28))
        .into_styled(stroke)
        .draw(display)
        .unwrap();

    // Progress bar fill
    Rectangle::new(Point::new(27, 735), Size::new(count * 71, 22))
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(display)
        .unwrap();
}

/// 48,000-byte frame buffer. Static rather than stack-allocated: the ESP32-C6's default
/// stack is well under 48 KB, so the RP2350 version's `let mut bw_buf = [0xFF; ...]`
/// would overflow it.
static mut BW_BUF: [u8; FRAME_BYTES] = [0xFFu8; FRAME_BYTES];

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();

    delay.delay_ms(100);
    esp_println::println!("Starting GDEQ0426T82 4.26\" Monochrome EPD example (epdsi SSD1677)");

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

    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Ssd1677Controller::new(GDEQ0426T82::WIDTH, GDEQ0426T82::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEQ0426T82>::new(controller).build(epd_bus);

    esp_println::println!("Initializing SSD1677 epdsi EPD driver...");
    epd.init(&mut delay).unwrap();

    // Clear both RAM banks to white. On this monochrome panel the secondary RAM (0x26) is
    // not a colour plane but the "previous image" used by differential updates. Each write
    // starts from the window origin and leaves the address counter where it stopped, so
    // re-assert window + cursor before each bank.
    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.clear_frame(ColorChannel::BlackWhite, 0xFF).unwrap();

    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0xFF).unwrap();

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    // SAFETY: single-threaded example, and this is the only reference taken to BW_BUF.
    let bw_buf: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(BW_BUF) };

    // The buffer keeps the panel's native 800x480 RAM geometry; Rotate270 turns it into a
    // 480x800 portrait drawing surface with the FPC ribbon at the bottom.
    let mut display = PageBuffer::new(
        bw_buf,
        GDEQ0426T82::WIDTH,
        GDEQ0426T82::HEIGHT,
        0,
    );
    display.set_rotation(DisplayRotation::Rotate270);

    esp_println::println!("--- Phase 1: Full Monochrome Refresh ---");
    draw_frame(&mut display, &ferris_bmp, &rust_bmp, false, 0);

    esp_println::println!("Sending Black/White frame (48,000 bytes)...");
    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .unwrap();

    esp_println::println!("Refreshing (full waveform, expect several seconds)...");
    epd.refresh(&mut delay).unwrap();

    // Seed the "previous image" RAM (0x26) with what is now physically on the panel, so the
    // Phase 2 differential updates have a correct base to diff against.
    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::RedYellow, display.as_slice())
        .unwrap();

    delay.delay_ms(2000);

    esp_println::println!("--- Phase 2: Fast Differential Refresh (logo swap) ---");
    epd.controller_mut()
        .set_refresh_mode(Ssd1677RefreshMode::Partial);

    for count in 1..=6u32 {
        let swapped = count % 2 == 1;

        display.clear_byte(0xFF);
        draw_frame(&mut display, &ferris_bmp, &rust_bmp, swapped, count);

        epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
            .unwrap();
        epd.set_cursor(0, 0).unwrap();
        epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
            .unwrap();

        esp_println::println!(
            "Differential refresh (Update #{}, logos {})...",
            count,
            if swapped { "swapped" } else { "normal" }
        );
        epd.refresh(&mut delay).unwrap();

        // Copy the frame just displayed into the "previous image" RAM so the next iteration
        // diffs against what is actually on the panel.
        epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
            .unwrap();
        epd.set_cursor(0, 0).unwrap();
        epd.write_frame(ColorChannel::RedYellow, display.as_slice())
            .unwrap();

        delay.delay_ms(1000);
    }

    esp_println::println!("--- Phase 3: Full-Waveform Cleanup Pass ---");

    // Differential updates use a much shorter waveform than the OTP full-refresh LUT, so
    // pixels that flipped white -> black in Phase 2 settle at dark grey rather than deep
    // black. Re-running the final frame through the full waveform evens the ink density.
    // Blanking the secondary RAM first stops it being read as a second colour plane.
    epd.controller_mut()
        .set_refresh_mode(Ssd1677RefreshMode::Full);

    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0xFF).unwrap();

    epd.set_window(0, 0, GDEQ0426T82::WIDTH - 1, GDEQ0426T82::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .unwrap();

    esp_println::println!("Refreshing with the full OTP waveform...");
    epd.refresh(&mut delay).unwrap();

    esp_println::println!("Display complete!");

    loop {
        delay.delay_ms(1000);
    }
}
