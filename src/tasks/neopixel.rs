use core::borrow::{Borrow, BorrowMut};

use defmt::*;
use embassy_embedded_hal::shared_bus::asynch::spi;
use embassy_executor::Spawner;
use embassy_rp::peripherals;
use embassy_rp::spi::{Config, Phase, Polarity, Spi};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::blocking_mutex::ThreadModeMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Timer};
use smart_leds::{brightness, RGB, RGB8};
use ws2812_async::Ws2812;
use {defmt_rtt as _, panic_probe as _};

const NUM_LEDS: usize = 16;

pub struct NeopixelManager {
    alarm_brightness: u8,
    clock_brightness: u8,
}

pub type SpiType =
    Mutex<ThreadModeRawMutex, Option<Spi<'static, peripherals::SPI0, embassy_rp::spi::Async>>>;

pub type NeopixelManagerType = Mutex<ThreadModeRawMutex, Option<NeopixelManager>>;

impl NeopixelManager {
    pub fn new(alarm_brightness: u8, clock_brightness: u8) -> Self {
        Self {
            alarm_brightness,
            clock_brightness,
        }
    }

    pub fn alarm_brightness(&self) -> u8 {
        self.alarm_brightness
    }

    pub fn clock_brightness(&self) -> u8 {
        self.clock_brightness
    }

    /// Function to convert RGB to GRB, we need ths because the crate ws2812_async uses GRB. That in itself is a bug, but we can work around it.
    pub fn rgb_to_grb(&self, color: (u8, u8, u8)) -> RGB8 {
        RGB8 {
            r: color.1,
            g: color.0,
            b: color.2,
        }
    }
}

#[embassy_executor::task]
pub async fn analog_clock(
    _spawner: Spawner,
    spi_np: &'static SpiType,
    neopixel_mgr: &'static NeopixelManagerType,
) {
    info!("Analog clock task start");

    // Lock the mutex asynchronously
    let mut spi_np_guard = spi_np.lock().await;
    let mut neopixel_mgr_guard = neopixel_mgr.lock().await;

    // Check if the mutex actually contains a NeopixelManager object
    let np_mgr: NeopixelManager;
    if let Some(mut np_mgr_inner) = neopixel_mgr_guard.take() {
        np_mgr = np_mgr_inner;
    } else {
        return; // Handle the case where the NeopixelManager object was not available (e.g., already taken or never set)
    }

    // Check if the mutex actually contains an Spi object
    let mut spi: Spi<'static, peripherals::SPI0, embassy_rp::spi::Async>;
    if let Some(mut spi_inner) = spi_np_guard.take() {
        info!("SPI object was available in the mutex.");
        spi = spi_inner;
    } else {
        return; // Handle the case where the SPI object was not available (e.g., already taken or never set)
    }

    let mut np: Ws2812<_, { 12 * NUM_LEDS }> = Ws2812::new(&mut spi); // Use the SPI object to create Ws2812

    info!("Switching all LEDs off");
    let data = [RGB8::default(); 16];
    np.write(brightness(data.iter().cloned(), np_mgr.alarm_brightness()))
        .await
        .ok();

    info!("Setting all LEDs to red");
    let red = np_mgr.rgb_to_grb((255, 0, 0));
    let data = [red; 16];
    let _ = np
        .write(brightness(data.iter().cloned(), np_mgr.clock_brightness()))
        .await;
    Timer::after(Duration::from_secs(3)).await;

    info!("Switching all LEDs off");
    let data = [RGB8::default(); 16];
    let _ = np.write(brightness(data.iter().cloned(), 0)).await;

    info!("hand back objects to the mutex");
    // put the objects back into the Mutex for future use
    *spi_np_guard = Some(spi);
    *neopixel_mgr_guard = Some(np_mgr);
}

#[embassy_executor::task]
pub async fn sunrise(
    _spawner: Spawner,
    spi_np: &'static SpiType,
    neopixel_mgr: &'static NeopixelManagerType,
) {
    info!("Sunrise task start");
    // Lock the mutex asynchronously
    let mut spi_np_guard = spi_np.lock().await;
    let mut neopixel_mgr_guard = neopixel_mgr.lock().await;

    // Check if the mutex actually contains a NeopixelManager object
    let np_mgr: NeopixelManager;
    if let Some(mut np_mgr_inner) = neopixel_mgr_guard.take() {
        np_mgr = np_mgr_inner;
    } else {
        return; // Handle the case where the NeopixelManager object was not available (e.g., already taken or never set)
    }

    // Check if the mutex actually contains an Spi object
    let mut spi: Spi<'static, peripherals::SPI0, embassy_rp::spi::Async>;
    if let Some(mut spi_inner) = spi_np_guard.take() {
        info!("SPI object was available in the mutex.");
        spi = spi_inner;
    } else {
        return; // Handle the case where the SPI object was not available (e.g., already taken or never set)
    }

    let mut np: Ws2812<_, { 12 * NUM_LEDS }> = Ws2812::new(&mut spi); // Use the SPI object to create Ws2812

    info!("Switching all LEDs off");
    let data = [RGB8::default(); 16];
    np.write(brightness(data.iter().cloned(), np_mgr.alarm_brightness()))
        .await
        .ok();

    info!("Setting all LEDs to green");
    let red = np_mgr.rgb_to_grb((0, 255, 0));
    let data = [red; 16];
    let _ = np
        .write(brightness(data.iter().cloned(), np_mgr.clock_brightness()))
        .await;
    Timer::after(Duration::from_secs(3)).await;

    info!("Switching all LEDs off");
    let data = [RGB8::default(); 16];
    let _ = np.write(brightness(data.iter().cloned(), 0)).await;

    info!("hand back objects to the mutex");
    // put the objects back into the Mutex for future use
    *spi_np_guard = Some(spi);
    *neopixel_mgr_guard = Some(np_mgr);
}
