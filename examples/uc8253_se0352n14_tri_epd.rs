//! # Waveshare 3.52" (B) SE0352N14TNGA0 Tri-Color `PageBufferPair` Draw Target Example (`epdsi`)
//!
//! > **Expect this to fail on the XIAO ESP32-C3, and treat that as the experiment.** The only
//! > other UC8253 panel here, the `GDEY037T03`, does not work on this board: it is alive (BUSY is
//! > driven and asserts on reset) but never acts on SPI commands, at any clock from 4 MHz down to
//! > 100 kHz, and stock Arduino GxEPD2 fails identically. See `BRINGUP.md`.
//! >
//! > `BRINGUP.md` leaves two untested causes: the UC8253's DC-DC booster sagging the C3's rail, or
//! > something about the 3.7" panel's FPC that the C3's pin assignment does not tolerate. **This
//! > example separates them**, because it is a *different* UC8253 panel on the same board:
//! >
//! > - **It works** -> the fault is specific to the 3.7" panel, not UC8253-on-C3. This is the
//! >   strong result: a tri-color panel drives a longer, more demanding waveform than the 3.7"
//! >   mono, so a BWR pass argues hard against the power-sag explanation.
//! > - **It fails the same way** -> the fault is the controller against this board, most likely
//! >   power delivery. Same symptom, second panel, is close to conclusive.
//! >
//! > Either way, record the outcome in `BRINGUP.md` — this is the missing measurement, not a
//! > routine port. Note the 3.52" is **not** on Seeed's supported panel list for this driver
//! > board either, the same caveat the 3.7" carries.
//!
//! Full-parity companion to `uc8253_se0352n14_epd` — same two phases, same content, same board
//! bring-up and diagnostics (`report_busy`, `refresh_timed`) — but drawn entirely through
//! `PageBufferPair`/`TriColor` instead of two separate `PageBuffer`s and the panel-specific
//! `INK = BinaryColor::Off` alias.
//!
//! Demonstrates:
//! 1. **Phase 1**: A framed white | black | red panel test that makes miswiring self-evident.
//! 2. **Phase 2**: Full Tri-Color refresh showing a header, the Rust logo in black, the Ferris
//!    logo in red, text labels and a red accent bar.
//!
//! The test runs first so the panel is left showing the picture, matching the other EPD examples
//! in this repo.
//!
//! Both phases report the BUSY level and the elapsed refresh time, because on this board the
//! documented failure is silent: `refresh` returns in 0 ms and the panel simply never changes.
//!
//! See `uc8253_se0352n14_epd` for the full narrative on this panel's quirks (swapped RAM plane
//! routing vs. `GDEY037T03`, inverted ink polarity on *both* planes, no partial/fast waveform,
//! the charge-pump `POWER_ON` requirement, active-low BUSY, and native orientation). None of that
//! changed — this file only differs in the drawing code:
//!
//! - `PlanePolarity::UC8253` replaces the `INK`/`WHITE_BYTE` constants: this is the one panel
//!   `epdsi` ships where *both* planes are inverted (a set bit is ink), unlike SSD168x where
//!   only the accent plane is. Getting the wrong constant here is exactly the mistake
//!   `PlanePolarity` exists to catch — see its docs.
//! - One shared `PageBufferPair` (built once per phase via `frame()`) replaces the two
//!   independently-rotated `PageBuffer`s the original built per phase. The original's own doc
//!   comment warned that going through anything other than one shared constructor risks
//!   `DisplayRotation::Rotate180` landing on three planes but not a fourth; going from four
//!   `frame()` call sites (two phases x two planes) to two (one per phase, both planes together)
//!   removes that risk structurally rather than by discipline.
//! - `blit()` takes a `TriColor` destination color as well as the `BinaryColor` source-pixel
//!   test, since which RAM plane a logo lands on is now a draw-time choice rather than which
//!   `PageBuffer` it was handed.
//!
//! ## Note on this panel versus the other UC8253 panel
//!
//! The same driver IC drives the `GDEY037T03` here, but the two panels are **not** interchangeable
//! behind one register profile, hence `Uc8253Variant`:
//!
//! - The Black/White plane is [`ColorChannel::BlackWhite`] -> `WRITE_OLD_DATA` (`0x10`) and red is
//!   [`ColorChannel::RedYellow`] -> `WRITE_NEW_DATA` (`0x13`). That is **swapped** relative to the
//!   `GDEY037T03`, following from the `KW/R` bit in Panel Setting selecting KWR mode.
//! - **Ink is a set bit and `0x00` is white, in both planes** — the opposite of the monochrome
//!   panel's `0xFF`. `PageBuffer` natively treats a *cleared* bit as ink, so both buffers start at
//!   `0x00` and everything is drawn with [`BinaryColor::Off`], aliased to `INK` below.
//! - **Full refresh only**, roughly 16-20 s. There is no partial or fast waveform: the red pigment
//!   needs the full OTP waveform. `Uc8253RefreshMode` is ignored for this variant, and no
//!   `set_window` call is made — a full-frame write must not be wrapped in a partial-window
//!   session.
//! - The controller drops its charge pump after each update, so `epdsi` issues `POWER_ON` at the
//!   start of every refresh. Skipping that does not error — `DISPLAY_REFRESH` is silently ignored,
//!   BUSY never asserts, and the refresh appears to finish instantly having drawn nothing.
//! - **BUSY is active-LOW**, as on the `GDEY037T03`, so the GPIO takes a pull-**up**.
//! - Orientation: this panel's native `(0,0)` is top-left with the FPC ribbon at the **top**,
//!   the opposite of the other panels here. Every drawing surface is built through `frame()`,
//!   which applies [`DisplayRotation::Rotate180`], so this example renders **with the ribbon at
//!   the bottom** like the rest of the repo.
//!
//! ## Reading the Phase 1 panel test
//!
//! The bands are, left to right, **white | black | red**, inside a black frame:
//!
//! - Black and red swapped -> the RAM plane routing is crossed (`0x10`/`0x13`).
//! - White and black swapped -> `CDI`/DDX polarity is wrong, or the buffers were not `0x00`-based.
//! - A cleared panel reading grey rather than white is this panel's white point, **not** a fault.
//!
//! ## Hardware
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** [ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html) (24-pin FPC)
//! - **Display:** Waveshare 3.52inch e-Paper (B), panel `SE0352N14-TNG-A0`, 240x360 BWR, FPC
//!   stamped `SE0352N01FPC-A 2024.12.04 X` — the flex is the shared `N01` part, not `N14`
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
//! Power-cycle the board first. Per `BRINGUP.md`, a run cut off mid-update leaves the controller
//! latched busy, and `hard_reset` does not clear it — only removing power does.
//!
//! ```bash
//! cargo run --release --example uc8253_se0352n14_tri_epd
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
use esp_hal::time::{Instant, Rate};
use tinybmp::Bmp;

