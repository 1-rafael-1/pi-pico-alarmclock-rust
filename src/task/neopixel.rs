use crate::task::alarm_mgr;
use crate::task::peripherals::NeopixelResources;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::adc::Async;
use embassy_rp::peripherals::{self, SPI0};
use embassy_rp::spi::{Config, Phase, Polarity, Spi};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, channel::Receiver};
use embassy_time::{Duration, Timer};
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
    r: NeopixelResources,
    control: Receiver<'static, CriticalSectionRawMutex, alarm_mgr::AlarmState, 1>,
) {
    info!("Analog clock task start");

    // Spi configuration for the neopixel
    let mut spi_config = Config::default();
    spi_config.frequency = 3_800_000;
    spi_config.phase = Phase::CaptureOnFirstTransition;
    spi_config.polarity = Polarity::IdleLow;
    let spi = Spi::new_txonly(r.inner_spi, r.clk_pin, r.mosi_pin, r.tx_dma_ch, spi_config);
    let neopixel_mgr = NeopixelManager::new(100, 10);
    let mut np: Ws2812<_, { 12 * NUM_LEDS }> = Ws2812::new(spi);

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

        let data = [RGB8::default(); 16];
        np.write(brightness(
            data.iter().cloned(),
            neopixel_mgr.alarm_brightness(),
        ))
        .await
        .ok();

        // Set all LEDs to blue for 3 seconds
        let blue = neopixel_mgr.rgb_to_grb((0, 0, 255));
        let data = [blue; 16];
        let _ = np
            .write(brightness(
                data.iter().cloned(),
                neopixel_mgr.clock_brightness(),
            ))
            .await;
        Timer::after(Duration::from_secs(3)).await;

        // Set all LEDs to off
        let data = [RGB8::default(); 16];
        let _ = np.write(brightness(data.iter().cloned(), 0)).await;
    }
}
