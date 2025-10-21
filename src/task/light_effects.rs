//! # Neopixel task
//! This module contains the tasks that control the neopixel LED ring.
//!
//! The tasks are responsible for initializing the neopixel, setting the colors of the LEDs, and updating the LEDs.
use crate::event::{Event, send_event};
use crate::state::{AlarmState, OperationMode, SYSTEM_STATE, SystemState};
use defmt::{info, warn};

use embassy_rp::peripherals::SPI0;
use embassy_rp::spi::Spi;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Instant;
use embassy_time::{Duration, Timer};
use smart_leds::SmartLedsWriteAsync;
use smart_leds::{RGB8, brightness};
use ws2812_async::{Grb, Ws2812};
use {defmt_rtt as _, panic_probe as _};

/// Signal for starting/updating the light effects with time data
static LIGHTFX_START_SIGNAL: Signal<CriticalSectionRawMutex, (u8, u8, u8)> = Signal::new();

/// Signal for stopping the light effects
static LIGHTFX_STOP_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signals the light effects to start/update with the given time
pub fn signal_lightfx_start(hour: u8, minute: u8, second: u8) {
    LIGHTFX_START_SIGNAL.signal((hour, minute, second));
}

/// Signals the light effects to stop
pub fn signal_lightfx_stop() {
    LIGHTFX_STOP_SIGNAL.signal(());
}

/// Waits for the next light effects start signal
async fn wait_for_lightfx_start() -> (u8, u8, u8) {
    LIGHTFX_START_SIGNAL.wait().await
}

/// Checks if the light effects stop signal has been signaled
fn is_lightfx_stop_signaled() -> bool {
    LIGHTFX_STOP_SIGNAL.signaled()
}

/// Resets the light effects stop signal
fn reset_lightfx_stop_signal() {
    LIGHTFX_STOP_SIGNAL.reset();
}

/// Number of LEDs in the ring (as usize for compile-time array sizing)
const NUM_LEDS_USIZE: usize = 16;

/// Number of LEDs in the ring (as u8 for calculations)
const NUM_LEDS: u8 = 16;

/// Type alias for the neopixel LED controller
type NeopixelType =
    Ws2812<Spi<'static, SPI0, embassy_rp::spi::Async>, Grb, { 12 * NUM_LEDS_USIZE }>;

/// Helper struct to bundle clock hand colors
struct ClockColors {
    /// Red color for hour hand
    hour: RGB8,
    /// Green color for minute hand
    minute: RGB8,
    /// Blue color for second hand
    second: RGB8,
}

impl ClockColors {
    /// Creates new clock colors with standard RGB values
    const fn new() -> Self {
        Self {
            hour: RGB8 { r: 255, g: 0, b: 0 },
            minute: RGB8 { r: 0, g: 255, b: 0 },
            second: RGB8 { r: 0, g: 0, b: 255 },
        }
    }
}

/// Manages the neopixel LED ring, including brightness settings for alarm and clock modes.
pub struct NeopixelManager {
    /// Brightness setting for alarm mode
    alarm_brightness: u8,
    /// Brightness setting for clock mode
    clock_brightness: u8,
}

impl NeopixelManager {
    /// Creates a new `NeopixelManager` with default brightness settings.
    pub const fn new() -> Self {
        Self {
            alarm_brightness: 10,
            clock_brightness: 1,
        }
    }

    /// Returns the alarm brightness setting.
    pub const fn alarm_brightness(&self) -> u8 {
        self.alarm_brightness
    }

    /// Returns the clock brightness setting.
    pub const fn clock_brightness(&self) -> u8 {
        self.clock_brightness
    }

    /// Mixes two colors together
    fn mix_colors(color1: RGB8, color2: RGB8) -> RGB8 {
        RGB8 {
            r: (u16::from(color1.r) + u16::from(color2.r)).min(255) as u8,
            g: (u16::from(color1.g) + u16::from(color2.g)).min(255) as u8,
            b: (u16::from(color1.b) + u16::from(color2.b)).min(255) as u8,
        }
    }

    /// Function to convert a color wheel value to RGB
    pub fn wheel(mut wheel_pos: u8) -> RGB8 {
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

/// Calculates the LED index for a given time value
///
/// Maps a time value (0-59 for minutes/seconds or 1-12 for hours) to an LED index on the ring.
/// Uses integer arithmetic: `(value * NUM_LEDS / max_value + offset) % NUM_LEDS`
#[allow(clippy::cast_possible_truncation)]
fn calculate_hand_index(value: u8, max_value: u8) -> u8 {
    let value_mod = u16::from(value % max_value);
    let index = (value_mod * u16::from(NUM_LEDS) / u16::from(max_value)
        + u16::from(NUM_LEDS / 2 + 1))
        % u16::from(NUM_LEDS);
    index as u8
}

/// Interpolates a color value between start and end based on elapsed time
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless
)]
fn interpolate_color_value(start: u8, end: u8, elapsed_millis: u32, total_millis: u32) -> u8 {
    if total_millis == 0 {
        return end;
    }
    let delta = i16::from(end) - i16::from(start);
    let progress = elapsed_millis as f32 / total_millis as f32;
    let change = (delta as f32 * progress) as i16;
    let result = i16::from(start) + change;
    result.clamp(0, 255) as u8
}

