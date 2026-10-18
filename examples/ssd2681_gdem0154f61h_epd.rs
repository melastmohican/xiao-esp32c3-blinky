//! # GDEM0154F61H 1.54" Quad-Colour e-Paper Example (`epdsi`)
//!
//! Sibling of `jd79660_gdem0154f51h_epd` for the Seeed Studio XIAO ESP32-C3 on the ePaper
//! Driver Board for XIAO. The 2bpp buffer and the `embedded-graphics` drawing are the same;
//! only the controller type differs.
//!
//! ## Phases
//!
//! 1. **Phase 1**: full refresh with `Ssd2681RefreshMode::Full`, timed. Bench verified on a
//!    XIAO ESP32-C3 (17 Oct 2026).
//! 2. **Phase 2**: the same frame with the footer relabelled, refreshed with
//!    `Ssd2681RefreshMode::Fast` (Good Display's `EPD_init_Fast()`) and timed against Phase 1.
//!    `Fast` writes registers the SSD2681 datasheet does not document (`EF`, `F6`, a bare
//!    `A5`) and forces the panel temperature to 92 degC, so check the colours as well as the
//!    time. Run on 17 Oct 2026: Full 20588 ms and Fast 12338 ms, with clean colours at room
//!    temperature.
//!
//! Draws a header, a four-swatch colour bar exercising all four inks, the Ferris and
//! Rust logos, and footer labels, then does a single full refresh and powers the panel
//! down. Expect several seconds per refresh.
//!
//! ## Panel identification
//!
//! Good Display `GDEM0154F61H` (rear marking `0154SW-F61H-B2`), 200x200, square. It has the
//! same 2bpp Black/White/Red/Yellow RAM layout as the `GDEM0154F51H` in the sibling
//! `jd79660_gdem0154f51h_epd` example, but a different IC (SSD2681) and a different init, so it
//! needs `Ssd2681Controller`. Picking the wrong controller draws nothing or wrong colours.
//!
//! ## Hardware
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** [ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html) (24-pin FPC)
//! - **Display:** GDEM0154F61H, 1.54" Quad-Colour, 200x200
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
//! cargo run --release --example ssd2681_gdem0154f61h_epd
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
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::{Instant, Rate};
use tinybmp::Bmp;

esp_bootloader_esp_idf::esp_app_desc!();

/// 4-color options for 2bpp e-Paper display (SSD2681)
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
        // RAM row stride is aligned to 8-pixel byte boundary. GDEM0154F61H is 200px
        // wide, already a multiple of 8, so this is a no-op here (unlike the 122px
        // ZJY122250 panel, which pads up to 128).
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

/// 2bpp frame buffer: 200x200 / 4 pixels-per-byte = 10,000 bytes. Held as a static
/// rather than on the stack, matching the other example in this repository — the
/// ESP32-C3's default stack is modest and this keeps the two consistent.
///
/// `0x55` is white repeated across all four 2-bit pixels in the byte.
static mut FRAME_BUF: [u8; 10_000] = [0x55u8; 10_000];

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(100);

    esp_println::println!("Starting GDEM0154F61H 1.54\" Quad-Colour EPD example (epdsi SSD2681)");

    // Pin assignments are fixed by the ePaper Driver Board for XIAO.
    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());

    // SSD2681 BUSY is active-LOW: the line is pulled low while the panel is working.
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
    let controller = Ssd2681Controller::new();
    let mut epd = EpdBuilder::<_, GDEM0154F61H>::new(controller).build(epd_bus);

    esp_println::println!("Initializing SSD2681 epdsi EPD driver...");
    epd.init(&mut delay).unwrap();

    // SAFETY: single-threaded example, and this is the only reference taken to FRAME_BUF.
    let frame_buf: &'static mut [u8; 10_000] = unsafe { &mut *core::ptr::addr_of_mut!(FRAME_BUF) };
    let mut display = QuadColorBuffer::new(frame_buf, 200, 200);
    display.set_rotation(DisplayRotation::Rotate0);

    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    let text_style = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);

    // Outer border
    display.draw_rect_outline(0, 0, 200, 200, QuadColor::Black);

    // "GDEM0154F61H" is 12 chars * 10px = 120px, centered: (200 - 120) / 2 = 40.
    Text::new("GDEM0154F61H", Point::new(40, 18), text_style)
        .draw(&mut display)
        .unwrap();

    Line::new(Point::new(6, 23), Point::new(193, 23))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(&mut display)
        .unwrap();

    Text::new("SSD2681 EPD", Point::new(6, 40), text_style)
        .draw(&mut display)
        .unwrap();

    // Colour bar: one swatch per ink, so a wrong plane mapping is obvious at a glance.
    display.draw_rect_outline(6, 48, 188, 14, QuadColor::Black);
    display.fill_rect(8, 50, 42, 10, QuadColor::Black);
    display.fill_rect(54, 50, 42, 10, QuadColor::Yellow);
    display.fill_rect(100, 50, 42, 10, QuadColor::Red);
    display.fill_rect(146, 50, 42, 10, QuadColor::White);
    display.draw_rect_outline(146, 50, 42, 10, QuadColor::Black);

    // Logos side by side, like the Feather example: Ferris at x=20, Rust at x=116.
    let ferris_offset = Point::new(20, 76);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_offset, BinaryColor::On)
                .draw(&mut display)
                .unwrap();
        }
    }

    let rust_offset = Point::new(116, 76);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_offset, BinaryColor::On)
                .draw(&mut display)
                .unwrap();
        }
    }

    Text::new("XIAO C3", Point::new(65, 182), text_style)
        .draw(&mut display)
        .unwrap();

    Text::new("epdsi Quad", Point::new(50, 196), text_style)
        .draw(&mut display)
        .unwrap();

    esp_println::println!("Sending 10,000-byte QuadColor 2bpp frame via epdsi...");
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .unwrap();

    esp_println::println!("Triggering display refresh (Full waveform, several seconds)...");
    let start = Instant::now();
    epd.refresh(&mut delay).unwrap();
    let full_ms = start.elapsed().as_millis();
    esp_println::println!("Phase 1 Full: {} ms", full_ms);

    esp_println::println!("Powering off DC/DC...");
    epd.sleep(&mut delay).unwrap();

    // --- Phase 2: Ssd2681RefreshMode::Fast, the vendor demo's `EPD_init_Fast()`. ---
    // The mode is read by `init`, so it takes effect on the next init after the sleep above.
    // The footer is relabelled so the second frame can be told apart from the first.
    delay.delay_ms(3000);
    display.fill_rect(2, 150, 196, 48, QuadColor::White);
    Text::new("Fast init", Point::new(55, 182), text_style)
        .draw(&mut display)
        .unwrap();
    Text::new("epdsi Quad", Point::new(50, 196), text_style)
        .draw(&mut display)
        .unwrap();

    epd.controller_mut()
        .set_refresh_mode(Ssd2681RefreshMode::Fast);
    epd.init(&mut delay).unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .unwrap();
    let start = Instant::now();
    epd.refresh(&mut delay).unwrap();
    let fast_ms = start.elapsed().as_millis();
    esp_println::println!("Phase 2 Fast: {} ms (Full was {} ms)", fast_ms, full_ms);
    epd.sleep(&mut delay).unwrap();

    esp_println::println!("SSD2681 QuadColor epdsi demo finished successfully!");

    loop {
        delay.delay_ms(1000);
    }
}
