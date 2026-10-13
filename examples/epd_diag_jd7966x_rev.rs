//! # JD79661 / JD79676 chip revision probe
//!
//! Reads the `R70H` REVISION register of the 2.13" Quad-Color controllers, which returns
//! three bytes, and compares them with what has been seen on real panels.
//!
//! | `70H` returns | Seen on | Notes |
//! |---------------|---------|-------|
//! | `09 02 01` | Seeed 5779, sticker `0213SW-F52-B2` lot `NRA2L21` | JD79676AA datasheet value |
//! | `89 02 01` | Good Display `GDEY0213F52`, sticker `0213SW-F52-B2` lot `N5B3S02` | top bit set, not in the datasheet |
//! | `09 01 01` | Adafruit, sticker `ZJY122250-0213EAH-E5` | not in either datasheet |
//!
//! The JD79661AA datasheet (V1.0.4, section 8.2.22) lists `03 02 01`. No unit has returned
//! it yet, so a match on that value would be new information.
//!
//! ## Why this exists
//!
//! The Adafruit `GDEY0213F51`, the Good Display `GDEY0213F52` and the Seeed Studio 5779 all
//! carry the same `FPC-J002` flex ribbon stamp and differ only in their rear stickers. The
//! JD79661 init also draws a JD79676 panel, so a picture does not identify the chip. The
//! revision register separates the units, which a picture cannot.
//!
//! //! ## What it does
//!
//! 1. Resets the panel with the Good Display demo's timings (20 ms, 40 ms low, 50 ms high).
//! 2. Sends command `0x70`, then clocks three bytes back on the bidirectional SDA line
//!    using `epdsi`'s bit-banged `Spi3Bus`.
//! 3. Repeats that three times and prints every result.
//!
//! It never powers the panel on and never refreshes it, so the image on the panel is
//! left exactly as it was. Run it once per panel and compare the bytes.
//!
//! ## If the bytes are `FF FF FF` or `00 00 00`
//!
//! The read path is not reaching the IC. That is a wiring or timing result, not a chip
//! result: SDA is the same line as MOSI here, and the driver board may buffer it.
//!
//! ## Sending results back
//!
//! A new byte pattern is worth recording together with the rear sticker text and lot.
//!
//! ## Hardware
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** [ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html) (24-pin FPC)
//!
//! | Signal | XIAO pin | ESP32-C3 GPIO |
//! |--------|----------|---------------|
//! | RST    | D0       | GPIO2         |
//! | CS     | D1       | GPIO3         |
//! | BUSY   | D2       | GPIO4         |
//! | DC     | D3       | GPIO5         |
//! | SCK    | D8       | GPIO8         |
//! | SDA    | D10      | GPIO10        |
//!
//! ```bash
//! cargo run --release --example epd_diag_jd7966x_rev
//! ```

#![no_std]
#![no_main]

use embedded_hal::delay::DelayNs;
use epdsi::prelude::*;
use esp_backtrace as _;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Flex, Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::main;

esp_bootloader_esp_idf::esp_app_desc!();

/// Datasheet `R70H` values. Only the JD79676AA one has been seen on hardware.
const REV_JD79661_DATASHEET: [u8; 3] = [0x03, 0x02, 0x01];
const REV_JD79676_DATASHEET: [u8; 3] = [0x09, 0x02, 0x01];
/// Values observed on real units (see the table in the module doc).
const REV_GD_F52_LOT_N5B3S02: [u8; 3] = [0x89, 0x02, 0x01];
const REV_ADAFRUIT_ZJY122250: [u8; 3] = [0x09, 0x01, 0x01];

/// The bidirectional SDA line, as a `Flex` pin that `Spi3Bus` can turn around per byte.
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

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();

    // Startup hold, matching the other diagnostics: attach `espflash monitor` after the
    // replug and before the panel is touched.
    delay.delay_ms(8000);

    esp_println::println!("=== diag: JD7966x R70H revision probe ===");
    esp_println::println!("JD79661AA datasheet {:02X?}", REV_JD79661_DATASHEET);
    esp_println::println!("JD79676AA datasheet {:02X?}", REV_JD79676_DATASHEET);
    esp_println::println!("seen: Good Display F52 {:02X?}", REV_GD_F52_LOT_N5B3S02);
    esp_println::println!("seen: Adafruit ZJY122250 {:02X?}", REV_ADAFRUIT_ZJY122250);

    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let mut rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    let sck = Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default());
    // Active-LOW BUSY, pulled up so an unpowered panel reads idle.
    let busy = Input::new(
        peripherals.GPIO4,
        InputConfig::default().with_pull(Pull::Up),
    );
    let sda = SdaPin(Flex::new(peripherals.GPIO10));

    // Reset with the Good Display demo's minimums: 20 ms settle, 40 ms low, 50 ms high.
    delay.delay_ms(20);
    rst.set_low();
    delay.delay_ms(40);
    rst.set_high();
    delay.delay_ms(50);

    let mut bus = Spi3Bus::new(cs, sck, sda, dc, rst, busy);
    bus.wait_busy(&mut delay).unwrap();

    let mut first = [0u8; 3];
    for attempt in 0..3 {
        let rev = bus.read_jd7966x_revision(&mut delay).unwrap();
        esp_println::println!("attempt {}: R70H = {:02X?}", attempt + 1, rev);
        if attempt == 0 {
            first = rev;
        }
        delay.delay_ms(10);
    }

    if first == REV_JD79676_DATASHEET {
        esp_println::println!("Result: matches the JD79676AA datasheet (seen on Seeed 5779)");
    } else if first == REV_GD_F52_LOT_N5B3S02 {
        esp_println::println!("Result: matches the Good Display F52 unit seen so far");
    } else if first == REV_ADAFRUIT_ZJY122250 {
        esp_println::println!("Result: matches the Adafruit ZJY122250 unit seen so far");
    } else if first == REV_JD79661_DATASHEET {
        esp_println::println!("Result: matches the JD79661AA datasheet (not seen before)");
    } else if first == [0xFF; 3] || first == [0x00; 3] {
        esp_println::println!(
            "Result: read path not reaching the IC (all {:02X}), see the doc comment",
            first[0]
        );
    } else {
        esp_println::println!(
            "Result: new pattern {:02X?}, record it with the sticker text and lot",
            first
        );
    }

    esp_println::println!("Done. The panel was not powered on or refreshed.");

    loop {
        delay.delay_ms(1000);
    }
}
