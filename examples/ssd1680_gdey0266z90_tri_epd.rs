//! # GDEY0266Z90 2.66" Tri-Color `PageBufferPair` Draw Target Example (`epdsi`)
//!
//! Full-parity companion to `ssd1680_gdey0266z90_epd` on this board — same four phases, same
//! content, same timing measurements, same board bring-up (esp-hal, GPIO wiring, logging) — but
//! drawn entirely through `PageBufferPair`/`TriColor` instead of two separate `PageBuffer`s and
//! panel-specific `BinaryColor::On`/`Off` polarity choices. Exists to prove the new draw-target
//! API can do everything the manual approach does, not just a minimal smoke test.
//!
//! 1. **Phase 1**: Full tri-color refresh ([`Ssd168xRefreshMode::Full`]) — header, Black and
//!    Accent swatches, Ferris logo (Accent), Rust logo (Black), and text labels.
//! 2. **Phase 2**: Partial *window* refresh loop on the full waveform, repainting only the bottom
//!    status band, leaving the logos untouched.
//! 3. **Phase 3**: [`Ssd168xRefreshMode::FastFull`], timed against Phase 1.
//! 4. **Phase 4**: [`Ssd168xRefreshMode::BaseMap`] and [`Ssd168xRefreshMode::Partial`], the two
//!    modes ported from Good Display's reference driver, shown at their real cost.
//!
//! See `ssd1680_gdey0266z90_epd` for the full narrative on refresh-mode behavior, ink polarity,
//! ink physics, duty cycle, and glass provenance — none of that changed, so it isn't repeated
//! here. What differs is only the drawing code:
//!
//! - One `page: &mut PageBufferPair` replaces the two `bw`/`red` `PageBuffer` parameters
//!   `draw_static_content`/`draw_band`/`draw_band_bar` used to take.
//! - `TriColor::{Black, Accent}` replaces every panel-polarity-aware `BinaryColor::On`/`Off`
//!   choice — no more picking `Off` "because the Red plane is inverted."
//! - `PageBufferPair::clear()` replaces the pair of `clear_byte(0xFF)` / `clear_byte(0x00)` calls
//!   before each windowed redraw — one call, no raw byte to keep in sync with polarity.
//! - The `NO_RED_BAND` static the original example wrote for the BaseMap phase (a workaround for
//!   needing to hand-compute an all-`0x00` array of the right size) is gone: `page.accent()`'s
//!   own background bytes, derived from `PlanePolarity`, already are that array.
//!
//! Everything else — including the `static mut` frame buffers, needed because two 5,624-byte
//! planes is more than this board's default stack wants to carry — is unchanged in shape; only
//! the initial fill bytes now come from `PlanePolarity::SSD168X` instead of being hardcoded.
//!
//! ## This panel is not on Seeed's supported list
//!
//! Seeed's catalogue for this driver board covers 1.54", 2.13", 2.9", 4.2", 4.26", 5.65", 5.83"
//! and 7.5". **2.66" is not among them**, which is the same undocumented-combination status as
//! the 3.7" `GDEY037T03` that does not work on this board at all.
//!
//! The odds here are much better than that comparison suggests, though. The 3.7" failure is a
//! UC8253 fault that also takes down the 3.52" on this board, and belongs to the controller
//! rather than the panel size. The SSD1680 has a clean record on this driver board — the 2.13"
//! mono runs on it — and the SSD1681 1.54" tri-colour shows the DC-DC booster handles a colour
//! panel of comparable area (200 x 200 = 40,000 px against this panel's 45,000).
//!
//! If it does fail, the diagnostic order is in `BRINGUP.md`: check the refresh durations first.
//! A refresh returning in ~0 ms means BUSY was never observed asserted, which is a board or
//! timing fault, not a register one — and on this board that symptom has been a driver bug twice
//! and a genuine incompatibility once.
//!
//! ## Expected timings
//!
//! Measured on RP2350 with the same panel; this port logs its own so the two can be compared.
//! A difference here would be an ESP32-C3 finding, not a panel one.
//!
//! | Mode | RP2350 |
//! |---|---:|
//! | `Full` | 20.0 s |
//! | `Full`, windowed | 20.0 s |
//! | `FastFull` | **16.2 s** |
//! | `BaseMap` | 19.9 s |
//! | `Partial` | 19.9 s |
//!
//! ## Note on duty cycle
//!
//! Waveshare recommend at least 180 s between refreshes on this panel, and one update every 24 h
//! to avoid burn-in. This example runs seven refreshes seconds apart, which is fine as a one-off
//! bring-up run. **Do not loop it** — see the "Do not loop a panel rated for daily updates"
//! section of `BRINGUP.md`.
//!
//! ## Hardware
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** [ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html) (24-pin FPC)
//! - **Display:** Good Display GDEY0266Z90 / Waveshare 2.66inch e-Paper Module (B), 152x296 BWR.
//!   The unit this was written for is DKE glass, stamped `DEPG0266RWS800F34HP`, ribbon
//!   `FPC-7510 Rev. C`. `S800` is the SSD1680; the same glass also ships with a JD79651B (`F51B`)
//!   or UC8251d (`U25D`), which this driver does not support.
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
//! cargo run --release --example ssd1680_gdey0266z90_tri_epd
//! ```
//!
//! No startup hold, matching the other SSD168x examples here. The replug-and-attach ritual in
//! `BRINGUP.md` exists for the UC8253 panels; SSD168x has never needed it on this board, and the
//! reference timings for SSD1677/1680/1681 were all taken from a plain `cargo run`.

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
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::{Instant, Rate};
use tinybmp::Bmp;