/// Calculates the number of LEDs to light for the sunrise effect
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn calculate_lit_leds(fraction_elapsed: f32) -> u8 {
    (((fraction_elapsed * f32::from(NUM_LEDS)) as u8) + 1)
        .clamp(1, u8::try_from(NUM_LEDS_USIZE).unwrap_or(16))
}

/// Displays the analog clock hands on the LED ring
async fn display_analog_clock(
    np: &mut NeopixelType,
    neopixel_mgr: &NeopixelManager,
    hour: u8,
    minute: u8,
    second: u8,
    colors: &ClockColors,
) {
    let mut data = [RGB8::default(); NUM_LEDS_USIZE];

    // Calculate LED indices for each hand
    let hour_normalized = if hour.is_multiple_of(12) {
        12
    } else {
        hour % 12
    };
    let hour_index = calculate_hand_index(hour_normalized, 12);
    let minute_index = calculate_hand_index(minute, 60);
    let second_index = calculate_hand_index(second, 60);

    // Set the colors of the hands
    data[hour_index as usize] = colors.hour;
    data[minute_index as usize] = colors.minute;
    data[second_index as usize] = colors.second;

    // When any hands are on the same index, their colors must be mixed
    if hour_index == minute_index && hour_index == second_index {
        data[hour_index as usize] = NeopixelManager::mix_colors(
            NeopixelManager::mix_colors(colors.hour, colors.minute),
            colors.second,
        );
    } else {
        if hour_index == minute_index {
            data[hour_index as usize] = NeopixelManager::mix_colors(colors.hour, colors.minute);
        }
        if hour_index == second_index {
            data[hour_index as usize] = NeopixelManager::mix_colors(colors.hour, colors.second);
        }
        if minute_index == second_index {
            data[minute_index as usize] = NeopixelManager::mix_colors(colors.minute, colors.second);
        }
    }

    // Write the data to the neopixel
    let _ = np
        .write(brightness(
            data.iter().copied(),
            neopixel_mgr.clock_brightness(),
        ))
        .await;
}

/// Turns off all LEDs
async fn turn_off_all_leds(np: &mut NeopixelType) {
    let data = [RGB8::default(); NUM_LEDS_USIZE];
    let _ = np.write(brightness(data.iter().copied(), 0)).await;
}

/// Helper struct for sunrise effect parameters
struct SunriseParams {
    /// Starting color (dark red)
    start_color: RGB8,
    /// Ending color (warm white)
    end_color: RGB8,
    /// Target brightness at end of effect
    end_brightness: f32,
    /// Duration in milliseconds
    duration_ms: u32,
}

impl SunriseParams {
    /// Creates standard sunrise effect parameters (60 second sunrise)
    const fn new() -> Self {
        Self {
            start_color: RGB8::new(139, 0, 0),
            end_color: RGB8::new(255, 250, 244),
            end_brightness: 100.0,
            duration_ms: 60_000,
        }
    }
}

/// Displays the sunrise effect
async fn sunrise_effect(np: &mut NeopixelType) {
    info!("Sunrise effect");

    let mut data = [RGB8::default(); NUM_LEDS_USIZE];
    let _ = np.write(brightness(data.iter().copied(), 0)).await;

    let params = SunriseParams::new();
    let start_time = Instant::now();

    // Loop for duration milliseconds
    'sunrise: while Instant::now() - start_time
        < Duration::from_millis(u64::from(params.duration_ms))
    {
        // Check if the effect should be stopped
        if is_lightfx_stop_signaled() {
            info!("Sunrise effect aborting");
            reset_lightfx_stop_signal();
            break 'sunrise;
        }

        // Calculate the elapsed time and the remaining time
        let elapsed_time = Instant::now() - start_time;
        let remaining_time = Duration::from_millis(u64::from(params.duration_ms)) - elapsed_time;
        #[allow(clippy::cast_possible_truncation)]
        let elapsed_millis = elapsed_time.as_millis() as u32;

        #[allow(clippy::cast_precision_loss)]
        let fraction_elapsed = elapsed_millis as f32 / params.duration_ms as f32;

        // Calculate the current brightness based on the elapsed time
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        #[allow(clippy::cast_possible_truncation)]
        let current_brightness = params.end_brightness as u8
            - (remaining_time.as_millis() as f32 / params.duration_ms as f32
                * params.end_brightness) as u8;

        // Calculate the current color based on the elapsed time
        let current_color = RGB8::new(
            interpolate_color_value(
                params.start_color.r,
                params.end_color.r,
                elapsed_millis,
                params.duration_ms,
            ),
            interpolate_color_value(
                params.start_color.g,
                params.end_color.g,
                elapsed_millis,
                params.duration_ms,
            ),
            interpolate_color_value(
                params.start_color.b,
                params.end_color.b,
                elapsed_millis,
                params.duration_ms,
            ),
        );

        // Calculate the number of leds to light up based on the elapsed time fraction
        let current_leds = usize::from(calculate_lit_leds(fraction_elapsed));

        // Set the leds
        for current_color_led in &mut data[..current_leds] {
            *current_color_led = current_color;
        }

        // Write the data to the neopixel
        let _ = np
            .write(brightness(data.iter().copied(), current_brightness))
            .await;
    }

    send_event(Event::SunriseEffectFinished).await;

    // Wait a bit, so that the last of the effect is visible
    Timer::after(Duration::from_millis(300)).await;
}

