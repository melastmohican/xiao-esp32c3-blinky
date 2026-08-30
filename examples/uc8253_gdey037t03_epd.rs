//! # Good Display GDEY037T03 3.7" Monochrome E-Paper Example (`epdsi`)
//!
//! > **This panel does not work on the XIAO ESP32-C3.** The port is believed correct —
//! > it is kept for use on a board that does work, and to document the finding. On the
//! > C3 the panel is connected, powered and alive (BUSY is driven and asserts on reset)
//! > but never acts on SPI commands, at any clock from 4 MHz down to 100 kHz. Stock
//! > Arduino GxEPD2 fails identically on this board, while the same panel and adapter
//! > work on XIAO MG24 and nRF52840. See `BRINGUP.md`.
//!
//! Port of the Raspberry Pi Pico 2 example from
//! [`rust-rpico2-discovery`](https://github.com/melastmohican/rust-rpico2-discovery)
//! to the Seeed Studio XIAO ESP32-C3 on the ePaper Driver Board for XIAO.
//!
//! Everything from `STRIDE` down to the end of `draw_content` is unchanged from the
//! RP2350 version — the `epdsi` types, `PageBuffer`, and the `embedded-graphics` drawing
//! are HAL-agnostic. Only `main` differs.
//!
//! Demonstrates:
//! 1. **Phase 1**: Full monochrome refresh — header, separator, Ferris and Rust side by
//!    side, text labels.
//! 2. **Phase 2**: Partial-window loop repainting only the content band (y = 66..415),
//!    swapping the logos each pass and advancing a progress bar. The header above the
//!    band is never touched.
//! 3. **Phase 3**: Full-panel, full-waveform cleanup restoring the ink density the
//!    shortened partial waveform leaves behind.
//!
//! ## The UC8253 command model differs from the SSD16xx family
//!
//! - The RAM area must be set before writing image data, and the partial window is
//!   re-opened around *every* RAM write and again around the refresh
//!   (`PARTIAL_IN` → `PARTIAL_WINDOW` → operation → `PARTIAL_OUT`). `set_window` records
//!   the area; `epdsi` emits the commands per operation. There is no `set_cursor` call.
//! - The two RAM banks are **old/new planes, not colours**:
//!   [`ColorChannel::BlackWhite`] maps to `WRITE_NEW_DATA` (`0x13`) and
//!   [`ColorChannel::RedYellow`] to `WRITE_OLD_DATA` (`0x10`). The old plane is primed to
//!   white before the first write.
//! - **BUSY is active-LOW**, the opposite of the SSD16xx panels, so the GPIO takes a
//!   pull-**up**. See `BRINGUP.md`.
//! - Buffer coordinates map straight to the panel, `(0,0)` at top-left with the FPC
//!   ribbon at the bottom. No rotation or mirroring.
//!
//! ## Hardware
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** [ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html) (24-pin FPC)
//! - **Display:** Good Display GDEY037T03, 3.7" monochrome, 240x416 (Adafruit 6395)
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
//! cargo run --release --example uc8253_gdey037t03_epd
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

/// Row stride in bytes: 240 / 8 = 30. This panel is already byte-aligned.
const STRIDE: usize = GDEY037T03::WIDTH.div_ceil(8) as usize;

/// Full frame buffer size: 30 x 416 = 12,480 bytes.
const FRAME_BYTES: usize = STRIDE * GDEY037T03::HEIGHT as usize;

/// Top Y coordinate of the content band repainted in Phase 2.
const BAND_Y: u32 = 66;

/// Height of the content band in pixels (y = 66..415).
const BAND_H: u32 = 350;

/// Last row of the band.
const BAND_END: u32 = BAND_Y + BAND_H - 1;

/// Byte offset of the band's first row within the frame buffer.
const BAND_START_BYTE: usize = BAND_Y as usize * STRIDE;

