//! # Good Display GDEW0215T12 2.15" Monochrome E-Paper Example (`epdsi`, XIAO ESP32-C3)
//!
//! XIAO ESP32-C3 on the Seeed Studio ePaper Driver Board for XIAO, driving the **GDEW0215T12**
//! 2.15" Monochrome (112x208) panel with `Uc8151dController` from `epdsi`.
//!
//! **Bench verified on 15 Oct 2026** (blocking, XIAO ESP32-C3 on the Seeed ePaper Driver Board):
//! full refresh, whole-frame partial and windowed partial updates all rendered correctly.
//!
//! Four screens, held briefly (`HOLD_MS`) so a run takes about 20 s (see `Screen n of 4` in the log):
//! 1. Full refresh on the panel's OTP waveform: header, Rust and Ferris logos, text labels.
//! 2. Partial profile, whole frame, no window (the vendor's `EPD_Dis_PartAll`), with a counter band.
//! 3. Partial profile with a window, old then new band data (the vendor's `EPD_Dis_Part`).
//! 4. The same again, counter advanced.
//!
//! Then deep sleep.
//!
//! ## Pin map (fixed by the driver board)
//!
//! | Signal | XIAO pin | ESP32-C3 GPIO |
//! |--------|----------|---------------|
//! | RST    | D0       | GPIO2         |
//! | CS     | D1       | GPIO3         |
//! | BUSY   | D2       | GPIO4 (active-LOW) |
//! | DC     | D3       | GPIO5         |
//! | SCK    | D8       | GPIO8         |
//! | MOSI   | D10      | GPIO10        |
//!
//! ## Run
//!
//! Flash, then unplug and replug USB, then attach the monitor (see `BRINGUP.md`). The example
//! waits 5 s at start so there is time to attach.
//!
//! ```bash
//! cargo run --release --example uc8151d_gdew0215t12_epd
//! espflash monitor
//! ```

#![no_std]
#![no_main]

use embedded_graphics::geometry::{Point, Size};
use embedded_graphics::mono_font::ascii::FONT_6X10;
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

/// Row stride in bytes: 112 / 8 = 14.
const STRIDE: usize = GDEW0215T12::WIDTH.div_ceil(8) as usize;

/// Full frame size: 14 x 208 = 2,912 bytes.
const FRAME_BYTES: usize = STRIDE * GDEW0215T12::HEIGHT as usize;

/// How long each screen is held before the next one, in milliseconds.
const HOLD_MS: u32 = 1_500;

/// Top row of the counter band repainted in the partial steps.
const BAND_Y: u32 = 176;

/// Height of the band in rows (y = 176..207).
const BAND_H: u32 = 32;

/// Last row of the band.
const BAND_END: u32 = BAND_Y + BAND_H - 1;

/// Byte offset of the band's first row.
const BAND_START_BYTE: usize = BAND_Y as usize * STRIDE;

/// Byte offset one past the band's last row.
const BAND_END_BYTE: usize = (BAND_END as usize + 1) * STRIDE;

/// X of the 64 px wide logos, centered in the 112 px wide panel.
const LOGO_X: i32 = (GDEW0215T12::WIDTH as i32 - 64) / 2;

