//! # Neopixel task
//! This module contains the tasks that control the neopixel LED ring.
//!
//! The tasks are responsible for initializing the neopixel, setting the colors of the LEDs, and updating the LEDs.
use defmt::{info, warn};
use defmt_rtt as _;
use embassy_rp::{
    Peri,
    peripherals::{DMA_CH2, PIN_15, PIO1},
    pio::{Common, StateMachine},
    pio_programs::ws2812::{PioWs2812, PioWs2812Program},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer};
use panic_probe as _;
use smart_leds::{RGB8, brightness};

use crate::{
    event::{Event, send_event},
    state::{AlarmState, OperationMode, SYSTEM_STATE, SystemState},
};

/// Signal for starting/updating the light effects with time data
static LIGHTFX_START_SIGNAL: Signal<CriticalSectionRawMutex, (u8, u8, u8)> = Signal::new();

/// Signal for stopping the light effects
static LIGHTFX_STOP_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signal for updating the `NeoPixel` brightness
static NEOPIXEL_BRIGHTNESS_UPDATE_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signals the light effects to start/update with the given time
pub fn signal_lightfx_start(hour: u8, minute: u8, second: u8) {
    LIGHTFX_START_SIGNAL.signal((hour, minute, second));
}

/// Signals the light effects to stop
pub fn signal_lightfx_stop() {
    LIGHTFX_STOP_SIGNAL.signal(());
}

