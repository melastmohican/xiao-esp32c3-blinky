//! # GDEM0213B74 4-Level Grayscale (Gray4) E-Paper Example (`epdsi`)
//!
//! Port of the Raspberry Pi Pico 2 example from
//! [`rust-rpico2-discovery`](https://github.com/melastmohican/rust-rpico2-discovery)
//! to the Seeed Studio XIAO ESP32-C3 on the ePaper Driver Board for XIAO.
//!
//! Companion to `ssd1680_gdem0213b74_epd` (plain 1-bit monochrome). Same board, panel and
//! wiring, but drives the panel's **4-level grayscale** mode instead: White/Light/Dark/Black
//! instead of just White/Black.
//!
//! Two static screens:
//!
//! 1. **Screen 1**: title/subtitle banner over a 4-band swatch (Black/Dark/Light/White), each
//!    band carrying a single-letter label (`B`/`D`/`L`/`W`) sized to fit this panel's narrower
//!    122px width without overflowing into its neighbor.
//! 2. **Screen 2**: three concentric rounded rectangles (Light/Dark/White) with a centered
//!    "4-Lvl Gray" label in the innermost White ring.
//!
//! Like `ssd1680_gdem0213b74_epd`, there is no partial/differential refresh phase here: Gray4
//! mode has exactly one trigger ([`Ssd168xRefreshMode::Gray4`]).
//!
//! ## Provenance: read before trusting this on hardware
//!
//! `GDEM0213B74::GRAY4` is **not** Adafruit's own product-page default mode for this breakout.
//! It is transcribed verbatim from Adafruit_EPD's `ti_213mfgn_gray4_init_code`/
//! `ti_213mfgn_gray4_lut_code`, and is byte-identical to the already-shipped `GDEY0266T90::GRAY4`
//! bundle (confirmed by direct byte comparison, not assumed). See `epdsi`'s `GDEM0213B74` panel
//! doc for the full caveat, including a per-breakout-revision `_colstart` offset Adafruit's own
//! driver carries that `epdsi` has no equivalent of.
//!
//! ## Note on the 122 pixel panel width
//!
//! See `ssd1680_gdem0213b74_gray4_epd`'s own module doc in `rust-rpico2-discovery` for the full
//! layout-math rationale this port carries over unchanged: `FONT_6X10` throughout, single-letter
//! swatch labels, and three rings instead of the four `ssd1680_gdey0266t90_gray4_epd`'s wider
//! panel uses.
//!
//! ## Hardware
//!
//! Same board, panel class, adapter and wiring as `ssd1680_gdem0213b74_epd`. See that example
//! for the full pin table. Repeated here for convenience:
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** [ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html) (24-pin FPC)
//! - **Display:** Good Display GDEM0213B74 2.13" Monochrome, 122x250 (Adafruit 6383)
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
//! cargo run --release --example ssd1680_gdem0213b74_gray4_epd
//! ```

#![no_std]
#![no_main]

use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::mono_font::ascii::FONT_6X10;
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
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::Rate;

esp_bootloader_esp_idf::esp_app_desc!();

/// Row stride in bytes. 122 px rounds up to 16, same alignment the plain-mono example uses.
const STRIDE: usize = GDEM0213B74::WIDTH.div_ceil(8) as usize;

/// Full frame buffer size per plane: 16 x 250 = 4,000 bytes.
const FRAME_BYTES: usize = STRIDE * GDEM0213B74::HEIGHT as usize;

/// Adafruit's SSD1680 Gray4 convention: neither RAM plane inverted, a set bit is the code bit
/// directly. The only convention `epdsi` has evidence for so far.
const POLARITY: Gray4Polarity = Gray4Polarity::ADAFRUIT_SSD1680;

/// Bit-plane A. `static` rather than a stack local: carrying two of these on the stack is more
/// than the default stack on this board wants to hold.
static mut PLANE_A: [u8; FRAME_BYTES] = [0u8; FRAME_BYTES];

/// Bit-plane B.
static mut PLANE_B: [u8; FRAME_BYTES] = [0u8; FRAME_BYTES];

