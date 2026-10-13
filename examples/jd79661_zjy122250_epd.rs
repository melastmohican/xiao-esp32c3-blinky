//! # ZJY122250-0213AJH-E5 2.13" Quad-Color E-Paper Example (`epdsi`)
//!
//! Port of the Raspberry Pi Pico 2 example from
//! [`rust-rpico2-discovery`](https://github.com/melastmohican/rust-rpico2-discovery)
//! to the Seeed Studio XIAO ESP32-C3 on the ePaper Driver Board for XIAO.
//!
//! Everything from `QuadColor` down to the end of `QuadColorBuffer` is unchanged from
//! the RP2350 version — the `epdsi` panel and controller types, the 2bpp buffer and the
//! `embedded-graphics` drawing are all HAL-agnostic. Only `main` differs.
//!
//! Draws a header, a four-swatch colour bar exercising all four inks, the Ferris and
//! Rust logos, and footer labels, then does a single full refresh and powers the panel
//! down. Quad-colour panels have no fast waveform, so expect several seconds.
//!
//! ## Panel check
//!
//! Before the refresh the example resets the panel and reads its `R70H` revision register over
//! a bit-banged 3-wire bus, before SPI takes SCK and MOSI, then logs which unit it is. Seen so
//! far: `09 01 01` (Adafruit `ZJY122250`, this example), `09 02 01` and `89 02 01` (the
//! `0213SW-F52-B2` F52 units, which want `jd79676_gdey0213f52_epd`). It never powers on or
//! refreshes the panel, and the example carries on whatever it reads.
//!
//! **Hardware-verified on a XIAO ESP32-C3** (12 Oct 2026) with the Adafruit unit (sticker
//! `ZJY122250-0213EAH-E5`): `R70H` read `09 01 01` and the refresh completed. Needs `epdsi`
//! 0.8.1 or later for the revision read.
//!
//! ## Panel identification
//!
//! This panel ships as Good Display `GDEY0213F51` and Adafruit 6373 / 6366 (sticker
//! `ZJY122250-0213...-E5`). The Seeed Studio 5779 is not the same panel: its sticker is
//! `0213SW-F52-B2`, the Good Display `GDEY0213F52` with a JD79676AA. All of them carry
//! an `FPC-J002` flex ribbon, so the ribbon does not tell the families apart. Check the
//! rear sticker, or run `epd_diag_jd7966x_rev`.
//!
//! ## Hardware
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** [ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html) (24-pin FPC)
//! - **Display:** ZJY122250-0213AJH-E5, 2.13" Quad-Color, 122x250
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
//! cargo run --release --example jd79661_zjy122250_epd
//! ```

#![no_std]
#![no_main]

use embedded_graphics::geometry::{Dimensions, Point, Size};
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
use esp_hal::gpio::{Flex, Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::Rate;
use tinybmp::Bmp;

esp_bootloader_esp_idf::esp_app_desc!();

/// 4-color options for 2bpp e-Paper display (JD79661)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuadColor {
    Black = 0b00,
    White = 0b01,
    Yellow = 0b10,
    Red = 0b11,
}

/// 2-bit per pixel buffer for 4-color displays (1 byte = 4 pixels)
pub struct QuadColorBuffer<'a> {
    buffer: &'a mut [u8],
    width: u32,
    height: u32,
    ram_stride: u32,
    rotation: DisplayRotation,
}

impl<'a> QuadColorBuffer<'a> {
    pub fn new(buffer: &'a mut [u8], width: u32, height: u32) -> Self {
        // Fill with White (0b01010101 = 0x55)
        buffer.fill(0x55);
        // RAM row stride is aligned to 8-pixel byte boundary (128 pixels / 32 bytes for 122px width)
        let ram_stride = width.div_ceil(8) * 8;
        Self {
            buffer,
            width,
            height,
            ram_stride,
            rotation: DisplayRotation::Rotate0,
        }
    }