/// Signals that the `NeoPixel` brightness should be updated from system state
pub fn signal_neopixel_brightness_update() {
    NEOPIXEL_BRIGHTNESS_UPDATE_SIGNAL.signal(());
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

/// Checks if the brightness update signal has been signaled
fn is_brightness_update_signaled() -> bool {
    NEOPIXEL_BRIGHTNESS_UPDATE_SIGNAL.signaled()
}

/// Resets the brightness update signal
fn reset_brightness_update_signal() {
    NEOPIXEL_BRIGHTNESS_UPDATE_SIGNAL.reset();
}

/// Number of LEDs in the ring (as usize for compile-time array sizing)
const NUM_LEDS_USIZE: usize = 16;

/// Number of LEDs in the ring (as u8 for calculations)
const NUM_LEDS: u8 = 16;

/// Type alias for the neopixel LED controller
type NeopixelType = PioWs2812<'static, PIO1, 0, NUM_LEDS_USIZE>;

/// Helper struct to bundle time values (hour, minute, second)
#[derive(Clone, Copy, PartialEq, Eq)]
struct ClockTime {
    /// Hour (0-23)
    hour: u8,
    /// Minute (0-59)
    minute: u8,
    /// Second (0-59)
    second: u8,
}

impl ClockTime {
    /// Creates a new `ClockTime` from hour, minute, and second values
    const fn new(hour: u8, minute: u8, second: u8) -> Self {
        Self { hour, minute, second }
    }
}

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

/// LED state tracker to avoid redundant writes and save power
#[derive(Clone, Copy, PartialEq, Eq)]
enum LedState {
    /// LEDs are off (all black)
    Off,
    /// LEDs are displaying the analog clock
    AnalogClock,
    /// LEDs are in alarm effect mode
    AlarmEffect,
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

    /// Updates brightness from system state
    pub async fn update_from_system_state(&mut self) {
        let state = SYSTEM_STATE.lock().await;
        if let Some(s) = state.as_ref() {
            self.clock_brightness = s.system_settings.get_clock_brightness();
            info!("Updated clock brightness to {}", self.clock_brightness);
        }
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

/// Helper function to apply brightness to an LED data array
fn apply_brightness(data: &[RGB8], brightness_level: u8) -> [RGB8; NUM_LEDS_USIZE] {
    let mut result = [RGB8::default(); NUM_LEDS_USIZE];
    for (i, led) in brightness(data.iter().copied(), brightness_level)
        .take(NUM_LEDS_USIZE)
        .enumerate()
    {
        result[i] = led;
    }
    result
}

/// Calculates the LED index for a given time value
///
/// Maps a time value (0-59 for minutes/seconds or 1-12 for hours) to an LED index on the ring.
/// Uses integer arithmetic: `(value * NUM_LEDS / max_value + offset) % NUM_LEDS`
#[allow(clippy::cast_possible_truncation)]
fn calculate_hand_index(value: u8, max_value: u8) -> u8 {
    let value_mod = u16::from(value % max_value);
    let index =
        (value_mod * u16::from(NUM_LEDS) / u16::from(max_value) + u16::from(NUM_LEDS / 2 + 1)) % u16::from(NUM_LEDS);
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

    // Apply quintic easing (ease-in-quintic) to keep red much longer
    // This makes the transition very slow at first (staying red) and fast at the end (white dominates in last ~20-25%)
    let eased_progress = progress * progress * progress * progress * progress;

    let change = (delta as f32 * eased_progress) as i16;
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
    (((fraction_elapsed * f32::from(NUM_LEDS)) as u8) + 1).clamp(1, u8::try_from(NUM_LEDS_USIZE).unwrap_or(16))
}

/// Displays the analog clock hands on the LED ring
async fn display_analog_clock(
    np: &mut NeopixelType,
    neopixel_mgr: &NeopixelManager,
    time: &ClockTime,
    colors: &ClockColors,
) {
    let mut data = [RGB8::default(); NUM_LEDS_USIZE];

    // Calculate LED indices for each hand
    let hour_normalized = if time.hour.is_multiple_of(12) {
        12
    } else {
        time.hour % 12
    };
    let hour_index = calculate_hand_index(hour_normalized, 12);
    let minute_index = calculate_hand_index(time.minute, 60);
    let second_index = calculate_hand_index(time.second, 60);

    // Set the colors of the hands
    data[hour_index as usize] = colors.hour;
    data[minute_index as usize] = colors.minute;
    data[second_index as usize] = colors.second;

    // When any hands are on the same index, their colors must be mixed
    if hour_index == minute_index && hour_index == second_index {
        data[hour_index as usize] =
            NeopixelManager::mix_colors(NeopixelManager::mix_colors(colors.hour, colors.minute), colors.second);
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
    let bright_data = apply_brightness(&data, neopixel_mgr.clock_brightness());
    np.write(&bright_data).await;
}

/// Turns off all LEDs
async fn turn_off_all_leds(np: &mut NeopixelType) {
    let data = [RGB8::default(); NUM_LEDS_USIZE];
    np.write(&data).await;
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
    np.write(&data).await;

    let params = SunriseParams::new();
    let start_time = Instant::now();

    // Loop for duration milliseconds
    'sunrise: while Instant::now() - start_time < Duration::from_millis(u64::from(params.duration_ms)) {
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
            - (remaining_time.as_millis() as f32 / params.duration_ms as f32 * params.end_brightness) as u8;

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
        let bright_data = apply_brightness(&data, current_brightness);
        np.write(&bright_data).await;
    }

    send_event(Event::SunriseEffectFinished).await;

    // Wait a bit, so that the last of the effect is visible
    Timer::after(Duration::from_millis(300)).await;
}

/// Displays the rainbow noise effect
async fn noise_effect(np: &mut NeopixelType, neopixel_mgr: &NeopixelManager) {
    info!("Noise effect");

    let mut data = [RGB8::default(); NUM_LEDS_USIZE];

    let brightness_level = neopixel_mgr.alarm_brightness();

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

                // Apply brightness directly to each LED to avoid recalculating for entire array
                let color = NeopixelManager::wheel(wheel_index);
                #[allow(clippy::cast_possible_truncation)]
                let r = (u16::from(color.r) * u16::from(brightness_level) / 255) as u8;
                #[allow(clippy::cast_possible_truncation)]
                let g = (u16::from(color.g) * u16::from(brightness_level) / 255) as u8;
                #[allow(clippy::cast_possible_truncation)]
                let b = (u16::from(color.b) * u16::from(brightness_level) / 255) as u8;
                *data_led = RGB8 { r, g, b };
            }
            np.write(&data).await;
            Timer::after(Duration::from_millis(5)).await;
        }
    }
}