esp_bootloader_esp_idf::esp_app_desc!();

/// Shortest elapsed time that can represent a real refresh on this panel, which takes ~20 s.
const MIN_REAL_REFRESH_MS: u64 = 5_000;

/// Row stride in bytes. 152 px is byte-aligned, so this is exactly 19 with no padding.
const STRIDE: usize = GDEY0266Z90::WIDTH.div_ceil(8) as usize;

/// Full frame buffer size per plane: 19 x 296 = 5,624 bytes.
const FRAME_BYTES: usize = STRIDE * GDEY0266Z90::HEIGHT as usize;

/// Top Y coordinate of the status band repainted in Phases 2 and 4. Everything above it is
/// painted in Phase 1 and never touched again, so the Accent logo stays put.
const BAND_Y: u32 = 220;

/// Height of the status band in pixels (y = 220..295).
const BAND_H: u32 = 76;

/// Status band buffer size: 19 x 76 = 1,444 bytes.
const BAND_BYTES: usize = STRIDE * BAND_H as usize;

/// The polarity this panel needs — Black/White plane normal, accent plane inverted. Passed to
/// every `PageBufferPair::new` call rather than assumed once, since a different panel could need
/// `PlanePolarity::UC8253` instead.
const POLARITY: PlanePolarity = PlanePolarity::SSD168X;

/// Black/White plane. `static` rather than a stack local: 5,624 bytes per plane, twice over, is
/// more than the default stack on this board wants to carry. Seeded from `POLARITY` rather than
/// a hardcoded `0xFF`, so a panel needing a different convention only has to change `POLARITY`.
static mut BW_BUF: [u8; FRAME_BYTES] = [POLARITY.bw_background_byte(); FRAME_BYTES];

/// Accent (Red/Yellow) plane. Seeded from `POLARITY` rather than a hardcoded `0x00` — see the
/// `BW_BUF` note above.
static mut RED_BUF: [u8; FRAME_BYTES] = [POLARITY.accent_background_byte(); FRAME_BYTES];

/// Refreshes the panel, reporting how long it took and returning the elapsed milliseconds.
///
/// A full refresh on this panel is ~20 s; anything close to zero means BUSY was never observed
/// asserted, which is the symptom `BRINGUP.md` documents at length — a failure that otherwise
/// looks like success in the log.
fn timed_refresh<BUS, C, P>(epd: &mut EpdDriver<BUS, C, P>, delay: &mut Delay, label: &str) -> u64
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    let start = Instant::now();
    epd.refresh(delay).unwrap();
    let elapsed_ms = start.elapsed().as_micros() / 1000;
    esp_println::println!("{}: refresh took {} ms", label, elapsed_ms);
    if elapsed_ms < MIN_REAL_REFRESH_MS {
        esp_println::println!(
            "  FAILED: a tri-colour panel cannot refresh in {} ms. BUSY was never observed \
             asserted, so the controller is not completing an update. See BRINGUP.md.",
            elapsed_ms
        );
    }
    elapsed_ms
}