/// Byte offset one past the band's last row.
const BAND_END_BYTE: usize = (BAND_END as usize + 1) * STRIDE;

/// X coordinate of the left logo slot.
const LOGO_LEFT_X: i32 = 30;

/// X coordinate of the right logo slot.
const LOGO_RIGHT_X: i32 = 146;

/// Draws the static chrome above the Phase 2 band: border, header, subtitle and separator.
fn draw_chrome(display: &mut PageBuffer) {
    let style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    Rectangle::new(
        Point::new(0, 0),
        Size::new(GDEY037T03::WIDTH, GDEY037T03::HEIGHT),
    )
    .into_styled(style)
    .draw(display)
    .unwrap();

    Text::new("GDEY037T03", Point::new(10, 24), text_style)
        .draw(display)
        .unwrap();

    Text::new("3.7\" Mono", Point::new(10, 48), text_style)
        .draw(display)
        .unwrap();

    Line::new(Point::new(10, 58), Point::new(229, 58))
        .into_styled(style)
        .draw(display)
        .unwrap();
}

/// Draws everything inside the Phase 2 band: the two logos, the footer labels and the progress
/// indicator.
///
/// `swapped` exchanges the two logo slots — Phase 2 flips it on every pass so the partial update
/// is obvious at a glance. Ferris is 64x42 and Rust is 64x64, and their Y positions differ so
/// their bottoms line up. `count` of 0 renders the Phase 1 state instead of an update counter.
fn draw_content(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
    swapped: bool,
    count: u32,
) {
    let style = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    let (ferris_x, rust_x) = if swapped {
        (LOGO_RIGHT_X, LOGO_LEFT_X)
    } else {
        (LOGO_LEFT_X, LOGO_RIGHT_X)
    };

    // The Ferris BMP has the opposite polarity to the Rust BMP, hence the `Off` test here.
    let ferris_pos = Point::new(ferris_x, 112);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }

    let rust_pos = Point::new(rust_x, 90);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }

    Text::new("XIAO ESP32-C3", Point::new(10, 200), text_style)
        .draw(display)
        .unwrap();

    Text::new("epdsi UC8253", Point::new(10, 225), text_style)
        .draw(display)
        .unwrap();

    Line::new(Point::new(10, 240), Point::new(229, 240))
        .into_styled(style)
        .draw(display)
        .unwrap();

    let mut count_buf = [0u8; 32];
    let label = if count == 0 {
        "Full refresh"
    } else {
        format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap()
    };
    Text::new(label, Point::new(10, 285), text_style)
        .draw(display)
        .unwrap();

    Rectangle::new(Point::new(10, 300), Size::new(220, 22))
        .into_styled(style)
        .draw(display)
        .unwrap();

    // Progress bar fill: 6 steps of 35 px stay inside the 216 px interior
    if count > 0 {
        Rectangle::new(Point::new(12, 303), Size::new(count * 35, 16))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(display)
            .unwrap();
    }
}