/// Handles the normal operation mode
async fn handle_normal_mode(
    np: &mut NeopixelType,
    neopixel_mgr: &NeopixelManager,
    system_state: &SystemState,
    time: &ClockTime,
    colors: &ClockColors,
    current_state: &mut LedState,
    last_time: &mut Option<ClockTime>,
) {
    if system_state.alarm_settings.get_enabled() {
        // Only write if we need to transition to off state
        if *current_state != LedState::Off {
            info!("Alarm enabled - turning off LEDs");
            turn_off_all_leds(np).await;
            *current_state = LedState::Off;
            *last_time = None;
        }
    } else {
        // Only write if state changed or time changed (for analog clock updates)
        if *current_state != LedState::AnalogClock || *last_time != Some(*time) {
            display_analog_clock(np, neopixel_mgr, time, colors).await;
            *current_state = LedState::AnalogClock;
            *last_time = Some(*time);
        }
    }
}

/// Handles the alarm mode
async fn handle_alarm_mode(
    np: &mut NeopixelType,
    neopixel_mgr: &NeopixelManager,
    system_state: &SystemState,
    current_state: &mut LedState,
    last_time: &mut Option<ClockTime>,
) {
    match system_state.alarm_state {
        AlarmState::Sunrise => {
            sunrise_effect(np).await;
            *current_state = LedState::AlarmEffect;
            *last_time = None;
        }
        AlarmState::Noise => {
            noise_effect(np, neopixel_mgr).await;
            *current_state = LedState::AlarmEffect;
            *last_time = None;
        }
        AlarmState::None => {
            warn!("Alarm state is None, this should not happen");
        }
    }
}

#[embassy_executor::task]
pub async fn light_effects_handler(
    mut common: Common<'static, PIO1>,
    sm: StateMachine<'static, PIO1, 0>,
    pin: Peri<'static, PIN_15>,
    dma: Peri<'static, DMA_CH2>,
    program: PioWs2812Program<'static, PIO1>,
) {
    info!("Analog clock task start");

    let mut neopixel_mgr = NeopixelManager::new();
    let mut np = PioWs2812::new(&mut common, sm, dma, pin, &program);

    // Load initial brightness from system state
    neopixel_mgr.update_from_system_state().await;
    let colors = ClockColors::new();

    // Track LED state to avoid redundant writes and save power
    let mut current_led_state = LedState::Off;
    let mut last_time: Option<ClockTime> = None;

    // All off initially
    turn_off_all_leds(&mut np).await;

    'mainloop: loop {
        // Check if brightness update is signaled
        if is_brightness_update_signaled() {
            reset_brightness_update_signal();
            neopixel_mgr.update_from_system_state().await;
            // Force re-render by clearing the last time
            last_time = None;
            // Re-trigger the current display by signaling with zeros
            signal_lightfx_start(0, 0, 0);
        }

        // Wait for the signal to update the neopixel
        let (hour, minute, second) = wait_for_lightfx_start().await;
        info!("LightFX signal received: ({}, {}, {})", hour, minute, second);

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
            OperationMode::Initializing => {
                // Only write if not already off
                if current_led_state != LedState::Off {
                    info!("Initializing mode - LEDs remain off");
                    turn_off_all_leds(&mut np).await;
                    current_led_state = LedState::Off;
                    last_time = None;
                }
            }
            OperationMode::Normal
            | OperationMode::Menu
            | OperationMode::SetAlarmTime
            | OperationMode::SystemInfo
            | OperationMode::SettingsMenu
            | OperationMode::SetVolume
            | OperationMode::SetClockBrightness
            | OperationMode::SetTimeManual => {
                let time = ClockTime::new(hour, minute, second);
                handle_normal_mode(
                    &mut np,
                    &neopixel_mgr,
                    &system_state,
                    &time,
                    &colors,
                    &mut current_led_state,
                    &mut last_time,
                )
                .await;
            }
            OperationMode::Alarm => {
                handle_alarm_mode(
                    &mut np,
                    &neopixel_mgr,
                    &system_state,
                    &mut current_led_state,
                    &mut last_time,
                )
                .await;
            }
            OperationMode::Standby => {
                // Only write once when entering standby, then skip redundant writes
                // This saves power by avoiding unnecessary PIO state machine activity
                if current_led_state != LedState::Off {
                    info!("Entering standby mode - turning off LEDs (power saving)");
                    turn_off_all_leds(&mut np).await;
                    current_led_state = LedState::Off;
                    last_time = None;
                }
            }
        }
    }
}
