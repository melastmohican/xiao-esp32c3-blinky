//! # GDEM0213B74 black / white / stripe confirmation
//!
//! Separates two questions that the full example cannot:
//!
//! 1. **Does the data path work?** Solid black then solid white, timed. A full refresh
//!    on this 2.13" panel should be roughly 2-3 s; far longer means the busy-wait is not
//!    tracking the panel, far shorter means it is not waiting at all.
//! 2. **Is the geometry right?** A vertical stripe pattern drawn in buffer coordinates:
//!    a black bar down the leftmost 16 px, and a black bar down the rightmost 16 px of
//!    the 122 px visible width. If those land at the panel edges, coordinates are
//!    correct. If they are shifted or wrapped, the RAM window or stride is wrong — which
//!    is what the offset logos in the full example look like.
//!
//! ```bash
//! cargo run --release --example epd_diag_213
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
use esp_hal::spi::Mode;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::time::{Instant, Rate};

esp_bootloader_esp_idf::esp_app_desc!();

const STRIDE: usize = GDEM0213B74::WIDTH.div_ceil(8) as usize; // 16 bytes = 128 px
const FRAME_BYTES: usize = STRIDE * GDEM0213B74::HEIGHT as usize; // 4000

static mut BUF: [u8; FRAME_BYTES] = [0xFFu8; FRAME_BYTES];

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut delay = Delay::new();
    delay.delay_ms(200);

    esp_println::println!("=== diag: GDEM0213B74 ===");
    esp_println::println!(
        "panel {}x{}, stride {} bytes ({} px of RAM per row, {} px visible)",
        GDEM0213B74::WIDTH,
        GDEM0213B74::HEIGHT,
        STRIDE,
        STRIDE * 8,
        GDEM0213B74::WIDTH
    );

    let cs = Output::new(peripherals.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
    let rst = Output::new(peripherals.GPIO2, Level::High, OutputConfig::default());
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
    let epd_bus = SpiBusWrapper::new(spi_device, dc, rst, busy);
    let controller = Ssd1680Controller::new(GDEM0213B74::WIDTH, GDEM0213B74::HEIGHT);
    let mut epd = EpdBuilder::<_, GDEM0213B74>::new(controller).build(epd_bus);

    esp_println::println!("init...");
    epd.init(&mut delay).unwrap();
    epd.controller_mut()
        .set_refresh_mode(Ssd168xRefreshMode::Full);

    // SAFETY: single-threaded, only reference taken.
    let buf: &'static mut [u8; FRAME_BYTES] = unsafe { &mut *core::ptr::addr_of_mut!(BUF) };

    let show = |epd: &mut EpdDriver<_, _, GDEM0213B74>,
                    delay: &mut Delay,
                    label: &str,
                    buf: &[u8]| {
        epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
            .unwrap();
        epd.set_cursor(0, 0).unwrap();
        epd.clear_frame(ColorChannel::RedYellow, 0xFF).unwrap();

        epd.set_window(0, 0, GDEM0213B74::WIDTH - 1, GDEM0213B74::HEIGHT - 1)
            .unwrap();
        epd.set_cursor(0, 0).unwrap();
        epd.write_frame(ColorChannel::BlackWhite, buf).unwrap();

        let start = Instant::now();
        epd.refresh(delay).unwrap();
        let ms = start.elapsed().as_millis();
        esp_println::println!("  {}: refresh {} ms  (expect ~2000-3000)", label, ms);
        delay.delay_ms(4000);
    };

    // 1. solid black
    esp_println::println!("--- solid BLACK ---");
    buf.fill(0x00);
    show(&mut epd, &mut delay, "BLACK", buf);

    // 2. solid white
    esp_println::println!("--- solid WHITE ---");
    buf.fill(0xFF);
    show(&mut epd, &mut delay, "WHITE", buf);

    // 3. edge stripes, drawn directly into the byte buffer so no drawing code is
    //    involved -- only the RAM window and stride are being tested.
    //    Byte 0 covers x=0..7, byte 15 covers x=120..127 (x=122..127 is off-panel).
    esp_println::println!("--- EDGE STRIPES (leftmost 8 px and x=112..119 black) ---");
    buf.fill(0xFF);
    for row in 0..GDEM0213B74::HEIGHT as usize {
        buf[row * STRIDE] = 0x00; // x = 0..7    -> should hug the LEFT edge
        buf[row * STRIDE + 14] = 0x00; // x = 112..119 -> should sit just shy of the RIGHT edge
    }
    show(&mut epd, &mut delay, "STRIPES", buf);

    esp_println::println!("=== done ===");
    esp_println::println!("Expect: one black bar hard against the left edge, and a second");
    esp_println::println!("about 8 px in from the right. Anything else means the RAM window");
    esp_println::println!("or stride is wrong, not the drawing code.");

    loop {
        delay.delay_ms(1000);
    }
}