esp_bootloader_esp_idf::esp_app_desc!();

/// This panel's polarity: *both* planes are inverted (a set bit is ink), unlike SSD168x where
/// only the accent plane is. See the module doc.
const POLARITY: PlanePolarity = PlanePolarity::UC8253;

/// Row stride in bytes: 240 / 8 = 30. This panel is already byte-aligned.
const STRIDE: usize = SE0352N14TNGA0::WIDTH.div_ceil(8) as usize;

/// Frame buffer size for one plane: 30 x 360 = 10,800 bytes.
const FRAME_BYTES: usize = STRIDE * SE0352N14TNGA0::HEIGHT as usize;

/// X coordinate of the left logo slot.
const LOGO_LEFT_X: i32 = 30;

/// X coordinate of the right logo slot.
const LOGO_RIGHT_X: i32 = 146;

/// Black/White plane. Static rather than stack-allocated, matching the other examples here.
static mut BW_BUF: [u8; FRAME_BYTES] = [POLARITY.bw_background_byte(); FRAME_BYTES];

/// Red (Accent) plane. Same size again — this example holds 21,600 bytes of frame buffer.
static mut RED_BUF: [u8; FRAME_BYTES] = [POLARITY.accent_background_byte(); FRAME_BYTES];

/// Builds a full-frame drawing surface addressing both RAM planes, rotated to this repo's
/// convention.
///
/// This panel's native `(0,0)` is top-left with the FPC ribbon at the **top**, but every other EPD
/// example here renders with the ribbon at the **bottom**, so the surface is rotated 180°. Going
/// through one constructor for both planes together — rather than one per plane, as the manual
/// `PageBuffer` approach needed — makes "rotation applied to one plane but not the other"
/// structurally impossible instead of a discipline to maintain.
fn frame<'a>(bw_buf: &'a mut [u8], accent_buf: &'a mut [u8]) -> PageBufferPair<'a> {
    let mut page = PageBufferPair::new(
        bw_buf,
        accent_buf,
        SE0352N14TNGA0::WIDTH,
        SE0352N14TNGA0::HEIGHT,
        0,
        POLARITY,
    );
    page.set_rotation(DisplayRotation::Rotate180);
    page
}