    pub fn set_rotation(&mut self, rotation: DisplayRotation) {
        self.rotation = rotation;
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, color: QuadColor) {
        let (mapped_x, mapped_y) = match self.rotation {
            DisplayRotation::Rotate0 => (x, y),
            DisplayRotation::Rotate90 => (self.width.saturating_sub(1).saturating_sub(y), x),
            DisplayRotation::Rotate180 => (
                self.width.saturating_sub(1).saturating_sub(x),
                self.height.saturating_sub(1).saturating_sub(y),
            ),
            DisplayRotation::Rotate270 => (y, self.height.saturating_sub(1).saturating_sub(x)),
        };

        if mapped_x >= self.width || mapped_y >= self.height {
            return;
        }

        let pixel_index = mapped_y * self.ram_stride + mapped_x;
        let byte_index = (pixel_index / 4) as usize;
        let pixel_offset = 3 - (pixel_index % 4);
        let bit_shift = pixel_offset * 2;

        if byte_index < self.buffer.len() {
            let mask = !(0b11 << bit_shift);
            let val = (color as u8) << bit_shift;
            self.buffer[byte_index] = (self.buffer[byte_index] & mask) | val;
        }
    }

    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: QuadColor) {
        for px in x..(x + w) {
            for py in y..(y + h) {
                self.set_pixel(px, py, color);
            }
        }
    }

    pub fn draw_rect_outline(&mut self, x: u32, y: u32, w: u32, h: u32, color: QuadColor) {
        if w == 0 || h == 0 {
            return;
        }
        for px in x..(x + w) {
            self.set_pixel(px, y, color);
            self.set_pixel(px, y + h - 1, color);
        }
        for py in y..(y + h) {
            self.set_pixel(x, py, color);
            self.set_pixel(x + w - 1, py, color);
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        self.buffer
    }
}

impl<'a> DrawTarget for QuadColorBuffer<'a> {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(coord, color) in pixels.into_iter() {
            if coord.x >= 0 && coord.y >= 0 {
                let c = match color {
                    BinaryColor::On => QuadColor::Black,
                    BinaryColor::Off => QuadColor::White,
                };
                self.set_pixel(coord.x as u32, coord.y as u32, c);
            }
        }
        Ok(())
    }
}

impl<'a> Dimensions for QuadColorBuffer<'a> {
    fn bounding_box(&self) -> Rectangle {
        let (w, h) = match self.rotation {
            DisplayRotation::Rotate0 | DisplayRotation::Rotate180 => (self.width, self.height),
            DisplayRotation::Rotate90 | DisplayRotation::Rotate270 => (self.height, self.width),
        };
        Rectangle::new(Point::zero(), Size::new(w, h))
    }
}

/// 2bpp frame buffer: 32 bytes per row x 250 rows = 8,000 bytes. Held as a static
/// rather than on the stack, matching the other example in this repository — the
/// ESP32-C3's default stack is modest and this keeps the two consistent.
///
/// `0x55` is white repeated across all four 2-bit pixels in the byte.
static mut FRAME_BUF: [u8; 8000] = [0x55u8; 8000];

/// The bidirectional data line for the pre-flight revision read, as a `Flex` pin that
/// `Spi3Bus` can turn around per byte. It borrows GPIO10 only until the read is done.
struct SdaPin<'d>(Flex<'d>);

impl DynamicPin for SdaPin<'_> {
    type Error = core::convert::Infallible;

    fn set_as_output(&mut self) -> Result<(), Self::Error> {
        self.0.apply_output_config(&OutputConfig::default());
        self.0.set_output_enable(true);
        self.0.set_input_enable(false);
        Ok(())
    }

    fn set_as_input(&mut self) -> Result<(), Self::Error> {
        self.0.set_output_enable(false);
        self.0
            .apply_input_config(&InputConfig::default().with_pull(Pull::None));
        self.0.set_input_enable(true);
        Ok(())
    }

    fn set_high(&mut self) -> Result<(), Self::Error> {
        self.0.set_high();
        Ok(())
    }

    fn set_low(&mut self) -> Result<(), Self::Error> {
        self.0.set_low();
        Ok(())
    }

    fn is_high(&mut self) -> Result<bool, Self::Error> {
        Ok(self.0.is_high())
    }
}

/// Says which rear-sticker family a `R70H` reading belongs to, from the units seen so far.
fn describe_revision(rev: [u8; 3]) -> &'static str {
    match rev {
        [0x09, 0x01, 0x01] => {
            "matches the Adafruit ZJY122250 unit seen (sticker ZJY122250-0213EAH-E5)"
        }
        [0x03, 0x02, 0x01] => "matches the JD79661AA datasheet (not seen on a unit before)",
        [0x09, 0x02, 0x01] | [0x89, 0x02, 0x01] => {
            "F52 unit (0213SW-F52-B2): this example is for the F51, use jd79676_gdey0213f52_epd"
        }
        [0xFF, 0xFF, 0xFF] | [0x00, 0x00, 0x00] => "read path not reaching the IC, no answer",
        _ => "new pattern, record it with the rear sticker text and lot",
    }
}

