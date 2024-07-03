use crate::task::alarm_mgr;
use cortex_m::register::control;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::peripherals;
use embassy_rp::spi::Spi;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, channel::Receiver};
use embassy_time::{Duration, Timer};
use serde::de;
use smart_leds::{brightness, RGB8};
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
    control: Receiver<'static, CriticalSectionRawMutex, alarm_mgr::AlarmState, 1>,
) {
    info!("Analog clock task start");

    loop {
        // await the control signal and check if it is idle
        // if it is idle, continue with the sunrise
        // if it is not idle, restart the loop
        // we do not really need to read & check the state, but it is nice to know what is happening
        let received_state = control.receive().await;
        info!("Received state: {:?}", received_state);
        if received_state == alarm_mgr::AlarmState::Idle {
            info!("Received Idle signal");
        } else {
            info!("Received other signal");
            continue;
        }

        // Lock the mutex asynchronously
        let mut spi_np_guard = spi_np.lock().await;
        let mut neopixel_mgr_guard = neopixel_mgr.lock().await;

        // Check if the mutex actually contains a NeopixelManager object
        let np_mgr: NeopixelManager;
        if let Some(np_mgr_inner) = neopixel_mgr_guard.take() {
            np_mgr = np_mgr_inner;
        } else {
            return; // Handle the case where the NeopixelManager object was not available (e.g., already taken or never set)
        }

        // Check if the mutex actually contains an Spi object
        let mut spi: Spi<'static, peripherals::SPI0, embassy_rp::spi::Async>;
        if let Some(spi_inner) = spi_np_guard.take() {
            spi = spi_inner;
        } else {
            return; // Handle the case where the SPI object was not available (e.g., already taken or never set)
        }

        // Use the SPI object to create Ws2812
        let mut np: Ws2812<_, { 12 * NUM_LEDS }> = Ws2812::new(&mut spi);

        // Set all LEDs to off
        let data = [RGB8::default(); 16];
        np.write(brightness(data.iter().cloned(), np_mgr.alarm_brightness()))
            .await
            .ok();

        // Set all LEDs to blue for 3 seconds
        let blue = np_mgr.rgb_to_grb((0, 0, 255));
        let data = [blue; 16];
        let _ = np
            .write(brightness(data.iter().cloned(), np_mgr.clock_brightness()))
            .await;
        Timer::after(Duration::from_secs(3)).await;

        // Set all LEDs to off
        let data = [RGB8::default(); 16];
        let _ = np.write(brightness(data.iter().cloned(), 0)).await;

        // put the objects back into the Mutex for future use
        *spi_np_guard = Some(spi);
        *neopixel_mgr_guard = Some(np_mgr);
    }
}

#[embassy_executor::task]
pub async fn sunrise(
    _spawner: Spawner,
    spi_np: &'static SpiType,
    neopixel_mgr: &'static NeopixelManagerType,
    control: Receiver<'static, CriticalSectionRawMutex, alarm_mgr::AlarmState, 1>,
) {
    info!("Sunrise task start");

    loop {
        // await the control signal and check if it is triggered
        // if it is triggered, continue with the sunrise
        // if it is not triggered, restart the loop
        // we do not really need to read & check the state, but it is nice to know what is happening
        let received_state = control.receive().await;
        info!("Received state: {:?}", received_state);
        if received_state == alarm_mgr::AlarmState::Triggered {
            info!("Received Triggered signal");
        } else {
            info!("Received other signal");
            continue;
        }

        // Lock the mutex asynchronously
        let mut spi_np_guard = spi_np.lock().await;
        let mut neopixel_mgr_guard = neopixel_mgr.lock().await;

        // Check if the mutex actually contains a NeopixelManager object
        let np_mgr: NeopixelManager;
        if let Some(np_mgr_inner) = neopixel_mgr_guard.take() {
            np_mgr = np_mgr_inner;
        } else {
            return; // Handle the case where the NeopixelManager object was not available (e.g., already taken or never set)
        }

        // Check if the mutex actually contains an Spi object
        let mut spi: Spi<'static, peripherals::SPI0, embassy_rp::spi::Async>;
        if let Some(spi_inner) = spi_np_guard.take() {
            spi = spi_inner;
        } else {
            return; // Handle the case where the SPI object was not available (e.g., already taken or never set)
        }

        // Use the SPI object to create Ws2812
        let mut np: Ws2812<_, { 12 * NUM_LEDS }> = Ws2812::new(&mut spi);

        // Set all LEDs to off
        let data = [RGB8::default(); 16];
        np.write(brightness(data.iter().cloned(), np_mgr.alarm_brightness()))
            .await
            .ok();

        // Set all LEDs to red for 3 seconds
        let red = np_mgr.rgb_to_grb((0, 255, 0));
        let data = [red; 16];
        let _ = np
            .write(brightness(data.iter().cloned(), np_mgr.clock_brightness()))
            .await;
        Timer::after(Duration::from_secs(3)).await;

        // Set all LEDs to off
        let data = [RGB8::default(); 16];
        let _ = np.write(brightness(data.iter().cloned(), 0)).await;

        // put the objects back into the Mutex for future use
        *spi_np_guard = Some(spi);
        *neopixel_mgr_guard = Some(np_mgr);
    }
}