/// Draws a BMP into `page` as `color`, treating source pixels equal to `source_ink` as ink.
///
/// The two bundled logos disagree on polarity: `ferrisbw.bmp` marks its subject with
/// `BinaryColor::Off` and `rustbw.bmp` with `BinaryColor::On` — that is a fact about the BMP
/// files, unrelated to which RAM plane `color` routes to.
fn blit(
    page: &mut PageBufferPair,
    bmp: &Bmp<BinaryColor>,
    origin: Point,
    source_ink: BinaryColor,
    color: TriColor,
) {
    for Pixel(point, pixel_color) in bmp.pixels() {
        if pixel_color == source_ink {
            Pixel(point + origin, color).draw(page).unwrap();
        }
    }
}

/// Draws the panel-test Black content: title, separator, the pattern frame, the black band and
/// the band labels.
fn draw_test_black_content(page: &mut PageBufferPair) {
    let text_style = MonoTextStyle::new(&FONT_10X20, TriColor::Black);
    let stroke = PrimitiveStyle::with_stroke(TriColor::Black, 1);

    Text::new("PLANE TEST", Point::new(10, 24), text_style)
        .draw(page)
        .unwrap();

    Line::new(Point::new(10, 34), Point::new(229, 34))
        .into_styled(stroke)
        .draw(page)
        .unwrap();

    // Black frame around the whole pattern. Without it the white band has no visible edge at all
    // and the pattern reads as two bands on a blank panel rather than three. Outer edges land on
    // x = 11/228 and y = 50/289, leaving a 216 x 238 interior.
    Rectangle::new(Point::new(11, 50), Size::new(218, 240))
        .into_styled(stroke)
        .draw(page)
        .unwrap();

    // Middle third solid black, inset by one pixel so it abuts the frame rather than overlapping
    // it. The left third is left bare, so it shows the panel's white.
    Rectangle::new(Point::new(84, 51), Size::new(72, 238))
        .into_styled(PrimitiveStyle::with_fill(TriColor::Black))
        .draw(page)
        .unwrap();

    // One label centred under each band (interior thirds are 72 px, so centres are x = 48, 120,
    // 192; FONT_10X20 glyphs are 10 px wide). The frame stops at y = 289, so these sit on white
    // and stay legible whichever plane misbehaves.
    for (label, centre_x) in [("W", 48), ("B", 120), ("R", 192)] {
        Text::new(label, Point::new(centre_x - 5, 320), text_style)
            .draw(page)
            .unwrap();
    }
}

/// Draws the panel-test Accent content: the right-hand red band.
///
/// Must not touch the frame drawn in [`draw_test_black_content`] — the two thirds are disjoint
/// (x = 84..156 Black, x = 156..228 Accent), so drawing order doesn't matter here, but drawing
/// Accent over a Black pixel would clear that pixel's Black-plane bit back to background (see
/// `PageBufferPair::set_pixel`'s docs on why redrawing a pixel clears the other plane).
fn draw_test_accent_content(page: &mut PageBufferPair) {
    Rectangle::new(Point::new(156, 51), Size::new(72, 238))
        .into_styled(PrimitiveStyle::with_fill(TriColor::Accent))
        .draw(page)
        .unwrap();
}

