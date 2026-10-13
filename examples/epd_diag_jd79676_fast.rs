//! # JD79676 fast-update probe (GDEY0213F52)
//!
//! Runs Good Display's own `EPD_init_Fast()` sequence from the `GDEY0213F52` V2.0 demo
//! (`A32-GDEY0213F52-20240827`) next to the plain full init and times both refreshes.
//! `epdsi` does not implement the fast path, so this example sends the raw commands
//! through `SpiBusWrapper` and bypasses the `Jd79676Controller`.
//!
//! ## Sequence under test
//!
//! | Step | Full init (`EPD_init`) | Fast init (`EPD_init_Fast`) |
//! |------|------------------------|-----------------------------|
//! | Pre-reset delay | 20 ms | 100 ms |
//! | RST low / high | 40 ms / 50 ms | 10 ms / 10 ms |
//! | Wait BUSY | yes | yes |
//! | `E0 02` | no | yes (CCSET, `TSFIX`: temperature from `TS_SET`) |
//! | `E6 5A` | no | yes (not in the JD79676AA datasheet; assumed `TS_SET`) |
//! | `A5` bare command | no | yes (not in the datasheet) |
//! | Wait BUSY | no | yes |
//! | `E9 01` | yes | yes |
//! | `04` (PON), wait BUSY | yes | yes |
//!
//! Both then send one 8,000-byte frame to `0x10`, refresh with `12 00`, and sleep with
//! `02 00` then `07 A5`.
//!
//! ## What to look for
//!
//! 1. The printed refresh times: full is documented at about 11 s.
//! 2. The picture after each refresh. Frame 1 is four colour bands (black, yellow, red,
//!    white, top to bottom), frame 2 is the same bands rotated by one. If the fast refresh
//!    leaves frame 1 visible, that is ghosting, which is the cost of a fast waveform.
//! 3. Whether the fast refresh completes at all. `E6` and the bare `A5` are undocumented,
//!    so a hang on BUSY is a possible and informative result.
//!
//! The panel is refreshed twice per run, with a 20 s pause between, so do not loop this.
//!
//! ## Hardware
//!
//! - **Board:** Seeed Studio XIAO ESP32-C3
//! - **Carrier:** [ePaper Driver Board for XIAO](https://www.seeedstudio.com/ePaper-breakout-Board-for-XIAO-V2-p-6374.html) (24-pin FPC)
//! - **Display:** GDEY0213F52 (rear sticker `0213SW-F52-B2`). Do not run this on the
//!   Adafruit `ZJY122250` panel, which is JD79661 and has a different init.
//!
//! ```bash
//! cargo run --release --example epd_diag_jd79676_fast
//! ```

#![no_std]
#![no_main]

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

/// 2bpp, 128 px (32 bytes) per row, 250 rows.
const ROW_BYTES: usize = 32;
const ROWS: usize = 250;
const FRAME_BYTES: usize = ROW_BYTES * ROWS;

/// One byte holds four pixels, so a fill byte repeats the colour code.
const BLACK: u8 = 0x00;
const WHITE: u8 = 0x55;
const YELLOW: u8 = 0xAA;
const RED: u8 = 0xFF;

static mut FRAME_BUF: [u8; FRAME_BYTES] = [WHITE; FRAME_BYTES];

/// Fills four horizontal bands, top to bottom, with the given fill bytes.
fn fill_bands(buf: &mut [u8; FRAME_BYTES], bands: [u8; 4]) {
    let band_rows = ROWS.div_ceil(4);
    for (row, chunk) in buf.chunks_mut(ROW_BYTES).enumerate() {
        chunk.fill(bands[(row / band_rows).min(3)]);
    }
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();

    // Startup hold so `espflash monitor` can attach before the panel is touched.
    delay.delay_ms(8000);

    esp_println::println!("=== diag: JD79676 fast-update probe (GDEY0213F52) ===");

    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
    // Active-LOW BUSY, pulled up so an unpowered panel reads idle.
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
    let mut bus = SpiBusWrapper::new(spi_device, dc, rst, busy);

    // SAFETY: single-threaded example, and this is the only reference taken to FRAME_BUF.
    let frame: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(FRAME_BUF) };

    // ---- Pass 1: plain full init, as `Jd79676Controller` does it. ----
    esp_println::println!("--- pass 1: full init (E9 01, PON) ---");
    fill_bands(frame, [BLACK, YELLOW, RED, WHITE]);

    delay.delay_ms(20);
    bus.hard_reset(&mut delay, 40).unwrap();
    bus.wait_busy(false).unwrap();
    bus.send_command_with_data(0xE9, &[0x01]).unwrap();
    bus.send_command(0x04).unwrap();
    bus.wait_busy(false).unwrap();

    bus.send_command_with_data(0x10, frame).unwrap();
    let t = Instant::now();
    bus.send_command_with_data(0x12, &[0x00]).unwrap();
    bus.wait_busy(false).unwrap();
    esp_println::println!("full refresh: {} ms", t.elapsed().as_millis());

    bus.send_command_with_data(0x02, &[0x00]).unwrap();
    bus.wait_busy(false).unwrap();
    bus.send_command_with_data(0x07, &[0xA5]).unwrap();

    esp_println::println!("pass 1 done, waiting 20 s");
    delay.delay_ms(20_000);

    // ---- Pass 2: the vendor's `EPD_init_Fast()`. ----
    esp_println::println!("--- pass 2: fast init (E0 02, E6 5A, A5, E9 01, PON) ---");
    fill_bands(frame, [RED, WHITE, BLACK, YELLOW]);

    delay.delay_ms(100);
    bus.hard_reset(&mut delay, 10).unwrap();
    bus.wait_busy(false).unwrap();

    bus.send_command_with_data(0xE0, &[0x02]).unwrap();
    bus.send_command_with_data(0xE6, &[0x5A]).unwrap();
    bus.send_command(0xA5).unwrap();
    esp_println::println!("sent E0 02, E6 5A, A5, waiting for BUSY...");
    let t = Instant::now();
    bus.wait_busy(false).unwrap();
    esp_println::println!("A5 busy: {} ms", t.elapsed().as_millis());

    bus.send_command_with_data(0xE9, &[0x01]).unwrap();
    bus.send_command(0x04).unwrap();
    bus.wait_busy(false).unwrap();

    bus.send_command_with_data(0x10, frame).unwrap();
    let t = Instant::now();
    bus.send_command_with_data(0x12, &[0x00]).unwrap();
    bus.wait_busy(false).unwrap();
    esp_println::println!("fast refresh: {} ms", t.elapsed().as_millis());

    bus.send_command_with_data(0x02, &[0x00]).unwrap();
    bus.wait_busy(false).unwrap();
    bus.send_command_with_data(0x07, &[0xA5]).unwrap();

    esp_println::println!("Done. Note the times above and whether frame 1 ghosts in frame 2.");

    loop {
        delay.delay_ms(1000);
    }
}