#[main]
fn main() -> ! {
    let mut peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(100);

    esp_println::println!("Starting ZJY122250 2.13\" Quad-Color EPD example (epdsi JD79661)");

    // --- Which panel is this? Read R70H before SPI2 takes SCK and MOSI. ---
    // Reset with the demo's timings, bit-bang the register read on reborrowed pins, then let
    // them go. Nothing is powered on or refreshed here.
    {
        let id_cs = Output::new(
            peripherals.GPIO3.reborrow(),
            Level::High,
            OutputConfig::default(),
        );
        let id_dc = Output::new(
            peripherals.GPIO5.reborrow(),
            Level::Low,
            OutputConfig::default(),
        );
        let mut id_rst = Output::new(
            peripherals.GPIO2.reborrow(),
            Level::High,
            OutputConfig::default(),
        );
        let id_sck = Output::new(
            peripherals.GPIO8.reborrow(),
            Level::Low,
            OutputConfig::default(),
        );
        let id_busy = Input::new(
            peripherals.GPIO4.reborrow(),
            InputConfig::default().with_pull(Pull::Up),
        );
        let id_sda = SdaPin(Flex::new(peripherals.GPIO10.reborrow()));

        delay.delay_ms(20);
        id_rst.set_low();
        delay.delay_ms(40);
        id_rst.set_high();
        delay.delay_ms(50);

        let mut id_bus = Spi3Bus::new(id_cs, id_sck, id_sda, id_dc, id_rst, id_busy);
        id_bus.wait_busy(&mut delay).unwrap();
        let rev = id_bus.read_jd7966x_revision(&mut delay).unwrap();
        esp_println::println!("Panel R70H = {:02X?}: {}", rev, describe_revision(rev));
    }

    // Pin assignments are fixed by the ePaper Driver Board for XIAO.
    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());

    // JD79661 BUSY is active-LOW: the line is pulled low while the panel is working.
    // Pull it up so a missing or unpowered panel reads "idle" rather than "busy
    // forever", which would otherwise present as a hang rather than a blank screen.
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
    let controller =
        Jd79661Controller::new(ZJY122250_0213AJH_E5::WIDTH, ZJY122250_0213AJH_E5::HEIGHT);
    let mut epd = EpdBuilder::<_, ZJY122250_0213AJH_E5>::new(controller).build(epd_bus);

    esp_println::println!("Initializing JD79661 epdsi EPD driver...");
    epd.init(&mut delay).unwrap();

    // SAFETY: single-threaded example, and this is the only reference taken to FRAME_BUF.
    let frame_buf: &'static mut [u8; 8000] = unsafe { &mut *core::ptr::addr_of_mut!(FRAME_BUF) };
    let mut display = QuadColorBuffer::new(frame_buf, 122, 250);
    display.set_rotation(DisplayRotation::Rotate0);

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    // Outer border
    display.draw_rect_outline(0, 0, 122, 250, QuadColor::Black);

    Text::new("ZJY122250", Point::new(16, 18), text_style)
        .draw(&mut display)
        .unwrap();

    Line::new(Point::new(6, 23), Point::new(115, 23))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(&mut display)
        .unwrap();

    Text::new("JD79661 EPD", Point::new(6, 40), text_style)
        .draw(&mut display)
        .unwrap();

    // Colour bar: one swatch per ink, so a wrong plane mapping is obvious at a glance.
    display.draw_rect_outline(6, 48, 110, 16, QuadColor::Black);
    display.fill_rect(8, 50, 25, 12, QuadColor::Black);
    display.fill_rect(35, 50, 25, 12, QuadColor::Yellow);
    display.fill_rect(62, 50, 25, 12, QuadColor::Red);
    display.fill_rect(89, 50, 25, 12, QuadColor::White);
    display.draw_rect_outline(89, 50, 25, 12, QuadColor::Black);

    // Logos, horizontally centred: (122 - 64) / 2 = 29.
    let ferris_offset = Point::new(29, 68);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_offset, BinaryColor::On)
                .draw(&mut display)
                .unwrap();
        }
    }

    let rust_offset = Point::new(29, 138);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_offset, BinaryColor::On)
                .draw(&mut display)
                .unwrap();
        }
    }

    Text::new("XIAO C3", Point::new(28, 220), text_style)
        .draw(&mut display)
        .unwrap();

    Text::new("epdsi BWRY", Point::new(11, 240), text_style)
        .draw(&mut display)
        .unwrap();

    esp_println::println!("Sending 8,000-byte QuadColor 2bpp frame via epdsi...");
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .unwrap();

    esp_println::println!("Triggering display refresh (no fast waveform on colour panels)...");
    epd.refresh(&mut delay).unwrap();

    esp_println::println!("Powering off DC/DC...");
    epd.sleep(&mut delay).unwrap();

    esp_println::println!("JD79661 QuadColor epdsi demo finished successfully!");

    loop {
        delay.delay_ms(1000);
    }
}
