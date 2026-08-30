//! # GDEY037T03 diagnostic: does this panel work on the C3, and if not, is the clock why?
//!
//! Established already: BUSY is driven (not floating) and responds to a hardware reset,
//! so the panel is connected, powered and alive. Yet `refresh` returns in 0 ms and the
//! display never changes.
//!
//! **Re-run this — the premise has changed.** That was previously read as "the panel is not
//! acting on SPI commands", with stock Arduino GxEPD2 failing identically taken as proof it was
//! not a driver problem. Then the 3.52" panel on this same board produced *the same 0 ms symptom*
//! and it turned out to be three driver bugs, all of which reproduced only on the C3. "Refresh
//! returns in 0 ms" means **BUSY was never observed asserted** — either polled too early, or the
//! controller ignored the command — which is not the same as a panel ignoring SPI. GxEPD2's busy
//! wait has the same race shape, so it failing too is weaker evidence than it looked.
//!
//! `epdsi` now waits for the BUSY *edge* after `DISPLAY_REFRESH` on this variant. A run that still
//! reports ~0 ms now means BUSY genuinely never asserted; a run reporting ~500 ms means the edge
//! wait timed out, which says the same thing more explicitly. Either way the reading is sharper
//! than before.
//!
//! The clock sweep is retained because it is still worth ruling out: on the XIAO ESP32-C3, SCK
//! lands on D8 = **GPIO8, an ESP32-C3 strapping pin** carrying a boot-time pull-up, and this is
//! the largest-COG panel in the set. Four smaller panels work fine here at 4 MHz.
//!
//! Each step fills the panel BLACK. A refresh under 500 ms means the panel never started;
//! anything longer means it did, and that rate works. The sweep stops at the first rate that
//! works, so a working panel is never refreshed repeatedly.
//!
//! ## Run — flash first, power-cycle second
//!
//! Power-up **is** program-start on this board, so replugging first runs the previous binary and
//! flashing then interrupts it. See `BRINGUP.md`.
//!
//! ```bash
//! cargo run --release --example epd_diag_370   # flash. Ignore this run.
//! # unplug USB, wait a few seconds, plug back in
//! espflash monitor                             # attach during the startup hold
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

const STRIDE: usize = GDEY037T03::WIDTH.div_ceil(8) as usize;
const FRAME_BYTES: usize = STRIDE * GDEY037T03::HEIGHT as usize;

static mut BUF: [u8; FRAME_BYTES] = [0xFFu8; FRAME_BYTES];

#[main]
fn main() -> ! {
    let mut peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(200);

    esp_println::println!("=== GDEY037T03: SPI clock sweep ===");
    esp_println::println!("BUSY is active-LOW on this panel. Watch the display, not just the log.");

    // Hold before touching the panel so `espflash monitor` can be attached after a replug.
    for remaining in (1..=8u32).rev() {
        esp_println::println!("starting in {} s (attach monitor now)", remaining);
        delay.delay_ms(1000);
    }

    for khz in [4000u32, 1000, 500, 200, 100] {
        esp_println::println!("--- SPI @ {} kHz: fill BLACK ---", khz);

        // Rebuild every handle each pass so the previous borrow has ended.
        let cs = Output::new(
            peripherals.GPIO3.reborrow(),
            Level::High,
            OutputConfig::default(),
        );
        let dc = Output::new(
            peripherals.GPIO5.reborrow(),
            Level::Low,
            OutputConfig::default(),
        );
        let rst = Output::new(
            peripherals.GPIO2.reborrow(),
            Level::High,
            OutputConfig::default(),
        );
        // UC8253 BUSY is active-LOW: pull up so an absent panel reads idle.
        let busy = Input::new(
            peripherals.GPIO4.reborrow(),
            InputConfig::default().with_pull(Pull::Up),
        );

        let spi = Spi::new(
            peripherals.SPI2.reborrow(),
            SpiConfig::default()
                .with_frequency(Rate::from_khz(khz))
                .with_mode(Mode::_0),
        )
        .expect("SPI2 config")
        .with_sck(peripherals.GPIO8.reborrow())
        .with_mosi(peripherals.GPIO10.reborrow());

        let spi_device = ExclusiveDevice::new_no_delay(spi, cs).expect("SpiDevice");
        let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
        let controller = Uc8253Controller::new(GDEY037T03::WIDTH, GDEY037T03::HEIGHT);
        let mut epd = EpdBuilder::<_, GDEY037T03>::new(controller).build(epd_bus);

        epd.init(&mut delay).unwrap();
        epd.controller_mut()
            .set_refresh_mode(Uc8253RefreshMode::Full);

        // Prime the old plane white so the update has a clean base.
        epd.set_window(0, 0, GDEY037T03::WIDTH - 1, GDEY037T03::HEIGHT - 1)
            .unwrap();
        epd.clear_frame(ColorChannel::RedYellow, 0xFF).unwrap();

        // SAFETY: single-threaded, and the previous iteration's borrow has ended.
        let buf: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(BUF) };
        buf.fill(0x00);

        epd.set_window(0, 0, GDEY037T03::WIDTH - 1, GDEY037T03::HEIGHT - 1)
            .unwrap();
        epd.write_frame(ColorChannel::BlackWhite, &buf[..]).unwrap();

        let t = Instant::now();
        epd.refresh(&mut delay).unwrap();
        let ms = t.elapsed().as_millis();

        if ms < 500 {
            esp_println::println!("    {} ms -- panel never started", ms);
        } else {
            esp_println::println!("    {} ms -- PANEL RESPONDED, should be BLACK", ms);
            esp_println::println!("    >>> {} kHz works. Lower the clock in the example.", khz);
            break;
        }

        delay.delay_ms(500);
    }

    esp_println::println!("=== done ===");
    esp_println::println!("If no rate worked, the clock is not the problem and this");
    esp_println::println!("panel/board combination needs a different explanation.");

    loop {
        delay.delay_ms(1000);
    }
}
