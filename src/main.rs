//! Heartbeat for the Seeed Studio XIAO ESP32-C3.
//!
//! Unlike the XIAO ESP32-C6 (which has `LED_BUILTIN` on GPIO15), the XIAO ESP32-C3 has
//! **no user-controllable onboard LED** — only a charge indicator. So this prints a
//! counter over USB serial and also toggles D6 / GPIO6, where an external LED can be
//! wired if you want something visible.
//!
//! D6 is chosen because it is clear of every pin the ePaper Driver Board uses
//! (D0, D1, D2, D3, D8, D10), so the blinky and the ePaper examples cannot collide.
//!
//! Build with `--release`; a debug image is over 5 MB against roughly 750 KB.
//!
//! ```bash
//! cargo run --release
//! ```

#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    gpio::{Level, Output, OutputConfig},
    main,
};

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let delay = Delay::new();

    // D6 on the XIAO ESP32-C3. Free for an external LED; not used by the ePaper board.
    let mut led = Output::new(peripherals.GPIO6, Level::Low, OutputConfig::default());

    esp_println::println!("XIAO ESP32-C3 alive");

    let mut ticks: u32 = 0;
    loop {
        led.toggle();
        ticks += 1;
        esp_println::println!("tick {}", ticks);
        delay.delay_millis(500);
    }
}