/// Draws the Phase 1 / Phase 3 static content: everything above the status band.
///
/// One `page` addresses both RAM planes — no separate `bw`/`red` buffers, and no polarity-aware
/// `BinaryColor` choice: `TriColor::Black`/`TriColor::Accent` say what they mean.
fn draw_static_content(
    page: &mut PageBufferPair,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
    mode_label: &str,
) {
    let stroke = PrimitiveStyle::with_stroke(TriColor::Black, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, TriColor::Black);
    let small_text_style = MonoTextStyle::new(&FONT_6X10, TriColor::Black);
    let accent_small_text_style = MonoTextStyle::new(&FONT_6X10, TriColor::Accent);

    // Outer border (Black), so a shifted or wrapped raster is obvious.
    Rectangle::new(
        Point::new(0, 0),
        Size::new(GDEY0266Z90::WIDTH, GDEY0266Z90::HEIGHT),
    )
    .into_styled(stroke)
    .draw(page)
    .unwrap();

    // Header (Black). 11 chars at 10 px each fits the 152 px width.
    Text::new("GDEY0266Z90", Point::new(8, 22), text_style)
        .draw(page)
        .unwrap();

    // Subtitle: "Tri-Color " in Black, "BWR" in Accent.
    Text::new("Tri-Color ", Point::new(8, 40), small_text_style)
        .draw(page)
        .unwrap();
    Text::new("BWR", Point::new(68, 40), accent_small_text_style)
        .draw(page)
        .unwrap();

    Line::new(Point::new(8, 48), Point::new(143, 48))
        .into_styled(stroke)
        .draw(page)
        .unwrap();

    // Colour swatches: Black left, Accent right, inside a shared outline.
    Rectangle::new(Point::new(8, 56), Size::new(136, 18))
        .into_styled(stroke)
        .draw(page)
        .unwrap();
    Rectangle::new(Point::new(10, 58), Size::new(64, 14))
        .into_styled(PrimitiveStyle::with_fill(TriColor::Black))
        .draw(page)
        .unwrap();
    Rectangle::new(Point::new(78, 58), Size::new(64, 14))
        .into_styled(PrimitiveStyle::with_fill(TriColor::Accent))
        .draw(page)
        .unwrap();

    // Ferris (64x42) in Accent and Rust (64x64) in Black, side by side — 128 px of artwork fits
    // the 152 px width, unlike the 122 px monochrome panel where they have to be stacked. Both
    // drawn straight onto the same `page`, unlike the original example's separate `red`/`bw`
    // targets.
    let ferris_pos = Point::new(10, 92);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_pos, TriColor::Accent)
                .draw(page)
                .unwrap();
        }
    }

    let rust_pos = Point::new(78, 82);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_pos, TriColor::Black)
                .draw(page)
                .unwrap();
        }
    }

    // Labels (Black). Board name differs from the RP2350 original; everything else matches.
    Text::new("XIAO ESP32-C3", Point::new(8, 170), small_text_style)
        .draw(page)
        .unwrap();
    Text::new("epdsi SSD1680", Point::new(8, 184), small_text_style)
        .draw(page)
        .unwrap();
    Text::new(mode_label, Point::new(8, 198), small_text_style)
        .draw(page)
        .unwrap();

    // Separator above the status band that Phases 2 and 4 repaint.
    Line::new(Point::new(8, 210), Point::new(143, 210))
        .into_styled(stroke)
        .draw(page)
        .unwrap();
}

/// Writes both colour planes for the full frame, resetting the RAM window and cursor first.
///
/// Each RAM write restarts from the window origin, so the window and cursor have to be re-armed
/// before every plane rather than once per frame.
fn write_full_frame<BUS, C, P>(epd: &mut EpdDriver<BUS, C, P>, page: &PageBufferPair)
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    epd.set_window(0, 0, GDEY0266Z90::WIDTH - 1, GDEY0266Z90::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::BlackWhite, page.bw().as_slice())
        .unwrap();

    epd.set_window(0, 0, GDEY0266Z90::WIDTH - 1, GDEY0266Z90::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::RedYellow, page.accent().as_slice())
        .unwrap();
}

