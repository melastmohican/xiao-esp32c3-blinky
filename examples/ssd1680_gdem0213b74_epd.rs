//! # Good Display GDEM0213B74 2.13" Monochrome E-Paper Example (`epdsi`)
//!
//! Port of the Raspberry Pi Pico 2 example from
//! [`rust-rpico2-discovery`](https://github.com/melastmohican/rust-rpico2-discovery)
//! to the Seeed Studio XIAO ESP32-C3 on the ePaper Driver Board for XIAO.
//!
//! Everything from `STRIDE` down to the end of `draw_logos` is unchanged from the RP2350
//! version — the `epdsi` panel and controller types, `PageBuffer`, and the
//! `embedded-graphics` drawing are HAL-agnostic. Only `main` differs.
//!
//! Demonstrates:
//! 1. **Phase 1**: Full monochrome refresh — border, header, Ferris and Rust logos,
//!    footer labels.
//! 2. **Phase 2**: Fast *partial window* refresh. Only a 122x200 band is rewritten, with
//!    the logos swapping each pass and a progress bar advancing. The header above the
//!    band is never re-sent, so it cannot flicker.
//! 3. **Phase 3**: Full-waveform cleanup pass over the band, restoring the ink density
//!    the shortened differential waveform leaves behind.
//!
//! ## Hardware
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** [ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html) (24-pin FPC)
//! - **Display:** Good Display GDEM0213B74, 2.13" monochrome, 122x250 (Adafruit 6383)
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
//! cargo run --release --example ssd1680_gdem0213b74_epd
//! ```

#![no_std]
#![no_main]

use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::mono_font::ascii::{FONT_10X20, FONT_6X10};
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

/// Row stride in bytes. 122 px rounds up to 16 bytes; see the module docs.
const STRIDE: usize = GDEM0213B74::WIDTH.div_ceil(8) as usize;

/// Full frame buffer size: 16 x 250 = 4,000 bytes.
const FRAME_BYTES: usize = STRIDE * GDEM0213B74::HEIGHT as usize;

/// Top Y coordinate of the content band repainted in Phase 2. The header and the separator
/// above it (y = 0..49) are painted once in Phase 1 and never touched again.
const BAND_Y: u32 = 50;

/// Height of the content band in pixels (y = 50..249).
const BAND_H: u32 = 200;

/// Content band buffer size: 16 x 200 = 3,200 bytes.
const BAND_BYTES: usize = STRIDE * BAND_H as usize;

/// All-white fill for the content band, used to blank the secondary RAM before the Phase 3
/// cleanup pass. Lives in flash rather than on the stack.
static WHITE_BAND: [u8; BAND_BYTES] = [0xFFu8; BAND_BYTES];

/// X coordinate that horizontally centres a 64 px logo on the 122 px panel.
const LOGO_X: i32 = (GDEM0213B74::WIDTH as i32 - 64) / 2;

/// Top Y coordinate of the upper logo slot.
const LOGO_TOP_Y: i32 = 52;

/// Draws the Ferris and Rust logos stacked vertically, horizontally centred on the panel.
///
/// The two 64 px-wide logos cannot sit side by side on a 122 px panel, so they are stacked.
/// `swapped` exchanges which logo occupies the upper slot: Phase 2 flips it on every partial
/// update so the differential refresh is obvious at a glance. Ferris is 64x42 and Rust is
/// 64x64, and the offsets are chosen so both arrangements end at y = 161.
fn draw_logos(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
    swapped: bool,
) {
    let (ferris_y, rust_y) = if swapped {
        (LOGO_TOP_Y + 68, LOGO_TOP_Y)
    } else {
        (LOGO_TOP_Y, LOGO_TOP_Y + 46)
    };

    // The Ferris BMP has the opposite polarity to the Rust BMP, hence the `Off` test here.
    let ferris_pos = Point::new(LOGO_X, ferris_y);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }

    let rust_pos = Point::new(LOGO_X, rust_y);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }
}