/// Draws the Phase 2 content's Black half: border, header, separators, the Rust logo and labels.
fn draw_black_content(page: &mut PageBufferPair, rust_bmp: &Bmp<BinaryColor>) {
    let stroke = PrimitiveStyle::with_stroke(TriColor::Black, 1);
    let text_style = MonoTextStyle::new(&FONT_10X20, TriColor::Black);

    Rectangle::new(
        Point::new(0, 0),
        Size::new(SE0352N14TNGA0::WIDTH, SE0352N14TNGA0::HEIGHT),
    )
    .into_styled(stroke)
    .draw(page)
    .unwrap();

    Text::new("SE0352N14", Point::new(10, 24), text_style)
        .draw(page)
        .unwrap();

    Text::new("3.52\" BWR", Point::new(10, 48), text_style)
        .draw(page)
        .unwrap();

    Line::new(Point::new(10, 58), Point::new(229, 58))
        .into_styled(stroke)
        .draw(page)
        .unwrap();

    blit(
        page,
        rust_bmp,
        Point::new(LOGO_RIGHT_X, 90),
        BinaryColor::On,
        TriColor::Black,
    );

    Text::new("XIAO ESP32-C3", Point::new(10, 200), text_style)
        .draw(page)
        .unwrap();

    Text::new("epdsi PageBufferPair", Point::new(10, 225), text_style)
        .draw(page)
        .unwrap();

    Line::new(Point::new(10, 240), Point::new(229, 240))
        .into_styled(stroke)
        .draw(page)
        .unwrap();

    Text::new("Waveshare (B)", Point::new(10, 285), text_style)
        .draw(page)
        .unwrap();
}

/// Draws the Phase 2 content's Accent half: the Ferris logo and an accent bar.
fn draw_accent_content(page: &mut PageBufferPair, ferris_bmp: &Bmp<BinaryColor>) {
    blit(
        page,
        ferris_bmp,
        Point::new(LOGO_LEFT_X, 112),
        BinaryColor::Off,
        TriColor::Accent,
    );

    Rectangle::new(Point::new(10, 300), Size::new(220, 30))
        .into_styled(PrimitiveStyle::with_fill(TriColor::Accent))
        .draw(page)
        .unwrap();
}

/// Reports the BUSY line, which is the first thing to check on this board.
///
/// BUSY is active-LOW here, so HIGH means idle. `BRINGUP.md` records that on the C3 the 3.7" panel
/// drives BUSY correctly while ignoring SPI entirely — so BUSY reading sanely proves the panel is
/// present and powered, and proves nothing at all about whether it is listening.
fn report_busy(label: &str, high: bool) {
    esp_println::println!(
        "BUSY {}: {} ({})",
        label,
        if high { "HIGH" } else { "LOW" },
        if high { "idle" } else { "busy" }
    );
}

