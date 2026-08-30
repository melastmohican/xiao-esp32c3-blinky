//! # 3.52" SE0352N14 single-shot diagnostic (`epdsi`, UC8253)
//!
//! > ## Do not loop this panel
//! >
//! > Waveshare specify **at least 180 s between refreshes**, and at least one refresh every 24 h.
//! > It is an ESL/signage part, not something to cycle continuously. An earlier version of this
//! > file ran seven full refreshes roughly **19 s** apart — about ten times the rated rate — to
//! > measure a failure rate. The panel rendered cleanly at first, degraded into streaked bands
//! > partway through, and then produced nothing but bands on the following run.
//! > **That was the test damaging the panel, not the panel revealing a fault.**
//! >
//! > If that has happened, leave the panel unpowered and idle for several hours before judging it.
//! > Over-refresh degradation is usually recoverable; heat and repeated same-direction drives are
//! > not something a driver change can compensate for.
//!
//! One `init` -> write -> refresh, then stop. About 20 seconds.
//!
//! ## What it shows
//!
//! A framed **white | black | red** pattern with "PLANE TEST" upright at the top:
//!
//! - Black and red swapped -> RAM plane routing crossed (`0x10`/`0x13`).
//! - White and black swapped -> `CDI`/DDX polarity, or buffers not `0x00`-based.
//! - Text mirrored or at the bottom -> the rotation in `frame()` is wrong for this mounting.
//! - A cleared panel reading grey rather than white -> this panel's white point, not a fault.
//! - Streaked bands over everything -> see the warning above before blaming the driver.
//!
//! ## Reading the timing
//!
//! A real full refresh is **~17 s**.
//!
//! | Elapsed | Meaning |
//! |---|---|
//! | ~16-20 s | Refreshed. |
//! | under ~1.5 s | The controller ignored `POWER_ON` and `DISPLAY_REFRESH` and drew nothing — BUSY never asserted for either. |
//! | near 0 ms | BUSY was polled before the controller asserted it; the update is still running in the background. |
//!
//! To judge reliability, run this **once**, wait at least 180 s, then run it again — and tally by
//! hand. A panel with a 180 s minimum interval cannot be characterised by a loop.
//!
//! ## Hardware
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** ePaper Driver Board for XIAO (24-pin FPC)
//! - **Display:** Waveshare 3.52inch e-Paper (B), `SE0352N14-TNG-A0`, 240x360 BWR
//!
//! Pins are fixed by the driver board: RST D0/GPIO2, CS D1/GPIO3, BUSY D2/GPIO4, DC D3/GPIO5,
//! SCK D8/GPIO8, MOSI D10/GPIO10.
//!
//! ## Run — flash first, power-cycle second
//!
//! Power-up **is** program-start on this board, so replugging first just runs the previous binary
//! and flashing then interrupts it mid-refresh — the latched state the power-cycle was meant to
//! avoid. `hard_reset` does not clear that; only removing power does (`BRINGUP.md`).
//!
//! ```bash
//! cargo run --release --example epd_diag_352   # flash. Ignore this run: the panel is dirty.
//! # unplug USB, wait a few seconds, plug back in
//! espflash monitor                             # attach during the startup hold
//! ```

#![no_std]
#![no_main]

use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
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
use esp_hal::time::{Instant, Rate};

esp_bootloader_esp_idf::esp_app_desc!();

/// Ink on this panel is a **set** bit in both planes, where `PageBuffer` treats a *cleared* bit as
/// ink. So drawing uses `BinaryColor::Off` against `0x00`-based buffers.
const INK: BinaryColor = BinaryColor::Off;

/// Clears a plane to white on this panel. Not `0xFF`.
const WHITE_BYTE: u8 = 0x00;

/// Row stride in bytes: 240 / 8 = 30.
const STRIDE: usize = SE0352N14TNGA0::WIDTH.div_ceil(8) as usize;

/// One plane: 30 x 360 = 10,800 bytes.
const FRAME_BYTES: usize = STRIDE * SE0352N14TNGA0::HEIGHT as usize;

/// Seconds to wait before touching the panel, so a monitor can be attached after a power-cycle.
///
/// Power-up is program-start on this board, so this hold is the only way to observe a run that
/// began from a genuinely power-cycled panel — see the module docs.
const STARTUP_HOLD_S: u32 = 8;

/// Shortest elapsed time that can represent a real refresh. The panel takes ~17 s.
const MIN_REAL_REFRESH_MS: u64 = 5_000;