/// Writes both bit-planes for the full frame, resetting the RAM window and cursor first.
fn write_full_frame<BUS, C>(epd: &mut EpdDriver<BUS, C, GDEM0213B74>, page: &GrayBufferPair)
where
    C: EpdController<BUS>,
    C::Error: core::fmt::Debug,
{
    epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::BlackWhite, page.plane_a().as_slice())
        .unwrap();

    epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
        .unwrap();
    epd.set_cursor(0, 0).unwrap();
    epd.write_frame(ColorChannel::RedYellow, page.plane_b().as_slice())
        .unwrap();
}

/// Screen 1: title/subtitle banner over a 4-band Black/Dark/Light/White swatch, each band
/// labeled with a single letter. All three text lines and all four band widths are sized to
/// this panel's 122px width explicitly (see the widths/positions computed below), not copied
/// from the wider `GDEY0266T90` layout.
fn draw_banner(page: &mut GrayBufferPair) {
    let dark_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Dark);
    let light_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Light);
    let black_small_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Black);
    let white_small_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::White);

    // "SSD1680 Gray4" is 13 chars at 6px = 78px, centered in the 122px width: x = (122-78)/2 = 22.
    Text::new("SSD1680 Gray4", Point::new(22, 10), black_small_style)
        .draw(page)
        .unwrap();

    // "GDEM0213B74 122x250" is 19 chars at 6px = 114px: x = (122-114)/2 = 4.
    Text::new("GDEM0213B74 122x250", Point::new(4, 24), dark_style)
        .draw(page)
        .unwrap();

    // "epdsi Gray4 demo" is 16 chars at 6px = 96px: x = (122-96)/2 = 13.
    Text::new("epdsi Gray4 demo", Point::new(13, 38), light_style)
        .draw(page)
        .unwrap();

    // Four bands spanning the full 122px width. 122/4 = 30 remainder 2, so the last band is
    // widened to 32px rather than leaving a 2px gap: 30+30+30+32 = 122.
    const BAR_Y: i32 = 55;
    const BAR_H: u32 = 40;
    const BAR_W: u32 = GDEM0213B74::WIDTH / 4;
    const LAST_BAR_W: u32 = GDEM0213B74::WIDTH - 3 * BAR_W;

    let bands = [
        (0u32, BAR_W, Gray4Color::Black, "B", white_small_style),
        (1u32, BAR_W, Gray4Color::Dark, "D", white_small_style),
        (2u32, BAR_W, Gray4Color::Light, "L", black_small_style),
        (3u32, LAST_BAR_W, Gray4Color::White, "W", black_small_style),
    ];
    let mut x = 0i32;
    for (_index, width, fill, label, label_style) in bands {
        Rectangle::new(Point::new(x, BAR_Y), Size::new(width, BAR_H))
            .into_styled(PrimitiveStyle::with_fill(fill))
            .draw(page)
            .unwrap();
        // Single 6px-wide character, centered in the band: x + (width-6)/2.
        Text::new(
            label,
            Point::new(x + (width as i32 - 6) / 2, BAR_Y + BAR_H as i32 / 2 + 3),
            label_style,
        )
        .draw(page)
        .unwrap();
        x += width as i32;
    }
    // Outline around the White band so its edge is visible against the page background.
    Rectangle::new(
        Point::new(3 * BAR_W as i32, BAR_Y),
        Size::new(LAST_BAR_W, BAR_H),
    )
    .into_styled(PrimitiveStyle::with_stroke(Gray4Color::Black, 1))
    .draw(page)
    .unwrap();
}