/// Full-frame buffer, 16 bytes per row x 250 rows = 4,000 bytes. Static rather than
/// stack-allocated, matching the other examples here. `0xFF` is white.
static mut BW_BUF: [u8; FRAME_BYTES] = [0xFFu8; FRAME_BYTES];

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(100);

    esp_println::println!("Starting GDEM0213B74 2.13\" Monochrome EPD example (epdsi SSD1680)");

    // Pin assignments are fixed by the ePaper Driver Board for XIAO.
    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    // SSD1680 BUSY is active-HIGH, so pull down: a floating line reads "idle".
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

    esp_println::println!("Initializing SSD1680 epdsi EPD driver...");
    epd.init(&mut delay).unwrap();

    // Clear both RAM banks to white. On this monochrome panel the secondary RAM (0x26)
    // is not a colour plane but the "previous image" used by differential updates.
    epd.clear_frame(ColorChannel::BlackWhite, 0xFF).unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0xFF).unwrap();

    // SAFETY: single-threaded example, and this is the only reference taken to BW_BUF.
    let bw_buf: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(BW_BUF) };

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    let style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);
    // The panel is only 122 px wide, so the footer labels use the smaller 6x10 font.
    let small_text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    esp_println::println!("--- Phase 1: Full Monochrome Refresh ---");

    // Scoped so the full-frame borrow of `bw_buf` ends before Phase 2 re-borrows it as a
    // smaller sub-region buffer.
    {
        let mut display =
            PageBuffer::new(&mut bw_buf[..], GDEM0213B74::WIDTH, GDEM0213B74::HEIGHT, 0);

        Rectangle::new(
            Point::new(0, 0),
            Size::new(GDEM0213B74::WIDTH, GDEM0213B74::HEIGHT),
        )
        .into_styled(style)
        .draw(&mut display)
        .unwrap();

        Text::new("GDEM0213B74", Point::new(6, 18), text_style)
            .draw(&mut display)
            .unwrap();

        Text::new("2.13\" Mono", Point::new(6, 38), text_style)
            .draw(&mut display)
            .unwrap();

        Line::new(Point::new(6, 45), Point::new(115, 45))
            .into_styled(style)
            .draw(&mut display)
            .unwrap();

        // Ferris on top, Rust below. Phase 2 swaps them on every partial update.
        draw_logos(&mut display, &ferris_bmp, &rust_bmp, false);

        Text::new("XIAO ESP32-C3", Point::new(6, 180), small_text_style)
            .draw(&mut display)
            .unwrap();

        Text::new("epdsi SSD1680", Point::new(6, 195), small_text_style)
            .draw(&mut display)
            .unwrap();

        Line::new(Point::new(6, 203), Point::new(115, 203))
            .into_styled(style)
            .draw(&mut display)
            .unwrap();

        // Each RAM write starts from the window origin, so reset window + cursor first.
        esp_println::println!("Sending Black/White frame (4,000 bytes)...");
        epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
            .unwrap();
        epd.set_cursor(0, 0).unwrap();
        epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
            .unwrap();

        esp_println::println!("Refreshing display hardware (Full refresh)...");
        epd.refresh(&mut delay).unwrap();

        // Seed the "previous image" RAM (0x26) with what is now physically on the panel,
        // so Phase 2's differential updates have a correct base to diff against.
        epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
            .unwrap();
        epd.set_cursor(0, 0).unwrap();
        epd.write_frame(ColorChannel::RedYellow, display.as_slice())
            .unwrap();
    }

    delay.delay_ms(2000);

    esp_println::println!("--- Phase 2: Fast Partial Window Refresh (logo swap) ---");

    // Select the SSD1680 built-in fast LUT (0x22 = 0xFC). On this monochrome panel that
    // is a real differential update and completes in well under a second.
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Partial);

    for count in 1..=6u32 {
        let swapped = count % 2 == 1;

        // Sub-region buffer: 16 x 200 = 3,200 bytes of the full-frame array. The
        // y_offset argument keeps embedded-graphics coordinates in full-panel space.
        let mut band = PageBuffer::new(
            &mut bw_buf[..BAND_BYTES],
            GDEM0213B74::WIDTH,
            BAND_H,
            BAND_Y,
        );
        band.clear_byte(0xFF);

        draw_logos(&mut band, &ferris_bmp, &rust_bmp, swapped);

        // Footer labels and separator, redrawn identically every pass. Differential mode
        // sees no change here, so they stay steady while the logos above them swap.
        Text::new("XIAO ESP32-C3", Point::new(6, 180), small_text_style)
            .draw(&mut band)
            .unwrap();

        Text::new("epdsi SSD1680", Point::new(6, 195), small_text_style)
            .draw(&mut band)
            .unwrap();

        Line::new(Point::new(6, 203), Point::new(115, 203))
            .into_styled(style)
            .draw(&mut band)
            .unwrap();

        let mut count_buf = [0u8; 32];
        let count_str =
            format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap();
        Text::new(count_str, Point::new(6, 224), text_style)
            .draw(&mut band)
            .unwrap();

        Rectangle::new(Point::new(6, 230), Size::new(110, 14))
            .into_styled(style)
            .draw(&mut band)
            .unwrap();

        Rectangle::new(Point::new(8, 232), Size::new(count * 17, 10))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(&mut band)
            .unwrap();

        // Restrict controller RAM to the band, then write the new image to B/W RAM.
        epd.set_window(0, BAND_Y, GDEM0213B74::WIDTH - 1, BAND_Y + BAND_H - 1)
            .unwrap();
        epd.set_cursor(0, BAND_Y).unwrap();
        epd.write_frame(ColorChannel::BlackWhite, band.as_slice())
            .unwrap();

        esp_println::println!(
            "Refreshing band y={}..{} (Update #{}, logos {})...",
            BAND_Y,
            BAND_Y + BAND_H - 1,
            count,
            if swapped { "swapped" } else { "normal" }
        );
        epd.refresh(&mut delay).unwrap();

        // Copy the band just displayed into the "previous image" RAM so the next
        // iteration diffs against what is actually on the panel.
        epd.set_window(0, BAND_Y, GDEM0213B74::WIDTH - 1, BAND_Y + BAND_H - 1)
            .unwrap();
        epd.set_cursor(0, BAND_Y).unwrap();
        epd.write_frame(ColorChannel::RedYellow, band.as_slice())
            .unwrap();

        delay.delay_ms(1000);
    }

    esp_println::println!("--- Phase 3: Full-Waveform Cleanup Pass ---");

    // Differential updates use a much shorter waveform than the OTP full-refresh LUT, so
    // pixels that flipped white -> black in Phase 2 settle at dark grey rather than deep
    // black. Re-running the final band through the full waveform evens the ink density.
    // Blanking the secondary RAM first stops it being read as a second colour plane.
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Full);

    epd.set_window(0, BAND_Y, GDEM0213B74::WIDTH - 1, BAND_Y + BAND_H - 1)
        .unwrap();
    epd.set_cursor(0, BAND_Y).unwrap();
    epd.write_frame(ColorChannel::RedYellow, &WHITE_BAND)
        .unwrap();

    // `bw_buf` still holds the last band drawn in Phase 2, so re-send it unchanged.
    epd.set_window(0, BAND_Y, GDEM0213B74::WIDTH - 1, BAND_Y + BAND_H - 1)
        .unwrap();
    epd.set_cursor(0, BAND_Y).unwrap();
    epd.write_frame(ColorChannel::BlackWhite, &bw_buf[..BAND_BYTES])
        .unwrap();

    esp_println::println!("Refreshing band with the full OTP waveform...");
    epd.refresh(&mut delay).unwrap();

    // Restore the full-frame RAM window for any subsequent updates.
    epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();

    esp_println::println!("Display complete!");

    loop {
        delay.delay_ms(1000);
    }
}