/// Draws the static chrome: border, header, Rust and Ferris logos and two text labels.
fn draw_chrome(
    display: &mut PageBuffer,
    ferris_bmp: &Bmp<BinaryColor>,
    rust_bmp: &Bmp<BinaryColor>,
) {
    let stroke = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    Rectangle::new(
        Point::new(0, 0),
        Size::new(GDEW0215T12::WIDTH, GDEW0215T12::HEIGHT),
    )
    .into_styled(stroke)
    .draw(display)
    .unwrap();

    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    Text::new("GDEW0215T12", Point::new(8, 16), style)
        .draw(display)
        .unwrap();
    Line::new(Point::new(0, 24), Point::new(111, 24))
        .into_styled(stroke)
        .draw(display)
        .unwrap();

    // Same polarity handling as the other epdsi examples: the Rust BMP draws where it is `On`,
    // the Ferris BMP where it is `Off`. On `PageBuffer`, `On` is the black ink.
    let rust_pos = Point::new(LOGO_X, 30);
    for pixel in rust_bmp.pixels() {
        if pixel.1 == BinaryColor::On {
            Pixel(pixel.0 + rust_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }
    let ferris_pos = Point::new(LOGO_X, 100);
    for pixel in ferris_bmp.pixels() {
        if pixel.1 == BinaryColor::Off {
            Pixel(pixel.0 + ferris_pos, BinaryColor::On)
                .draw(display)
                .unwrap();
        }
    }

    Text::new("XIAO ESP32-C3", Point::new(8, 160), style)
        .draw(display)
        .unwrap();
    Text::new("epdsi UC8151D", Point::new(8, 172), style)
        .draw(display)
        .unwrap();
}

/// Draws the counter band: a label and a bar that grows with the count.
fn draw_band(display: &mut PageBuffer, count: u32) {
    let clear = PrimitiveStyle::with_fill(BinaryColor::Off);
    Rectangle::new(
        Point::new(1, BAND_Y as i32),
        Size::new(GDEW0215T12::WIDTH - 2, BAND_H - 1),
    )
    .into_styled(clear)
    .draw(display)
    .unwrap();

    let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let label = heapless_label(count);
    Text::new(label.as_str(), Point::new(8, BAND_Y as i32 + 12), style)
        .draw(display)
        .unwrap();

    let fill = PrimitiveStyle::with_fill(BinaryColor::On);
    Rectangle::new(
        Point::new(8, BAND_Y as i32 + 18),
        Size::new(20 * count + 4, 8),
    )
    .into_styled(fill)
    .draw(display)
    .unwrap();
}

/// A tiny fixed-size label buffer, so the example needs no allocator or `format_no_std`.
struct Label {
    buf: [u8; 12],
    len: usize,
}

impl Label {
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("?")
    }
}

/// Builds `"update N"` for a single-digit `N`.
fn heapless_label(count: u32) -> Label {
    let mut buf = [0u8; 12];
    let text = b"update ";
    buf[..text.len()].copy_from_slice(text);
    buf[text.len()] = b'0' + (count % 10) as u8;
    Label {
        buf,
        len: text.len() + 1,
    }
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(100);

    esp_println::println!("Starting GDEW0215T12 2.15\" Monochrome EPD example (epdsi UC8151D)");

    // Hold before touching the panel so `espflash monitor` can attach after a power cycle.
    delay.delay_ms(5_000);

    // Pin assignments are fixed by the ePaper Driver Board for XIAO.
    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    // UC8151D BUSY is active-LOW: pull up so a missing panel reads idle, not busy forever.
    let busy = Input::new(
        peripherals.GPIO4,
        InputConfig::default().with_pull(Pull::Up),
    );

    // 4 MHz: this chip's serial write cycle is at least 100 ns, so 10 MHz is the ceiling.
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
    let controller = Uc8151dController::new(GDEW0215T12::WIDTH, GDEW0215T12::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEW0215T12>::new(controller).build(epd_bus);

    esp_println::println!("Initializing UC8151D (Full profile)...");
    epd.init(&mut delay).unwrap();

    let mut buf = [0xFFu8; FRAME_BYTES];
    let mut display = PageBuffer::new(&mut buf, GDEW0215T12::WIDTH, GDEW0215T12::HEIGHT, 0);
    let ferris_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("ferrisbw.bmp")).unwrap();
    let rust_bmp: Bmp<BinaryColor> = Bmp::from_slice(include_bytes!("rustbw.bmp")).unwrap();

    esp_println::println!("--- Screen 1 of 4: full refresh on the OTP waveform ---");
    draw_chrome(&mut display, &ferris_bmp, &rust_bmp);

    // Old plane white first, as the vendor demo does, then the new plane.
    epd.set_window(0, 0, GDEW0215T12::WIDTH - 1, GDEW0215T12::HEIGHT - 1)
        .unwrap();
    epd.clear_frame(ColorChannel::RedYellow, 0xFF).unwrap();
    epd.set_window(0, 0, GDEW0215T12::WIDTH - 1, GDEW0215T12::HEIGHT - 1)
        .unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .unwrap();
    epd.refresh(&mut delay).unwrap();
    esp_println::println!("Full refresh done, holding briefly");
    delay.delay_ms(HOLD_MS);

    // The UC8151D's partial profile uploads its own LUT, so it needs a fresh init.
    epd.controller_mut()
        .set_refresh_mode(Uc8151dRefreshMode::Partial);
    epd.init(&mut delay).unwrap();
    epd.controller_mut().clear_window();

    esp_println::println!("--- Screen 2 of 4: partial profile, whole frame, no window ---");
    epd.set_window(0, 0, GDEW0215T12::WIDTH - 1, GDEW0215T12::HEIGHT - 1)
        .unwrap();
    epd.write_frame(ColorChannel::RedYellow, display.as_slice())
        .unwrap();
    epd.controller_mut().clear_window();
    draw_band(&mut display, 1);
    epd.set_window(0, 0, GDEW0215T12::WIDTH - 1, GDEW0215T12::HEIGHT - 1)
        .unwrap();
    epd.write_frame(ColorChannel::BlackWhite, display.as_slice())
        .unwrap();
    epd.controller_mut().clear_window();
    epd.refresh(&mut delay).unwrap();
    esp_println::println!("Whole-frame partial done, holding briefly");
    delay.delay_ms(HOLD_MS);

    for count in 2..=3u32 {
        esp_println::println!(
            "--- Screen {} of 4: partial window y={}..{}, update {} ---",
            count + 1,
            BAND_Y,
            BAND_END,
            count
        );
        // Old band = what the panel shows now, which is the previous pass's band.
        epd.set_window(0, BAND_Y, GDEW0215T12::WIDTH - 1, BAND_END)
            .unwrap();
        epd.write_frame(
            ColorChannel::RedYellow,
            &display.as_slice()[BAND_START_BYTE..BAND_END_BYTE],
        )
        .unwrap();
        draw_band(&mut display, count);
        epd.write_frame(
            ColorChannel::BlackWhite,
            &display.as_slice()[BAND_START_BYTE..BAND_END_BYTE],
        )
        .unwrap();
        epd.refresh(&mut delay).unwrap();
        epd.controller_mut().clear_window();
        esp_println::println!("Windowed partial done, holding briefly");
        delay.delay_ms(HOLD_MS);
    }

    esp_println::println!("--- Deep sleep ---");
    epd.sleep(&mut delay).unwrap();
    esp_println::println!("Display complete!");

    loop {
        delay.delay_ms(500);
    }
}