/// Refreshes the panel and reports how long it took, flagging the known C3 failure.
///
/// A full refresh on this panel is 16-20 s. A near-instant return is the documented
/// UC8253-on-ESP32-C3 symptom: the panel never acts on the command and `refresh` finds BUSY
/// already idle.
fn refresh_timed<BUS, C, P>(epd: &mut EpdDriver<BUS, C, P>, delay: &mut Delay)
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
    P: EpdPanel,
{
    let start = Instant::now();
    epd.refresh(delay).unwrap();
    let elapsed_ms = (Instant::now() - start).as_millis();

    esp_println::println!("Refresh returned after {} ms", elapsed_ms);
    if elapsed_ms < 1_000 {
        esp_println::println!(
            "  !! Expected ~16-20 s. This is the documented UC8253-on-C3 symptom (see BRINGUP.md):"
        );
        esp_println::println!(
            "  !! the panel is alive but never acts on SPI. If the display is also unchanged,"
        );
        esp_println::println!(
            "  !! the fault follows the controller, not the 3.7\" panel — record that."
        );
    }
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(100);

    esp_println::println!(
        "Starting SE0352N14TNGA0 3.52\" Tri-Color EPD example (epdsi UC8253, PageBufferPair)"
    );

    // Pin assignments are fixed by the ePaper Driver Board for XIAO.
    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    // UC8253 BUSY is active-LOW, unlike the SSD16xx panels: pull up so a missing or unpowered
    // panel reads "idle" rather than "busy forever".
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

    // The variant is not optional. The default Gdey037t03 profile's init, plane order and CDI
    // value are all wrong for this panel, and it renders inverted or blank rather than erroring.
    let controller = Uc8253Controller::new(SE0352N14TNGA0::WIDTH, SE0352N14TNGA0::HEIGHT)
        .with_variant(Uc8253Variant::Se0352n14);
    let mut epd = EpdBuilder::<_, SE0352N14TNGA0>::new(controller).build(epd_bus);

    // A floating BUSY reads HIGH through the pull-up too, so this only distinguishes "panel
    // driving BUSY low" from everything else. It is still the cheapest first check.
    report_busy("before init", epd.bus_mut().busy_is_high().unwrap());

    esp_println::println!("Initializing UC8253 epdsi EPD driver (Se0352n14 profile)...");
    epd.init(&mut delay).unwrap();
    report_busy("after init", epd.bus_mut().busy_is_high().unwrap());

    // Both planes start white — PlanePolarity::UC8253's background byte for both, 0x00, not the
    // monochrome UC8253 panel's 0xFF.
    epd.clear_frame(ColorChannel::BlackWhite, POLARITY.bw_background_byte())
        .unwrap();
    epd.clear_frame(ColorChannel::RedYellow, POLARITY.accent_background_byte())
        .unwrap();

    // SAFETY: single-threaded example, and these are the only references taken to the buffers.
    let bw_buf: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(BW_BUF) };
    let red_buf: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(RED_BUF) };

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    esp_println::println!("--- Phase 1: Panel Test ---");

    // The buffers start white, so nothing to clear yet.
    {
        let mut page = frame(&mut bw_buf[..], &mut red_buf[..]);
        draw_test_black_content(&mut page);
        draw_test_accent_content(&mut page);
    }

    // No set_window: a full-frame write must not be wrapped in a partial-window session, and this
    // panel has no partial mode anyway. Leaving the window unset keeps the SPI stream identical to
    // Waveshare's reference driver.
    esp_println::println!("Sending diagnostic pattern (white | black | red)...");
    epd.write_frame(ColorChannel::BlackWhite, &bw_buf[..])
        .unwrap();
    epd.write_frame(ColorChannel::RedYellow, &red_buf[..])
        .unwrap();

    esp_println::println!("Refreshing display hardware (full waveform, expect ~16-20 s)...");
    refresh_timed(&mut epd, &mut delay);
    report_busy("after refresh", epd.bus_mut().busy_is_high().unwrap());

    esp_println::println!("Bands left to right should read white | black | red.");
    esp_println::println!("  black/red swapped -> plane routing crossed (0x10/0x13)");
    esp_println::println!("  white/black swapped -> CDI/DDX polarity or buffer base wrong");
    esp_println::println!("  cleared panel looking grey -> normal white point, not a fault");
    esp_println::println!("  nothing at all, refreshes instant -> the C3 issue, see BRINGUP.md");

    delay.delay_ms(3000);

    esp_println::println!("--- Phase 2: Full Tri-Color Refresh ---");

    // Drawn second so the panel is left showing the picture rather than the test pattern, matching
    // the other EPD examples in this repo.
    {
        let mut page = frame(&mut bw_buf[..], &mut red_buf[..]);
        page.clear();
        draw_black_content(&mut page, &rust_bmp);
        draw_accent_content(&mut page, &ferris_bmp);
    }

    esp_println::println!("Sending both planes (10,800 bytes each)...");
    epd.write_frame(ColorChannel::BlackWhite, &bw_buf[..])
        .unwrap();
    epd.write_frame(ColorChannel::RedYellow, &red_buf[..])
        .unwrap();

    esp_println::println!("Refreshing display hardware (full waveform, expect ~16-20 s)...");
    refresh_timed(&mut epd, &mut delay);
    report_busy("after refresh", epd.bus_mut().busy_is_high().unwrap());

    // After sleep the controller is in deep sleep: init() must be called again before drawing.
    epd.sleep(&mut delay).unwrap();
    esp_println::println!("Display complete, panel asleep!");

    loop {
        delay.delay_ms(1000);
    }
}
