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
use embassy_time::Instant;
use embassy_time::{Duration, Timer};
use smart_leds::{brightness, RGB8};
use ws2812_async::Ws2812;
use {defmt_rtt as _, panic_probe as _};

// Number of LEDs in the ring
const NUM_LEDS: usize = 16;

/// Manages the neopixel LED ring, including brightness settings for alarm and clock modes.
pub struct NeopixelManager {
    alarm_brightness: u8,
    clock_brightness: u8,
}

impl NeopixelManager {
    /// Creates a new `NeopixelManager` with default brightness settings.
    pub fn new() -> Self {
        Self {
            alarm_brightness: 90,
            clock_brightness: 1,
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

    /// Function to convert a color wheel value to RGB
    pub fn wheel(&self, mut wheel_pos: u8) -> RGB8 {
        wheel_pos = 255 - wheel_pos;
        if wheel_pos < 85 {
            return (255 - wheel_pos * 3, 0, wheel_pos * 3).into();
        }
        if wheel_pos < 170 {
            wheel_pos -= 85;
            return (0, wheel_pos * 3, 255 - wheel_pos * 3).into();
        }
        wheel_pos -= 170;
        (wheel_pos * 3, 255 - wheel_pos * 3, 0).into()
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
    let neopixel_mgr = NeopixelManager::new();
    let mut np: Ws2812<_, { 12 * NUM_LEDS }> = Ws2812::new(spi);

    let red = neopixel_mgr.rgb_to_grb((255, 0, 0));
    let green = neopixel_mgr.rgb_to_grb((0, 255, 0));
    let blue = neopixel_mgr.rgb_to_grb((0, 0, 255));

    // all off
    let mut data = [RGB8::default(); NUM_LEDS];
    np.write(brightness(data.iter().cloned(), 0)).await.ok();

    '_outer: loop {
        // wait for the signal to update the neopixel
        let command = LIGHTFX_SIGNAL.wait().await;
        info!("LightFX signal received: {:?}", command);
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
                    // Analog Clock mode

                    // Calculate the LED indices for each hand
                    // the hour hand will deliberately be dragging behind since we choose to not account for minutes passed in the hour

                    // Convert the hour value to an index on the ring of 16 LEDs
                    let hour = if (hour % 12) == 0 { 12 } else { hour % 12 };
                    let hour_index = ((hour as f32 / 12.0 * NUM_LEDS as f32) as u8
                        - (NUM_LEDS as f32 / 2.0) as u8
                        + 1)
                        % NUM_LEDS as u8;

                    // Convert the minute value to an index on the ring of 16 LEDs
                    let minute_index = (((minute % 60) as f32 * NUM_LEDS as f32 / 60.0
                        + NUM_LEDS as f32 / 2.0
                        + 1.0)
                        % NUM_LEDS as f32) as u8;

                    // Convert the second value to an index on the ring of 16 LEDs
                    let second_index = (((second % 60) as f32 * NUM_LEDS as f32 / 60.0
                        + NUM_LEDS as f32 / 2.0
                        + 1.0)
                        % NUM_LEDS as f32) as u8;

                    // clear the data
                    data = [RGB8::default(); NUM_LEDS];

                    // Set the colors of the hands
                    data[hour_index as usize] = red;
                    data[minute_index as usize] = green;
                    data[second_index as usize] = blue;

                    // but when any hands are on the same index, their colors must be mixed
                    if hour_index == minute_index {
                        data[hour_index as usize] = neopixel_mgr.rgb_to_grb((255, 255, 0));
                    };
                    if hour_index == second_index {
                        data[hour_index as usize] = neopixel_mgr.rgb_to_grb((255, 0, 255));
                    };
                    if minute_index == second_index {
                        data[minute_index as usize] = neopixel_mgr.rgb_to_grb((0, 255, 255));
                    };
                    if minute_index == second_index && hour_index == minute_index {
                        data[hour_index as usize] = neopixel_mgr.rgb_to_grb((255, 255, 255));
                    };

                    // write the data to the neopixel
                    let _ = np
                        .write(brightness(
                            data.iter().cloned(),
                            neopixel_mgr.clock_brightness(),
                        ))
                        .await;
                } else {
                    // all off
                    let data = [RGB8::default(); NUM_LEDS];
                    let _ = np.write(brightness(data.iter().cloned(), 0)).await;
                }
            }
            OperationMode::Alarm => {
                info!("Alarm mode");
                match state_manager.alarm_state {
                    AlarmState::Sunrise => {
                        info!("Sunrise effect");
                        // simumlate a sunrise: start with all leds off, then slowly add leds while all leds that are already used slowly change color from red to warm white

                        // all off
                        let mut data = [RGB8::default(); NUM_LEDS];
                        let _ = np.write(brightness(data.iter().cloned(), 0)).await;

                        // initialize the variables
                        let start_color = RGB8::new(139, 0, 0); //dark red
                        let end_color = RGB8::new(255, 250, 244); // warm white
                        let end_brightness = 100.0;
                        let effect_duration: u64 = 60; // seconds
                        let start_time = Instant::now();

                        // loop for duration seconds
                        'sunrise: while Instant::now() - start_time
                            < Duration::from_secs(effect_duration)
                        {
                            // check if the effect should be stopped
                            if LIGHTFX_STOP_SIGNAL.signaled() {
                                info!("Sunrise effect aborting");
                                LIGHTFX_STOP_SIGNAL.reset();
                                break 'sunrise;
                            }

                            // calculate the elapsed time and the remaining time
                            let elapsed_time = Instant::now() - start_time;
                            let remaining_time =
                                Duration::from_secs(effect_duration) - elapsed_time;
                            let fraction_elapsed =
                                elapsed_time.as_secs() as f32 / effect_duration as f32;

                            // calculate the current brightness based on the elapsed time
                            let current_brightness = end_brightness as u8
                                - (remaining_time.as_secs() as f32 / effect_duration as f32
                                    * end_brightness) as u8;

                            // calculate the current color based on the elapsed time
                            let mut current_color = RGB8::new(
                                start_color.r
                                    + ((end_color.r as i16 - start_color.r as i16) as f32
                                        / effect_duration as f32
                                        * elapsed_time.as_secs() as f32)
                                        as u8,
                                start_color.g
                                    + ((end_color.g as i16 - start_color.g as i16) as f32
                                        / effect_duration as f32
                                        * elapsed_time.as_secs() as f32)
                                        as u8,
                                start_color.b
                                    + ((end_color.b as i16 - start_color.b as i16) as f32
                                        / effect_duration as f32
                                        * elapsed_time.as_secs() as f32)
                                        as u8,
                            );
                            current_color = neopixel_mgr.rgb_to_grb((
                                current_color.r,
                                current_color.g,
                                current_color.b,
                            ));

                            // calculate the number of leds to light up based on the elapsed time fraction, min 1, max NUM_LEDS
                            let current_leds = (((fraction_elapsed * NUM_LEDS as f32) as usize)
                                + 1)
                            .clamp(1, NUM_LEDS);

                            // set the leds
                            for i in 0..current_leds {
                                data[i] = current_color;
                            }

                            // write the date to the neopixel
                            let _ = np
                                .write(brightness(data.iter().cloned(), current_brightness))
                                .await;
                        }

                        EVENT_CHANNEL
                            .sender()
                            .send(Events::SunriseEffectFinished)
                            .await;

                        // and wait a bit, so that the last of the effect is visible
                        Timer::after(Duration::from_millis(300)).await;
                    }
                    AlarmState::Noise => {
                        info!("Noise effect");

                        // a beautiful rainbow effect, taken from https://github.com/kalkyl/ws2812-async
                        let mut data = [RGB8::default(); NUM_LEDS];

                        'noise: loop {
                            for j in 0..(256 * 5) {
                                if LIGHTFX_STOP_SIGNAL.signaled() {
                                    info!("Noise effect aborting");
                                    LIGHTFX_STOP_SIGNAL.reset();
                                    break 'noise;
                                };

                                for i in 0..NUM_LEDS {
                                    data[i] = neopixel_mgr.wheel(
                                        (((i * 256) as u16 / NUM_LEDS as u16 + j as u16) & 255)
                                            as u8,
                                    );
                                }
                                np.write(brightness(
                                    data.iter().cloned(),
                                    neopixel_mgr.alarm_brightness,
                                ))
                                .await
                                .ok();
                                Timer::after(Duration::from_millis(5)).await;
                            }
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
                // all off
                let data = [RGB8::default(); NUM_LEDS];
                let _ = np.write(brightness(data.iter().cloned(), 0)).await;
                // we do nothing
            }
        }
    }
}
