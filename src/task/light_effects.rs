//! # Neopixel task
//! This module contains the tasks that control the neopixel LED ring.
//!
//! The tasks are responsible for initializing the neopixel, setting the colors of the LEDs, and updating the LEDs.
use crate::task::resources::NeopixelResources;
use crate::task::state::{AlarmState, OperationMode, STATE_MANAGER_MUTEX};
use crate::task::task_messages::{
    Commands, Events, EVENT_CHANNEL, LIGHTFX_SIGNAL, LIGHTFX_STOP_SIGNAL,
};
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::spi::{Config, Phase, Polarity, Spi};
use embassy_time::{Duration, Timer};
use smart_leds::{brightness, RGB8};
use ws2812_async::Ws2812;

use {defmt_rtt as _, panic_probe as _};

const NUM_LEDS: usize = 16;

pub struct NeopixelManager {
    alarm_brightness: u8,
    clock_brightness: u8,
}

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
pub async fn light_effects_handler(_spawner: Spawner, r: NeopixelResources) {
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
        // all off
        let data = [RGB8::default(); 16];
        np.write(brightness(data.iter().cloned(), 0)).await.ok();

        // wait for the signal to update the neopixel
        let command = LIGHTFX_SIGNAL.wait().await;
        let (hour, minute, second) = match command {
            Commands::LightFXUpdate((hour, minute, second)) => (hour, minute, second),
            _ => (0, 0, 0),
        };

        // get the state of the system out of the mutex and quickly drop the mutex
        let state_manager_guard = STATE_MANAGER_MUTEX.lock().await;
        let state_manager = match state_manager_guard.clone() {
            Some(state_manager) => state_manager,
            None => {
                error!("State manager not initialized");
                continue;
            }
        };
        drop(state_manager_guard);

        match state_manager.operation_mode {
            OperationMode::Normal
            | OperationMode::Menu
            | OperationMode::SetAlarmTime
            | OperationMode::SystemInfo => {
                if !state_manager.alarm_settings.get_enabled() {
                    info!("Analog clock mode");
                    // no loop, just set the time on the neopixel ring
                    // just a simple effect for now
                    // Set all LEDs to green
                    let color = neopixel_mgr.rgb_to_grb((0, 255, 0));
                    let data = [color; 16];
                    let _ = np
                        .write(brightness(
                            data.iter().cloned(),
                            neopixel_mgr.clock_brightness(),
                        ))
                        .await;
                    Timer::after(Duration::from_millis(300)).await;
                } else {
                    // we do nothing
                }
            }
            OperationMode::Alarm => {
                info!("Alarm mode");
                match state_manager.alarm_state {
                    AlarmState::Sunrise => {
                        info!("Sunrise effect");

                        // ToDo: loop through the sunrise effect, just a simple blinking effect for now
                        let mut cntr = 0;
                        loop {
                            if cntr > 10 {
                                break;
                            }

                            // Set all LEDs to blue
                            let color = neopixel_mgr.rgb_to_grb((0, 0, 255));
                            let data = [color; 16];
                            let _ = np
                                .write(brightness(
                                    data.iter().cloned(),
                                    neopixel_mgr.clock_brightness(),
                                ))
                                .await;
                            Timer::after(Duration::from_secs(1)).await;

                            // all off
                            let data = [RGB8::default(); 16];
                            np.write(brightness(data.iter().cloned(), 0)).await.ok();
                            Timer::after(Duration::from_secs(1)).await;

                            cntr += 1;
                        }

                        EVENT_CHANNEL
                            .sender()
                            .send(Events::SunriseEffectFinished)
                            .await;
                    }
                    AlarmState::Noise => {
                        info!("Noise effect");
                        // ToDo: loop through the noise effect, until the alarm is stopped
                        loop {
                            if LIGHTFX_STOP_SIGNAL.signaled() {
                                info!("Noise effect aborting");
                                LIGHTFX_STOP_SIGNAL.reset();
                                break;
                            }

                            // Set all LEDs to red
                            let color = neopixel_mgr.rgb_to_grb((255, 0, 0));
                            let data = [color; 16];
                            let _ = np
                                .write(brightness(
                                    data.iter().cloned(),
                                    neopixel_mgr.clock_brightness(),
                                ))
                                .await;
                            Timer::after(Duration::from_secs(1)).await;

                            // all off
                            let data = [RGB8::default(); 16];
                            np.write(brightness(data.iter().cloned(), 0)).await.ok();
                            Timer::after(Duration::from_secs(1)).await;

                            Timer::after(Duration::from_secs(1)).await;
                        }
                    }
                    AlarmState::None => {
                        // we do nothing, and even getting here is an error
                        error!("Alarm state is None, this should not happen");
                    }
                }
            }
            OperationMode::Standby => {
                info!("Standby mode");
                // we do nothing
            }
        }
    }
}