/// Writes both colour planes for the status band window.
fn write_band<BUS, C, P>(epd: &mut EpdDriver<BUS, C, P>, page: &PageBufferPair)
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
        .unwrap();
    epd.set_cursor(0, BAND_Y).unwrap();
    epd.write_frame(ColorChannel::BlackWhite, page.bw().as_slice())
        .unwrap();

    epd.set_window(0, BAND_Y, GDEY0266Z90::WIDTH - 1, BAND_Y + BAND_H - 1)
        .unwrap();
    epd.set_cursor(0, BAND_Y).unwrap();
    epd.write_frame(ColorChannel::RedYellow, page.accent().as_slice())
        .unwrap();
}

/// Draws the status band's Black content: label, counter and progress bar outline.
///
/// The bar *fill* is left to the caller as a separate call, always Accent.
fn draw_band(page: &mut PageBufferPair, count: u32, label: &str) {
    let stroke = PrimitiveStyle::with_stroke(TriColor::Black, 1);
    let small_text_style = MonoTextStyle::new(&FONT_6X10, TriColor::Black);

    Text::new(label, Point::new(8, BAND_Y as i32 + 14), small_text_style)
        .draw(page)
        .unwrap();

    let mut count_buf = [0u8; 32];
    let count_str = format_no_std::show(&mut count_buf, format_args!("Update #{}", count)).unwrap();
    Text::new(
        count_str,
        Point::new(8, BAND_Y as i32 + 28),
        small_text_style,
    )
    .draw(page)
    .unwrap();

    // Progress bar outline always lands on the Black/White plane.
    Rectangle::new(Point::new(8, BAND_Y as i32 + 38), Size::new(136, 16))
        .into_styled(stroke)
        .draw(page)
        .unwrap();
}

