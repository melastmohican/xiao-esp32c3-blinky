//! # GDEY029F52 2.9" Quad-Color E-Paper Example (`epdsi`)
//!
//! Drives the 2.9" 128x296 Quad-Color Good Display `GDEY029F52` through epdsi's
//! `Jd79661Controller` and the `GDEY029F52` panel type. The drawing code, `QuadColor` and
//! `QuadColorBuffer` are the same as in `jd79661_zjy122250_epd`; only the panel, the buffer
//! size and the layout differ.
//!
//! Two pages, one full refresh each, with a 20 s rest between them:
//!
//! 1. **Geometry page.** A black border on all four edges, a red square at the RAM origin and
//!    black, yellow and red bands across the middle third. A wrong resolution shows up here as
//!    a shifted or sheared picture.
//! 2. **Demo page.** The same page as the other colour examples: header, four-swatch colour
//!    bar, Ferris and Rust logos, and footer labels.
//!
//! Do not loop this. Each refresh takes about 16 s.
//!
//! ## Panel check
//!
//! Before the refresh the example resets the panel and reads its `R70H` revision register over a
//! bit-banged 3-wire bus, before SPI takes SCK and MOSI, then logs which unit it is. Seen so far
//! on this panel: `83 02 01`. It never powers on or refreshes the panel at that point, and the
//! example carries on whatever it reads.
//!
//! ## Panel identification
//!
//! Check the rear sticker first. This panel's reads `029SW-F52-B2`. A `GDEY029F51H` (JD79667,
//! 168x384) is a different Good Display product and will not draw correctly here. A unit
//! ordered as an F51H arrived with the F52 sticker, so the order label is not enough.
//!
//! **Hardware-verified on a XIAO ESP32-C3** (5 Nov 2026), sticker `029SW-F52-B2`, lot
//! `N2B3F02`. Needs the `GDEY029F52` panel type, which is newer than epdsi 0.10.1.
//!
//! ## Hardware
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** [ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html) (24-pin FPC)
//! - **Display:** GDEY029F52, 2.9" Quad-Color, 128x296
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
//! ```bash
//! cargo run --release --example jd79661_gdey029f52_epd
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

/// 2bpp frame buffer: 32 bytes per row x 296 rows = 9,472 bytes. Held as a static rather than
/// on the stack, matching the other examples in this repository.
///
/// `0x55` is white repeated across all four 2-bit pixels in the byte.
static mut FRAME_BUF: [u8; 9472] = [0x55u8; 9472];

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

/// Says which kind of unit a `R70H` reading belongs to, from the units seen so far.
fn describe_revision(rev: [u8; 3]) -> &'static str {
    match rev {
        [0x83, 0x02, 0x01] => "matches the GDEY029F52 unit seen (sticker 029SW-F52-B2)",
        [0x03, 0x02, 0x01] => "matches the JD79661AA/JD79667AA datasheet value",
        [0xFF, 0xFF, 0xFF] | [0x00, 0x00, 0x00] => "read path not reaching the IC, no answer",
        _ => "new pattern, record it with the rear sticker text and lot",
    }
}

/// Page 1: geometry check.
fn draw_geometry_page(display: &mut QuadColorBuffer) {
    display.draw_rect_outline(0, 0, 128, 296, QuadColor::Black);
    display.fill_rect(1, 1, 24, 24, QuadColor::Red);
    // Three bands across the middle third, one per ink.
    display.fill_rect(1, 99, 42, 98, QuadColor::Black);
    display.fill_rect(43, 99, 42, 98, QuadColor::Yellow);
    display.fill_rect(85, 99, 42, 98, QuadColor::Red);
}

/// Page 2: the same page as the other colour examples.
fn draw_demo_page(display: &mut QuadColorBuffer) {
    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();
    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    // Outer border
    display.draw_rect_outline(0, 0, 128, 296, QuadColor::Black);

    Text::new("GDEY029F52", Point::new(14, 18), text_style)
        .draw(display)
        .unwrap();

    Line::new(Point::new(6, 23), Point::new(121, 23))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(display)
        .unwrap();

    Text::new("JD79661 EPD", Point::new(9, 40), text_style)
        .draw(display)
        .unwrap();

    // Colour bar: one swatch per ink, so a wrong plane mapping is obvious at a glance.
    display.draw_rect_outline(9, 48, 110, 16, QuadColor::Black);
    display.fill_rect(11, 50, 25, 12, QuadColor::Black);
    display.fill_rect(38, 50, 25, 12, QuadColor::Yellow);
    display.fill_rect(65, 50, 25, 12, QuadColor::Red);
    display.fill_rect(92, 50, 25, 12, QuadColor::White);
    display.draw_rect_outline(92, 50, 25, 12, QuadColor::Black);

    // Logos, horizontally centred: (128 - 64) / 2 = 32.
    let ferris_offset = Point::new(32, 78);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_offset, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }

    let rust_offset = Point::new(32, 150);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_offset, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }

    Text::new("XIAO C3", Point::new(29, 262), text_style)
        .draw(display)
        .unwrap();

    Text::new("epdsi BWRY", Point::new(14, 284), text_style)
        .draw(display)
        .unwrap();
}

#[main]
fn main() -> ! {
    let mut peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(100);

    esp_println::println!("Starting GDEY029F52 2.9\" Quad-Color EPD example (epdsi JD79661)");

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
    let controller = Jd79661Controller::new(GDEY029F52::WIDTH, GDEY029F52::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEY029F52>::new(controller).build(epd_bus);

    // SAFETY: single-threaded example, and this is the only reference taken to FRAME_BUF.
    let frame_buf: &'static mut [u8; 9472] = unsafe { &mut *core::ptr::addr_of_mut!(FRAME_BUF) };
    let mut display = QuadColorBuffer::new(frame_buf, 128, 296);
    display.set_rotation(DisplayRotation::Rotate0);

    for page in 1..=2 {
        esp_println::println!("--- page {} of 2 ---", page);
        epd.init(&mut delay).unwrap();

        // Start each page from white, so page 1 does not bleed into page 2.
        display.fill_rect(0, 0, 128, 296, QuadColor::White);
        if page == 1 {
            draw_geometry_page(&mut display);
        } else {
            draw_demo_page(&mut display);
        }

        esp_println::println!("Sending 9,472-byte QuadColor 2bpp frame via epdsi...");
        epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
            .unwrap();

        esp_println::println!("Triggering display refresh (about 16 s)...");
        epd.refresh(&mut delay).unwrap();

        esp_println::println!("Powering off DC/DC...");
        epd.sleep(&mut delay).unwrap();

        if page == 1 {
            esp_println::println!("Resting 20 s before page 2...");
            delay.delay_ms(20_000);
        }
    }

    esp_println::println!("GDEY029F52 QuadColor epdsi demo finished successfully!");

    loop {
        delay.delay_ms(1000);
    }
}