static mut BW_BUF: [u8; FRAME_BYTES] = [WHITE_BYTE; FRAME_BYTES];
static mut RED_BUF: [u8; FRAME_BYTES] = [WHITE_BYTE; FRAME_BYTES];

/// Builds a full-frame drawing surface, rotated so the image bottom sits at the FPC ribbon,
/// matching every other example in this repo.
fn frame(buf: &mut [u8]) -> PageBuffer<'_> {
    let mut page = PageBuffer::new(buf, SE0352N14TNGA0::WIDTH, SE0352N14TNGA0::HEIGHT, 0);
    page.set_rotation(DisplayRotation::Rotate180);
    page
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(100);

    esp_println::println!("=== SE0352N14 3.52\" single-shot diagnostic (epdsi UC8253) ===");
    esp_println::println!("One refresh only. This panel is rated for roughly daily updates.");

    // Hold before touching the panel, so `espflash monitor` can be attached after a replug. A run
    // watched without this window is one that flashing interrupted.
    for remaining in (1..=STARTUP_HOLD_S).rev() {
        esp_println::println!("starting in {} s (attach monitor now)", remaining);
        delay.delay_ms(1000);
    }

    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    // UC8253 BUSY is active-LOW: pull up so a missing panel reads idle, not busy forever.
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
    let controller = Uc8253Controller::new(SE0352N14TNGA0::WIDTH, SE0352N14TNGA0::HEIGHT)
        .with_variant(Uc8253Variant::Se0352n14);
    let mut epd = EpdBuilder::<_, SE0352N14TNGA0>::new(controller).build(epd_bus);

    // SAFETY: single-threaded example, and these are the only references taken to the buffers.
    let bw_buf: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(BW_BUF) };
    let red_buf: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(RED_BUF) };

    {
        let mut display_bw = frame(&mut bw_buf[..]);
        let text_style = MonoTextStyle::new(&FONT_10X20, INK);

        // Doubles as the orientation check: must read upright, at the top, ribbon at the bottom.
        Text::new("PLANE TEST", Point::new(10, 30), text_style)
            .draw(&mut display_bw)
            .unwrap();

        // Frame around the pattern, so the white band has a visible edge. Interior is 216 x 238.
        Rectangle::new(Point::new(11, 50), Size::new(218, 240))
            .into_styled(PrimitiveStyle::with_stroke(INK, 1))
            .draw(&mut display_bw)
            .unwrap();

        // Middle third black, inset one pixel so it abuts the frame instead of overlapping it.
        Rectangle::new(Point::new(84, 51), Size::new(72, 238))
            .into_styled(PrimitiveStyle::with_fill(INK))
            .draw(&mut display_bw)
            .unwrap();

        for (label, centre_x) in [("W", 48), ("B", 120), ("R", 192)] {
            Text::new(label, Point::new(centre_x - 5, 320), text_style)
                .draw(&mut display_bw)
                .unwrap();
        }
    }
    {
        let mut display_red = frame(&mut red_buf[..]);
        // Right third red. Must not touch the frame: a pixel set in both planes has no defined
        // colour.
        Rectangle::new(Point::new(156, 51), Size::new(72, 238))
            .into_styled(PrimitiveStyle::with_fill(INK))
            .draw(&mut display_red)
            .unwrap();
    }

    esp_println::println!("init + write + one refresh (expect ~17 s)...");
    epd.init(&mut delay).unwrap();

    // No set_window: a full-frame write must not be wrapped in a partial-window session.
    epd.write_frame(ColorChannel::BlackWhite, &bw_buf[..])
        .unwrap();
    epd.write_frame(ColorChannel::RedYellow, &red_buf[..])
        .unwrap();

    let start = Instant::now();
    epd.refresh(&mut delay).unwrap();
    let ms = (Instant::now() - start).as_millis();

    if ms >= MIN_REAL_REFRESH_MS {
        esp_println::println!("refresh: {} ms  OK", ms);
        esp_println::println!(
            "Panel should read white | black | red, 'PLANE TEST' upright at top."
        );
    } else {
        esp_println::println!("refresh: {} ms  FAILED (expected ~17 s)", ms);
        esp_println::println!("  BUSY never asserted: the controller ignored POWER_ON and");
        esp_println::println!("  DISPLAY_REFRESH and drew nothing.");
    }

    esp_println::println!("Done. Let the panel rest before running again.");
    epd.sleep(&mut delay).unwrap();

    loop {
        delay.delay_ms(1000);
    }
}