/// Screen 2: three concentric rounded rectangles (Light/Dark/White, outer to inner) with a
/// centered "4-Lvl Gray" label in the innermost White ring. One ring fewer than
/// `ssd1680_gdey0266t90_gray4_epd`'s four: that version's 76px-wide innermost ring has room for
/// a 12-character label; this panel's narrower width does not reach that at the same padding
/// (see the inner-ring math below), so the ring count and label text are sized down instead of
/// risking the overflow that example's own module doc warns about.
fn draw_geometric(page: &mut GrayBufferPair) {
    let stroke = PrimitiveStyle::with_stroke(Gray4Color::Black, 1);
    Rectangle::new(
        Point::new(0, 0),
        Size::new(GDEM0213B74::WIDTH, GDEM0213B74::HEIGHT),
    )
    .into_styled(stroke)
    .draw(page)
    .unwrap();

    // (padding, corner radius, fill) for each nested ring, outer to inner. Innermost pad=22
    // leaves an inner rectangle 122 - 2*22 = 78px wide: enough for the 60px label below plus a
    // margin, which pad=30 or higher (as used on the wider panel) would not be.
    let rings: [(u32, u32, Gray4Color); 3] = [
        (6, 6, Gray4Color::Light),
        (14, 5, Gray4Color::Dark),
        (22, 4, Gray4Color::White),
    ];
    for (pad, radius, fill) in rings {
        let rect = Rectangle::new(
            Point::new(pad as i32, pad as i32),
            Size::new(GDEM0213B74::WIDTH - 2 * pad, GDEM0213B74::HEIGHT - 2 * pad),
        );
        RoundedRectangle::with_equal_corners(rect, Size::new(radius, radius))
            .into_styled(PrimitiveStyle::with_fill(fill))
            .draw(page)
            .unwrap();
    }

    // Innermost ring (pad=22) is 78px wide. "4-Lvl Gray" at FONT_6X10 is 10 chars * 6px = 60px,
    // centered: x = 22 + (78-60)/2 = 31.
    let inner_pad = 22i32;
    let inner_width = GDEM0213B74::WIDTH as i32 - 2 * inner_pad;
    let label_style = MonoTextStyle::new(&FONT_6X10, Gray4Color::Black);
    let label = "4-Lvl Gray";
    let label_width = label.len() as i32 * 6;
    Text::new(
        label,
        Point::new(
            inner_pad + (inner_width - label_width) / 2,
            GDEM0213B74::HEIGHT as i32 / 2,
        ),
        label_style,
    )
    .draw(page)
    .unwrap();
}

#[esp_hal::main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(100);

    esp_println::println!(
        "Starting GDEM0213B74 4-Level Grayscale (Gray4) EPD example (epdsi SSD1680)"
    );

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

    // `for_panel` picks up `GDEM0213B74`'s dimensions; `.with_gray4` layers on the Adafruit_EPD-
    // sourced register bundle, and `Ssd168xRefreshMode::Gray4` selects its one refresh trigger.
    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Ssd1680Controller::for_panel::<GDEM0213B74>()
        .with_gray4(GDEM0213B74::GRAY4)
        .with_refresh_mode(Ssd168xRefreshMode::Gray4);
    let mut epd = EpdBuilder::<_, GDEM0213B74>::new(controller).build(epd_bus);

    esp_println::println!("Initializing SSD1680 epdsi EPD driver (Gray4)...");
    epd.init(&mut delay).unwrap();

    // SAFETY: single-threaded example, and these are the only references taken to the buffers.
    let plane_a: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(PLANE_A) };
    let plane_b: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(PLANE_B) };

    esp_println::println!("--- Screen 1: Banner & 4-Level Swatch ---");
    {
        let mut page = GrayBufferPair::new(
            &mut plane_a[..],
            &mut plane_b[..],
            GDEM0213B74::WIDTH,
            GDEM0213B74::HEIGHT,
            0,
            POLARITY,
        );
        page.clear();
        draw_banner(&mut page);
        write_full_frame(&mut epd, &page);
    }
    epd.refresh(&mut delay).unwrap();

    delay.delay_ms(8000);

    esp_println::println!("--- Screen 2: Concentric Geometric Grayscale Test Pattern ---");
    {
        let mut page = GrayBufferPair::new(
            &mut plane_a[..],
            &mut plane_b[..],
            GDEM0213B74::WIDTH,
            GDEM0213B74::HEIGHT,
            0,
            POLARITY,
        );
        page.clear();
        draw_geometric(&mut page);
        write_full_frame(&mut epd, &page);
    }
    epd.refresh(&mut delay).unwrap();

    // Deep sleep. init() must be called again before any further frame.
    epd.sleep(&mut delay).unwrap();
    esp_println::println!("Display complete, controller asleep.");

    loop {
        delay.delay_ms(1000);
    }
}