/// Draws the progress bar fill for `count`, always in Accent — the original example's
/// `draw_band_bar` took a `BinaryColor` parameter, but every call site passed the same value
/// (`BinaryColor::Off`, targeting the Red plane), so there was nothing for that parameter to
/// vary; `TriColor::Accent` is simply hardcoded here instead.
fn draw_band_bar(page: &mut PageBufferPair, count: u32) {
    Rectangle::new(
        Point::new(10, BAND_Y as i32 + 40),
        Size::new(count * 33, 12),
    )
    .into_styled(PrimitiveStyle::with_fill(TriColor::Accent))
    .draw(page)
    .unwrap();
}

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(100);

    esp_println::println!(
        "Starting GDEY0266Z90 2.66\" Tri-Color EPD example (epdsi SSD1680, PageBufferPair)"
    );
    esp_println::println!("Seven refreshes, ~2 min total. Do not interrupt.");

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
    // No variant selection needed: this panel shares the default SSD1680 register profile with
    // the GDEM0213B74 that already runs on this board.
    let controller = Ssd1680Controller::new(GDEY0266Z90::WIDTH, GDEY0266Z90::HEIGHT)
        .with_refresh_mode(Ssd168xRefreshMode::Full);
    let mut epd = EpdBuilder::<_, GDEY0266Z90>::new(controller).build(epd_bus);

    esp_println::println!("Initializing SSD1680 epdsi EPD driver...");
    epd.init(&mut delay).unwrap();

    // SAFETY: single-threaded example, and these are the only references taken to the buffers.
    let bw_buf: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(BW_BUF) };
    let red_buf: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(RED_BUF) };

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    esp_println::println!("--- Phase 1: Full Tri-Color Refresh ---");

    let full_ms = {
        let mut page = PageBufferPair::new(
            &mut bw_buf[..],
            &mut red_buf[..],
            GDEY0266Z90::WIDTH,
            GDEY0266Z90::HEIGHT,
            0,
            POLARITY,
        );

        draw_static_content(&mut page, &ferris_bmp, &rust_bmp, "mode: Full");

        esp_println::println!("Sending both planes ({} bytes each)...", FRAME_BYTES);
        write_full_frame(&mut epd, &page);

        timed_refresh(&mut epd, &mut delay, "Phase 1 (Full)")
    };

    delay.delay_ms(2000);

    esp_println::println!("--- Phase 2: Windowed Refresh on the Full Waveform ---");

    // Colour panels have no differential waveform, so a region update is not a speed-up — it is a
    // full refresh over a smaller area. Both planes must be written for the window.
    for count in 1..=2u32 {
        {
            let mut band = PageBufferPair::new(
                &mut bw_buf[..BAND_BYTES],
                &mut red_buf[..BAND_BYTES],
                GDEY0266Z90::WIDTH,
                BAND_H,
                BAND_Y,
                POLARITY,
            );
            band.clear();

            draw_band(&mut band, count, "Full window");
            draw_band_bar(&mut band, count);

            write_band(&mut epd, &band);
        }

        esp_println::println!("Refreshing band y={}..{}...", BAND_Y, BAND_Y + BAND_H - 1);
        timed_refresh(&mut epd, &mut delay, "Phase 2 (windowed Full)");
        delay.delay_ms(1000);
    }

    esp_println::println!("--- Phase 3: FastFull Full-Screen Refresh ---");

    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::FastFull);

    let fast_ms = {
        let mut page = PageBufferPair::new(
            &mut bw_buf[..],
            &mut red_buf[..],
            GDEY0266Z90::WIDTH,
            GDEY0266Z90::HEIGHT,
            0,
            POLARITY,
        );
        page.clear();

        draw_static_content(&mut page, &ferris_bmp, &rust_bmp, "mode: FastFull");

        write_full_frame(&mut epd, &page);

        timed_refresh(&mut epd, &mut delay, "Phase 3 (FastFull)")
    };

    esp_println::println!(
        "Full {} ms vs FastFull {} ms. RP2350 reference for this glass is 20045 vs 16181 \
         (~19% faster). A large deviation here is an ESP32-C3 finding, not a panel one.",
        full_ms,
        fast_ms
    );

    delay.delay_ms(2000);

    esp_println::println!(
        "--- Phase 4: BaseMap and Partial (both full-waveform on this panel) ---"
    );

    // Both planes are written for every pass here, exactly as in Phases 1-3. There is no
    // previous-frame seeding: on a Tri-Color panel 0x26 is *always* the colour plane, so writing a
    // Black/White image into it — the idiom the monochrome ssd1680_gdem0213b74_epd example uses —
    // paints the band solid red instead of priming a differential buffer.
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::BaseMap);

    {
        let mut band = PageBufferPair::new(
            &mut bw_buf[..BAND_BYTES],
            &mut red_buf[..BAND_BYTES],
            GDEY0266Z90::WIDTH,
            BAND_H,
            BAND_Y,
            POLARITY,
        );
        band.clear();

        draw_band(&mut band, 0, "BaseMap");
        // count == 0 draws a zero-width rectangle — a no-op, kept only for structural parity
        // with Phase 2/the Partial loop below.
        draw_band_bar(&mut band, 0);

        // No `NO_RED_BAND` constant needed: `band.accent()` is already all background bytes,
        // since nothing drew Accent content into it above.
        write_band(&mut epd, &band);
    }

    timed_refresh(&mut epd, &mut delay, "Phase 4 (BaseMap)");

    delay.delay_ms(1000);

    // Partial selects the controller's built-in fast LUT (0x22 = 0xFC). That LUT exists only for
    // monochrome panels, so here it is neither fast nor differential — measured at 19.9 s on
    // RP2350, the same as Full. It still needs both planes written or red in the window is lost.
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Partial);

    for count in 1..=2u32 {
        {
            let mut band = PageBufferPair::new(
                &mut bw_buf[..BAND_BYTES],
                &mut red_buf[..BAND_BYTES],
                GDEY0266Z90::WIDTH,
                BAND_H,
                BAND_Y,
                POLARITY,
            );
            band.clear();

            draw_band(&mut band, count, "Partial mode");
            draw_band_bar(&mut band, count);

            write_band(&mut epd, &band);
        }

        esp_println::println!("Partial-mode update #{}...", count);
        timed_refresh(&mut epd, &mut delay, "Phase 4 (Partial)");

        delay.delay_ms(1000);
    }

    // Restore the full-frame RAM window and the default waveform for any subsequent updates.
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Full);
    epd.set_window(0, 0, GDEY0266Z90::WIDTH - 1, GDEY0266Z90::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();

    // Deep sleep. init() must be called again before any further frame.
    epd.sleep(&mut delay).unwrap();
    esp_println::println!("Display complete, controller asleep.");

    loop {
        delay.delay_ms(1000);
    }
}