/// Frame buffer: 30 bytes per row x 416 rows = 12,480 bytes. Static rather than
/// stack-allocated, matching the other examples here. `0xFF` is white.
static mut BW_BUF: [u8; FRAME_BYTES] = [0xFFu8; FRAME_BYTES];

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(100);

    esp_println::println!("Starting GDEY037T03 3.7\" Monochrome EPD example (epdsi UC8253)");

    // Pin assignments are fixed by the ePaper Driver Board for XIAO.
    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    // UC8253 BUSY is active-LOW, unlike the SSD16xx panels: pull up so a missing or
    // unpowered panel reads "idle" rather than "busy forever".
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
    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Uc8253Controller::new(GDEY037T03::WIDTH, GDEY037T03::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEY037T03>::new(controller).build(epd_bus);

    esp_println::println!("Initializing UC8253 epdsi EPD driver...");
    epd.init(&mut delay).unwrap();

    // Prime the old plane to white so the first update has a clean base.
    epd.clear_frame(ColorChannel::RedYellow, 0xFF).unwrap();

    // SAFETY: single-threaded example, and this is the only reference taken to BW_BUF.
    let bw_buf: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(BW_BUF) };

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    // Buffer coordinates map straight to the panel with the FPC ribbon at the bottom:
    // (0,0) is the top-left of the visible image. No rotation or mirroring needed.
    let mut display = PageBuffer::new(&mut bw_buf[..], GDEY037T03::WIDTH, GDEY037T03::HEIGHT, 0);

    esp_println::println!("--- Phase 1: Full Monochrome Refresh ---");

    draw_chrome(&mut display);
    draw_content(&mut display, &ferris_bmp, &rust_bmp, false, 0);

    esp_println::println!("Sending Black/White frame (12,480 bytes)...");
    epd.set_window(0, 0, GDEY037T03::WIDTH - 1, GDEY037T03::HEIGHT - 1)
        .unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .unwrap();

    esp_println::println!("Refreshing display hardware (Full refresh)...");
    epd.refresh(&mut delay).unwrap();

    // Sync the old plane with what is now on the panel.
    epd.set_window(0, 0, GDEY037T03::WIDTH - 1, GDEY037T03::HEIGHT - 1)
        .unwrap();
    epd.write_frame(ColorChannel::RedYellow, display.as_slice())
        .unwrap();

    delay.delay_ms(2000);

    esp_println::println!("--- Phase 2: Partial Window Refresh (logo swap) ---");

    // GxEPD2 declares `hasFastPartialUpdate = true` for this panel, so its partial path
    // always applies the CCSET/TSSET temperature override -- that is `FastPartial` here,
    // not `Partial`.
    epd.controller_mut()
        .set_refresh_mode(Uc8253RefreshMode::FastPartial);

    for count in 1..=6u32 {
        let swapped = count % 2 == 1;

        // The whole buffer is redrawn, but only the band's rows are sent, so the chrome
        // above the band is never repainted.
        display.clear_byte(0xFF);
        draw_chrome(&mut display);
        draw_content(&mut display, &ferris_bmp, &rust_bmp, swapped, count);

        epd.set_window(0, BAND_Y, GDEY037T03::WIDTH - 1, BAND_END)
            .unwrap();
        epd.write_frame(
            ColorChannel::BlackWhite,
            &display.as_slice()[BAND_START_BYTE..BAND_END_BYTE],
        )
        .unwrap();

        esp_println::println!(
            "Refreshing band y={}..{} (Update #{}, logos {})...",
            BAND_Y,
            BAND_END,
            count,
            if swapped { "swapped" } else { "normal" }
        );
        epd.refresh(&mut delay).unwrap();

        // Keep the old plane in step with the panel for the next differential pass.
        epd.set_window(0, BAND_Y, GDEY037T03::WIDTH - 1, BAND_END)
            .unwrap();
        epd.write_frame(
            ColorChannel::RedYellow,
            &display.as_slice()[BAND_START_BYTE..BAND_END_BYTE],
        )
        .unwrap();

        delay.delay_ms(1000);
    }

    esp_println::println!("--- Phase 3: Full-Waveform Cleanup Pass ---");

    // The partial waveform settles pixels at dark grey rather than deep black. Redraw the
    // whole frame and run it through the full waveform to restore even ink density.
    epd.controller_mut()
        .set_refresh_mode(Uc8253RefreshMode::Full);

    display.clear_byte(0xFF);
    draw_chrome(&mut display);
    draw_content(&mut display, &ferris_bmp, &rust_bmp, false, 6);

    epd.set_window(0, 0, GDEY037T03::WIDTH - 1, GDEY037T03::HEIGHT - 1)
        .unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .unwrap();

    esp_println::println!("Refreshing full panel with the full waveform...");
    epd.refresh(&mut delay).unwrap();

    esp_println::println!("Display complete!");

    loop {
        delay.delay_ms(1000);
    }
}