/// Displays the rainbow noise effect
async fn noise_effect(np: &mut NeopixelType, neopixel_mgr: &NeopixelManager) {
    info!("Noise effect");

    let mut data = [RGB8::default(); NUM_LEDS_USIZE];

    'noise: loop {
        for j in 0u16..(256 * 5) {
            if is_lightfx_stop_signaled() {
                info!("Noise effect aborting");
                reset_lightfx_stop_signal();
                break 'noise;
            }

            for (i, data_led) in data.iter_mut().enumerate() {
                // Calculate the color wheel index with wraparound behavior.
                // The base offset for each LED progresses through the color wheel,
                // and j cycles through the spectrum. We use wrapping arithmetic to
                // ensure the rainbow continuously cycles.
                #[allow(clippy::cast_possible_truncation)]
                let base_offset = ((i as u16 * 256) / u16::from(NUM_LEDS)) as u8;
                let j_clamped = (j & 255) as u8;
                let wheel_index = base_offset.wrapping_add(j_clamped);
                *data_led = NeopixelManager::wheel(wheel_index);
            }
            np.write(brightness(
                data.iter().copied(),
                neopixel_mgr.alarm_brightness(),
            ))
            .await
            .ok();
            Timer::after(Duration::from_millis(5)).await;
        }
    }
}

/// Handles the normal operation mode
async fn handle_normal_mode(
    np: &mut NeopixelType,
    neopixel_mgr: &NeopixelManager,
    system_state: &SystemState,
    hour: u8,
    minute: u8,
    second: u8,
    colors: &ClockColors,
) {
    if system_state.alarm_settings.get_enabled() {
        turn_off_all_leds(np).await;
    } else {
        display_analog_clock(np, neopixel_mgr, hour, minute, second, colors).await;
    }
}

/// Handles the alarm mode
async fn handle_alarm_mode(
    np: &mut NeopixelType,
    neopixel_mgr: &NeopixelManager,
    system_state: &SystemState,
) {
    match system_state.alarm_state {
        AlarmState::Sunrise => {
            sunrise_effect(np).await;
        }
        AlarmState::Noise => {
            noise_effect(np, neopixel_mgr).await;
        }
        AlarmState::None => {
            warn!("Alarm state is None, this should not happen");
        }
    }
}

#[embassy_executor::task]
pub async fn light_effects_handler(spi: Spi<'static, SPI0, embassy_rp::spi::Async>) {
    info!("Analog clock task start");

    let neopixel_mgr = NeopixelManager::new();
    let mut np: Ws2812<_, Grb, { 12 * NUM_LEDS_USIZE }> = Ws2812::new(spi);
    let colors = ClockColors::new();

    // All off initially
    turn_off_all_leds(&mut np).await;

    'mainloop: loop {
        // Wait for the signal to update the neopixel
        let (hour, minute, second) = wait_for_lightfx_start().await;
        info!(
            "LightFX signal received: ({}, {}, {})",
            hour, minute, second
        );

        // Get the state of the system out of the mutex and quickly drop the mutex
        let system_state: SystemState;
        '_system_state_mutex: {
            let system_state_guard = SYSTEM_STATE.lock().await;
            system_state = if let Some(system_state) = system_state_guard.clone() {
                system_state
            } else {
                warn!("System state not initialized");
                drop(system_state_guard);
                Timer::after(Duration::from_secs(1)).await;
                continue 'mainloop;
            };
        }

        info!("{}", system_state);

        match system_state.operation_mode {
            OperationMode::Normal
            | OperationMode::Menu
            | OperationMode::SetAlarmTime
            | OperationMode::SystemInfo => {
                handle_normal_mode(
                    &mut np,
                    &neopixel_mgr,
                    &system_state,
                    hour,
                    minute,
                    second,
                    &colors,
                )
                .await;
            }
            OperationMode::Alarm => {
                handle_alarm_mode(&mut np, &neopixel_mgr, &system_state).await;
            }
            OperationMode::Standby => {
                info!("Standby mode");
                turn_off_all_leds(&mut np).await;
            }
        }
    }
}
